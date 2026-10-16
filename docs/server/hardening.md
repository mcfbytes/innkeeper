# Hardening

A public `innkeeperd` is jailed twice. The virtual machine's systemd unit (in the infra repo, not
in this tree) starts it as a non-root user with no capabilities, in its own namespaces, and under
that unit's syscall filter. This page is the second layer: the process jails itself, so the same
limits hold when the unit is loosened or the binary is started by hand. It also records the
`--public` ingress rule. Privacy of logs and stored rows is [privacy.md](privacy.md).

## 1. Public mode accepts only a loopback WebSocket

Without `--public`, today's behaviour is unchanged. `--bind` still defaults to `127.0.0.1:2314`,
the INT 14h listener still comes up unless `--no-int14h`, and `--line` still says how bytes on an
existing connection are read. There is no `--private` flag.

`--public` is the hosted shape. Players arrive only on the WebSocket, which sits behind a TLS
proxy on the same machine (Caddy). A raw TCP connection cannot be tied to a sign-in, so none is
opened.

- An explicit `--bind` is an error (`public mode accepts players only over the WebSocket, so there
  is no raw TCP listener`). The default address is not bound.
- `--ws-bind` is required, and it must be `127.0.0.0/8` or `::1`. `0.0.0.0`, `::` and an
  IPv4-mapped address such as `::ffff:127.0.0.1` are refused. TLS is the proxy's job.
- An explicit `--int14h-bind` is an error. `--no-int14h` is redundant and accepted. The INT 14h
  listener stays off.
- `--line` is accepted. It does not open a socket.
- `--allow-root` is an error, including together with `--no-sandbox`.
- `--capture-dir` is still an error ([privacy.md](privacy.md) section 2).

`connection_gate::admit` (`crates/innkeeperd/src/connection_gate.rs`) runs on the upgrade request,
before the WebSocket handshake. In public mode it requires `?session=<token>` naming a live sign-in
session ([innkeeperd.md](innkeeperd.md) section 8). A missing, unknown or expired token is an HTTP
401 with no upgrade, and the refusal is logged without the token. The line is then bound to that
identity: a Login is accepted only for an account the identity owns. The same listener serves
`/healthz` and `/auth/*` over plain HTTP; nothing else is answered.

With `--public` and no `--auth-config`, startup logs this warning once, and the server still starts:

```
public mode: no sign-in providers are configured (--auth-config), so every WebSocket is refused
```

`--public` also requires `--key-vault`. `--insecure-dev-keys` replaces it only when every
`--page-origin` is loopback, so development keys cannot back a page other machines can reach.

`--public` turns the sandbox in section 3 on. `--no-sandbox` turns it off.

## 2. The outer unit

The unit is the layer that does not depend on this binary cooperating. It is what a public VM
should keep even after the process jails itself: a dedicated user, no ambient capabilities, a
mount and network namespace, and a syscall filter of its own. This repo does not ship that unit
and does not name its path. Weakening it does not remove section 3.

## 3. When the process jails itself

`--sandbox` applies the steps below. It is implied by `--public`, by `--sandbox-log` and by the
hidden probe. `--no-sandbox` wins over `--public` and conflicts with the other three.

The jail is installed only after startup has finished the work that needs a wider process: the
config is parsed, `--data-dir` and the database are open, listening sockets are bound, and the
server vault's secrets have been fetched. The accept loop starts after that. Tokio's worker
threads are created then, so they inherit the jail. Landlock before ABI v8 restricts the calling
thread only; `all_threads` is requested and best effort drops it on older kernels.
Inheritance is why the runtime is built after `confine`.

Offline commands return first and stay unrestricted: `account` (including `account export`),
`audit`, `hold` and `release`. They read a database the server has already closed. The
development provider `dev-oidc` also returns first; it is a loopback test tool, not part of a
public host.

Order inside the jail:

1. Refuse effective uid 0 unless `--allow-root`. The error is `refusing to run as root; pass
   --allow-root only for a local DOSBox setup`. `--allow-root` is that local case. The check runs
   only when the sandbox is applied, so `--public --no-sandbox` as root is the opt-out, and
   `--public` with `--allow-root` is still a parse error.
2. `prctl(PR_SET_NO_NEW_PRIVS, 1)`, so a later setuid or file capability cannot raise privilege.
3. Landlock (section 4).
4. seccomp (section 5).
5. One info line:
   `sandbox: landlock abi=N fs=restricted|open net=bind-restricted|unrestricted, seccomp=kill|log|unsupported, no_new_privs`.

On an operating system other than Linux, `--sandbox` logs
`sandbox: --sandbox is unsupported on this operating system` and continues.

The effective uid is the second field of the `Uid:` line in `/proc/self/status`. `libc`'s
`geteuid` is unsafe, and this crate forbids unsafe. An unreadable status file is treated as root.

## 4. Landlock

The `landlock` crate 0.4 (MIT OR Apache-2.0) builds the ruleset. Compatibility is best effort:
rights the running kernel does not understand are dropped, and the rest are enforced.

Filesystem access handled is ABI v5's full set. What is granted is narrower.

- `--data-dir`, read-write: read file, read directory, write, remove file, remove directory, make
  directory, make a regular file, refer, truncate. Execute, device nodes and device ioctl stay
  denied. This is the only writeable tree. SQLite's database, journal and lock live here.
- Read-only (read file and read directory, not execute) on the paths below, when they exist. A
  missing path is skipped, and one warning names them: `sandbox: skipped missing read-only paths`.
  A symlink is also granted at its canonical target, which is how `/etc/resolv.conf` and
  `/etc/localtime` stay readable when they point elsewhere.

| Need | Paths |
|---|---|
| Outbound TLS, CA store | `/etc/ssl`, `/etc/ssl/certs`, `/etc/ssl/certs/ca-certificates.crt`, `/usr/share/ca-certificates` |
| Resolver | `/etc/resolv.conf`, `/etc/hosts`, `/etc/nsswitch.conf`, `/etc/gai.conf`, `/etc/host.conf` |
| Local time | `/etc/localtime`, `/usr/share/zoneinfo` |
| Runtime reads while serving | `/proc/self`, `/proc/<pid>`, `/proc/cpuinfo`, `/proc/stat`, `/sys/devices/system/cpu`, `/sys/fs/cgroup`, `/dev/urandom` |

A capture directory outside `--data-dir` is not added. One warning says those writes will be
denied. Public mode writes no captures.

TCP bind is handled on ABI v4 and later, and only for the ports the process actually bound. Port
0 is not listed, so an explicit bind of an ephemeral port is denied. `connect` is not handled:
outbound TCP to the sign-in providers stays allowed. On a Landlock ABI 7 kernel, a connect to a
closed port still failed with `ECONNREFUSED`, while a bind of a port that was not listed failed
with `EACCES`. If a later kernel starts denying the implicit bind inside `connect`, the fix is a
port-0 rule, and it has to be retested before it is added.

A kernel with no Landlock logs one warning and continues, in public mode as well:
`sandbox: this kernel has no Landlock; filesystem and TCP bind stay unrestricted`. The summary
then says `abi=unsupported fs=open net=unrestricted`. That is a warning, not a startup error.

`localhost` is resolved once before the jail so glibc's files resolver is already mapped. A later
`dlopen` of a DNS module would need a new executable mapping, which section 5 refuses, so outbound
HTTP does not use glibc's resolver: `reqwest` is built with the pure-Rust `hickory` resolver, which
reads `/etc/resolv.conf` and `/etc/hosts` (granted above) and asks the name servers over UDP and TCP
itself. TLS is rustls with the system CA store, read from the paths in the table.

**Outbound connections.** Before the jail, with `--key-vault`:

- the managed identity endpoint, `http://169.254.169.254` (IMDS), or for a service principal
  `https://login.microsoftonline.com`;
- the server vault, `https://NAME.vault.azure.net`.

After the jail, only while a player signs in:

- each OIDC provider's issuer: discovery (`/.well-known/openid-configuration`), its JWKS, and its
  token endpoint. Discovery and keys are cached for an hour;
- a plain-OAuth provider's token and profile endpoints;
- the name servers in `/etc/resolv.conf`, for those names.

With `--app-insights-connection-string`, after the jail, once a minute:

- the managed identity endpoint (IMDS, or Entra ID for a service principal) for a token whose
  audience is `https://monitor.azure.com/`, cached until five minutes before it expires;
- the connection string's ingestion endpoint, `https://REGION.in.applicationinsights.azure.com`,
  `POST /v2.1/track` (docs/server/innkeeperd.md section 10). The DNS and TLS path is the one the
  sign-in calls use. The tests run it under seccomp kill mode against a local mock; the TLS handshake
  to the real endpoint is untested until a deploy.

No client follows redirects, and each request has a 15-second timeout. Nothing else connects out.
Landlock does not restrict `connect`, so the outer unit's network rules are where the destinations
can be pinned.

## 5. seccomp

The `seccompiler` crate 0.5 (Apache-2.0 OR BSD-3-Clause) installs an allowlist. A syscall that is
not listed is `SECCOMP_RET_KILL_PROCESS`. `--sandbox-log` uses `SECCOMP_RET_LOG` instead: the
kernel audits the syscall and the process continues, which is how the list was built. The action
for a listed syscall stays allow. The filter is applied on the main thread before any other thread
exists. Installing it also sets no_new_privs; the explicit prctl still runs first.

x86_64 and aarch64 are supported. Another architecture logs
`sandbox: seccomp is not implemented for this architecture` and the summary says
`seccomp=unsupported`. A failure to install the filter is an error (`seccomp: ...`) and the
process exits.

`mmap` and `mprotect` are allowed only when the protection does not include `PROT_EXEC`. Mappings
made before the filter stay executable. A new executable mapping is killed, which blocks loading
a new shared object after startup.

These are not on the list: `execve`, `execveat`, `ptrace`, `mount`, `unshare`, `setns`, `bpf`,
`perf_event_open`, `keyctl`, `init_module`, `finit_module`, `delete_module`, `kexec_load`,
`kexec_file_load`, `reboot`, `swapon`, `swapoff`, `personality`. A unit test checks each one.

`prctl`, `clone`, `clone3` and `ioctl` are allowed with no argument check. `clone3` puts its flags
in a structure, so a namespace flag on that call cannot be filtered here; `unshare` and `setns`
are the calls that are refused. Device ioctl is refused by Landlock, which does not grant it.
`ioctl` itself stays, because the runtime issues it while the workers start.

The list is the syscalls a scripted login, a mailbox assignment, a sent letter and a WebSocket
save upload used after the filter was installed, plus `exit`, `exit_group`, `rt_sigreturn`,
`restart_syscall`, `signalfd4` and the `timerfd` calls a short session did not reach. x86_64 also
allows the legacy names glibc still emits there (`access`, `arch_prctl`, `epoll_wait`, `mkdir`,
`poll`, `unlink`); aarch64 has no such constants. `readlink`, the `landlock_*` calls and `seccomp`
ran only while the jail was being built, so they are not allowed afterwards. On x86_64 the
allowlist is 104 numbers. On aarch64 it is the same calls without those six legacy names, 98 numbers.
`open`, `stat` and `dup2` are on neither list: both architectures allow `openat`, `newfstatat` and
`dup3`.

Sign-in added no syscall. The HTTP endpoints share the WebSocket listener, the outbound calls in
section 4 use `socket`, `connect`, `sendto` and `recvfrom` that the list already had, and the
vault is read before the filter exists. `crates/innkeeperd/tests/signin.rs` runs the whole sign-in
contract against a sandboxed `--public` server in kill mode, and a public provider was reached
through DNS and TLS from a jailed process without a `SIGSYS`.

### 5.1 systemd notify and watchdog

With `Type=notify` and `WatchdogSec=`, systemd sets `NOTIFY_SOCKET` and `WATCHDOG_USEC`. The daemon
sends `READY=1` after the jail is on, `WATCHDOG=1` every half period, and `STOPPING=1` on Ctrl-C (SIGINT; SIGTERM ends the
process as before). It uses a unix datagram socket from the standard library. That needs `socket`,
`sendto` and `sendmsg`, which the allowlist already has, so no syscall was added. Landlock (ABI 5 is
what the ruleset handles) does not restrict `connect` or `sendto` to a unix socket, path or abstract:
only the ABI 6 scopes and ABI 9 rules do, and the ruleset asks for neither. `tests/metrics.rs` runs
the jailed daemon in seccomp kill mode against a path socket and checks `READY=1` and `WATCHDOG=1`.
The abstract form (`@name`) is covered by a unit test outside the jail.

The watchdog task proves two things: the runtime is turning (the task itself runs) and the database
answers (a read of one account on a blocking thread, within half a period). A wedged database or a
stalled runtime stops the pings and systemd restarts the unit. It does not prove the accept loops are
alive: probing them would mean opening a connection and so a sealed record. Without the two variables
the whole thing is a no-op.

## 6. Debugging

`--sandbox-log` is the way to extend the list. Run a session, then read the audit log (or
`strace -f`) for the syscalls that were not listed. Add a call only when a real session needs it.

A kill-mode death is signal 31 (`SIGSYS`). The syscall in flight at that stop is the one the
filter refused.

`--sandbox-probe PATH` is hidden. After the jail is installed it opens `PATH`, prints
`sandbox-probe: denied` or `sandbox-probe: opened`, and exits. The integration test uses it to
show that a file outside `--data-dir` cannot be read. The ruleset can also be built in a unit
test without calling `restrict_self`, so a machine that is running this server is not jailed by
`cargo test`.

The integration test is Linux-only. It returns early, with a skip line, when the kernel has no
Landlock or when seccomp fails to install, so `cargo test --workspace` stays green there. A
`SIGSYS` during the scripted login is a failure.

## 7. Crates

Direct dependencies of the Linux build, also recorded in [privacy.md](privacy.md) section 5:
`landlock` 0.4 (MIT OR Apache-2.0), `seccompiler` 0.5 (Apache-2.0 OR BSD-3-Clause), `prctl` 1
(MIT), `libc` 0.2 (MIT OR Apache-2.0). `prctl` pulls `nix`; the server does not call it. The
crates sign-in and the vault client add, with their licences, are listed in the same section of
privacy.md.

## 9. The admin portal listener

`--admin-bind` is a second TCP listener for the operator portal. Under `--public` it must be loopback
(the proxy routes `/admin/*` to it), must differ from `--ws-bind`, and `--admin-dev-open` is refused.
The infra unit needs the flag `--admin-bind 127.0.0.1:2317`, a source of staff keys (the vault
secret `admin-identities`, or `--admin-allowlist FILE`) and a Caddy `/admin/*` route.

- Landlock: `--admin-allowlist` grants read-only access (file read, directory read) on **the
  allowlist file's directory**, so edits and replacements by rename are seen. Nothing else is added.
- seccomp: the end-to-end test drives the portal under `--public` with the filter in kill mode and
  needed **no new syscalls**.

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
proxy on the same machine (Caddy). A raw TCP connection cannot be tied to the future sign-in, so
none is opened.

- An explicit `--bind` is an error (`public mode accepts players only over the WebSocket, so there
  is no raw TCP listener`). The default address is not bound.
- `--ws-bind` is required, and it must be `127.0.0.0/8` or `::1`. `0.0.0.0`, `::` and an
  IPv4-mapped address such as `::ffff:127.0.0.1` are refused. TLS is the proxy's job.
- An explicit `--int14h-bind` is an error. `--no-int14h` is redundant and accepted. The INT 14h
  listener stays off.
- `--line` is accepted. It does not open a socket.
- `--allow-root` is an error, including together with `--no-sandbox`.
- `--capture-dir` is still an error ([privacy.md](privacy.md) section 2).

Until sign-in exists, startup logs this warning once:

```
public mode: sign-in is not implemented yet, so WebSocket connections are not authenticated; do not expose this server publicly until it is
```

`connection_gate::admit` (`crates/innkeeperd/src/connection_gate.rs`) runs when a WebSocket is
accepted, before the HTTP upgrade. It admits every socket today. Sign-in will require and validate
the session token there, and a refusal is logged and the socket is dropped.

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
config is parsed, `--data-dir` and the database are open, listening sockets are bound, and (when
the vault client exists) secrets have been fetched. The accept loop starts after that. Tokio's
worker threads are created then, so they inherit the jail. Landlock before ABI v8 restricts the
calling thread only; `all_threads` is requested and best effort drops it on older kernels.
Inheritance is why the runtime is built after `confine`.

Offline commands return first and stay unrestricted: `account` (including `account export`),
`audit`, `hold` and `release`. They read a database the server has already closed.

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
outbound TCP for Key Vault and IMDS stays allowed. On a Landlock ABI 7 kernel, a connect to a
closed port still failed with `ECONNREFUSED`, while a bind of a port that was not listed failed
with `EACCES`. If a later kernel starts denying the implicit bind inside `connect`, the fix is a
port-0 rule, and it has to be retested before it is added.

A kernel with no Landlock logs one warning and continues, in public mode as well:
`sandbox: this kernel has no Landlock; filesystem and TCP bind stay unrestricted`. The summary
then says `abi=unsupported fs=open net=unrestricted`. That is a warning, not a startup error.

`localhost` is resolved once before the jail so glibc's files resolver is already mapped. An
external name is not resolved here. A later `dlopen` of a DNS module would need a new executable
mapping, which section 5 refuses. The vault client is not built, so that lookup does not happen yet.

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
allowlist is 104 numbers.

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
(MIT), `libc` 0.2 (MIT OR Apache-2.0). `prctl` pulls `nix`; the server does not call it.

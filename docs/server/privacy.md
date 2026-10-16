# Privacy

How `innkeeperd` treats personal data when it is hosted on a public VM. The default (no `--public`)
is unchanged so development tools that parse verbose logs keep working. Sealed connection records
are the one addition that is written in every mode, and only when an audit recipient is configured.

## 1. What is stored

The SQLite file is `DIR/innkeeper.db` (`--data-dir DIR`). A row that a player or the operator deletes
stays in the file until the retention job in section 6 hard-purges it. Schema:
`crates/innkeeperd/src/sqlite_store.rs` (`user_version` 1 adds `deleted_at` and the audit tables).

| Table | Columns | Why | How long |
|---|---|---|---|
| `accounts` | `id`, encoded `password` (10 bytes as the client sent them), `persona` (the Login name), `user_flags`, `rating`, `status`, `password_expired`, `deleted_at` | Logon, command 44, signup. The password is reversible obfuscation, not a hash | Soft-deleted on `account delete`; hard-purged after the account window (section 6) |
| `mailboxes` | `number` (stable from 1), `account`, `deleted_at` | Command 45 assigns one box per account | With the account. A box with letters still in it waits for those letters (section 6) |
| `letters` | `id`, `mailbox`, `body` (six time bytes then the client's envelope: names, subject, text), `deleted_at` | Mail (command 37). The store does not look inside | Hidden at the player's delete; hard-purged after the mail window |
| `posts` | `id`, `board`, `body` (`author\|text` as the RPG client wrote it), `deleted_at`, `created_at` | Bulletin board (command 27) | Hidden when deleted; hard-purged after the post window. Complaints use `created_at` (section 6) |
| `money` | `account`, `amount`, `deleted_at` | CasinoLand bankroll (command 40) | With the account |
| `saves` | `account`, `path`, `version`, `hash`, `data`, `updated_at`, `current`, `deleted_at` | Browser game files ([saves.md](saves.md)) | With the account. The channel still keeps three versions per path and drops older versions at once |
| `audit_sessions` | `id`, `started_at`, `ended_at`, `account_id`, `identity_key`, `hold_until`, `sealed` | Lawful-request record of a connection (section 5) | From creation, for `--audit-retention-days` (default 90) |
| `holds` | `account`, `hold_until` | A preservation request covering that account's content and sealed rows | Until `release`, or until the date passes |

`deleted_at` and the timestamps are unix seconds. `identity_key` is reserved for the future sign-in
HMAC and is written NULL today.

A 40/4 persona rename is session-only and is not written to `accounts`. Board posts are attributed by
matching the stored Login persona to the `author` prefix, case-insensitive; a renamed session can leave
a post the delete command will not find. An empty persona matches no posts.

Readable IP addresses and message contents are not written to logs or to clear columns. The peer
address of a connection is inside `audit_sessions.sealed` only (section 5). The planned sign-in table
is described in [PLAN.md](../PLAN.md): routine lookups use an HMAC, and the raw issuer and subject
are kept only as a sealed copy.

## 2. Public mode (`innkeeperd --public`)

Opt-in. The default log and capture format stay exactly as they are.

- **No capture.** `--capture-dir` given on the command line is an error. `--no-capture` is implied.
- **No message contents.** `step=client Login` / `step=host LoginAccepted`: the kind only, no fields,
  no hex dump. The raw line `client message` keeps `len=N` and drops the bytes.
- **No IP addresses in logs.** The `peer=` field in spans and lines is the first eight hex characters of
  HMAC-SHA256 over the IP (not the port), keyed by a random per-process secret. Sessions correlate
  within one run and are not reversible or stable across restarts.
- **No free text.** Persona names in info/warn lines become the account number. PAD commands, dial
  strings and AT lines log the event kind only.
- **Sealed records.** With `--public` and no `--audit-public-key`, startup logs one warning that no
  sealed connection records are kept, and the server still starts. A file key is accepted until the
  vault client in section 7 exists, and it is logged as a development key.

`LogPolicy` in `innkeeper-world` and `PeerTokens` in `innkeeperd` are the formatters; call sites do
not branch on `--public` by hand.

The same flag is also the only way in. A public server binds no raw TCP listener and no INT 14h
listener; `--ws-bind` is required and must be a loopback address, because a TLS proxy on this host
terminates TLS. Connections are not authenticated until sign-in exists, and startup says so.
[hardening.md](hardening.md) is that ingress rule and the in-process sandbox `--public` turns on.

## 3. Export, delete and restore

Offline, against `--data-dir`. Stop the server first: `serve` writes `DIR/innkeeper.lock` with its
pid, and the tools refuse while that pid is alive. A leftover file from a crash is ignored if the pid
is gone. `audit open`, `audit dev-keygen` and `audit seal` do not open the database.

```
innkeeperd account export <id> --data-dir DIR [--with-saves]
innkeeperd account delete <id> --data-dir DIR [--yes] [--purge-now]
innkeeperd account restore <id> --data-dir DIR
innkeeperd hold --account <id> --until YYYY-MM-DD --data-dir DIR
innkeeperd release --account <id> --data-dir DIR
```

`audit hold` and `audit release` are the same two commands.

**Export** prints one JSON document on stdout. Access requests cover data still held, so soft-deleted
items are included and each carries `"deleted_at"` (null while the row is live):

- the account row, with `"encoded_password": "omitted: the encoded password is not exported"` and no
  password bytes, plus `deleted_at`;
- mailbox number, letters received (that mailbox) and letters sent (other mailboxes whose envelope
  `from` is this mailbox), each with `deleted_at`;
- posts whose `author` matches the stored persona, each with `deleted_at`;
- `money` as a number, and `money_deleted_at` beside it;
- save file list, sizes, hashes, versions and `deleted_at`; `--with-saves` adds `data_base64`.

**Delete** without `--yes` prints the plan (live letters and posts only) and exits 1. With `--yes` the
account is disabled at once: it cannot log on, and its number is not issued again. The same
`deleted_at` is written on the account, its mailbox, letters in that box, letters it sent, posts
attributed to its stored persona, its money and its saves. The command prints the UTC purge date
(`deleted_at` plus `--retain-account-days`). A second delete prints that date and leaves the stamp
as it was. Sealed connection records are kept, held or not, and age out on their own clock.
Someone who abuses the service and then deletes the account does not erase that trail.

**Restore** clears `deleted_at` on the account and on owned rows that share the account's stamp, so a
letter the player deleted earlier stays deleted. It succeeds only while `now` is still inside the
account window. After that the command reports that the window has closed.

**`--purge-now`** also requires `--yes`. It hard-deletes the account and everything it owns, and it
refuses, even on the dry run, while a hold applies to that account. Sealed connection records stay.

A soft-deleted account's Login is the same reply as an unknown account (`This account is not known
here.`). Open enrolment does not create a new row while the old one exists, and signup's next number
is one past the highest id, counting soft-deleted rows.

## 4. Recommended VM settings

- **journald:** bound retention, e.g. `SystemMaxUse=50M` and `MaxRetentionSec=7day`, so info logs
  (even in public mode) do not keep peer tokens forever.
- **Reverse proxy:** no access logs. Nginx `access_log off;` on the WebSocket and TCP upgrade
  location; Caddy `log` off for that site. The proxy otherwise records client IPs that `--public`
  just hashed.
- **Backups:** the live database is the only long store. Copy it to Azure Blob Storage, private
  access only, encrypted at rest. A lifecycle management rule deletes each blob after 30 days, so
  data that the purge job has removed leaves backups within 30 days of that purge. Complaints (a
  year) and legal holds live in the database, so backups do not need a longer rule and the archive
  tier is unnecessary: that tier also charges for deletion before 180 days. A preservation request
  that needs one backup uses a blob legal hold (immutability) on that blob. Database backups never
  contain vault secrets.
- Run the public host with `--public` and `--data-dir` on encrypted disk. `--public` also jails
  the process ([hardening.md](hardening.md)); `--no-sandbox` is the opt-out. The VM's systemd unit
  is a separate layer and lives in the infra repo.

## 5. Sealed connection records

The operator can answer a valid law-enforcement request about serious abuse without keeping a
readable IP on the server. Each connection, when a recipient is configured, gets one row. With no
recipient, nothing is written. The running server and anyone who copies the database can see
`account_id` and the timestamps, and cannot read the rest.

On open the row's `sealed` blob is the envelope of this JSON:

```
{peer_ip, peer_port, transport: "tcp"|"websocket", started_at, ended_at, account_id}
```

`transport` is `tcp` for a legacy line and for INT 14h, and `websocket` for a WebSocket line. The row
is written when the connection opens (`account_id` and `ended_at` null). It is sealed again when
Login is accepted and when the connection ends. `identity_key` stays NULL until sign-in exists. A
failure to seal is a warning without the IP, and the connection continues.

**Who can read them.** Only the holder of the operator private key, and that key is not on the
server (section 7). Decryption is only for specific accounts named in valid legal process, or for a
serious abuse investigation.

**Envelope.** A fresh 256-bit data key encrypts the JSON with AES-256-GCM (`aes-gcm` 0.10) and a
random 96-bit nonce. The data key is wrapped with RSA-OAEP-256 (SHA-256, MGF1-SHA-256) under the
operator RSA public key (`rsa` 0.9). Both crates are MIT OR Apache-2.0. The in-process sandbox
([hardening.md](hardening.md)) adds, on Linux only, `landlock` 0.4 (MIT OR Apache-2.0),
`seccompiler` 0.5 (Apache-2.0 OR BSD-3-Clause), `prctl` 1 (MIT) and `libc` 0.2 (MIT OR
Apache-2.0). There is no separate third-party licence list in this repo; this paragraph is the
record. The server only encrypts with
`rsa`, so that crate's decryption-timing advisory does not apply to the running process. The offline
`--private-key` path does decrypt and is for local development only.

The blob is JSON, version 1: `{v, kid, wrapped_key, nonce, ciphertext}`. The three binary fields are
standard base64. The ciphertext includes the GCM tag. `kid` is the key identifier, so a rotated key
can still be found. In production `kid` is the escrow vault's key version URL.

**Flags.** `--audit-public-key FILE` is an SPKI PEM, checked when the server starts, and
`--audit-key-id STRING` is required with it. `--audit-retention-days` defaults to 90. A key shorter
than 2048 bits is refused; a key shorter than 3072 bits is a warning. The production key is 3072
bits or larger. Loading the public key from the server vault is later work (section 7); a file is
enough until then, and every file load logs `DEVELOPMENT KEY`. A key with no `--data-dir` logs a
warning and writes nothing.

**Purge.** At startup, and then every hour, rows whose `started_at` is older than the retention are
deleted, unless `hold_until` is still in the future or a `holds` row covers that account. The job
logs counts only. It runs whenever `--data-dir` is open, including when no recipient is set.

**Holds.** `innkeeperd hold --account ID --until YYYY-MM-DD` is the preservation request. The named
day is included: the stored instant is 00:00:00 UTC on the next day. A later date extends a hold; an
earlier date does not shorten it. A date already past is refused. The hold covers that account's
sealed rows and its content (section 6). `release` clears it. A hold may be placed after the account
row is gone. A connection that logs in while a hold is active copies `hold_until` onto its sealed
row. A row that has not logged in yet has a null `account_id`, so an account hold does not see it
until Login.

**Export and offline decryption.**

```
innkeeperd audit export --account <id> [--since DATE] [--until DATE] --data-dir DIR
innkeeperd audit open --in rows.jsonl (--private-key dev.pem | --data-keys keys.jsonl)
innkeeperd audit dev-keygen --out-dir DIR
innkeeperd audit seal --public-key PEM --key-id KID --record JSON
```

Export writes JSON lines `{id, started_at, ended_at, account_id, kid, wrapped_key_b64, sealed_b64}`.
`--since` is inclusive of that UTC date; `--until` stops at the following midnight. `open` prints the
decrypted JSON, one line per row. `--private-key` is labelled development-only on stderr. Production
unwraps each `wrapped_key` with the escrow vault under the operator's own Azure login, then passes
the data keys:

```
{"id": 1, "data_key_b64": "...", "wrapped_key_b64": "..."}
```

`tools/audit_unwrap.sh` runs that loop. A tampered ciphertext fails authentication. `dev-keygen`
writes a 3072-bit pair, `audit-public.pem` and `audit-private.pem` (mode 0600), and refuses to
overwrite. `audit seal` exists so `tools/audit_roundtrip.sh` can seal one record with `innkeeperd`
and `az` alone.

`account delete` leaves these rows in place.

## 6. Retention

Every number is a field of `RetentionPolicy` (`innkeeper-world`) and a flag. Defaults:

| What | When it becomes deletable | Hard purge after | Flag | Why |
|---|---|---|---|---|
| A letter or mail item the player deletes | at the player's delete | 90 days | `--retain-mail-days` | grace for abuse reports; the 90-day preservation period of 18 U.S.C. 2703(f) |
| A bulletin post deleted by its author or the operator | at delete | 90 days | `--retain-post-days` | same |
| An account deleted on request | at the request; the account is disabled at once | 30 days | `--retain-account-days` | erasure without undue delay (GDPR one month), long enough to undo abuse of the deletion itself |
| Complaints on the operator board | on arrival | 365 days | `--retain-complaint-days` | reports are evidence; one year matches the CyberTipline preservation period (REPORT Act 2024) |
| Sealed connection records | on creation | 90 days | `--audit-retention-days` | section 5 |
| Expired session tokens (future sign-in) | at expiry | 30 days | `--retain-session-days` | the `sessions` table is not created yet; the job purges it once the table has `expiry` and `account_id` |
| Anything under a legal hold | never while held | when the hold ends, if the clock above has already run out | `hold` / `release` | preservation requests. The clock is counted from the delete or from creation and is not restarted when the hold ends |

Players see a deleted letter or post as gone: every read skips `deleted_at`. The only user-facing
delete in the protocol today is mail sub-command 26. Command 27 has no delete sub-command on this
host (27/6, 27/8 and 27/9 are unsupported). `Store::delete_post` is there for the operator path, and
`account delete` soft-deletes posts whose `author` is the stored persona.

Complaints are posts on board location `0x63` (`COMPLAINT_BOARD_LOCATION`), any land. That is the
operator-only complaint board the RPG client already lists. LSCI 27/9 is not a stored complaint
command. Those posts are purged from `created_at` even while they are still visible, and they are
left out of the 90-day deleted-post purge. A hold on the author's account keeps them.

The account window and the letter window are different lengths, and `letters.mailbox` references
`mailboxes`, which reference `accounts`. When the account is due, money and saves are removed. The
account and its mailbox stay, still soft-deleted, while any letter row points at the mailbox. The
next purge removes the mailbox and the account once those letters are gone. During that wait the
account still cannot log on and its number is still not reused. The password is left in place for
the restore window, which is the same 30 days.

The purge job runs at startup and hourly. It logs one line of counts (audit, letters, posts,
complaints, accounts, mailboxes, money, saves, sessions) and no contents. A letter is kept when the
mailbox owner or the sending account is held. A post is kept when any account with that persona,
including a soft-deleted one, is held.

Backups rotate within 30 days (section 4) so a hard-purged row leaves backups within 30 days of
its purge.

## 7. Key handling

Two Azure Key Vaults. The vault clients are not built yet; this is the plan the server will follow,
also written under Sign-in in [PLAN.md](../PLAN.md).

**Server vault.** innkeeperd's managed identity on Azure, or a service principal when self-hosted,
has the Key Vault Secrets User role here and nothing else. It holds the identity HMAC key, the OIDC
client secret for each provider, future server-side keys, and a copy of the audit public key as a
secret together with its key identifier. Default secret names: `audit-public-key` (SPKI PEM) and
`audit-key-id` (the escrow key version URL).

**Escrow vault.** The server's managed identity and its service principal have no role assignment
on this vault. It holds the audit key as a non-exportable RSA key, RSA-HSM when the SKU allows,
3072-bit or larger. Only the operator's own Entra account may use it, ideally with just-in-time
elevation (PIM). Key Vault diagnostic logging is on, so every unwrap is an entry in those logs.
Opening a record means asking this vault to unwrap the data key:

```
az keyvault key decrypt --algorithm RSA-OAEP-256 \
  --vault-name <escrow> --name <key> --value <wrapped_b64>
```

`tools/audit_unwrap.sh --vault-name ESCROW --key-name KEY` reads export lines, adds `--version`
when `kid` is a key version URL, and writes `audit open --data-keys` lines. It accepts `result`
or `plaintext` from the CLI's JSON.

**Creating the keys.** Infrastructure as code (Bicep or Terraform) creates both vaults and the RSA
key inside the escrow vault. The private key is generated in the vault and never exists anywhere
else, so it cannot leak through IaC state. The same deployment reads the key resource's public JWK,
writes the PEM and the key version URL into the server vault, and ends with a round trip.
`tools/audit_roundtrip.sh --server-vault NAME --escrow-vault NAME --key-name KEY` is that check: it
reads the two secrets, seals a probe with `innkeeperd audit seal`, unwraps with the escrow vault
using the operator's `az` login, opens the row, and compares the plaintext to the probe. It needs
`az` and `innkeeperd` (`INNKEEPERD` overrides the binary). Optional `--public-secret` and
`--kid-secret` override the default secret names.

**Authentication, when the client lands.** innkeeperd reaches the server vault with the VM's managed
identity, or with a service principal from the environment when self-hosted. The intended crates are
`azure_identity` and `azure_security_keyvault_secrets` (MIT); the Key Vault REST API is the
alternative if those crates do not fit. Configuration will be `--key-vault https://name.vault.azure.net/`
plus the secret names above. That flag is not accepted today. When the vault client lands, `--public`
will refuse to start without `--key-vault`. Until then a PEM file (`--audit-public-key`) is accepted
and logged as a development key.

**Local development uses no vault.** The identity HMAC key will come from `dev-identity.key` (mode
0600) generated in `--data-dir` once sign-in exists; that file is not created yet. The audit keypair
is `innkeeperd audit dev-keygen --out-dir DIR`. Either file path logs `DEVELOPMENT KEY`.

**Runtime.** Secrets are fetched at startup and held in memory only, never logged and never written
to disk. Rotation is a restart, or a periodic refresh of the latest secret versions.

**Rotation.** Each identity row records the HMAC key version that produced it. At sign-in the server
tries the current version first, then older enabled versions, and on a match with an old version
rewrites the row under the current version while the raw subject is in hand. Old versions are
disabled after a window. A new audit RSA key version means new records carry the new `kid`; old
versions stay enabled until the records that name them age out.

**Vault protection.** Soft delete and purge protection are on for both vaults.

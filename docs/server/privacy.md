# Privacy

How `innkeeperd` treats personal data when it is hosted on a public VM. The default (no `--public`)
is unchanged so development tools that parse verbose logs keep working. Sealed connection records
are the one addition that is written in every mode, and only when an audit recipient is configured.

## 1. What is stored

The SQLite file is `DIR/innkeeper.db` (`--data-dir DIR`). A row that a player or the operator deletes
stays in the file until the retention job in section 6 hard-purges it. The schema is the baseline of
[store.md](store.md) section 3 (`crates/innkeeperd/src/sqlite_store.rs`, version 5): every table is
STRICT, every column that names another row is a foreign key, and every time is a `*_at` column of
Unix milliseconds UTC.

| Table | Columns | Why | How long |
|---|---|---|---|
| `accounts` | `id`, encoded `password` (10 bytes as the client sent them), `persona` (the Login name), `user_flags`, `rating`, `status`, `password_expired`, `created_at`, `deleted_at`, `suspended_at`, `purged_at` | Logon, command 44, signup. The password is reversible obfuscation, not a hash | Soft-deleted on `account delete`; after the account window (section 6) the row becomes a tombstone: every personal column is cleared and only the number and three dates stay |
| `mailboxes` | `number` (stable from 1), `account`, `created_at`, `deleted_at` | Command 45 assigns one box per account | With the account. A box with letters still in it waits for those letters (section 6) |
| `letters` | `id`, `to_mailbox`, `from_account` (the account whose session sent it), `body` (six time bytes then the client's envelope: names, subject, text), `created_at`, `deleted_at` | Mail (command 37). The store does not look inside | Hidden at the player's delete; hard-purged after the mail window |
| `posts` | `id`, `board`, `kind` (derived from `board`), `account` (the account that wrote it), `about_account`, `body` (`author\|text` as the RPG client wrote it), `created_at`, `deleted_at`, `handled_at`, `handled_by` | Bulletin board (command 27) | Hidden when deleted; hard-purged after the post window. Complaints use `created_at` (section 6) |
| `posts` (Member Services) | as `posts`, kind `member_services`, on board location `0x63`, section 2, land 0: `persona\|Member Services: text` | A question a player types into Town Hall "Ask Member Services" is a message to the operator ([services.md](services.md) section 5) | Complaint retention, 365 days from arrival (section 6) |
| `posts` (complaint) | as `posts`, kind `complaint`, on board location `0x63`, section 3, land 0: `persona\|Complaint: text`, and `about_account`, the account the complaint names when the host could tell (section 6) | The hub's "Complain" form and the Sketch Pad's complaint ([hub-services.md](hub-services.md) section 2) | Complaint retention, 365 days from arrival (section 6); in the export and the delete of its author |
| `posts` (topics) | as `posts`, kind `topic`, land 0, board location 0 sections 67 and 84: the conference and trivia topic text a moderator wrote, with no author prefix | Not personal data; one message each, replaced on a write ([hub-services.md](hub-services.md) section 5) | Post window |
| `money` | `account`, `amount`, `deleted_at` | CasinoLand bankroll (command 40) | With the account |
| `saves` | `account`, `path`, `version`, `hash`, `data`, `modified_at`, `current`, `deleted_at` | Browser game files ([saves.md](saves.md)) | With the account. The channel still keeps three versions per path and drops older versions at once |
| `audit_sessions` | `id`, `started_at`, `ended_at`, `account`, `identity_key`, `sealed` | Lawful-request record of a connection (section 5) | From creation, for `--audit-retention-days` (default 90) |
| `holds` | `account`, `placed_at`, `expires_at` | A preservation request covering that account's content and sealed rows | Until `release`, or until the date passes |
| `identities` | `identity_key` (HMAC-SHA256 of issuer, a zero byte and subject), `key_version`, `subject_sealed`, `provider` (the auth file's id), `adult_attested_at`, `attested_age`, `terms_version`, `created_at`, `deleted_at` | Sign-in ([innkeeperd.md](innkeeperd.md) section 8): finds the identity again without keeping who it is; the age and terms answer is checked before an account or password is issued | Stamped when its last account is deleted; hard-purged once it owns no account and has no session row |
| `identity_accounts` | `account`, `identity_key`, `created_at` | Which accounts an identity owns, and the per-day account limit | With the account |
| `sessions` | `token_hash` (SHA-256 of the token), `identity_key`, `account` (the account the line logged on to), `created_at`, `expires_at` | A WebSocket is bound to the identity | Logout and account deletion set `expires_at` to now; hard-purged 30 days after `expires_at` (section 6) |
| `operators`, `staff`, `admin_sessions`, `admin_audit` | the portal's own tables (section 8) | Who may use the portal and what they did | Section 6 |

`audit_sessions.identity_key` is the signed-in identity's HMAC, NULL for a line without a session
or once that identity has been purged.

**A tombstone holds no personal data.** A purged account keeps its row so that the sealed records,
complaints and audit entries that must outlive it still name a valid account, and so that its number
is never issued again. The row keeps `id`, `created_at`, `deleted_at` and `purged_at` only: the
password, persona, flags, rating, status and suspension are NULL, a CHECK keeps them NULL, and a
trigger refuses any later write to the row ([store.md](store.md) section 3.4).

`accounts.persona` is a pseudonym the player typed (a name in the client's Select Player screen, or one
given when the account was opened); sign-in itself stores no name. It is empty for a web account until the
client's first 40/4, which writes the name the player picked (the last one wins). The server keeps a name
only when it is 1 to 16 printable ASCII characters with no `|`; it is part of the account's export and is
erased with the account like the row. Names are not unique, as in the original, so **nothing is
attributed by name**. Every post records the account that wrote it (`posts.account`): board posts
(27/0), the hub's complaints (27/9), Town Hall "Ask Member Services" messages (37/30) and the
conference and trivia topic text. Every letter records the account that sent it
(`letters.from_account`); the envelope's `from` mailbox and names are the client's bytes and are not
used to find an account. The account export, `account delete` and its restore, the retention purge
and its legal holds, the operator portal's links and the bulletin board's "erase your own message"
all use those columns, so two accounts that share a persona never see, hide, erase or hold each
other's posts or letters. There is no fallback to the name at the head of a post body.

`Envelope`'s `Debug` prints the text length and not the text, so a letter or a Town Hall form never appears in a log line, in any mode. (The raw hex line of the verbose default mode still carries message bytes; `--public` drops them.)

Readable IP addresses and message contents are not written to logs or to clear columns. The peer
address of a connection is inside `audit_sessions.sealed` only (section 5).

**Sign-in stores no personal profile.** The provider is asked for `openid` only (a plain-OAuth
provider for the minimum scope that returns its user id), and no email, name, username, avatar or
access token is kept: the plain-OAuth profile answer is read for its id field and dropped. Routine
lookups use `identity_key` only. The raw issuer and subject exist in the database only inside
`subject_sealed`, sealed with the audit envelope of section 5 as
`{"provider", "issuer", "subject"}`, so the operator can name the provider account
behind one account under legal process. Without an audit recipient `subject_sealed` is NULL and
startup warns. The player sees a handle such as `Caller 7F3A`, the first two bytes of the key. The
session token itself is never stored or logged; the table keeps its SHA-256. The age and terms
answer is a self-declaration and is stored as given. A login in flight is kept nowhere: its PKCE
verifier, nonce and page return travel inside the `state` value, encrypted under a key that exists
only in the process's memory, and expire after ten minutes. The only thing remembered is which
states have already come back, until each expires ([innkeeperd.md](innkeeperd.md) section 8.2).

## 2. Public mode (`innkeeperd --public`)

Opt-in. The default log and capture format stay exactly as they are.

- **No capture.** `--capture-dir` given on the command line is an error. `--no-capture` is implied.
- **No message contents.** `step=client Login` / `step=host LoginAccepted`: the kind only, no fields,
  no hex dump. The raw line `client message` keeps `len=N` and drops the bytes.
- **No IP addresses in logs.** The `peer=` field in spans and lines is the first eight hex characters of
  HMAC-SHA256 over the IP (not the port), keyed by a random per-process secret. Sessions correlate
  within one run and are not reversible or stable across restarts. On a WebSocket the IP is the
  client's, the right-most `X-Forwarded-For` entry of the loopback proxy, not the proxy's
  ([hardening.md](hardening.md) section 1).
- **No free text.** Persona names in info/warn lines become the account number. PAD commands, dial
  strings and AT lines log the event kind only.
- **Sealed records.** With `--key-vault` the audit key is read from the server vault (section 7),
  so a public server always has one. With `--public --insecure-dev-keys` and no
  `--audit-public-key`, startup logs one warning that no sealed connection records are kept, and
  the server still starts. A file key is logged as a development key.

`LogPolicy` in `innkeeper-world` and `PeerTokens` in `innkeeperd` are the formatters; call sites do
not branch on `--public` by hand.

The same flag is also the only way in. A public server binds no raw TCP listener and no INT 14h
listener; `--ws-bind` is required and must be a loopback address, because a TLS proxy on this host
terminates TLS. Every WebSocket must carry a live sign-in session, or the upgrade is a 401
([innkeeperd.md](innkeeperd.md) section 8).
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
- mailbox number, letters received (that mailbox) and letters sent (by `from_account`, to other
  mailboxes), each with `created_at` and `deleted_at`;
- posts written by the account (`posts.account`), each with its `kind`, `created_at` and
  `deleted_at`;
- `money` as a number, and `money_deleted_at` beside it;
- save file list, sizes, hashes, versions and `deleted_at`; `--with-saves` adds `data_base64`;
- `identity`, when the account was made through sign-in: the handle, `identity_key_hex`, provider id,
  key version, the age and terms answer, `created_at`, `deleted_at`, every account the identity
  owns, and its sessions (`account_id`, `created_at`, `expiry`, never the token hash). The subject
  is named as sealed, not exported: `audit export` carries it for the escrow key (section 5).

A purged account exports as its tombstone: `id`, `created_at`, `deleted_at` and `purged_at`, and
whatever rows still wait out their own windows.

**Delete** without `--yes` prints the plan (live letters and posts only) and exits 1. With `--yes` the
account is disabled at once: it cannot log on, and its number is not issued again. The same
`deleted_at` is written on the account, its mailbox, letters in that box, letters it sent, posts
written by the account, its money and its saves. The command prints the UTC purge date
(`deleted_at` plus `--retain-account-days`). A second delete prints that date and leaves the stamp
as it was. Sealed connection records are kept, held or not, and age out on their own clock.
Someone who abuses the service and then deletes the account does not erase that trail. When the
account was the identity's last live one, the identity is stamped too and its sessions end at once.

**Restore** clears `deleted_at` on the account and on owned rows that share the account's stamp, so a
letter the player deleted earlier stays deleted. An identity stamped by that delete is restored
with it; its ended sessions stay ended. It succeeds only while `now` is still inside the
account window. After that the command reports that the window has closed.

**`--purge-now`** also requires `--yes`. It hard-deletes what the account owns at once (its mailbox
and every letter in it or sent by it, its posts other than complaints and Member Services messages,
its money, its saves and its identity link) and leaves the account row as a tombstone (section 1).
When it was the identity's last live account, the identity is stamped and its sessions end, as on a
delete.
It refuses, even on the dry run, while a hold applies to that account. Sealed connection records,
complaints it wrote or that name it, and portal audit entries stay and age out on their own clocks.

A soft-deleted account's Login is the same reply as an unknown account (`This account is not known
here.`). Open enrolment does not create a new row over an old one, and signup's next number is one
past the highest id, counting soft-deleted rows and tombstones, so a number is never issued twice,
the highest one included.

## 4. Recommended VM settings

- **journald:** bound retention, e.g. `SystemMaxUse=50M` and `MaxRetentionSec=7day`, so info logs
  (even in public mode) do not keep peer tokens forever.
- **Reverse proxy:** no access logs. Nginx `access_log off;` on the WebSocket and TCP upgrade
  location; Caddy `log` off for that site. The proxy otherwise records client IPs that `--public`
  just hashed.
- **Backups:** the live database is the only long store. Copy it to Azure Blob Storage, encrypted at
  rest, with access by identity only (no Private Endpoints):
  - anonymous blob access and shared-key access are off, so Entra ID is the only way in;
  - the VM's managed identity has a role on the backup container that can read and write but not
    delete;
  - blob versioning and soft delete are on;
  - a free Microsoft.Storage service endpoint on the VM's subnet, plus a virtual-network rule,
    limits the network. The trusted-services exception does not cover VMs, and an IP rule for the
    VM's own public address does not match traffic from the same region.

  A lifecycle management rule removes each backup, and its versions, within about 30 days
  including soft delete (the arithmetic is in the infra repo's README). Data that the purge job
  has removed therefore leaves backups within about 30 days of that purge. Complaints (a
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
`account` and the timestamps, and cannot read the rest.

On open the row's `sealed` blob is the envelope of this JSON:

```
{peer_ip, peer_port, transport: "tcp"|"websocket", started_at, ended_at, account_id}
```

`peer_ip` is the client: on a WebSocket in public mode, the right-most `X-Forwarded-For` entry the
loopback proxy sent ([hardening.md](hardening.md) section 1). The proxy does not pass the client's
port, so `peer_port` is null there; it is the socket's port on a line that arrived directly.
An upgrade with no usable header is refused before a row is written, so no record names the proxy.
`started_at` and `ended_at` are Unix milliseconds, here and in the row's clear columns. The JSON key
stays `account_id`; the column is `account`. `transport` is `tcp` for a legacy line and for INT 14h,
and `websocket` for a WebSocket line. The row
is written when the connection opens (`account_id` and `ended_at` null). It is sealed again when
Login is accepted and when the connection ends. A line that carried a session token gets the
identity's HMAC in `identity_key`, in the clear like `account`, so a request about one identity
finds its connections. A failure to seal is a warning without the IP, and the connection continues.

**Who can read them.** Only the holder of the operator private key, and that key is not on the
server (section 7). Decryption is only for specific accounts named in valid legal process, or for a
serious abuse investigation.

**Envelope.** A fresh 256-bit data key encrypts the JSON with AES-256-GCM (`aes-gcm` 0.10) and a
random 96-bit nonce. The data key is wrapped with RSA-OAEP-256 (SHA-256, MGF1-SHA-256) under the
operator RSA public key (`rsa` 0.9). Both crates are MIT OR Apache-2.0. The in-process sandbox
([hardening.md](hardening.md)) adds, on Linux only, `landlock` 0.4 (MIT OR Apache-2.0),
`seccompiler` 0.5 (Apache-2.0 OR BSD-3-Clause), `prctl` 1 (MIT) and `libc` 0.2 (MIT OR
Apache-2.0). Sign-in and the vault client ([innkeeperd.md](innkeeperd.md) section 8) add
`hyper` 1, `hyper-util` 0.1, `http-body-util` 0.1 and `bytes` 1 (MIT), `openidconnect` 4 (MIT),
`oauth2` 5, `reqwest` 0.12 with rustls and the hickory resolver, `url` 2, `serde` 1, `toml` 0.8,
`zeroize` 1 and `sha1` 0.10 (each MIT OR Apache-2.0). Their dependencies bring `rustls` (Apache-2.0
OR ISC OR MIT), `ring` (Apache-2.0 AND ISC), `rustls-webpki` and `untrusted` (ISC),
`curve25519-dalek` and `ed25519-dalek` (BSD-3-Clause, through `openidconnect`), `moka` ((MIT OR
Apache-2.0) AND Apache-2.0, through the resolver), `sync_wrapper` (Apache-2.0) and the ICU crates
under `url`'s `idna` (Unicode-3.0, a permissive licence). The rest are MIT or Apache-2.0, or offer one
of them. There is no separate third-party licence list in this repo; this paragraph is the
record. The server only encrypts with
`rsa`, so that crate's decryption-timing advisory does not apply to the running process. The offline
`--private-key` path does decrypt and is for local development only.

The blob is JSON, version 1: `{v, kid, wrapped_key, nonce, ciphertext}`. The three binary fields are
standard base64. The ciphertext includes the GCM tag. `kid` is the key identifier, so a rotated key
can still be found. In production `kid` is the escrow vault's key version URL.

**Flags.** `--audit-public-key FILE` is an SPKI PEM, checked when the server starts, and
`--audit-key-id STRING` is required with it. `--audit-retention-days` defaults to 90. A key shorter
than 2048 bits is refused; a key shorter than 3072 bits is a warning. The production key is 3072
bits or larger. With `--key-vault` the public key and its identifier are read from the server
vault instead (section 7), and the two flags are refused. A file load logs `DEVELOPMENT KEY`. A key with no `--data-dir` logs a
warning and writes nothing.

**Purge.** At startup, and then every hour, rows whose `started_at` is older than the retention are
deleted, unless a `holds` row covering that account has not expired. The job
logs counts only. It runs whenever `--data-dir` is open, including when no recipient is set.

**Holds.** `innkeeperd hold --account ID --until YYYY-MM-DD` is the preservation request. The named
day is included: the stored instant is 00:00:00 UTC on the next day. A later date extends a hold; an
earlier date does not shorten it. A date already past is refused. The hold covers that account's
sealed rows and its content (section 6). `release` clears it. A hold needs a number that was issued
(live, deleted or a tombstone); an unknown number is refused, since there is nothing to preserve. A
row that has not logged in yet has a null `account`, so an account hold does not see it until Login.

**Export and offline decryption.**

```
innkeeperd audit export --account <id> [--since DATE] [--until DATE] --data-dir DIR
innkeeperd audit open --in rows.jsonl (--private-key dev.pem | --data-keys keys.jsonl)
innkeeperd audit dev-keygen --out-dir DIR
innkeeperd audit seal --public-key PEM --key-id KID --record JSON
```

Export writes JSON lines `{id, started_at, ended_at, account_id, kid, wrapped_key_b64, sealed_b64}`.
When the account belongs to a sign-in identity with a sealed subject, one more line follows with
`"record": "identity"`, a null `id`, the provider and the same envelope fields; `open` prints the
provider, issuer and subject from it.
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
| Admin audit log (`admin_audit`) | at each portal action | 365 days | `--retain-admin-audit-days` | who did what to whom, with a reason and no content; a row naming an account under a legal hold is kept while the hold lasts |
| Admin sessions | at expiry or sign-out | 1 day after expiry | none | operator sign-in rows, hashed tokens only |
| Sealed connection records | on creation | 90 days | `--audit-retention-days` | section 5 |
| Sign-in sessions | at expiry, logout, or deletion of the identity's last account | 30 days after `expires_at` | `--retain-session-days` | a short trail for abuse on a line; held when the session's account, or any account of its identity, is held |
| Sign-in identities | when they own no account (the last one hard-purged) and have no session row | at the next purge, an hour after creation at the earliest | none | the identity exists only to own accounts; the hour covers a sign-in that has not created one yet |
| Anything under a legal hold | never while held | when the hold ends, if the clock above has already run out | `hold` / `release` | preservation requests. The clock is counted from the delete or from creation and is not restarted when the hold ends |

Players see a deleted letter or post as gone: every read skips `deleted_at`. The only user-facing
delete in the protocol today is mail sub-command 26. Command 27 has a delete sub-command, 27/6, from the
hub's bulletin board (own post or privileged reader only, no reply; [hub-services.md](hub-services.md) section 3). `Store::delete_post` is there for the operator path, and
`account delete` soft-deletes the posts the account wrote.

The Town Hall's "Update Account Information" form is free text about billing, so it may hold an address or a card number. It is acknowledged and discarded unread: nothing of it is stored, and only the account number and the fact that a form arrived are logged. The "Ask Member Services" text is kept as a complaint post (section 1) and is part of the account's export and delete (matched by the writing account).

Complaints are posts on board location `0x63` (`COMPLAINT_BOARD_LOCATION`), any land. That is the
operator-only complaint board the RPG client already lists. The hub's complaint form (27/9, or 27/0 with
board 0/99) is stored there too, as `persona|Complaint: text` on section 3, land 0, 2000 bytes at most, and
the log carries only the account, the post id, the length, whether a player was named and the account
resolved for them; the debug print of the request shows the length, never the text. The client sends a
connection-local number for the player complained about, which means nothing later, and names them in its
own text as `about: Name`. At filing time the host looks that name up among the players online: when exactly
one other account is playing under it, the complaint stores that account as `about_account`; otherwise
`about_account` is NULL and the name stays only in the text. Nothing resolves the name again later. Those posts are purged from `created_at` even while they are still visible, and they are
left out of the 90-day deleted-post purge. A hold on the author's account, or on the `about_account`, keeps them.

The account window and the letter window are different lengths, and `letters.to_mailbox` references
`mailboxes`, which reference `accounts`. When the account is due, its money, saves and identity link
are removed and the account becomes a tombstone. Its mailbox stays, still soft-deleted, while any
letter row points at it, and a later purge removes it once those letters are gone. The account row
itself is never removed (section 1), so its number is never reused. The password is left in place
for the restore window, which is the same 30 days; a tombstone cannot be restored.

The purge job runs at startup and hourly. It logs one line of counts (audit, letters, posts,
complaints, accounts, mailboxes, money, saves, sessions, identities, admin rows, operators) and no contents.
A letter is kept when the mailbox owner or the sending account (`from_account`) is held. A post is kept
when its author account is held, and a complaint also when its `about_account` is held. Holds are by
account number only: an account that shares a persona with a held one is not held.

Backups rotate within 30 days (section 4) so a hard-purged row leaves backups within 30 days of
its purge.

## 7. Key handling

Two Azure Key Vaults. The server half is built: `--key-vault` and the client are
[innkeeperd.md](innkeeperd.md) section 8.3. The design is also under Sign-in in
[PLAN.md](../PLAN.md).

**Server vault.** innkeeperd's managed identity on Azure, or a service principal when self-hosted,
has the Key Vault Secrets User role here and nothing else. It holds the identity HMAC key
(`identity-hmac-key`), the client secret for each provider (the auth file's
`client_secret_name`), future server-side keys, and a copy of the audit public key as a secret
together with its key identifier: `audit-public-key` (SPKI PEM) and `audit-key-id` (the escrow key
version URL).

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
`--kid-secret` override the default secret names. The deployment also writes `identity-hmac-key`
(32 random bytes or more, base64) and the provider client secrets.

**Authentication.** innkeeperd reaches the server vault with the VM's managed identity through
IMDS, or with a service principal from the environment when self-hosted (a client secret or a
certificate). The client is the Key Vault REST API over `reqwest` and rustls, not the
`azure_identity` and `azure_security_keyvault_secrets` crates: startup needs one token and a few
secret reads, and the SDK would add its own HTTP stack and a much larger dependency tree for that.
`--public` refuses to start without `--key-vault`, except with `--insecure-dev-keys` and loopback
page origins only (innkeeperd.md section 8.1).

**Local development uses no vault.** With `--insecure-dev-keys`, or without `--public`, the identity
HMAC key is `dev-identity.key` (32 random bytes in base64, mode 0600), generated in `--data-dir` on first use.
Its key version is the label `dev-identity.key`. Client secrets come from the auth file's
`[dev_secrets]` or `INNKEEPERD_SECRET_<NAME>`. The audit keypair is `innkeeperd audit dev-keygen
--out-dir DIR`. Each of these logs `DEVELOPMENT KEY`.

**Runtime.** Secrets are fetched once at startup, before the sandbox, and held in memory only,
zeroized on drop where the type allows. They are never logged and never written to disk. There is
no periodic refresh: rotation is a restart.

**Rotation.** Each identity row records the HMAC key version that produced it. Startup reads every
enabled version of `identity-hmac-key`, newest first. At sign-in the server tries the current
version first, then the older ones, and on a match with an old version rewrites the row, its
account links, its sessions and its sealed connection records under the current version while the
raw subject is in hand. Old versions are disabled after a window; an identity that has not signed in
by then is not found again and starts afresh. A new audit RSA key version means new records carry
the new `kid`; old versions stay enabled until the records that name them age out.

**Vault protection.** Soft delete and purge protection are on for both vaults.

## 7a. Metrics

`--app-insights-connection-string` sends a handful of numbers to Application Insights once a minute:
heartbeat, open lines, players logged in, and counts of logins, sign-ins, accounts opened, refused
WebSockets, store errors and client messages, plus the process's resident memory. They are counts
and sizes only. There are no dimensions or properties, so no account number, persona, address,
provider, session or message text goes with them, and nothing in them can single out a player. The
envelopes carry the role name `innkeeperd` and the instrumentation key, which is no secret (the VM's
managed identity is the credential). The portal's Status panel shows the same counters from memory.

## 8. Operator portal

The portal ([innkeeperd.md](innkeeperd.md) section 9) records each action in `admin_audit`: time, the
operator's identity key (the portal shows its handle; never the subject), action, target, the
account it concerns, and the typed reason. Letters are metadata
only until an operator types a reason of at least 8 characters to read one; the reason is stored
in the audit row before the body is shown. Bodies, subjects and tokens are not logged. A refused
sign-in logs the handle only. `operators` holds each identity key the portal has seen, the parent
of its rows; `staff` holds allowed or revoked identity keys with a note; the "handled" marks are
`handled_at` and `handled_by` on the complaint and Member Services posts themselves. The portal links
a complaint to its author's account and its `about_account`, never by a name. All are purged as in
section 6: an operator row goes once no staff row, cookie, audit entry or mark names it.

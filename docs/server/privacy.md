# Privacy

How `innkeeperd` treats personal data when it is hosted on a public VM. The default (no `--public`)
is unchanged so development tools that parse verbose logs keep working.

## 1. What is stored

The SQLite file is `DIR/innkeeper.db` (`--data-dir DIR`). There is no retention job: a row lives until
the account is deleted or the operator removes the file. Schema: `crates/innkeeperd/src/sqlite_store.rs`.

| Table | Columns | Why | How long |
|---|---|---|---|
| `accounts` | `id`, encoded `password` (10 bytes as the client sent them), `persona` (the Login name), `user_flags`, `rating`, `status`, `password_expired` | Logon, command 44, signup. The password is reversible obfuscation, not a hash | Until `account delete` |
| `mailboxes` | `number` (stable from 1), `account` | Command 45 assigns one box per account | Until `account delete` |
| `letters` | `id`, `mailbox`, `body` (six time bytes then the client's envelope: names, subject, text) | Mail (command 37). The store does not look inside | Until the owner is deleted; letters this account *sent* are deleted too, not kept for the recipient |
| `posts` | `id`, `board`, `body` (`author\|text` as the RPG client wrote it) | Bulletin board (command 27) | Until `account delete` of the stored persona that matches `author` |
| `money` | `account`, `amount` | CasinoLand bankroll (command 40) | Until `account delete` |
| `saves` | `account`, `path`, `version`, `hash`, `data`, `updated_at`, `current` | Browser game files ([saves.md](saves.md)) | Until `account delete`; three versions per path |

A 40/4 persona rename is session-only and is not written to `accounts`. Board posts are attributed by
matching the stored Login persona to the `author` prefix, case-insensitive; a renamed session can leave
a post the delete command will not find.

No IP addresses, emails or OAuth subjects are stored today. The planned sign-in table is described in
[PLAN.md](../PLAN.md) and will store only an HMAC of the issuer and subject.

## 2. Public mode (`innkeeperd --public`)

Opt-in. The default log and capture format stay exactly as they are.

- **No capture.** `--capture-dir` given on the command line is an error. `--no-capture` is implied.
- **No message contents.** `step=client Login` / `step=host LoginAccepted`: the kind only, no fields,
  no hex dump. The raw line `client message` keeps `len=N` and drops the bytes.
- **No IP addresses.** The `peer=` field in spans and lines is the first eight hex characters of
  HMAC-SHA256 over the IP (not the port), keyed by a random per-process secret. Sessions correlate
  within one run and are not reversible or stable across restarts.
- **No free text.** Persona names in info/warn lines become the account number. PAD commands, dial
  strings and AT lines log the event kind only.

`LogPolicy` in `innkeeper-world` and `PeerTokens` in `innkeeperd` are the formatters; call sites do
not branch on `--public` by hand.

## 3. Export and delete

Offline, against `--data-dir`. Stop the server first: `serve` writes `DIR/innkeeper.lock` with its
pid, and the tools refuse while that pid is alive. A leftover file from a crash is ignored if the pid
is gone.

```
innkeeperd account export <id> --data-dir DIR [--with-saves]
innkeeperd account delete <id> --data-dir DIR [--yes]
```

**Export** prints one JSON document on stdout:

- the account row, with `"encoded_password": "omitted: the encoded password is not exported"` and no
  password bytes;
- mailbox number, letters received (that mailbox) and letters sent (other mailboxes whose envelope
  `from` is this mailbox);
- posts whose `author` matches the stored persona;
- money;
- save file list, sizes, hashes and versions; `--with-saves` adds `data_base64`.

**Delete** without `--yes` prints the plan and exits 1. With `--yes`, one SQLite transaction removes:

- the account row;
- its mailbox and every letter in it;
- letters it sent to other mailboxes (deleted; they are not kept as "(deleted)");
- board posts attributed to its stored persona (deleted; the author is not anonymised);
- money and saves.

Those two delete choices are the safer default: leftover mail and posts would still name the person.

## 4. Recommended VM settings

- **journald:** bound retention, e.g. `SystemMaxUse=50M` and `MaxRetentionSec=7day`, so info logs
  (even in public mode) do not keep peer tokens forever.
- **Reverse proxy:** no access logs. Nginx `access_log off;` on the WebSocket and TCP upgrade
  location; Caddy `log` off for that site. The proxy otherwise records client IPs that `--public`
  just hashed.
- **Backups:** encrypt at rest. Rotate so a deleted account leaves the last backup after a bounded
  window (for example daily copies kept 7 days, then dropped). A backup that still has the SQLite
  file still has the mail and saves `account delete` removed from the live database.
- Run the public host with `--public` and `--data-dir` on encrypted disk.

# The persistence boundary (`store`)

What the host must remember between connections and restarts sits behind one synchronous trait,
`innkeeper_world::Store` (`crates/innkeeper-world/src/store.rs`). The world crate stays free of I/O
and of SQL; the daemon supplies the durable implementation,
`innkeeperd::sqlite_store::SqliteStore`. Both are in the module map of
[innkeeperd.md](innkeeperd.md) section 5.

## 1. What is stored

Five families, each as narrow as its first users need.

| Family | Key | Operations | Used by |
|---|---|---|---|
| accounts | `AccountId` | `account` (get), `create_account`, `update_password`, `set_persona`, `highest_account_id` | account book, logon, command 44, signup |
| money | `AccountId` | `money` (get), `set_money` | CasinoLand bankroll (commands 40/3 and 40/2) |
| mailboxes | `MailboxNumber`, assigned per account | `assign_mailbox`, `append_letter`, `letters`, `delete_letter` | mail (`NewBoxHandler` 45, `EMMsg` 37) |
| boards | `BoardId` | `post`, `posts`, `delete_post` | the RPG bulletin board (command 27), complaints, Member Services |
| saves | `AccountId`, canonical path, version | `saves`, `save_versions`, `save_read`, `save_write` | the browser client's game files ([saves.md](saves.md)) |

An `AccountRecord` holds the `EncodedPassword` exactly as the client sends it, the persona name, the
`user_flags` and `rating` words and the `LoginStatus` byte of the login Ack, and the
password-expired flag. A `Letter` and a `Post` are the client's own bytes plus a store-assigned id
and the accounts the host recorded when it wrote them: a letter's `from` (the account whose session
sent it) and a post's `account` (its author) and `about` (for a complaint, the account it names).
The store never looks inside a body, so the mail and board layouts belong to the modules that parse
them.

A `Money` is the client's 32-bit CasinoLand bankroll as a value of its own rather than a field of
`AccountRecord`, because a read has to tell "never stored" from "stored 0". `money` returning `None`
is the first case. A 40/3 read then reports the starting 500 (`MONEY_STARTING_BALANCE`), while
`Some(Money(0))` is a player who lost everything. Signup and open enrolment store that 500 when they
create an account that has no balance yet. `set_money` stores the amount it is given and supplies no
default of its own, so a later save above or below 500 replaces the grant and a stored balance is
never rewritten to 500.

## 2. Rules every implementation follows

- **Reads of unknown keys return values.** An unknown account is `None`; the letters of an unknown
  mailbox and the posts of an unused board are empty lists; deleting a letter that is not there is
  `false`. Nothing panics on an id the client made up.
- **No row without its parent.** A row that belongs to an account needs that account to exist and
  be live when it is written: `set_money`, `save_write`, `assign_mailbox`, `update_password`,
  `set_persona` and `post` (for its author) give `StoreError::UnknownAccount` otherwise, and
  `append_letter` needs a live recipient box (`UnknownMailbox`) and a live sender
  (`UnknownAccount`). A complaint's `about` must name an account that exists, in any state. A store
  therefore never holds an orphan; the SQLite store also enforces it with foreign keys.
- **Attribution is by account number only.** A post's author is `Post::account`, a letter's sender
  `Letter::from`, a complaint's subject `Post::about`. The `name|` at the head of a post body and
  the names in a letter's envelope are the client's bytes: nothing reads them to find an account,
  and two accounts may share a persona.
- **Only a complaint names an account.** `post` with an `about` on a board whose kind is not
  `Complaint` (`bbs::post_kind`) is refused.
- **An account number is never issued twice.** `create_account` over any number ever created,
  deleted and purged ones included, gives `AccountExists` and keeps the old row.
  `highest_account_id` counts every row, so signup's and the web's "one past the highest" is always
  new.
- **A password change also clears the password-expired flag.** That is what command 44 means
  (`messages.md` section 3.3), so the store does it in the same write.
- **Mailbox numbers count from 1 and never change.** `assign_mailbox` is idempotent: the first call
  for an account assigns, later calls return the same number.
- **Order is posting order.** `letters` and `posts` return oldest first. Letter ids are unique across
  all mailboxes and post ids across all boards, and are never reused.
- **A board exists from its first post.** There is no board table to maintain.
- **Backend faults are `StoreError::Backend`.** The error carries the backend's own error, boxed, so
  the world crate does not name `rusqlite`.

The trait takes `&self` and requires `Send + Sync`, so one store is shared as `Arc<dyn Store>` by
every connection task. Each implementation serialises its own access with a mutex.

A book that is not the store's (the dev default `AccountBook::anyone`, a test's list) admits numbers
the store has never seen. The logon then gives the account a row (`logon::give_a_row`), so its
bankroll, posts and saves have a parent. Such a book still runs no post office: mail answers as if
the account had no box (status 2 to 45/1, [mail.md](mail.md)).

## 3. Implementations and the schema

| Type | Where | Lifetime |
|---|---|---|
| `MemoryStore` | `innkeeper-world::store` | until the process exits; the default for tests and for a host started without a data dir |
| `SqliteStore` | `innkeeperd::sqlite_store` | a SQLite file (`open`) or an in-memory database (`open_in_memory`) |

`MemoryStore` keeps the same rules in its own maps: it never removes an account, a write checks its
parent, and a retired account takes no writes.

`save_write` hashes the content (SHA-256 hex), numbers the version per file without reusing a
number, and trims to three versions with the current one always kept; the rules about what is
stored and who wins are the save channel's ([saves.md](saves.md)).

### 3.1 Versions and the baseline

The schema version is SQLite's `user_version`. **Version 5 is the baseline** (`BASELINE` in
`sqlite_store.rs`): `SqliteStore::open` on an empty file creates every table, index and trigger
below in one transaction and writes 5. After it comes `MIGRATIONS`, an ordered list, empty so far:
entry `n` takes a database from version `5 + n` to the next, each in one transaction with its
version bump. `SCHEMA_VERSION` is 5 plus the list's length.

Versions 1 to 4 were a chain of in-place `ALTER TABLE` upgrades over what was all test data. They
are not converted. A file with tables and a version below 5 (or no version) is refused, untouched,
with the remedy in the error: `innkeeperd db reset --data-dir DIR` ([innkeeperd.md](innkeeperd.md)
section 2) renames it to `innkeeper-<timestamp>.db.old`, never deleting it, and creates a fresh
baseline. A file from a newer build is refused too. `innkeeperd db check` prints the version and
runs `PRAGMA integrity_check` and `PRAGMA foreign_key_check`.

Every connection turns `PRAGMA foreign_keys` on and reads it back, since SQLite's setting is per
connection (`foreign_keys_are_on_for_every_connection`). `rusqlite` is built with the `bundled`
feature, so no system SQLite is needed and `STRICT` tables (SQLite 3.37 and later) are available.

### 3.2 Conventions

- **Every table is `STRICT`**: a value of the wrong type is an error, not a silent conversion.
- **Every time is INTEGER Unix milliseconds UTC, in a column named `*_at`.** `saves.modified_at` is
  the page's `mtime`, also milliseconds, clamped to the host clock ([saves.md](saves.md)).
- **Every column that names another row is a foreign key and has an index**: its own, a `UNIQUE`,
  or the leading column of the primary key (`every_foreign_key_column_has_an_index`).
- **CHECK constraints** hold the ranges the client can send and no tighter: account numbers, money
  and board ids are 32-bit unsigned, `user_flags` and `rating` 16-bit, `status` 8-bit, flags
  `IN (0, 1)`, an encoded password 10 bytes, mailbox numbers 1 to 4294901759, keys and token
  hashes 32 bytes, a save hash 64 hex digits.
- A column holding an account number is named `account`, or `from_account` and `about_account`
  where a row names two.

### 3.3 Tables

ON DELETE is NO ACTION unless the table says otherwise: the parent cannot go while a child names it.
A cascade is used only where the child has no meaning alone.

| Table | Key | Columns | Foreign keys (ON DELETE) |
|---|---|---|---|
| `accounts` | `id` (the account number) | `password`, `persona`, `user_flags`, `rating`, `status`, `password_expired`, `created_at`, `deleted_at`, `suspended_at`, `purged_at` | none. Triggers: a row is never deleted, its `id` never changes, and a tombstone is never written again |
| `mailboxes` | `number` (AUTOINCREMENT) | `account` (UNIQUE), `created_at`, `deleted_at` | `account` → `accounts` (CASCADE) |
| `letters` | `id` (AUTOINCREMENT) | `to_mailbox`, `from_account`, `body`, `created_at`, `deleted_at` | `to_mailbox` → `mailboxes` (CASCADE: a letter goes with its box); `from_account` → `accounts` |
| `posts` | `id` (AUTOINCREMENT) | `board`, `kind` (generated from `board`), `account`, `about_account`, `body`, `created_at`, `deleted_at`, `handled_at`, `handled_by` | `account` → `accounts`; `about_account` → `accounts`; `handled_by` → `operators`. CHECK: only a complaint has an `about_account`; only a complaint or a Member Services message is handled, and `handled_at` and `handled_by` are set together |
| `money` | `account` | `amount`, `deleted_at` | `account` → `accounts` (CASCADE) |
| `saves` | (`account`, `path`, `version`) | `hash`, `data`, `modified_at`, `current` (one current version per path, by a partial unique index), `deleted_at` | `account` → `accounts` (CASCADE) |
| `holds` | `account` | `placed_at`, `expires_at` | `account` → `accounts` |
| `identities` | `identity_key` | `key_version`, `subject_sealed`, `provider`, `adult_attested_at`, `attested_age`, `terms_version`, `created_at`, `deleted_at`. CHECK: the three attestation columns are set together | none |
| `identity_accounts` | `account` | `identity_key`, `created_at` | `account` → `accounts` (CASCADE); `identity_key` → `identities` (CASCADE; ON UPDATE CASCADE for a key rotation) |
| `sessions` | `token_hash` | `identity_key`, `account`, `created_at`, `expires_at` | `identity_key` → `identities` (CASCADE; ON UPDATE CASCADE); `account` → `accounts` |
| `audit_sessions` | `id` (AUTOINCREMENT) | `started_at`, `ended_at`, `account`, `identity_key`, `sealed` | `account` → `accounts`; `identity_key` → `identities` (SET NULL: the sealed record outlives the identity and forgets it; ON UPDATE CASCADE) |
| `operators` | `identity_key` | `first_seen_at` | none |
| `staff` | `identity_key` | `note`, `added_at`, `revoked_at` | `identity_key` → `operators` |
| `admin_sessions` | `token_hash` | `identity_key`, `created_at`, `last_seen_at`, `expires_at` | `identity_key` → `operators` (CASCADE) |
| `admin_audit` | `id` (AUTOINCREMENT) | `recorded_at`, `identity_key`, `action`, `target`, `reason`, `account` | `identity_key` → `operators`; `account` → `accounts` |

`posts.kind` is a virtual generated column: `member_services` for land 0, location `0x63`,
section 2; `complaint` for any other board at location `0x63`, in any land; `topic` for land 0,
location 0, sections 67 and 84; `post` otherwise. It is the same function as `bbs::post_kind`
(`the_kind_column_agrees_with_the_world_crates_post_kind`). The bodies of complaints and Member
Services messages are never edited: "handled" is the two `handled_*` columns beside them, which
replace the old `complaint_handled` table.

`operators` lists every identity the portal has seen sign in, or that `admin allow` or
`admin revoke` named. It is the parent of that identity's staff row, cookies, audit entries and
"handled" marks. It exists because a portal identity may come from the vault, the allowlist file or
the open door, so neither `staff` nor `identities` (the players' sign-ins) is a parent for all of
them. A staff revocation only stamps `revoked_at`, so the operator row and every audit entry naming
it stay. The handle the portal shows (`Caller 7F3A`) is derived from the key and is not stored.

### 3.4 The account lifecycle and tombstones

```
 create ──► live ──(account delete / portal delete)──► deleted ──(restore, inside the window)──► live
                                                          │
                                          window over and no hold (purge),
                                          or account delete --purge-now
                                                          ▼
                                   tombstone: purged_at set; password, persona, user_flags,
                                   rating, status, password_expired, suspended_at NULL;
                                   id, created_at, deleted_at kept. Never written again.
```

The account row is the parent of everything that must outlive it: sealed connection records (90
days from the connection), complaints written by it or about it (365 days from arrival), the
portal's audit log (365 days), and letters and posts that wait out windows of their own. A hard
delete of the row would have to cascade those away early or orphan them, so the purge never deletes
an account. It makes the row a **tombstone**. A CHECK keeps the personal columns either all set
(live or deleted) or all NULL (purged), a trigger refuses any `DELETE FROM accounts`, and another
refuses any update to a tombstone, so a purged account can be neither revived nor refilled. Because
the row stays, `MAX(id)` never falls and the next number (`highest_account_id` + 1) has never been
issued, including after the highest account is purged
(`the_highest_number_is_not_issued_again_after_it_is_purged`).

What the purge removes with the account, in the same transaction: its `money`, its `saves` and its
`identity_accounts` link (then the identity, once it owns no account and has no session). Its
mailbox goes as soon as no letter is left in it, in that purge or a later one: a deleted letter
keeps the 90-day mail window, which is longer than the 30-day account window. Nothing cascades from
the account row, because it is never deleted. The CASCADEs from `accounts` state which children have
no meaning alone, and the one from `mailboxes` to `letters` is the same rule one level down; the
purge deletes those rows itself, by name, so its counts are exact.

The other windows, and the holds that override them, are in [privacy.md](privacy.md) section 6.
`retention.rs` runs them. `every_table_lives_through_delete_hold_release_and_purge_without_an_orphan`
takes a row of every table through creation, soft delete, a purge that a hold blocks, the release
and the purge, with `PRAGMA foreign_key_check` empty after every step.

## 4. The conformance suite

`innkeeper_world::conformance` (behind the `test-support` feature, and always in the crate's own
tests) holds the cases every `Store` must pass:

- `run_all(fresh)` runs each case on a store from `fresh`: unknown ids, create and read back,
  duplicate create, password update, bankroll absent then per account (replacing, and 0 apart from
  absent), a bankroll and a save needing a live account, saves (absent, per account and path, three
  versions kept, opaque bytes), mailbox numbering, letter order and isolation between mailboxes, a
  letter needing a live sender and box, arbitrary bytes (empty, NUL, high bit), board order and
  isolation, a post recording its author, only a complaint naming an account (one that exists), two
  accounts with one persona never sharing a post or a letter, account numbers never reused, and a
  deleted account taking no writes.
- `run_persistence(reopen)` writes through one handle, drops it and checks a handle from `reopen`:
  accounts, the changed password, the bankroll, the mailbox number, the remaining letters with their
  sender and the posts with their author survive, and numbering carries on without reusing an id.

`MemoryStore` runs `run_all` in the world crate. `SqliteStore` runs `run_all` on an in-memory and on
a fresh file database, and `run_persistence` on a file, in
`crates/innkeeperd/src/sqlite_store_tests.rs`. That file also tests the schema itself: an orphan
insert fails for every foreign key, every foreign key column has an index, every table is STRICT
and its CHECKs refuse out-of-range values while keeping the legal extremes, an old database is
refused untouched with the remedy, a newer one is refused, and a tombstone keeps its number. A new
implementation adds the same calls to its tests and needs nothing else.

## 5. Who uses it

The account book and `World` take a store (`World::stored`), and `innkeeperd --data-dir` opens a
`SqliteStore`.

The bankroll serves CasinoLand's command 40 ([messages.md](../protocol/messages.md) section 3.3.1):
`innkeeperd` answers a 40/3 read from `money`, reporting 500 for an account the store has no
balance for, and writes a 40/2 save with `set_money` before it acks it. The saved amount is kept
as the client sent it.

Mail is the first user of the mailbox family ([mail.md](mail.md)):

- `assign_mailbox` answers command 45 and finds the requester's own box for every command 37 request.
- `append_letter` stores a body of six bytes of delivery time followed by the client's envelope, and
  records the requester's account as the sender; the store still never looks inside. A delivery to
  a number nobody was assigned, or to a deleted account's box, is `UnknownMailbox`.
- `letters` serves the new-mail check, the listing and the read, and skips rows with `deleted_at` set.
  `delete_letter` sets `deleted_at` (the row stays until the retention purge in [privacy.md](privacy.md));
  it returns false for a letter that is not in that mailbox or is already hidden, which the client sees
  as "that letter does not exist".

Boards (`post`, `posts`) serve the RPG bulletin board (`innkeeper-world::bbs`): a post body is the
client's `author|text`, and a `BoardId` is `landType << 24 | location << 8 | section`, so the two
titles keep apart ([yserbius.md](../protocol/yserbius.md) section 3). `post(board, author, about,
body)` records the writing account, which is what ownership means for a post: export, delete,
holds and "erase my own message" use it and nothing else. A complaint also records `about`, the
account of the player its text names, resolved while that player is online
([hub-services.md](hub-services.md) section 2).

A first-class delivery time on `Letter` would let the stored body be the envelope alone; that is a
trait change for the owner of `store.rs`.

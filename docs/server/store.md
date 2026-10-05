# The persistence boundary (`store`)

What the host must remember between connections and restarts sits behind one synchronous trait,
`innkeeper_world::Store` (`crates/innkeeper-world/src/store.rs`). The world crate stays free of I/O
and of SQL; the daemon supplies the durable implementation,
`innkeeperd::sqlite_store::SqliteStore`. Both are in the module map of
[innkeeperd.md](innkeeperd.md) section 5.

## 1. What is stored

Three families, each as narrow as its first users need.

| Family | Key | Operations | Used by |
|---|---|---|---|
| accounts | `AccountId` | `account` (get), `create_account`, `update_password` | account book, logon, command 44 |
| mailboxes | `MailboxNumber`, assigned per account | `assign_mailbox`, `append_letter`, `letters`, `delete_letter` | mail (`NewBoxHandler` 45, `EMMsg` 37) |
| boards | `BoardId` | `post`, `posts` | the RPG bulletin board (command 27) |

An `AccountRecord` holds the `EncodedPassword` exactly as the client sends it, the persona name, the
`user_flags` and `rating` words and the `LoginStatus` byte of the login Ack, and the
password-expired flag. A `Letter` and a `Post` are the client's own bytes plus a store-assigned id:
the store never looks inside, so the mail and board layouts belong to the modules that parse them.

## 2. Rules every implementation follows

- **Reads of unknown keys return values.** An unknown account is `None`; the letters of an unknown
  mailbox and the posts of an unused board are empty lists; deleting a letter that is not there is
  `false`. Nothing panics on an id the client made up.
- **Writes that need a parent say so.** `update_password` and `assign_mailbox` on an unknown account
  give `StoreError::UnknownAccount`, `append_letter` to an unknown mailbox gives `UnknownMailbox`,
  and `create_account` over an existing id gives `AccountExists` and keeps the old record.
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

## 3. Implementations

| Type | Where | Lifetime |
|---|---|---|
| `MemoryStore` | `innkeeper-world::store` | until the process exits; the default for tests and for a host started without a data dir |
| `SqliteStore` | `innkeeperd::sqlite_store` | a SQLite file (`open`) or an in-memory database (`open_in_memory`) |

`SqliteStore` creates its tables on open with `CREATE TABLE IF NOT EXISTS`, so opening a new path
yields an empty store and opening an existing one keeps its contents. Mailbox numbers, letter ids
and post ids are `AUTOINCREMENT` row ids, which is how they stay unique and unreused. The schema has
no version column yet; the first change that alters a table adds one together with its migration.

`rusqlite` is built with the `bundled` feature, so no system SQLite is needed.

## 4. The conformance suite

`innkeeper_world::conformance` (behind the `test-support` feature, and always in the crate's own
tests) holds the cases every `Store` must pass:

- `run_all(fresh)` runs each case on a store from `fresh`: unknown ids, create and read back,
  duplicate create, password update, mailbox numbering, letter order and isolation between
  mailboxes, arbitrary bytes (empty, NUL, high bit) and board order and isolation.
- `run_persistence(reopen)` writes through one handle, drops it and checks a handle from `reopen`:
  accounts, the changed password, the mailbox number, the remaining letters and the posts survive,
  and numbering carries on without reusing an id.

`MemoryStore` runs `run_all` in the world crate. `SqliteStore` runs `run_all` on an in-memory and on
a fresh file database, and `run_persistence` on a file, in `crates/innkeeperd/src/sqlite_store.rs`.
A new implementation adds the same calls to its tests and needs nothing else.

## 5. Who uses it

The account book and `World` take a store (`World::stored`), and `innkeeperd --data-dir` opens a
`SqliteStore`. Mail is the first user of the mailbox family ([mail.md](mail.md)):

- `assign_mailbox` answers command 45 and finds the requester's own box for every command 37 request; the
  open book's store has no accounts, so it fails with `UnknownAccount`, which the mail module reports as a
  status 2 to the client.
- `append_letter` stores a body of six bytes of delivery time followed by the client's envelope; the store still
  never looks inside. A delivery to a number nobody was assigned is `UnknownMailbox`.
- `letters` serves the new-mail check, the listing and the read; `delete_letter` returns false for a letter
  that is not in that mailbox, which the client sees as "that letter does not exist".

Boards (`post`, `posts`) have no user yet; the RPG bulletin board will parse its own bodies on top of them.
A first-class delivery time on `Letter` would let the stored body be the envelope alone; that is a trait
change for the owner of `store.rs`.

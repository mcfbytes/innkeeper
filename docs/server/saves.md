# Game saves for the browser client (`saves`)

The browser build of the client keeps the C: drive in the browser's IndexedDB, so a player's characters
and automaps live in one browser. The save channel keeps them per account in the host's SQLite store
instead, and puts them back when the same account signs on from another browser. The browser side is
described in `INN-Decomp/docs/web.md`; this page is the host side. Code: `crates/innkeeperd/src/save_channel.rs`
(protocol), `crates/innkeeper-world/src/saves.rs` (paths, limits, hash), the `saves` family of
[`Store`](store.md).

## 1. Channel and authority

The channel is the browser's own WebSocket (`--ws-bind`, [innkeeperd.md](innkeeperd.md) section 2).
Binary messages stay the line's serial bytes. **Text messages are JSON control messages** for the save
channel; the page sends and reads them from JavaScript and the emulated serial line never sees them.

There are no credentials of its own. A line session that has completed a Login (`LoginAccepted`) names the
account, and the host publishes it to that socket's save channel (`Host` to `relay` through a `watch`
channel). Until then every control message is answered `error` with code `not_logged_on`; after a
hang-up (`LinkLost`, or the socket closing) the channel is closed again. A second Login on the same line
replaces the account and sends a fresh manifest.

Limits on the socket: a text message may be up to 2 MiB (a 1 MiB file is 1.4 MiB as base64); a *binary*
message over 64 KiB still closes the connection, as before.

## 2. Messages

Every message is a JSON object with `"op"`. A request may carry `"id"` (any JSON value); the reply
echoes it. `mtime` is milliseconds since the Unix epoch. `data` is standard base64. A file entry is

```
{"path": "INN\\YSERBIUS\\MYCHARS.DAT", "version": 3, "hash": "<sha-256 hex>", "size": 1234, "mtime": 1760000000000, "current": true}
```

| Direction | Message | Meaning |
|---|---|---|
| host to page | `{"op":"manifest","account":N,"files":[entry...]}` | Sent unprompted right after the logon, and for a `manifest` request. The current version of every file the account has. |
| page to host | `{"op":"manifest"}` | Ask again. |
| page to host | `{"op":"get","path":P,"version":V?}` | Download the current version, or the kept version V. |
| host to page | `{"op":"file", ...entry, "data":B64}` | The download. |
| page to host | `{"op":"put","path":P,"base":V,"mtime":T,"data":B64}` | Upload. `base` is the version the page last synced (0 for none), `mtime` when the content was last modified. |
| host to page | `{"op":"stored", ...entry, "replaced":bool, "unchanged":bool}` | Accepted. `entry` is the new version. `replaced`: the upload won a conflict. `unchanged`: the host already held exactly these bytes, no new version. |
| host to page | `{"op":"superseded", ...entry of the current version, "kept_version":K}` | Not accepted: the host's copy is newer. The upload is kept as history version K; the page should download. |
| page to host | `{"op":"versions","path":P}` | List the kept versions, newest first. |
| host to page | `{"op":"versions","path":P,"versions":[entry...]}` | |
| host to page | `{"op":"error","code":C,"detail":text}` | `not_logged_on`, `bad_request`, `unknown_op`, `bad_path`, `not_allowed`, `too_large`, `bad_data`, `quota`, `no_such_file`, `server_error`. |

## 3. What may be stored

Paths are relative to the drive root, compared without regard to case or slash direction and kept in
upper case with `\`:

| Path | Holds |
|---|---|
| `INN\YSERBIUS\MYCHARS.DAT`, `INN\YSERBIUS\AUTOMAPS.DAT` | Yserbius characters and automaps |
| `INN\TWINION\TWINCHAR.DAT`, `INN\TWINION\AUTOMAPS.DAT` | Twinion characters and automaps |
| `INN\PLAYER.DIR` and `INN\<name>.DTA`, name of 1 to 8 letters, digits, `_`, `$`, `-` | the INN persona files |
| `INN\LL\PLAYER.DIR`, `INN\LL\<name>.DTA`, `INN\SL\PLAYER.DIR`, `INN\SL\<name>.DTA` | the CasinoLand and SierraLand personas |
| `INN\LL$$<digits>.DAT`, 1 to 4 digits | the CasinoLand bankroll, named from the account number; the client asks the host (command 40) only when it is missing |

`PASS_SET.DTA` is never stored or served, whatever its case or directory. Neither is `INN\LLSYSTEM.DAT`,
CasinoLand's optional access password: like the INN password it belongs to the installation. Everything else (`.NEW` work files,
executables, `..`, other directories) is `not_allowed` or `bad_path`.

Limits (constants in `saves.rs`): 1 MiB per file, 8 MiB for the current versions of one account's
files together, 64 files per account. History versions do not count toward the 8 MiB. A refused upload
changes nothing.

## 4. Versions and conflicts

Per account and path the store numbers versions from 1 and never reuses a number. One version is
*current*; the host keeps **3 versions per file** (the current one and the 2 newest others) and drops older
ones. A `put` is decided by the host, in this order:

1. No current version: stored as version 1.
2. Same SHA-256 as the current version: `stored` with `unchanged`; nothing is written.
3. `base` equals the current version (the page had the latest): stored as the new current.
4. Otherwise both sides changed since the page's last sync, and **the later modification wins**. `mtime`
   is clamped to the host clock; if it is later than the current version's `mtime` the upload becomes
   current (`replaced`) and the old current stays as history. If not, the upload is stored as history
   beside the current version and the reply is `superseded`.

So the losing content is always recoverable with `get` and a version.

## 5. What the page does with it

The page remembers, per account and path, the version and hash it last synced. On a manifest, for each
path, with L the local file (if any), S the host's entry and "synced" the remembered pair:

| Case | Action |
|---|---|
| neither side has it | nothing |
| only the host has it | download and write it |
| only the page has it | upload with base 0 |
| same hash | remember it as synced |
| host version differs from synced, local hash equals synced | download (server newer, local unchanged) |
| local hash differs from synced, host version equals synced | upload with base = the host version |
| both differ (or nothing synced yet, and the contents differ) | upload with base = synced version (0 if none); the host decides by `mtime` as in section 4, and the page downloads on `superseded` |

Uploads also happen when the emulated program closes a file it wrote or renames onto a save path, 1.5 s
debounced, and for all changed files when the page is hidden or unloaded. A downloaded file is given
the host's `mtime` so it is not mistaken for a local change.

Timing: the manifest is sent as the logon completes, and a download is a round trip after it, so
the files are in place long before a player can pick a land and start CGENN or TWGENN, which read the
character files after that. The window is not closed by construction: a game started within that round
trip would read the old file. Verified end to end in headless Chromium: a fresh browser profile with an
install lacking the character files received them after the logon (`INN-Decomp/docs/web.md`).

## 6. Fetching before the dial (HTTP)

The logon manifest arrives after the client has already read its persona, so a browser that has never
held the account would show an empty Select Player (the account's personas live in its save space: its `PLAYER.DIR` and `.DTA` files) and its first Create would overwrite the host's. A signed-in page therefore
fetches the account's files over HTTP before it dials, on the sign-in listener, authorised by the session
token like the other `/auth/*` calls (`signin/routes.rs`):

| Request | Answer |
|---|---|
| `GET /auth/accounts/<id>/saves` with `Authorization: Bearer <token>` | `200 {"account":N,"files":[entry...]}`: the same entries as the channel manifest (`path`, `version`, `hash`, `size`, `mtime`, `current`), each with `"data"` (base64) while the running total of inline bytes stays within 2 MiB. A file that would pass the cap comes without `data`. |
| `GET /auth/accounts/<id>/saves?path=P` | `200` one entry with `data`, always. `P` is URL-encoded and goes through the same canonical path rules; `400 bad_path` for a path that cannot be stored, `404 no_such_file` for one the account lacks. |

`401 unauthorized` without a live token; `404 no_such_account` when the identity does not own the account
(the same answer as for a credential). The files are exactly the current versions the save channel would
list: same store, same allowlist, same per-account filter, so `PASS_SET.DTA` and the like never appear.
CORS is the same as the other `/auth/*` routes. The log gets counts and byte sizes only (`saves
prefetched`), never paths or contents. A Login is not required, which is the point; the token is the
proof that the identity owns the account. Tests: `signin/tests.rs` (cap, ownership, errors) and
`tests/signin.rs` (a put over the WebSocket read back over HTTP, under the sandbox where the kernel
allows it).

## 7. Store and upgrade

`Store` gains `saves`, `save_versions`, `save_read` and `save_write` ([store.md](store.md)). SQLite, in the
baseline schema (store.md section 3.3):

```
CREATE TABLE saves (
    account INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    path TEXT NOT NULL, version INTEGER NOT NULL CHECK (version >= 1),
    hash TEXT NOT NULL CHECK (length(hash) = 64), data BLOB NOT NULL,
    modified_at INTEGER NOT NULL CHECK (modified_at >= 0),
    current INTEGER NOT NULL CHECK (current IN (0, 1)), deleted_at INTEGER,
    PRIMARY KEY (account, path, version)) STRICT;
CREATE UNIQUE INDEX saves_one_current ON saves(account, path) WHERE current = 1;
```

A save needs a live account row: `save_write` for an unknown or deleted account is `UnknownAccount`.
`modified_at` is the content's modification time in Unix milliseconds as the uploader said, not the
time of the upload.

## 8. Tests

`save_channel.rs` (sans-IO, memory store): refusal before logon and after hang-up, upload and download, id
echo, account isolation, the allowlist, size and quota, both conflict outcomes, three versions kept, time
clamp. `websocket.rs` (real sockets, captured Login frame): `not_logged_on` on a live line, the manifest
after the logon, a 168,000-byte round trip, a second line of the same account seeing the file, an
oversized binary message closing the line. The store conformance cases run on `MemoryStore`, in-memory
and file `SqliteStore`; persistence across reopen is in `run_persistence`.

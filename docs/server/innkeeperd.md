# innkeeperd

The host daemon. It accepts the stock DOS client over TCP, answers the modem, PAD and link layer, plays
the host side of the logon, and records everything. With it the stock client logs in, reaches the town map
and enters the Clubhouse waiting room (`docs/protocol/captures.md` sections 9 to 11). Connections share
one object store, so players in the same waiting room see each other ([objects.md](objects.md)).
Command coverage against every program is [coverage.md](coverage.md).

## 1. Crates

```
innkeeperd           bin: command line, tokio runtime, TCP and WebSocket, capture files, wiring
  ├ innkeeper-world    host behaviour: typed messages, accounts, logon, land tables, no I/O
  ├ innkeeper-session  one legacy connection: line then link, no I/O
  │   ├ pad_thai         Hayes modem (hayes_fever), SprintNet PAD dialogue, line detection, no I/O
  │   └ tsn-link         TSNEXEC framing, CRC, ACK/NAK, resends, message packing, no I/O
  └ int14h             the 17 INT 14h exports and the transport codec (int14h-transport.md), no I/O
```

`innkeeperd` has up to three listeners on one world. The legacy one (`--bind`, `src/connection.rs`) answers
the stock DOS client through the modem, PAD and link layer. The WebSocket one (`--ws-bind`,
`src/websocket.rs`) runs the same legacy session for a browser client: each WebSocket is one serial line and
its binary messages are the line's raw bytes (section 2). That listener is an HTTP server (`src/http.rs`), so
it also serves `/healthz` and sign-in (`src/signin/`, with the vault client in `src/keyvault/`; section 8).
The transport one (`--int14h-bind`, `src/int14h_listener/`) answers a client that implements TSNEXEC's exports
itself and sends each call as an envelope ([int14h-transport.md](../protocol/int14h-transport.md) section 7);
its `Executive` holds one connection's export state. Both draw their `ConnectionId`s from one counter in
`ConnectionSettings`.

`innkeeper-world` does not depend on the session crates: it turns message bodies into deliveries, and
`innkeeperd` (`src/host.rs`, `Host`) carries them between the world and either transport. `Host::answer`
parses a client message body into a `ClientMessage` and hands it to the connection's `PlayerSession` with
the shared `World` and the host's `ObjectStore`, which every connection shares behind one `Mutex`. What
comes back is a list of `Delivery { to, message }`: deliveries for other connections go through
`src/switchboard.rs` to their inboxes, and `answer` returns the connection's own `HostMessage`s with the
`Exchange` log. The legacy driver encodes them, queues them with `Session::send_message` and sends them with
`Session::flush`, which closes the frame; the transport queues them for Receive. Each connection's pump loop
treats what arrives in its inbox the same way. When the connection ends, the store releases what it held
and tells the others ([objects.md](objects.md) section 5).

On the legacy link a connection's lifecycle follows its call. A call ends when the client clears it at the PAD (`D`) or the
emulated modem hangs up (`ATH`, `ATZ`); the session then reports `SessionEvent::LinkLost` with a `LinkLoss`,
once per call and never for a connection that had no call. `innkeeperd` answers with `Host::hang_up`: the
store releases what the connection held and tells the others ([objects.md](objects.md) section 5), while
the socket stays open for a new call. When the TCP connection closes, or a transport client calls
Disconnect, the driver does the same.
`Session::is_transmit_idle` says whether everything sent to the client has been acknowledged.
`Session::begin_program_switch` holds the host's new DATA frames while the client chains programs
(`int14h-api.md` 9.3), until the client's next DATA frame or `PROGRAM_SWITCH_QUIET_MAX_ASSUMED` (10 s),
after which they leave in order; a frame already awaiting its ACK is still resent. Nothing arms it yet.

Every crate below `innkeeperd` is synchronous and deterministic: bytes and an `Instant` go in, bytes and
events come out, and timers are a `next_deadline()` that the caller sleeps on. `innkeeperd` depends on
`tsn-link` directly only to choose log levels for link events.

## 2. Running

```
cargo run -p innkeeperd -- [--bind 127.0.0.1:2314] [--capture-dir work/captures] [--no-capture]
                           [--public] [--sandbox | --no-sandbox] [--allow-root] [--sandbox-log]
                           [--data-dir DIR]
                           [--audit-public-key FILE --audit-key-id KID] [--audit-retention-days 90]
                           [--line auto|hayes|pad] [--connect-rate 2400]
                           [--int14h-bind 127.0.0.1:2315] [--no-int14h]
                           [--ws-bind 127.0.0.1:2316]
                           [--auth-config FILE] [--page-origin ORIGIN ...]
                           [--enrolment open|web-only|closed] [--session-hours 12]
                           [--accounts-per-day 3]
                           [--key-vault https://NAME.vault.azure.net | --insecure-dev-keys]
cargo run -p innkeeperd -- dev-oidc [--bind 127.0.0.1:2902] [--issuer URL]
```

- On a successful bind it logs `INT 14h hooked. Please wait while ImagiNation loads...`, then `serving the
  INT 14h transport` with the transport's address.
- `--int14h-bind` is where transport clients connect (`docs/protocol/int14h-transport.md`), and `--no-int14h`
  leaves that listener off. A transport connection gets a capture file like a legacy one: `tx` and `rx` are
  the envelope bytes, `ev` the decoded client and host messages.
- `--ws-bind` turns on WebSocket lines, used by the browser build of the client in the sibling project
  `INN-Decomp` (its `docs/web.md`). It is off unless given. A connection upgrades over plain HTTP (`ws://`;
  put a TLS proxy in front for `wss://`), then is a legacy session exactly like one on `--bind`: line
  auto-detect, Hayes modem, PAD, link layer and a capture file. Binary messages carry raw bytes in both
  directions, split anywhere; text messages are the save channel's JSON control messages once the line has
  logged on ([saves.md](saves.md)); a binary message over 64 KiB (a text message over 2 MiB) or a handshake that
  takes over 10 s closes the connection. Closing the WebSocket is a TCP hang-up. Any `Origin` is accepted
  on the upgrade; the session token is the check (section 8). The same listener answers plain HTTP:
  `GET /healthz` is 200 `ok`, and `/auth/*` is sign-in when `--auth-config` is given (section 8). Any
  other request that is not an upgrade gets 404.
- `--public` is for a host that should process as little personal data as possible: it refuses
  `--capture-dir`, writes no capture files, logs message kinds without fields, and replaces peer
  addresses with a per-process HMAC token. Persona names in other info lines become the account
  number. See [privacy.md](privacy.md). It also accepts players only over a loopback WebSocket: an
  explicit `--bind` or `--int14h-bind` is an error, `--ws-bind` is required and must be
  `127.0.0.0/8` or `::1`, and the INT 14h listener stays off. `--line` does not open a socket, so it
  stays available. There is no `--private` flag; without `--public` the listeners are as above.
  Every WebSocket must carry a live sign-in session (`?session=<token>`); without one the upgrade is
  an HTTP 401 (`connection_gate::admit`, section 8). With no `--auth-config`, startup warns that
  every WebSocket is refused. `--public` needs `--key-vault`; `--insecure-dev-keys` replaces it only
  when every `--page-origin` is loopback (section 8). Development tools that parse verbose logs keep
  working: the default format is unchanged. The in-process jail that `--public` turns on is
  [hardening.md](hardening.md).
- `--sandbox` applies Landlock, seccomp and `no_new_privs` after the sockets are bound and before
  the accept loop. It is on by default with `--public`. `--no-sandbox` leaves the process
  unrestricted. `--allow-root` is only for a local DOSBox setup and is refused with `--public`.
  `--sandbox-log` audits unknown syscalls instead of killing the process. Details, the paths and
  the kernel requirements are in [hardening.md](hardening.md).
- `RUST_LOG=debug` adds one line per received frame; the default level `info` shows line and PAD events
  and every decoded client message.
- `--data-dir` keeps the accounts in `DIR/innkeeper.db` (`SqliteStore`, created on first use) and enrols an
  unknown account from the password its first Login presents (INFERRED, `STORED_BOOK_ENROLS_ASSUMED`).
  That enrolment stores the starting 500 CasinoBucks when the number has no balance yet.
  Without it every account number and password is admitted and nothing is kept, which is what the DOSBox
  scripts use. Offline, `innkeeperd account export` reads that file, including soft-deleted rows.
  `account delete` disables the account until the retention purge; `account restore` undoes that
  inside the window; `hold` and `release` cover the account's content and its sealed connection
  records. `account delete` leaves those sealed records in place. The flags, the purge clocks and
  the offline `audit` tools are in [privacy.md](privacy.md).
- `--line` says what the client's serial port reaches (section 4). `--connect-rate` is the rate in the
  emulated modem's `CONNECT` line, which the client stores as its line rate (link-layer section 4 notes);
  the transport's GetLineRate answers the same rate.

## 3. Connecting the stock client from DOSBox-X

Both setups ran with the real client and reached the first message (`docs/protocol/captures.md`).
`tools/dosbox/run_client.py` writes the configuration for either one; `docs/dosbox.md` explains how to use it.
Option names are from the DOSBox-X 2026.01 help text.

**Modem emulation** (`--line pad`, or `auto`). DOSBox-X answers the `AT` commands itself and opens a TCP
connection when the client dials, so the server first hears the PAD wake-up. Map the number in the
`LSCI.CFG` `modem` string to the server with the phonebook file:

```
[serial]
serial1       = modem
phonebookfile = phonebook-dosbox-x.txt
```

with a line such as `5551234 127.0.0.1:2314` in `phonebook-dosbox-x.txt` and `modem = ATDT5551234!~` in
`LSCI.CFG`. Leave the modem's telnet mode off; it would treat `FF` bytes in frames as Telnet commands.
DOSBox-X logs `Modem could not open TCP port 23` at start; that is only its incoming-call listener.

**Null modem** (`--line hayes`, or `auto`). DOSBox-X forwards the raw UART bytes, so the server also plays
the Hayes modem:

```
[serial]
serial1 = nullmodem server:127.0.0.1 port:2314 transparent:1
```

`transparent:1` stops DOSBox-X from inserting its own handshake-line messages into the byte stream.

## 4. What the server plays

| Stage | Server behaviour | Reference |
|---|---|---|
| line detection (`auto`) | skip CR, LF and space; a first byte of `A`, `a` or `+` means a raw line, anything else a modem emulator | `pad_thai::Line` |
| Hayes modem (raw lines) | echoes commands, answers `OK`, answers `ATD…` with `CONNECT <rate>`, `+++` with a 1 s guard time returns to command mode, `ATO` goes back online, `ATH`/`ATZ` hang up, `AT\B` sends a BREAK to the PAD | link-layer sections 4, 2.5, 5.2 |
| PAD wake-up | `TERMINAL=` after the CR that follows `D`, then the `@` prompt; prompts anyway after two bare CRs | link-layer checklist 3 |
| call | `c <host>` gets `<host> CONNECTED`; a new link starts with both sequences at 0 | checklist 4, 6 |
| link | hunts for `81`, verifies the CRC, ACKs every good DATA frame (duplicates too), NAKs bad ones, delivers in-order payloads as messages; during a program switch new DATA frames wait for the client's next DATA frame or 10 s | checklist 7, 8, 10, 12 |
| hang-up | `D` at the PAD prompt, or `ATH`/`ATZ` while a call is up, ends the link once (`SessionEvent::LinkLost`); the connection's objects are released and its peers told, and the socket stays open | link-layer 5.2, checklist 12, 13 |
| escape to the PAD | a CR while no frame is open, or a modem BREAK, returns to the `@` prompt with the call still up; `SET?` gets a prompt, `D` gets `<host> DISCONNECTED` | checklist 12 |
| `DIRECT` | a byte with bit 7 set during wake-up starts the link at once | checklist 14 |

The INFERRED parts (prompt texts, the CR escape heuristic, the `DIRECT` detection, the program-switch
deadline) are named constants in `crates/pad_thai/src/assumptions.rs`, `crates/tsn-link/src/assumptions.rs`
and `crates/innkeeper-session/src/assumptions.rs`.

Host-side link timing (`tsn_link::LinkConfig`): no DATA frame for 1.5 s after a call connects unless the
client sends first, a resend after 3 s without an ACK, and at most 10 resends. These are server choices,
not facts about the original. In practice the client always sends first, so replies leave at once, one
DATA frame per client message.

## 5. The host side (`innkeeper-world`)

| Module | Holds |
|---|---|
| `message` | `ClientMessage` (parse, encode) and `HostMessage` (encode, parse) for the commands below, on a `WireReader` and `WireWriter` of the `b`/`w`/`a`/`s` field codes; `Command` is the table of command bytes. One file per family (`send`, `group`, `properties`, `multicast`, `service_lookup`, `notice`, `ack`, `user_info`, `user_find`, `object_kind`, `object_list`, `mail`, `mall`, `signup`); the type names are in `messages.md` section 3.2.1 |
| `account` | `AccountBook`: who may log in. `anyone()` admits every account number and password, the dev default; `listed(...)` checks number and encoded password; `stored(store, Enrolment)` reads the `AccountRecord` and gives the `Account` its `user_flags`, `rating` and `LoginStatus` (11 while the record's password is expired), and `change_password` updates the record |
| `logon` | `logon::admit`: the Login policy (land type in the `LandCatalog`, then the account book) as an `Account` or the Nak that refuses it, and the command 44 handler. `PlayerSession` only calls it |
| `land` | `LandCatalog`: the land directory with each row's version range, `admits(land_type, version)` against that range, and occupancy counted from the object store (`current` is the member count of the land's waiting room, `maximum` the catalog's 64) |
| `mail` | the mailbox assignment (45/1) and the `EMMsg` requests (37) over the store: new-mail check, listing, send, read, delete, forward. See [mail.md](mail.md) |
| `money` | the CasinoLand bankroll: a 40/3 read answered from the store with the starting 500 for an account that has none, and a 40/2 save stored and acked. Signup and open enrolment store that 500 when the new account has no balance yet. See [store.md](store.md) section 5 |
| `presence` | the waiting room of a land as a `GroupKey` (kind 5, land type, land number), and the GrpJoin that passes through the version gate before the store sees it |
| `town_hall` | the Town Hall: account texts (34), the system list boxes (37/32) and the Member Services and Accounting forms (37/30). See [services.md](services.md) |
| `player` | `PlayerSession`: one client from Login to hang-up, as an enum of `AwaitingLogin` and `LoggedIn`; turns each client message into `Delivery` values, with the object commands answered by the store; `persona()` reports the name the player plays now (the Login's, then the latest 40/4). Each message republishes the signed-on player for the phone book |
| `rpg` | Yserbius and Twinion: the title table (land type, version, land size), the 41/2 service lookup, the place a land joinNet sets and the map groups shared within it, the land version gate, no echo of RPG sends. See [yserbius.md](../protocol/yserbius.md) |
| `bbs` | the RPG bulletin board (27) over the store's boards: a fixed directory per location, messages numbered in posting order |
| `mall` | Shoppers Advantage (54): the gate launches Shop, the join is answered, and one line says the mall is closed |
| `signup` | command 55/1, before Login: create an account and return its number on Ack 55/1 |
| `conference` | command 41/0: a free conference name is SID 0, a taken name is that object's SID |
| `user_find` | command 49: who is logged in, by account (49/1) or by name (49/2), and the land they are still in |
| `object_list` | commands 51/0 and 52/0: create a list of plain objects and free them the way leaveNet does |
| `router` | `router::send` and `router::multicast`: a `Send` or a multicast becomes one `HostMessage::Send` per connection that holds the target, read from `ObjectStore::recipients`. See [router.md](router.md) |
| `objects` | `ObjectStore`: every networked object of the host, its holders and the members of each group, with host-wide SIDs; `ConnectionId` and `Delivery { to, message }` address a message to a connection. It also keeps who is logged in, and drops them on hang-up. See [objects.md](objects.md) |
| `world` | `World`: host number, accounts, lands, clock and the `Store` handle, shared read-only by every connection; `World::stock()` is the open dev host, `World::stored(store)` keeps accounts in a store |
| `store` | `Store`: accounts, bankrolls, mailboxes and boards behind one synchronous trait, with `MemoryStore`; `innkeeperd::sqlite_store::SqliteStore` is the durable implementation. See [store.md](store.md) |
| `assumptions` | values the client does not pin down, each naming its section of `messages.md`. A name ending `_ASSUMED` is an inference; `MONEY_STARTING_BALANCE` is an operator choice |

What the host answers (`docs/protocol/messages.md` for the layouts):

| Client sends | Host replies |
|---|---|
| Login (53, 59) | Ack `whichCmd` 22 carrying the account's `userFlags`, `status` and `rating` (all 0 on the open book). Nak 22 otherwise: reason 9 (the client asks for the password again, three tries) for a wrong password, reason 1 with a text for an unknown account or an unreadable store, reason 2 (INFERRED, `UNLISTED_LAND_NAK_ASSUMED`) for a land type the catalog lacks. A Login on a connection that holds objects first releases them, as a hang-up does. A signup row whose password is still the placeholder adopts the password this Login presents (`SIGNUP_ADOPTS_PRESENTED_PASSWORD_ASSUMED`) |
| signup (55/1) | Ack 55/1 with the new account number at words 6 and 8, before Login, and the session stays logged out. The form has no password, so the row keeps `SIGNUP_PASSWORD_ASSUMED` until that first Login (`SIGNUP_KEEPS_ONLY_THE_ACCOUNT_ASSUMED`). Numbers start at `SIGNUP_FIRST_ACCOUNT_ASSUMED`. An explicit signup creates a row even when enrolment is closed (`SIGNUP_CREATES_WHEN_ENROLMENT_IS_CLOSED_ASSUMED`). The new account is given 500 CasinoBucks unless that number already has a balance. A store fault sends nothing |
| 54/27, 54/25, 54/24, 54/26 | The gate launches Shop (`MALL_LAUNCHES_SHOP_ASSUMED`): one 54/19, then 54/17 and a 54/16 kind 1 that says the mall is closed (`MALL_CLOSED_TEXT_ASSUMED`, `MALL_NOTICE_FOLLOWS_JOIN_ASSUMED`). Word 2 of each reply is `MALL_HEADER_SLOT_ASSUMED`. A typed `/EXIT` also gets 54/20 kind `0x1A`. 54/26 gets 54/19 kind `0x1A`. There is no mall back end |
| 41/0 | `ObjectLocated` with the SID at word 6. Zero means the name is free; the bound object's SID means it is taken. The check does not create the conference. The key is the raw tail (`CONFERENCE_KEY_IS_THE_RAW_TAIL_ASSUMED`) |
| 49/1, 49/2 | one reply to the requester. `whichCmd` echoes the sub, `toSID` is 0, and `w count` is at byte 4. Sub 1 echoes the account words before the records; sub 2 does not. Each record is `b host, b landType, b landNumber, s persona`. The search is logged-in players. A name matches the persona, and the Login name (`FIND_ALSO_MATCHES_LOGIN_NAME_ASSUMED`); the printed name is the persona. `host` is this world's host number. The land is the player's place only while they still hold a member of that land's waiting room; otherwise both are 0, so the client prints "the Map". Count 0 means not found. Order is connection order (`FIND_RESULTS_IN_CONNECTION_ORDER_ASSUMED`) |
| 51/0 | one plain object per requested count, no `ObjID`. Command 51 echoes the caller's cookie at word 4 and lists the new SIDs from byte 8. A longer list is cut to `OBJ_NEW_LIST_MAX_ASSUMED`. The words at bytes 2 and 6 are `OBJ_NEW_LIST_PREFACE_WORD_ASSUMED` and `OBJ_NEW_LIST_UNREAD_WORD_ASSUMED` |
| 52/0 | nothing to the sender. Each listed SID is released like leaveNet, so a peer hears `ObjFree` (9), not command 52 (`OBJ_FREE_LIST_TELLS_PEERS_ASSUMED`) |
| 44 | Ack 44/1 to the request's SID after the password is stored and the expired flag cleared; Nak 44 when the store refuses. The open book stores nothing and still acks |
| joinNet (7) | `ObjID` with the cookie echoed and a host-wide SID counting up from `0x0100`; a group of kind 2, 4 or 5 is shared by `(kind, landType, parameter)`, so every joiner of the same waiting room gets the same SID; a game object joined again with the same cookie replaces the old one ([objects.md](objects.md) section 3) |
| leaveNet (9) | nothing to the sender; releases one reference, and the last one frees the object: `GrpDel` and `ObjFree` to the other connections that see it |
| add (10) to a group | `GrpJoin` for the member to the joiner and to every other connection with a member in the group; Nak 10 with code 1 (no such group), 2 (full), 3 (not the sender's object) or 6 (the version bytes of a waiting-room join lie outside the land's range, see `admits`; the member is not added, and a join without version bytes is not gated) |
| delete (11) | `GrpDel` to the connections of the remaining members |
| `GrpMem` (12) | the member list, from byte 6 in joining order |
| `Send` (2) | the same `Send` to every connection that holds the object `to`, or a member of the group `to`, the sender's own connection included (INFERRED); nothing for a SID nobody holds, and no Nak |
| multicast (28) | one `Send` of the body per recipient SID, in list order, under the same rules |
| 36/5 | `HostInfo` type 5: host number 7, the first host of the stock `HOSTADDR` |
| 36/6 | the land directory, always (the stamp is not compared): Clubhouse, SierraLand and CasinoLand, land number 1, on host 7 |
| 47/1 | occupancy for the same lands: maximum 64, current the number of members in the land's waiting room (0 for a land nobody entered; the room itself holds 128, the size of the stock joinNet) |
| 40/4 | nothing (`PERSONA_SET_UNANSWERED_ASSUMED`). The name replaces the account's persona for the session, and is also written to the store as the account's persona (`Store::set_persona`) when it is 1 to 16 printable ASCII characters with no `\|`, no leading or trailing space; any other name only renames the session. It applies to every account, not only web accounts, so that phone-signup accounts (which start with an empty persona) are named the same way; the last name presented wins, and a name need not be unique |
| 40/3 | the bankroll from the store as `HostMessage::Money`; 500 (`MONEY_STARTING_BALANCE`) for an account with no balance yet, and nothing at all when the store fails, so the client keeps its own copy |
| 40/2 | Ack 40/2 after the bankroll is stored, because the client waits for a reply naming command 40; Nak 40/2 with a text when the store fails |
| 36/2 | `HostInfo` type 2: the host's wall-clock time as `b year-1900, b month0, b mday, b hour, b minute, b second` |
| 36/1 | nothing; the client keeps its files |
| 45/1 | the account's mailbox number (status 1), assigned once and then stable; status 2 on the open book and 4 when the store fails |
| 37/17, 18, 19, 25 | the account's box, the new-mail status, the listing and the letter's text; Nak 37 with a fault number for another account's box, a missing letter or box |
| 37/20, 21, 24, 27 | Ack 37 naming the box that received the letter; the letter is stored for the addressee's box |
| 37/26, 30 | nothing: the letter is deleted, the service form is dropped |
| 37/31, 32 | an empty services listing and an empty system list |

`SetInt` (13), `SetStr` (14), `getProp` (32) and member properties (31) are mirrored and answered as
[objects.md](objects.md) describes. Command 41 answers a service lookup and a conference name check.
A command the codec does not know is logged as "not decoded" and gets no reply. At logon the client
still sends 34/4, which is not answered, and in the Clubhouse 26 is absorbed; the client carries on
(`captures.md` section 9). Mail (37, 45) is answered on a host with `--data-dir`; without one every 37 request is refused
and 45/1 answers status 2 ([mail.md](mail.md) section 6).

## 6. Capture files

One file per connection under the capture directory (default `work/captures/`, which git ignores), named
`<UTC start time>-session<N>.hexlog`. The first line is `# innkeeperd session N from <peer>, started <time>`.
Every other line starts with the seconds since the connection opened and a tag. Bytes of one direction that
arrive within 50 ms of each other share one line group stamped with the first byte (a 2400 bps client
delivers a byte every 4 ms), and an event ends the group:

| Tag | Meaning |
|---|---|
| `tx` | bytes the client transmitted, up to 16 per line, as hex plus an ASCII column |
| `rx` | bytes the client receives from the server, same layout |
| `ev` | a decoded event: line detection, AT or PAD command, call state, link frame, `message len=N <bytes>`, then `client <decoded message>` (or `client message not decoded: <reason>`; the encoded password is shown as `..`) and one `host <reply>` per reply |

`tx` and `rx` are named from the client's point of view, as everywhere in this project.
Example, the end of a real session (`docs/protocol/captures.md`):

```
     3.753 tx 63 20 53 49 45 52 52 41 0d                      |c SIERRA.|
     3.784 ev PAD: PAD command "c SIERRA"
     3.784 rx 0d 0a 53 49 45 52 52 41 20 43 4f 4e 4e 45 43 54 |..SIERRA CONNECT|
     3.784 rx 45 44 0d 0a                                     |ED..|
     5.437 tx 81 58 d8 00 21 35 00 00 00 01 02 03 12 a1 86 01 |.X..!5..........|
     5.437 tx 00 01 1d 7a 01 66 16 66 18 73 03 00 00 67 75 79 |...z.f.f.s...guy|
     5.437 tx 62 72 75 73 68 00 82                            |brush..|
     5.594 ev link: frame DATA 0, 34 payload bytes
     5.594 rx 81 49 62 90 82                                  |.Ib..|
     5.594 ev message len=33 35 00 00 00 01 02 03 12 a1 86 01 00 01 1d 7a 01 66 16 66 18 73 03 00 00 67 75 79 62 72 75 73 68 00
```

## 7. Not done yet

- Password hashing (Phase 7): the encoded password is stored as the client sends it. A second Login for an
  account that is already online is not refused.
- Replies still open in `messages.md` section 3.3: 34/4 (rates), hints (38) and LSCI's own
  bulletin-board subs 27/6, 27/8 and 27/9. See [coverage.md](coverage.md) section 3.2.
- Mail beyond [mail.md](mail.md): letters arriving while the recipient is online are not pushed. (New
  accounts start with INN Stamps, `userFlags` `0x200`, so they can send.)
- On the INT 14h transport, SwitchHost and Connect accept any address and keep the client on this host; a
  land switch between hosts is not modelled. No client speaks the transport yet (ScummVM's online executive
  is the first one planned).
- Sign-in (section 8): Apple, and a periodic vault refresh (the admin allowlist from the vault is
  also read at startup only, section 9). The age and terms answer is required before an account or password is issued (8.6).

## 8. Sign-in

The browser page signs a player in with a common provider, receives a session token, and opens the
WebSocket with it. The page side and the JSON are the contract in the sibling repo `INN-Decomp`,
`docs/web.md` section "Sign-in (server contract)". This section is the server half. What is stored
is [privacy.md](privacy.md) sections 1 and 6; the sandbox and outbound connections are
[hardening.md](hardening.md).

### 8.1 Flags

- `--auth-config FILE` turns sign-in on. It needs `--data-dir` (identities and sessions are rows in
  `innkeeper.db`) and `--ws-bind` (the `/auth/*` endpoints share that listener).
- `--page-origin ORIGIN`, repeatable: the exact origin of a page allowed to sign in, such as
  `https://inn.example.net` or `http://127.0.0.1:8080`. It is the CORS allow list, the check on the
  login's `return` address and the `postMessage` target. No origin means no page can sign in, and
  startup warns.
- `--enrolment open|web-only|closed`. `open` is the old behaviour: a Login for an unknown number
  creates the account, and sign-in also creates accounts. `web-only` creates accounts only through
  `POST /auth/accounts`; a Login never does. `closed` creates none. The default is `open`, or
  `web-only` with `--public`.
- `--session-hours 12`: how long a session token lives. Logout ends it at once.
- `--minimum-age 18` and `--terms-version 1`: the least age and terms version an identity must have
  attested before the server opens an account or issues a password (8.6).
- `--accounts-per-day 3`: accounts one identity may open in any 24 hours. `canCreate` is false and
  `POST /auth/accounts` is 429 `rate_limited` beyond that.
- `--key-vault https://NAME.vault.azure.net`: read the identity HMAC key, the client secrets and the
  audit key from the server vault (8.3). It cannot be combined with `--audit-public-key`.
- `--insecure-dev-keys`: no vault. The identity key is `dev-identity.key` in `--data-dir` and the
  client secrets come from the auth file or the environment (8.2). Every one of those logs
  `DEVELOPMENT KEY`.

**`--public` and the vault.** `--public` refuses to start without `--key-vault`. The one exception
is `--insecure-dev-keys` when every `--page-origin` is a loopback origin (`127.0.0.0/8`, `::1` or
`localhost`). Refusing `--insecure-dev-keys` with `--public` outright would leave no way to run the
public shape, jail included, on one machine without Azure. Accepting it with any origin would let a
public page sign players in against a key on disk. A loopback-only page cannot be reached from
another machine, so the dev keys can never back a public deployment.

Without `--public`, sign-in is optional. A WebSocket with `?session=` is bound the same way, an
unknown or expired token is a 401, and one with no token behaves as before. With no
`--auth-config`, a token on the upgrade is ignored.

### 8.2 The auth file

TOML. `public_url` is the auth origin as the provider sees it, and the callbacks are
`<public_url>/auth/callback` (players) and `<public_url>/admin/auth/callback` (the admin portal, section 9;
its origin is `admin_public_url` below when the portal has its own host name). It defaults to
`http://<--ws-bind>`. Register the player callback with each provider, and the admin one too.
An optional top-level `admin_public_url` names the portal's origin as the browser sees it (for
example `https://host.example`); it defaults to `public_url`, and when it is `https` the cookie is
`__Host-` prefixed.

```toml
public_url = "https://inn.example.net"

# OpenID Connect: discovery from the issuer, scope openid only.
[[provider]]
id = "google"
name = "Google"
issuer = "https://accounts.google.com"
client_id = "1234.apps.googleusercontent.com"
client_secret_name = "oidc-google"

# Plain OAuth 2.0: the stable user id from the profile endpoint, nothing else.
[[provider]]
id = "discord"
name = "Discord"
kind = "oauth2"
issuer = "https://discord.com"
authorize_url = "https://discord.com/oauth2/authorize"
token_url = "https://discord.com/api/oauth2/token"
userinfo_url = "https://discord.com/api/users/@me"
id_field = "id"
scopes = ["identify"]
client_id = "5678"
client_secret_name = "oauth-discord"

# Development only; refused with --key-vault.
[dev_secrets]
oidc-google = "not-a-real-secret"
```

- `id` is 1 to 32 of A-Z, a-z, 0-9, `-` and `_`, unique. The page shows it as `AT+CARD=<id>`.
- `name` is the calling card text, 1 to 64 characters.
- `issuer` is required for both kinds. For OIDC it is the issuer URL that discovery starts from and
  that the ID token must name. For plain OAuth it is a stable name for the provider; it never
  changes, because it is part of every stored identity key.
- `kind` is `oidc` (the default) or `oauth2`. An `oidc` provider needs `issuer` and has no
  `scopes`: the request is `openid` alone. An `oauth2` provider needs `authorize_url`,
  `token_url` and `userinfo_url`; `id_field` names the JSON field with the user id (default
  `id`). Its `scopes` must be the minimum the provider needs for that id, and `email`, `profile`,
  `address` and `phone` are refused.
- `client_secret_name` is the secret's name in the server vault (1 to 127 of letters, digits
  and `-`). Without a vault it is looked up
  in `[dev_secrets]`, then in `INNKEEPERD_SECRET_<NAME>` (upper case, `-` as `_`).

Each login uses PKCE (S256), a random `state` and, for OIDC, a `nonce`; the ID token's signature,
issuer, audience, expiry and nonce are checked. Discovery metadata and signing keys are cached for an
hour and fetched again when a token names an unknown key. Redirects are not followed. A login that
has not come back within ten minutes is forgotten.

Apple is not supported: its client secret is a signed JWT, which this file cannot express yet. For
Microsoft, use a tenant-specific issuer, or for personal accounts the consumer tenant's
`https://login.microsoftonline.com/9188040d-6c67-4c5b-b112-36a304b66dad/v2.0`. Discovery must return the
configured issuer exactly: the `/consumers/v2.0` document names that GUID issuer instead of itself, and the
multi-tenant `common` issuer does not match the tokens it signs.

### 8.3 Key Vault

The client is plain REST (`reqwest` with rustls, Key Vault API version 7.4), not the Azure SDK.
Startup reads a handful of secrets once, so it needs two token requests and a few GETs. The SDK
crates would bring their own HTTP stack and async runtime choices, and a much larger dependency
tree, for that.

Credentials, chosen from the environment:

- managed identity through IMDS (`http://169.254.169.254/metadata/identity/oauth2/token`,
  `api-version=2018-02-01`, resource `https://vault.azure.net`). `AZURE_CLIENT_ID` alone picks a
  user-assigned identity;
- a service principal for a self-hosted server: `AZURE_TENANT_ID` and `AZURE_CLIENT_ID`, with
  `AZURE_CLIENT_SECRET` or `AZURE_CLIENT_CERTIFICATE_PATH` (a PEM with the certificate and its
  RSA private key; the client signs an RS256 assertion). The token comes from
  `https://login.microsoftonline.com/<tenant>/oauth2/v2.0/token`.

Secrets read, all before the sandbox and held in memory only (zeroized on drop where the type
allows):

| Name | What |
|---|---|
| `identity-hmac-key` | base64, at least 32 bytes. Every enabled version is read, newest first; the newest is current |
| each `client_secret_name` | the provider's client secret |
| `audit-public-key` | SPKI PEM; replaces `--audit-public-key` |
| `audit-key-id` | the escrow key version URL; replaces `--audit-key-id` |

A missing secret or a vault error stops startup. There is no
periodic refresh: rotation is a restart. Old HMAC versions stay readable so an identity hashed under
one is found and rewritten under the current version at its next sign-in (privacy.md section 7).
The handle (`Caller 7F3A`) is taken from the key, so it changes after that rewrite.

The hidden flags `--test-vault-endpoint`, `--test-imds-endpoint` and `--test-authority-endpoint`
point the client at a local mock; the tests use them and never reach Azure.

### 8.4 Accounts, sessions and the Login

`POST /auth/accounts` numbers a new account like signup (one past the highest id, at least
100001) and gives it a random password nobody knows. `POST /auth/accounts/<id>/credential` sets a
new random password of 8 characters from A-Z and 0-9, stores it in the client's encoded form and
returns it. Every call rotates it. The body of `POST /auth/accounts` may be empty, `{}` or carry a
null `persona`: the account then has no persona yet (the row's `persona` is empty, and the session
JSON's `accounts` entry says `"persona": null`). The browser page now sends `{}`: the original
client's own Select Player screen (Create) makes the persona, and the name it presents becomes the
account's persona through 40/4, below. A `persona` in the body still works, for compatibility, with
today's checks: 1 to 10 of A-Z, a-z and 0-9, unique without regard to case across every account row,
deleted or not (`400 bad_persona`, `409 persona_taken`). A name that came from the client is not
checked for uniqueness: the original let two players share a name, and the phone book and the board
posts already cope with that (privacy.md section 1).

The session token is 32 random bytes, base64url. The `sessions` row keeps only its SHA-256. The
WebSocket upgrade takes it from `?session=` and nowhere else; the token is never logged. On a bound
line a Login for an account the identity does not own gets the same reply as a wrong password
(`That password is not right.`). The Login's name, which the client takes from the persona it picked
in Select Player, is accepted as it is; the stored persona changes only on a 40/4 (the 40/4 row of
section 5). One INN account can hold several personas in the client's `PLAYER.DIR`; the server keeps
the name the client last presented, for display (the session list, the operator portal, the phone
book). The page keeps each account's `PLAYER.DIR` and persona files in that account's own save space,
so the lists never mix (INN-Decomp `docs/web.md`, "The persona on the drive").

### 8.5 The mock provider

`innkeeperd dev-oidc --bind 127.0.0.1:2902` is a development OpenID Connect provider on loopback,
for local runs and the integration test. Its consent page has a Subject field and Allow and Deny
buttons, and signs in whoever is typed there. It refuses any scope but `openid`, requires PKCE S256
and signs RS256 ID tokens with a key made at startup. It takes any `client_id` (the token request
must repeat the one that started the login) and does not check the secret. Point an `oidc`
provider's `issuer` at it (`--issuer` sets what it announces).

A full local run, with a page served from `http://127.0.0.1:2901`:

```
innkeeperd dev-oidc --bind 127.0.0.1:2902 --issuer http://localhost:2902
innkeeperd --public --insecure-dev-keys --ws-bind 127.0.0.1:2900 --data-dir work/signin \
    --auth-config auth.toml --page-origin http://127.0.0.1:2901
```

`crates/innkeeperd/tests/signin.rs` runs that shape under the sandbox and walks the whole contract,
then checks that the log holds no token, password, secret or subject.

### 8.6 Server-side sign-in

What the server decided where the `docs/web.md` contract is open or silent:

- `GET <url>&return=…` answers with a 302 to the provider. There is no consent page of the
  server's own; the provider's is the consent page. `mode` other than `redirect` is treated as
  `popup`.
- Callback errors are `access_denied` when the provider says so, `provider_error` for any other
  provider failure (including a bad token), and `server_error` when the store fails. A callback
  with an unknown or expired `state` is a 400 HTML page, because there is no verified page to post
  to.
- An `attest` on the login URL that does not parse is ignored. `POST /auth/attest` with one is 400
  `bad_attest`.
- `canCreate` is false when enrolment forbids it or the identity reached `--accounts-per-day`; the
  429 body is `rate_limited`.
- 401 bodies are `unauthorized`; a credential request for an account the identity does not own is
  404 `no_such_account`.
- More than 4096 logins in flight is 503 `busy`.
- **Attestation is enforced.** `POST /auth/accounts` and `POST /auth/accounts/<id>/credential` are
  403 `attest_required` unless the identity's stored answer (from `attest=` on the login URL or
  `POST /auth/attest`) has age at least `--minimum-age` and terms at least `--terms-version`. The
  message is plain: "The desk needs your word first: you are 18 or older, and you accept terms
  version 1 or later. Say so, then ask again." The check runs whenever sign-in is configured, with or
  without `--public`: the paths exist only with `--auth-config`, so one rule is simpler than two. It
  comes after the enrolment check, so a closed host still says `enrolment_closed`. The session
  JSON's `attested` field is unchanged, and a Login on an existing account is not gated.
- `GET /auth/accounts/<id>/saves` (and `?path=P` for one file) hands a signed-in page the account's
  current save files before it dials, so a fresh browser does not overwrite them; 401 without a
  live token, 404 `no_such_account` when the identity does not own the account. The shape is in
  [saves.md](saves.md) section 6. Only counts and sizes are logged.
- `POST /auth/logout` is 204 with or without a valid token; only a store failure is a 500.
- A preflight from an origin that is not a `--page-origin` gets 204 without
  `Access-Control-Allow-Origin`, so the browser blocks it. `Access-Control-Max-Age` is 600.
- The token lives `--session-hours` (12). A token presented after logout or expiry is 401 on
  every call and on the upgrade.
- The upgrade is accepted on any path; only `?session=` matters.
- Without `--public`, an unknown or expired token on the upgrade is refused (401) rather than
  ignored, so a stale tab does not silently log on unbound.
- Deleting an identity's last account (`account delete`) revokes its sessions; once nothing refers
  to it, the purge job removes the identity.

## 9. Admin portal

A small web office for the operator, served by `innkeeperd` on **its own listener** under `/admin/`.
It is off unless `--admin-bind` is given. It shares nothing with player sessions: its own cookie, its
own tables, its own callback.

### 9.1 Flags

- `--admin-bind ADDR` listens for the portal (for example `127.0.0.1:2317`). It needs
  `--auth-config` (the same providers sign the operator in), must differ from `--ws-bind`, and under
  `--public` must be a loopback address. The reverse proxy routes `/admin/*` to it.
- `--admin-allowlist FILE` names a file of staff identity keys (needs `--admin-bind`). One key per
  line, `#` starts a comment. The file is re-read whenever its size or modification time changes, so
  an edit takes effect on the next request. An unreadable file lists nobody.
- `--admin-dev-open` opens the door to any signed-in identity for local work (needs `--admin-bind`).
  Refused with `--public`.
- `--retain-admin-audit-days N` (365) keeps the audit log ([privacy.md](privacy.md) section 6).
- Optional vault secret `admin-identities` (section 8.3): the same one-key-per-line list, read at
  startup. A missing secret is not an error.

### 9.2 Who may come in

There is no sign-up. A person signs in with one of the configured providers, and the portal then
decides from the identity's key (the full hex `identity_key`, an HMAC of provider and subject, never
the subject itself). The allowlist is the union of: the vault secret, the `--admin-allowlist` file
and the active rows of the `staff` table (`innkeeperd admin allow KEY [--note TEXT]`,
`innkeeperd admin revoke KEY`, `innkeeperd admin list`, all with `--data-dir`, which also work
while the server runs). **Precedence:** a revoked `staff` row beats everything, even a key still in
the vault or the file, so revocation always works; otherwise any one source that lists the key
lets it in. The vault list is read at startup only: to remove a vault entry for good, edit the
secret and restart, or `admin revoke` it meanwhile. The decision is made again on **every
request**; a revoked operator's session row is deleted and the next click lands on the login page.
After an HMAC key rotation the candidate keys of older versions are also tried; re-add the new key
when convenient.

A signed-in identity that is not listed sees a polite 403 page in the narrator's voice with its own
key, a Copy button and the three ways to add it. Only the handle is logged. This is how you get
your key the first time: sign in once, copy it, put it in the file or run `admin allow`.

**Dev fallback.** With no vault list, no file and no `staff` rows of any kind, the door is open to
any signed-in identity, the log carries a loud WARN and every page wears a red "DEV: open door"
banner. `--public` refuses to start in that state. A list that exists but is empty means closed.

### 9.3 Caddy

```
handle /admin/* {
    reverse_proxy 127.0.0.1:2317
}
```

Add it beside the player routes; pass `Host` and `X-Forwarded-Proto` through as usual.

### 9.4 What it does

Menu: Ledger (dashboard, audit), Desk (complaints, holds), Records (accounts, posts, letters, saves),
Help (Time of day toggle: system, day, dusk; saved as `inn.theme`).

- **Dashboard:** who is online with a Kick button, counts, links to `/healthz` and `/admin/healthz`.
- **Accounts:** search by number or persona; an account the client has not named yet shows
  "(not named yet)"; detail page; suspend and unsuspend (a suspended
  account's Login is refused with a Nak and its lines are kicked); stamps, staff level and land
  rights; CasinoBucks; password reset (the new password is shown once); revoke web sessions;
  soft delete and restore (the account tools of [privacy.md](privacy.md) section 3).
- **Complaints and Member Services:** the operator board (location 0x63, sections 2 and 3). "Mark
  handled" records a mark beside the message and never alters the original.
- **Posts:** browse, read, soft-delete.
- **Letters:** a metadata list (sender, recipient, size, date). Reading a body needs a typed reason of
  at least 8 characters, written to the audit log **before** the body is shown.
- **Legal holds:** list, place (with a reason) and release, through the existing hold code.
- **Saves:** list and download; no editing. Each download is audited.
- **Audit:** every state-changing action and every letter read or save download, with who, what,
  target and reason. No bodies, subjects or tokens are ever written.

Online accounts: flags and land rights, CasinoBucks and password reset are **refused (409)** while
the account is logged on (the live session would overwrite or contradict the change). Suspend,
revoke sessions and delete kick the live lines first, then apply. Unsuspend and restore need nothing.
Destructive actions ask for confirmation: a dialog, or a visible "I mean it" tickbox without script.

### 9.5 Security model

- Cookie `__Host-inn-admin` (`Path=/; Secure; HttpOnly; SameSite=Strict`) when the portal origin is
  https; over plain http on loopback it is `inn-admin` with `Path=/admin/` and no `Secure`. The value is an opaque
  random token, stored only as a hash. Lifetime 8 hours, 30 minutes idle; Sign out deletes the row.
- CSRF: every POST needs the per-session token (derived from the session token, not stored). The
  `Origin` must equal the portal origin. Because the portal sends `Referrer-Policy: no-referrer`,
  browsers send `Origin: null` on same-origin form posts; that (or no Origin) is accepted only when
  `Referer` matches or `Sec-Fetch-Site: same-origin` says so. Anything else is 403.
- Headers on every answer: CSP `default-src 'self'; script-src 'self'; style-src 'self'
  https://fonts.googleapis.com; font-src https://fonts.gstatic.com; img-src 'self' data:;
  connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors
  'none'`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
  `X-Frame-Options: DENY`, `Cache-Control: no-store`. No inline script or style, no third-party script.
- Every stored string is HTML-escaped (tests use hostile strings). Assets are embedded in the binary.
- Letter and post bodies and raw subjects never reach the log.
- The callback is `/admin/auth/callback`, a separate audience from the players'.

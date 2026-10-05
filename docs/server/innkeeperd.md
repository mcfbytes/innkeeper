# innkeeperd

The host daemon. It accepts the stock DOS client over TCP, answers the modem, PAD and link layer, plays
the host side of the logon, and records everything. With it the stock client logs in, reaches the town map
and enters the Clubhouse waiting room (`docs/protocol/captures.md` sections 9 to 11). It serves one player
at a time: nothing is shared between connections yet.

## 1. Crates

```
innkeeperd           bin: command line, tokio runtime, TCP, capture files, wiring
  ├ innkeeper-world    host behaviour: typed messages, accounts, logon, land tables, no I/O
  └ innkeeper-session  one connection: line then link, no I/O
      ├ pad_thai         Hayes modem (hayes_fever), SprintNet PAD dialogue, line detection, no I/O
      └ tsn-link         TSNEXEC framing, CRC, ACK/NAK, resends, message packing, no I/O
int14h               the 17 INT 14h exports and the transport codec (int14h-transport.md), no I/O
```

`innkeeper-world` does not depend on the session crates: it turns message bodies into replies, and
`innkeeperd` (`src/host.rs`, `Host`) carries them between the two. A delivered client message is parsed
into a `ClientMessage`, handed to the connection's `PlayerSession` with the shared `World`, and every
`HostMessage` that comes back is encoded, queued with `Session::send_message` and sent with
`Session::flush`, which closes the frame.

Every crate below `innkeeperd` is synchronous and deterministic: bytes and an `Instant` go in, bytes and
events come out, and timers are a `next_deadline()` that the caller sleeps on. `innkeeperd` depends on
`tsn-link` directly only to choose log levels for link events.

## 2. Running

```
cargo run -p innkeeperd -- [--bind 127.0.0.1:2314] [--capture-dir work/captures] [--no-capture]
                           [--line auto|hayes|pad] [--connect-rate 2400]
```

- On a successful bind it logs `INT 14h hooked. Please wait while ImagiNation loads...`.
- `RUST_LOG=debug` adds one line per received frame; the default level `info` shows line and PAD events
  and every decoded client message.
- `--line` says what the client's serial port reaches (section 4). `--connect-rate` is the rate in the
  emulated modem's `CONNECT` line, which the client stores as its line rate (link-layer section 4 notes).

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
| link | hunts for `81`, verifies the CRC, ACKs every good DATA frame (duplicates too), NAKs bad ones, delivers in-order payloads as messages | checklist 7, 8, 10 |
| escape to the PAD | a CR while no frame is open, or a modem BREAK, returns to the `@` prompt with the call still up; `SET?` gets a prompt, `D` gets `<host> DISCONNECTED` | checklist 12 |
| `DIRECT` | a byte with bit 7 set during wake-up starts the link at once | checklist 14 |

The INFERRED parts (prompt texts, the CR escape heuristic, the `DIRECT` detection) are named constants in
`crates/pad_thai/src/assumptions.rs` and `crates/tsn-link/src/assumptions.rs`.

Host-side link timing (`tsn_link::LinkConfig`): no DATA frame for 1.5 s after a call connects unless the
client sends first, a resend after 3 s without an ACK, and at most 10 resends. These are server choices,
not facts about the original. In practice the client always sends first, so replies leave at once, one
DATA frame per client message.

## 5. The host side (`innkeeper-world`)

| Module | Holds |
|---|---|
| `message` | `ClientMessage` (parse, encode) and `HostMessage` (encode, parse) for the commands below, on a `WireReader` and `WireWriter` of the `b`/`w`/`a`/`s` field codes; `Command` is the table of command bytes. One file per family (`send`, `group`, `properties`, `multicast`, `service_lookup`, `notice`, `ack`, `object_kind`); the type names are in `messages.md` section 3.2.1 |
| `account` | `AccountBook`: who may log in. `anyone()` admits every account number and password, the stand-in while nothing persists; `listed(...)` checks number and encoded password |
| `land` | `LandCatalog`: the land directory and occupancy |
| `player` | `PlayerSession`: one client from Login to hang-up, as an enum of `AwaitingLogin` and `LoggedIn`; owns the client's SIDs and group memberships |
| `world` | `World`: host number, accounts and lands, shared read-only by every connection |
| `assumptions` | the INFERRED values the replies encode, each naming its section of `messages.md` |

What the host answers (`docs/protocol/messages.md` for the layouts):

| Client sends | Host replies |
|---|---|
| Login (53, 59) | Ack `whichCmd` 22 with all fields 0; a refused account gets Nak 22: reason 9 (the client asks for the password again, three tries) for a wrong password, reason 1 with a text for an unknown account |
| joinNet (7) | `ObjID` with the next free SID, counting up from `0x0100` per connection and skipping SIDs still in use after the wrap |
| leaveNet (9) | nothing; the SID and its group memberships are forgotten |
| add (10) to a group | `GrpJoin` for the member, delivered to the group |
| 36/5 | `HostInfo` type 5: host number 7, the first host of the stock `HOSTADDR` |
| 36/6 | the land directory, always (the stamp is not compared): Clubhouse, SierraLand and CasinoLand, land number 1, on host 7 |
| 47/1 | occupancy for the same lands: maximum 64, current 0 |
| 36/1, 36/2 | nothing; the client keeps its files |

Commands 2, 11, 12, 13, 14, 28 and 41 decode (`messages.md` section 3.2.1), are logged and get no reply
until the object store routes them. Every other command is logged as "not decoded" and gets no reply. At
logon that is 34/4, 45/1, 37/32 and 40/4, and in the Clubhouse 26; the client carries on without answers
(`captures.md` section 9).

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

- Anything shared between players: each connection has its own `PlayerSession`, SIDs are only unique per
  connection, and a `Send` (2) or a group change is not routed to anyone. The next step is a world task that
  owns SIDs and groups, fed by the connections through a channel.
- Persistent accounts: `AccountBook::anyone()` admits everybody; a store on disk comes with a second
  `AccountBook` source.
- Replies to 34/4 (rates), 45/1 (mailbox), 37 (mail), 40/4 (name), `getProp` (32) and the other services of
  `messages.md` section 3.3.
- Serving the INT 14h transport.

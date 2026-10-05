# innkeeperd

The host daemon. Version 0 is a capture tool: it accepts the stock DOS client over TCP, answers the
modem, PAD and link layer well enough for the client to reach the host and start sending, and records
everything. It has no application logic yet; it acknowledges frames and logs the messages inside them.

## 1. Crates

```
innkeeperd           bin: command line, tokio runtime, TCP, capture files
  └ innkeeper-session  one connection: line then link, no I/O
      ├ pad_thai         Hayes modem (hayes_fever), SprintNet PAD dialogue, line detection, no I/O
      └ tsn-link         TSNEXEC framing, CRC, ACK/NAK, resends, message packing, no I/O
int14h               the 17 INT 14h exports and the transport codec (int14h-transport.md), no I/O
```

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
not facts about the original. Version 0 sends no DATA frames, only ACK and NAK.

## 5. Capture files

One file per connection under the capture directory (default `work/captures/`, which git ignores), named
`<UTC start time>-session<N>.hexlog`. The first line is `# innkeeperd session N from <peer>, started <time>`.
Every other line starts with the seconds since the connection opened and a tag. Bytes of one direction that
arrive within 50 ms of each other share one line group stamped with the first byte (a 2400 bps client
delivers a byte every 4 ms), and an event ends the group:

| Tag | Meaning |
|---|---|
| `tx` | bytes the client transmitted, up to 16 per line, as hex plus an ASCII column |
| `rx` | bytes the client receives from the server, same layout |
| `ev` | a decoded event: line detection, AT or PAD command, call state, link frame, or `message len=N <bytes>` |

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

## 6. Not done yet

- Any application message. The client's Login is captured (`docs/protocol/captures.md`), but nothing answers
  it, so the logon script times out after 70 s (`script.101` "LoginTimeout", error 999).
- Serving the INT 14h transport.

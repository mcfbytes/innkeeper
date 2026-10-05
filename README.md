# innkeeper

> *Look behind you, a three-headed modem!*

A revival of The Sierra Network / ImagiNation Network: a server that keeps the INN open, plus the
reverse-engineering tools and notes behind it. The client side runs in a ScummVM fork
(`mcfbytes/scummvm`, branch `lsci`) or as the original DOS client under DOSBox.

| Path | What |
|---|---|
| `docs/PLAN.md` | Plan of attack and phase status |
| `docs/` | Format, protocol and convention notes |
| `crates/` | The server, a Rust workspace (below) |
| `tools/` | Python extraction and analysis tools for the original media |
| `tools/dosbox/` | Runs the original client in headless DOSBox-X against the server; see `docs/dosbox.md` |

## Crates

| Crate | What |
|---|---|
| `innkeeperd` | The daemon: one session per TCP connection, logs and hex captures. Run it with `cargo run -p innkeeperd`; see `docs/server/innkeeperd.md`. |
| `innkeeper-session` | One client connection as a sans-IO state machine: phone line first, then the host link. |
| `pad_thai` | The SprintNet X.25 PAD dialogue, plus `hayes_fever`, a Hayes modem for raw serial and null-modem links. |
| `tsn-link` | TSNEXEC's link layer: `81 crc crc ctrl … 82` frames, CRC-16, stop-and-wait ACK/NAK, message packing. |
| `int14h` | TSNEXEC's 17 INT 14h exports as typed calls, and the transport that carries them (`docs/protocol/int14h-transport.md`). |

Everything below `innkeeperd` is synchronous and does no I/O. `cargo test` runs the golden byte tests.

Game media and anything extracted from it are never committed. Bring your own disks.

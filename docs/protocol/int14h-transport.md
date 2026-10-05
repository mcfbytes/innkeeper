# INT 14h transport

A byte-stream protocol that carries TSNEXEC's 17 export calls between a client that implements the
INT 14h API locally and `innkeeperd`. It replaces the modem, the X.25 PAD and the link layer
(`docs/protocol/link-layer.md`) with plain calls and replies. Users:

- ScummVM's built-in "virtual TSNEXEC", which serves `kTSN` (`docs/protocol/ktsn.md`).
- An optional DOS-side helper (a replacement TSNEXEC TSR, or a DOSBox-X patch that services INT 14h).

The export semantics are those of `docs/protocol/int14h-api.md`; this file only defines how they travel.
The Rust types and codec are in `crates/int14h` (`Call`, `Reply`, `Envelope`, `encode_envelope`,
`EnvelopeParser`). Status: version 1, specified and implemented as a codec; `innkeeperd` does not
listen for it yet.

## 1. Model

- **Client** is the side that runs the INN programs. It answers INT 14h itself, forwards each export
  call as a CALL envelope and returns the REPLY result to the caller. **Server** is `innkeeperd`.
- Every CALL gets exactly one REPLY with the same tag. The server handles calls strictly in arrival order
  and replies in that order, so a client may pipeline calls; tags only help matching and logging.
- **Handles stay on the client.** On Send the client dereferences the handle and sends the bytes, and frees
  the handle on the next Poll as TSNEXEC does (int14h-api section 4). On Receive the client allocates through
  the program's `alloc` callback and copies the bytes in. SetCallbacks carries only whether the three
  callbacks are installed.
- **The clock stays on the client.** SetAckTimeout's result pointer (TSNEXEC's tick counter) is local; the
  ticks argument is forwarded for information only.
- **No link layer.** Nothing is framed or acknowledged inside this protocol; TCP or WebSocket provides
  reliability. Poll and Service report status 1 (carrier lost) when the server ends the session; statuses
  2 and 3 never occur. IsTransmitIdle is true whenever the server holds no unsent data for the client.
- **Connect and SwitchHost** carry the original dial string and host address. The server may use the host
  part (link-layer section 3.2) to pick a land. Results use the codes of int14h-api section 6.
- **Executive-local exports** (1 GetSharedData, 2 SetSharedData, 6 SetAckTimeout, 9 SetNextProgram,
  11 GetPreviousProgram, 15 SetCallbacks) only touch TSNEXEC state. A client may answer them itself and
  never send them. A server must still answer them if they arrive, by keeping that state per session.
- The reentrancy lock (int14h-api section 5) is not modelled: the client serialises its own calls.

## 2. Stream framing

| Carrier | Framing |
|---|---|
| TCP | each envelope is preceded by its length as a `u32` little-endian, not counting the prefix itself |
| WebSocket | one binary message per envelope, without the length prefix |

An envelope longer than `0x10100` bytes (a maximal message plus headers) is a protocol error.

## 3. Envelope

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | kind |
| 1 | 2 | tag, `u16` little-endian |
| 3 | rest | body, by kind |

| Kind | Name | Direction | Body |
|---|---|---|---|
| `01` | HELLO | client to server | magic `49 31 34 48` ("I14H"), version `u8`, client name `str8` |
| `02` | WELCOME | server to client | version `u8`, server name `str8` |
| `10` | CALL | client to server | export index `u8` (0 to 16), then the arguments in section 5 |
| `11` | REPLY | server to client | export index `u8`, then the result in section 5 |

- The client opens with HELLO (tag 0) and the server answers WELCOME (tag 0) before any CALL. Version 1 is
  this document. A client that receives a different version closes the connection.
- Malformed input closes the connection; version 1 has no error envelope. Malformed means: unknown kind,
  wrong magic, export index above 16, a field that breaks section 4, an unknown result code, a body that ends
  early, or bytes left over after the body.

## 4. Field types

| Type | Encoding |
|---|---|
| `u8`, `u16` | unsigned, little-endian |
| `bool` | one byte, 0 or 1; any other value is malformed |
| `str8` | `u8` length, then that many bytes of printable ASCII (`20`..`7E`), no NUL terminator |
| `bytes16` | `u16` length, then that many bytes |
| `opt<T>` | `bool` present flag, then `T` when present |

## 5. Arguments and results per export

| # | Export | CALL arguments | REPLY result |
|---|---|---|---|
| 0 | GetStatus | none | driver version `u8` (MODEM.DRV reports 3), connection byte `u8` (`80` connected, `00` not) |
| 1 | GetSharedData | none | shared block `bytes16`, at most 256 bytes |
| 2 | SetSharedData | shared block `bytes16`, at most 256 bytes; empty clears it | none |
| 3 | Connect | dial string `str8`, at most 127 bytes | result `u8`: 0, 5 to 10, 12, 13, 15 to 19 or 23 |
| 4 | Send | message body `bytes16`; empty is answered `false` | queued `bool` |
| 5 | Receive | none | oldest complete message `opt<bytes16>`; absent when none is waiting |
| 6 | SetAckTimeout | ticks `u16` | none |
| 7 | Disconnect | none | none |
| 8 | Poll | none | status `u8`, 0 to 3 |
| 9 | SetNextProgram | `TSN.PRG` block name `opt<str8>`; absent cancels | none |
| 10 | Service | none | status `u8`, 0 to 3 |
| 11 | GetPreviousProgram | none | block name `opt<str8>` |
| 12 | IsTransmitIdle | none | idle `bool` |
| 13 | SwitchHost | host address `str8`, at most 127 bytes | result `u8`: 0, 1, 13, 18, 20, 21, 22 or 255 |
| 14 | Flush | none | none |
| 15 | SetCallbacks | installed `bool` | none |
| 16 | GetLineRate | none | line rate `u16` |

The Connect and SwitchHost codes are the values TSNEXEC returns after mapping the driver's `AH`
(int14h-api section 6, meanings in link-layer section 4.1 and 5.1). SetNextProgram's original return value
(always 1) is implied and not sent.

## 6. Examples

These are the golden bytes in `crates/int14h/tests/golden/envelopes.txt`, length prefix included.

| Example | Bytes |
|---|---|
| HELLO, version 1, client "scummvm" | `10 00 00 00` `01` `00 00` `49 31 34 48` `01` `07 73 63 75 6d 6d 76 6d` |
| WELCOME, version 1, server "innkeeperd" | `0f 00 00 00` `02` `00 00` `01` `0a 69 6e 6e 6b 65 65 70 65 72 64` |
| CALL tag 1, Connect "ATDT0" | `0a 00 00 00` `10` `01 00` `03` `05 41 54 44 54 30` |
| REPLY tag 1, Connect: connected | `05 00 00 00` `11` `01 00` `03` `00` |
| CALL tag 2, Send `22 04 10` | `09 00 00 00` `10` `02 00` `04` `03 00 22 04 10` |
| REPLY tag 2, Send: queued | `05 00 00 00` `11` `02 00` `04` `01` |
| CALL tag 3, Poll | `04 00 00 00` `10` `03 00` `08` |
| REPLY tag 3, Poll: status 0 | `05 00 00 00` `11` `03 00` `08` `00` |
| CALL tag 4, Receive | `04 00 00 00` `10` `04 00` `05` |
| REPLY tag 4, Receive: message `07 41` | `09 00 00 00` `11` `04 00` `05` `01 02 00 07 41` |
| REPLY tag 5, Receive: nothing waiting | `05 00 00 00` `11` `05 00` `05` `00` |
| CALL tag 6, SetNextProgram(NULL) | `05 00 00 00` `10` `06 00` `09` `00` |
| REPLY tag 7, GetStatus: driver 3, connected | `06 00 00 00` `11` `07 00` `00` `03 80` |

## 7. Open points

- Version 1 polls: the client learns about new messages only through Poll and Receive, one round trip each.
  A later version may let the server push a "messages waiting" notice.
- There is no authentication; logon happens in the application messages.
- `innkeeperd` does not serve this transport yet, and no client implements it yet.

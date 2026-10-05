# Serial link layer: com driver, dial-up, X.25 PAD, framing

This document covers what crosses the serial line between the stock INN client and the host, from the
first `AT` command to the first data frame after logon. It also lists what a server must do to impersonate
SprintNet and the Sierra host. For the API that client programs call above this layer, see
`docs/protocol/int14h-api.md`. Its export names (Connect, SwitchHost, Service, Flush, ...) are used here.

Primary files (Feb-1994 INN): `work/sets/inn_cd/MODEM.DRV` (5816 bytes), `NOBRK.DRV` (5563 bytes),
`work/exe/TSNEXEC_inn_cd.EXE` and `work/exe/LSCITV_inn_cd.EXE`.

## 0. Address convention

- **Drivers** are flat images that run with `CS=DS=ES` = load segment and offset 0 = file offset 0.
  `MODEM.DRV:0A3E` therefore means both `CS:0A3E` and file offset `0x0A3E`.
- **TSNEXEC** follows the int14h-api convention. `seg:off` is relative to the load image, with code at `0000`
  and data at `031D`. File offset = `0x190 + seg*16 + off`, so `031D:xxxx` is at file `0x3360 + xxxx`.
  Ghidra shows segment `0000` as `1000`.
- **LSCITV** addresses are image offsets. File offset = image + `0x30E0`.
- **Scripts** are cited by offset inside the decompressed resource `work/res/inn_feb94/hub/script.NNN`.
- Driver timing is counted in BIOS ticks from `INT 1Ah` (18.2 Hz). TSNEXEC timing is counted in the
  client's own ticks (section 6.6).

## 1. Who owns which byte

| Layer | Code | What it puts on the wire | Status |
|---|---|---|---|
| UART, Hayes modem, X.25 PAD dialogue, line break | com driver `MODEM.DRV` / `NOBRK.DRV` (functions 3, 7 and 8) | `AT` strings, `@`, `D`, `c <addr>`, `SET?`, BREAK | CONFIRMED |
| Framing, CRC, ACK/NAK, retransmission, message packing | TSNEXEC `0000:1A46`..`2A49` | `81 crc crc ctrl … 82` frames | CONFIRMED |
| Message contents | LSCITV `kTSN` and the scripts; the DOS games' `hostcomm` | opaque, length-delimited messages | CONFIRMED (opaque to this layer) |
| Dial-string composition, host selection | LSCI scripts (`script.012`, `script.101`) with `LSCI.CFG` and `HOSTADDR` | `prefix`, `t`, `hostID`, `modem` | CONFIRMED strings; composition INFERRED |

TSNEXEC hooks no serial or timer interrupt. Only the driver's UART interrupt handler runs asynchronously,
and it only fills and drains two ring buffers. CONFIRMED (`MODEM.DRV:14EF`; int14h-api section 5).

## 2. The Sierra com-driver format

### 2.1 Header and dispatch

| Offset | Bytes (MODEM.DRV) | Meaning | Status |
|---|---|---|---|
| `00` | `EB 61 00 00` | `jmp 0063` | CONFIRMED |
| `04` | `21 43 65 87` | Sierra driver magic `0x87654321`, as in `ADL.DRV`, `VGA320.DRV` and `IBMKBD.DRV` | CONFIRMED |
| `08` | `05` | driver class. In the same set 0 = video, 1 = sound, 4 = keyboard, so 5 = comms. | CONFIRMED value; meaning INFERRED |
| `09` | `09 "modem.drv"` | Pascal string: name | CONFIRMED |
| `13` | `3B "Hayes-compatible modem communications. Southern California."` | Pascal string: description. NOBRK.DRV carries the same text. | CONFIRMED |
| `4F` | 10 near words | function table, index 0–9 | CONFIRMED |
| `63` | `push ds; push es; mov cx,es; mov ds,cs; mov es,cs; shl bx,1; push cx; push si; call [bx+4F]; add sp,4; pop es; pop ds; retf` | entry: `BX` = function number, `ES:SI` = far argument, result in `AX` (fn 1: `DX:AX`) | CONFIRMED |

| Fn | MODEM.DRV | NOBRK.DRV | Purpose | Status |
|---|---|---|---|---|
| 0 | `08E2` | `07D7` | returns 3 (version) | CONFIRMED; "version" INFERRED |
| 1 | `0914` | `0809` | Init(options): program the UART, hook the IRQ, return the port block pointer (`DS:0099`) | CONFIRMED |
| 2 | `1351` | `1254` | Terminate: hang up (fn 7), restore the UART registers (`13AB`), restore the PIC mask and IRQ vector (`132F`) | CONFIRMED |
| 3 | `0A3E` | `0933` | Dial(dial string): Hayes dial, then the X.25 PAD call (section 4) | CONFIRMED |
| 4 | `124E` | `1151` | StartTransmit: wait for THRE, then set IER bits | CONFIRMED |
| 5 | `126B` | `116E` | CarrierPoll: debounce DCD loss and clear the online flag | CONFIRMED |
| 6 | `128B` | `118E` | read and clear the error counters: `AL` = overrun count, `AH` = framing-error count | CONFIRMED |
| 7 | `12D1` | `11D4` | Hangup (section 5.2) | CONFIRMED |
| 8 | `0FC0` | `0EB5` | Reconnect(address): escape to the PAD, disconnect, call a new host (section 5.1) | CONFIRMED |
| 9 | `08E6` | `07DB` | returns the line rate (word `+1C`) | CONFIRMED |

### 2.2 Port block shared with TSNEXEC (`MODEM.DRV:0099`, held at `031D:06D0`)

| Offset | Field | Default | Writer / reader | Status |
|---|---|---|---|---|
| `+00` far | transmit ring `CS:02C2` | | | CONFIRMED (`0938`..`0952`) |
| `+04` | transmit ring size | `0x200` | | CONFIRMED |
| `+06` / `+08` | transmit head / tail | 0 | TSNEXEC writes the head (`0000:1D9C`), the ISR advances the tail | CONFIRMED |
| `+0A` far | receive ring `CS:00C2` | | | CONFIRMED |
| `+0E` | receive ring size | `0x200` | | CONFIRMED |
| `+10` / `+12` | receive head / tail | 0 | the ISR advances the head, TSNEXEC advances the tail (`0000:1F04`) | CONFIRMED |
| `+16` | status: bit `0x80` = online. Set by Dial and cleared by CarrierPoll or Hangup. | 0 | TSNEXEC treats 0 as carrier lost | CONFIRMED |
| `+18` / `+19` | link-layer expected receive sequence / next transmit sequence | 0 | owned by TSNEXEC | CONFIRMED |
| `+1A` | UART base port | `0x3F8` | | CONFIRMED (bytes at `00B3`) |
| `+1C` | line rate | `0x04B0` = 1200 | | CONFIRMED |
| `+1E` | last MSR value | | ISR | CONFIRMED |
| `+20` / `+21` / `+22` | PIC mask bit / interrupt vector / PIC data port | `10` / `0C` / `0021` | | CONFIRMED |

COM port table: bases `3F8 2F8 3E8 2E8` at `0081`, PIC bits `10 08 10 08` at `0089` and vectors `0C 0B 0C 0B`
at `0091`. CONFIRMED.

### 2.3 Init options (fn 1, parser `MODEM.DRV:13DA`)

The option string is the part of the `tsn.cfg` line `comm = MODEM.DRV : b2400 c1` that follows the `:`
(int14h-api 7.1). Tokens are case-insensitive and separated by spaces or tabs.

| Token | Meaning | Status |
|---|---|---|
| `b<decimal>` | line rate. Divisor = `0x1C200 / rate` (`08EA`). | CONFIRMED |
| `c<1-4>` | COM port; out of range means COM1. Sets the port, PIC bit and vector from the table. | CONFIRMED |
| `i<0-15>` | IRQ. 0–7 uses vector `8+n` on PIC `21h`; 8–15 uses vector `70h+n-8` on PIC `A1h`. Above 15 stops parsing. | CONFIRMED |
| `p<hex>` | port base override | CONFIRMED |

Init sequence (`0914`): MCR = loopback (`10h`). Set the divisor and then LCR = `03` (**8N1**). Wait 3 ticks, read
LCR and MSR, then MCR = 0. Hook the vector, set MCR = `0Ah` (RTS, OUT2) and IER = `09h` (receive data and modem
status), and read LCR, MSR, LSR and RBR to clear stale state. Unmask the PIC, then drop DTR for 7 ticks, raise it
and wait 7 more (a hang-up pulse). The online flag `+16` is cleared and `DS:04CD` (PAD mode) is set to 0. CONFIRMED.

### 2.4 UART interrupt handler (`MODEM.DRV:14EF`)

| IIR cause (jump table `1516`) | Action | Status |
|---|---|---|
| receive data (`1577`) | store the byte at `rx[head]` and advance the head unless it would hit the tail (on overflow the byte is dropped). **While a dial or reconnect is in progress (`DS:04CF`=1) bit 7 is stripped.** | CONFIRMED |
| THRE (`15B9`) | if the IER THRE bit is enabled and LSR.THRE is set: send `tx[tail]`, or clear the IER THRE bit when the ring is empty | CONFIRMED |
| modem status (`153F`) | store the MSR. On delta-DCD with DCD now low, arm the 5-poll countdown `DS:04D1` that CarrierPoll uses to clear `+16`. | CONFIRMED |
| line status (`151E`) | count overruns (`DS:007B`) and framing errors (`DS:007C`) | CONFIRMED |

There is **no flow control** of either kind: CTS is never tested, and XON/XOFF bytes are not
interpreted. RTS and DTR stay asserted while online. CONFIRMED (`15B9`..`1601`, `09BA`).

### 2.5 MODEM.DRV versus NOBRK.DRV (and older builds)

Both files have the same strings, the same table layout and the same logic apart from the following.
CONFIRMED by a normalised disassembly diff.

| Point | MODEM.DRV | NOBRK.DRV | Status |
|---|---|---|---|
| Escape from data mode to the PAD (`Reconnect`) | **UART BREAK** (`07D7`): mask the IRQ, wait for THRE, send `00`, set LCR bit 6 for 10 ticks (~0.55 s), clear it and wait 19 ticks (~1 s) | no BREAK routine (107 instructions, 253 bytes removed). Sends `~~~+++~~~AT!~AT\B!~ATO!~` (`055D`, at `0EF8` and `0F7A`) | CONFIRMED (bytes); meaning INFERRED: Hayes escape, `AT\B` = the modem sends the break itself, `ATO` = back online |
| StartTransmit IER bits | `IER |= 0Fh` (`1264`) | `IER |= 02h` (`1167`) | CONFIRMED |
| Default host string at `0628` | `"SIERRA\0"` | `"SIERRA\r"` | CONFIRMED |

`READ.ME` ("I CAN'T GET THERE FROM HERE") describes NOBRK as the "alternate modem driver" for Rockwell-based
modems that cannot pass a break, chosen by the "Generic switchless (alternate)" modem model. That model
uses init string 3 in `MODEM.TXT`, and `INSTTSN.HLP` says it "uses an alternate method of transmitting a
break signal". CONFIRMED (text). The name means "no hardware break". INFERRED.

With MODEM.DRV, the default-host path through `06E0` builds `c SIERRA` followed by a NUL. The CR is appended
after the NUL (`0B7C`), so it is never sent. NOBRK's `SIERRA\r` fixes this. CONFIRMED by code; the
path is reached only when the dial string has no usable `t` field (section 3.2).

Older builds: the Dec-1993 MODEM.DRV differs from Feb-1994 in a single byte, the StartTransmit immediate
at file `0x1265` (`02` instead of `0F`).
TSN 2.1 ships different MODEM.DRV and NOBRK.DRV files that use the same strings. The TSNEXEC framing code
(CRC routine, escape set, ACK type `0x90`) is present unchanged in TSN 2.1. CONFIRMED (byte-pattern match;
MD5s differ).

## 3. Configuration that drives a dial

### 3.1 Files

| Item | Where | Example / format | Status |
|---|---|---|---|
| com driver and options | `tsn.cfg`: `comm = %s : b%s c%d` or `… i%s p%s` | `comm = MODEM.DRV : b2400 c1` | CONFIRMED format (unpacked `INSTTSN.EXE`); example INFERRED |
| modem init prefix | `LSCI.CFG` key `prefix` | one of the 34 strings in `MODEM.TXT`, for example `+++~~~ATZ!~~~~AT&D2!~AT&C1!~ATV1!~` | CONFIRMED key (`INSTALL.SCR` line 373, `script.012` `0x301`); value source INFERRED |
| dial command | `LSCI.CFG` key `modem` | installer format `ATD%c%s!~`, for example `ATDT5551234!~` | CONFIRMED format string; key link INFERRED (`script.012` `0x317`; error text "The 'modem' string in LSCI.CFG file is incorrect" at `0x7CA7`) |
| host to call | `LSCI.CFG` key `hostID` | written as `hostID = SIERRA` | CONFIRMED (`INSTALL.SCR` lines 362, 374, 379) |
| alternate hosts | `HOSTADDR` | `Sierra7  311083420207  CC  7` … `Sierra14 …214` | CONFIRMED; read by scripts, not by TSNEXEC |
| downloaded address tables | `hostaddr.tim`, `landaddr.tim` (format `%20s %3d %3d %3d %03d.%03d.%03d %03d.%03d.%03d %3d`) | | CONFIRMED strings (`script.101` `0x411`..`0x485`); semantics unknown |

In `prefix` and `modem` strings, `~` means a pause of more than 9 BIOS ticks (~0.5 s) and `!` means CR.
CONFIRMED (`MODEM.DRV:0762`; `INSTTSN.HLP` "prefix" topic).

### 3.2 The dial string passed to Connect (driver fn 3)

`script.012` holds the format `%st%s%s` (`0x2F2`) next to the keys `hostID`, `SIERRA`, `prefix` and `modem`.
The dial string is therefore built as:

```
<prefix> 't' <hostID> <modem>      e.g.  +++~~~AT&F!~~~~AT&D2!~AT&C1!~t SIERRA ATDT5551234!~   (spaces added for reading only)
```

The composition is INFERRED from the strings. The driver's parsing of the result is CONFIRMED (`0A6B`..`0BEF`):

1. Copy up to 127 bytes to `DS:0576`. If `ATD` is not found, return `AH=40h`.
2. Find the first lowercase `t` before `ATD`.
   - **No `t`, or `t` directly before `ATD`**: use the default host `SIERRA`.
   - **Digits, 4–14 characters**: skip the first 4 digits (the DNIC) and keep the rest.
     `311083420207` becomes `83420207`. CONFIRMED (`0B0F`). Reading the 4 digits as the SprintNet DNIC `3110` is INFERRED.
   - **Letters, up to 14 characters**: use them as is (for example `SIERRA`). If the text is `DIRECT`, the PAD
     dialogue is **skipped** (`DS:04CD`=0): the modem `CONNECT` alone completes the link.
   - Otherwise: default host.
3. Send the text before `t`, interpreting `~` and `!`.
4. Send the text from `ATD` to the end, appending CR if it contains no `!`.

The PAD call command becomes `"c " + host + "\r"` in the buffer at `DS:05F6`. CONFIRMED.

## 4. Connect sequence (driver fn 3, `MODEM.DRV:0A3E`)

Every tick wait in the driver is a `> N` compare on the BIOS tick (18.2 Hz), so a wait of N ticks lasts N+1 ticks
(for example "18 ticks" is about 1.04 s). The keyboard interrupt (INT 9) is hooked for the whole call. Any key release aborts with `AH=11h`, which is
checked between steps. CONFIRMED (`0694`, `06CD`, `0BA6`).

| # | Client sends | Client waits for | Timeout / timing | On failure (`AH`) | Evidence |
|---|---|---|---|---|---|
| 1 | DTR low for 6 ticks, then high | — | ~0.33 s | — | `0A68` → `129F` |
| 2 | `\rAT\r` | nothing (the `OK` is never checked) | 1 s (18 ticks) | — | `0A94` |
| 3 | the `prefix` part, `~` = 0.5 s pause, `!` = CR | nothing | — | — | `0B9A` |
| 4 | `ATDT<number>` + CR | `CONNECT` anywhere in the receive buffer (needs at least 7 bytes); `BUSY` returns at once | 810 ticks (~44.5 s) | `10h` BUSY. At timeout: DTR pulse, then `04h` if `NO CARRIER` is present, `10h` if `BUSY`, else `08h` | `0C06`..`0D05` |
| 5 | — | CR after `CONNECT`; the first digit run is parsed as the rate | 180 ticks (~9.9 s) | — | `0D0B`..`0D83` |
| 6 | if `DIRECT`: done (go to step 12) | | | | `0DA9` |
| 7 | (receive buffer cleared) pause | — | 1 s | — | `0DB3`..`0DDE` |
| 8 | rate > 1200: `@`; otherwise `\r`. Then `D`, `\r`, `\r`, each followed by a 9-tick pause. | — | ~2 s total | — | `0DE0`..`0E32` |
| 9 | `\r` each loop (9-tick pause) | `@` anywhere in the receive buffer | 180 ticks (~9.9 s) | `02h` (after a DTR pulse) | `0E41`..`0E92` |
| 10 | (buffer cleared) `c <host>\r` | ` CONNECTED` (leading space, 10 bytes) | 270 ticks (~14.8 s), then the command is re-sent; the 6th re-send gives up. The start time is not reset, so after the first timeout the re-sends follow back to back. | `01h` | `0E94`..`0F25` |
| 11 | — | — | 1 s pause | — | `0F27`..`0F45` |
| 12 | — | — | the receive and transmit rings are reset (**all bytes received so far are discarded**), INT 9 is unhooked, the 7-bit mask is cleared | success: `AX=0080h` | `0F47`..`0F75` |

Notes:

- Rate change on CONNECT (step 5): the parsed number is used only if it is at least 1200 by a **signed** compare
  and differs from the current rate. `CONNECT 38400` and `CONNECT 57600` read as negative and are ignored.
  CONFIRMED (`0D6C`..`0D7A`).
- `NO CARRIER` is examined only after the 44.5 s timeout, so the driver does not fail fast on it. CONFIRMED.
- The strings `TERMINAL=` (`0639`) and `D1\r` (`0643`) are present but never referenced. Most `AT` strings at
  `04D3`..`052D` and `0552`/`0558`/`055D` are unused by MODEM.DRV too; only `+++` (`0500`), `AT H0\r` (`0504`) and
  `\rAT\r` (`052D`) are used. CONFIRMED (a scan of all code operands finds no other references).
- The sequence matches a Telenet/SprintNet asynchronous PAD: `@D` + CR autobauds and selects the terminal,
  the next CR answers `TERMINAL=`, and `@` is the PAD prompt. INFERRED.

### 4.1 Result codes

TSNEXEC Connect (`0000:185F`) translates the driver's `AH` (int14h-api section 6). The LSCI script `script.012`
holds one message per resulting number from `0x7B80` onwards. The order of the messages matches the numbers
below. The mapping of message to number is INFERRED from that order; the numbers are CONFIRMED.

| Driver `AH` | Connect returns | `script.012` text (abridged) | Produced by MODEM.DRV? |
|---|---|---|---|
| `00` (`AL=80h`) | 0 | — | yes |
| `80` | 5 | "Your modem cannot be initialized… (No DSR.)" | no |
| `20` | 6 | "Your modem is set to always assert Carrier Detect…" | no |
| `40` | 7 | "The 'modem' string in LSCI.CFG file is incorrect." | yes (no `ATD`) |
| `10` | 8 | "That phone number is busy." | yes |
| `08` | 9 | "INN cannot connect to the phone network." | yes |
| `04` | 10 | "There is no carrier. Either: 1) The PAD is down or busy…" | yes |
| `02` | 12 | "…connect to the phone network, but our software is not receiving the correct prompt… disable MNP…" | yes |
| `01` | 13 | "The carrier is unable to connect to ImagiNation." | yes |
| `09`..`0D` | 15..19 | Novell station and workstation errors (from a non-shipped Novell driver) | no |
| `11` | 23 | "You have pressed a key. You have interrupted the modem…" | yes |

`READ.ME` uses the same numbers: ERROR #1 (connection lost), #8 (busy), #9 (cannot reach the phone network), #10
(access number or PAD trouble), #12 (wrong prompt, MNP), #13 (carrier cannot reach INN) and #21 (reconnection
failure). CONFIRMED. Those texts fix the number-to-meaning mapping independently of the order of the `script.012`
messages.

## 5. Reconnect, hangup and carrier loss

### 5.1 Reconnect (driver fn 8 via SwitchHost, `MODEM.DRV:0FC0`)

This is used to move between hosts (lands) without dialling again. `READ.ME` says moving areas "tells the
network to disconnect you from your current host machine and connect you to another one", which needs a break
signal. CONFIRMED (text).

| # | Client sends | Client waits for | Timeout | Fail (`AH` → SwitchHost return) |
|---|---|---|---|---|
| 0 | — (not in PAD mode, for example `DIRECT`) | | | `0Eh` → 20 "The current driver you are running does not support reconnecting" |
| 1 | receive buffer cleared; 7-bit mask on; escape: MODEM = BREAK, NOBRK = `+++ AT\B ATO` | | | `0Dh` if the UART never reports THRE (SwitchHost returns 255, leftover arithmetic) |
| 2 | each round: check for `@`; send `\r` (10-tick pause); check again; repeat the escape (BREAK about 1.5 s, NOBRK string about 4 s) | `@` anywhere in the receive buffer | 3 rounds | `02h` → 21 "INN has lost the connection to the phone network… break signal…" |
| 3 | address argument parsed like the `t` field (digits: skip 4; letters: up to 14). **Empty argument: go to step 7.** | | | |
| 4 | (buffer cleared) `SET? 0:0,32:0\r` | `@` | 270 ticks | `02h` → 21 |
| 5 | `D\r` | `DISCONNECTED` | 270 ticks | `12h` → 18 |
| 6 | `c <new>\r` | ` CONNECTED` | 270 ticks | on timeout go to step 7. On success remember the new address and return `AX=0`. |
| 7 | `c SIERRA\r` (default; the buffer is cleared only when coming from step 6) | ` CONNECTED` | 270 ticks | found: `0Fh` → 22 "We attempted to reconnect, but the carrier rejected the request. The connection has been remade…". Not found: `01h` → 13. |

All steps CONFIRMED (`0FC0`..`124D`); the error texts are `script.012` `0x8548`, `0x85B5`, `0x885D`.
On exit the receive ring is reset with **no** pause after ` CONNECTED` (unlike Dial). On success TSNEXEC
resets both link sequence numbers to 0 and drops its queues and partial messages (`0000:2A79`, `0000:197B`).
CONFIRMED.

A `DISCONNECTED` timeout returns 18 from SwitchHost, a number that falls in the Novell message range of
`script.012` (15 to 19). The script text shown for it is therefore probably misleading. INFERRED.

Meaning of `SET? 0:0,32:0`: an X.3-style parameter set or read. Which parameters 0 and 32 are is INFERRED unknown.
`D` is the PAD "disconnect" command. INFERRED from the ` DISCONNECTED` it waits for.

### 5.2 Hangup (driver fn 7, `12D1`) and carrier loss

- Hangup acts only if online. It clears the online flag, drops DTR for 18 ticks, sends `+++` and waits
  1 s, sends `AT H0\r` and waits 6 ticks, then raises DTR. CONFIRMED.
- Carrier loss: on a DCD drop the ISR arms a 5-poll countdown. If DCD is still low when it expires,
  CarrierPoll (fn 5, called on every TSNEXEC Service) clears `+16`. Service then returns 1 (connection lost).
  CONFIRMED (`1565`, `126B`, `0000:1AA4`).

## 6. Framing after the host connection is up

All of this is TSNEXEC code and identical in every build checked. The line is 8-bit clean, 8N1, with no
flow control (section 2.4).

### 6.1 Frame layout

```
+------+--------+--------+------+--------------------------+------+
| 0x81 | crc lo | crc hi | ctrl | payload (escaped) 0..249 | 0x82 |
+------+--------+--------+------+--------------------------+------+
  SOF    CRC-16, LE        type|seq                          EOF
```

| Field | Rule | Status |
|---|---|---|
| SOF `81`, EOF `82` | always present; frame pool entry built at `0000:1CA1` (`81` at byte 0, length 5) | CONFIRMED |
| CRC | covers `ctrl` and the payload **as sent, escape bytes included**; stored low byte first | CONFIRMED (`0000:1A46`: `crc(&frame[3], len-4)`; receiver `0000:1F66`, `20AE`) |
| `ctrl` | high nibble = type, low nibble = sequence | CONFIRMED |
| maximum size | the packer adds bytes only while the total escaped frame (SOF to EOF) stays at or below `0xFE` = 254 bytes, so at most 249 escaped payload bytes | CONFIRMED (`0000:2685`, `2738`, `27A6`) |
| CRC bytes and `ctrl` | positional and **never escaped**; they can be `81`, `82` or `1B` (for example NAK 1 has `ctrl`=`81`) | CONFIRMED (receiver states 1–3 take any byte) |

### 6.2 Escaping

Only payload bytes are escaped. `81`, `82` and `1B` are each sent as `1B` followed by the same byte unchanged.
There is no XOR. The receiver stores whatever follows `1B` literally. CONFIRMED (sender: short prefix
`0000:2692`..`26B1`, long prefix `0000:2755`..`279D`, body `0000:27B1`..`27DF`; receiver state 5 at `0000:20E9`).

### 6.3 CRC

CRC-16/CCITT-FALSE (also called CRC-16/IBM-3740): polynomial `0x1021`, initial value `0xFFFF`, no reflection,
no final XOR, check value `0x29B1` for `"123456789"`. The original code is the table-free byte update:

```
x   = (crc >> 8) ^ byte
x  ^= x >> 4
crc = (crc << 8) ^ (x << 12) ^ (x << 5) ^ x        (16-bit)
```

CONFIRMED: `0000:16E0` (one byte) and `0000:171B` (buffer, init `0xFFFF`). A transcription of the
instructions was checked against a bitwise CRC-16/CCITT-FALSE on 2000 random buffers.

### 6.4 Frame types

| `ctrl` | Type | Payload | Sent by client when | Status |
|---|---|---|---|---|
| `0s` (s = 0–7) | DATA | message stream bytes | queued data, after Flush or when the frame is full | CONFIRMED |
| `9s` | ACK s | none | a DATA frame with good CRC arrives (in sequence **or** duplicate) | CONFIRMED (`0000:1BE1`, `222E`, `22BA`) |
| `8s` | NAK s | none | bad CRC (s = the `ctrl` nibble just received), or `81` inside a frame | CONFIRMED (`0000:1BB2`, `20C8`, `21CD`) |
| other types | — | — | never sent. Silently ignored on receive (no ACK, no NAK). | CONFIRMED (`0000:21F8`..`2210`) |

The complete control frames, bytes on the wire:

| s | ACK s | NAK s |
|---|---|---|
| 0 | `81 49 62 90 82` | `81 78 70 80 82` |
| 1 | `81 68 72 91 82` | `81 59 60 81 82` |
| 2 | `81 0B 42 92 82` | `81 3A 50 82 82` |
| 3 | `81 2A 52 93 82` | `81 1B 40 83 82` |
| 4 | `81 CD 22 94 82` | `81 FC 30 84 82` |
| 5 | `81 EC 32 95 82` | `81 DD 20 85 82` |
| 6 | `81 8F 02 96 82` | `81 BE 10 86 82` |
| 7 | `81 AE 12 97 82` | `81 9F 00 87 82` |

### 6.5 Sequence numbers and window

- Transmit sequence: port block `+19`, assigned when a DATA frame buffer is allocated, counting 0..7 and wrapping.
  Receive expected sequence: `+18`, advanced modulo 8 when a DATA frame is accepted. CONFIRMED (`0000:1D3F`,
  `0000:2232`..`2253`).
- Both start at **0 after every successful Connect or SwitchHost**. They do **not** reset between
  child programs. CONFIRMED (`0000:185F`, `197B`; int14h-api 9.3).
- **Window = 1 (stop-and-wait).** A new DATA frame is sent only when no DATA frame is awaiting an ACK
  (`DS:06E2`=0) and the driver's transmit ring is empty. CONFIRMED (`0000:1B42`..`1BA8`).
- A received DATA frame with a sequence other than the expected one is ACKed with its own number and
  **discarded**. So a host that sends a second frame before the first is ACKed loses data whenever the
  first is lost. CONFIRMED (`0000:1F7C`..`1FA8`, `22AB`).

### 6.6 Timers and retry limits (client as sender)

| Item | Value | Status |
|---|---|---|
| clock | 32-bit counter `031D:06EC`, written by the client before each Service. LSCITV copies its timer-ISR counter `DS:0B58` (image `0x16E8A`, `0x16F01`), which runs at **60 Hz** by default (PIT divisor `0x4DAE` = 60.0 Hz, loaded at image `0xCADF` when the setup call passes 0; the increment is at `0xCBF2`). | CONFIRMED default; whether another divisor is passed later is not checked |
| retransmit timeout | `031D:06F0`, default 300 (file `0x3A50`). LSCITV sets 300 = **5 s**. GOLF and CGENN set 90 at 18.2 Hz ≈ 5 s. | CONFIRMED values; GOLF/CGENN rate INFERRED |
| on timeout | resend the same frame bytes (same sequence and CRC) and restamp it. After the 12th resend (counter above 10) drop the frame; Service returns **3** "ImagiNation is not acknowledging sends". | CONFIRMED (`0000:1AEC`..`1B40`) |
| on NAK s (s = outstanding) | increment the same counter. At 10, Service returns **2** "The phone line is too noisy". Otherwise resend at once. A NAK or ACK for any other sequence is ignored. | CONFIRMED (`0000:225A`..`2286`) |
| ACK s (s = outstanding) | free the frame; the next frame may go | CONFIRMED (`0000:2288`..`22A9`) |
| control frame priority | one pending ACK/NAK slot (a newer one replaces the older); it is sent before any DATA frame, once the transmit ring is empty | CONFIRMED (`0000:1B54`..`1B7F`) |
| keepalive | **none** in the driver or TSNEXEC; nothing is sent while there is no data | CONFIRMED (no periodic sender) |

Polling cadence, and therefore ACK latency, depends on how often the client calls Service or Poll. LSCITV calls them
from the `kTSN` sub-op 8 and 10 loops. INFERRED.

### 6.7 Receiver state machine (`0000:1E7B`, state `031D:06F2`, jump table `0000:1F18`)

| State | Byte | Action |
|---|---|---|
| 0 hunt | `81` → state 1; anything else is discarded | |
| 1 | CRC low | |
| 2 | CRC high | |
| 3 | `ctrl`: type = high nibble, seq = low nibble; running CRC = update(`FFFF`, ctrl). Flag "duplicate" if type 0 and seq ≠ expected. | |
| 4 | `1B`: CRC update, state 5. `81`: NAK(seq), discard the partial frame, restart at state 1. `82`: end of frame (below). Anything else: CRC update and append to the receive byte ring (unless duplicate; if the ring is full, mark duplicate and overflow). | |
| 5 | any byte: CRC update, append, state 4 | |

At the end of a frame: on CRC mismatch, NAK(seq) and discard. Otherwise: an overflowed frame is dropped
without an ACK. A duplicate DATA frame gets ACK(seq). An in-sequence DATA frame is committed and ACKed. An ACK or
NAK frame is handled as in 6.6. CONFIRMED.

Bytes of an ACK or NAK frame that carries a payload are appended to the uncommitted receive area and are
not rolled back. A later DATA frame would commit them. **A host must send ACK and NAK frames with an empty
payload.** CONFIRMED (no reset of `031D:0736` on the `0x80`/`0x90` paths), consequence INFERRED.

### 6.8 Message stream inside DATA frames

The payloads of the accepted DATA frames, unescaped and concatenated in sequence order, form one byte
stream. Messages are packed back to back on that stream:

```
len < 0xFF :  [len]            body[len]
len >= 0xFF:  [FF][len lo][len hi]  body[len]
```

- A length prefix is never split across frames: the packer checks room for the whole escaped prefix.
  A body may be split at any byte. Several messages may share a frame. CONFIRMED (sender `0000:2604`,
  receiver `0000:2872`).
- A frame is closed (CRC and EOF written, ready to send) when it is full, when a message flagged by Flush
  completes, or when Flush finds a partly filled frame. CONFIRMED (`0000:2817`, `2848`, `2594`).
- TSNEXEC adds nothing else: no message type, channel or checksum. The first body byte is the application
  opcode (int14h-api section 10). CONFIRMED.

### 6.9 First frames at logon

There is **no link-level handshake**. After Connect returns 0, the next frame on the wire is either:

- the client's first DATA frame, `ctrl` = `00`. Its payload is the first message(s) the logon script
  queues, each with a length prefix, for example `81 cL cH 00 [len] [opcode] … 82`; or
- a DATA frame from the host with `ctrl` = `00`, which the client ACKs with `81 49 62 90 82`.

Which side speaks first, and what the first message bodies contain, is decided by `kTSN` and the scripts
(`script.012` "MakeConnection", `script.101` "LoginTimeout", "We are not receiving any messages from the
network"). That is outside this document and still open (section 8).

## 7. Checklist: a server that emulates the PAD and host for the stock client

Assumes DOSBox modem emulation, or any byte pipe that looks like a Hayes modem to the client.

1. **Modem stage.** The emulated modem must answer `ATD…` with `CONNECT[ rate]\r\n`, and must hold carrier
   (DCD high) for the whole session; a DCD drop ends it (section 5.2). A rate of 32768 or above in the CONNECT line is harmless
   (it is ignored). Do not reply `BUSY` or `NO CARRIER` unless failure is intended.
2. **Terminate the CONNECT line with CR.** The client waits up to 9.9 s for a CR after `CONNECT`. It then clears its
   receive buffer once, at the start of a 1 s pause. Bytes that arrive after that point are kept, including during
   the pause, and step 3 searches them for `@`. Staying quiet until the wake-up is the simplest choice.
3. **PAD wake-up.** Expect `@` (or CR at 1200 bps or below), then `D`, CR, CR, about 0.5 s apart. Reply so that an
   `@` is in the stream after the last one, for example by echoing, or `\r\nTERMINAL=` after the first CR and
   `\r\n@` after the second. Keep answering every further CR with `\r\n@`; the client gives up after ~9.9 s.
   Use 7-bit ASCII here (the client strips bit 7 anyway).
4. **Call.** Expect `c <host>\r`, where `<host>` is `SIERRA` (from `hostID`) or an 8-digit NTN such as
   `83420207` (`HOSTADDR` minus the DNIC `3110`). Reply `\r\n<host> CONNECTED\r\n` within ~14 s. The space
   before `CONNECTED` is required. Never print ` CONNECTED` for any other reason.
5. **Silence for at least 1 s after CONNECTED.** The client pauses 1 s and then **discards everything received**
   (driver `0F27`..`0F72`). Wait at least 1.5 s, or for the client's first frame, before sending.
6. **Switch to 8-bit transparent mode.** Reset the link state: next transmit sequence 0, expected receive sequence 0.
7. **Framing.** Hunt for `81`. Read 2 CRC bytes and `ctrl` positionally, then unescape (`1B x` → `x`) up to an
   unescaped `82`. Verify CRC-16/CCITT-FALSE over `ctrl` plus the escaped bytes. An unescaped `81` inside a
   frame aborts it and starts a new one.
8. **Answer every client DATA frame:**
   - good CRC → `ACK seq` (section 6.4 table), including duplicates;
   - bad CRC → `NAK seq`, or simply stay silent; the client resends after ~5 s.
   - Deliver the payload only when seq = expected; then advance expected modulo 8.
   - ACK quickly: the client resends after 5 s and gives up after 12 resends.
9. **Send to the client stop-and-wait.** Send one DATA frame (`ctrl` = seq 0..7, escaped payload of at most 249
   bytes, total frame of at most 254 bytes, CRC, `82`). Wait for `ACK seq` before sending the next.
   - Resend on `NAK seq` or after a timeout. Use a few seconds; the client ACKs only when it polls.
   - ACK and NAK frames sent to the client must have **no payload**.
10. **Message layer.** Split the reassembled client stream into `[len][body]` or `[FF lo hi][body]` messages.
    Encode host messages the same way. Messages may straddle frames.
11. **No keepalive is needed at the link level.** Application-level timeouts exist in the scripts (section 6.9).
12. **Escape to PAD (land switch).** The client sends a BREAK (MODEM.DRV) or `+++`, `AT\B`, `ATO` (NOBRK.DRV) to the
    modem, then CRs while the link is between frames.
    - A TCP pipe normally carries neither the BREAK nor the escape. **Treat any CR (`0D`) received in hunt state as
      a PAD escape** and reply `\r\n@`. In hunt state the client never sends anything but `81`. INFERRED
      heuristic. A Telnet-style `IAC BRK`, if the modem emulation sends one, should be treated the same way. INFERRED.
    - Then expect `SET? 0:0,32:0\r` and reply `\r\n@`.
    - Expect `D\r` and reply `\r\n<old host> DISCONNECTED\r\n@`.
    - Expect `c <new>\r` and reply `\r\n<new> CONNECTED\r\n`. Reset sequence numbers to 0 and stay silent for at least 1.5 s
      (the client clears its buffer right after ` CONNECTED` and also drops its own queues).
13. **Hangup.** DTR drop, `+++` and `AT H0` close the call, and the client reports nothing further. Discard any
    in-band `+++` / `AT H0` bytes that reach the server.
14. **Optional `DIRECT` host.** With `hostID = DIRECT` in `LSCI.CFG`, steps 2–5 do not happen: framing starts right
    after the modem `CONNECT` (the receive buffer is still cleared on return). SwitchHost then fails with error
    20, so this only suits single-host testing.

## 8. Open questions

- What the application payloads are: the first logon messages, which side sends first, and the opcode
  set. This belongs to the `kTSN`/script analysis.
- Exact meaning of the `HOSTADDR` columns `CC n`, and of `hostaddr.tim` / `landaddr.tim`. Which script passes which
  address to SwitchHost.
- What X.3 parameters `SET? 0:0,32:0` sets, and the real SprintNet response text. The client needs only an `@`.
- How DOSBox / DOSBox-X modem emulation handles UART BREAK, `+++`, `AT\B` and `ATO`. This decides whether
  the CR-in-hunt-state heuristic (checklist 12) is sufficient. Needs a capture.
- `novell` in `script.012` (`0x324`) and the Novell error texts point to a LAN driver that is not in these sets.

## 9. Verification

A second pass went back to the binaries with a capstone disassembly of `MODEM.DRV`, `NOBRK.DRV`, `TSNEXEC_inn_cd.EXE`
and `LSCITV_inn_cd.EXE`, plus the `script.012`, `READ.ME`, `INSTALL.SCR` and `MODEM.TXT` files. It was made without
reusing the first pass's disassembly.

Reproduced as written:

- Driver header bytes, the 10-entry tables of both drivers, the entry stub, the init option parser (`13DA`) and
  the port block layout. The ISR IIR table (`1516`) dispatches IIR 0, 2, 4, 6 to modem status, THRE, receive data
  and line status. CTS is never read on the transmit path.
- The CRC: a new transcription of `0000:16E0` gives `0x29B1` for `"123456789"`, and all sixteen ACK and NAK control
  frames in section 6.4 match byte for byte.
- Frame build and close (`0000:1CA1`, `1CE8`, `1D3F`, `1A46`): `ctrl` = `type | seq`, the CRC covers `ctrl` plus
  the escaped payload and is stored low byte first, data `ctrl` has no high bit.
- The receiver state machine, including the duplicate flag, the rollback of duplicate and bad-CRC frames, the ACK of
  duplicates, no ACK on ring overflow, ignoring of unknown types, and the uncommitted bytes of a control frame.
- The packer limit of `0xFE` bytes per frame, escape set `81 82 1B`, the long prefix `FF lo hi` with escaped bytes,
  and the unpacker state at `031D:0712`.
- Retransmission: restamp on every send, the shared retry counter at slot `+8` (timeout path drops the frame when
  the old value is above 10, so the 12th timeout drops; the NAK path returns 2 at the 10th NAK), window 1, and
  Connect and SwitchHost resetting both sequence numbers and the queues before calling the driver.
- Dial parsing, the `ATD` search, the `t` and host rules (4 to 14 digits minus 4, up to 14 letters, `DIRECT`),
  the buffers at `0576`, `05F6` and `0612`, every timeout in the section 4 table (ticks 810, 180, 270, 6 and 18), the
  eight-result code mapping, and the 7-bit strip. The default-host CR bug in MODEM.DRV (`06E0` versus the fixed
  `06F0` used by SwitchHost) and the fixed string in NOBRK.DRV.
- The reconnect sequence and its three failure codes, and the Connect and SwitchHost return maps at `0000:185F` and
  `0000:197B`.
- MODEM.DRV against NOBRK.DRV: a normalised instruction diff shows only the removed BREAK routine, the escape
  string sends, the `IER |= 02h` byte, the default host fix and data bytes. The Dec-93 MODEM.DRV differs from the
  Feb-94 file only at file offset `0x1265`, and the two NOBRK.DRV files are identical.
- `README`, `script.012`, `INSTALL.SCR`, `MODEM.TXT` and installer strings quoted in sections 3 and 4.1.
- LSCITV: the INT 14h stub, the `kTSN` jump table, GetStatus before Connect in sub-op 3 (the dial string comes from
  the script argument), SetAckTimeout(300) and the SetCallbacks argument order.

Corrected in this pass:

- Section 2.3: init sequence timings and the registers read to clear stale state.
- Section 2.5: the NOBRK escape string bytes are CONFIRMED but their Hayes meaning is INFERRED; the BREAK lasts
  about 0.55 s plus a 1 s wait; the unused-strings list wrongly included three strings that Hangup and Dial use.
- Section 4: all driver tick waits last N+1 ticks.
- Section 4.1: README confirms the numbers 1, 8, 9, 10, 12, 13 and 21 directly.
- Section 5.1: the retry round is CR plus an escape (about 2 s with BREAK, about 4.5 s with NOBRK), not "every
  0.5 s". A failed BREAK makes SwitchHost return 255. A `DISCONNECTED` timeout (18) lands in the Novell message range.
- Section 6.6: the 60 Hz client clock is the default timer divisor. A different divisor passed by a later setup
  call was not ruled out.
- Section 7 step 2: the receive buffer is cleared once, at the start of the 1 s pause after CONNECT, not at its end,
  so bytes sent during that pause are kept and are searched for `@`.

Not reproduced: the `script.012` order of messages against numbers (the multi-page messages make the order
ambiguous, but the README texts make the numbers certain), the dial string argument order of `%st%s%s`
(`prefix`, `hostID`, `modem` is consistent with the driver's parser but the script bytecode was not decoded), and
all application-layer questions in section 8.

# INT 14h census: every program, every export

Which of TSNEXEC's 17 exports (`docs/protocol/int14h-api.md`) each INN program calls, from where, and
which host messages it depends on. One file per program holds the evidence; this page is the index and
the cross-program view. The implementation fan-out derived from it is `docs/server/fanout-plan.md`.

| Program | Binary (`work/`) | Doc | Table sites | Live exports |
|---|---|---|---|---|
| LSCITV (all three lands) | `exe/LSCITV_inn_cd.EXE` | [lsci.md](lsci.md) | 19 | 17 |
| Yserbius character generator | `games/YSERBIUS/CGENN.EXE` | [yserbius.md](yserbius.md) | 38 | 10 |
| Yserbius game | `games/YSERBIUS/DARKSTRT.EXE` | [yserbius.md](yserbius.md) | 40 | 10 |
| Twinion gallery | `games/TWINION/TWGENN.EXE` | [twinion.md](twinion.md) | 38 | 10 |
| Twinion game | `games/TWINION/FATES.EXE` | [twinion.md](twinion.md) | 41 | 10 |
| 3-D Golf | `games/GOLF/GOLF.EXE` | [golf.md](golf.md) | 30 | 11 |
| Multi-Player Red Baron | `games/BARON/RB_unpacked.EXE` | [redbaron.md](redbaron.md) | 33 | 14 |
| Shoppers Advantage | `games/SHOPADV/SHOPADV_unpacked.EXE` | [shopadv.md](shopadv.md) | 14 | 9 |

All counts CONFIRMED by two independent scans (byte pattern `les bx,[p]; lcall es:[bx+N]` plus capstone,
and Ghidra xrefs) in each program's Verification section. Every program has exactly one `CD 14` (the
20-byte lookup stub) and one stub caller. Coverage: all 34 EXE/COM/DRV/SYS files of `work/sets` were
unpacked and scanned; only LSCITV has INT 14h code among them (lsci.md section 9). Not scanned as code:
the TSN-basic container (`_SCITV.EXE`, `_SNEXEC.EXE`) and the SZDD DLLs. INFERRED: no other user exists.

## 1. Export-by-program matrix

Cell = call sites, then the wrapper(s). `†` = the site exists but its wrapper or caller is never reached
(dead). `-` = no site. Yserbius and Twinion share `hostcomm.cpp` offsets, so their wrapper columns are
merged; per-binary counts are given as CGENN/DARKSTRT and TWGENN/FATES.

| # | Export | LSCITV (`kTSN`, seg `16B1`) | Yserbius `149A`/`172D` | Twinion `150F`/`1716` | GOLF (image) | Red Baron (image) | SHOPADV (`05C9`) |
|---|---|---|---|---|---|---|---|
| 0 | GetStatus | 2: TSN(0) `00AB`, Connect pre-check `016D` | - | - | - | 1: `initNet` `0x2F4B7` (AL 2, or 3 with rate ≥ 2400) | - |
| 1 | GetSharedData | 1: TSN(1) `00BC` | 4/4: `01AD`†, `01D0` cookie, `01EF` reader, `02A1`† | 4/4: `01B1`†, `01D4` cookie, `01F3` reader, `0299`† | 4: `hcGetUserWord4`†, `hcGetUserSid`, `hcGetSharedParams`, `hcSetSharedParams` | 4: `sharedWord4`†, `sharedWord0`, `readSharedBlock`, `writeSharedParams` | - |
| 2 | SetSharedData | 1: TSN(2) `0155` | 1†/1†: `02A1` (no caller) | 1†/1†: `0299` (no caller) | 1: `hcSetSharedParams` (exit) | 1: `writeSharedParams` (exit) | - |
| 3 | Connect | 1: TSN(3) `0198` | - | - | - | - | 1†: `05B0` |
| 4 | Send | 1: formatted send `042E` | 21/21: 19 builders (8†), game send, BBS send | 21/21: 19 builders (8†), game send, BBS send | 15: 13 `hostcomm.c` builders (8†), `gcSendToObject`, `gcMulticast` | 16: 14 builders (5†), `sendToGroup`, `sendMulticast` | 2: formatted `066A`, raw `061C`† |
| 5 | Receive | 1: event pump `0358` | 1/1: pump `1073`/`1165` | 1/1: pump `106B` | 1: `hcPump` | 1: `pump` `2DA2:0C6C` | 1: receive loop `08FA` |
| 6 | SetAckTimeout | 1: init, 300 ticks | 1/1: init, 90 | 1/1: init, 90 | 1: `hcInit`, 90 | 1: `tsnInit`, 90 | 1: ctor `03B7`, 90 |
| 7 | Disconnect | 1: TSN(7) `01F8` | - | - | - | - | 1†: `099A` |
| 8 | Poll | 1: event pump `0358` | 1/1: `0109`/`010D` | 1/1: `010D` (TWGENN also via vtable thunk) | 1: `hcPoll` | 1: `pollChecked` | 1: `04A5` |
| 9 | SetNextProgram | 1: TSN(9) `0229` | 3/5: init + exit menus | 3/6: init + exit menus | 2: `hcInit`, `hcGetBackOn` | 1: `tsnInit` | 1: ctor `01BB` |
| 10 | Service | 1: timer server `03F1` (every 20 ticks) | - | - | 1: `hcTimerTick` (INT 8, every 6th) | 1: `serviceTick` (PIT, every 6th) | 1: `0574` (INT 1Ch) |
| 11 | GetPreviousProgram | 1: TSN(11) `0238` (no script uses it) | 1/1: init | 1/1: init | 1: `hcInit` | 1: `tsnInit` | 1: ctor (unguarded) |
| 12 | IsTransmitIdle | 1: TSN(12) `0289` | 2/2: leave land, end session | 2/2 (TWGENN `01F2`†) | - | 1: inline `0x9B7F` (clean exit) | - |
| 13 | SwitchHost | 1: TSN(13) `02A2` | - | - | - | - | 1†: `05E6` |
| 14 | Flush | 1: TSN(14) `0300` | 2/2: `0179`/`0190`†, `018A`/`01A1` | 2/2: `017D`†, `018E` | 2: `hcPollAndFlush`†, `hcFlush` | 2: `pollAndFlush`, `flushChecked` | 1: `0505` |
| 15 | SetCallbacks | 2: init, `onexit` (NULLs) | 1/1: init | 1/1: init | 1: `hcInit` | 1: `tsnInit` | 1: `03D9` (ctor; dtor passes zeros) |
| 16 | GetLineRate | 1: TSN(16) `030C` | - | - | - | 1: `initNet` `0x2F4AC` | - |
| (+44) | not an export | - | - | - | - | - | 1†: `09B0` (would call `CD35:14B8`) |

Highlights (all CONFIRMED):

- **Only LSCITV owns the connection.** Connect, Disconnect and SwitchHost are live only in LSCITV; SHOPADV
  links wrappers for them that nothing calls. Every DOS game inherits the live session and never logs in.
- **Common live core of the DOS games**: Send, Receive, Poll, Flush, SetAckTimeout(90), SetCallbacks,
  GetPreviousProgram → SetNextProgram. GetSharedData adds to that everywhere except SHOPADV.
- **Service is called from interrupt context** by GOLF (INT 8), Red Baron (PIT callback) and SHOPADV
  (INT 1Ch); LSCITV calls it from its IRQ0 server. A stand-in executive must take the reentrancy lock.
- **GetStatus and GetLineRate** matter only to LSCITV (dial branch, `AH` = 0x80 while connected) and Red
  Baron (refuses lines below 2400 baud when `AL` = 3).
- **SetSharedData is live only in LSCITV, GOLF and Red Baron.** Yserbius and Twinion never write the block.
- **Send failure is fatal** in every DOS game (`!= 1` → "send failure" / "TSN_NetSend() failed"); SHOPADV
  discards the result.
- Tool caveat: `tools/int14h_census.py` reports C++ virtual calls (`lcall [bx+N]` after `mov bx,es:[bx]`) as
  table calls in DARKSTRT, CGENN, TWGENN, FATES and SHOPADV, stops at `+40`, and misses far calls after a
  segment whose low byte is `0x9A`. The counts above exclude the false positives.

## 2. The TSN client library, by program

| Library | Programs | Language | Builders (Send) | Receive table | Status |
|---|---|---|---|---|---|
| `kTSN` kernel, sub-op = export index | LSCITV | C in the interpreter; scripts build bodies with format codes `b w s a +` | 1 formatted send | none: every message becomes event `0x400` for the scripts | CONFIRMED |
| `src\tsn\hostcomm.c` + `gamecomm.c` (+ `edit.c`, `dialog.c`) | GOLF; Red Baron (`s\tsn\…`) | Borland C | GOLF 13, RB 14 (same order as `hostcomm.cpp`) | 13 commands `0 1 2 8 9 10 11 12 13 14 36 41 48` in both | CONFIRMED |
| `c:hostcomm.cpp` + `rpgcomms.cpp` + `rpgbbs.cpp` | CGENN, DARKSTRT (`__LINE__` `0x184`..`0x37D`); TWGENN, FATES (`0x180`..`0x370`, later revision) | Borland C++ | 19, identical code in all four | 14 commands: the C set plus 27 (BBS) | CONFIRMED (byte diff, twinion.md section 7) |
| `tsn.cpp` class `Tsn` + `Element` framework | SHOPADV | Borland C++ | formatted send with LSCI-like format strings; dead Login (53) and Element joinNet/register/setInt | 4-class `Msg` parse (0, 1, 8, 13) plus command 48 and 54 handlers | CONFIRMED |

The two C/C++ families share the message header (`b cmd, b sub, w @2, w @4`), the init order
(stub → SetCallbacks → SetAckTimeout(90) → GetPreviousProgram → SetNextProgram) and the pump shape
(Poll, Receive, dispatch on byte 0, free, Flush). INFERRED: `hostcomm.cpp` is a C++ port of
`hostcomm.c`; the builder order and command numbering match, the `__LINE__` values do not.

## 3. Shared block per program

The 256-byte block lives in TSNEXEC (`031D:013A`, length `031D:023A`) and survives program switches; it
never crosses the wire. Offsets below are hexadecimal.

| Offset | LSCITV writes (messages.md 5.3) | Yserbius | Twinion | GOLF | Red Baron | SHOPADV |
|---|---|---|---|---|---|---|
| +00 w | user SID | joinNet cookie | joinNet cookie | user SID (in CA) | user id (cookie, 41/2, 17, 0x32) | - |
| +02 w | `userFlags` | bit 1/2 → BBS text @0x16 | bit 0x02/0x04 → same | - | - | - |
| +04 w | SID handed to the next program (old game object; game group for a game launch) | read, unused | read, unused | **game group SID** | **game group SID** | - |
| +06 w | land number | land joinNet `param` | land joinNet `param` (default 1) | - | - | - |
| +08 w | land type | land joinNet `landType` | **not read** (fixed 8) | - | - | - |
| +0A, +0C | host number; rating, land flags | read, unused | read, unused | - | - | - |
| +10 11 bytes | user name (decimal 16) | - | - | user name | user name | - |
| +1C, +2C | host name, encoded password | - | - | - | - | - |
| +80.. | next-program parameters (one byte each) | - | - | 14 launch bytes (byte 4 = players) | 15 mission bytes (byte 0 = players) | - |
| written back | on leaving a land | never | never | `+80` w result (len `0x82`) | `+80` 7 bytes (score word + 5 junk) (len `0x87`) | never |

**The layout mismatch in `int14h-api.md` section 9.3 and `messages.md` sections 5.3, 9 and 11 is
resolved:** the DOS games' `+10` is hexadecimal. GOLF and Red Baron copy 11 bytes from `block+0x10`
(GOLF image `0x134F5`..`0x13526`, RB image `0x2DC8C`..`0x2DCA4`), which is decimal 16, LSCI's user-name
slot. CONFIRMED (both games); "same writer" INFERRED. Word `+04` is the only field whose meaning depends
on context: LSCITV writes the SID it wants the next program to inherit. On a land switch that is the old
game object (freed by the arriving land); for Red Baron the land stores `global119`, the game group
(`SL/script.120 runRedBaronScript`, INFERRED); GOLF reads it as the group to join (CONFIRMED). Consequence
(INFERRED): when GOLF or RB returns, the first `0x80` bytes are unchanged, so the arriving land sends
`leaveNet(9, groupSid)` for the game group; the host must accept that after the players' own GrpDel.

Chaining (CONFIRMED in every DOS game): GetPreviousProgram → SetNextProgram(previous) at start, so exit
returns to the launching land. NULL previous is fatal ("Application Orphaned." / "orphaned application!")
except in SHOPADV, which calls through a null table unguarded. Exit targets: CGENN `YSERBA`/`DEFAULT`,
DARKSTRT `DEFAULT`/`YSERBIUS`/NULL, TWGENN `TWINA`/`DEFAULT`, FATES `DEFAULT`/`Twinion`/NULL; TSN.PRG
lookup must be case-insensitive (INFERRED from `TWINA` vs block `Twina`).

## 4. Host messages per program

C→H lists live senders only. Full layouts and dead builders are in each program's doc.

| Program | C→H (live) | H→C handled | Awaited replies and timeouts | Doc section |
|---|---|---|---|---|
| LSCITV | all of `messages.md` (381 Send sites, 33 constant first values); Login 53/59 first | every command through event `0x400`; errors as event `0x800` | Login Ack (`whichCmd` 22) in 70 s; IsTransmitIdle drain at land switch | lsci.md 6, 10 |
| Yserbius | 7 joinNet kinds 1/5/2/4, 9, 10/0, 10/3 (v1.1.7), 11, 40/4, 41/2, 27 BBS, 2 game (200–218) | 0, 1, 2, 8, 9, 10, 11, 12, 13, 14, 27, 36, 41, 48 | ObjID in request order; 41 (resent every 60 s); all waits 60 s; link ACK 90 ticks | yserbius.md 4, 5, 7 |
| Twinion | as Yserbius plus 17/2 (props 8–11); 10/3 v1.0.22; land type fixed 8; map cap 80; game header +4 bytes, msgType 219 | as Yserbius | as Yserbius; TWGENN pumps Receive only at start-up and exit | twinion.md 4, 5, 6 |
| GOLF | 7 kind 1 (type `0x66`), 10, 12, 11, 9, 2 (CA, CB, CD), 28 multicast (CC, with own SID) | 0, 1, 2, 8, 9, 10, 11, 12, 13, 14, 36/2, 41, 48 | ObjID, GrpJoin echo, GrpMem each in 120 s (`0x888` ticks) | golf.md 6, 7, 9 |
| Red Baron | 41/2 (type 101), 17/0 props 9 and `0xF`, 36/2 host time, 7 kind 3, 10, 12, 11, 9, 50 status, 2 (0xC8), 28 multicast (0xC9, with own SID) | 0, 1, 2, 8, 9, 10, 11, 12, 13, 14, 36/2, 41, 48 | each join step in about 119 s; setInt prop 9 must be `0x0201` | redbaron.md 5, 6, 10 |
| SHOPADV | 54/25 JOIN, 54/24 TEXT, 54/26 EXIT, 54/27 STATUS (after a login Ack) | 0, 1, 8, 13 (ignored), 48, 54/16..20 | JOIN: no timeout (hangs); TEXT: 636 ticks; EXIT ack: forever without error | shopadv.md 7, 9, 11 |

Common to every DOS game (CONFIRMED): no Connect or Login; the group joiner must receive its own
GrpJoin echo; a group `Send` is relayed untouched with the real sender in `fromSid`; multicast 28 must
loop back to the sender when its own SID is listed (GOLF, RB); Unsolicited 48 is shown as operator text.
Open across programs: whether a group `Send` is echoed to its sender, what 41/2 resolves to, and how the
host scopes Yserbius/Twinion map groups (each doc's open questions).

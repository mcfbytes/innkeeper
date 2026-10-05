# INT 14h census: Multi-Player Red Baron

How the INN conversion of Dynamix's Red Baron (`work/games/BARON/RB_unpacked.EXE`, "(c) 1991-93 The
ImagiNation Network", "INN Conversion by Steve Luzietti and Dave Eaton") uses the TSNEXEC export table, every
message it builds on Send and Receive, and what a host must do to serve it. Export numbers and semantics are in
`docs/protocol/int14h-api.md`; shared command numbering is in `docs/protocol/messages.md`; the sibling census for
the Yserbius programs (same `hostcomm` lineage) is `docs/protocol/int14h-census/yserbius.md`.

| Item | Value |
|---|---|
| `TSN.PRG` block | `program RedBaron` / `cd BARON` / `rb.exe -C..\LSCI.CFG` (`work/sets/inn_cd/TSN.PRG` line 30). `RB.EXE` is the packed copy of `RB_unpacked.EXE` |
| MZ header | `0x3700` (relocation table at `0x1C`, 0xDB8 entries); no overlay; file = header + image |
| Data segment | `3C3D` (Ghidra `4C3D`), file `0x3FAD0` |
| Stub | `2FB4:0006` (image `0x2FB46`, file `0x33246`); the only `int 14h` is at image `0x2FB50` (file `0x33250`) |
| Cached table pointer | `DS:2A86` (image `0x3EE56`, file `0x42556`) |
| Stub callers | one: `2DA2:0048` (image `0x2DA68`, file `0x31168`), in `tsnInit`; no other code touches the cell |
| Source modules (strings) | `s\tsn\hostcomm.c` (code `2DA2`), `s\tsn\gamecomm.c` (code `2EA6`), also `cobj.c`, `mpgutil.c`, `cloudgen.c`, `my_fmal.c` (not on the wire) |

## 0. Method and conventions

- `seg:off` is relative to the load image; **file offset = `0x3700` + seg×16 + off**. Ghidra 12.1.4 (language
  `x86:LE:16:Real Mode`, project `work/ghidra/census-redbaron/`) loads at `1000:0000`, so add `0x1000` to the
  segment there. 1684 functions were decompiled to `work/decomp/redbaron/all.c`; `fn/<imageOffset>_*.c` holds one
  function per file, `calls.txt` every call edge, `INDEX.txt` the names used below (not committed). Ghidra did
  not create the dead lock builder `2DA2:0B1E` (image `0x2E53E`; its table call at `0x2E599` has no Ghidra
  reference), so the raw scan, not the decompile, is the authority for call sites.
- Ghidra's segmented decompile is poor for far pointers, so every claim below was read from capstone disassembly.
  Call sites come from a scan for `les bx,[2A86]` (`C4 1E 86 2A`) followed by `lcall es:[bx+N]`. All 33 sites
  were found this way; the cell `DS:2A86` has no other reader or writer than the stub. CONFIRMED.
- Image offsets are written as `image 0xNNNNN`; add `0x3700` for the file offset. The far-call convention is
  cdecl, far pointers pushed as segment then offset.
- Message layouts use `messages.md` notation: `b` byte, `w` LE word, `s` NUL-terminated string, `@n` byte offset.
  Commands carry their decimal number from `messages.md`, with the hex value where the code shows hex.
  C→H = client to host, H→C = host to client, C→C = a `Send` relayed between players.
- CONFIRMED = seen in code; INFERRED = deduced from names, strings or from the Yserbius census.

## 1. Library layout

The TSN client library is two Borland C modules plus the stub. The same 14 `Send` builders appear in the same
order as Yserbius' `hostcomm.cpp`, with source line numbers passed to the fatal-error routine.

| Function | Image | `seg:off` | Role |
|---|---|---|---|
| `waitKey` | `0x2DA22` | `2DA2:0002` | loop `pump(0)` until any key, then return it; error screens call it until ENTER |
| `tsnInit` | `0x2DA56` | `2DA2:0036` | stub call, SetCallbacks, SetAckTimeout, program chaining (section 3) |
| `pollChecked` | `0x2DB0F` | `2DA2:00EF` | copy BIOS tick, Poll, fatal on status (section 8) |
| `pollAndFlush` / `flushChecked` | `0x2DBAC` / `0x2DBBD` | `2DA2:018C` / `019D` | Poll+Flush; tick copy+Flush |
| `readSharedBlock`, `writeSharedParams`, `sharedWord0` | `0x2DC4D`, `0x2DCE9`, `0x2DC17` | `2DA2:022D`, `02C9`, `01F7` | shared block access (section 4) |
| 14 Send builders | `0x2DD5A`..`0x2E5E5` | `2DA2:033A`..`0BC5` | one per host command (section 5.1) |
| `pump` | `0x2E68C` | `2DA2:0C6C` | Poll, Receive, dispatch, free; `pump(1)` drains, `pump(0)` takes one |
| `fatalError` | `0x2E8DF` | `2DA2:0EBF` | `(file, line, fmt, …)`: text screen, ENTER, exit 0 |
| alloc / deref / free callbacks | `0x2E9C4`, `0x2EA12`, `0x2EA29` | `2DA2:0FA4`, `0FF2`, `1009` | wrap the game heap (`3037:0321`, `055F`, `02E9`); alloc returns `DX:AX` = near handle in `AX`, `DX` = 0, so Send and Receive carry a 32-bit handle (Send takes 6 argument bytes: handle, length); each callback reloads `DS` = `3C3D` first, so TSNEXEC may call them with another `DS` (INFERRED) |
| `serviceTick` | `0x2EA40` | `2DA2:1020` | timer-ISR callback calling Service (section 7) |
| `initNet`, `joinGame`, `leaveGame` | `0x2F2C0`, `0x2F5C5`, `0x2F572` | `2EA6:0860`, `0B65`, `0B12` | session phases (section 6) |
| `onSend`, `broadcastState`, `sendPlayerInfo` | `0x2EE95`, `0x2F087`, `0x2F1AE` | `2EA6:0435`, `0627`, `074E` | game messages (section 7) |

## 2. Export usage

33 call sites, 14 of the 17 exports. Every site is `les bx,[2A86]; lcall es:[bx+N]` followed by the caller's
`add sp,n`. File offset = image + `0x3700`.

| # | Export | Sites | Image offsets | Wrapper(s) | Arguments | Result use | Status |
|---|---|---|---|---|---|---|---|
| 0 | GetStatus | 1 | `0x2F4B7` | `initNet` | – | `AL` (`& 0xFF`) must be 2, or 3 with GetLineRate ≥ `0x960` (2400); else "requires a %d baud or greater connection." (2400) and exit 0 | CONFIRMED |
| 1 | GetSharedData | 4 | `0x2DBEB`, `0x2DC22`, `0x2DC60`, `0x2DCF9` | `0x2DBE0` word@4 (**no caller**), `sharedWord0`, `readSharedBlock`, `writeSharedParams` | `&farPtr` | length 0 → fatal "orphaned application!" (hostcomm line 322); else pointer to block | CONFIRMED |
| 2 | SetSharedData | 1 | `0x2DD4A` | `writeSharedParams` | `(ptr, 0x80+n)` | none | CONFIRMED |
| 4 | Send | 16 | 14 builders `0x2DDA8`..`0x2E640` (of which 5 have no caller), `sendToGroup` `0x2EF88`, `sendMulticast` `0x2F02E` | section 5.1 | `(handle, len)`, handle from `alloc` | `≠1` → fatal "send failure %d" with message byte 0 | CONFIRMED |
| 5 | Receive | 1 | `0x2E69D` | `pump` | `&handle` | 0 → Flush, return; else dispatch on byte 0, `free(handle)`, Flush | CONFIRMED |
| 6 | SetAckTimeout | 1 | `0x2DAB0` | `tsnInit` | 90 (BIOS ticks ≈ 4.9 s) | pointer kept at `DS:72D2`; every Poll/Flush copies the BIOS tick (`0040:006C`, far pointer at `DS:72CE`) into it | CONFIRMED |
| 8 | Poll | 1 | `0x2DB2E` | `pollChecked` | – | status ≠ 0 → fatal (section 8); `pollChecked` has 18 callers (14 builders, `sendToGroup`, `sendMulticast`, `pollAndFlush`, `pump`) | CONFIRMED |
| 9 | SetNextProgram | 1 | `0x2DAD8` | `tsnInit` | the name from GetPreviousProgram | – | CONFIRMED |
| 10 | Service | 1 | `0x2EA59` | `serviceTick` | – | status kept in `DS:265F`, reported at the next `pollChecked` | CONFIRMED |
| 11 | GetPreviousProgram | 1 | `0x2DAC2` | `tsnInit` | – | NULL → "Parent failure: application has been orphaned." and exit 1 | CONFIRMED |
| 12 | IsTransmitIdle | 1 | `0x9B7F` | clean-exit routine `0x9B59` (outside the library) | – | loop `pollAndFlush` until the result is non-zero, no time limit | CONFIRMED |
| 14 | Flush | 2 | `0x2DBB3`, `0x2DBD6` | `pollAndFlush`, `flushChecked` | – | – | CONFIRMED |
| 15 | SetCallbacks | 1 | `0x2DA95` | `tsnInit` | alloc `2DA2:0FA4`, deref `0FF2`, free `1009` | – | CONFIRMED |
| 16 | GetLineRate | 1 | `0x2F4AC` | `initNet` | – | compared with 2400 (`0x960`) | CONFIRMED |

Never called: Connect (3), Disconnect (7), SwitchHost (13). RB inherits the live session and never logs in.
CONFIRMED. The offsets match `int14h-api.md` section 10 (`00 04 08 10 14 18 20 24 28 2C 30 38 3C 40`).

Call-site counts of the wrappers (Ghidra call graph plus a raw far-call scan):

| Wrapper | Callers | Callers (image) |
|---|---|---|
| `pollAndFlush` | 10 | main init steps `0x5D5C`, `0x5D75`, `0x5D87`, `0x5DC7`; exit wait `0x9B7A`; five UI input loops (`0x1F415`, `0x1F45E`, `0x1F8D3`, `0x1FA8C`, `0x317A1`) that keep the link polled while waiting for a key |
| `pump` | 11 | game loop `0x1FFD4` (`pump(1)`); the wait loops in `initNet` and `joinGame` (`pump(0)`); `waitKey` |
| `leaveGame` | 4 | main `0x5DFF`; game loop exits `0x1F5D8`, `0x20192`, `0x20251` |
| `sendStatus` (0x32) | 3 | start `0x2F890`, clean exit `0x9B70`, fatal-error routine `0x24CF2` (65 callers) |
| `serviceTick` | 0 direct | registered as a timer callback at `0x62C5` (section 7) |

## 3. Start-up and program chaining

`tsnInit` (`2DA2:0036`, image `0x2DA56`), called once from `main` (`0x5CC4`):

1. game heap initialisation (`3037:000E`, arg `0x1000`); INFERRED purpose.
2. stub call (`0x2DA68`): caches the table pointer. RB does not test it, so without TSNEXEC the INT 14h call
   goes to the BIOS. CONFIRMED.
3. SetCallbacks, then SetAckTimeout(90), pointer to `DS:72D2`; the clock source is the far pointer `0040:006C`.
4. GetPreviousProgram → SetNextProgram: exiting returns to the land that launched RB; NULL gives "Parent
   failure: application has been orphaned." and exit 1.
5. `DS:265E` = 1 (initialised).

The stub segment also supplies `2FB4:001A` (image `0x2FB5A`), a bare `retf`, which `tsnInit` registers through the C
`atexit` routine (`0:28E3`, call at `0x2DA75`): no TSNEXEC call happens at exit. CONFIRMED (code); purpose INFERRED.

`main` (image `0x5CAD`) then runs: tsnInit, memory check (needs `0x35B60` bytes, else "Insufficient
memory", ENTER, exit 0), `initNet` (`0x5CCE`), UI set-up with `pollAndFlush` between steps, `joinGame` (`0x5DCC`),
the game loop (`0x1FF8C`, from `0x5DF6`), `leaveGame` (`0x5DFF`). The `-C` argument is read for the key `music =
<driver>` only (`15CA:03DE`); no identity comes from `LSCI.CFG`. CONFIRMED.

## 4. Shared block

Read by `readSharedBlock` (`0x2DC4D`) twice (in `initNet` and `joinGame`) and by `sharedWord0` (`0x2DC17`); written
by `writeSharedParams` (`0x2DCE9`) once at exit.

| Block offset | Read into | Use | Status |
|---|---|---|---|
| `+00` w | local; `player+0x13A` | user id: cookie of the player joinNet, key of the 41/2 and 17 requests, last field of 0xC8, id sorted for team slots, `w@4` of the 0x32 status | CONFIRMED |
| `+04` w | `DS:72F9` | **group SID**: GrpJoin, GrpDel, GrpMem and `Send` target; compared with `w@2` of every group reply | CONFIRMED |
| `+10` 11 bytes | `player+0x12F` | name, sent in 0xC8 and shown when a player leaves | CONFIRMED |
| `+80` 15 bytes | `DS:72E2`..`72F0` | mission parameters (below) | CONFIRMED |
| length | local | `readSharedBlock` also returns `len-0x80` (the parameter length); no caller uses it | CONFIRMED |

`+10` in `int14h-api.md` section 9.3 and `messages.md` section 9/11 is hexadecimal: `movedata` copies 11 bytes from
`block+0x10` (image `0x2DC8C`..`0x2DCA4`), which is decimal 16, the user-name slot of `messages.md` section 5.3.
The "disagreement" noted in `messages.md` section 11 does not exist for RB. Word `+04` is the group SID for RB
(the land stores `global119`, the game group, there). CONFIRMED (RB side); the LSCI writer side is INFERRED from
`SL/script.120 runRedBaronScript`.

Mission parameters at `+0x80` (consumers searched over the whole code image; bytes `+11..+14` have no reader):

| `+0x80+` | RAM | Consumer | Meaning |
|---|---|---|---|
| 0 | `72E2` | copy `DS:2842`; assert 2..4 ("bogus # players", gamecomm line 1239); the wait loops | number of players in the game |
| 1 | `72E3` | selects plane `72E5` (≠0) or `72E6` (0); byte `@7` of 0xC8 | which side this player flies; INFERRED |
| 2 | `72E4` | `2F556` clamps to 0..2 → `DS:2848` | difficulty or skill; INFERRED |
| 3, 4 | `72E5`, `72E6` | plane for each side, loaded through `0704:01FB`, getters `2EBB1`/`2EBB9` | plane types |
| 5 | `72E7` | `2EC7C`: non-zero → game date taken from host time (day `72F3`, month `72F4`); getter `2EC96` | "use host date"; INFERRED |
| 6 | `72E8` | `2EC33` (jump table at `2EA6:0214`, image `0x2EC74`): 1 → hour 0, 2 → host hour (`DS:72F6`), 3 → 7, 4 → 17, other → 12 (`DS:6242`) | time-of-day mode; CONFIRMED mapping (the first draft had 1 and 2 swapped) |
| 7 | `72E9` | getter `2EBA9` → `4192:020E` modes 1..3 swap side assignment | team arrangement; INFERRED |
| 8 | `72EA` | `313C:0012(byte)` | unknown |
| 9, 10 | `72EB`, `72EC` | getters `2EC9E`, `2ECA6` → `417C:00E4` (two option words) | option flags; INFERRED |

Exit (`leaveGame`, once, guarded by `DS:284B`): `writeSharedParams` keeps the first 0x80 bytes of the current block,
appends 7 bytes at `+0x80` and stores `0x87` bytes (SetSharedData length). The first word is the running score counter `DS:2CD1`
(read through `314F:0102`, image `0x315F2`, the same getter that feeds the 0xC9 field at payload offset 1; adjusted by +5, +10, +100, -50 and similar steps in the scoring code at image `0x31522`..`0x315E1`). The other 5 bytes are uninitialised stack. The fatal
path does not write the block. INFERRED use: the land reads the score back; no reader was traced.

## 5. Host command set

### 5.1 C→H (hostcomm builders; `2DA2` offsets, image = `0x2DA20` + off)

All builders alloc the buffer through the callback, fill it, call Send, then `flushChecked` and `pollChecked`.
The `line` is the `__LINE__` passed to `fatalError` when Send ≠ 1.

| Cmd/sub | Builder | Len | Layout | Called from | Meaning | Status |
|---|---|---|---|---|---|---|
| 41/2 (0x29) | `03D3` (image `0x2DDF3`), line 406 | 9 | `b 0x29, b 2, w 0, w userId, b 0x65, w 0` | `initNet` `0x2F364` | look up the game's service object for game type 101 (`DS:2816`, never written); same shape as the Yserbius 41/2 | CONFIRMED layout; meaning INFERRED |
| 17/0 (0x11) | `08AB` (image `0x2E2CB`), line 646 | 8 | `b 0x11, b 0, w serviceSid, w userId, w prop` | `initNet` `0x2F3B7` (prop 9), `0x2F417` (prop 0xF) | read one integer property of the service object; the reply is a setInt (13) | CONFIRMED |
| 36/2 (0x24) | `0477` (image `0x2DE97`), line 427 | 2 | `b 0x24, b 2` | `initNet` `0x2F50F` | get host time | CONFIRMED |
| 7/0 kind 3 | `05BA` (image `0x2DFDA`), line 495 | 12 | `b 7, b 0, w 0, w userId, b 3, b 0x65, w 0xFFFF, w 1` | `joinGame` `0x2F621` | joinNet: the player's in-game object, game type 101, capacity 1; reply ObjID | CONFIRMED |
| 10 | `0959` (image `0x2E379`), line 669 | 6 | `w 10, w groupSid, w playerSid` | `joinGame` `0x2F689` | GrpJoin | CONFIRMED |
| 12 | `0A87` (image `0x2E4A7`), line 715 | 6 | `w 12, w groupSid, w playerSid` | `joinGame` `0x2F6F1` | GrpMem: list the group | CONFIRMED |
| 11 | `09F0` (image `0x2E410`), line 692 | 6 | `w 11, w groupSid, w playerSid` | `leaveGame` `0x2F5A4`, `leaveOnFatal` `0x2EE67` | GrpDel: leave the group | CONFIRMED |
| 9 | `0679` (image `0x2E099`), line 517 | 4 | `w 9, w playerSid` | `leaveGame` `0x2F5B4`, `leaveOnFatal` `0x2EE7E` | ObjFree (the player's own object only) | CONFIRMED |
| 50 (0x32) | `033A` (image `0x2DD5A`), line 380 | 6 | `b 0x32, b state, w 0, w userId` | state 0 at `0x2F890` (game starts); state 1 at `0x9B70` (clean exit) and `0x24CF2` (fatal error) | per-user status: 0 playing, 1 leaving; same layout as GOLF's `32 b w w`; meaning INFERRED | CONFIRMED layout |
| 2 | `sendToGroup` (image `0x2EF4B`), line 855 | 25 | `b 2, b 0, w groupSid, w playerSid, …` + 0xC8 body (section 7.1) | `sendPlayerInfo` | `Send` to the group SID | CONFIRMED |
| 28 (0x1C) | `sendMulticast` (image `0x2EFD5`), line 888 | 44 (5+2n+35, n=2) | `b 0x1C, w playerSid @1, w n @3, w recip[n] @5, a[35] body` | `broadcastState` | multicast, 0xC9 body (section 7.2); `n` = 2 | CONFIRMED |

Linked but never called (layouts CONFIRMED): 7/0 kind 2 (`0500`, 12 bytes, `w 7, w 0, w 0, w userId, b 2, b type, w param,
w max`), 14 setStr (`0709`, `w 14, w sid, w sid, w prop, s`), 13 setInt (`07E0`, `w 13, w sid, w sid, {w prop, w val}…`),
4 lock (`0B1E`, `w 4, w fromSid, w lockId`), 6 unlock (`0BC5`, `w 6, w fromSid, w lockId`). The Ack/Nak paths for 4
(section 5.2) are therefore dead as well.

### 5.2 H→C dispatch (`pump`, `2DA2:0C6C`; jump table `CS:0EA9`, image `0x2E8C9`)

Reads byte 0. Anything not listed is freed unread. CONFIRMED.

| Cmd | Reads | Handler (image) | Effect | Status |
|---|---|---|---|---|
| 0 Ack / 1 Nak | `b@4` = 4 | `0x2ED89` / `0x2ED7F` | sets `DS:2838` to 1 / 0; never read (lock is dead) | CONFIRMED |
| 2 Send | `w@4` from, `b@6` msgType | `0x2EE95` | 1 and 28: ignored; 200 (0xC8): player info; 201 (0xC9): state update (section 7) | CONFIRMED |
| 8 ObjID | `w@6` sid | `0x2ED17` | the player object's SID (`player+0x13C`), sets flag bit 0; only the first is taken; the cookie is not checked | CONFIRMED |
| 9 ObjFree | – | `0x2ED3B` | ignored (empty function) | CONFIRMED |
| 10 GrpJoin | `w@2` group, `w@4` who | `0x2ED93` | group must equal `DS:72F9`; `who` = self → flag bit 1 (join confirmed); other → member counter `DS:2840` +1 | CONFIRMED |
| 11 GrpDel | `w@2` group, `w@4` who | `0x2EDC8` | other player in the group → remove that player's aircraft and show its name; `who` = self or unknown → ignored | CONFIRMED |
| 12 GrpMem | `w@2` group, `w@6…` members | `0x2EE20` | `DS:2840` = (len-6)/2 members; sets "mem reply" `DS:283F`; group must equal `DS:72F9` | CONFIRMED |
| 13 setInt | `w@2` obj, pairs @6 | `0x2ED45` | obj must equal the service SID; **only the first pair** is read: prop 9 → `DS:2844`, prop 0xF → `DS:2846`; sets `DS:283E` | CONFIRMED |
| 14 setStr | – | `0x2ED40` | ignored (empty function) | CONFIRMED |
| 36 HostInfo | `b@1` type | `0x2ECB5` / `0x2ECBA` | type 0 ignored; type 2 host time (section 8.2) | CONFIRMED |
| 41 | `w@6` | `0x2EE39` | service SID → `DS:283A`/`283C`, sets `DS:2839` | CONFIRMED |
| 48 Unsolicited | text @2 | `0x2EF31` | shown through `309C:0008` only once the game runs (`DS:2837`): up to two lines of 50 characters | CONFIRMED |

## 6. Session flow and waits

Each wait loop is `timeoutStart; while (!flag) { pump(0); if (timeoutExpired) fatal(...) }`. `DS:106A` (a user-abort flag set by the input code at `0x1EDEB`)
ends any wait with the fatal "user abort!". The timeout is the same everywhere: more than `0x6EA0` = 28320 timer
ticks at 236.7 Hz (119.6 s) on the PIT counter `DS:307D`, or `0x882` = 2178 BIOS ticks (119.6 s) if the PIT module
is off. CONFIRMED (`2EA6:000D`, `0033`).

`initNet` (image `0x2F2C0`):

| Step | Sends | Waits for | Timeout text (gamecomm line) | Status |
|---|---|---|---|---|
| 0 | – | `readSharedBlock`; parameters copied to `DS:72E2` | assert "bogus # players" (1239) | CONFIRMED |
| 1 | 41/2 `{userId, 0x65}` | cmd 41, `w@6` | "get perm obj timeout" (1257) | CONFIRMED |
| 2 | 17 prop 9 | setInt on the service SID, first pair prop 9 | "get group stats timeout" (1270) | CONFIRMED |
| 3 | 17 prop 0xF | setInt, first pair prop 0xF | "get group stats timeout" (1284) | CONFIRMED |
| 4 | – | prop 9 value must be `0x0201` (hi byte 2, lo byte 1), else "You appear to have an incompatible version of Multi-Player Red Baron. Contact the ImagiNation Network for an update." and exit 0. Prop 0xF is clamped to 5..30 (so any value is accepted), multiplied by 14160 (`0x3750`) and offset by 236 (`0xEC`), and stored as a 32-bit count at `DS:2B8A` by `30EB:007F` (image `0x30F2F`); 14160 ticks = 60 s at 236.7 Hz, so prop 0xF looks like a length in minutes (INFERRED) | – | CONFIRMED arithmetic |
| 5 | – | GetStatus `AL` = 2, or 3 with line rate ≥ 2400 | "requires a %d baud or greater connection." | CONFIRMED |
| 6 | 36/2 | host time (`b@1` = 2) | "get host time timeout" (1330) | CONFIRMED |

`joinGame` (image `0x2F5C5`):

| Step | Sends | Waits for | Timeout text (line) | Status |
|---|---|---|---|---|
| 1 | – | re-read block: group SID → `72F9`, user id and name into the player record; `player+0x140` = BIOS tick (`0040:006C` low word), used as a random seed | – | CONFIRMED |
| 2 | joinNet kind 3 | ObjID: player SID, flag bit 0 | "create object timeout" (1461) | CONFIRMED |
| 3 | GrpJoin | GrpJoin with `who` = self: flag bit 1 | "join group timeout" (1485) | CONFIRMED |
| 4 | GrpMem | GrpMem reply | "get group mem timeout" (1507) | CONFIRMED |
| 5 | – | "Waiting for other players...": loop until member count `DS:2840` ≥ expected `DS:2842`; on timeout `2842` := `2840` | – (no fatal) | CONFIRMED |
| 6 | 0xC8 to the group | "Gathering player info...": loop until `DS:2841` (starts at 1 for self; +1 per new sender) ≥ `2842`; on timeout `2842` := `2841`+1 | – | CONFIRMED; the "+1" looks like an off-by-one, INFERRED |
| 7 | – | after the loop `DS:2836` = 1 closes the roster. Fewer than 2 players → "Timed out waiting for the other player(s). Unable to start game." (1560) | | CONFIRMED |
| 8 | 0x32 state 0 | – | – | CONFIRMED |

After step 6, `2F8A2` numbers the players by ascending SID (`+0x13E`), and `2F966` numbers each side by ascending user
id (`+0x13F`); the cloud generator is seeded from the host month and `player0+0x140`. Every client therefore derives
the same order without a host message. CONFIRMED (code); the cloud seeding purpose is INFERRED.

## 7. Game messages (C→C)

Delivered to `onSend` (`0x2EE95`) as `Send`: `b 2, b 0, w toSid, w fromSid, b msgType @6, body @7`.

### 7.1 msgType 200 (0xC8), player info

Built by `sendPlayerInfo` (`0x2F1AE`), sent to the **group SID** with `fromSid` = the sender's player SID, once, after the member
wait. 25 bytes.

| Offset | Field | Source | Status |
|---|---|---|---|
| @6 | `b 0xC8` | | CONFIRMED |
| @7 | `b` side | parameter `+1` (`DS:72E3`) | CONFIRMED |
| @8 | `a[11]` name | `player+0x12F` (block `+0x10`) | CONFIRMED |
| @0x13 | `w` | `player+0x13A` (user id) | CONFIRMED |
| @0x15 | `w` | `player+0x13C` (own SID) | CONFIRMED |
| @0x17 | `w` | `player+0x140` (BIOS tick seed) | CONFIRMED |

The receiver (`0x2F24A`) ignores it once `DS:2836` = 1, ignores its own SID, ignores a SID already known, and treats
`fromSid` = group SID as a fatal "group sent init!" (line 1136). A new sender is added by `3FB5:0314` (an `add_crate()
failed` fatal if that fails, line 1166). CONFIRMED.

### 7.2 msgType 201 (0xC9), state update

Built by `broadcastState` (`0x2F087`, from the game loop `0x201E4`), sent as a **multicast** with the recipients
`{groupSid, ownPlayerSid}`. Payload 35 bytes, so with n = 2 the multicast message is `5 + 2n + 35` = 44 bytes. Received as `Send` with
the payload at `@6`.

| Payload offset | Field | Source | Status |
|---|---|---|---|
| 0 | `b 0xC9` | | CONFIRMED |
| 1 | `w` | score change since the previous packet: `314F:0102` (image `0x315F2`, returns the score `DS:2CD1`) minus `DS:2A8A`, which is then reloaded with the score (`0x2FFB6`..`0x3001D`); the first draft called it a game clock. No reader in the receive chain (`0x30052`, `0x30BB5`, `0x30C3D`); the receiver paces by its own timer `DS:120E` | CONFIRMED sender; use INFERRED |
| 3 | `w` | player flags `+0x145` | CONFIRMED |
| 5..16 | 6 `w` | position `+0x08..0x13` | CONFIRMED |
| 17..22 | 3 `w` | attitude `+0x14..0x18` | CONFIRMED |
| 23..30 | 4 `w` | velocity `+0x10A`, `+0x108`, `+0x106`, `+0x104` | CONFIRMED |
| 31 | `b` | player number of the SID in `+0x147` when flag bit 1 of `+0x144` is set, else `0xFF` (`30AF1`) | CONFIRMED; meaning INFERRED |
| 32 | `b` | bit mask of players (by `+0x13E`) whose flag bit 0 of `+0x144` is set | CONFIRMED |
| 33 | `w` | bit mask, by aircraft index (`+0x85`), of aircraft with flag `0x8` set and `0x40` clear; each is marked `0x40` so it is reported once | CONFIRMED; meaning (hits or kills) INFERRED |

`onState` (`0x2F13F`) handles it: from self → `DS:7300` := 1 (the echo); from an unknown SID → ignored; else dead-reckoning
state is updated (`3FF5:0102`, `30AB:0105`, `018D`). The length is not checked. CONFIRMED.

Send gate: `3FF5:003E` is true only when the 0xEC-tick timer (`24DA`) has expired **and** `DS:7300` = 1; after each send
`3FF5:0001` clears the flag and re-arms the timer. The first send is forced (`3FF5:0028`). So one 0xC9 leaves about
once per second (0xEC = 236 clock ticks; the clock `DS:1353` is a timer callback with reload 1, 236.7 Hz), and only after
the previous one returned. CONFIRMED (timer constants); the one-second figure follows from them. The own SID in the recipient
list exists to produce that echo. INFERRED.

## 8. Time, timing and failure

### 8.1 Timers

| Source | Value | Evidence |
|---|---|---|
| Link retransmit timeout | 90 BIOS ticks ≈ 4.9 s | SetAckTimeout, `0x2DAAC` |
| PIT | channel 0 reprogrammed to `0xFFFF/13` = 5041 → 236.7 Hz; init at `0x2CAA0` (arg 13), ISR `2510:3FC9` (image `0x290C9`) | `0x29027`..`0x2908A` |
| `serviceTick` | timer callback, reload 12 (`0x62C5`), 19.7 Hz; counter `DS:2662` skips 5 of 6 calls: Service every 72 ticks ≈ 0.30 s | `0x2EA40`; `0x62CC`..`0x62D4` |
| Wait timeout | 119.6 s (section 6) | `0x2EA93` |
| State update | about 1 s, echo-gated (section 7.2) | `0x2FF51` |
| TSNEXEC tick counter | rewritten from the BIOS tick before every Poll and Flush, not from the PIT | `0x2DB18`..`0x2DB2B`, `flushChecked` |

Service runs from the timer interrupt while the main line may be inside Send, Poll, Receive or Flush, so TSNEXEC's
reentrancy lock matters: a busy lock makes Service return 0 and a busy Send return 0, which RB treats as a fatal
"send failure". INFERRED risk for a replacement that holds a lock across the caller's thread.

### 8.2 Host time (`b 0x24, b 2, …`)

Reply layout, read by `onHostTime` (`0x2ECBA`) from `msg+2`:

| Offset | Field | Stored | Used | Status |
|---|---|---|---|---|
| @2 | `b` years since 1900 | `DS:72F1` (+1900) | no reader | CONFIRMED |
| @3 | `b` month, 0-based | `DS:72F4` (+1) | month of the scenario date and the cloud seed | CONFIRMED |
| @4 | `b` day of month | `DS:72F3` | scenario date | CONFIRMED |
| @5 | `b` hour, bit 7 ignored | `DS:72F6` | time of day when parameter `+6` = 2 | CONFIRMED |
| @6, @7 | `b` minute, `b` second | `DS:72F5`, `72F8` | no reader | CONFIRMED |

Every player gets the date and time of day from the host, so the sim date and sun agree. Without the reply, "get
host time timeout" is fatal after 119.6 s. CONFIRMED.

### 8.3 Connection-lost and failure paths

| Trigger | Where | Result | Status |
|---|---|---|---|
| Poll or deferred Service status 1 (carrier lost) | `pollChecked`, line 218 | `DS:2661` := 1; fatal "Connection to ImagiNation Network has been lost (Poll)/(Doit).\n  We will attempt to reconnect." The label is `(Poll)` when the deferred Service status was set, else `(Doit)`; it looks inverted. | CONFIRMED |
| any other non-zero status (2 or 3 per `int14h-api.md`: too many NAKs or retransmits) | `pollChecked`, line 234 | same screen with "has failed %s"; the label is `(Doit)` if Poll's own status ≠ 0, else `(Poll)` (the first draft had this reversed; both screens label a deferred Service failure `(Poll)`) | CONFIRMED |
| Send returns ≠ 1 | every builder, `sendToGroup`, `sendMulticast` | fatal "send failure %d", `%d` = command byte | CONFIRMED |
| any wait times out | section 6 | fatal with the timeout text | CONFIRMED |
| missing shared block | `readSharedBlock` line 322 | fatal "orphaned application!" | CONFIRMED |

`fatalError` (`0x2E8DF`) strips the file name, calls `leaveOnFatal` (only when `DS:2661` = 0: GrpDel if the join was sent, `DS:2819`; ObjFree if the player object was created, `DS:2818`), runs the clean
exit (`0x9B59`: 0x32 state 1 and the idle wait, both skipped when the connection is lost), prints "Fatal Error
Encountered / File / Line / Message", waits for ENTER and calls `exit(0)`. The promised reconnect is not implemented in RB: TSNEXEC
then runs the next program, which is the previous one (the land), whose own start-up redials. CONFIRMED (code
path); the redial is INFERRED.

## 9. How players are grouped

| Fact | Evidence | Status |
|---|---|---|
| The land (LSCI `RedBaronQueue`, a `QueuedGroup` with a `GameGroup` held in `global119`) forms the game and passes the group SID in block `+04` and the player count in parameter `+0` | `SL/script.120`; section 4 | INFERRED for the LSCI side, CONFIRMED for the RB side |
| All players of one game receive the same group SID and the same parameters | the group SID is used as both GrpJoin target and `Send` address | INFERRED |
| A player's own object is a joinNet kind 3, type 101, capacity 1 | `0x2DFDA` | CONFIRMED |
| 2 to 4 players | assert at `0x2F31C`; section 6 | CONFIRMED |
| Player numbers come from sorted SIDs, side slots from sorted user ids | `0x2F8A2`, `0x2F966` | CONFIRMED |
| A late joiner (after the roster closes) is ignored by the others' 0xC8 handler and unknown to the 0xC9 handler | `0x2F24A`, `0x2F13F` | CONFIRMED |

## 10. Server responsibilities

What innkeeper must do for RB, in order of use:

1. **Inherit the session.** No Login, no Connect. The shared block must exist and be non-empty when RB starts: `+0` user
   id, `+4` group SID, `+0x10` name, `+0x80` 15 parameter bytes (section 4). A test harness that starts RB without
   the land must write it first. CONFIRMED (fatal otherwise).
2. **Answer 41/2** (`b 0x29, b 2, w 0, w userId, b 0x65, w 0`) with a cmd 41 message carrying the service SID at `w@6`,
   within 119 s. INFERRED meaning, CONFIRMED wire shape.
3. **Answer 17 (prop 9 and prop 0xF)** with setInt (13): `b 13, b 0, w serviceSid @2, w x @4, w prop @6, w value @8`.
   Prop 9 must be `0x0201`; prop 0xF is clamped to 5..30 and scaled by 14160 ticks per unit (INFERRED meaning: minutes; any value is accepted). Answer each request with its own
   setInt whose **first** pair is the requested prop: the handler reads only pair 0, and the client waits for each reply separately.
4. **Answer 36/2** with `b 0x24, b 2, year-1900, month0, mday, hour, minute, second`; the host's wall clock. All players in
   a game must get the same hour.
5. **joinNet kind 3, type 101** → ObjID (cmd 8) with a new player-object SID at `w@6`, unique host-wide. One per player.
6. **GrpJoin (10)**: add the player to the group named in `w@2`; send `b 10, …, w group @2, w who @4` to **every member
   including the joiner**: the joiner needs its own echo to continue, and members use the others' joins to count the roster.
   A Nak is not handled by RB (it times out after 119.6 s). CONFIRMED client side.
7. **GrpMem (12)**: reply with `b 12, b 0, w group @2, w ?, w member[n] @6` (n members including the requester).
8. **GrpDel (11)** and **ObjFree (9)**: remove the player; tell the remaining members `b 11, …, w group @2, w who @4`. Do the
   same when a connection drops, or the others keep a ghost aircraft. INFERRED for the drop case.
9. **Send (2)** to a group SID: deliver to every other member with `fromSid` set to the real sender, never the group SID
   (RB treats `fromSid` = group as fatal in the 0xC8 handler). The body is opaque.
10. **Multicast (28)**: `b 0x1C, w fromSid @1, w n @3, w recip[n] @5, body`. Deliver to each recipient a `Send` whose payload
    is the body: a group SID fans out, an individual SID gets one copy, and the **sender's own SID** must get one copy back
    (the loopback echo that releases the next state update, section 7.2). If a group fan-out also reaches the sender,
    the second copy is harmless. INFERRED; no capture exists.
11. **Absorb 50 (0x32)** (`w@4` user id, `b@1` state 0 or 1). No reply is expected. A server may use it to show who is in the
    sim. INFERRED.
12. **Optionally push 48 (Unsolicited)**: text at `@2`, up to two 50-character lines, only shown during the game.
13. **Timing**: ACK every data frame promptly (retransmit timeout 90 BIOS ticks, 4.9 s, with 12 timeouts fatal); answer every
    request in section 6 within 119 s; keep Service callable from an interrupt-like context.
14. **State**: RB needs no host storage. The host keeps SIDs, group membership and the service object's two properties.

## 11. Findings that touch other documents

- `int14h-api.md` section 9.3 and `messages.md` sections 9 and 11: `+10` is a hex offset (decimal 16); RB reads the user name
  from the LSCI user-name slot. Not a conflict.
- `messages.md` section 3.2: command 17 (`b 17, b 0, w obj, w userId, w prop`) is used for property reads by RB with a setInt reply, and
  `messages.md` section 9 lists 0x29 as "ObjExists" while RB and Yserbius both call it 41/2 "service lookup". Command 50 (0x32) has the same
  `b w w` shape in RB as in GOLF: `b 0x32, b 0|1, w 0, w userId`.
- `int14h-api.md` sections 5 and 10: RB calls Service from a PIT timer callback (every 72 ticks, about 0.3 s), not from its main loop. GOLF's Service call site was not compared.

## 12. Open questions

- What the host's service object for game type 101 is, and what its properties 9 (`0x0201`) and 0xF (5..30) mean.
- How the host forms the group: which messages make several land clients share one group SID, and who sets the player count.
- Whether the host echoes a group `Send` to its sender, and whether a multicast to a group SID also reaches the sender.
- What 50 (0x32) state 0 and 1 are used for on the host, and whether the host expects a reply.
- The meaning of parameter bytes `+7`, `+8`, `+9`, `+10` and the score word at `+0x80` after exit.
- Whether the `(Poll)`/`(Doit)` label swap and the `2841+1` count are bugs or intended.

## 13. Verification

Independent re-check (adversarial pass): Ghidra project `work/ghidra/verify-census-redbaron/` (xref and `INT` dump),
raw byte scans and capstone disassembly of `RB_unpacked.EXE`, and `tools/int14h_census.py`.

| Claim | Check | Result |
|---|---|---|
| One `INT 14h`, one stub | scan for `CD 14`, the 20-byte stub pattern, `INT 21h` get-vector (AH 35h) with AL 14h | one `CD 14` (image `0x2FB50`), one stub (`0x2FB46`); the only vectors touched are `24h` (`0x610`..`0x65A`, `0x10C5`, `0x10DE`, `0x129F`). CONFIRMED |
| Cached pointer touched only by the stub and 33 call sites | every absolute `DS:2A86` and `DS:2A88` operand in the image; `C4 /r 86 2A` (`les`) forms followed by `26 FF /3` | 33 `les`+`lcall es:[bx+N]` sites, 0 other readers, 2 stub accesses; Ghidra sees 32 of the 33 (misses the dead lock builder, `0x2E599`). CONFIRMED |
| Export per site | displacement `N` ÷ 4 | GetStatus 1, GetSharedData 4, SetSharedData 1, Send 16, Receive 1, SetAckTimeout 1, Poll 1, SetNextProgram 1, Service 1, GetPreviousProgram 1, IsTransmitIdle 1, Flush 2, SetCallbacks 1, GetLineRate 1; Connect, Disconnect, SwitchHost 0. Same totals from `tools/int14h_census.py`. CONFIRMED |
| Stub callers | far calls and relocations to segment `2FB4` | one call (`0x2DA68`); the other `2FB4` reference (`0x2DA6E`) is the `atexit` registration of a `retf`. CONFIRMED |
| Caller counts | far and near call scans | `sharedWord4` 0, `sharedWord0` 3 (`0x9B67`, `0x24CE9`, `0x2E021`), `readSharedBlock` 2, `writeSharedParams` 1, `pollAndFlush` 10, `pump` 11, `pollChecked` 18 (was 17), `leaveGame` 4, dead builders (kind 2, setStr, setInt, lock, unlock) 0. CONFIRMED |
| Builder layouts and `__LINE__` values | disassembly of all 14 builders, `sendToGroup`, `sendMulticast`, `broadcastState` | all layouts and lines in section 5.1 reproduce (380, 406, 427, 495, 517, 646, 669, 692, 715, 855, 888); `broadcastState` is called with `{group, own SID, 0}`, so n = 2 and the message is 44 bytes. CONFIRMED |
| `pump` dispatch | jump table `CS:0EA9` and compare chain | commands 0, 1, 2, 8, 9, 10, 11, 12, 13, 14, 36, 41, 48 and handler arguments as in section 5.2; ObjFree handler is `0x2ED3B`, not `0x2ED40`. CONFIRMED |
| Corrections made | | 0xC9 payload offset 1 is a score delta, not a clock; time-of-day mode 1 = hour 0 and 2 = host hour (were swapped); "failed" screen label reversed; prop 0xF is scaled to ticks; `pollChecked` has 18 callers; five, not four, UI input loops; IsTransmitIdle loops until non-zero; handle is 32-bit |
| Not reproduced | | the LSCI side (`RedBaronQueue` / `GameGroup`, section 9) and the host-side meaning of properties 9 and 0xF, command 17 and command 50 were not re-derived here. UNCERTAIN |

`tools/int14h_census.py` now parses short displacements correctly and agrees with the table above; the early-draft
mis-parse mentioned in the hand-off is not present in the current file.

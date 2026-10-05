# INT 14h census: 3-D Golf (`GOLF.EXE`)

Every use of TSNEXEC's INT 14h export table by `work/games/GOLF/GOLF.EXE` (Borland C++ 1991, large model, unpacked,
Borland VROOMM overlays), the message set it speaks through Send and Receive, and what a server must do for it.
Background: `docs/protocol/int14h-api.md` (exports, calling convention, shared block), `docs/protocol/messages.md`
(LSCI message catalogue, whose command numbers GOLF reuses).

Status words: CONFIRMED = seen in code or data with an address; INFERRED = reasoned, not seen.

## 0. Addresses and method

| Item | Value |
|---|---|
| MZ header | `0x1A00` bytes; **file offset = image offset + `0x1A00`** |
| Resident image | file `0x1A00`..`0x25EB0` (`0x244B0` bytes), then an `FBOV` overlay area at file `0x25EB0` (60640 bytes of overlay data from `0x25EC0`) |
| Data segment | DS = image segment `0x1AF6`; **file offset of DS:`x` = `0x1C960 + x`** |
| Ghidra view | image segment + `0x1000` (a function at image `0x132C6` is `2329:0036`) |
| Decompilation | `work/decomp/golf/golf_all.c` (all 629 functions Ghidra found), `golf_tsn_library.c` (the TSN library only, with the names below), `golf_names.txt` (address to name) |
| Call-site finder | capstone, one-off scripts; the pattern is `les bx,[1898]` (`C4 1E 98 18`) then `lcall es:[bx+N]` (`26 FF 5F N`) |

Addresses are image offsets; add `0x1A00` for the file. Library functions are cited as `image 0xNNNNN (seg:off)`
with `seg:off` in the original image view (`1329:xxxx` is `hostcomm.c`, `1451:xxxx` is `gamecomm.c`).

## 1. Lookup stub and cached pointer

| Item | Where | Status |
|---|---|---|
| Stub `tsnLookupTable`: `mov ax,[1898]; or ax,[189A]; je +1; retf; int 14h; mov [1898],ax; mov [189A],dx; retf` | image `0x13282` (`1328:0002`), file `0x14C82`; `INT 14h` at file `0x14C8C` | CONFIRMED |
| Cached table pointer `p` | DS:`1898` (offset word) and DS:`189A` (segment word), file `0x1E1F8` | CONFIRMED |
| Only caller of the stub | `hcInit` image `0x132CC`: `xor dx,dx; lcall 1328:0002`, then `cmp [189A],0` | CONFIRMED |
| Missing executive | segment 0: prints `TSN Executive not loaded!` (DS:`18B1`) and calls `exit(1)` (RTL `0000:132A`) | CONFIRMED |
| Overlays | no `INT 14h`, no `[1898]`/`[189A]` reference anywhere in the overlay data | CONFIRMED (byte search) |
| Other table users | none: the 30 call sites in section 2 are all of them | CONFIRMED |

## 2. Export usage

All calls are `les bx,[1898]; lcall es:[bx+N]` with arguments pushed right to left and popped by the caller.
GOLF uses 11 of the 17 exports at 30 call sites. It never calls GetStatus, Connect, Disconnect, IsTransmitIdle,
SwitchHost or GetLineRate: it inherits the session from the land that launched it.

| # | Export | Sites | Wrapper(s) (image) | Arguments GOLF passes | How the result is used |
|---|---|---|---|---|---|
| 0 | GetStatus | 0 | none | none | n/a |
| 1 | GetSharedData | 4 | `hcGetUserWord4` `0x13465` (no callers), `hcGetUserSid` `0x1349A`, `hcGetSharedParams` `0x134CE`, `hcSetSharedParams` `0x13565` | `&far*` local (SS:`bp-4`) | length 0 means "no block"; `hcGetSharedParams` raises a fatal error ("orphaned application!"); the others return 0. Section 5 |
| 2 | SetSharedData | 1 | `hcSetSharedParams` `0x13565` | `(local buffer, 0x80 + len)`, `len` at most `0x80` | none; only run on exit |
| 3 | Connect | 0 | none | none | n/a |
| 4 | Send | 15 | 13 in `hostcomm.c` (`0x135D0`, `0x1366D`, `0x1371B`, `0x137A2`, `0x13986`, `0x13A85`, `0x13B67`, `0x13C3C`, `0x13CF2`, `0x13DDF`, `0x13EC8`, `0x13F67`, `0x14013`), 2 in `gamecomm.c` (`0x14C6D`, `0x14CFD`) | `(handle, len)`; the handle is the `farmalloc` pointer, the message starts at its offset 0 | anything but 1 is fatal: `send failure %d`, printing the **command byte**, not the result. 7 sites are live, 8 have no caller (section 3.3) |
| 5 | Receive | 1 | `hcPump` `0x140BF` | `(&handle)`, a far pointer local | 0 = nothing waiting; else length (kept in `si`) and handle. The handle is dereferenced, the command byte dispatched, then the message freed with `farfree` (`hcFree`) |
| 6 | SetAckTimeout | 1 | `hcInit` `0x13319` | `90` (`0x5A`) | far pointer stored at DS:`8708`; every Poll, Flush and Service first writes the game's 32-bit tick (DS:`92EA`/`92EC`) through it |
| 7 | Disconnect | 0 | none | none | n/a |
| 8 | Poll | 1 | `hcPoll` `0x13375` | none | status 0..3; 1 to 3 print the table at DS:`18A4` (section 4.3). Poll runs before every Receive and after every Send |
| 9 | SetNextProgram | 2 | `hcInit` `0x13341`; `hcGetBackOn` `0x1430D` | the far pointer from GetPreviousProgram; `NULL` (user answers `N`) | return value ignored |
| 10 | Service | 1 | `hcTimerTick` `0x144DC`, called from the INT 8 handler | none | non-zero status is stored in DS:`189D` and reported by the next `hcPoll` |
| 11 | GetPreviousProgram | 1 | `hcInit` `0x13329` | none | NULL gives the fatal error `Parent failure: application has been orphaned.`; else it is the argument of SetNextProgram |
| 12 | IsTransmitIdle | 0 | none | none | n/a |
| 13 | SwitchHost | 0 | none | none | n/a |
| 14 | Flush | 2 | `hcPollAndFlush` `0x13435` (no callers), `hcFlush` `0x13446` | none | none; `hcFlush` stamps the tick first and runs after every Send and after every pump iteration |
| 15 | SetCallbacks | 1 | `hcInit` `0x1330C` | `(1329:11D4, 1329:121E, 1329:1232)` = `hcAlloc`, `hcDeref`, `hcFree` | none |
| 16 | GetLineRate | 0 | none | none | n/a |

### 2.1 Call-site list (30)

Site = address of the `les bx,[1898]` (file = image + `0x1A00`).

| Site | Export | Containing function |
|---|---|---|
| `0x1330C` | SetCallbacks | `hcInit` |
| `0x13319` | SetAckTimeout | `hcInit` |
| `0x13329` | GetPreviousProgram | `hcInit` |
| `0x13341` | SetNextProgram | `hcInit` |
| `0x13390` | Poll | `hcPoll` |
| `0x1343C` | Flush | `hcPollAndFlush` (no callers) |
| `0x1345B` | Flush | `hcFlush` |
| `0x1346E` | GetSharedData | `hcGetUserWord4` (no callers) |
| `0x134A3` | GetSharedData | `hcGetUserSid` |
| `0x134D7` | GetSharedData | `hcGetSharedParams` |
| `0x13572` | GetSharedData | `hcSetSharedParams` |
| `0x135C2` | SetSharedData | `hcSetSharedParams` |
| `0x13623` | Send | `hcSendCmd50` (no callers) |
| `0x136D1` | Send | `hcSendObjExists` (no callers) |
| `0x13758` | Send | `hcSendGetHostTime` (no callers) |
| `0x138D6` | Send | `hcCreateObject` |
| `0x13A05` | Send | `hcFreeObject` |
| `0x13B1B` | Send | `hcSendSetStr` (no callers) |
| `0x13BF0` | Send | `hcSendSetInt` (no callers) |
| `0x13CA7` | Send | `hcSendCmd17` (no callers) |
| `0x13D47` | Send | `hcGroupAdd` |
| `0x13E30` | Send | `hcGroupDelete` |
| `0x13F1D` | Send | `hcGroupGetMembers` |
| `0x13FC8` | Send | `hcSendLock` (no callers) |
| `0x14074` | Send | `hcSendUnlock` (no callers) |
| `0x140CD` | Receive | `hcPump` |
| `0x1430D` | SetNextProgram | `hcGetBackOn` |
| `0x14507` | Service | `hcTimerTick` |
| `0x14CB1` | Send | `gcSendToObject` |
| `0x14D5C` | Send | `gcMulticast` |

CONFIRMED by byte search of the whole file; "no callers" means no far call, near call, far pointer or relocation
reference in the resident image and no call with a matching offset in any overlay (section 8).

## 3. The shared TSN client library

GOLF links `src\tsn\hostcomm.c`, `src\tsn\gamecomm.c`, `src\tsn\edit.c` and `src\tsn\dialog.c` (strings at DS:`19DD`,
`1AE0`, `1834`, `1870`). The names below are this project's; the original names are unknown. `hostcomm.c` is the part
the other DOS clients are expected to share; `gamecomm.c` is the golf layer on top. CGENN's source name is
`c:hostcomm.cpp`, so it is a C++ rewrite and the layouts below may differ there (INFERRED).

### 3.1 `hostcomm.c` (image `0x13282`..`0x1450F`)

| Image (seg:off) | Name | Role | Status |
|---|---|---|---|
| `0x13282` (`1328:0002`) | `tsnLookupTable` | the INT 14h stub, section 1 | CONFIRMED |
| `0x13297` (`1329:0007`) | `hcWaitKey` | loop of `hcPump(0)` until a key is pressed (RTL `0000:3387` and `0000:3141`, `kbhit` and `getch` by INFERRED name); returns the key; no callers | CONFIRMED code |
| `0x132C6` (`1329:0036`) | `hcInit` | stub, `atexit(1328:0016)` (an empty `retf` function, INFERRED), SetCallbacks, SetAckTimeout(90), GetPreviousProgram, SetNextProgram, sets DS:`189C` | CONFIRMED |
| `0x13369` | `hcIsInitialized` | returns DS:`189C` | CONFIRMED |
| `0x13375` (`1329:00E5`) | `hcPoll` | copy DS:`189D` (status from the INT 8 Service) to DS:`8700`, stamp tick, Poll, report errors | CONFIRMED |
| `0x13435` | `hcPollAndFlush` | `hcPoll` then Flush; no callers | CONFIRMED |
| `0x13446` (`1329:01B6`) | `hcFlush` | stamp tick, Flush | CONFIRMED |
| `0x13465` | `hcGetUserWord4` | returns shared-block word at +4, or 0; no callers | CONFIRMED |
| `0x1349A` | `hcGetUserSid` | returns shared-block word at +0, or 0 | CONFIRMED |
| `0x134CE` (`1329:023E`) | `hcGetSharedParams` | out: word +4, word +0, 11 bytes from +0x10, far pointer to +0x80, length minus `0x80` | CONFIRMED |
| `0x13565` (`1329:02D5`) | `hcSetSharedParams` | read the block, keep its first `0x80` bytes, append `len` bytes at +0x80, SetSharedData | CONFIRMED |
| `0x135D0`..`0x14013` | `hcSend*` | one builder per command, section 3.3 | CONFIRMED |
| `0x1392B` | `hcOnObjId` | receive command 8 | CONFIRMED |
| `0x13D91`, `0x13E7A` | `hcOnGroupAdd`, `hcOnGroupDelete` | receive commands 10, 11 | CONFIRMED |
| `0x140BF` (`1329:0E2F`) | `hcPump` | `hcPoll`; Receive; dispatch on byte 0 through a 13-entry table at `0x14293`; free; `hcFlush`; repeat while the argument is 1 | CONFIRMED |
| `0x142C7` (`1329:1037`) | `hcGetBackOn` | prompt `Get Back On? (Y/N): `; `N` calls SetNextProgram(NULL); then `exit(0)` | CONFIRMED |
| `0x14323` (`1329:1093`) | `hcFatalAt(file, line, fmt, ...)` | calls `gcLeaveGame`, prints `Fatal Error Encountered`, `File`, `Line`, `Message`, then `hcGetBackOn` | CONFIRMED |
| `0x14400` (`1329:1170`) | `hcFatal(fmt, ...)` | same without file and line | CONFIRMED |
| `0x14464` (`1329:11D4`) | `hcAlloc` | alloc callback: loads DS, `farmalloc(size)`, fatal `farmalloc() cannot alloc %d bytes, himem = %lu` on NULL | CONFIRMED |
| `0x144AE` (`1329:121E`) | `hcDeref` | deref callback: returns its argument (the handle is the far pointer) | CONFIRMED |
| `0x144C2` (`1329:1232`) | `hcFree` | free callback: `farfree` | CONFIRMED |
| `0x144DC` (`1329:124C`) | `hcTimerTick` | every sixth call: stamp tick, Service, remember a non-zero status | CONFIRMED |

The three callbacks reload DS (`push ds; mov ax,1AF6; mov ds,ax`), as TSNEXEC requires
(`int14h-api.md` section 4). `hcTimerTick` does not: it runs under the INT 8 handler, which has set DS already.

### 3.2 `gamecomm.c` (image `0x14510`..`0x159E0`)

| Image (seg:off) | Name | Role | Status |
|---|---|---|---|
| `0x1451E` (`1451:000E`) | `gcFindMember` | member record by SID | CONFIRMED |
| `0x1455D` | `gcFindPlayer` | player record (stride `0x4C`, DS:`720C`) by member | CONFIRMED |
| `0x145AE`, `0x145C1` | `gcMarkTime`, `gcTimedOut` | handshake timeout: more than `0x887` ticks since the mark (about 120 s at 18.2 Hz) | CONFIRMED |
| `0x145EA` | `gcIsLastHole` | end-of-round test | INFERRED |
| `0x1462B` (`1451:011B`) | `gcOnHostTime` | reads a 6-byte date and time from a host reply (section 6.2); never fed in GOLF | CONFIRMED code |
| `0x146A6` | `gcAddMember` | new member record (14 bytes), flags bit 1 | CONFIRMED |
| `0x14715` | `gcRemoveMember` | drop a member record | CONFIRMED |
| `0x1478D`, `0x147CD`, `0x1481C` | `gcOnGroupAddEvent`, `gcOnGroupMembers`, `gcOnGroupDeleteEvent` | group events, section 7.3 | CONFIRMED |
| `0x148BB` (`1451:03AB`) | `gcApplyDeparture` | prints `%s has left the game`, compacts the player tables, may print `Switching to Stroke Play`; called from the game loop | CONFIRMED |
| `0x14B8D` | `gcOnUnsolicited` | command 48 text, shown only after the join completed (DS:`1AB9`) | CONFIRMED |
| `0x14BAC` (`1451:069C`) | `gcDispatchSend` | table of 6 message types at `0x14C55`: `01`, `1C`, `CA`, `CB`, `CC`, `CD` | CONFIRMED |
| `0x14C6D` (`1451:075D`) | `gcSendToObject(to, from, handle, len)` | fills the 6-byte Send header, sends | CONFIRMED |
| `0x14CFD` (`1451:07ED`) | `gcMulticast(recips, n, from, handle, len)` | command 28 | CONFIRMED |
| `0x14E00`, `0x14E9B` | `gcSendShot`, `gcOnShot` | message `CB` | CONFIRMED layout; meaning INFERRED |
| `0x14FE9`, `0x150A5`, `0x15128` | `gcPumpPosition`, `gcSendPosition`, `gcOnPosition` | message `CC` | CONFIRMED layout; meaning INFERRED |
| `0x151B8`, `0x151FA`, `0x15280` | `gcSendQueuedChat`, `gcSendChat`, `gcOnChat` | message `CD` | CONFIRMED |
| `0x15387`, `0x15454` | `gcSendPlayerInfo`, `gcOnPlayerInfo` | message `CA` | CONFIRMED |
| `0x155C4` | `gcCheckPlayerCount` | too few players: print `Switching to Stroke Play` | CONFIRMED |
| `0x155F6` (`1451:10E6`) | `gcInit` | `hcGetSharedParams`, copy 14 parameter bytes, reset tables, allocate three queues | CONFIRMED |
| `0x1574E` (`1451:123E`) | `gcSaveResult` | `hcSetSharedParams(result, 2)`, free the queues; once | CONFIRMED |
| `0x157EC` (`1451:12DC`) | `gcJoinGame` | the join handshake, section 7 | CONFIRMED |
| `0x159B0` (`1451:14A0`) | `gcLeaveGame` | `gcSaveResult`; unless the link died, send GrpDel and ObjFree | CONFIRMED |

Callers of the three entry points: `gcInit` and `gcJoinGame` are called from overlay 1 (overlay offsets `0x1E43` and
`0x1081`), `gcLeaveGame` from the overlay area's first chunk (`0xA4`) and from `hcFatalAt`/`hcFatal`. The overlay
far-call segments are overlay-relative (`lcall 0108:12DC` for image `1451:12DC`), so these three matches are by
offset (CONFIRMED offsets; the segment mapping is INFERRED). The resident main loop (`19FC:0009`, image `0x9FC9`)
calls `hcInit`, then each frame `hcPump(1)`, `gcPumpPosition` and `gcSendQueuedChat`. CONFIRMED.

### 3.3 Send builders in `hostcomm.c`

Header word 2 is the target SID or 0. `ownSid` is the SID that `ObjID` assigned to GOLF's object.

| Image | Name | Bytes | Live | Command |
|---|---|---|---|---|
| `0x137A2` | `hcCreateObject` | `07 00 00 00 w cookie, b kind, b landType, w param, w size` (12) | yes | 7 joinNet |
| `0x13986` | `hcFreeObject` | `09 00 w ownSid` (4) | yes | 9 leaveNet |
| `0x13CF2` | `hcGroupAdd` | `0A 00 w group, w ownSid` (6) | yes | 10 add |
| `0x13DDF` | `hcGroupDelete` | `0B 00 w group, w ownSid` (6) | yes | 11 delete |
| `0x13EC8` | `hcGroupGetMembers` | `0C 00 w group, w ownSid` (6) | yes | 12 GrpMem |
| `0x14C6D` | `gcSendToObject` | `02 00 w to, w from, body...` (6 + n) | yes | 2 Send |
| `0x14CFD` | `gcMulticast` | `1C w from, w n, w recips[n], body...` (5 + 2n + body) | yes | 28 multicast |
| `0x135D0` | `hcSendCmd50` | `32 b ?, w, w` (6) | no | 50, not in the LSCI catalogue |
| `0x1366D` | `hcSendObjExists` | `29 00 00 00 w, b, w` (9) | no | 41 `ObjExists` |
| `0x1371B` | `hcSendGetHostTime` | `24 02` (2) | no | 36 `HostInfo`, sub 2 |
| `0x13A85` | `hcSendSetStr` | `0E 00 w sid, w sid, w prop, s value` (9 + n) | no | 14 setStr |
| `0x13B67` | `hcSendSetInt` | `0D 00 w sid, w sid, {w, w}...` (6 + 4n) | no | 13 setInt |
| `0x13C3C` | `hcSendCmd17` | `11 00 w, w, w` (8) | no | 17, not in the LSCI catalogue |
| `0x13F67` | `hcSendLock` | `04 00 w, w` (6) | no | 4 lock |
| `0x14013` | `hcSendUnlock` | `06 00 w, w` (6) | no | 6 unlock |

Every builder is `alloc(len)` (`hcAlloc`), fill, Send, fatal on a result other than 1, `hcFlush`, `hcPoll`. CONFIRMED.

### 3.4 `edit.c`, `dialog.c` and the chat module

| Image | Segment | Content | Status |
|---|---|---|---|
| `0x12400`..`0x1266A` | `1232` | chat line queue: 22-byte line records in two queues (incoming DS:`86F6`, outgoing DS:`86F2`), `%s: %s` display with the sender's name and colours. No source string; flags at DS:`1828`, `1829` | CONFIRMED structure; name INFERRED |
| `0x12670`..`0x129FF` | `1267`, `edit.c` | single-line editor: allocator `0x12670` (fatal `cannot alloc %d bytes`), free `0x126D8`, draw `0x12717`, `0x1273A`, key handler `0x12759` (backspace 8, enter 0D, space 20) | CONFIRMED |
| `0x12A00`..`0x12C3A` | `12A0`, `dialog.c` | dialog box: 22-byte record allocated at `0x12A00` (fatal `cannot alloc %d bytes`), frame drawing, a countdown field that closes it | CONFIRMED |

None of these modules touches the export table. The text a player types is queued in the chat module and sent by
`gcSendQueuedChat` as message `CD`.

## 4. Runtime sequence

### 4.1 Start and tick

| Step | Where | Evidence |
|---|---|---|
| 1 | `19FC:0009` calls `1655:006D` (image `0x165BD`): saves the INT 8 vector to DS:`92E6`, zeroes the 32-bit tick DS:`92EA`, installs the handler `1655:0006` | CONFIRMED |
| 2 | `hcInit` (section 3.1) | CONFIRMED |
| 3 | INT 8 handler: DS = `1AF6`; add 1 to the tick; `hcTimerTick`; chain to the saved vector with `pushf` | CONFIRMED (`0x16556`) |
| 4 | `hcTimerTick`: DS:`18B0` counts 5 down to 0; at 0 it reloads 5, writes the tick through the SetAckTimeout pointer, calls Service | CONFIRMED: **Service runs every 6th timer interrupt, from interrupt context** |

So the reentrancy lock of `int14h-api.md` section 5 is exercised: Service from the handler can interrupt a Poll,
Send or Flush in the main thread, and then returns 0. GOLF's tick counts its own IRQ 0 interrupts, not the BIOS
tick (`int14h-api.md` section 5 says the DOS games use the BIOS tick; that holds for CGENN). Code that also programs the DMA controller, probably the sound driver (INFERRED),
reprograms PIT channel 0 (image `0xECBC` writes divisor `0x2000`); whether the counter then runs
faster than 18.2 Hz depends on handler order and is not resolved (INFERRED risk).

### 4.2 Pump

`hcPump(mode)`: `hcPoll`; Receive; if empty, `hcFlush` and return; else dispatch on byte 0, free the message,
`hcFlush`, and loop only when `mode` is 1. Callers (CONFIRMED by byte search, push immediate before the call):
join steps 1 to 4 (image `0x15831`, `0x15884`, `0x158D2`, `0x15904`) pass 0 (one message per call); join step 5
(`0x15932`) and the main loop (`0xA002`) pass 1. Messages with an unknown command byte are freed and ignored. CONFIRMED.

### 4.3 Errors

| Status (Poll or Service) | Message at DS | Status |
|---|---|---|
| 1 | `18FC` The connection to ImagiNation has been lost. | CONFIRMED |
| 2 | `1929` The phone line is too noisy to continue communications. Please try calling again. | CONFIRMED |
| 3 | `197B` ImagiNation is not acknowledging sends from your computer. Please try calling again. | CONFIRMED |
| other | `19D0` `INN Error %d` | CONFIRMED |

Any non-zero status sets DS:`18A3` (link dead, so `gcLeaveGame` sends no GrpDel/ObjFree), clears DS:`189D`, calls
`hcFatal`, which runs `gcLeaveGame` (it skips GrpDel and ObjFree because DS:`18A3` is set), and asks `Get Back On? (Y/N)`. `Y` exits with the next program still set to
the previous one (back to the land); `N` cancels it. CONFIRMED.

## 5. Shared block

Read once by `gcInit` through `hcGetSharedParams`; word +0 is read again by `gcSendPlayerInfo`; written once by
`gcSaveResult` on exit.

| Offset | Size | GOLF use | Status |
|---|---|---|---|
| +0 | word | user SID. Sent in message `CA` at +0x1E, stored in the player record at +0x42 | CONFIRMED use |
| +4 | word | **SID of the game group** that the player joins (DS:`1ACF`); `gcJoinGame` sends it in GrpAdd and GrpMem | CONFIRMED use |
| +0x10 | 11 bytes | user name (10 characters and NUL); copied to player 0's name (DS:`720C`) | CONFIRMED |
| +0x80 | 14 bytes | launch parameters, copied to DS:`8B48`, section 5.1 | CONFIRMED |
| written back | 0x80 + 2 | first `0x80` bytes unchanged, then one word at +0x80 (section 5.2); only if the block still exists and `len` is at most `0x80` | CONFIRMED |

`messages.md` section 5.3 gives +4 as "SID of the game object to free". GOLF reads the same word as the group it
joins, which fits the LSCI waiting-room group being passed down and freed by the land afterwards (INFERRED).

### 5.1 Launch parameters at +0x80

| Byte | Stored at | Use | Status |
|---|---|---|---|
| 0..3 | DS:`8B48`..`8B4B` | not read by the library | CONFIRMED copy |
| 4 | DS:`8B4C` | **expected number of players**, DS:`1ABD` | CONFIRMED |
| 5 | DS:`8B4D` | holes selection: 0 = all 18 (end hole `0x12`), 1 = front 9 (`9`), 2 = back 9 | INFERRED from `gcIsLastHole` |
| 6 | DS:`8B4E` | game option, tested with the result word | INFERRED |
| 7 | DS:`8B4F` | game type, 0 = Stroke Play; the 13 format names are at DS:`1D59D` | INFERRED |
| 8..11 | DS:`8B50`..`8B53` | four bytes copied to player 0 and sent in `CA` +0x12..+0x15 | CONFIRMED copy |
| 12, 13 | DS:`8B54`, `8B55` | two bytes sent in `CA` +0x1C, +0x1D | CONFIRMED copy |

### 5.2 Result word

`gcSaveResult` writes `0x7FFF` unless game type, option and holes selection are all 0 and the last hole was played
(DS:`4A66` = `0x12`, DS:`4A64` = `0x11`); then it writes DS:`6E95` + `0x48`. CONFIRMED (image `0x1575C`..`0x15787`);
that this is the round's stroke total, with 72 = par, is INFERRED. Whether the land reads it is unknown.

## 6. Host messages

Command numbers are the LSCI numbers (`messages.md` section 3). C-to-H is client to host.

### 6.1 Sent to the host (live)

| Cmd | Name | Layout | Meaning | Status |
|---|---|---|---|---|
| 7 | joinNet | `07 00 00 00 w cookie, b 01, b 66, w FFFF, w 0001` | create a networked object: kind 1 (object), landType `0x66` (102), param `-1`, size 1. `cookie` is a local counter at DS:`189F`, first value 0 | CONFIRMED (`0x13872`) |
| 10 | GrpAdd | `0A 00 w group, w ownSid` | join the group | CONFIRMED |
| 12 | GrpMem | `0C 00 w group, w ownSid` | ask for the member list | CONFIRMED |
| 11 | GrpDel | `0B 00 w group, w ownSid` | leave the group (exit, when the link is alive) | CONFIRMED |
| 9 | leaveNet | `09 00 w ownSid` (4 bytes; `messages.md` section 3 shows 6 for LSCI, so accept both lengths) | free the object (exit) | CONFIRMED |
| 2 | Send | `02 00 w toSid, w fromSid, body` | routed message; body byte 0 is the type (section 7) | CONFIRMED |
| 28 | multicast | `1C w fromSid, w n, w recips[n], body` | one body to several SIDs | CONFIRMED |

Dead sends (no caller): 4, 6, 13, 14, 17, `24 02`, 41, 50 (section 3.3).

### 6.2 Received from the host

| Cmd | Name | Layout | GOLF action | Status |
|---|---|---|---|---|
| 8 | `ObjID` | `08 ? w cookie@2, w ?@4, w sid@6` | the object with that cookie gets `sid` (+4), flags bit 0 (and `w@4` goes to +10) | CONFIRMED |
| 10 | GrpAdd event | `0A ? w group@2, w who@4` | if `who` is an own object, set flags bit 1; then add `who` as a member when `group` is DS:`1ACF` and `who` is not the player; count it once the member list has arrived | CONFIRMED |
| 11 | GrpDel event | `0B ? w group@2, w who@4` | clear flags bit 1 of own objects; for the group, once joined, queue a departure for another player | CONFIRMED |
| 12 | GrpMem | `0C ? w group@2, w ?@4, w member[n]@6`, n = (len - 6) / 2 | for the group: member count := n, mark "list received", add unknown members | CONFIRMED |
| 2 | Send | `02 00 w to, w from, body` | dispatch on byte 6, section 7 | CONFIRMED |
| 48 (`30`) | Unsolicited | `30 ? text@2` | popup text, only once joined | CONFIRMED |
| 0, 1 | Ack, Nak | `b whichCmd@4` | only `whichCmd` 4 (lock, never sent) is acted on: Ack sets DS:`1AD6` = 2, Nak sets it to 1 (`1451:018C`, `1451:0182`); Nak 6 does nothing. DS:`1AD6` has no reader in the resident image (overlay readers not excluded) | CONFIRMED handlers |
| 9, 13, 14, 41 (`29`) | | | handlers are empty | CONFIRMED |
| 36 (`24`) | HostInfo | sub 0: empty handler; sub 2: `24 02 b year-1900, b month-1, b day, b t1 (low 7 bits), b t2, b t3` | sub 2 sets the date and time globals at DS:`8B56`..`8B5D`; never requested by GOLF | CONFIRMED code; time-of-day order INFERRED |

## 7. Game channel and handshake

### 7.1 Peer messages (body of a Send or multicast)

Byte 6 of a received Send (byte 9 of the sent multicast) is the type; GOLF treats it as a byte, so byte 7 is data,
not the high byte of a word type. Types `01` and `1C` have empty handlers (LSCI chat is ignored).

| Type | Total bytes | Layout (offsets in a received Send) | Meaning | Status |
|---|---|---|---|---|
| `CA` | 32 | `@7 s name (to @0x11), @0x12 b a, @0x13 b b, @0x14 b c, @0x15 b d, @0x16 a[4] e, @0x1A w seed, @0x1C b f, @0x1D b g, @0x1E w userSid` | player announcement. `seed` = low 16 bits of that player's `time(NULL)` (seconds since 1970: `getdate`/`gettime` via INT 21h `2A`/`2C`, then `dostounix`, image `0x2084`, `0x5288`; it is the word at player record `+0x44`); `userSid` = shared block word +0. Sent once, to the group | CONFIRMED layout; names INFERRED |
| `CB` | 39 | `@7 w a, @9 w b, @B w c, @D w d, @F double x, @17 a[16] s` | a shot event; receiver splits it into a 6-byte record (`a`, `b`, sender) and a 30-byte record (`c`, `d`, `x`, `s`, sender) | CONFIRMED layout; meaning INFERRED |
| `CC` | 38 as multicast | `@7 w c, @9 w d, @B double x, @13 a[16] s` | periodic state update; the same 28 bytes as the second half of `CB` | CONFIRMED layout; meaning INFERRED |
| `CD` | 8 + strlen | `@7 s text` | chat text | CONFIRMED |

Sender and flow:

| Type | Sender | Target | Pacing |
|---|---|---|---|
| `CA` | `gcSendPlayerInfo` via `gcSendToObject(group, ownSid)` | the group | once, after the member count is reached |
| `CB` | `gcSendShot` (called from the resident game state function at image `0x8CD9`) via `gcSendToObject(group, ownSid)` | the group | at a shot |
| `CC` | `gcSendPosition` via `gcMulticast` with recipients `{group, ownSid}` | the group **and the sender** | at most once per 37 ticks (about 2 s), only when the state changed (compared with the last 28 bytes) and only after the previous one **came back** (below); never when the expected player count is 1 or three game-state words (DS:`6E52`, `7F54`, `4A6C`) are non-zero (meaning INFERRED) |
| `CD` | `gcSendChat` via `gcSendToObject(group, ownSid)` | the group | when the player sends a line |

**Echo rule (CONFIRMED).** `gcOnPosition` sets DS:`1AD7` when the `CC` it receives is from its own SID; `gcPumpPosition`
sends the next `CC` only while DS:`1AD7` is set, and clears it on each send. DS:`1AD7` starts at 1. A host that does
not deliver a multicast copy to the sender stops the position updates after the first one. Also, `gcOnPlayerInfo`
ignores a `CA` from its own SID and every `CA` after the join, so a Send to the group may or may not echo to the sender (INFERRED: the explicit own SID in the
multicast suggests it does not).

### 7.2 Handshake (`gcJoinGame`, image `0x157EC`)

Each wait pumps once per turn (one message in steps 1 to 4, a drain in step 5) and fails after more than `0x887` ticks since the mark (`gcTimedOut`).

| # | Step | Waits for | On timeout | Status |
|---|---|---|---|---|
| 1 | `hcCreateObject(0x66, 1, 0xFFFF, 1)`, command 7 | `ObjID` sets flags bit 0 | fatal `create object timeout` (line `0x715`) | CONFIRMED |
| 2 | `hcGroupAdd(obj, group)`, command 10 | a GrpAdd event whose `who` is the own SID: flags bit 1 | fatal `join group timeout` (`0x733`) | CONFIRMED |
| 3 | `hcGroupGetMembers`, command 12 | a GrpMem reply | fatal `get group mem timeout` (`0x74E`) | CONFIRMED |
| 4 | wait until the member count (DS:`1ABB`) reaches the expected players (DS:`1ABD`), pumping GrpAdd events | count | not fatal: expected := count | CONFIRMED |
| 5 | `gcSendPlayerInfo`, message `CA` | `CA` from every other player (DS:`1ABC` starts at 1) | not fatal: expected := received + 1 | CONFIRMED |
| 6 | player count := expected; `gcCheckPlayerCount` may print `Switching to Stroke Play`; **`srand(seed of the member with the lowest SID)`**; set the joined flag DS:`1AB9` | | | CONFIRMED |

Step 6 synchronises the random number generator of all players: the lowest SID's `seed` (low 16 bits of its Unix time, one-second granularity) seeds every
player's `rand`. The wait is 2184 ticks = 120 s at 18.2 Hz (`0x888`).

### 7.3 Group events

| Event | Action | Status |
|---|---|---|
| GrpAdd (10) for the own group, member not known | add a member record; count it if the list was already received | CONFIRMED |
| GrpMem (12) | count := n; add every unknown SID | CONFIRMED |
| GrpDel (11) after the join | queue the SID in DS:`1ACB`; the game loop calls `gcApplyDeparture`: remove the player, print `%s has left the game`, fall back to Stroke Play when too few remain | CONFIRMED |

## 8. Overlays

| Item | Value | Status |
|---|---|---|
| Overlay area | FBOV at file `0x25EB0`; 9 stub segments at image `0x1ACF0`..`0x1AF30` (`INT 3Fh` stubs, 32-byte headers); the overlay data starts at file `0x25EC0` | CONFIRMED |
| Overlay file offsets (from the area start) | `0x0210`, `0x22B0`, `0x24D0`, `0x6290`, `0x75E0`, `0x8340`, `0x96E0`, `0xA440`, `0xB7E0`, end `0xECE0` | CONFIRMED (stub headers) |
| TSN use | overlays contain no table access. They call `gcInit` (overlay 1 `+0x1E43`), `gcJoinGame` (`+0x1081`) and `gcLeaveGame` (first chunk `+0xA4`) | CONFIRMED offsets; segment mapping INFERRED |
| Other library calls from overlays | none with a `hostcomm.c` or `gamecomm.c` offset | CONFIRMED (scan) |

Overlay code was not decompiled; it was scanned as raw bytes.

## 9. What the server must do for GOLF

The list assumes the INT 14h transport (`int14h-transport.md`) or the link layer carries these messages unchanged.

| # | Responsibility | Why | Status |
|---|---|---|---|
| 1 | Keep the session and the identity across the land-to-GOLF program switch; GOLF sends no Connect, Login or SwitchHost | `hcInit` only reads the block | CONFIRMED |
| 2 | Answer joinNet (kind 1, landType `0x66`) with `ObjID`: echo `cookie` at +2, new SID at +6, within 120 s | step 1; timeout is fatal | CONFIRMED |
| 3 | Treat the group SID from the shared block (+4) as an existing group; on GrpAdd add the member and send a GrpAdd event (`group`, `who`) to **every member including the joiner**, within 120 s | step 2 needs the joiner's own event | CONFIRMED |
| 4 | Answer GrpMem with the current member SIDs from +6 | step 3; the answer must come within 120 s | CONFIRMED |
| 5 | Deliver later GrpAdd events from other players | step 4 waits for the expected count | CONFIRMED |
| 6 | Relay Send to a SID or group SID unchanged (bytes 6 onward), setting `to` and `from`; never interpret game state | all four peer types | CONFIRMED relay; "never interprets" INFERRED |
| 7 | Split multicast `1C` into one Send per recipient SID (group SID expands to its members) and **deliver the sender's own copy** when its SID is listed | echo rule, section 7.1 | CONFIRMED |
| 8 | Broadcast GrpDel to the remaining members when a player leaves (cmd 11, or the object dies with the connection) | departures, section 7.3 | CONFIRMED handler |
| 9 | Accept GrpDel (11) and leaveNet (9) at exit without a reply; tolerate that the land frees the same group afterwards | exit | CONFIRMED sends; INFERRED tolerance |
| 10 | Accept every Send (return 1): a Send that fails is fatal in GOLF. Messages are at most 39 bytes | `send failure %d` | CONFIRMED |
| 11 | Report a dead session with Poll status 1, not by silence | fatal path | CONFIRMED |
| 12 | Optional operator text: command 48 is shown | `gcOnUnsolicited` | CONFIRMED |
| 13 | Host time (`24 02`) is not needed by GOLF | dead sender | CONFIRMED |
| 14 | Nothing to store: no accounts, scores or rankings are sent to the host by GOLF | no such message | CONFIRMED |

Per-round traffic is low: one `CA` per player, a `CB` per shot, a `CC` at most every 2 s per player, and chat lines.

## 10. Reuse notes for the other DOS clients

| Check | Pattern |
|---|---|
| Table call | `C4 1E pp pp 26 FF 5F NN` (`les bx,[p]`, `lcall es:[bx+NN]`) |
| Stub | `A1 pp pp 0B 06 qq qq 74 01 CB CD 14 A3 pp pp 89 16 qq qq CB` with `qq = pp + 2` |
| Callback prologue | `55 8B EC 56 1E B8 ss ss 8E D8` (reload DS) |
| Init order | `xor dx,dx`; stub; check segment; SetCallbacks; SetAckTimeout(90); GetPreviousProgram; SetNextProgram |
| Send wrapper | `alloc(len)`, deref, fill, Send, `!= 1` fatal (prints the command byte), Flush, Poll |
| Pump | Poll, Receive, 13-entry command table in the code segment, free, Flush |

The 13 receive commands in the table: `00 01 02 08 09 0A 0B 0C 0D 0E 24 29 30`. Whether the other games keep that
table is for their censuses to confirm.

## 11. Open questions

- Does the host echo a Send to a group SID back to the sender? GOLF drops its own `CA` and relies on an explicit
  own SID in the `CC` multicast.
- What the land does with the result word at +0x80, and whether it expects the +4 word to be left alone.
- Meaning of launch bytes 0..3, 5, 6, 7 and of the `CA`/`CB`/`CC` fields beyond the structure above; the game-state
  variables behind DS:`4A58`, `4A5A`, `4A6A`, `75E8`, `7530` were not traced.
- Why joinNet carries landType `0x66`, and what command 17 and command 50 do (both are dead in GOLF).
- Whether the PIT reprogramming in the sound code speeds up the tick counter, which would shorten the 90-tick
  acknowledgement timeout. Channel 0 is written at several places (mode `0x36`; divisors `0x2000` at image
  `0xECBC`, `0x54` at `0xED72` and `0xEDA6`, `0xA8` at `0xEDC9`, a variable one at `0xFB82`), so it is not
  one fixed rate; handler order was not traced.
- Overlay far-call segment values are overlay-relative; only offsets were matched, so a TSN call from an overlay
  through another route is excluded only by the byte searches in sections 1 and 8.

## 12. Verification

- The 30 `les bx,[1898]` sites and their offsets were found by byte search of the file, then cross-checked against
  the Ghidra decompilation (33 mentions of `DAT_2af6_1898`: 30 sites, the stub's two, one compare).
- Each live Send builder was read in capstone disassembly for its byte offsets; the dispatch tables were read as
  words from the image (`0x14293` receive commands and handlers, `0x14C55` peer types).
- Overlay structure was read from the stub headers; no `CD 14`, `98 18` or `9A 18` byte pair occurs in the overlay
  data.

### 12.1 Independent re-verification (adversarial)

Own Ghidra project `work/ghidra/verify-census-golf/` (decompilation in `work/decomp/verify-golf/`) and capstone.

| Check | Result |
|---|---|
| Whole-file byte search `C4 1E 98 18`, `98 18`, `9A 18`, `CD 14` | 30 `les bx,[1898]` sites, all `lcall es:[bx+N]`; one `INT 14h` (the stub, file `0x14C8C`); the stub's 2 plus 30 hits make the 32 `98 18` hits; `[189A]` read only by the stub and the `cmp` in `hcInit`. No other stub copy, no pointer copy, nothing in the overlay area, no `AX=3514h` vector fetch. CONFIRMED |
| Export index per site (`N / 4`) | matches the table in section 2 for all 30 sites (`+3C` 15, `+18` 6, `+2C` 11, `+24` 9, `+20` 8, `+38` 14, `+04` 1, `+08` 2, `+10` 4, `+14` 5, `+28` 10). CONFIRMED |
| Dead builders and `hcGetUserWord4`, `hcPollAndFlush`, `hcWaitKey` | no near call, far call (any segment value, resident or overlay area) or far pointer to their offsets. The two `9A F5 07` hits have segment `0x0EB0`, not `0x1329`. CONFIRMED |
| Live Send builders | `gcSendToObject` has 3 callers (`CB`, `CD`, `CA`), `gcMulticast` 1 (`CC`); `gcSendShot` is called from `0x8CD9`, `gcSendPlayerInfo` from `gcJoinGame`. CONFIRMED |
| Message layouts | byte offsets re-read from disassembly for all 15 builders, the `CA`, `CB`, `CC`, `CD` bodies and the receive handlers (table `0x14293` words `00 01 02 08 09 0A 0B 0C 0D 0E 24 29 30`, peer table `0x14C55` words `01 1C CA CB CC CD`). Agree with sections 3.3, 6 and 7 |
| Corrections made | (1) join step 5 pumps with mode 1, not 0; (2) the `CA` seed is the low 16 bits of Unix `time(NULL)`, not a DOS time; (3) Ack and Nak set different values in DS:`1AD6`; (4) `CA` offsets written in hex; (5) `CC` suppression conditions added; (6) the PIT note lists all channel-0 writes; (7) leaveNet length differs from LSCI |
| Not reproduced | overlay-side callers of `gcInit`, `gcJoinGame`, `gcLeaveGame` use far-call segment `0x108`, which is not the resident segment `0x1451`; offsets match exactly (`0x10E6`, `0x12DC`, `0x14A0`), the segment mapping stays INFERRED |

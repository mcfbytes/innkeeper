# INT 14h census: LSCITV

Every use of the TSNEXEC INT 14h interface by the LSCI interpreter, at export level. LSCITV is the
program behind the hub, SierraLand, CasinoLand and every other SCI land. The export table is in
`docs/protocol/int14h-api.md`; the script-side kernel is in `docs/protocol/ktsn.md`; the messages are in
`docs/protocol/messages.md` and are only referenced here.

Binary: `work/exe/LSCITV_inn_cd.EXE` (INN Feb-1994, LZEXE-unpacked). Evidence: capstone
(`tools/int14h_census.py`), Ghidra 12.1.4 headless decompilation (`work/decomp/lsci/`, not committed),
and `tools/ktsn_callsites.py` for script sites.

## 0. Address convention

- `seg:off` is relative to the unpacked load image. MZ header `0x30E0`, so **file = `0x30E0` + seg x 16 + off**.
  Ghidra loads at `1000:0000` (add `0x1000` to every segment there).
- DGROUP is `2074`. `p` below means the cached table pointer `DGROUP:1EC4` (far, file `0x256E4`).
- Export N = entry N of the 17-entry table, table offset `4N`. Sub-op N of the script kernel `TSN` calls export N.
- CONFIRMED means read in code or data; INFERRED means concluded from behaviour.

## 1. Summary

| Fact | Value | Status |
|---|---|---|
| `CD 14` sites in LSCITV | 1: the lookup stub | CONFIRMED |
| Functions that read `p` | 5: `16B1:0004`, `0358`, `031C`, `03F1`, `042E`; plus the stub `178B:0012` | CONFIRMED |
| Calls through the table | 19 sites in those 5 functions, all in segment `16B1` | CONFIRMED |
| Exports used | all 17; GetStatus and SetCallbacks twice, the other 15 once | CONFIRMED |
| Call sites outside `16B1` | none: nothing but `16B1` and the stub reads `p` (byte scan of the whole image for displacement `1EC4`/`1EC6`, then disassembly of every hit). `178B` also holds the Borland long-arithmetic helpers (`178B:0027` onward, about 50 callers program-wide) that never touch `p` | CONFIRMED |
| Same shape in other builds | Dec-93 and TSN 2.1: the same 19 sites, shifted addresses (section 8) | CONFIRMED |
| Other binaries | no other program in the LSCI set calls INT 14h (section 9) | CONFIRMED for every EXE and driver in `work/sets` (packed ones unpacked) and the installed DLLs; INFERRED for the TSN basic container and the SZDD DLLs |

## 2. The lookup stub and the cached pointer

| Item | Address (file) | Evidence | Status |
|---|---|---|---|
| Stub | `178B:0012` (`0x1A9A2`), 20 bytes | `mov ax,[1EC4]; or ax,[1EC6]; jz +1; retf; int 14h; mov [1EC4],ax; mov [1EC6],dx; retf`; Ghidra: `if (p != 0) return; p = swi(0x14)()` | CONFIRMED |
| The INT 14h instruction | `178B:001C` (`0x1A9AC`) | only `CD 14` in the image's code | CONFIRMED |
| Cached pointer `p` | `DGROUP:1EC4` / `1EC6` | read only by segments `16B1` and `178B` | CONFIRMED |
| Only caller of the stub | `16B1:0020` (`0x19C10`), `lcall 178B:0012` inside the `TSN` handler | the only other `lcall 178B:xxxx` in `16B1` is `0026`, a bare `retf` at shutdown (`16B1:0352`); the `178B` calls elsewhere are long-arithmetic helpers | CONFIRMED |
| Guard | `16B1:000A`..`001E` tests `p`; the stub runs only while it is 0 | | CONFIRMED |
| No "executive missing" check | nothing tests `DX` or the table segment | a BIOS INT 14h status would become `p` | CONFIRMED (no check); crash INFERRED |

LSCITV reaches `p` three ways. This matters for any scanner:

| Form | Where | Example |
|---|---|---|
| `les bx,[bp-10h]; les bx,es:[bx]; lcall es:[bx+N]` | `TSN` handler `16B1:0004`: the prelude stores `&p` in `[bp-10h]` (`mov bx,1EC4; mov es,[33D0]`) | `16B1:00A5`..`00AB` |
| `mov es,[33D0]; les bx,es:[1EC4]; lcall es:[bx+N]` | helpers: pump, shutdown, timer server, Send | `16B1:0361`..`0366` |
| `les si,es:[bx]; lcall es:[si+N]` | init, SetAckTimeout | `16B1:0038`..`003B` |

The DOS games use only the second form, so `tools/int14h_census.py` handles both and labels them
`direct` and `pointer-to-pointer`.

## 3. Every call through the table

All sites are in `16B1`. "Case" is the `TSN` jump-table entry (`16B1:0083`). Arguments are far cdecl, pushed right to left.

| # | Site (file) | Export | In function | Arguments pushed | Result use | Status |
|---|---|---|---|---|---|---|
| 1 | `16B1:003B` (`0x19C2B`) | 6 SetAckTimeout | `TSN` first-use init `0004` | `300` (`push 12Ch`); caller pops 2 bytes | `DX:AX` stored at `DGROUP:0430`/`0432`: far pointer to TSNEXEC's tick counter | CONFIRMED |
| 2 | `16B1:0065` (`0x19C55`) | 15 SetCallbacks | init `0004` | alloc `16B1:0B8E`, deref `16B1:0BB8`, free `16B1:0BE5` (3 far pointers, 12 bytes) | none | CONFIRMED |
| 3 | `16B1:00AB` (`0x19C9B`) | 0 GetStatus | case 0 `00A5` | none | `AX` written to the accumulator `DGROUP:075A` | CONFIRMED |
| 4 | `16B1:00BC` (`0x19CAC`) | 1 GetSharedData | case 1 `00B1` | far pointer to a local far slot (`ss:[bp-0Ch]`) | length in `AX`; 0 gives accumulator 0, else a new byte array (`ArrayNew(len,2)`, `0BF4:017D`) filled with `len` bytes from `*slot` through `034F:000E` | CONFIRMED |
| 5 | `16B1:0155` (`0x19D45`) | 2 SetSharedData | case 2 `011E` | far source = element 0 of the byte array (`ArrayData`, `0BF4:0629`), `len` | none (accumulator kept). Array or length 0 gives `(NULL, 0)` | CONFIRMED |
| 6 | `16B1:016D` (`0x19D5D`) | 0 GetStatus | case 3 `0167` | none | `test ah,0FFh`: nonzero means already connected, so the case returns 1 without dialling | CONFIRMED |
| 7 | `16B1:0198` (`0x19D88`) | 3 Connect | case 3 | far `char *` = `StrPtrGet(argv[2])` (`02C4:0312`) | `0` then `InstallServer(03F1, 20)`; nonzero then `ErrorEvent(0, code)`. Either way the poll hook `0358` is added. Accumulator = 1 on success, else 0 | CONFIRMED |
| 8 | `16B1:01F8` (`0x19DE8`) | 7 Disconnect | case 7 `01D8` | none | none; before it: `DisposeServer(03F1)` and removal of the poll hook | CONFIRMED |
| 9 | `16B1:0229` (`0x19E19`) | 9 SetNextProgram | case 9 `0206` | far `char *` = `StrPtrGet(argv[2])`, or `0:0` when `argv[2]` is 0 | discarded (accumulator kept) | CONFIRMED |
| 10 | `16B1:0238` (`0x19E28`) | 11 GetPreviousProgram | case 11 `0232` | none | NULL gives accumulator 0; else `StrNew(strlen+1)` plus copy | CONFIRMED |
| 11 | `16B1:0289` (`0x19E79`) | 12 IsTransmitIdle | case 12 `0283` | none | `AX` written to the accumulator | CONFIRMED |
| 12 | `16B1:02A2` (`0x19E92`) | 13 SwitchHost | case 13 `0290` | far `char *` = `StrPtrGet(argv[2])` | 0 then `DisposeServer` plus `InstallServer(03F1, 20)`; nonzero then `ErrorEvent(0, code)`. The poll hook is removed and re-added. Accumulator = 1 when the result was 0, else 0 | CONFIRMED |
| 13 | `16B1:0300` (`0x19EF0`) | 14 Flush | case 14 `02FA` | none | none (accumulator kept) | CONFIRMED |
| 14 | `16B1:030C` (`0x19EFC`) | 16 GetLineRate | case 16 `0306` | none | `AX` written to the accumulator | CONFIRMED |
| 15 | `16B1:034B` (`0x19F3B`) | 15 SetCallbacks | `onexit` handler `031C` | `(NULL, NULL, NULL)` | none | CONFIRMED |
| 16 | `16B1:0366` (`0x19F56`) | 8 Poll | event pump `0358` | none | nonzero then `ErrorEvent(0, code)` (event `0x800`) | CONFIRMED |
| 17 | `16B1:03A7` (`0x19F97`) | 5 Receive | event pump `0358` | far pointer to a local handle slot (`ss:[bp-4]`) | length in `AX`; nonzero then post event `0x400` with `message` = low word of the handle, `modifiers` = length | CONFIRMED |
| 18 | `16B1:0418` (`0x1A008`) | 10 Service | timer server `03F1` | none | nonzero then `ErrorEvent(0, code)` | CONFIRMED |
| 19 | `16B1:0786` (`0x1A376`) | 4 Send | `Send` helper `042E` | far handle `0:h` (handle high word pushed as 0), body length | result returned to the accumulator by case 4 (1 queued, 0 failed) | CONFIRMED |

Cases 5, 6, 10 and 15 of the jump table are bare returns, so exports 5, 6, 10 and 15 are reachable
only through rows 1, 2, 15, 17 and 18. CONFIRMED (jump-table words `00A5 00B1 011E 0167 01C2 0318 0318 01D8 01FF 0206 0318 0232 0283 0290 02FA 0318 0306`).

## 4. Per export

"Script sites" counts `callk TSN` sites with that sub-op in the Feb-94 hub, SL and LL scripts (`tools/ktsn_callsites.py work/res/inn_feb94`).
Case 4 and case 8 reach their exports through helpers, so the binary site is the helper.

| # | Export | Binary sites | Wrapper | Script sites | How the scripts use it |
|---|---|---|---|---|---|
| 0 | GetStatus | 2 | case 0, case 3 | 55 | `AL` = driver id: `script.101 proc_80` and `hub/script.012 MakeConnection::doit` switch on `TSN(0) & 255` (3 = modem dial string, 2 = Novell host name, 1 = serial with `"foo"`, else "Comm driver problems"); `AH` = connection byte, 0x80 while connected. 30 sites read `TSN(0) >> 8` (17 store it in `global85`, the script's connected flag), 25 read `TSN(0) & 255` |
| 1 | GetSharedData | 1 | case 1 | 3 | `script.101` `proc_125` on arrival (section 6) |
| 2 | SetSharedData | 1 | case 2 | 9 | `script.101`: one write on leaving, two clears `TSN(2, 0, 0)` after reading |
| 3 | Connect | 1 | case 3 | 10 | `script.101 proc_80` (3 per land, one per `AL` branch, each after `TSN(7)`) and `hub/script.012 MakeConnection::doit` (1; its argument is set only in the `AL` 3 and 2 branches): modem string, Novell host name, or the literal `"foo"` (string at `script.101` `0x1C04`, call at `proc_80 +0x0254`) for serial (`messages.md` section 7.1) |
| 4 | Send | 1 | `042E`, case 4 | 381 | formatted send, `b w s a +` (`ktsn.md` section 3.1); the first value is the message code |
| 5 | Receive | 1 | pump `0358` | 0 | never called by a script; the pump posts `0x400` events |
| 6 | SetAckTimeout | 1 | init in `0004` | 0 | once per run, 300 ticks |
| 7 | Disconnect | 1 | case 7 | 33 | start of `MakeConnection::doit`, dial retries, sign-off, error paths |
| 8 | Poll | 1 | pump `0358` | 59 | the pump runs from `TSN(8)` and from every `GetEvent` (section 7) |
| 9 | SetNextProgram | 1 | case 9 | 6 | `script.101` only: 2 per land (the land name, or `"Default"`) |
| 10 | Service | 1 | timer server `03F1` | 0 | IRQ0 timer server, every 20 ticks |
| 11 | GetPreviousProgram | 1 | case 11 | 0 | no script uses it, so LSCITV never "returns to the caller" |
| 12 | IsTransmitIdle | 1 | case 12 | 22 | drain loops (section 7.3) |
| 13 | SwitchHost | 1 | case 13 | 6 | `script.101` `attachScript` state 3, 2 per land |
| 14 | Flush | 1 | case 14 | 259 | after nearly every send and every `TSN(8)` |
| 15 | SetCallbacks | 2 | init, `onexit` | 0 | alloc/deref/free for byte arrays |
| 16 | GetLineRate | 1 | case 16 | 5 | `hub/script.097` passes it to a display object when `TSN(0) & 255 == 3`, else `-1` |

Sub-op totals: 55 + 3 + 9 + 10 + 381 + 33 + 59 + 6 + 22 + 6 + 259 + 5 = 848 sites, 0 unresolved. CONFIRMED (re-run of `tools/ktsn_callsites.py`).

## 5. Buffers and callbacks

| Buffer | Layout | Owner | Status |
|---|---|---|---|
| Message handle | 16-bit handle of an interpreter byte array (`ArrayNew(n, 2)`); the far "handle" TSNEXEC holds is `0000:h`; `alloc` returns `DX=0` | alloc: TSNEXEC calls `16B1:0B8E`; deref `16B1:0BB8` = `ArrayData(BlkGetPtrP(h), 0)`; free `16B1:0BE5` = `BlkFree(h)` | CONFIRMED |
| Send body | bytes produced by the format string; `0130:012C(h, 1)` sets flag `0x02` on the array (`18E5:04E5`) | after Send the executive frees it (Poll, deferred FIFO of 99) | CONFIRMED; purpose of the flag (keep the block from being reclaimed) INFERRED |
| Received body | byte array allocated by TSNEXEC through `alloc` (flag `0x02` set there too); handle in event `message`, length in `modifiers` | the script owns it after `GetEvent`; LSCITV keeps no reference | CONFIRMED (allocation); ownership INFERRED |
| Shared block | at most 256 bytes at `031D:013A`, length at `031D:023A`; GetSharedData copies it out, SetSharedData copies it in | TSNEXEC, survives program chaining | CONFIRMED |
| Tick counter | 32-bit at `031D:06EC`; LSCITV copies `TickCount` (`DGROUP:0B58`/`0B5A`, word-incremented by IRQ0) through `[DGROUP:0430]` | LSCITV writes, TSNEXEC reads | CONFIRMED |

## 6. Shared block use

TSNEXEC holds the block; nothing reaches the wire. LSCITV is the only LSCI-side user (`TSN(1)`, `TSN(2)`):

| Step | Site | What happens | Status |
|---|---|---|---|
| Write on leaving a land | `script.101 export_1` (`TSN(2, array, 0x80 + n)` at script offset `0x397B` in the hub, `0x3985` in SL and LL) | builds a 255-byte array: words and bytes at +0..+15, user name at +16, host at +28, password at +44, then the next program's parameters from +128. The layout is `messages.md` section 5.3; this census re-decoded the same offsets from the bytecode of the writer and of the reader | CONFIRMED |
| Callers of `export_1` | 7 sites: `script.050` in each land (the string `"ShopAdv"`, 7 arguments), SL `script.120` (2 sites), `script.909` and `script.910` (21 arguments) | the 8th argument and later become the parameter bytes at +128 | CONFIRMED |
| Name the next program | `TSN(9, name)` | name from the land table by `landType`, else `"Default"` | CONFIRMED |
| Wait for the wire to drain | `TSN(12)` loop with `TSN(8)` and `TSN(14)`, then quit | section 7.3 | CONFIRMED |
| Leave the old game object | `script.101 proc_125 +0x020B`: `TSN(4, "w+", 9, oldSid, 0)` | sent only when the block's SID word at +4 is nonzero and `global85` (the connected flag) is set | CONFIRMED |
| Read on arrival | `script.101 proc_125` (`TSN(1)` at script offset `0x3A46`, SL and LL `0x3A50`) | `GetSharedData` returns 0 when empty and the reader skips everything; else it restores identity, sets `landNumber`, `landType`, `hostNumber`, `landFlags` from bytes 6 to 13, copies the tail from +128 into a new array, disposes the block array | CONFIRMED |
| Consume | `TSN(2, 0, 0)` right after the read (also in `sparkScript::changeState`) | clears the block | CONFIRMED |

Parameter bytes are one byte each (the array is a byte array and each value is stored through `at`), so
a next-program parameter above 255 would be truncated. INFERRED from the array type.

The DOS games' reads of words at `+0` and `+4`, an 11-byte string and parameters from `+0x80`
(`int14h-api.md` section 9.3) match this layout when the string offset is read as hex `0x10` (decimal 16, the user-name field). INFERRED.

## 7. Timer, poll and tick integration

### 7.1 Where the exports run

| Caller | Exports | Context |
|---|---|---|
| Event pump `16B1:0358`, from `TSN(8)` and from the `GetEvent` poll-hook table (`0847:098A` calls each hook, `0847:0926` adds, `0847:0956` removes) | Poll, then Receive until it returns 0 or the network queue (`0847:0569(0x400)`) has 2 or fewer free slots | foreground, game loop |
| Timer server `16B1:03F1`, registered `InstallServer(03F1, 20)` after a good Connect or SwitchHost | Service | IRQ0 handler `0CAB:0130`: increments `TickCount`, then for every server decrements its countdown (`[bx+6]`), reloads from `[bx+4]` and does `lcall [bx]` |
| `onexit` `16B1:031C` | removes the server and the hook, then SetCallbacks(NULL x3) | process exit |

CONFIRMED (disassembly of `0CAB:0130`..`01A5` and the cases; hook table per `ktsn.md` section 4.1). The timer server writes `TickCount` through `DGROUP:0430` before it calls Service; the pump calls Poll first and writes
`TickCount` after it (`16B1:0366` then `037E`..`0393`), so Poll sees the value left by the previous pump or server run.
Either way TSNEXEC's retransmit clock advances at LSCITV's tick rate.

### 7.2 Rates

| Quantity | Value | Status |
|---|---|---|
| Tick rate | 60 Hz (PIT divisor `0x4DAE`), no Feb-94 script changes it | CONFIRMED (`ktsn.md` section 4.2) |
| Service period | 20 ticks, about 3 per second | CONFIRMED (constant `0x14`), rate INFERRED from 60 Hz |
| ACK timeout | 300 ticks, about 5 s | CONFIRMED value, seconds INFERRED |
| Poll and Receive | once per `GetEvent` call and per `TSN(8)`; no fixed period | CONFIRMED |
| Dead-link detection | Service returns 2 or 3 after 10 NAKs or 12 timeouts (about a minute at 5 s) | CONFIRMED in `int14h-api.md` section 6 |

A registered hook runs before every `GetEvent` read, so the game must keep calling `GetEvent` for
messages to arrive. Several failed `TSN(3)` calls add several copies of the hook: the table holds 5 far pointers at `DGROUP:3444`, `0847:0926` does not test for duplicates (a full table drops the add) and `0847:0956` removes one copy per call. CONFIRMED.

### 7.3 Idioms that depend on server timing

| Idiom | Where | Dependence |
|---|---|---|
| `TSN(4, ...); TSN(14)` | 259 flushes | the message must leave at once; the client does not wait |
| `TSN(8); TSN(14)` | 59 pump sites | polling loops while a script waits for a reply |
| `while (!TSN(12)) { TSN(8); TSN(14) }` | `script.101` land switch (`+0x0274`..`+0x028A`) and `script.130` (CONFIRMED); `script.120`, `530`, `747`, `780`, `909`, `910` also call `TSN(12)` (INFERRED to be drains) | spins until every data frame has been ACKed; with no ACK it ends only through an error event |
| Login timeout 70 s | `script.101 LoginTimeout` | the host must answer Login within 70 s (`messages.md` section 4.2) |
| Connect error | `ErrorEvent(0, code)` and accumulator 0 | script shows the text for that code |

## 8. Other builds

`tools/int14h_census.py` finds the same 19 call sites, the same wrapper shape (init and dispatcher in one function, pump, timer server, shutdown, Send helper) and one stub in each build. CONFIRMED.

| Build | `p` | Stub | TSN handler | Pump / server / Send helper |
|---|---|---|---|---|
| INN Feb-94 | `DGROUP:1EC4` | `178B:0012` | `16B1:0004` | `0358` / `03F1` / `042E` |
| INN Dec-93 | `DGROUP:1C26` | `16B9:001A` | `15DF:000C` | `0360` / `03F9` / `0436` |
| TSN 2.1 | `DGROUP:1258` | `1331:001A` | `126A:000C` | `0363` / `03F8` / `0435` |

## 9. Other users of INT 14h

A byte scan of every EXE, DLL and driver under `work/sets`, `work/ex`, `work/exe` and `work/games` for the
20-byte stub found it only in LSCITV (3 builds), GOLF, CGENN, DARKSTRT, TWGENN, FATES and the unpacked RB
and SHOPADV. The other `CD 14` hits are inside LZEXE-packed files or data files. Verification unpacked all 34
distinct EXE, COM, DRV and SYS files of `work/sets` with `tools/unlzexe.py` and rescanned: INSTALL, INSTTSN, BOOTDISK, RBJOY, ID,
PATCHPIF, LSCIGET and TSNEXEC contain no INT 14h code (the two INSTALL builds hold `CD 14` bytes only in a
data table, image `0x1F52` and `0x1F4A`). The installed `GRAPH256.DLL` and `NLNULL.DLL` (`work/dosbox/c/INN`)
contain no `CD 14`. CONFIRMED. The other sets' DLLs are SZDD-compressed and the TSN basic set stores
`LSCITV` and `TSNEXEC` in a different container (`_SCITV.EXE`, `_SNEXEC.EXE`, name in the header); that
none of them calls INT 14h beyond the same stub is INFERRED.

## 10. What innkeeper must provide for LSCITV

| Responsibility | What the client does | Status |
|---|---|---|
| Accept a connection | `TSN(3, string)` with a modem string, a Novell host name or `"foo"`; success needs no data from the host. Errors are `ErrorEvent(0, code)` with the codes of `int14h-api.md` section 6 | CONFIRMED |
| Speak second | after Connect the client sends Login first (command 53 or 59) and expects an Ack within 70 s | CONFIRMED (`messages.md`) |
| Report device state | `TSN(0)`: `AL` 1, 2 or 3 picks the serial, Novell or modem dial branch (3 builds a modem string from `LSCI.CFG` keys); `AH` 0x80 while connected and 0 after `TSN(7)`, because `TSN(3)` returns 1 without calling Connect while `AH` is nonzero; `TSN(16)` any rate, shown in the version box when `AL` is 3 | CONFIRMED; value choice INFERRED |
| ACK promptly | transmit is idle only after the last data frame is ACKed; the land switch and batch senders spin on it | CONFIRMED |
| Deliver messages in order, whole | one `0x400` event per message, at most one per `Receive`; messages up to 65535 bytes; the event queue stops draining at 2 free slots | CONFIRMED; limit INFERRED |
| Signal link loss | event `0x800` with code 1 (carrier lost), 2 or 3 (retries); the script reacts | CONFIRMED |
| Handle SwitchHost | `TSN(13, address)` between lands that differ in host number: flush both queues, then the client sends Login again. Address is the `HOSTADDR` column 2, or the host name for Novell | CONFIRMED (`messages.md` section 5.2, 7.2) |
| Handle Disconnect | `TSN(7)` frees queued messages and hangs up; scripts call it before every redial | CONFIRMED |
| Store nothing for the shared block | it lives in the local TSNEXEC; innkeeper never sees it. A land switch arrives as a new session or SwitchHost followed by Login and joinNet | CONFIRMED |
| Tolerate lost messages across a land switch | queued and half-received messages are lost when TSNEXEC runs the next program | CONFIRMED (`int14h-api.md` section 9.3); effect on the stream INFERRED |
| Timing | Service every 20 ticks and a 300-tick ACK timeout apply only to a real link; a ScummVM `kTsn` has no link layer, but the drain idiom needs `TSN(12)` to become 1 once its sends are flushed | INFERRED |

## 11. Ghidra and tool outputs

| Output | Content |
|---|---|
| `work/decomp/lsci/client_segments.c`, `.asm` | whole-segment export of the functions found in `16B1`, `178B` and neighbours. The `TSN` handler `16B1:0004` decompiles badly because its jump table sits in code. The pump, `Send` helper and callbacks decompile cleanly |
| `work/decomp/lsci/ktsn_cases.c` | one function per jump-table case, the stub, `onexit` and the timer server, produced by `work/ghidra/census-lsci/scripts/CaseDump.java` |
| `work/decomp/lsci/pointer_xrefs.txt` | empty: Ghidra's real-mode model creates no data references for DS-relative operands, so `p` has no xrefs there |
| `tools/int14h_census.py` | recursive-descent disassembly of every function that touches `p`, listing each table call with its export, argument bytes and wrappers. Options: `--wrappers`, `--context`, `--json` |

Reproduce:

```
.venv/bin/python tools/int14h_census.py work/exe/LSCITV_inn_cd.EXE --wrappers
.venv/bin/python tools/ktsn_callsites.py work/res/inn_feb94 --summary-only
```

## 12. Open questions

- What the serial driver does with `"foo"` (the script path is `script.101 proc_80`, `AL` = 1, section 4; the serial driver is not in the set, `messages.md` section 7.1 has the grammar).
- Whether any script frees the received byte arrays (nothing in LSCITV's TSN code does; the kernel `Array` dispose paths clear flag `0x02` before unlocking, `0130:012C(h, 0)` at image `0x1151D` and `0x119A8`, so a script `dispose` would work; INFERRED).
- Whether a program above 255 in the parameter tail is ever passed.

## 13. Verification

Independent re-check by capstone (linear disassembly of `16B1`, `0847`, `0CAB`, `0130`, `178B`) and a Ghidra 12.1.4
project in `work/ghidra/verify-census-lsci/` (output in `work/decomp/lsci/verify_funcs.c*`, not committed).

| Claim checked | Result |
|---|---|
| 19 table calls, export indices, argument bytes, 17 jump-table words, offsets `4N` | reproduced; every site read in disassembly |
| Other readers of `p` or copies of the stub | none: whole-image scan finds one `CD 14` and displacement `1EC4`/`1EC6` only in `16B1` and the stub; Ghidra lists no other `CALLF [reg+N]` into the table (the four `1B39` sites call function-pointer fields at `es:[si+12h]` and `es:[si+16h]`, not the table) |
| Same shape in Dec-93 and TSN 2.1 | reproduced: 19 sites, one stub, five `les es:[p]` and one `mov bx,p` in each build |
| Script site counts | reproduced: 848 `TSN` sites over hub, SL and LL; sub-op 0 split 25 `& 255` and 30 `>> 8`, 55 in total |
| Send first values | corrected: 33 distinct constants over 342 sites, not 34; the other 39 sites (13 per land, `type31.558`) compute the value. `0x11F` is a `w` value (bytes `1F 01`), see `messages.md` section 3 |
| The alloc, deref and free callbacks, pump, timer, `ErrorEvent` and `EventPost` layouts | reproduced (`0847:063A` builds type `0x800`, `message` = first argument, `modifiers` = code) |
| `178B` "holds only the stub" | refuted: it also holds Borland long-arithmetic helpers; none reads `p` |
| Pump writes `TickCount` first | refuted: Poll runs first, then the write (section 7.1) |
| "Connection byte is set" for `TSN(4, "w+", 9, ...)` | corrected: the script tests `global85`, its own connected flag |
| `"foo"` path | resolved: `script.101 proc_80`, `AL` = 1; `AL` 3 and 2 are the modem and Novell branches |
| Packed installers and DLLs | upgraded to CONFIRMED by unpacking (section 9), except the TSN basic container and SZDD DLLs |

Not reproducible: real-hardware timing of the 20-tick server and 300-tick ACK timeout (60 Hz from the PIT divisor only), the effect of
a stale `AH` after Disconnect on a real TSNEXEC (derived from case 3 code, not run).

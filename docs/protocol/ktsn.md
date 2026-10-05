# The `TSN` kernel (kTSN) in LSCITV

How LSCI scripts reach the network. The script side is one kernel, `TSN` (0x54), whose first argument
selects a sub-op. This document covers that kernel, how it binds to the resident executive, how
network traffic turns into events, and the related interpreter infrastructure: timer servers,
`ConfigStr`, and leaving LSCITV for another land. The executive side of the same interface (TSNEXEC
exports, com driver, link layer, `TSN.PRG`) is in `docs/protocol/int14h-api.md`. This file uses the
export names from that document.

Primary binary: `work/exe/LSCITV_inn_cd.EXE` (INN Feb-1994, LZEXE-unpacked).

## 0. Address convention

- `seg:off` is relative to the unpacked load image. LSCITV's MZ header is `0x30E0` bytes, so
  **file offset = `0x30E0` + seg×16 + off**. Ghidra loads the image at `1000:0000`; add `0x1000` to the segment there.
- DGROUP is `2074`. `DGROUP:xxxx` means `2074:xxxx`, which is file `0x23820 + xxxx`.
- TSNEXEC addresses use that binary's convention: header `0x190`, data segment `031D`.
- Kernel calling convention, documented in `docs/lsci/interpreter.md`: the handler is `far f(argv far *)`
  with `argv[0]` = argc. It returns a result by writing the VM accumulator at `DGROUP:075A`.
  **A handler that does not write `075A` leaves the accumulator unchanged.** Several sub-ops below
  rely on this.

## 1. Locating the handler

| Claim | Evidence | Status |
|---|---|---|
| The kernel handler table is 89 far pointers at `DGROUP:1CE6`. Entry 0x54 is `16B1:0004`, which is `TSN` in the parallel name table. | `DGROUP:1CE6 + 0x54*4` holds `0004 16B1` (file `0x25656`); `tools/lsci_tables.py` | CONFIRMED |
| `callk` indexes that table directly with `n*4`, after checking `n` against `DGROUP:1EC2` (0x58). | `09C8:052C`..`057B` (file `0x0D28C`) | CONFIRMED |
| `TSN` handler: `16B1:0004` (file `0x19BF4`). It is a 17-way jump table on `argv[1]` at `16B1:0083` (file `0x19C73`). A sub-op above 0x10 returns without action. | `cmp ax,10h` at `16B1:0073`, then `jbe` to the table or `jmp 0318` | CONFIRMED |
| Kernels 0x55–0x58 (`ObjPropOffset`, `ObjOffsetProp`, `SID`, `InvokeMethod`) share segment `16B1`. They have nothing to do with the network. | handler table | CONFIRMED |

## 2. Binding to the executive

### 2.1 Finding the export table

- The pointer to the TSNEXEC export table is cached at `DGROUP:1EC4` (far, file `0x256E4`). CONFIRMED.
- On every `TSN` call the handler first tests that pointer (`16B1:000A`). While it is NULL, the handler
  calls the stub `178B:0012` (file `0x1A9A2`): `mov ax,[1EC4]; or ax,[1EC6]; jz; retf; int 14h;
  mov [1EC4],ax; mov [1EC6],dx; retf`. The stub is entered with `AX=0` (just tested) and `DX`
  undefined, and it keeps `DX:AX` as the table pointer. CONFIRMED.
- This is the **only `INT 14h` site in LSCITV**, and only segments `16B1` and `178B` read `DGROUP:1EC4`. CONFIRMED
  (byte scan of the code segments for `CD 14` and for `C4 1E` displacements).
- LSCITV has no "executive missing" check. Without TSNEXEC, the BIOS INT 14h status lands in
  `DX:AX`, and the next far call goes to an arbitrary address. CONFIRMED (no check in code); crash INFERRED.

### 2.2 First-use initialisation (`16B1:0020`..`0069`)

1. `onexit(16B1:031C)`. `onexit` is `0000:0177`; DLLs see it under the same export name (section 8). CONFIRMED.
2. `SetAckTimeout(300)` (export 6). The returned far pointer to TSNEXEC's tick counter is stored at
   `DGROUP:0430`. CONFIRMED.
3. `SetCallbacks(alloc=16B1:0B8E, deref=16B1:0BB8, free=16B1:0BE5)` (export 15). CONFIRMED.

### 2.3 Shutdown (`16B1:031C`, file `0x19F0C`, run by `onexit`)

`DisposeServer(16B1:03F1)` (section 4.2), then remove the event poll hook `16B1:0358`, then
`SetCallbacks(NULL, NULL, NULL)`, then `178B:0026`, which is a bare `retf`. CONFIRMED. This step
matters: the executive outlives LSCITV and must not call back into freed memory.

### 2.4 Handle callbacks

The executive moves message bodies only as **handles** (int14h-api.md section 4). LSCITV implements
them with interpreter byte arrays. Names are from the DLL export table (section 8). CONFIRMED.

| Callback | Address (file) | Body | Status |
|---|---|---|---|
| alloc(size) | `16B1:0B8E` (`0x1A77E`) | `h = ArrayNew(size, 2)`; `0130:012C(h, 1)`; returns `DX:AX = 0:h` | CONFIRMED |
| deref(handle) | `16B1:0BB8` (`0x1A7A8`) | `ArrayData(BlkGetPtrP(h), 0)`: far pointer to element 0 | CONFIRMED |
| free(handle) | `16B1:0BE5` (`0x1A7D5`) | `BlkFree(h)` | CONFIRMED |

- `ArrayNew(count, type)` (`0BF4:017D`). Types: 0 = 2-byte elements, 1 = 2-byte elements (ID array),
  2 = 1-byte elements, 3 = 1-byte elements with memory type 7 (string). The header is
  `{uint16 elementSize, uint16 count}`, and data starts at header+4. CONFIRMED (`0BF4:01C6`..`0228`).
- So every network buffer is a **byte array (type 2)**, and the 16-bit handle is the whole reference.
  The far "handle" the executive stores is `0000:h`. CONFIRMED.
- `0130:012C(h, 1)` sets bit `0x02` in the block's memory-manager flags (`18E5:04E5`). CONFIRMED. It is
  applied only to buffers the executive will hold: alloc, and the sub-op 4 send buffer. It is not applied
  to arrays returned straight to scripts. Its purpose, keeping blocks that no script references from
  being reclaimed, is INFERRED.

## 3. Sub-operations

`TSN(n, ...)` calls export `n` for every sub-op that does anything; the sub-op number is the
export index. CONFIRMED (jump table words `00A5 00B1 011E 0167 01C2 0318 0318 01D8 01FF 0206 0318
0232 0283 0290 02FA 0318 0306`, where `0318` is the common return).

"acc kept" means the accumulator is not written (section 0). "Sites" is the Feb-94 script scan (section 9).

| Sub-op | Script form | Case (file) | Export | Returns | Side effects | Sites | Status |
|---|---|---|---|---|---|---|---|
| 0x00 | `TSN(0)` | `16B1:00A5` (`0x19C95`) | 0 GetStatus | `AX`: `AL` driver status, `AH` connection byte | — | 55 | CONFIRMED |
| 0x01 | `TSN(1)` | `16B1:00B1` (`0x19CA1`) | 1 GetSharedData | new byte array holding a copy of the stored shared block (at most 256 bytes), or 0 when the stored length is 0 | allocates `ArrayNew(len,2)` and copies with `034F:000E` | 3 | CONFIRMED |
| 0x02 | `TSN(2, array, len)` | `16B1:011E` (`0x19D0E`) | 2 SetSharedData | acc kept | copies `len` bytes from element 0 of `array`. If either argument is 0, calls `SetSharedData(NULL,0)`, which clears the block. | 9 | CONFIRMED |
| 0x03 | `TSN(3, string)` | `16B1:0167` (`0x19D57`) | 3 Connect | 1 = connected or already connected; 0 = failed | if `GetStatus` `AH≠0`, returns 1 immediately. Otherwise calls Connect(`StrPtrGet(string)`). On error: `ErrorEvent(0, code)`. On success: `InstallServer(16B1:03F1, 20)`. On both of these paths it adds the poll hook `16B1:0358`. | 10 | CONFIRMED |
| 0x04 | `TSN(4, format, values...)` | `16B1:01C2` → `16B1:042E` (`0x1A01E`) | 4 Send | Send result (1 queued, 0 failed); 0 if the formatted body is empty | builds a byte array from `format` (section 3.1) and passes it as `Send(0:h, size)`. The executive frees it later. | 381 | CONFIRMED |
| 0x05, 0x06 | — | `16B1:0318` | — | acc kept | none. Exports 5 and 6 are internal (sections 4.1, 2.2). | 0 | CONFIRMED |
| 0x07 | `TSN(7)` | `16B1:01D8` (`0x19DC8`) | 7 Disconnect | acc kept | `DisposeServer(16B1:03F1)`, remove the poll hook, then Disconnect | 33 | CONFIRMED |
| 0x08 | `TSN(8)` | `16B1:01FF` (`0x19DEF`) | 8 Poll (+ 5 Receive) | acc kept | runs the event pump `16B1:0358` once (section 4.1) | 59 | CONFIRMED |
| 0x09 | `TSN(9, name)` / `TSN(9, 0)` | `16B1:0206` (`0x19DF6`) | 9 SetNextProgram | acc kept (the export's 1 is discarded) | sets the `TSN.PRG` block to run when LSCITV exits; 0 cancels (section 5) | 6 | CONFIRMED |
| 0x0A | — | `16B1:0318` | — | acc kept | none. Export 10 runs from the timer server (section 4.2). | 0 | CONFIRMED |
| 0x0B | `TSN(11)` | `16B1:0232` (`0x19E22`) | 11 GetPreviousProgram | new string (`StrNew(strlen+1)` plus copy), or 0 | — | 0 | CONFIRMED (no script uses it) |
| 0x0C | `TSN(12)` | `16B1:0283` (`0x19E73`) | 12 IsTransmitIdle | 1 when no data frame awaits an ACK | — | 22 | CONFIRMED |
| 0x0D | `TSN(13, string)` | `16B1:0290` (`0x19E80`) | 13 SwitchHost | 1 = success; 0 = failed | error: `ErrorEvent(0, code)`. Success: `DisposeServer` + `InstallServer(16B1:03F1, 20)`. Either way it removes and re-adds the poll hook. | 6 | CONFIRMED |
| 0x0E | `TSN(14)` | `16B1:02FA` (`0x19EEA`) | 14 Flush | acc kept | forces queued messages onto the wire | 259 | CONFIRMED |
| 0x0F | — | `16B1:0318` | — | acc kept | none. Export 15 is used only by init and shutdown. | 0 | CONFIRMED |
| 0x10 | `TSN(16)` | `16B1:0306` (`0x19EF6`) | 16 GetLineRate | `AX` from driver function 9 | — | 5 | CONFIRMED |

- String arguments go through `StrPtrGet` (`02C4:0312`). It accepts a string handle or an object;
  for an object it reads property selector 0x1D. CONFIRMED (`02C4:0329`..`033E`).
- First use (section 2.2) runs before the dispatch for any sub-op, including the no-ops. CONFIRMED.
- Usage pattern: scripts follow most `TSN(4, ...)` calls with `TSN(14)`. Poll is `TSN(8)`, usually
  followed by `TSN(14)`. CONFIRMED (scan order, for example `hub/script.010` at `0x5B7`/`0x5C4`).

### 3.1 Formatted send (sub-op 4)

`16B1:042E` receives `argv` shifted by one: `argv'[0] = argc-1`, `argv'[1] = format`, and values from
`argv'[2]`. It makes two passes over the format string: one to size the body, one to fill it. CONFIRMED.

| Code | Consumes | Emits | Status |
|---|---|---|---|
| `b` | 1 value | low byte | CONFIRMED (`16B1:06C6`) |
| `w` | 1 value | 2 bytes, little-endian | CONFIRMED (`16B1:0716`) |
| `s` | 1 string | its bytes plus the terminating NUL (`StrLen`+1) | CONFIRMED (`16B1:06D6`; `02C4:0206` for the size) |
| `a` | 2 values: `count`, then an array | `count` bytes for a 1-byte-element array, otherwise `count*2` raw bytes (native little-endian) from element 0 | CONFIRMED (`16B1:0656`) |
| `+` | all remaining values | repeats the previous code: bytes if the previous code was `b`, otherwise words | CONFIRMED (`16B1:05DB`) |
| other | 1 value | nothing | CONFIRMED (`16B1:05D5`) |

- An empty body returns 0 and sends nothing. Otherwise the body goes into a new byte array with flag `0x02`
  set (section 2.4), and the result of `Send(0:h, size)` is returned. CONFIRMED (`16B1:052A`, `16B1:0775`).
- Format strings in the scripts include `bbw`, `bb`, `bbw+`, `bbs`, `bwwaw+`, `bwwaa`, `wwwwbbba`,
  `bbwwwa`, `w+`. CONFIRMED as string data in `script.101`, `.010` and `.005`. Each call pushes its format
  as a string literal (`pushID "bbw"`), which `tools/lsci_disasm.py` prints, for example
  `TSN(4, "bbw", 34, 4, sid)` at `hub/script.101` item 4 and `TSN(4, "bb", 36, 5)`. CONFIRMED.
- The first value after the format is a small constant at almost every site (33 distinct values,
  0x02–0x3B and 0x11F). Feb-94 counts: `0x02`×86, `0x25`×54, `0x1B`×35, `0x22`×15, `0x0D`×14, `0x1C`×14…
  This matches the DOS games, which put an opcode in the first message byte (int14h-api.md section 10).
  CONFIRMED (scan); "message opcode" INFERRED.

## 4. Network events

### 4.1 Event pump (`16B1:0358`, file `0x19F48`)

1. `code = Poll()` (export 8). If `code≠0`, `ErrorEvent(0, code)`. CONFIRMED.
2. Writes `TickCount` (`DGROUP:0B58`) through the tick-counter pointer `DGROUP:0430`. CONFIRMED.
3. While the network event queue has more than 2 free slots (`0847:0569(0x400)`):
   `len = Receive(&h)` (export 5). Stop when it returns 0. Otherwise post
   `{type=0x400, message=h, modifiers=len, when=TickCount}` with `EventPost` (`0847:04AB`). CONFIRMED.
   Only the low word of the far handle is kept. CONFIRMED.

The pump runs in two ways:

- **From `GetEvent`**. `TSN(3)` and `TSN(13)` register `16B1:0358` with `0847:0926` in the five-slot
  poll-hook table at `DGROUP:3444` (file `0x26C64`). `0847:098A` calls every hook before reading a queue:
  `0847:010B` for the network queue, `0847:01F8` for the main queue, `0847:02D7` for a network peek,
  `0847:0376` for a main-queue peek. CONFIRMED.
- **From `TSN(8)`**, directly. CONFIRMED.

The hook table has no duplicate check (`0847:0926` takes the first empty slot, and silently does nothing when
all five are full). `TSN(3)` adds the hook after every attempt that is not already connected, failed ones
included, so repeated failed dials stack copies of the pump. CONFIRMED (`16B1:01AF`..`01BF`, `16B1:02E4`).

When the network queue is nearly full, messages wait in the executive's receive queue
(250 slots, int14h-api.md section 8). CONFIRMED (loop condition); the backpressure effect is INFERRED.

### 4.2 Timer server (`16B1:03F1`, file `0x19FE1`)

- Installed by `InstallServer(16B1:03F1, 20)` after a successful Connect or SwitchHost. Each call writes
  `TickCount` through `DGROUP:0430`, then calls `Service()` (export 10). A nonzero result is posted with
  `ErrorEvent(0, code)`. CONFIRMED.
- Servers run **inside LSCITV's IRQ0 handler** `0CAB:0130` (file `0x0FCC0`). That handler increments
  `TickCount` (`DGROUP:0B58`), then walks the server table `DGROUP:0AEA`..`[0B3A]` (8-byte entries:
  far function, period, countdown) and calls each server whose countdown reaches 0. CONFIRMED.
- So the link layer is serviced every 20 interpreter ticks from interrupt context. Allocation and
  message assembly happen only in the foreground through Poll (export 8). This is why the
  executive's reentrancy lock exists (int14h-api.md section 5). CONFIRMED (call path).
- The tick is 60 Hz by default, so Service runs about 3 times a second and the 300-tick ACK timeout is
  about 5 s. `SetTimerFreq` (`0CAB:0023`) is kernel 0x36. Its initial state at `DGROUP:0B3C` is PIT divisor
  `0x4DAE` (1193182 / 19886 = 60.0 Hz) with the BIOS INT 8 chained about every third tick. No Feb-94 script
  calls `SetTimerFreq`. CONFIRMED (static data, handler code, `lsci_disasm` search of all three lands).

### 4.3 Event types, queues and `GetEvent`

| Item | Detail | Evidence | Status |
|---|---|---|---|
| Event record | 14 bytes: `+0` type, `+2` message, `+4` modifiers, `+6` when (uint32 ticks), `+A`/`+C` position | `0847:0605` (null event), `0847:063A` | CONFIRMED |
| `0x0400` network message | `message` = byte-array handle holding the body, `modifiers` = body length | `16B1:03B3`..`03D9` | CONFIRMED |
| `0x0800` network error | `message` = 0, `modifiers` = status or error code (Poll/Service 1–3; Connect and SwitchHost codes in int14h-api.md section 6) | `ErrorEvent` = `0847:063A` (file `0x0BB8A`) | CONFIRMED |
| Two queues | an event whose type has bit `0x400` goes to the network queue (handle `DGROUP:070C`, head `0712`, tail `0714`); everything else, including `0x800`, goes to the main queue (`070A`, `070E`/`0710`). Both have `DGROUP:0708` slots of 0x14 bytes. | `0847:04BA`, `0847:0403` | CONFIRMED |
| `GetEvent(mask, event)` | `mask & 0x8000` means peek. `mask == 0x400` reads only the network queue. A mask containing `0x400` tries the network queue first, then the main queue. Otherwise only the main queue, where the first event with `type & mask` is taken. | `0847:0798` (file `0x0BCE8`), `0847:01C6`, `0847:033C` | CONFIRMED |
| Event object | type, message, modifiers, `+C` and `+A` are written to the properties whose offsets are cached at `DGROUP:0556`, `0558`, `055A`, `0552`, `0554`. `when` is not copied. | `0847:068E` | CONFIRMED |
| Position of a `0x400` event | not initialised. The pump's local record leaves `+A`/`+C` as stack contents. | `16B1:03B3`..`03D9` | CONFIRMED |

- The received byte array belongs to the script after `GetEvent`. Receive hands over ownership
  (int14h-api.md section 3), and LSCITV keeps no reference to it. CONFIRMED. Which kernel the scripts
  use to free it is INFERRED to be `Array`'s dispose operation.

## 5. Leaving LSCITV: land and program switching

- LSCITV has no "run program" facility. A script calls `TSN(9, name)` to register the next `TSN.PRG`
  block (export 9 falls back to `DEFAULT` for an unknown name), then ends the game normally. After LSCITV exits, TSNEXEC
  runs that block: `program SLand` = `cd SL` + `..\lscitv.exe`. CONFIRMED (sub-op 9 path; TSNEXEC
  `0000:00DE` loop at `0000:0193`). `TSN(9, 0)` clears the pending flag (`031D:0248`). With no block pending
  when the child exits, the main loop leaves through `0000:0224` and TSNEXEC itself ends. CONFIRMED (path);
  what the teardown at `0000:2378` does is not traced.
- The connection, link state and 256-byte shared block survive the switch. Callbacks and queued
  messages do not (int14h-api.md section 9.3). LSCITV's `onexit` handler clears its callbacks first
  (section 2.3). CONFIRMED.
- `TSN(2, array, len)` writes the hand-off block before leaving, and `TSN(1)` reads it on arrival.
  `TSN(11)` returns the previous block name but no script calls it. CONFIRMED (scan: sub-op 1
  and 9 only in `script.101`, sub-op 2 only in `script.101`).
- `LAND.CFG` maps program names to directories and land numbers (`Sierr . 1 Clubhouse`,
  `SLand .\SL 2 SierraLand`, `LLand .\LL 3 CasinoLand`, `Yserbius .\YSERBIUS 4`, `Twinion .\TWINION 8`).
  The names match the `TSN.PRG` blocks. `script.101` contains the string `land.cfg`. CONFIRMED (data);
  that `script.101` reads it to choose the `TSN(9)` argument is INFERRED.

## 6. `ConfigStr` and `LSCI.CFG`

| Claim | Evidence | Status |
|---|---|---|
| Kernel 0x4F `ConfigStr(key)` → new string copy of the value (`StrPtrDup`, at most 200 bytes), or 0 when the key is absent | `181C:08AA` (file `0x1BB4A`) | CONFIRMED |
| Loader `181C:06B6` reads the file a line at a time (`0028:0071`: stops at LF, drops CR, at most 200 bytes), skips lines that are empty after leading blanks or that start with `;`, and keeps the raw lines in a list | `181C:070A`..`0775` | CONFIRMED |
| Lookup `181C:0781`/`07A9`: the key is the first token delimited by any of `= ; space tab` (`DGROUP:2159`), compared **case-sensitively** (`0051:0124`). The value is the rest of the line after skipping delimiter characters. The first matching line wins. | `181C:0804`..`0879` | CONFIRMED |
| A key containing a space cannot be found as written. `default baud = none` is stored under the key `default` with value `baud = none`. | follows from the delimiter set | INFERRED |
| Interpreter keys: `virtualDir` (default `.`, 80 bytes to `1C86:3C6E`), `maxSounds` (`atoi`, default 5) | `181C:09CE`..`0A1E` | CONFIRMED |
| Default file `lsci.cfg` (`DGROUP:219A`); `TSN.PRG` passes `-cLSCI.CFG` in the root land only | string; `TSN.PRG` | CONFIRMED (string); `-c` parsing not traced |
| Keys that scripts pass to `ConfigStr` (`pushID "key"; callk ConfigStr`, decoded with `tools/lsci_disasm.py` over all three Feb-94 lands): `prefix`, `hostID`, `modem`, `modem2`, `novell` (`script.101`, hub `.012`), `hostID` (`.004`, `.055`), `landNum`, `hostNum`, `teleport`, `ShutUp` (`script.101`), `LOGONVOL` (`.000`, `.050`, `.101`), `id`, `name`, `CHATALOG`, `SHOWREV`, `demo`, `music`, `notelog`, `pFlag` (`.004`, `.005`, hub `.000`), `SEASONS`, `NoChain`, `video`, `pathStr`, `prodPath` (`.050`), `MONOMAIL` (`.140`), `promo` (hub `.012`) | `callk ConfigStr, 2` sites with the key pushed as a string literal; `script.101` message "The 'modem' string in LSCI.CFG file is incorrect"; `script.005` message "no CHATALOG line in your LSCI.CFG file for this land" | CONFIRMED |
| `dialScript`, `attachScript` and `LoginTimeout` are **not** config keys. They are script class and object names in `script.101` (`loadID`, `LoginTimeout::changeState`). The string `ID` in hub `script.012` is a literal for a `Str` object. | `lsci_disasm` of `hub/script.101`, `hub/script.012` | CONFIRMED |

The shipped `LSCI.CFG` holds only `directory`, `default baud`, `maxSounds` and `promo`. The installer
and `ID.EXE` presumably add the rest. INFERRED.

## 7. `InstallServer` / `DisposeServer` and the DLL export table

- `InstallServer(fn far, period)` (`0CAB:025D`, file `0x0FDED`) appends `{fn, period, period}` to the
  timer server table (10 entries, `DGROUP:0AEA`..`0B3A`), ignoring duplicates. `DisposeServer(fn)`
  (`0CAB:02A5`) removes the entry with interrupts disabled. CONFIRMED.
- They are not kernels. They appear in the **DLL export table** at `DGROUP:0EEA`..`1098` (file `0x2470A`):
  43 entries of `{far function, far name, uint16 0}`, which DLLs such as `GRAPH256.DLL` and
  `NLNULL.DLL` link against. CONFIRMED.
  Table entries used by kTSN: `ArrayData 0BF4:0629`, `ArrayNew 0BF4:017D`, `ArrayPtr 0BF4:05F2`,
  `BlkFree 0130:0042`, `BlkNew 0130:0016`, `BlkGetPtrP 16AC:0002`, `BlkLockMask 0130:0642`,
  `BlkUnlockMask 0130:06A2`, `DisposeServer`, `ErrorEvent 0847:063A`, `EventPost 0847:04AB`,
  `InstallServer`, `onexit 0000:0177`, `StrNew 02C4:01A0`, `StrPtrDup 02C4:01CA`,
  `StrPtrGet 02C4:0312`, `TickCount 0F96:0090`. CONFIRMED.
  So a DLL could post network-style events or install its own servers. None of the shipped DLLs has
  been checked for that. INFERRED.

## 8. Build differences

The kTSN jump table has the same shape in all three builds, with no-ops at sub-ops 5, 6, 10 and 15.
CONFIRMED.

| Build | Handler | Table | Case words |
|---|---|---|---|
| INN Feb-94 | `16B1:0004` | `16B1:0083` | `00A5 00B1 011E 0167 01C2 0318 0318 01D8 01FF 0206 0318 0232 0283 0290 02FA 0318 0306` |
| INN Dec-93 | `15DF:000C` | `15DF:008B` | `00AD 00B9 0126 016F 01CA 0320 0320 01E0 0207 020E 0320 023A 028B 0298 0302 0320 030E` |
| TSN 2.1 | `126A:000C` | `126A:008B` | `00AD 00B9 0125 0171 01CC 0323 0323 01E2 0209 0210 0323 023C 028E 029B 0305 0323 0311` |

The init sequence (stub, SetAckTimeout(300), SetCallbacks) is the same in TSN 2.1 (`126A:000C`..`0071`).
CONFIRMED. The sub-op histogram in section 9 has the same set of sub-ops in every set. CONFIRMED.

## 9. Script usage scan

`tools/ktsn_callsites.py work/res/<set>` lists every `callk 0x54` in every `script.*` and `type31.*`
item. It decodes with `tools/lsci_bytecode.py` and follows the stack and accumulator through the
item (`tools/lsci_callflow.py`), so the sub-op and the argument list of each call are read off the
frame that `callk` really pops, not guessed from the bytes before it. CONFIRMED method; the earlier
byte-pattern scan had 11 unresolved hits (7 of them inside string data) and is superseded.
Usage across all kernels: `docs/lsci/kernel-usage.md`.

Feb-94 (hub + SL + LL, sites per sub-op):

| Sub-op | 0 | 1 | 2 | 3 | 4 | 7 | 8 | 9 | 0x0C | 0x0D | 0x0E | 0x10 | unresolved |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| sites | 55 | 3 | 9 | 10 | 381 | 33 | 59 | 6 | 22 | 6 | 259 | 5 | 0 |
| argc | 1 | 1 | 3 | 2 | 3–20 | 1 | 1 | 2 | 1 | 2 | 1 | 1 | — |

- Connection management (sub-ops 1, 2, 3, 9, 13) is confined to `script.101` in each land, plus
  `hub/script.012` (sub-op 3). This is the dial/login/teleport script: it holds strings such as `Dialing`,
  `Net Connect`, `teleport`, `ReconnectToPrevious`, `hostaddr`, `landaddr.tim` and the modem/PAD error texts.
  CONFIRMED (strings).
- Sub-op 16 (GetLineRate) is used only in `script.097`, `script.905` and `script.908`.
- Dec-93 has the same counts except sub-op 4 (380 sites against 381). TSN 2.1 has the same sub-op set
  with fewer sites (sub-op 4: 342, sub-op 14: 204). The single-disk TSN "basic" set (hub only) uses the
  same sub-op set. CONFIRMED.
- The 11 hits the old scan could not resolve were 7 false hits (the two bytes `43 54` inside string items
  of `script.130`, hub `script.012` and `type31.628`) and 4 real sends whose argument lists contain
  branches or computed values. They resolve now and raise sub-op 4 from 377 to 381 and message code
  `0x25` from 51 to 54. The argc 20 sends are 12 sites: `script.050` (2), `script.142` and `script.145`
  per land. CONFIRMED (`tools/ktsn_callsites.py`).

## 10. Consequences for the ScummVM `kTsn`

- One kernel with a 17-row sub-op table, matching the export table one to one. Sub-ops 5, 6, 10 and 15,
  and any sub-op above 16, must return `s->r_acc` unchanged, as must 2, 7, 8, 9 and 14.
- Network input becomes `0x400` events, with the byte-array handle in `message` and the length in `modifiers`.
  Errors become `0x800` events with the code in `modifiers`. Both surface through `GetEvent`, and network
  events go to a separate queue that is read first when the mask contains `0x400`.
- The timer-driven Service and the foreground Poll collapse into one pump in ScummVM, run from the
  `GetEvent` path and from `TSN(8)`.
- Land switching becomes "remember `TSN(9)`'s name, end the game, restart in the land's directory with
  the shared block preserved". There is no `TSN.PRG` interpreter in the engine.

## 11. Open questions

- Encoding of the strings passed to Connect (`TSN(3)`) and SwitchHost (`TSN(13)`). They are built in
  `script.101` from the `prefix`, `modem`, `hostID` and `HOSTADDR` data. The decoded scripts
  (`tools/lsci_disasm.py`) can now be followed to find the grammar.
- Building the per-opcode message layout table for the server. `tools/lsci_disasm.py` already resolves the
  format string at each send (`pushID "bbw"`), so what remains is collecting the pairs.
- Which kernel and call the scripts use to free received byte arrays, and whether any script leaks them.
- Whether any other code path reprograms the timer after start-up. No script does (section 4.2).

## 12. Verification

Independent re-check by disassembling `LSCITV_inn_cd.EXE` with capstone (16-bit) and running
`tools/ktsn_callsites.py`, `tools/lsci_disasm.py` on `work/res`.

Confirmed unchanged:

- Kernel table at `DGROUP:1CE6`, 89 entries, with `TSN` = 0x54 -> `16B1:0004`. The name order is also
  anchored by `Array` -> `0BF4:0004`, `SetTimerFreq` -> `0CAB:0023` and `ConfigStr` -> `181C:08AA`.
  The `callk` dispatch (`09C8:052C`) indexes it with `n*4` after the `[1EC2]` bound check.
- The 17-way jump table and every case body (export offsets `+0` to `+40`, which sub-ops write the
  accumulator, the `1 = ok` inversion in sub-ops 3 and 13, the `ErrorEvent(0, code)` argument order).
  Jump-table words match in all three builds.
- The single `CD 14` (file `0x1A9AC`), the stub, first-use initialisation and shutdown.
- Send formatter codes `b w s a +`, sizing and fill passes, `Send(0:h, size)`, byte-array allocation
  (`ArrayNew` types, header layout) and the `0x02` flag setter (`18E5:04E5`).
- The pump, the `0x400` and `0x800` event layouts, queue routing, free-slot check, `GetEvent`
  ordering, the five-slot hook table, the timer-server table and IRQ0 handler, and the 43-entry DLL
  export table.
- TSNEXEC export order against int14h-api.md, `SetNextProgram` and the `DEFAULT` fallback.
- `ConfigStr` parsing rules (delimiters, case-sensitive compare, 200-byte cap, 40-byte key cap).
- The Feb-94 scan histogram (all rows), the Dec-93, TSN 2.1 and basic sets using the same sub-op set,
  and the message-code histogram (34 values).

Corrected in this pass:

- Script `ConfigStr` keys are now decoded rather than inferred. `dialScript`, `attachScript` and
  `LoginTimeout` are class names, not config keys, and many more keys exist (section 6).
- The tick rate is 60 Hz by default and no script changes it (section 4.2).
- Dec-93 differs by one sub-op 4 site, and the unresolved hits are gone (section 9).
- The poll-hook table has no duplicate check (section 4.1). The shared-block copy returns the stored
  length, not always 256 bytes (section 3).

Not independently reproduced: crash behaviour without TSNEXEC, the `-c` switch, the Service period
as seen on real hardware, and which kernel frees received arrays.

Note for `int14h-api.md` (the line on the sub-op to export mapping): it says sub-op 8 runs Poll, Receive and Service. The pump
(`16B1:0358`) calls Poll (`+20`) and Receive (`+14`) only. Service (`+28`) runs from the timer server.

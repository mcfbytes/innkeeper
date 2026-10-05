# TSNEXEC INT 14h API

The interface between every INN client program and the resident executive `TSNEXEC.EXE`: how a
client finds it, the 17 far-call services behind it, the com driver below it, and the `TSN.PRG`
program chaining around it.

Primary binary: `work/exe/TSNEXEC_inn_cd.EXE` (Feb-1994, LZEXE-unpacked, MD5 `21f6121b…`).
The Dec-1993 build `TSNEXEC_inn_v2317.EXE` is byte-identical. TSN 2.1 differs only in addresses (section 11).

## 0. Address convention

- `seg:off` is relative to the unpacked load image (segment `0000` = first paragraph after the MZ header).
  Ghidra loads the same image at `1000:0000`, so add `0x1000` to the segment to search there.
- TSNEXEC's MZ header is `0x190` bytes, so **file offset = `0x190` + seg×16 + off**.
- Code is segment `0000`, data is `031D` (file `0x3360`), and a 16-byte callback block sits at `031C:0000`
  (file `0x3350`).
- Client binaries use their own header size: LSCITV `0x30E0`, GOLF `0x1A00`, CGENN `0x3400`.
- "Export N" means entry N of the far-pointer table returned by INT 14h (section 2).

## 1. Discovery: INT 14h returns a table

| Claim | Evidence | Status |
|---|---|---|
| TSNEXEC hooks INT 14h with DOS `AX=3514h` then `AX=2514h`. It registers an exit handler that restores the old vector. | `0000:16A4` (file `0x1834`); restore `0000:16CC` (file `0x185C`) | CONFIRMED |
| The handler ignores every input register. It returns `DX:AX` = far pointer to the export table and does not chain to the BIOS. | `0000:16D8` (file `0x1868`): `mov ax,1660h / mov dx,<CS> / iret`. The `DX` immediate is a relocation at image `0x16DC`. | CONFIRMED |
| So there are **no AH sub-functions**: one `INT 14h` call finds the API, and every later call is a far call through the table. BIOS serial services are unavailable while TSNEXEC runs. | same | CONFIRMED |
| Every client links the same 20-byte stub: `mov ax,[p]; or ax,[p+2]; jz +1; retf; int 14h; mov [p],ax; mov [p+2],dx; retf`, which caches the table pointer in `p`. | LSCITV `178B:0012` (file `0x1A9A2`, `p`=DS:1EC4); GOLF `1328:0002` (file `0x14C82`, `p`=DS:1898); CGENN `172C:0006` (file `0x1A6C6`, `p`=DS:10F2); DARKSTRT image `0x15E62`; FATES image `0x165AA`; RB image `0x2FB46` | CONFIRMED |
| GOLF zeroes `DX` before the stub and checks the cached segment. If it is 0, GOLF prints "TSN Executive not loaded!" and exits with code 1. LSCITV and CGENN skip that check. | GOLF image `0x132CA`..`0x132EB` (file `0x14CCA`) | CONFIRMED |

## 2. Calling convention

- Exports are **far, cdecl** functions: the caller pushes arguments right to left, makes a far call, and
  pops its own arguments. Results come back in `AX` (16-bit) or `DX:AX` (far pointer). CONFIRMED
  from the `retf` endings and the caller `add sp,n` sequences in GOLF and CGENN.
- Every export saves the caller's `DS`, loads `DS=031D`, and restores `DS` before returning. `SI`, `DI` and `BP` are preserved. CONFIRMED (for example `0000:022F`).
- The table is 17 far pointers at `0000:1660` (file `0x17F0`). Each segment word is relocated (relocations
  `0x1662`..`0x16A2`, step 4). CONFIRMED.

## 3. Export table

`lock` means the export takes the reentrancy lock (section 5). When the lock is busy it does nothing and returns 0.

| # | Table offset | Entry (file) | Name used here | C signature | Result | Status |
|---|---|---|---|---|---|---|
| 0 | `+00` | `0000:2A4A` (`0x2BDA`) | GetStatus | `uint16 f(void)` | `AL` = driver function 0 (`MODEM.DRV` returns 3), `AH` = connection byte `DS:06D4` (`0x80` while connected, 0 otherwise) | CONFIRMED; AL meaning INFERRED (driver version) |
| 1 | `+04` | `0000:0273` (`0x403`) | GetSharedData | `uint16 f(void far * far *out)` | `*out` = `031D:013A`; returns stored length | CONFIRMED |
| 2 | `+08` | `0000:022F` (`0x3BF`) | SetSharedData | `void f(const void far *src, uint16 len)` | copies `min(len,256)` bytes to `031D:013A`. NULL or 0 clears the length. | CONFIRMED |
| 3 | `+0C` | `0000:2A60` → `0000:185F` (`0x19EF`) | Connect | `uint16 f(const char far *arg)` | 0 = connected or already connected; else an error code (section 6) | CONFIRMED; `arg` is a NUL-terminated string (at most 127 bytes) that driver function 3 copies to `CS:0576` and sends after `ATD` (`MODEM.DRV:0A6B`..`0AB2`), so it is the dial string; its exact grammar is INFERRED |
| 4 | `+10` | `0000:23D1` (`0x2561`) | Send | `uint16 f(void far *handle, uint16 len)` | 1 = queued; 0 = lock busy, `len==0`, queue full or offline | CONFIRMED, lock |
| 5 | `+14` | `0000:2455` (`0x25E5`) | Receive | `uint16 f(void far * far *outHandle)` | length of the oldest complete message plus its handle; 0 with `*outHandle=NULL` when empty. Ownership of the handle passes to the caller. | CONFIRMED, lock |
| 6 | `+18` | `0000:22CC` (`0x245C`) | SetAckTimeout | `uint32 far *f(uint16 ticks)` | stores the retransmit timeout (`DS:06F0`, default 300) and returns a pointer to TSNEXEC's tick counter `031D:06EC` | CONFIRMED |
| 7 | `+1C` | `0000:25A3` (`0x2733`) | Disconnect | `void f(void)` | driver function 7 (hang up). Frees link frames, clears the connection byte, and frees all queued messages through the free callback. | CONFIRMED |
| 8 | `+20` | `0000:24D9` (`0x2669`) | Poll | `uint16 f(void)` | runs Service. On status 0 it also turns received bytes into messages and frees sent handles. Returns status (section 6). | CONFIRMED, lock |
| 9 | `+24` | `0000:0290` (`0x420`) | SetNextProgram | `uint16 f(const char far *name)` | sets the `TSN.PRG` block to run after this child exits. An unknown name falls back to `DEFAULT`; NULL cancels. Always returns 1. | CONFIRMED |
| 10 | `+28` | `0000:2507` (`0x2697`) | Service | `uint16 f(void)` | link layer only: transmit, driver poll, receive frames, retransmit. Returns status. | CONFIRMED, lock |
| 11 | `+2C` | `0000:02DF` (`0x46F`) | GetPreviousProgram | `char far *f(void)` | name of the `TSN.PRG` block that ran before this one (`031D:0268`), or NULL | CONFIRMED |
| 12 | `+30` | `0000:22E5` (`0x2475`) | IsTransmitIdle | `uint16 f(void)` | 1 when no data frame is waiting for an ACK | CONFIRMED |
| 13 | `+34` | `0000:2A79` → `0000:197B` (`0x1B0B`) | SwitchHost | `uint16 f(const char far *arg)` | spins until it holds the lock. Flushes both queues, resets the receive ring and link state, then calls driver function 8. Returns 1 if not connected, 0 on success, else an error (section 6). | CONFIRMED; "switch X.25 host" INFERRED from driver strings (section 7.4) |
| 14 | `+38` | `0000:2531` (`0x26C1`) | Flush | `void f(void)` | marks the last queued message "flush after", packs pending messages, and closes the partly filled frame so it is sent now | CONFIRMED, lock |
| 15 | `+3C` | `0000:03C3` (`0x553`) | SetCallbacks | `void f(alloc, deref, free)` (3 far function pointers) | installs the memory callbacks (section 4); NULLs remove them | CONFIRMED |
| 16 | `+40` | `0000:2A55` (`0x2BE5`) | GetLineRate | `uint16 f(void)` | driver function 9; `MODEM.DRV` returns its state word `+1C` | CONFIRMED; "baud rate" INFERRED |

## 4. Memory callbacks (handles)

Clients pass **handles**, not data pointers. TSNEXEC turns a handle into a pointer only at the moment it
copies bytes, so the client's heap may move blocks between calls. That fits SCI hunk memory. CONFIRMED (mechanism), INFERRED (motive).

| Slot | Address (file) | Called as | Called from | Status |
|---|---|---|---|---|
| alloc | `031C:0004` (`0x3354`) | `void far *alloc(uint16 size)` | `0000:2C50`: a received message's length prefix is complete | CONFIRMED |
| deref | `031C:000C` (`0x335C`) | `void far *deref(void far *handle)` | `0000:264B` (packing to send), `0000:297B` (unpacking received) | CONFIRMED |
| free | `031C:0008` (`0x3358`) | `void free(void far *handle)` | `0000:23B9` (sent handles, during Poll), `0000:2DF3` (queue flush), `0000:2AFA`, `0000:2C81` | CONFIRMED |

- Callbacks are far cdecl calls made with **TSNEXEC's DS**, so each callback must load its own DS. CONFIRMED (`0000:0431`..`04CC` call through the slots without touching DS).
- After Send, the client must not free the handle. Once the message has been packed into frames (not
  necessarily acknowledged), its handle goes into a 100-entry deferred-free FIFO
  (`0000:2E1F` → `0000:2FEB`). The next Poll calls `free` on it (`0000:239D`). If the FIFO is full, the
  push fails silently, and that handle is never freed. The ring has 100 slots and so holds 99 handles. CONFIRMED (`0000:2E68` ignores the result of `0000:2FEB`).
- If alloc returns NULL, the message is not stored, and its payload bytes would be read as the next length prefix. CONFIRMED (no recovery path at `0000:2952`); stream desync INFERRED.
- `alloc` takes one 16-bit size and returns the handle in `DX:AX`. LSCITV returns a 16-bit handle with `DX=0` and reads only the low word of the handle in `deref` and `free` (`16B1:0B8E`..`0BE5`). TSNEXEC itself treats a handle as an opaque 32-bit value, and a NULL slot makes the wrapper return 0 (`0000:0431`..`04CC`). CONFIRMED.
- After each child program exits, TSNEXEC clears all three slots (`0000:040A`). LSCITV clears them itself on shutdown (image `0x16E46`). CONFIRMED.

## 5. Execution model

- **Polled, single-threaded.** TSNEXEC hooks no timer or serial interrupt. The driver's UART interrupt handler only fills ring buffers. All framing, ACKs, retransmission and message assembly happen inside the client's calls to Send, Poll, Service and Flush. CONFIRMED (only INT 14h is hooked; driver IRQ hook at `MODEM.DRV:09B3`).
- **Reentrancy lock** `031D:0710`, a test-and-set via `XCHG` at `0000:164D`. Exports 4, 5, 8, 10 and 14 return 0 without doing anything when the lock is held. Export 13 spins on it. Exports 0–3, 6, 7, 9, 11, 12, 15 and 16 ignore it. CONFIRMED. The lock suggests clients may call Poll from an interrupt handler. INFERRED.
- **Tick counter.** Nothing in TSNEXEC advances `031D:06EC`, so the client must keep writing its own 32-bit clock there through the pointer from SetAckTimeout. CONFIRMED (no writer in the image; GOLF image `0x1337E` and CGENN image `0x173E0` store their tick count there before every Poll; LSCITV copies a 32-bit variable at `DS:0B58` of its data segment, image `0x16E8A`).
  CGENN reads the BIOS tick count at `0040:006C` (image `0x17355`..`0x1735B`), so the DOS games run the counter at 18.2 Hz. CONFIRMED.
  LSCITV sets a timeout of 300 (image `0x16B42`); GOLF and CGENN set 90 (`0x5A`). Both are roughly 5 s, if LSCITV counts at 60 Hz. INFERRED.
- Between children, and during a `pause` in `TSN.PRG`, nobody advances the tick counter, so retransmit timeouts stop. INFERRED.

## 6. Status and error codes

Poll and Service (`0000:1A95`):

| Value | Meaning | Evidence | Status |
|---|---|---|---|
| 0 | OK, or not connected | `0000:1BAE` | CONFIRMED |
| 1 | carrier lost: driver online flag (state `+16`) cleared, connection byte reset | `0000:1AAB` | CONFIRMED |
| 2 | the outstanding frame's retry counter (`frame+8`) reaches 10 when a NAK arrives | `0000:2274` | CONFIRMED |
| 3 | a retransmit timeout finds the retry counter already above 10, so the 12th timeout | `0000:1B29` | CONFIRMED |

NAKs and timeouts increment the same per-frame counter (`0000:226C`/`2270` and `0000:1B25`), so the two failure codes count a mixed total. The counter starts at 0 for each new data frame (`0000:1CD9`).

GOLF prints one message per code from a table at DS:18A4: 1 "The connection to ImagiNation has been lost.", 2 "The phone line is too noisy…", 3 "ImagiNation is not acknowledging sends…". Any other code prints "INN Error %d". CONFIRMED (GOLF image `0x13375`, strings at file `0x1E25C`..`0x1E330`).

Connect (`0000:185F`) maps the driver's `AH` to a return code:
`01→0D 02→0C 04→0A 08→09 09→0F 0A→10 0B→11 0C→12 0D→13 10→08 11→17 20→06 40→07 80→05`.
Unlisted values return leftover arithmetic. CONFIRMED. The driver's `AL` is saved as the connection byte. CONFIRMED.

SwitchHost (`0000:197B`) maps `01→0D 02→15 0E→14 0F→16`. A value above `0F` is returned unchanged, and other values return leftover arithmetic. CONFIRMED.

## 7. Com driver (MODEM.DRV / NOBRK.DRV)

### 7.1 Loading

- `tsn.cfg` (or the `-c` file) line `comm = <file> : <params>` is found by the key lookup at `0000:04CC`, called from `0000:1772`. Lines that start with `;` are skipped. CONFIRMED.
- The file name is the first token. The driver is read whole into a newly allocated block at offset 0, and the entry point is `<seg>:0000`, stored at `031D:06CC`. CONFIRMED.
- The text after `:` is passed to driver function 1, which returns the state block pointer stored at `031D:06D0`. CONFIRMED.
- The installer writes `comm = %s : b%s c%d` or `comm = %s : b%s c%d i%s p%s`. Read as baud, COM number, IRQ and port. CONFIRMED (strings in unpacked `INSTTSN.EXE`); field meanings INFERRED.
- Errors: no `comm` key gives "INNExec: No com driver specified!"; a file that will not open gives "INNExec: Couldn't open com driver %s". Both exit with code 1. CONFIRMED.
- Shutdown (`0000:2378` → `0000:1C88`) hangs up unless a hang-up already ran (flag `031D:06D6`, set by `0000:1C15`), then calls driver function 2. CONFIRMED.

### 7.2 File header (Sierra driver format)

| Offset | MODEM.DRV bytes | Meaning | Status |
|---|---|---|---|
| `00` | `EB 61` | jump to entry stub at `0063` | CONFIRMED |
| `04` | `21 43 65 87` | Sierra driver magic `0x87654321`, same as `ADL.DRV`, `VGA320.DRV`, `IBMKBD.DRV` | CONFIRMED |
| `08` | `05` | driver class (0 video, 1 sound, 4 keyboard in the same set, so 5 = comms) | INFERRED |
| `09` | Pascal string `modem.drv`, then Pascal string description (59 bytes) | NOBRK.DRV carries the identical name and description | CONFIRMED |
| `4F` | 10 near word offsets | function table, indexes 0–9 | CONFIRMED |
| `63` | entry stub | `DS=ES=CS`, `call [bx*2+4F]` with `ES:SI` pushed as a far argument, `retf` | CONFIRMED |

**Entry convention:** `BX` = function index, `ES:SI` = far argument (or NULL), result in `AX` or `DX:AX`. The TSNEXEC wrapper is `0000:175E`. CONFIRMED.

### 7.3 Functions and which export uses them

| Fn | MODEM.DRV | Use (TSNEXEC caller) | Behaviour | Status |
|---|---|---|---|---|
| 0 | `08E2` | GetStatus (export 0) | returns 3 | CONFIRMED |
| 1 | `0914` | startup `0000:183C` | parse params, program UART, hook IRQ, return `DX:AX` = state block | CONFIRMED |
| 2 | `1351` | shutdown `0000:1C9A` | calls fn 7 (hang up), then two teardown routines (`13AB`, `132F`) | CONFIRMED; "restore IRQ vector and PIC mask" INFERRED |
| 3 | `0A3E` | Connect (export 3) | Hayes dial, then PAD dialogue: `AT…`, `ATD`, `CONNECT`/`NO CARRIER`/`BUSY`, `c `, `DIRECT`, ` CONNECTED` (`TERMINAL=` is never used) | CONFIRMED; step by step in `link-layer.md` section 4 |
| 4 | `124E` | after a frame is copied to the TX ring (`0000:1E4B`) | wait for THRE, enable UART interrupts (start transmit) | CONFIRMED |
| 5 | `126B` | every Service (`0000:1ABF`) | carrier watchdog: on DCD loss clear online flag (`+16`) | CONFIRMED |
| 6 | `128B` | not called by TSNEXEC | returns and clears two error bytes | CONFIRMED |
| 7 | `12D1` | Disconnect (`0000:1C27`) | if online: drop DTR ~1 s, send `+++`, `AT H0`, raise DTR | CONFIRMED |
| 8 | `0FC0` | SwitchHost (export 13) | PAD-level re-connect: escape to the PAD, `SET? 0:0,32:0`, `D`, `c <host>`, ` CONNECTED`, `DISCONNECTED` | CONFIRMED; `link-layer.md` section 5.1 |
| 9 | `08E6` | GetLineRate (export 16) | returns state word `+1C` (initialised to `0x4B0` = 1200) | CONFIRMED |

NOBRK.DRV has the same header and 10-entry table at different offsets
(`07D7 0809 1254 0933 1151 116E 118E 11D4 0EB5 07DB`), and is 253 bytes shorter. It drops the UART BREAK routine
(`MODEM.DRV:07D7`) and escapes to the PAD with a modem command string instead; the full difference is in
`docs/protocol/link-layer.md` section 2.5. CONFIRMED.

### 7.4 Driver state block (MODEM.DRV `CS:0099`, held at `031D:06D0`)

| Offset | Field | Owner | Status |
|---|---|---|---|
| `+00` far, `+04` size | transmit ring (`CS:02C2`, 0x200) | — | CONFIRMED |
| `+06` / `+08` | transmit write index (TSNEXEC, `0000:1E36`) / read index (driver ISR) | split | CONFIRMED |
| `+0A` far, `+0E` size | receive ring (`CS:00C2`, 0x200) | — | CONFIRMED |
| `+10` / `+12` | receive write index (ISR) / read index (TSNEXEC, `0000:1F04`) | split | CONFIRMED |
| `+16` | online flag (`0x80` while carrier is up; driver function 3 returns it in `AL`, which becomes the connection byte) | driver | CONFIRMED |
| `+18` / `+19` | link-layer expected receive sequence / next transmit sequence (0–7) | TSNEXEC | CONFIRMED |
| `+1A` / `+1C` / `+20` / `+21` | UART base port, line rate, PIC mask bit, interrupt vector | driver | INFERRED from use |

## 8. Above the driver: link and message layers

Wire format details belong to Track B2; this section records only what the API boundary depends on.

- **Frame** = `81` `crcLo` `crcHi` `ctrl` payload… `82`. CONFIRMED (builder `0000:1CA1`/`1CE8`/`1D3F`, receiver state machine `0000:1E7B`, states 0–5 dispatched at `0000:1F18`).
- Payload bytes `81`, `82` and `1B` are preceded by `1B`. The CRC and `ctrl` are positional and never escaped. CONFIRMED (`0000:2604`, receiver state 5).
- **CRC** is CRC-16/CCITT-FALSE (poly `0x1021`, init `0xFFFF`), computed over `ctrl` plus the escaped payload as sent and stored little-endian. CONFIRMED: the routine at `0000:16E0`, run on `"123456789"`, gives `0x29B1`.
- **ctrl**: data `0x00|seq`; NAK `0x80|seq`; ACK `0x90|seq`; sequence numbers 0–7. CONFIRMED (`0000:1CE8`, `0000:21F8`..`22B9`).
- **Recovery**: stop-and-wait, with one unacknowledged data frame. A bad CRC is answered with NAK. A duplicate is re-ACKed. A frame that arrives while the receive ring is full is not ACKed. A new `81` inside a frame triggers a NAK and a resync. CONFIRMED.
- **Frame pool**: 4 × 0x117 bytes (`0000:2B76`). The payload is packed while the escaped length stays at or below about 0xFE. CONFIRMED.
- **Message** = length prefix + body, carried on the in-order byte stream. The prefix is one byte when `len < 0xFF`, otherwise `FF lo hi`; prefix bytes are escaped too. Messages are packed back to back across frame boundaries. CONFIRMED (`0000:2604` sender, `0000:2872` receiver).
- **Queues**: send queue at `031D:0702` (100 slots) and receive queue at `031D:06F6` (250 slots). Each slot is 10 bytes: `+0` flags and next index (bit 0 free, bit 1 flush-after, `0x3FFF` = nil), `+2` handle, `+6` progress (`-1` = prefix not yet sent), `+8` length. CONFIRMED (`0000:2CBF`, `2D4C`, `2DB8`, `2E95`).
- **Receive byte ring**: size from the `inBufSize` key in `tsn.cfg`, minimum and default `0x400` (`0000:2F1E`). Pointers at `031D:072A`..`073C`. CONFIRMED.
- **Counters**: NAKs `031D:06E6`, timeouts `031D:06E8`, duplicates `031D:06EA`. CONFIRMED (increments); there is no export to read them.

## 9. Program chaining and TSN.PRG

### 9.1 Start-up (`0000:00DE`)

1. Command line: `tsnexec [-c<cfg>] [-p<prg>] program_name [commands]`. The defaults are `tsn.cfg` and `tsn.prg` (`031D:0056`/`005E`). The installer writes `tsnexec DEFAULT` into `INN.BAT`. CONFIRMED (`0000:02F8`; `INSTALL.SCR`).
2. Find `program_name` in `TSN.PRG`. If it is missing, print "INNExec: Unable to find program '%s'." and exit 1. CONFIRMED.
3. Load the com driver and allocate the queues (`0000:2334`), then hook INT 14h (`0000:16A4`). The driver is initialised but **not dialled**: dialling is the client's Connect call. CONFIRMED.
4. Run the block (`0000:05A0`), then loop while a child has called SetNextProgram (`031D:0248`). Before each run, copy current → previous (`031D:0286` → `0268`) and next → current (`024A` → `0286`). CONFIRMED (`0000:0193`..`0206`).
5. If a child exits with a nonzero `AX` from `INT 21h/4Dh` (return code plus termination type), print "INNExec: Error %d in %s.", wait for a key, and exit with that code. If no next program was set, shut down normally. CONFIRMED.

### 9.2 TSN.PRG language (interpreter `0000:05E6`)

The whole file is loaded on first use (`0000:09C0`). Tokens are separated by space or tab, keywords are case-insensitive (`0000:0ECB` folds case), and `;` starts a comment line. Keyword table at `031D:0438`. CONFIRMED.

| Statement | Effect | Status |
|---|---|---|
| `program <name>` | starts a block; the next `program` ends the current one | CONFIRMED |
| `cd <dir>` | `INT 21h/3Bh`; failure aborts the block with "Unable to run program" | CONFIRMED |
| `echo <text>` | prints the rest of the line | CONFIRMED |
| `pause <text>` | prints the text and "Press a key to continue.", **calling Poll** until a key arrives | CONFIRMED (`0000:06A9`) |
| `set NOLOAD_MUSIC TRUE\|FALSE` | sets a global flag; any word other than `TRUE` means false; the flag survives across programs | CONFIRMED |
| `IF NOLOAD_MUSIC` … `ELSE` … `END` | the only supported condition | CONFIRMED |
| anything else | `<path> <rest of line>` runs via `INT 21h/4B00h` (no PATH search, extension required). `$*` in the tail is replaced by the tsnexec `[commands]` text, for the first program only. Other `$` characters are copied literally. | CONFIRMED (`0000:0AF0`, `0000:1586`) |

- The working directory is saved before each block and restored after it, so `cd SL` affects only that block. CONFIRMED (`0000:05CB`/`05DA`).
- After every child: the callbacks are cleared, and the send and receive queues are flushed (`0000:090C`/`090F`). CONFIRMED.

### 9.3 State that survives across children

| Survives | Where | Status |
|---|---|---|
| modem connection, PAD session, link sequence numbers, any outstanding frame | driver and link state | CONFIRMED (nothing resets them between programs) |
| 256-byte shared block, used for identity and hand-off | `031D:013A`, length `031D:023A` | CONFIRMED |
| previous program name (the caller to return to) | `031D:0268` | CONFIRMED |
| `NOLOAD_MUSIC`, ACK timeout, tick-counter address | `031D:0466`, `06F0`, `06EC` | CONFIRMED |
| **lost**: callbacks, queued messages, a half-received message | `0000:040A`, `0000:237F` | CONFIRMED; the resulting stream desync is INFERRED, unless the host stays quiet during a switch |

Return-to-caller idiom: on start, GOLF and CGENN call GetPreviousProgram and pass the result straight to
SetNextProgram, so exiting the game returns to the land that launched it. When the previous program is NULL,
GOLF prints "Parent failure: application has been orphaned." CONFIRMED (GOLF image `0x13329`..`0x13362`,
CGENN image `0x17375`..`0x17391`).

Shared-block layout as used by the DOS games:

- word `+00`, word `+04`, and an 11-byte string at `+10` (GOLF image `0x134F5`..`0x13526`).
- the caller's parameters start at `+80`; GOLF writes back `0x80 + n` bytes to keep the first 128 (image `0x13565`).
- CONFIRMED offsets; "+00..+7F is the user identity block written by LSCITV" is INFERRED.

### 9.4 Config files

| File | Read by | Status |
|---|---|---|
| `tsn.cfg` (`-c`): keys `comm`, `inBufSize` | TSNEXEC (`0000:04CC`) | CONFIRMED |
| `TSN.PRG` (`-p`) | TSNEXEC | CONFIRMED |
| `LSCI.CFG` | LSCITV (string at file `0x259BA`), CGENN ("Bad or Missing LSCI.CFG."), Red Baron (`-C..\LSCI.CFG` in TSN.PRG). Never TSNEXEC. | CONFIRMED |
| `HOSTADDR` | LSCI scripts (`script.012`, `.055`, `.101` contain `hostaddr`/`HostAddr`). Never TSNEXEC. | CONFIRMED string presence; use INFERRED |
| `TSNVER` | No reference found in TSNEXEC, LSCITV or the Feb-94 scripts. `INSTALL.SCR` only copies it (line `copy %2:tsnver`). The file is 4 bytes `33 33 13 40`, which is the IEEE single 2.3. | CONFIRMED absence and content; reader unknown |

## 10. Cross-check against the clients

Table offsets each client calls, found from `les bx,[p]` followed by `lcall es:[bx+N]` (CONFIRMED):

| Client | Offsets used | Init sequence |
|---|---|---|
| LSCITV (`kTSN`, `16B1:0004`, file `0x19BF4`) | all 17 through a sub-op jump table at `16B1:0083` (file `0x19C73`) | stub → SetAckTimeout(300) → SetCallbacks(`16B1:0B8E`, `0BB8`, `0BE5`) |
| GOLF (`src\tsn\hostcomm.c`) | `04 08 10 14 18 20 24 28 2C 38 3C` | stub (DX=0, checked) → SetCallbacks → SetAckTimeout(90) → GetPreviousProgram → SetNextProgram |
| CGENN (`c:hostcomm.cpp`), DARKSTRT, FATES | `04 08 10 14 18 20 24 2C 30 38 3C` | stub → SetCallbacks → SetAckTimeout(90) → GetPreviousProgram → SetNextProgram |
| RB | `00 04 08 10 14 18 20 24 28 2C 30 38 3C 40` | same stub; the only DOS client that calls GetStatus or GetLineRate |

The offsets differ per client. Of GOLF and CGENN, only GOLF calls Service (`+28`, image `0x14507`) and only CGENN calls IsTransmitIdle (`+30`, image `0x18B1C`). Only LSCITV calls Connect (`+0C`), Disconnect (`+1C`) and SwitchHost (`+34`); the DOS games inherit a live connection. The common core of all DOS clients is `04 08 10 14 18 20 24 2C 38 3C`.

- **The LSCI `TSN` kernel sub-op number equals the export index.** Sub-ops 0–4, 7–9 and 11–16 call export N directly or through a small wrapper; 5, 6, 10 and 15 are no-ops for scripts. Sub-op 8 runs a loop of Poll (`+20`), Receive (`+14`) and Service (`+28`) that hands each message to the script layer (`16B1:0358`). CONFIRMED (jump table words `00A5 00B1 011E 0167 01C2 0318 0318 01D8 01FF 0206 0318 0232 0283 0290 02FA 0318 0306`, with `0318` = return).
- LSCITV sub-op 3 calls GetStatus first and returns 1 without dialling when `AH≠0` (image `0x16C7D`). CONFIRMED.
- The DOS games send the opcode as the first message byte (GOLF `0x32`, `0x29`, `0x24`, `0x07`) and treat any Send result other than 1 as "send failure %d". CONFIRMED (GOLF image `0x135F9`, `0x13696`, `0x13744`, `0x13872`).
- GOLF and CGENN call Poll then Flush as a pair, after first copying their clock into the tick counter. CONFIRMED (GOLF image `0x13435`, CGENN image `0x17460`).

## 11. Build differences

| Build | Handler | Table | Exports |
|---|---|---|---|
| INN Feb-94, INN Dec-93 (identical files) | `0000:16D8` | `0000:1660` | 17 |
| TSN 2.1 (Mar-93) | `0000:16B2` | `0000:163A` | 17, same order. Exports 1, 2, 9, 11 and 15 sit at the same offsets; the others move by `-0x26`. |

CONFIRMED for the Dec-93 build (same MD5) and for the TSN 2.1 handler, table and 17 entry points. For TSN 2.1 the first 48 bytes of every export match the Feb-94 export apart from address operands; function bodies were not compared one by one, so identical semantics is INFERRED. A single implementation is expected to serve every set.

## 12. Consequences for a replacement executive

- To serve unmodified DOS clients, a replacement only needs to answer INT 14h with `DX:AX` → 17 far cdecl entry points with the semantics above. The com driver and link layer are internal to TSNEXEC.
- The ScummVM side can treat the 17 exports as the `kTSN` sub-op set one-to-one. Poll, Receive and Send carry the payloads; messages are opaque, length-delimited byte strings.

## 13. Open questions

- Which LSCI script passes which SwitchHost address, and how a land name maps to an X.25 address. The Connect and SwitchHost argument grammar (`<prefix> t <host> ATD...`, host = 4 to 14 digits minus the first 4, or up to 14 letters, or `DIRECT`) is settled in `link-layer.md` sections 3.2 and 5.1.
- Who reads `TSNVER`, and what the shared block holds at `+00`, `+04` and `+10` (user ID, flags, handle?).
- Whether the host keeps quiet around program switches, avoiding the half-received-message loss in 9.3.

## 14. Verification

Second-pass check against the binaries (capstone disassembly of `TSNEXEC_inn_cd.EXE`, `MODEM.DRV`, `NOBRK.DRV` and the client images), made independently of the first pass.

Reproduced as written:

- INT 14h handler `0000:16D8`, hook `0000:16A4`, 17-entry relocated table at `0000:1660`, and the 17 entry offsets.
- Export prologues and results for exports 0 to 16, including argument order, `DS=031D` handling, the `XCHG` lock at `0000:164D` and which exports take it.
- Poll/Service structure (`0000:24D9`, `0000:2507`, `0000:1A95`), status codes 0 to 3, the Connect and SwitchHost error maps, and the Dec-93 MD5 match.
- Frame layout and control bytes (builders `0000:1CA1`/`1CE8`/`1D3F`, close `0000:1A46`), the CRC: a Python port of `0000:16E0` gives `0x29B1` for `"123456789"`, and the data-frame CRC covers `ctrl` plus the escaped payload (`0000:1A73`..`1A80`).
- Length prefix rules, the 0xFE payload limit, frame pool size (4 x 0x117), queue sizes (100 and 250 slots) and the `inBufSize` default of `0x400`.
- Driver header, 10-entry tables of `MODEM.DRV` and `NOBRK.DRV`, entry stub, TSNEXEC wrapper `0000:175E`, driver functions used (0 to 5, 7 to 9, not 6), state block layout and `0x4B0` initial rate.
- The identical 20-byte client stub in all six clients, the LSCITV `kTSN` jump table, and sub-op to export index equality (sub-ops 0 to 3, 7, 9, 11 to 14 and 16 call their own offset; 4 and 8 go through helpers that use `+10`, `+14`, `+20`, `+28`).
- Callback argument order (`0B8E` alloc, `0BB8` deref, `0BE5` free), `TSN.PRG` keywords, the child spawn (`INT 21h/4B00h`, `4Dh`), and the per-child reset (`0000:040A`, `0000:237F`).
- TSN 2.1: handler `0000:16B2`, table `0000:163A`, entries shifted by `-0x26` except exports 1, 2, 9, 11 and 15.

Corrected in this pass:

- Section 10: the first pass listed identical offsets for GOLF and CGENN. GOLF also calls Receive and Service, CGENN also calls Receive and IsTransmitIdle, and RB calls GetStatus and GetLineRate.
- Section 6: status 2 and 3 are driven by one shared retry counter, not by separate NAK and timeout counts.
- Section 7.1: shutdown hangs up unless a hang-up already ran, not "if still connected".
- Section 5: the DOS games take their clock from the BIOS tick count, which makes 18.2 Hz CONFIRMED.
- Section 3: the connection byte is `0x80` while connected; Connect's argument is the dial string sent after `ATD`.
- Section 4: the deferred-free FIFO holds 99 handles, and the handle width in LSCITV is 16 bits.
- Section 9.4: `TSNVER` is copied by `INSTALL.SCR` and holds the float 2.3.

The receiver state machine, the duplicate-frame counter (`0000:1FA8` increments `031D:06EA`), the driver function 3 and 8 dialogue order and the NOBRK.DRV difference were reproduced in the link-layer verification (`link-layer.md` section 9).

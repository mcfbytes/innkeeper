# INT 14h census: Shoppers Advantage

How `SHOPADV.EXE` uses the TSNEXEC export table, every message it builds on Send and Receive, and what a
host must do to serve it. Export numbers and semantics are in `docs/protocol/int14h-api.md`; the shared
command numbering is in `docs/protocol/messages.md`.

| Item | Value |
|---|---|
| Binary | `work/games/SHOPADV/SHOPADV_unpacked.EXE` (LZEXE-unpacked; the shipped `SHOPADV.EXE` is packed), Borland C++ 1991, Fastgraph V3.00 |
| `TSN.PRG` block | `program ShopAdv` = `cd shopadv` + `shopadv.exe` (no arguments; `work/raw/inn_cd/INN/TSN.PRG`) |
| MZ header | `0x19A0`; entry `0000:0000` (the header word `0x1C` is the relocation table offset); stack `23AA:0000`; data segment `174C` |
| Stub | `0000:0296` (file `0x1C36`), `int 14h` at `0000:02A0` (file `0x1C40`) |
| Cached table pointer | `DS:06DC` (file `0x1953C`) |
| TSN module | `tsn.cpp`, segment `05C9` (file `0x7630`..), class `Tsn`, one static instance at `DS:34EB` |

## 0. Method and conventions

- `seg:off` is relative to the load image; **file offset = `0x19A0` + seg×16 + off**. Ghidra (project
  `work/ghidra/census-shopadv/`) loads at `1000:0000`, so add `0x1000` to every segment (data is `274C`
  there). Decompiler output: `work/decomp/shopadv/shopadv2.c` (not committed).
- Call sites come from a capstone scan for `les bx,[DS:06DC]` + `lcall es:[bx+N]` (14 hits), checked against
  `tools/int14h_census.py` (section 12). Message formats come from disassembly of the one formatted-send
  routine and of every call to it.
- Ghidra lost several jump tables and the receive loop (`05C9:08FA`); those were read from disassembly.
- Layout notation follows `messages.md`: `b` byte, `w` LE word, `a[n]` raw bytes, `@n` byte offset. C→H is client
  to host, H→C host to client.

## 1. Summary

- CONFIRMED: one INT 14h site (the stub), reached from one place: `Tsn::init` (`05C9:0385`, call at `05C9:038A`).
  `DX` is zeroed first and the cached segment is tested afterwards, like GOLF. A missing executive leaves the
  pointer 0. Most wrappers test it and do nothing, but four sites do not (section 3.1): GetPreviousProgram in the
  constructor, the Service wrapper (its INT 1Ch hook is only installed when the pointer is set) and both Send wrappers.
- CONFIRMED: the program has call sites for **12 of the 17 exports**, plus one call at table offset **`+44`** that no TSNEXEC we
  have implements (`+44` of `TSNEXEC_inn_cd.EXE` is the INT 14h hook installer, bytes `B8 14 35 CD 21`).
  That call sits in a method nobody calls. The 4 bytes after the 17-entry table in all three TSNEXEC builds we
  hold (`inn_cd`, `inn_v2317`, `tsn21_031293`) are the first code bytes of the INT 14h hook installer
  (`B8 14 35 CD 21`), so a call through `+44` would be a far call to `CD35:14B8`, not a real export. CONFIRMED.
- CONFIRMED: **9 exports are live**: GetPreviousProgram, SetNextProgram, SetAckTimeout, SetCallbacks, Poll, Flush,
  Service, Send, Receive. Connect, SwitchHost, Disconnect and the `+44` call have wrappers but no caller.
  GetStatus, GetSharedData, SetSharedData, IsTransmitIdle and GetLineRate have no call site at all.
- CONFIRMED: **Service runs from an INT 1Ch (timer tick) interrupt handler** (`05C9:000C`). This confirms that the reentrancy
  lock serves an interrupt-time caller (`int14h-api.md` section 5 had it as INFERRED).
- CONFIRMED: the shared block is **never touched**. The program inherits a live X.25 session and identifies
  itself only by a three-word token that the host hands out in its first reply (section 7).
- CONFIRMED: every message the program sends is a `PMSMsg` (command 54 = `0x36`, the shopping mall gateway of
  `messages.md` section 3.3), except three unreferenced shared-object senders and one unreferenced Login (command
  53) sender in `Tsn` slot `+68` (section 6).
- INFERRED: the "third-party shopping service" is reached through a gateway on the INN host. The client is a
  plain text terminal for it (section 9).

## 2. The lookup stub and the cached pointer

| Claim | Evidence | Status |
|---|---|---|
| The stub is the standard 20 bytes: `mov ax,[p]; or ax,[p+2]; jz +1; retf; int 14h; mov [p],ax; mov [p+2],dx; retf`, with `p` = `DS:06DC`. | `0000:0296` (file `0x1C36`), bytes `A1 DC 06 0B 06 DE 06 74 01 CB CD 14 A3 DC 06 89 16 DE 06 CB` | CONFIRMED |
| Its only caller is `05C9:0385` (`Tsn` vtable slot `+2C`, "init"): `xor dx,dx; lcall 0000:0296; cmp [06DE],0; jne; clear both words`. | `05C9:0388`..`03A2` (file `0x79B5`) | CONFIRMED |
| With no executive the BIOS INT 14h runs with `DX=0`, so the segment word stays 0 and the init code clears the pointer. | same | CONFIRMED (code); BIOS behaviour INFERRED |
| `DS:06DC` is written only by the stub and by the clearing in init (`05C9:0396`, `039C`); the other readers are the 14 `les bx,[06DC]` sites and null tests. | byte scan of `DC 06` over the code segments | CONFIRMED |
| The stub is followed by a bare `retf` at `0000:02AA`, used as the empty "shutdown" slot (`Tsn` slot `+30`). | `05C9:03A4` calls `0000:02AA` | CONFIRMED |

## 3. Every call through the table

All 14 sites are in `tsn.cpp` (segment `05C9`). Each is `les bx,[DS:06DC]; lcall es:[bx+N]` inside a thin
`Tsn` method; the wrapper column is the method entry. "Slot" is the offset in the `Tsn` vtable at `DS:066C`
(file `0x194CC`). Slots `+04`..`+28` are the shared base class `Element` (`06FA`).

| Table off | Export | Site (file) | Wrapper (slot) | Called from | Live | Status |
|---|---|---|---|---|---|---|
| `+2C` | GetPreviousProgram | `05C9:028D` (`0x78BD`) | `Tsn::Tsn` `05C9:01BB` | constructor | yes | CONFIRMED |
| `+24` | SetNextProgram | `05C9:02A5` (`0x78D5`) | same | constructor | yes | CONFIRMED |
| `+18` | SetAckTimeout | `05C9:03CA` (`0x79FA`) | `05C9:03B7` (`+34`) | constructor, argument `0x5A` | yes | CONFIRMED |
| `+3C` | SetCallbacks | `05C9:03FB` (`0x7A2B`) | `05C9:03D9` (`+38`) | constructor (set), destructor (clear) | yes | CONFIRMED |
| `+20` | Poll | `05C9:04B5` (`0x7AE5`) | `05C9:04A5` (`+3C`) | pump (`05C9:0404`) | yes | CONFIRMED |
| `+38` | Flush | `05C9:052C` (`0x7B5C`) | `05C9:0505` (`+40`) | pump | yes | CONFIRMED |
| `+28` | Service | `05C9:057B` (`0x7BAB`) | `05C9:0574` (`+48`) | INT 1Ch handler `05C9:000C` | yes | CONFIRMED |
| `+10` | Send | `05C9:08C5` (`0x7EF5`) | `05C9:066A` (`+5C`), formatted send | 8 live sites and 1 dead (slot `+68`), section 6 | yes | CONFIRMED |
| `+10` | Send | `05C9:064B` (`0x7C7B`) | `05C9:061C` (`+60`), raw send | `0DDC:00DF` only when the mode word at `DS:3511` is nonzero | no | CONFIRMED; never nonzero |
| `+14` | Receive | `05C9:0986` (`0x7FB6`) | receive loop `05C9:08FA` (`+64`) | pump, twice per pass | yes | CONFIRMED |
| `+0C` | Connect | `05C9:05C6` (`0x7BF6`) | `05C9:05B0` (`+4C`) | none found | no | CONFIRMED |
| `+34` | SwitchHost | `05C9:05FC` (`0x7C2C`) | `05C9:05E6` (`+50`) | none found | no | CONFIRMED |
| `+1C` | Disconnect | `05C9:09AA` (`0x7FDA`) | `05C9:099A` (`+54`) | none found | no | CONFIRMED |
| `+44` | (unknown) | `05C9:09BC` (`0x7FEC`) | `05C9:09B0` (`+58`) | none found | no | CONFIRMED |

"None found" means: the singleton's address (`B8 EB 34` and every other encoding of `0x34EB`) occurs only in the
constructor and destructor stubs, the INT 1Ch handler (`05C9:001D`, calls the Service wrapper directly) and the nine
message-send calls. The framework loop (`15B0:000E`, image `05B0:000E`) calls slot `+00` of every element in its second
pass; its first pass hands flagged elements to the list object's own slot `+0C` and to `06FA:00C2`, which are not
`Tsn` slots. A byte scan for `FF /3` calls through `[bx/si/di/bp+disp]` with the displacements `+4C`, `+50`, `+54`,
`+58`, `+68` finds no call on a `Tsn` object: the `+4C`, `+50` and `+54` hits are on other objects (for example `0DDC:014C` on `DS:C479`),
because the singleton's address is loaded nowhere else. CONFIRMED.

Never called: `+00` GetStatus, `+04` GetSharedData, `+08` SetSharedData, `+30` IsTransmitIdle, `+40` GetLineRate.
`tools/int14h_census.py` lists some of these; those rows are C++ virtual calls (section 12). CONFIRMED.

### 3.1 Per export

| Export | Sites | Arguments (far cdecl) | How the result is used |
|---|---|---|---|
| SetAckTimeout `+18` | 1 | `ticks` = `0x5A` (90), the value GOLF and CGENN use; at 18.2 Hz about 5 s. | Returns the far pointer to TSNEXEC's tick counter `031D:06EC`. Stored in `Tsn+0x20/0x22`. Every Poll, Flush and Service wrapper then writes the low word of the BIOS tick (`0000:046C`, far pointer at `DS:02BE` and `DS:1226`) and 0 into the high word. CONFIRMED |
| SetCallbacks `+3C` | 2 | six words: alloc `05C9:0037`, deref `05C9:009E`, free `05C9:006A`. The destructor passes six zeros. | none. CONFIRMED |
| GetPreviousProgram `+2C` | 1 | none | `DX:AX` non-NULL is passed straight to SetNextProgram, so exit returns to the caller (the LSCI land). NULL: nothing is set and there is no "orphaned" message. The call is not guarded by the null-table test. Also unguarded: the Service wrapper `05C9:0574`, the raw and formatted Send wrappers (`05C9:061C`, `05C9:066A`); Poll, Flush, Connect, SwitchHost, Disconnect, `+44`, SetAckTimeout and SetCallbacks test the pointer. Without an executive the constructor therefore calls through `ES:BX = 0:0` (IVT entry `0Bh`) before `main`: INFERRED crash, which would make the "You must have the TSN executive" message (section 8) unreachable. CONFIRMED (code) |
| SetNextProgram `+24` | 1 | far string pointer | result ignored |
| Poll `+20` | 1 | none | `AX` stored at `Tsn+0x24`; nonzero calls the error handler (`05C9:0A2A`, section 8) |
| Flush `+38` | 1 | none | none; then the same error handler runs on the stale status |
| Service `+28` | 1 | none | status stored and handled as for Poll |
| Send `+10` | 2 | `(handle far ptr, len)` | **ignored**. A failed Send is dropped silently. CONFIRMED (`05C9:08C9` and `05C9:064F` discard `AX`) |
| Receive `+14` | 1 | `(far ptr to a far-pointer slot)` on the stack (`push ss; lea ax,[bp-4]`) | `AX` = length, 0 ends the loop; the slot is the message handle |
| Connect `+0C`, SwitchHost `+34`, Disconnect `+1C`, `+44` | 1 each | far string; far string; none; one word | dead; the `+44` wrapper stores its word at `Tsn+0x26` (`DS:3511`) |

## 4. Run-time sequence

1. **Static constructor** (`05C9:01BB`, start-up wrapper `05C9:0AB2`; the destructor wrapper is `05C9:0AC2`): init (stub) → SetAckTimeout(90) →
   SetCallbacks → install the timer hook (method slot `+44`, `05C9:0543`: saves INT 1Ch with `INT 21h/35h`
   and installs `05C9:000C` with `INT 21h/25h`) → GetPreviousProgram/SetNextProgram → append the object to the
   framework list `DS:3418`. CONFIRMED.
2. **Per framework iteration** (slot `+00`, `05C9:0404`; the main loop `15B0:000E` runs in a tight loop):
   Flush wrapper → Poll wrapper (which also runs the receive loop) → receive loop again → statistics. So the
   order is **Flush, Poll, Receive\*, Receive\***. Flush comes before Poll, the reverse of GOLF and CGENN.
   CONFIRMED.
3. **INT 1Ch handler** (`05C9:000C`): saves all registers, loads `DS=174C`, calls the Service wrapper, then
   chains to the saved vector with `pushf; lcall [DS:3513]`. Service therefore runs at 18.2 Hz even when the
   main loop is busy elsewhere (disk I/O; INFERRED). The tick counter is refreshed after Service in the
   wrapper, so it lags one tick. CONFIRMED.
4. **Receive loop** (`05C9:08FA`): runs only while the message queue at `DS:BEDF` has free slots (word at
   `DS:C013`, initial 64). For each message it adds to the byte and packet counters (`DS:02C8`..`02D2`), wraps
   the handle in a `Msg` object (`0D74:032C`, length at `+0x20`) and queues it; the queue hands each `Msg` to
   the handlers in `DS:BF03` (section 7). The handle is never freed by the program (`Msg` destructor only
   clears the pointer): INFERRED leak. CONFIRMED loop, INFERRED leak.
5. **Exit**: `exit(0)` runs the static destructor (`05C9:02CC`): restore INT 1Ch, SetCallbacks(0 x6). No
   Disconnect, no SetSharedData. TSNEXEC then runs the program that GetPreviousProgram returned. CONFIRMED.

Counters at `DS:02C4`..`02D2` (bytes and packets, out and in) are printed with a "Datalink Statistics" debug
routine that is an empty stub in this build (`17CB:02B2`, `17F6:001E` are `retf`). CONFIRMED.

## 5. Memory callbacks

| Slot | Address (file) | Behaviour | Status |
|---|---|---|---|
| alloc | `05C9:0037` (`0x7667`) | loads `DS=174C`, `operator new(size)` (`0570:0169`, size+2 with a 2-byte tag in front) with source tag `tsn.cpp` line `0x44`; on failure prints "Fatal Error: Out of memory in %s at line %d." (`0F31:000D`) | CONFIRMED; that it never returns NULL is INFERRED |
| deref | `05C9:009E` (`0x76CE`) | returns its argument unchanged: **handles are real far pointers** | CONFIRMED |
| free | `05C9:006A` (`0x769A`) | `operator delete` with tag `tsn.cpp` line `0x4E` | CONFIRMED |

Outgoing messages are allocated by the program (`tsn.cpp` line `0x14D`) with exactly the formatted length and are
freed by TSNEXEC through the callback after Send. Received handles are allocated by TSNEXEC through alloc.

## 6. The formatted send

`05C9:066A` (slot `+5C`) is `printf` for message bodies: `Send(handle, len)` of a heap block it builds from a
far format string and C-stack arguments. Two passes over the format (length, then copy); the length is what
Send receives. CONFIRMED (`05C9:066A`..`08D1`, tables at `05C9:08D2`/`08E6`).

| Char | Stack argument | Bytes written | Status |
|---|---|---|---|
| `b` | one word (int) | low byte | CONFIRMED |
| `w` | one word | 2, little-endian | CONFIRMED |
| `l` | two words | 4 | CONFIRMED |
| `a` | `int len` then far pointer | `len` raw bytes | CONFIRMED |
| `s` | far pointer | string with NUL | CONFIRMED; unused here (the length pass omits the NUL: a latent overrun) |

The 8 live call sites (all push the singleton `DS:34EB` as the object):

| Site | Format (`DS` offset) | Arguments left to right | Body bytes |
|---|---|---|---|
| `0DDC:0121` (file `0xF881`) | `bbwwwwwa` (`122A`) | `0x36, 0x18, 5, T0, T1, T2, 0xCCCC, strlen+1, text` | 12 + len |
| `0DDC:01E2` (`0xF942`) | `bbwwwww` (`1261`) | `0x36, 0x19, 5, 0xFFFF, 0xEEEE, 0xDDDD, 0xCCCC` | 12 |
| `0DDC:0347` (`0xFAA7`) | `bbwwwww` (`1297`) | `0x36, 0x1B, 5, 0xFFFF, 0xEEEE, 0xDDDD, 0xCCCC` | 12 |
| `0DDC:0976`, linear `0xE736` (`0x100D6`) | `bbwwwww` (`15B6`) | `0x36, 0x1A, 5, T0, T1, T2, 0xCCCC` | 12 |
| `0F39:0C9B`, linear `0x1002B` (`0x119CB`) | `bbwwwwwa` (`20B7`) | `0x36, 0x18, 5, T0, T1, T2, 0xCCCC, strlen+1, text` | 12 + len |
| `06FA:023C` (`0x8B7C`) | `wwwbbww` (`0735`) | `7, 0, this[0x1E], kind, 1, param, this[0x14]` | 12 |
| `06FA:0268` (`0x8BA8`) | `bww` (`073D`) | `0x1E, this[0x1C]` and **no third argument** | 5 |
| `06FA:02B7` (`0x8BF7`) | `wwwww` (`0741`) | `0x0D, this[0x1C], this[0x1C], v, w` | 10 |

A ninth caller exists but is dead: `Tsn` vtable slot `+68` (`05C9:09CB`, never called; no `FF /3 +68` and no direct
call in the image) sends `bbwbbbblbaa` (`DS:0658`, site `05C9:0A22`, file `0x8052`) = `b 0x35, b 0, w 0, b 1, b 2, b 3, b 0x0A,
l idLow/idHigh, b 1, a[11] password, a[strlen+1] name`. This is exactly the `Login` (command 53) layout of
`messages.md` section 4.2 with landType 1, version 2.3.10 and `fromFile` 1: the INN `Tsn` class SHOPADV links carries the
login sender, SHOPADV never uses it. CONFIRMED. The login Ack (`whichCmd` 22) is what the STATUS rule in section 7.2 reacts to.

`T0 T1 T2` are the three words at `DS:1220`, `1222`, `1224` (section 7). The `bww` call pushes two arguments for
three format characters, so its last word is whatever sits above the arguments on the stack (the caller's saved
BP). CONFIRMED by the `add sp,0Ch` at `06FA:026D`; the cause (a source bug) is INFERRED.

## 7. Message catalogue

### 7.1 C→H

All bodies start `36 sub | 05 00 | id words | tail`. This is `PMSMsg` (command 54); LSCI's own request is
`36 1B | 05 00 | FF FF | EE EE | DD DD | 00 00` (`hub/script.050 gotoCUCmall`, `messages.md` section 3.3).

| Cmd/sub | Name used here | Layout | Sent when | Status |
|---|---|---|---|---|
| `54/25` (`36 19`) | JOIN | `b 54, b 25, w 5, w FFFF, w EEEE, w DDDD, w CCCC` (12 bytes) | State 2 of the join machine (`0DDC:0184`), started right after the first modal dialog closes; again whenever the host sends `54/19` with kind `0x1B` | CONFIRMED layout; sequence CONFIRMED |
| `54/24` (`36 18`) | TEXT | `b 54, b 24, w 5, w T0, w T1, w T2, w CCCC, a[len] text` with `len = strlen+1` (NUL included) | One typed line when no earlier line is unanswered (`DS:C28B == 0`) and a line is waiting (`DS:C289 != 0`); also the three button commands `/EXIT\n`, `/TO MAIN\n`, `/TO STORE\n` | CONFIRMED layout. Typed lines end in **`\r\n`**, not `\n`: the edit handler `0F39:0B3B` appends `DS:209E` (`"\r\n"`) with `strcat` (`0000:3325`, call `0F39:0B93`); button commands end in `\n` only. CONFIRMED |
| `54/26` (`36 1A`) | EXIT | `b 54, b 26, w 5, w T0, w T1, w T2, w CCCC` | End of `main` (`0DDC:05D3`), once, whatever the reason for leaving | CONFIRMED |
| `54/27` (`36 1B`) | STATUS | `b 54, b 27, w 5, w FFFF, w EEEE, w DDDD, w CCCC` | Received `AckMsg` (command 0) whose byte@4 is `0x16` (the login Ack's `whichCmd` 22) | CONFIRMED trigger and layout; purpose INFERRED |
| `53` | Login (dead) | `b 53, b 0, w 0, b 1, b 2, b 3, b 10, l id, b 1, a[11] password, a[strlen+1] name` | `Tsn` slot `+68` (`05C9:09CB`), never called | layout CONFIRMED; unreachable CONFIRMED (scan) |
| `7` | joinNet | `w 7, w 0, w cookie, b kind, b 1, w param, w size` | base-class method `Element::joinNet` (slot `+14`) | layout CONFIRMED; reachable? INFERRED not |
| `30` | register | `b 30, w sid` (+ one stray word) | `Element` slot `+18` | CONFIRMED layout; INFERRED not reachable |
| `13` | setInt | `w 13, w sid, w sid, w prop, w value` | `Element` slot `+1C` | CONFIRMED layout; INFERRED not reachable |

The three shared-object senders match the JOINNET, `register` and `SetIntMsg` layouts in `messages.md` and exist because
the program links the INN `Element` class: 35 class vtables in the data segment inherit all three methods unchanged. No
virtual call through `+14`, `+18` or `+1C` (about 20 sites) was tied to an object whose vtable keeps the `Element`
method (several go through the second vtable at object `+0x24`, others call classes with their own override), so
unreachability is INFERRED, not proven.

`T0 T1 T2` start at 0 and are replaced by the three words of the host's `54/17` reply. The `5` is a constant in
every message; INFERRED to be a header word count (5 words including itself) or a protocol version.

### 7.2 H→C

`Msg` (`0D74:0449`) reads `b@0` as the command and `w@2` as a target id, looks the id up in the object table at
`DS:364E`, and for commands 0, 1, 8 and 13 also reads the fixed fields below. The main handler
(`0DDC:0268`, file `0xF9C8`) then dispatches on the command. Every message that reaches it is marked handled,
and a handled message decrements the outstanding-send counter `DS:C28B` if it is nonzero.

| Cmd/sub | Name | Layout read by the client | Client action | Status |
|---|---|---|---|---|
| `0` | AckMsg | `b@0, w@2 id, b@4 whichCmd, w@5, b@7` | `whichCmd == 0x16`: send STATUS. Otherwise nothing | CONFIRMED |
| `1` | NakMsg | same | nothing | CONFIRMED |
| `8`, `13` | ObjID, SetInt | `w@2`, `w@6`, `w@8` parsed by `Msg`; no handler in the main dispatch | marked handled, nothing else | CONFIRMED parse; use INFERRED nil |
| `48` (`30`) | Unsolicited | `b@0, b@1, text@2` | modal dialog "INN Notice:" | CONFIRMED |
| `54/16` (`36 10`) | OUTPUT | `b@12 kind, text@13` (NUL-terminated) | kind 1: append the text to the screen. Kind 2: modal "Mall Down", then quit with the error flag. Any other kind: modal "Mall Notice:" | CONFIRMED |
| `54/17` (`36 11`) | JOIN OK | `w@4 T0, w@6 T1, w@8 T2` | store the token; join machine to state 3 (idle) | CONFIRMED |
| `54/18` (`36 12`) | JOIN FAILURE | text@14 | modal "Server Join Failure", then quit with the error flag | CONFIRMED |
| `54/19` (`36 13`) | CONTROL | `b@12 kind` | kind `0x1A`: exit acknowledged (`DS:121E`). Kind `0x1B`: join machine to state 2, i.e. send JOIN again. Other kinds: nothing | CONFIRMED |
| `54/20` (`36 14`) | NAK | `b@12 kind = the client sub-command that failed`, text@15 | `0x18`: modal "Send Nak"; `0x19`: "Join Nak"; `0x1B`: "Status Nak"; each then quits with the error flag. `0x1A`: no dialog, **quit and exit acknowledged** | CONFIRMED |
| `149` (`95`) | raw text | whole body is text | only when the mode word `DS:3511` is nonzero, which never happens | CONFIRMED; dead |

"Quit" means `DS:121A += 5000`, which ends the main loop; "the error flag" is `DS:121C = 1`. Bytes 2 to 11 of the
`54/x` replies are not read except in `54/17`, so a host may copy them from the request, with two cautions. CONFIRMED:
`Msg` (`0D74:0449`) uses `w@2` unchecked as an index into the object table at `DS:364E` (4096 far-pointer slots, zeroed at
start, `0D74:04BE`), and a non-NULL entry's handler (slot `+0C`) runs before the main handler (`0DDC:0268`, entry check
at `0DDC:0275`); with the usual `5` the slot is empty (nothing registers objects: INFERRED), so keep `w@2 < 0x1000`.
The client never checks the body length (`Msg+0x20` is read only in raw mode), so every body must be long enough for
the fields in the table (14 bytes for `54/16` text, 13 for `54/19`). LSCI reads `w@10` as a
routing id for `54` (`messages.md` section 2.1) and sees the same `54/19` and `54/20`: `gotoCUCmall::handleMsg`
treats `54/19` as "go" (state 2 starts `ShopAdv` through `script.101` export 1) and `54/20` as "unavailable",
showing the text from byte 15 in a dialog " S.A. Unavailable ". CONFIRMED.

### 7.3 Peers

None. Nothing is addressed to another user; every message is between the client and the gateway.

## 8. Status and error handling

Poll, Flush and Service share the handler `05C9:0A2A`. A nonzero status is mapped through a 25-entry string
table at `DS:02E0` (file `0x19140`), posted as an event, and shown in the modal "Executive Error"
(`0DDC:0202`), which sets the quit and error flags. TSNEXEC returns only 1 to 3 from these exports; the table also
holds the Connect codes (`int14h-api.md` section 6).

| Code | Text | Code | Text | Code | Text |
|---|---|---|---|---|---|
| 1 | Connection lost. | 9 | No answer. | 17 | No novell workstation. |
| 2 | NAK count exceeded. | 10 | No carrier. | 18 | Novell workstation is not responding. |
| 3 | NAK timeout. | 11 | No packets received. | 19 | IPX setup problem. |
| 4 | Packet size exceeded. | 12 | No network prompt. | 20 | Reconnection is not supported. |
| 5 | Data send ready bit is not set. | 13 | No network connection established. | 21 | Reconnection prompt not found. |
| 6 | Carrier detect bit is stuck. | 14 | No connection established. | 22 | Network fallback. |
| 7 | Modem command rejection. | 15 | Name too short. | 23 | Keyboard interrupt. |
| 8 | Line is busy. | 16 | Name too long. | 24 | Zero length? |

CONFIRMED (strings at file `0x191A5`..). The Novell/IPX entries show the library also served an IPX transport
(INFERRED); TSNEXEC never returns them. With no executive, the first pump posts "You must have the TSN executive
loaded to execute this program." (`DS:05F0`) through the same event path. CONFIRMED.

## 9. Session flow

| Step | Client | Host must |
|---|---|---|
| 0 | LSCI `gotoCUCmall` sends `54/27`; on `54/19` it exits to `ShopAdv`, on `54/20` it shows the text from byte 15 | answer one of them |
| 1 | `main` shows "Please wait a few seconds while a connection is established" and waits for the dialog to close | nothing |
| 2 | JOIN (`54/25`) | answer `54/17` (token), `54/18` or `54/20` kind `0x19`. **There is no join timeout and no retry**: silence hangs the client |
| 3 | idle; user types or presses a button | send `54/16` kind 1 text when it has something to show |
| 4 | each typed TEXT line (`54/24`) | answer with at least one message within **636 ticks (about 35 s)** of the send (timer reset at `0DDC:00BA`); any handled message counts. Otherwise: modal "Send Time-Out", exit to the map. Button commands (`/EXIT`, `/TO ...`, sent by `0F39:0C2E`) neither start the timer nor raise the outstanding counter |
| 5a | user presses Exit (or Alt-E / Alt-X; INFERRED from scan codes `0x12`, `0x2D`): TEXT `/EXIT\n` is sent, the quit flag is set, EXIT (`54/26`) is sent | answer EXIT with `54/19` kind `0x1A` (or `54/20` kind `0x1A`). With no error flag the client waits **forever** for this |
| 5b | host-initiated end: `54/20` kind `0x1A`, or `54/16` kind 2 ("Mall Down"), or `54/18` | the client shows its goodbye (or error) dialog, sends EXIT and leaves; after an error it waits at most 90 ticks (about 5 s) for the acknowledgement |
| 6 | `exit(0)` | nothing; the link stays up and TSNEXEC runs the previous program |

The exit wait loop is `0DDC:05D3` near its end (`0DDC:097E`..`09D1`): it continues while a modal is open (`DS:BCA8 > 1`),
while the outstanding counter is nonzero, or while the exit is unacknowledged and (no error, or the 90-tick timer since
`0DDC:094C` has not elapsed). The error branch zeroes the outstanding counter first (`0DDC:0933`), the clean branch does not,
so a clean exit also needs every typed line answered. CONFIRMED.

Typed `/EXIT` is not recognised by the client: only the button path compares against `"/EXIT\n"`
(`0F39:0C2E`, button handler `0F39:0D35`). A typed line goes to the host, which must end the session itself with
`54/20` kind `0x1A`. CONFIRMED that the typed-line path `0F39:0B3B` compares nothing; INFERRED that no other
code does.

Dialog texts for the stand-in (CONFIRMED strings): "Thank you for using Shoppers Advantage-- now returning you
to the map" (title "Exiting"), "A network error has forced this session to end" ("Forced Exit"),
"The system has not responded to your last command; you will be returned to the map now, just in case the INN
server has malfunctioned" ("Send Time-Out"). The program prints two voice numbers in the first and last dialogs
(technical assistance, ordering); a stand-in should not reuse them.

## 10. Screen model for a stand-in

| Aspect | Finding | Status |
|---|---|---|
| Output | the OUTPUT text is written one character at a time by `09BE:292B`: `\r` is dropped, `\t` becomes four spaces, everything else goes to the character writer. No escape sequences are interpreted | CONFIRMED for `\r`, `\t`; INFERRED that `\n` starts a new line and no colour codes exist |
| Geometry | the text window carries `0x17` and `0x48` at `+46`/`+48`: 23 rows by 72 columns | INFERRED |
| Input | a one-line edit field; Enter queues the line; at most one line may wait while one is unanswered, otherwise the modal "Buffer Congestion" (" Please wait for your screen to update before typing more commands.") appears | CONFIRMED |
| Local commands | File menu: "Screen snapshot", "Begin text capture", "End text capture" append to `\CAPTURES.TXT`. Not network traffic | CONFIRMED |
| Buttons | " Exit " sends `/EXIT\n`; the place chooser (" Choose a place to go to: ") sends `/TO MAIN\n` (" Main Directory ") or `/TO STORE\n` (" Department Store "); " Cancel " sends nothing | CONFIRMED |
| Pacing | every wait in `main` pumps the framework loop, so Flush, Poll and Receive keep running under a modal dialog; Service also runs from INT 1Ch | CONFIRMED |

## 11. What innkeeper must provide

Responsibilities, in order of the session:

1. TSNEXEC contract: nothing beyond the 17 exports. Do **not** answer the `+44` call; a stand-in executive may
   return 0 there. Service must be safe to call from a timer interrupt while another export holds the lock.
2. Answer LSCI's `54/27` (`gotoCUCmall`) with `54/19` ("open") or `54/20` with text at byte 15 ("closed").
   A "closed" joke edition needs nothing else: LSCI shows the text and never starts `SHOPADV.EXE`.
3. On JOIN: reply `54/17` with a token (three words, any value; the client echoes it in every later message), or
   `54/20` kind `0x19` with text at byte 15.
4. Hold the session state per user: the token, the current "place" (main directory, department store).
5. Answer every TEXT with at least one `54/16` message in under 35 s. Kind 1 text is the screen; kind 3 or higher
   (any value but 1 and 2) is a modal "Mall Notice:".
6. Parse `/EXIT`, `/TO MAIN`, `/TO STORE` as commands: button commands end in `\n` plus NUL, typed lines end in `\r\n`
   plus NUL; everything else is free text for the service.
7. Answer EXIT (`54/26`) with `54/19` kind `0x1A`, or the client hangs. End a typed `/EXIT` with `54/20` kind `0x1A`.
8. Send nothing between `54/19` to LSCI and the JOIN from the new program: TSNEXEC drops callbacks and queued
   messages when a child exits (`int14h-api.md` section 9.3), and SHOPADV installs its callbacks only after it
   starts. INFERRED.
9. Do not send command 0 with byte@4 = `0x16` to SHOPADV unless a STATUS reply is wanted.
10. Link layer: SHOPADV never calls Connect, SwitchHost or Disconnect, never reads the shared block, and sends
    no keep-alive. The ACK timeout is 90 ticks (about 5 s) at 18.2 Hz.

Timers the host can rely on: 636 ticks (35 s) text answer limit; 90 ticks (5 s) error exit limit; no join
limit; no exit limit without an error.

## 12. Ghidra and tool outputs

| Output | Where |
|---|---|
| Ghidra project and scripts | `work/ghidra/census-shopadv/` (`DumpFuncs.java`, `AddFuncs.java`) |
| Decompiled C, 575 functions | `work/decomp/shopadv/shopadv2.c` (`shopadv.c` is the first pass, 557) |
| `tools/int14h_census.py` | 20 call rows. Of them, 13 are direct table calls and match section 3 (it also finds `+2C` GetPreviousProgram at `05C9:028D`, `+24`, `+18`, `+3C`, `+20`, `+38`, `+28`, `+0C`, `+34`, `+10` x2, `+14`, `+1C`). The `pointer-to-pointer` and `other` rows (`+2C`, `+34`, `+38`, `+3C`, `+40`, `+00`, `+04` at `05C9:021F`..`0942`) are C++ virtual calls on `this` or on the list at `DS:3418`. The table walk stops at `+40`, so the `+44` site `05C9:09BC` is not listed. CONFIRMED |

Ghidra's function list missed the receive loop and every vtable-only method until `AddFuncs.java` created functions at
the far-pointer targets found by relocation scan.

## 13. Open questions

- What export `+44` is. The wrapper stores its argument in a mode word; nonzero would switch the client to raw
  text in both directions (command `0x95`, no `54` envelope). A later TSNEXEC build or an IPX transport is the
  likely owner. Not implemented by any TSNEXEC we hold: in all three the table ends at `+43` and code follows.
- What command 0 with byte@4 `0x16` and the `STATUS` reply are for, and what the constant `5` and the tail
  `CCCC` mean.
- Whether the host sends the login Ack again after a program switch. The `STATUS` rule suggests it can.
- Whether `Element::joinNet`, `register` and `setInt` are ever reached.
- Exact text window size and newline handling (section 10).

## 14. Verification

Independent re-check (adversarial pass; Ghidra project `work/ghidra/verify-census-shopadv/`, byte scans and capstone
disassembly of `SHOPADV_unpacked.EXE`). Reproduced:

- The only `CD 14` is at `0000:02A0`; the stub is called once (`05C9:038A`); `DS:06DC`/`06DE` have no other reader or
  writer than the 14 call sites, the null tests and the init clear (byte scan of `DC 06`/`DE 06`, Ghidra xrefs agree). No
  second copy of the stub. All 14 table offsets, the 12 exports with sites and the 5 without are as in section 3.
- All message formats and constants (`DS:122A`, `1261`, `1297`, `15B6`, `20B7`, `0735`, `073D`, `0741`), the `Msg` field
  offsets, the command table (`0, 1, 0x30, 0x36, 0x95`), the `54/16..20` sub-tables and the dialog strings.
- Timers (636 and 90 ticks), the status string table, the INT 1Ch hook, the startup and exit table entries for the
  `Tsn` constructor and destructor (image `0x1A7EE` and `0x1A860`, file `0x1C18E` and `0x1C200`).

Corrected: entry point (`0000:0000`); typed lines end in `\r\n`; the formatted send has a ninth, dead caller (Login,
slot `+68`); unguarded wrappers (Service, both Send, GetPreviousProgram) and the constructor risk without an executive;
the `+44` explanation (bytes after the table, not an export); out-of-memory behaviour of alloc downgraded to INFERRED;
object-table index and body-length cautions; exit-wait detail; 636-tick timer base.

Not reproduced or still INFERRED: reachability of the three `Element` senders, the receive-handle leak, the text window
geometry, the Alt-E/Alt-X scan-code reading, and every host-side behaviour (no capture exists).

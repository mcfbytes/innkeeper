# INT 14h census: The Shadow of Yserbius

How the two Yserbius programs use the TSNEXEC export table, every message they build on Send and
Receive, and what a host must do to serve them. Export numbers and semantics are in
`docs/protocol/int14h-api.md`; the shared command numbering is in `docs/protocol/messages.md`.

| Program | `TSN.PRG` block | Binary | MZ header | Data segment | Stub | Cached table pointer |
|---|---|---|---|---|---|---|
| character generator | `Yserbius` | `work/games/YSERBIUS/CGENN.EXE` | `0x3400` | `23B5` | `172C:0006` (file `0x1A6C6`) | `DS:10F2` |
| game | `Yserba` | `work/games/YSERBIUS/DARKSTRT.EXE` (`darkstrt.cpp 3.10 09/07/93`) | `0x3800` | `1F02` | `15E6:0002` (file `0x19662`) | `DS:2328` |

## 0. Method and conventions

- `seg:off` is relative to the load image; **file offset = header + seg×16 + off**. Ghidra (project
  `work/ghidra/census-yserbius/`) loads at `1000:0000`, so add `0x1000` to the segment there.
  Full decompiler output: `work/decomp/yserbius/CGENN.c`, `DARKSTRT.c` (not committed).
- Call sites come from `tools/int14h_census.py` and were checked by a separate capstone scan for
  `les bx,[p]` + `lcall es:[bx+N]`; the cached pointer has no other readers or writers besides the stub.
  The tool also reports `+18` at `08C5:02B1` (DARKSTRT) and `0794:02A5` (CGENN): both are
  `mov bx,es:[bx]; lcall [bx+18h]`, a C++ virtual call, **not** a table call. CONFIRMED.
- `DARKSTRT.EXE` carries a Borland overlay area: `FBOV` at file `0x32870`, 159,776 bytes, about 50 overlays
  behind `CD 3F` thunks (stub segments from file `0x21500`). `CGENN.EXE` has none. No overlay code reaches the
  table: the pointer word `0x2328`, the stub and every `les bx,[2328]` lie in the resident image, and no far
  call in the whole overlay area names `149A`, `10F7` or `15E6`. The 50 overlays with a fix-up table (152,787 of
  the 159,776 bytes) only fix segment 0 and values up to `0x350`. CONFIRMED. Ghidra does not load overlays.
- Ghidra's decompile of the comms segment (`10F7`/`188C`) loses several jump tables; those parts were
  read from capstone disassembly.
- Message layouts use `messages.md` notation: `b` byte, `w` LE word, `s` NUL-terminated string,
  `@n` byte offset. C→H = client to host, H→C = host to client, C→C = relayed `Send` between clients.

## 1. Modules

Both programs link the same three Borland C++ modules (the source names are strings in both files).

| Module | DARKSTRT | CGENN | Role |
|---|---|---|---|
| `c:hostcomm.cpp` | `149A` (file `0x181A0`) | `172D` (file `0x1A6D0`) | export wrappers, one function per host command, receive dispatcher |
| `rpgcomms.cpp` + `rpgbbs.cpp` | `10F7` (file `0x14770`) | `188C` (file `0x1BCC0`) | player/land/map/party logic, game messages, bulletin board |

- `hostcomm.cpp` is the same code in both: the 19 Send wrappers sit in the same order and pass the same
  `__LINE__` values (`0x184`..`0x37D`) to the fatal-error routine. CONFIRMED.
- The comms module matches at 0.88 (capstone instruction sequences with operands masked). Both have the
  same 14-entry receive table, the same game message types 200–218, and the same hard-coded 8-byte key
  (section 5.1). CONFIRMED.
- CGENN calls only the start-up (`188C:0007`), shutdown (`188C:0280`), key (`188C:031F`, 8 callers) and the
  receive handlers (`188C:377C`..`3B31`) from outside the module; no relocated far call reaches the land,
  map, party or BBS code. CONFIRMED (relocated-call scan). The direct call graph from those entries
  (near and far calls, jump tables) reaches 9 functions for the first three entries: init, shared block,
  joinNet kind 1 (`188C:114B`), 41/2 (`188C:02C0`), ObjFree of the player and the dead 17/2. So CGENN at run
  time only creates and frees its player object and does the 41/2 lookup. CONFIRMED (indirect `vtable` calls
  not traced). The receive handlers do reach map-join and GrpDel code (`188C:0400`, `24AE`), so CGENN would
  join groups only if the host sent it an unsolicited 10, 11 or 2. CONFIRMED (graph), trigger INFERRED.

## 2. Export usage

| # | Export | DARKSTRT sites | CGENN sites | Wrapper (DARKSTRT / CGENN) | Arguments | Result use | Status |
|---|---|---|---|---|---|---|---|
| 1 | GetSharedData | 4 | 4 | `149A:01AD`, `01D0`, `01EF`, `02A1` / `172D:01C4`, `01E9`, `020A`, `02BC` | `&farPtr` | pointer deref'd: `01AD` → w@4 (**no caller**), `01D0` → w@0 (near call from the joinNet kind 1 wrapper `0861`), `01EF` → 7 words @0..@C (called once at start-up); `01EF` aborts with fatal "Application Orphaned." (`hostcomm.cpp` line `0x122`) when the returned length is 0, so the block must be non-empty; `02A1` is the dead SetSharedData path | CONFIRMED |
| 2 | SetSharedData | 1 | 1 | `149A:02A1` / `172D:02BC` | `(ptr,len)`: rebuilds the block from the current one plus the caller's bytes when they fit | none; **wrapper never called** | CONFIRMED |
| 4 | Send | 21 | 21 | 19 hostcomm wrappers (section 4.1), game send `10F7:19EC` / `188C:1BD8`, BBS send `10F7:33A3` / `188C:36DB` | `(handle, len)`; handle from the client's own allocator | `≠1` → fatal "TSN_NetSend() failed on command %d" with byte 0 | CONFIRMED |
| 5 | Receive | 1 | 1 | `149A:1073` / `172D:1165` | `&handle` | length 0 → return; else dispatch on byte 0 (section 4.2), then free the handle | CONFIRMED |
| 6 | SetAckTimeout | 1 | 1 | `149A:003C` / `172D:003E` | 90 | returned pointer kept; every Poll copies the BIOS tick (`0040:006C`) into it | CONFIRMED |
| 8 | Poll | 1 | 1 | `149A:0109` / `172D:010D` (6 / 7 far callers, 20 near callers) | – | 1 → "TSN Net Connect Failure." (code 101); any other nonzero → "TSN Net Maintenance Failure." (code = status); the wrapper returns after the dialog | CONFIRMED |
| 9 | SetNextProgram | 5 | 3 | init `149A:00BF`; exits `08C5:01CD`, `041A`, `0431`, `0448` / init `172D:00C1`; exit `0794:032C`, `033D` | previous-program name; `"YSERBIUS"`, `"DEFAULT"`, NULL / `"YSERBA"`, `"DEFAULT"` | ignored | CONFIRMED |
| 11 | GetPreviousProgram | 1 | 1 | `149A:00A7` / `172D:00A9` | – | NULL → "Application Orphaned." and exit 1; else passed to SetNextProgram | CONFIRMED |
| 12 | IsTransmitIdle | 2 | 2 | `10F7:01E9`, `0266` (sites `0246`, `0286`) / `188C:01F3`, `0280` (sites `0260`, `02A0`) | – | shutdown: loop Poll until idle or 60 s, then delay 72 or 20 ticks | CONFIRMED |
| 14 | Flush | 2 | 2 | `149A:0179` (Poll + Flush, **no caller**), `018A` (tick + Flush, 2 far and 20 near callers) / `172D:0190` (**no caller**), `01A1` | – | – | CONFIRMED |
| 15 | SetCallbacks | 1 | 1 | `149A:003C` / `172D:003E` | alloc `149A:1447`, deref `1497`, free `14AC` / `172D:1579`, `15CB`, `15E2` | – | CONFIRMED |

Never called: GetStatus, Connect, Disconnect, Service, SwitchHost, GetLineRate. The programs inherit the
connection and never log in. CONFIRMED. Totals: DARKSTRT 40 sites, CGENN 38.

Init sequence (both): stub → SetCallbacks → SetAckTimeout(90) → GetPreviousProgram → SetNextProgram.
CONFIRMED (`149A:003C`, file `0x181DC`). Every Send wrapper ends with Flush and then Poll, so each
message leaves at once. CONFIRMED (for example `149A:03D2`..`03D6`).

## 3. Shared block and program chaining

Neither program writes the shared block: the only SetSharedData wrapper has no caller. CONFIRMED.

| Offset | Read into (DARKSTRT / CGENN) | Use | Status |
|---|---|---|---|
| +0 user SID | `DS:19B7` / `DS:1473` | cookie of the player joinNet (section 4.1) | CONFIRMED |
| +2 userFlags | `19B9` / `1475` | bit 1 or 2 set → privileged flag `DS:A048`; BBS message text then starts at @0x16 instead of @0x12 | CONFIRMED; "privileged" INFERRED |
| +4 game-object SID | `19BB` / `1477` | read, never used | CONFIRMED |
| +6 land number | `19BD` / `1479` | `param` of the land joinNet | CONFIRMED |
| +8 land type (low byte) | `19BF` / `147B` | `landType` of the land joinNet | CONFIRMED |
| +A, +C | `19C1`, `19C3` / `147D`, `147F` | read, no use found | CONFIRMED read |

| Program | Start | Exit choice → next program | Status |
|---|---|---|---|
| CGENN (`0794:0001`, file `0xAD41`) | next = previous | character chosen (`DS:019E` = 100) → `"YSERBA"` (DARKSTRT); otherwise `"DEFAULT"` (INN hub) | CONFIRMED |
| DARKSTRT (`08C5:000C`, file `0xC45C`) | next = previous | menu `0x4B5` → `"DEFAULT"`; `0x4B7` → `"YSERBIUS"` (CGENN); `0x4B6` → NULL. Start-up failure → `"YSERBIUS"` when the character was refused ("off-line character above level 20"), else `"DEFAULT"` | CONFIRMED; menu meanings INFERRED |

Characters live only in the client's `MYCHARS.DAT` (CGENN reads and writes it; DARKSTRT reads it).
No message carries a full character record to the host. CONFIRMED by the absence of any such send;
"the host never stores characters" INFERRED.

## 4. Host command set

Header as in `messages.md`: `b command @0, b sub @1, w @2, w @4`. The host commands use the same numbers
as LSCI's object and service families.

### 4.1 C→H (hostcomm wrappers, DARKSTRT addresses; CGENN `172D` wrappers are in the same order)

| Cmd/sub | Wrapper (file) | Len | Layout | Called by | Meaning | Status |
|---|---|---|---|---|---|---|
| 7/0 kind 1 | `0819` (`0x189B9`) | 12 | `b 7, b 0, w 0, w cookie=shared+0, b 1, b 0x8B, w -1, w 1` | create player (`10F7:0FC9`) | joinNet: player object; `0x8B` = Yserbius land type | CONFIRMED layout |
| 7/3 kind 5 | `05DF` (`0x1877F`) | 12 | `b 7, b 3, w 0, w playerSID, b 5, b landType, w landNo, w 100` | land join (`10F7:0DF9`) | joinNet: the land group (≤100) | CONFIRMED |
| 7/0 kind 2 | `069D` (`0x1883D`) | 12 | `b 7, b 0, w 0, w playerSID, b 2, b 0x8B, w mapNo, w 104` | `10F7:0F67` | joinNet: the group for one map (≤104) | CONFIRMED; sharing INFERRED |
| 7/0 kind 4 | `075B` (`0x188FB`) | 12 | `b 7, b 0, w 0, w playerSID, b 4, b 0x8B, w -3, w 4` | `10F7:1023` | joinNet: personal party (≤4) | CONFIRMED |
| 9/0 | `08DC` (`0x18A7C`) | 4 | `b 9, b 0, w sid` | 5 sites | ObjFree (also sent for the shared map and land groups) | CONFIRMED |
| 10/0 | `0CA7` (`0x18E47`) | 6 | `b 10, b 0, w group, w member` | map join `03DC`, party join `108A` | GrpJoin | CONFIRMED |
| 10/3 | `0D42` (`0x18EE2`) | 9 | `b 10, b 3, w group, w member, b 1, b 1, b 7` | land join | GrpJoin with client version 1.1.7 | CONFIRMED; version meaning INFERRED |
| 11/0 | `0DF2` (`0x18F92`) | 6 | `b 11, b 0, w group, w member` | 6 sites | GrpDel (leave) | CONFIRMED |
| 40/4 | `03DD` (`0x1857D`) | n+3 | `b 40, b 4, s name` | `10F7:0C4E` | set display name (character name) | CONFIRMED layout; "character" INFERRED |
| 41/2 | `04AA` (`0x1864A`) | 9 | `b 41, b 2, w 0, w playerSID, b 0x8B, w 0` | `10F7:02A6`, `188C:02C0` | look up a service SID for land type 0x8B; resent every 60 s until answered | CONFIRMED layout and loop; meaning INFERRED |
| 17/2 | `0A4A` (`0x18BEA`) | 2n+6 | `b 17, b 2, w svcSID, w playerSID, w[n]` | `10F7:0303` | key request; dead: the key is compiled in (5.1) | CONFIRMED dead |
| 27/sub | `10F7:33A3` (`0x17B13`) | var | `b 27, b sub, w board @2, w playerSID @4, w boardId @6, body @8` | section 4.4 | bulletin board | CONFIRMED |
| 2/0 | `10F7:19EC` (`0x1615C`) | var | `b 2, b 0, w toSID, w fromSID, b msgType @6, body @7` | section 5 | relayed game message | CONFIRMED |

Linked but never called (dead in both programs, layouts CONFIRMED; no far call, near call or address-taking
reference in either image or in the DARKSTRT overlays): `0x32` `b 50, b sub, w, w` (`0340`);
36/2 host time `b 36, b 2` (`0552`); 13 setInt `b 13, b 0, w sid, w sid, {w prop, w val}…` (`0B22`);
14 setStr `b 14, b 0, w sid, w sid, w prop, s` (`0970`); 17/0 `b 17, b 0, w, w, w` (`0BF5`);
12 GrpMem (`0E8D`); 4 lock and 6 unlock `b 4|6, b 0, w, w` (`0F28`, `0FD3`).

### 4.2 H→C dispatch (`149A:1073`, file `0x19213`; table of 14 keys at `149A:12DD`, file `0x1947D`)

Anything else is freed silently. CONFIRMED.

| Cmd | Reads | Handler | Effect | Status |
|---|---|---|---|---|
| 0 Ack | b@4 whichCmd | `10F7:373D`, `3438` | whichCmd 4 → nothing; 27 → BBS reply (4.4), sub from b@1; other Acks ignored | CONFIRMED |
| 1 Nak | b@4, w@5 | sub-table `149A:12CD` (file `0x1946D`; keys 4, 6, 10, 27) | 4 → nothing; 6 → nothing; 10 → join error w@5 (4.3; 0 is stored as 7); 27 → "BBS Server offline." and the BIOS tick is advanced by `0x444`, which ends the current wait; other → "TSN NAK Error." dialog with whichCmd | CONFIRMED |
| 2 Send | w@4 from, b@6 msgType, body @7 | `10F7:34AC` (file `0x17C1C`) | game message, section 5 | CONFIRMED |
| 8 ObjID | w@6 sid, w@4 | `10F7:3642` | assigns the SID to the **pending** create kind (`DS:19A7`: 1 player → player record +0, 2 map group → +0xE, 3 personal party → +4, 5 land group → `DS:19DD`); the cookie is not checked. Kind 4 (`DS:19DB`) is never set as pending: `DS:19DB` is a copy of the land group taken at `10F7:0D9C` and used for the land leave and the GrpDel match | CONFIRMED |
| 9 ObjFree | w@2 | `3751` | ignored | CONFIRMED |
| 10 GrpJoin | w@2 group, w@4 who | `369C` | only `who == self` counts: sets "joined" (`DS:19A9`) for the pending group | CONFIRMED |
| 11 GrpDel | w@2 group, w@4 who | `36F9` | someone else left: map group → drop the remote player; party → " has left your party."; land group → drop from the name table | CONFIRMED |
| 12 GrpMem | w@2, list @6 | `3778` | ignored | CONFIRMED |
| 13 setInt | w@2, w@4, pairs @6 | `375B` | the handler reads the word at @0x10 (the value of the third pair) without checking the pair count; if it is in 12..48 it becomes the party-step delay in ticks (`DS:19C5`) | CONFIRMED; "delay" INFERRED |
| 14 setStr | w@2, w@4, w@6, @8 | `3756` | ignored | CONFIRMED |
| 27 BBS | b@1 sub | `3438` | 4.4 | CONFIRMED |
| 36 HostInfo | b@1 type 0 or 2 | `3747`, `374C` | ignored | CONFIRMED |
| 41 | w@6 | `377D` | stores the service SID (`DS:19DF`), ends the 41/2 loop | CONFIRMED |
| 48 Unsolicited | text @2 | `361B` | "::: MESSAGE FROM TSN :::" plus the text | CONFIRMED |

### 4.3 Join sequence and error codes (DARKSTRT; CGENN stops after step 2)

Each step waits for its reply with the 60-second timeout (`10F7:37AE`: more than `0x444` BIOS ticks).
CONFIRMED.

| Step | Sends | Waits for | On failure | Status |
|---|---|---|---|---|
| 1 `10F7:0FC9` | joinNet kind 1 | ObjID → player SID | "Unable To Create Player Object!" | CONFIRMED |
| 2 `10F7:02A6` | 41/2 | cmd 41 | resend every 60 s, forever | CONFIRMED |
| 3 `10F7:0DF9` | joinNet kind 5 | ObjID → land group | "Unable To Join Land!" | CONFIRMED |
| 4 | GrpJoin 10/3 into the land group | GrpJoin echo for self, or Nak 10 | Nak code w@5: 1 "Invalid Land Group.", 2 "Land Place Full.  Try another Place.", 3 "Invalid Land Object.", 4 "Land Group Locked.", 5 "No Rights to Land.", 6 "Incompatible Land Version." (table `10F7:0F5B`). A code of 0 (stored as 7) or above 6 shows no message, but the client still calls `0:7090` (the routine used after "Please press any key to exit.", so a key wait, INFERRED) before the join fails | CONFIRMED path; key wait INFERRED |
| 5 `10F7:1023` | joinNet kind 4, later GrpJoin 10/0 into it (`10F7:04C4` → `108A`) | ObjID → personal party; echo | "Unable To Create Personal Party!"; "TSN Code n" | CONFIRMED |
| 6 `10F7:0C4E` | 40/4 name, then game 217/1 to the land group | – | – | CONFIRMED |
| 7 `10F7:03DC` | joinNet kind 2 for the current map, GrpJoin 10/0 | ObjID, echo | "Unable To Create Map Group!", "Unable To Join Map!" | CONFIRMED |
| 8 | game 200/1 to the map group | – | – | CONFIRMED |

Moving to another map repeats step 7 after a GrpDel and an ObjFree of the old map group (`10F7:0489`).
Joining another player's party is GrpJoin 10/0 into that party's group (`10F7:108A`). A Nak there shows
"TSN Code 1".."TSN Code 6" (same w@5 codes). CONFIRMED. Shutdown (`10F7:01E9`, `0266`): GrpDel and ObjFree
for the party, personal party, map and land groups, then wait for IsTransmitIdle (≤60 s) plus 72 ticks,
then ObjFree for the player. CONFIRMED.

### 4.4 Bulletin board (`rpgbbs.cpp`, `10F7:2E1C`..`3438`)

| Sub | C→H body (len) | Reply (cmd 27, or Ack with whichCmd 27, sub at b@1) | Status |
|---|---|---|---|
| 13 list | `@2 = 0`, `@6 = param` (8) — `10F7:2EAC` | `w n @2`; n × 17-byte records @8 (`b`, `w`, `w board`, `w boardId`, `w count`); then n names (≤21 chars, `_` shown as space) | CONFIRMED |
| 14 index | `@2 board, @6 boardId`, @8..@F zero (27) — `30B6` | `w current @0x12`, then a word per message from @0x12 (low byte kept) | CONFIRMED layout |
| caller's sub (read) | `@2 board, @6 boardId, w msgNo @8` (10) — `31B7` | sub 1: `w @8`, `w @0xA`, then `author\|text` at @0x12 (@0x16 when privileged) | CONFIRMED |
| 0 post | `"name\|text\0"` @8, name ≤10, text ≤240 printable (309) — `32D5` | none awaited | CONFIRMED |

## 5. Game messages (C→C, relayed by the host)

All are `Send` (cmd 2) from the player SID, with `b msgType @6` and the body from @7, built by
`10F7:19EC`. The host only routes them. The receive switch is `10F7:34AC`; its 19-entry table is
`10F7:35F5` (file `0x17D65`); 202 and 203 are not handled. CONFIRMED. The player record (`DS:A1F0`,
0x28 bytes, also used for 104 remote players) holds: +0 player SID, +2 party SID, +4 personal party,
+6..+C party members, +E map group, +10 challenge peer, +12 leader pointer, +16 map, +1A/+1C position,
+1E..+20 encounter bytes, +27 in use. CONFIRMED offsets; the names for +16..+20 are INFERRED.

| Type | Builder (file) | To | Len | Body | Meaning | Status |
|---|---|---|---|---|---|---|
| 200 | `1167` (`0x158D7`) | map group or one player | 19+4n | `b kind @7` (1 hello, 2 answer, 3 update), `b n @8`, 5 party-flag bytes @9 + `05` @E (or zeros + `FF`), `w map @F, b @11, b @12`, n × `w sid, w partySid` @13 | party roster; a leader that receives kind 1 answers kind 2 to the sender | CONFIRMED layout; meaning INFERRED |
| 201 | `1252` | map group | 11 | `w map @7, b @9, b @A` | leader moved; followers copy it | CONFIRMED layout |
| 204 | `12FD` | party / player | 20 | `b kind @7, w sid @8, b @A, b @B, w member[4] @C` | member joined (" has joined your party.") | CONFIRMED |
| 205 | `136A` | party | 19+4n | `w rngIndex\|-1 @7, w members @9, b n @B, 3 encounter bytes @C, w map @F, b @11, b @12`, n × `w sid, b, b` @13 | leader snapshot: index into the shared 512-byte random table (`DS:17A6`), nearby players | CONFIRMED layout; "keeps rolls in step" INFERRED |
| 206 | `14DB` | party | 15 | 4 words @7 | party action or effect | CONFIRMED layout |
| 207 | `1541` | party, map, or one player | n+20 | `a[11] name @7, s text @0x12` | chat line | CONFIRMED |
| 208 / 209 | `16DB` / `25D5` | one player | 11 / 9 | `w @7, w 1 @9` / `w answer @7` | prompt and its answer (`10F7:09E0` waits) | CONFIRMED layout; meaning INFERRED |
| 210 | `1648` | party, map group | 14 | `b 1 leader\|2 member @7`, one flag byte @8+i, `b i @D` | set one party-flag byte | CONFIRMED |
| 212 / 211 | `15F8` / `2650` | a leader / the asker | 7 / 10 | – / `b status @7, w partySID @8` | join request; 0 OK, 1 "Joining another player.", 2 "In combat.", 3 "Not party leader.", 4 "Has a full party.", 5 "Refuses followers.", 6 | CONFIRMED |
| 213 / 214 | `1741` / `17A0`, `182F` | one player | 9 / 27 | `w partySID @7` / `w @7 (-1 = refuse), w @9, w @B, w @D, w @F, w @11, w member[4] @13` | challenge (player against player) | CONFIRMED layout; strings "Unable to Challenge" |
| 215 | `188B` | party, challenge peer | 15 | `w @7, w @9, w map @B, b @D, b @E` | combat move | CONFIRMED layout; meaning INFERRED |
| 216 | `18FF` | party, challenge peer | 35 | 28 raw bytes @7 | combat state block | CONFIRMED layout |
| 217 | `1971` | land group, or one player | 46 | `b kind @7` (1 hello → each receiver answers 2 to the sender; 3 update), 38-byte player record @8 (`w sid, a[11] name, …`) | who is in the land | CONFIRMED |
| 218 | – (never sent) | – | – | handled as "left party" plus 204 | – | CONFIRMED |

### 5.1 Shared key and random table

`10F7:0303` (file `0x14A73`) stores the constant key `12 2F 39 39 32 21 07 0C` in `DS:19CF`. It is never
zero, so the network path that would fetch it (17/2 and a wait) is unreachable. The key decodes the
512-byte table at `DS:17A6` that `10F7:3842` uses as a random sequence. CONFIRMED (same key at
`188C:0326` in CGENN). Combat, monsters and items are computed on each client from local data. The
party leader keeps the members in step with 200/201/205. INFERRED.

## 6. Who is authoritative

| Data | Owner | Evidence | Status |
|---|---|---|---|
| characters, inventory, level | client file `MYCHARS.DAT` | no upload message; level check is local | INFERRED |
| combat, monsters, random rolls | party leader's client (shared table + 205 index) | 5.1 | INFERRED |
| positions, party membership, roster | clients, broadcast as 200–206, 217 | section 5 | CONFIRMED messages |
| object SIDs, group membership, join limits, version gate | host | 4.3 Nak codes, ObjID | CONFIRMED (client side) |
| bulletin boards | host | 4.4 | CONFIRMED (client side) |
| chat | relayed | 207 | CONFIRMED |

## 7. Server responsibilities

1. Accept the inherited session: no Login, no Connect. Identity comes from the shared block that the
   hub wrote (user SID at +0, land at +6/+8).
2. joinNet: answer each with one ObjID (`b 8, …, w sid @6`) in request order; the client matches by
   order only. Kind 1 and 4 are new objects. Kind 5 (land type, land number) and kind 2 (map number)
   must return the **same** SID to every player in the same land (and map) so they meet. The capacities
   are 100, 104 and 4. INFERRED for sharing and scoping.
3. GrpJoin: echo `b 10, …, w group @2, w who @4` to the joiner, or Nak with whichCmd 10 and w@5 = 1..6.
   Check the version bytes 1.1.7 against the land's range (code 6).
4. GrpDel: tell the remaining members `b 11, w group @2, w who @4`, also when a connection drops.
   INFERRED for the drop case.
5. ObjFree on a shared group means "release", not "destroy", while other members remain. INFERRED.
6. Send: route cmd 2 to an object SID (its owner) or a group SID (every member). Pass the body through
   unchanged.
7. Answer 41/2 with cmd 41 carrying a SID at w@6 within 60 s. Without an answer the client resends
   forever.
8. Accept 40/4 (name). Optionally push 48 (operator text) and 13 (w@16 = party-step delay).
9. Bulletin board: keep boards and messages (persistent), answer 27/13, 27/14 and reads, and store posts.
   Nak 27 means "BBS Server offline".
10. Timing: every request must be answered within 60 s; the client's ACK timeout is 90 ticks (≈5 s).
11. Persist: bulletin-board messages only. Nothing else in these two programs needs host storage.
    INFERRED.

## 8. Open questions

- What 41/2 asks for, and what the SID returned at w@6 is.
- How the host scopes map groups (kind 2 carries only land type 0x8B and the map number).
- Whether the host echoes a group `Send` back to its sender.
- The meanings of 206, 208/209, 215 and 216, and the encounter bytes +1E..+20.
- The BBS directory record fields `b @0`, `w @1` and the purpose of the extra 4 bytes for privileged users.
- Whether Poll failures end the program: the wrapper returns after the dialog (`149A:0174`) and the dialog routine `1C76:07D2` calls exit only when its 0x600-byte buffer cannot be allocated (`1C76:0916`); whether it waits for a key is not traced.

## 9. Verification

Independent re-check (own Ghidra project `work/ghidra/verify-census-yserbius/`, own decompile in
`work/decomp/verify-yserbius/`, capstone scans). Result: the export usage, counts and message layouts stand;
corrections are folded into the sections above.

| Check | Method | Result | Status |
|---|---|---|---|
| Table call count | raw scan for `les bx,[ptr]` + `lcall es:[bx+N]` over the whole file, independent of the tool | DARKSTRT 40, CGENN 38; same per-export counts (offsets 4:4, 8:1, 0x10:21, 0x14:1, 0x18:1, 0x20:1, 0x24:5/3, 0x2C:1, 0x30:2, 0x38:2, 0x3C:1); no sites for 0, 0xC, 0x1C, 0x28, 0x34, 0x40 | CONFIRMED |
| Other pointer readers | every word equal to the pointer offset (and +2) in the file | only the stub and the 40 / 38 `les` sites; no `lds`, `lea` or copy | CONFIRMED |
| Other stubs | all `CD 14` bytes; far calls to the stub | one `CD 14` and one caller per program (`149A:004C`, `172D:004E`) | CONFIRMED |
| Overlays | `FBOV` parse, fix-up lists, raw far-call scan of the whole area | DARKSTRT only; overlay code never references the table, the wrappers or the comms module | CONFIRMED |
| Dead wrappers | far calls (overlap-safe scan), near `push cs / call`, address-taking | `01AD`, `02A1`, `0340`, `0552`, `0970`, `0B22`, `0BF5`, `0E8D`, `0F28`, `0FD3` and the Poll+Flush wrapper `0179` have no reference (CGENN equivalents likewise); `0179` and `0190` were new | CONFIRMED |
| Message layouts | Ghidra decompile and capstone of every wrapper, builder and the 14-key receive table | joinNet kinds 1, 2, 4, 5, 9/0, 10/0, 10/3, 11/0, 40/4, 41/2, 17/2, 27 and game messages 200..217 match the census, including lengths | CONFIRMED |
| Shared block map | pushes at `10F7:0016`..`003E` against `149A:01EF` | +0 → `19B7`, +2 → `19B9`, +4 → `19BB`, +6 → `19BD`, +8 → `19BF`, +A → `19C1`, +C → `19C3`; no reader of `19B7`, `19BB`, `19C1`, `19C3` except the copy | CONFIRMED |
| Next programs | strings at the `SetNextProgram` pushes | `YSERBA` / `DEFAULT` (CGENN), `YSERBIUS` / `DEFAULT` / NULL (DARKSTRT, menu `0x4B7` / `0x4B5` / `0x4B6`) | CONFIRMED |
| Tool defect | `far_call_targets` in `tools/int14h_census.py` | its `9A ..` regex consumes the opcode of a far call that directly follows a far call whose segment has low byte `9A` (every `149A` wrapper call): the caller lists miss, for example, `10F7:1A6F` and `10F7:342F` (Poll wrapper); the table-call list is unaffected | CONFIRMED |

Corrections to the first census: overlays (section 0), dead Flush wrapper, `GetSharedData` abort on an empty
block, Nak code 0, pending-kind table, setInt word position, CGENN reachability now by call graph. Not
reproduced and left open: the meaning of 41/2, map-group scoping, group-`Send` echo, host behaviour on Poll
failure. Not checked: the TSN.PRG block names (no `TSN.PRG` in the repository), the INN hub side.

# INT 14h census: The Fates of Twinion

How the two Twinion programs use the TSNEXEC export table, every message they build on Send and
Receive, what a host must do to serve them, and how they differ from Yserbius. Export semantics:
`docs/protocol/int14h-api.md`; command numbering: `docs/protocol/messages.md`; Yserbius:
`docs/protocol/int14h-census/yserbius.md`.

| Program | `TSN.PRG` block | Binary (MD5) | MZ header | DS | Stub (file) | Cached pointer |
|---|---|---|---|---|---|---|
| gallery (character select) | `Twinion` | `work/games/TWINION/TWGENN.EXE` (`7060fa0e…`) | `0x3200` | `23AE` | `1715:000A` (`0x1A35A`) | `DS:114E` |
| game | `Twina` | `work/games/TWINION/FATES.EXE` (`ace122c4…`, `twinion.cpp 2.30 09/16/93`) | `0x3A00` | `1F85` | `165A:000A` (`0x19FAA`) | `DS:1F12` |

Blocks: `work/sets/inn_v23/TSN.PRG` lines 50–58 (`cd twinion`, `twgenn.exe` / `fates.exe`). CONFIRMED.

## 0. Method and conventions

- `seg:off` is relative to the load image; **file = header + seg×16 + off**. Ghidra (project
  `work/ghidra/census-twinion/`) loads at `1000:0000`: add `0x1000` to the segment there. Decompiled C
  and listings: `work/decomp/twinion/{twgenn,fates}.{c,lst}` (not committed).
- **FATES is a Borland VROOMM overlay EXE** (`FBOV` at file `0x30BC0`, 51 overlay segments). Overlay
  fixups store a segment-table offset (index×8), not a paragraph. `work/decomp/twinion/flatten_fbov.py`
  rebuilds a plain MZ (`FATES_flat.EXE`) with every overlay as a resident segment so Ghidra and
  capstone see overlay callers; overlay addresses below are the original file offsets. CONFIRMED.
  DARKSTRT and GOLF are FBOV files too; none of the three has a table call inside an overlay (raw scan
  for the cached-pointer offset and `CD 14`). CONFIRMED.
- Table call sites: capstone scan for `les bx,[p]; lcall es:[bx+N]` (`work/decomp/twinion/sites.py`),
  cross-checked with `tools/int14h_census.py`. The tool also reports `+18` at `0794:02AB` (TWGENN) and
  `08C3:035C` (FATES): both are `mov bx,es:[bx]; lcall [bx+18h]`, a C++ virtual call (the application
  object's run method, vtable slot `+18`; TWGENN vtable `DS:08F5`, slot `08A5:0640`). Not a table call.
  CONFIRMED.
- Layouts: `b` byte, `w` LE word, `s` NUL string, `a[n]` bytes, `@n` offset. C→H client to host, H→C host
  to client, C→C a `Send` relayed between clients.

## 1. Modules

| Module | FATES (file) | TWGENN (file) | Role |
|---|---|---|---|
| `c:hostcomm.cpp` | `150F` (`0x18AF0`) | `1716` (`0x1A360`) | export wrappers, one function per host command, receive pump |
| `rpgcomms.cpp` + `rpgbbs.cpp` | `1151` (`0x14F10`) | `1861` (`0x1B810`) | land/map/party logic, game messages, bulletin board |
| stub | `165A` | `1715` | the 20-byte lookup stub and a bare `retf` (`+001E`, registered with `atexit`) |

- The two `hostcomm` segments are the same 0x14B9 bytes at the same offsets; every differing byte is an
  address operand (DS offset, segment, far target). CONFIRMED (byte diff). Offsets below are given once.
- TWGENN links the whole `rpgcomms` module but calls only start-up `1861:0009`, shutdown `1861:027F` and
  the key routine `1861:033A` from outside it; land, map, party and BBS entry points have no static
  caller. CONFIRMED (far-call scan). So the gallery only creates its player object, looks up the service
  (41/2, 17/2) and frees the player object. INFERRED.
- TWGENN also reaches `Poll` without a static call to `hostcomm 010D`: `08A5:016A` is a one-instruction
  thunk (`lcall 1716:010D`) stored in slot `+14` of the application vtable `DS:08F5`; the main loop
  `1FDA:02CA` ends with `1FDA:02A8` -> `1ED7:0145`, which calls slot `+14` of every object in the task
  list headed at `DS:5246`. So the gallery polls once per UI-loop pass. CONFIRMED (call chain and vtable
  words); that the application object is on the list at run time INFERRED (its base constructor `1ED7:0004`
  stores `this` in `DS:5246`). Receive is not run there. CONFIRMED.
- In FATES the land/party/BBS entry points are reached from overlay code (for example BBS list
  `1151:2EB2` from overlays `5564`, `642C`, `647B`). CONFIRMED (flattened image).

## 2. Export usage

| # | Export | FATES | TWGENN | Wrapper (`hostcomm` offset; FATES file / TWGENN file) | Arguments | Result use | Status |
|---|---|---|---|---|---|---|---|
| 1 | GetSharedData | 4 | 4 | `01B1` (`0x18CA1`/`0x1A511`), `01D4`, `01F3`, `0299` | `&farPtr` | `01B1` → w@4 (no caller); `01D4` → w@0 (cookie of the player joinNet); `01F3` → section 3; `0299` → length | CONFIRMED |
| 2 | SetSharedData | 1 | 1 | `0299` (`0x18D89`/`0x1A5F9`) | rebuilt block: current bytes up to `6 + strlen + 1 + strlen + 1`, then the caller's bytes (only when they fit in 256) | **wrapper has no caller** | CONFIRMED |
| 4 | Send | 21 | 21 | 19 `hostcomm` builders (4.1); game send `1151:1ADC` / `1861:1CEC`; BBS send `1151:3547` / `1861:38C0` | `(handle, len)` from the alloc callback | `≠1` → fatal "TSN_NetSend() failed on command %d" (byte 0), file/line dialog, exit 1 | CONFIRMED |
| 5 | Receive | 1 | 1 | pump `106B` (`0x19B5B`/`0x1B3CB`) | `&handle` | 0 → Flush and return; else dispatch on byte 0 (4.2), free, Flush; repeat while the argument is 1 | CONFIRMED |
| 6 | SetAckTimeout | 1 | 1 | init `0040` | 90 | pointer kept (`DS:D286` / `8DB2`); each Poll copies the BIOS tick `0040:006C` into it | CONFIRMED |
| 8 | Poll | 1 | 1 | `010D` (TWGENN also via the vtable thunk `08A5:016A`, section 1) | – | 1 → "TSN Net Connect Failure."; other ≠0 → "TSN Net Maintenance Failure." with the status; the dialog (`1CF7:07FC`, error ≠0) waits for a key, restores text mode and exits with that status | CONFIRMED |
| 9 | SetNextProgram | 6 | 3 | init `0040`; FATES `08C3:01A9`, `0273`, `04C3`, `04DA`, `04F1`; TWGENN `0794:032E`, `033F` | previous program; names in section 3 | ignored | CONFIRMED |
| 11 | GetPreviousProgram | 1 | 1 | init `0040` | – | NULL → "Application Orphaned." dialog, exit 1 | CONFIRMED |
| 12 | IsTransmitIdle | 2 | 2 | `1151:01E1`, `025E` / `1861:01F2`, `027F` | – | shutdown: Poll until idle or 60 s | CONFIRMED |
| 14 | Flush | 2 | 2 | `017D` (Poll then Flush), `018E` (tick then Flush) | – | – | CONFIRMED |
| 15 | SetCallbacks | 1 | 1 | init `0040` (`0x18B30`/`0x1A3A0`) | alloc `143F`, deref `148F`, free `14A4` | – | CONFIRMED |

Never called: GetStatus, Connect, Disconnect, Service, SwitchHost, GetLineRate. Totals: FATES 41 sites,
TWGENN 38. CONFIRMED.

- Init `0040`: pool of 0x2000 bytes (`0000:080A` / `0000:030F`, from farmalloc), stub, `atexit(165A:001E)` / `atexit(1715:001E)`
  (a bare `retf`), SetCallbacks, tick pointer = `0040:006C`, SetAckTimeout(90), GetPreviousProgram →
  SetNextProgram. CONFIRMED.
- Callbacks: alloc returns a 16-bit pool handle (`DX=0`); deref returns `poolSeg:*handle`; free
  releases the block (`0000:0715` in TWGENN). A failed alloc is fatal ("NewHandle cannot alloc %d bytes…"). CONFIRMED.
- Every builder sends, then calls `018E` (Flush) and `010D` (Poll), so each message leaves at once.
  CONFIRMED (for example `150F:03C9`..`03CD`).

## 3. Shared block and program chaining

Neither program writes the block (the only SetSharedData wrapper has no caller). CONFIRMED.
`01F3` takes seven out-pointers but fills six: it skips w@8. CONFIRMED (`150F:026D`..`0275`: two
`add [bp-8],2` without a store).

| Offset (LSCI meaning, `messages.md` 5.3) | FATES / TWGENN variable | Use | Status |
|---|---|---|---|
| +0 user SID | `DS:195E` / `14FE` | `01D4` reads it again as the `cookie` of the player joinNet | CONFIRMED |
| +2 userFlags | `1960` / `1500` | bit `0x02` or `0x04` → privileged flag `DS:888C` / `50D0`: BBS text starts at @0x16, not @0x12 | CONFIRMED; "privileged" INFERRED |
| +4 game-object SID | `1962` / `1502` | stored, never read | CONFIRMED |
| +6 land number | `1964` / `1504` (default 1) | `param` of the land joinNet | CONFIRMED |
| +8 land type | **not read**; `1966` / `1506` keeps its initial **8** | `landType` of the land joinNet | CONFIRMED |
| +A, +C | `1968`, `196A` / `1508`, `150A` | stored, never read | CONFIRMED |

| Program | Exit choice → SetNextProgram | Status |
|---|---|---|
| TWGENN (`0794:0007`, file `0xAB47`) | `DS:01DE` = 100 (Play) → `"TWINA"` (FATES); otherwise `"DEFAULT"` (hub). It first frees its player object (`1861:027F`). | CONFIRMED; "Play" from the gallery help text |
| FATES (`08C3:000C`, file `0xC63C`) | menu `0x4B5` → `"DEFAULT"`, `0x4B6` → NULL, `0x4B7` → `"Twinion"` (gallery). Network start-up failure → `"DEFAULT"` (`01A9`). Character refused ("off-line character above level 20") → `"Twinion"` (`0273`). | CONFIRMED; menu labels (MAP, DOS, GALLERY) INFERRED from "QUIT TO …" strings |

`TSN.PRG` names the blocks `Twinion` and `Twina`; the programs pass `"TWINA"`, so block lookup must be
case-insensitive. INFERRED.

## 4. Host command set

Header as in `messages.md`: `b command @0, b sub @1, w @2, w @4`.

### 4.1 C→H builders (`hostcomm`; `__LINE__` passed to the fatal routine in brackets)

| Cmd/sub | Offset (FATES file) | Len | Layout | Called by (FATES) | Meaning | Status |
|---|---|---|---|---|---|---|
| 7/0 kind=arg | `0811` (`0x19301`) [0x241] | 12 | `b 7, b 0, w 0, w cookie=shared+0, b 1, b 0x8B, w -1, w 1` | `1151:1053` | joinNet: player object | CONFIRMED |
| 41/2 | `04A2` (`0x18F92`) [0x1B2] | 9 | `b 41, b 2, w 0, w playerSID, b 0x8B, w 0` | `1151:029E` | service lookup; resent every 60 s until cmd 41 arrives | CONFIRMED layout and loop; meaning INFERRED |
| 17/2 | `0A42` (`0x19532`) [0x29B] | 2n+6 | `b 17, b 2, w svcSID, w playerSID, w 8, w 9, w 10, w 11` (props from `DS:196E` / TWGENN `150E`) | `1151:029E` right after the 41 reply (no wait); a second site `1151:0317`+`7F` (TWGENN `1861:033A`+`82`) is dead: it needs the compiled-in key word `DS:1976` to be 0, but the same routine stores it nonzero first | property request to the service object | CONFIRMED layout, live at `029E`; meaning INFERRED |
| 7/3 kind 5 | `05D7` (`0x190C7`) [0x1E2] | 12 | `b 7, b 3, w 0, w playerSID, b 5, b 8, w landNo, w 100` | land join `1151:0E81` | joinNet: land group (≤100) | CONFIRMED |
| 10/3 | `0D3A` (`0x1982A`) [0x30D] | 9 | `b 10, b 3, w landGroup, w playerSID, b 1, b 0, b 22` | `1151:0E81` | GrpJoin with client version 1.0.22 | CONFIRMED; version meaning INFERRED |
| 7/0 kind 4 | `0753` (`0x19243`) [0x21F] | 12 | `b 7, b 0, w 0, w playerSID, b 4, b 0x8B, w -3, w 4` | `1151:10AD` | joinNet: personal party (≤4) | CONFIRMED |
| 40/4 | `03D5` (`0x18EC5`) [0x199] | n+3 | `b 40, b 4, s name` (character record +2) | `1151:0CD6` | set display name | CONFIRMED |
| 7/0 kind 2 | `0695` (`0x19185`) [0x200] | 12 | `b 7, b 0, w 0, w playerSID, b 2, b 0x8B, w map (record +0x16), w 80` | `1151:0FF1` | joinNet: map group (≤80) | CONFIRMED |
| 10/0 | `0C9F` (`0x1978F`) [0x2F1] | 6 | `b 10, b 0, w group, w member` | map join `03F0`, party join `1114` | GrpJoin | CONFIRMED |
| 11/0 | `0DEA` (`0x198DA`) [0x324] | 6 | `b 11, b 0, w group, w member` | 6 sites | GrpDel | CONFIRMED |
| 9/0 | `08D4` (`0x193C4`) [0x256] | 4 | `b 9, b 0, w sid` | 5 sites, also for the shared map and land groups | ObjFree | CONFIRMED |
| 27/sub | `1151:3547` (`0x18457`) [rpgbbs 0x175] | var | `b 27, b sub, w @2, w playerSID @4, w @6, body @8` | 4.4 | bulletin board | CONFIRMED |
| 2/0 | `1151:1ADC` (`0x169EC`) [rpgcomms 0x23A] | var | `b 2, b 0, w toSID, w fromSID, w map @6, b @8, b @9, b msgType @0xA, body @0xB` | section 5 | relayed game message | CONFIRMED |

Linked, never called (layouts CONFIRMED): `0338` `b 0x32, b arg, w, w` (6); `054A` 36/2 host time;
`0968` 14 setStr `w sid, w sid, w prop, s`; `0B1A` 13 setInt `w sid, w sid, {w prop, w val}…`; `0BED`
17/0 `w, w, w`; `0E85` 12 GrpMem; `0F20` 4 lock; `0FCB` 6 unlock. `0FCB` never allocates: it
dereferences and sends an uninitialised handle (the same defect is in Yserbius `149A:0FD3`). CONFIRMED.

### 4.2 H→C dispatch (pump `106B`; 14 keys at `150F:12D5`, file `0x19DC5`)

Unlisted commands are freed silently. Handlers are in `1151` (FATES). CONFIRMED unless noted.

| Cmd | Reads | Handler | Effect |
|---|---|---|---|
| 0 Ack | b@4 | `38F1` (whichCmd 4, empty), `35DC` (27) | whichCmd 27 → BBS reply, which code from b@1 (4.4); others ignored |
| 1 Nak | b@4, w@5 | sub-table `150F:12C5` (file `0x19DB5`) | 4 → `38F6` (empty); 6 → nothing; 10 → error code w@5 into `DS:1D93` (0 becomes 7); 27 → "BBS Server is offline." and the BIOS tick is advanced by `0x444`, which ends the current 60 s wait; other → "TSN NAK Error." with whichCmd |
| 2 Send | w@4 from, b@0xA msgType | `3650` (file `0x18560`) | section 5 |
| 8 ObjID | w@6 sid, w@4 | `37F6` | gives the SID to the **pending** create (`DS:194F`): 1 player (record +0 and character record +0), 2 map group (+0xE), 3 personal party (+4), 4 `DS:1982` (no code sets pending 4: unreached), 5 land group `DS:1984`. The cookie is not checked. |
| 9 ObjFree | w@2 | `3905` | ignored |
| 10 GrpJoin | w@2 group, w@4 who | `3850` | only `who == self` counts: sets "joined" (`DS:1951`) when group matches the pending kind |
| 11 GrpDel | w@2 group, w@4 who | `38AD` | someone else left: map group → `1DCA`, party → `23CF` (" has left your party."), `DS:1982` → `2E12` (land roster; `0E21` copies the land group `DS:1984` into `DS:1982`) |
| 12 GrpMem | w@2, list @6 | `392C` | ignored |
| 13 setInt | w@2, w@4, pairs @6 | `390F` | w@16 (third pair's value) in 12..48 → step delay `DS:196C` (ticks) |
| 14 setStr | w@2, w@4, w@6, @8 | `390A` | ignored |
| 27 BBS | b@1 | `35DC` | 4.4 |
| 36 HostInfo | b@1 0 or 2 | `38FB`, `3900` | ignored (empty) |
| 41 | w@6 | `3931` | stores the service SID (`DS:1986`), ends the 41/2 loop |
| 48 Unsolicited | text @2 | `37CF` | "::: MESSAGE FROM TSN :::" and the text |

### 4.3 Join sequence (FATES; TWGENN stops after step 3)

Every wait ends after more than `0x444` BIOS ticks (≈60 s, `1151:3962`). CONFIRMED.

| Step | Code | Sends | Waits for | Failure | Status |
|---|---|---|---|---|---|
| 1 | `1151:0008` → `150F:0040`, `01F3` | – | – | GetPreviousProgram NULL → orphaned | CONFIRMED |
| 2 | `1053` | joinNet kind 1 | ObjID | "Create Player Object Failed!" | CONFIRMED |
| 3 | `029E` | 41/2 (again every 60 s), then 17/2 once | cmd 41 | none (loops) | CONFIRMED |
| 4 | `0E81` (`0x15D91`) | joinNet kind 5 | ObjID → `DS:1984` | "Unable To Join Land!" | CONFIRMED |
| 5 | `0E81` | 10/3 into the land group | GrpJoin echo for self, or Nak 10 | w@5: 1 "Invalid Land Group.", 2 "Land Place Full.  Try another Place.", 3 "Invalid Land Object.", 4 "Land Group Locked.", 5 "No Rights to Land.", 6 "Incompatible Land Version."; other (0 is stored as 7): no dialog, wait for a key, return failure (table `1151:0FE5`, file `0x15EF5`) | CONFIRMED |
| 6 | `10AD` | joinNet kind 4 | ObjID → personal party | "Create Personal Party Failed!" | CONFIRMED |
| 7 | `0CD6`, `0E21` | 40/4 name; game 217 kind 1 to the land group | – | – | CONFIRMED |
| 8 | `04D8` → `1114` | 10/0 into the own party (record +2) | echo, or Nak 10 | w@5: 1 "Invalid Group", 2 "Group Full", 3 "Invalid Object", 4 "Group Locked", 5 "No Land Rights", 6 "Incompatible Version" (table `1151:11E5`) | CONFIRMED |
| 9 | `03F0` → `0FF1` | joinNet kind 2 (map), 10/0 into it, game 200 kind 1 to the map group | ObjID, echo | "Create Map Group Failed!", "Unable To Join Map Group!" | CONFIRMED |

Shutdown `1151:01E1`: GrpDel from party and personal party, ObjFree personal party, map group (`049D`)
and land group (`0E50`), wait IsTransmitIdle (≤60 s), delay 72 ticks; `025E`: ObjFree player, wait
idle, delay 20 ticks. TWGENN runs only the second. CONFIRMED.

### 4.4 Bulletin board (`rpgbbs.cpp`, `1151:2EB2`..`35DC`)

The reply is cmd 27, or an Ack with whichCmd 27; both reach `35DC`, which switches on b@1. CONFIRMED.

| Request (builder) | Len | C→H | Reply b@1 → handler | Reply layout | Status |
|---|---|---|---|---|---|
| list (`3050`) | 8 | `b 27, b 13, w 0, w playerSID, w board` | 13 → `30B8` | `w n @2`; n × 17-byte records @8 (`b @+0, w @+1, w @+3, w @+5, w @+7`); then n names (≤21 chars, `_` → space, CR ends) | CONFIRMED |
| index (`325A`) | 27 | `b 27, b 14, w b, w playerSID, w a`, zero @8..@F, @0x10..@0x1A unset | 14 → `3300` | `w @0x12`; one byte per message kept from the words @0x12.. | CONFIRMED |
| read (`335B`) | 10 | `b 27, b op, w b, w playerSID, w a, w msgNo @8`; op 1, 4, 5 always, 2 only without flag `0x4000`, 3 only without `0x8000` | 1 → `33E1` | `w flags @8`, `w @0xA`, `author\|text` at @0x12 (@0x16 when privileged) | CONFIRMED; op meanings INFERRED |
| post (`3479`) | 309 | `b 27, b 0, w b, w playerSID, w a`, `"name\|text\0"` @8 (name ≤10, text ≤240 printable) | none awaited | – | CONFIRMED |

## 5. Game messages (C→C)

All are cmd 2 from the player SID built by `1151:1ADC`; bytes @6..@9 copy the sender's player record
(`DS:89D4`: +0x16 map word, +0x1A, +0x1C), msgType @0xA, body from @0xB. Receive switch `1151:3679`, 20-entry
table `1151:37A7` (file `0x186B7`); 202 and 203 are ignored. CONFIRMED. Every body is the Yserbius body
(`yserbius.md` section 5) moved by +4; lengths below include the 11-byte header.

| Type | Builder | Len | To | Handler | Meaning | Status |
|---|---|---|---|---|---|---|
| 200 `C8` | `11F1` | 23+4n | map group, one player | `1B95` (only when not following) | party roster; kind @0xB 1 hello, 2 answer, 3 update | CONFIRMED layout; meaning INFERRED |
| 201 `C9` | `12DC` | 15 | map group | `1CD7` | leader moved | CONFIRMED layout |
| 204 `CC` | `1387` | 24 | party | `1E97` | member joined | CONFIRMED |
| 205 `CD` | `13F4` | 23+4n | party | `21EB` | leader snapshot | CONFIRMED layout |
| 206 `CE` | `1565` | 19 | party | `262C` | party action | CONFIRMED layout |
| 207 `CF` | `15CB` | n+24 | party, map, player | `2647` | chat: `a[11] name @0xB, s text @0x16` | CONFIRMED |
| 208 / 209 `D0`/`D1` | `17CB` / `2713` | 15 / 13 | one player | `2713` answers 209; `277F` stores the answer (`DS:1950`) | prompt and answer | CONFIRMED layout |
| 210 `D2` | `16D2` | 18 | party, map | `2679` | set one party-flag byte; `b 1\|2 @0xB` | CONFIRMED |
| 212 / 211 `D4`/`D3` | `1682` / `278E` | 11 / 14 | leader / asker | `278E` replies; `298D` stores status (`DS:1958`) | join request; status @0xB: 0 OK (w party SID @0xC), 1 joining another, 2 in combat, 3 not leader, 4 full party, 5 refuses followers, 6 (`DS:C1C7` set), **7 asker's @6/@8 differ from the leader's** | CONFIRMED |
| 213 / 214 `D5`/`D6` | `1831` / `1890`, `191F` (refuse: w -1 @0xB) | 13 / 31 | one player | `2A03` / `2AF9` (`DS:195A`) | challenge | CONFIRMED layout |
| 215 `D7` | `197B` | 19 | party, challenge peer | `2CF7` | combat move | CONFIRMED layout |
| 216 `D8` | `19EF` | 39 | party, challenge peer | `2D5A` | combat state, 28 bytes | CONFIRMED layout |
| 217 `D9` | `1A61` | 50 | land group, one player | `2D73` answers kind 2 | who is in the land: `b kind @0xB`, 38-byte record @0xC | CONFIRMED |
| 218 `DA` | – | – | – | `23CF` + `1E97` | never sent | CONFIRMED |
| 219 `DB` | `1765`; live caller `278E` (near call `1151:2958`, after "joining your party..." on join status 0) sends it to the asker's SID; `0A36` (to the party, no static caller) is the other | 48 | one player (asker) | `26F8` | 32 bytes @0xF from `DS:C19F` (copy loop `0DA2:0211`), `b 0x20 @0x2F`; the receiver copies them into its own `DS:C19F` when @0x2F is `0x20`; @0xB..@0xE unset | CONFIRMED layout and live sender; use INFERRED (leader hands the joiner its block) |

## 6. Server responsibilities

1. Accept the inherited session: no Login, no Connect. Identity is shared-block +0 (cookie); the land is
   +6. +8 is ignored, so the land joinNet always says land type 8. CONFIRMED (client side).
2. Answer each joinNet with exactly one ObjID (`b 8, …, w sid @6`) in request order; the client matches by
   order, not cookie. Kind 1 and 4 are private objects. Kind 5 (type 8, land number) and kind 2 (type
   `0x8B`, map word) must give every player in the same land or map the **same** group SID. Capacities
   100, 80, 4. INFERRED for sharing.
3. GrpJoin: echo `b 10, w group @2, w who @4` to the joiner (and to members, INFERRED), or Nak with
   whichCmd 10 and w@5 = 1..6. Gate 10/3 on version 1.0.22 (code 6).
4. GrpDel: tell the remaining members `b 11, w group @2, w who @4`, also on a dropped connection
   (INFERRED). ObjFree of a shared group means "leave", not "destroy". INFERRED.
5. Route cmd 2 to an object SID or to every member of a group SID; pass bytes @6.. through unchanged
   (peers compare @6/@8 themselves).
6. Answer 41/2 with cmd 41, SID at w@6, within 60 s, or the client loops forever. Accept the 17/2 that
   follows; a setInt (13) to the player with w@16 in 12..48 sets the step delay. INFERRED pairing.
7. Accept 40/4. Optionally push 48 (operator text).
8. Bulletin board: store boards and messages persistently; answer 27/13 with 13, 27/14 with 14, reads
   (ops 1–5) with 1; store 27/0 posts; Nak 27 = offline. Reply b@1 must use these codes.
9. Timing: replies within 60 s; ACK timeout 90 ticks (≈5 s); at exit allow ≤60 s for the transmit queue.
10. The gallery (TWGENN) polls but only runs Receive inside its start-up and exit waits; messages sent
    to its player SID meanwhile sit in TSNEXEC's 250-slot queue and are flushed at the program switch.
    The host should send it nothing unsolicited. INFERRED.
11. Persist: bulletin-board content only. Characters stay in the client's `twinchar.dat`. INFERRED.

## 7. Differences from Yserbius (CGENN + DARKSTRT, compared byte for byte)

| Area | Yserbius | Twinion | Evidence | Status |
|---|---|---|---|---|
| `hostcomm.cpp` code | 19 builders, 14-key receive table, Nak keys 4/6/10/27 | identical (instruction streams match after operand masking, ratio 0.96 with DARKSTRT) | capstone diff | CONFIRMED |
| shared-block reader | DARKSTRT `149A:0273` stores w@8 → land type | `150F:026D` skips w@8; land type fixed 8 | disassembly | CONFIRMED |
| `__LINE__` of builders | `0x184`..`0x37D` | `0x180`..`0x370` (different file revision) | pushed constants | CONFIRMED |
| Send header | `b 2, b 0, w to, w from, b msgType @6` | 4 extra bytes: `w map @6, b @8, b @9`, msgType @0xA | `1151:1ADC` vs DARKSTRT `10F7:19EC` | CONFIRMED |
| game types | 200–218, switch of 19 | adds 219 `DB` (switch of 20) | tables `10F7:35F5` / `1151:37A7` | CONFIRMED |
| join reply 211 | status 0–6 | adds 7: location mismatch | `1151:27D3`..`27FE` | CONFIRMED |
| 41/2 | reply ends the step | Twinion also sends 17/2 with props 8–11 (live in both programs) | `1151:029E`, `1861:02BF` vs `10F7:02A6` | CONFIRMED |
| land joinNet | land type from the block (default 4) | constant 8 | `DS:19BF` vs `DS:1966` | CONFIRMED |
| land GrpJoin version | 1.1.7 | 1.0.22 | pushed bytes | CONFIRMED |
| map group size | 104 | 80 | joinNet kind 2 | CONFIRMED |
| party-join Nak text | "TSN Code 1..6" | "Invalid Group", "Group Full", … | strings | CONFIRMED |
| BBS Nak text | "BBS Server offline." | "BBS Server is offline." | strings | CONFIRMED |
| next programs | `YSERBA`/`DEFAULT`; `DEFAULT`/NULL/`YSERBIUS` | `TWINA`/`DEFAULT`; `DEFAULT`/NULL/`Twinion` | 3 | CONFIRMED |
| unchanged | init sequence, 41/2 loop, kinds 1/2/4/5 and `0x8B`, BBS layouts, setInt delay rule, 60 s waits, compiled-in key `12 2F 39 39 32 21 07 0C` | | | CONFIRMED |

## 8. Open questions

- What 41/2 and 17/2 ask for; which SID the host returns in cmd 41; whether 17/2 gets a reply.
- What +0x1A/+0x1C and the 32-byte block `DS:C19F` (type 219, sent by `278E` when a join is accepted) hold; whether `0A36` is reached indirectly.
- Whether the host fans a group `Send` back to its sender.

## 9. Verification

Independent re-check (own Ghidra project `work/ghidra/verify-census-twinion/`, output
`work/decomp/verify-twinion/`; capstone byte scans of the raw EXEs and of `FATES_flat.EXE`).

| Claim | Result | Evidence |
|---|---|---|
| One `INT 14h` and one stub per program | CONFIRMED | `CD 14` only at file `0x1A364` (TWGENN), `0x19FB4` (FATES) |
| Table pointer use is complete | CONFIRMED | every word `DS:114E` / `DS:1F12` in the whole file (overlays included): 40 / 43 hits = 2 stub stores + 38 / 41 `les bx,[p]; lcall es:[bx+N]`; the segment word has no other hit (FATES: one data table at `0x37F37`) |
| Counts and export indices (`N/4`) | CONFIRMED | FATES 41, TWGENN 38; per-export counts as in section 2; `+18` sites are C++ virtual calls |
| Unused builders and `SetSharedData` wrapper have no caller | CONFIRMED | no far call in the raw EXEs or in `FATES_flat.EXE` |
| `hostcomm` identical in both programs | CONFIRMED | 1864 instructions match shape and opcode; only operands differ |
| Shared-block reader skips w@8; land type 8; land number default 1 | CONFIRMED | `150F:01F3`; initial data `DS:1966`=8, `1964`=1 (TWGENN `1506`, `1504`) |
| Builder layouts, lengths, `__LINE__`, call sites (4.1) | CONFIRMED | decompile of all builders and their callers (GrpDel 6, ObjFree 5 sites in the flat image) |
| Receive table (14 keys), Nak sub-table, handlers (4.2), BBS (4.4), game table and lengths (5) | CONFIRMED | key words at `150F:12D5`, `12C5`; table `1151:37A7`; builder lengths |
| Corrected: 219 is sent | was "caller `0A36` has no static caller" | `278E` calls `1765` at `1151:2958` |
| Corrected: second 17/2 site | new, dead | `1151:0396` / TWGENN `1861:03BC` |
| Corrected: Poll dialog terminates | was open | `1CF7:07FC` ends in the exit routine with the status |
| Added: TWGENN polls via vtable thunk | new | section 1 |
| Not reproduced | UNCERTAIN | meaning of 41/2, 17/2 props, +0x1A/+0x1C, group-`Send` echo; whether the TWGENN application object is on the task list at run time; overlay caller list of `1151:2EB2` (flat image has 4: `0x46BF1`, `0x5452D`, `0x5489A`, `0x54C62`, not the three overlay segments named in section 1) |

FATES overlay code calls the pumps `1151:00FA` (Poll, Receive(1), `165B:0BAB`) at flat offsets `0x3C3BC`,
`0x3F891`, `0x43EA1`, `0x520F9` and `1151:0113` (Poll) at `0x343D8`, `0x3446D`, `0x344B2`, `0x344F0`.
TWGENN has no caller of its `010B` pump; its `0124` (Poll only) is reached only from the delay `0133` (near call at `1861:0151`). CONFIRMED.

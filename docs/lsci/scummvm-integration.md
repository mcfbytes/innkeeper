# LSCI in ScummVM: integration groundwork

How the ScummVM fork (`scummvm/`, branch `lsci`) recognises the LSCI clients, opens their resources,
selects LSCI's kernel table and loads LSCI scripts, and why it is done that way. The script loader
has its own document, `docs/lsci/script-loader.md`.
Original-software facts come from `docs/lsci/interpreter.md`, `docs/lsci/script-format.md`,
`docs/lsci/kernel-usage.md` and `docs/protocol/ktsn.md`; their labels are repeated here.

Status (2026-10-05): detection, resource loading for every land of the three builds, kernel-table
selection and script loading work. 72 of LSCI's 93 kernels are implemented or mapped, and `kTsn` runs on
a virtual TSNEXEC with two executives (section 10). With the offline one the Feb-1994 Clubhouse boots
headless through the title and Select Player screens to the Dialing screen, where the "no carrier"
result is shown as the game's Error #10 dialog; its OK button leads to the game's redial prompt
(section 7). With the online one, which speaks the INT 14h transport to `innkeeperd`, the persona from
the DOSBox setup logs on, reaches the Fall Map and enters the Clubhouse Waiting Room (section 7.1).

## 1. What the fork does now

| Area | Result | Where |
|---|---|---|
| Detection | `inn` (Feb-1994 and Dec-1993) and `tsn` (2.1) detected by MD5, `ADGF_UNSTABLE` | `engines/sci/detection_tables.h` |
| Compression | method 8 decompressed by the existing `DecompressorDCL` | `Resource::readResourceInfo` |
| Type-31 modules | loaded as scripts `0xE800 + n` | `ResourceManager::convertSci0ResId` |
| Lands | Clubhouse, SierraLand, CasinoLand from one game entry; `RESOURCE.002` shared | `ResourceManager::addAppropriateSources`, `engines/sci/resource/lsci_land.cpp` |
| "Is LSCI" | data-detected predicate `ResourceManager::isLsci()`; version stays `SCI_VERSION_1_EARLY` | `ResourceManager::detectLsci` |
| Kernels | LSCI's own 89 names plus 4 DLL kernels, mapped through an LSCI-only table | `engines/sci/engine/lsci_kernel_tables.h`, `Kernel::loadKernelNames` |
| Scripts | each item container becomes an SCI0 block image in `Script::load`; the unchanged SCI0/SCI1 script, object, VM and debugger code runs on it (`docs/lsci/script-loader.md`) | `engines/sci/engine/lsci_container.*`, `lsci_script_image.*`, `script.cpp` |
| TSN executive | `TsnExecutive` interface with TSNEXEC's 17 exports; `OfflineTsnExecutive`; `OnlineTsnExecutive`, which speaks the INT 14h transport to `innkeeperd` when the game option `lsci_host` is set; `TsnSession` for LSCITV's side (binding, poll hooks, timer server, event queues) | `engines/sci/engine/tsn_*`, `ktsn.cpp` (section 10) |
| LSCI kernels | LSCI-specific kernels and sub-op tables; built with SCI32, whose arrays and strings they use | `klsci.cpp`, `klsci_graphics.cpp`, `klsci_lists.cpp`, `klsci_file.cpp`, `lsci_array.*` (section 11) |
| Engine touch points | class lookup through `-dict-`, view rects in Rect objects, LSCI file modes, SID table and waiting network messages in the GC root set, object handles in `send`, `eq?` and `ne?`; each behind `isLsci()` | `object.cpp`, `selector.cpp`, `animate.cpp`, `file.cpp`, `gc.cpp`, `vm.cpp` (section 11) |
| Debugger | `screenshot` and `click x y [down\|up]` for headless runs | `console.cpp` (section 12) |

Every resource of every land loads and decompresses byte-identically to `tools/dcl.py` (section 8).

## 2. Detection

| Game ID | Variant | Files (MD5 of the first 5000 bytes, size) |
|---|---|---|
| `inn` | Feb 1994 (`work/ex/inn_cd`; same data on the V2.3 floppies) | `resource.map` b98c22f9d6e4759362b223f5b046c758, 2034; `resource.001` cde6597f4ed82125c8eb12a7f18582d5, 1142883; `resource.002` 174df4fa6035e68aef91d70247d61d6b, 356039 |
| `inn` | extra "Dec 1993" (`work/ex/inn_v2317`) | `resource.map` 803803ac7f5d28713225ef7140f1609c, 1902; `resource.001` 13a17664349c37932102986acd19e0a3, 1211546 |
| `tsn` | 2.1 (`work/ex/tsn21_031293`) | `resource.map` 8be9d51b3caf963d09bed3f117b9cb64, 1566; `resource.001` 4bfa2280a50382a976099ba5485ae671, 860593 |

- New `SciGameId` values `GID_INN` and `GID_TSN`, titles "The ImagiNation Network" and "The Sierra
  Network", `gameIdStrToEnum` rows with version `SCI_VERSION_1_EARLY` (it decides which render modes
  `VGA320.DRV` and `EGA640.DRV` in the game directory offer) and an empty Sierra ID, since the
  fallback detector cannot read an LSCI game object's name.
- Flags: `ADGF_UNSTABLE` as `docs/CONVENTIONS.md` requires; GUI options `GUIO_NOSPEECH` and
  `GUIO_NOLAUNCHLOAD`, because the INN interpreter disables `Save` and `Restore` (CONFIRMED,
  interpreter.md section 3.3) and an online client has nothing to restore.
- The entries list only the Clubhouse files in the game directory. The lands are needed at run time,
  not to identify the game, and listing `SL/` and `LL/` files would mean adding directory globs that
  every SCI game's detection would then scan.
- The fallback detector no longer crashes on LSCI data: `findGameObject` returns no object for LSCI, so
  `findSierraGameId` returns an empty ID and unknown builds (for example `tsn_basic`) are reported with
  their MD5s as before.

## 3. Lands: one game entry, several resource directories

Facts (CONFIRMED by data, `LAND.CFG` and `TSN.PRG` in every set; ktsn.md section 5): TSNEXEC runs each
land as a `TSN.PRG` program. `Sierr` starts LSCITV in the game directory, `SLand` and `LLand` do
`cd SL` / `cd LL` and run `..\lscitv.exe`. A script picks the next program with `TSN(9, name)` and ends;
TSNEXEC then starts it. In the Feb-1994 set the three maps place the same 18 resources in volume 2 at
the same offsets, and only the game directory holds `RESOURCE.002`; the Dec-1993 and TSN 2.1 maps use
volume 1 only (CONFIRMED by data, `tools/sci_res.py`).

Design:

- One detection entry per build. The engine chooses the land when it creates the `ResourceManager`:
  `ResourceManager::addAppropriateSources(landDirectory)` reads `<land>/resource.map` and
  `<land>/resource.0##`, then adds game-directory volumes whose numbers the land lacks. A volume is
  added once per map (`findVolume`), so a land's own copy wins over the shared file.
- The land table `s_lsciLands` maps program names to directories (`Sierr` → game directory, `SLand` →
  `SL`, `LLand` → `LL`). The program name is the key because it is what `TSN(9)` will hand over.
- Until `TSN(9)` is implemented, the configuration key `lsci_land` (a program name) selects the land at
  start-up; empty means the Clubhouse, as `TSN.PRG`'s `DEFAULT` program. An unknown name or a missing
  `resource.map` logs a warning and falls back to the Clubhouse.
- Land switching will follow ktsn.md section 10: remember the `TSN(9)` name and the hand-off block, end
  the game, restart the engine with the new land. This matches the original, where every land is a
  new interpreter process. Running different lands as separate ScummVM targets was rejected: they are
  one product sharing one connection and one hand-off block.
- A ScummVM `SearchMan` subdirectory, as used for language directories, was tried on paper and
  rejected: every directory's `RESOURCE.001` would match `resource.0##` and the volume chosen would
  depend on sort order.

## 4. Resources

| LSCI fact | ScummVM change | Why this way |
|---|---|---|
| Compression method 8 is PKWARE DCL (CONFIRMED, `tools/dcl.py` decodes all resources) | `kLsciCompressionDcl` maps to `kCompDCL`; `detectVolVersion` accepts 8 in SCI0 headers | 8 is not used by any SCI interpreter, so the mapping needs no condition; reuses `DecompressorDCL` |
| Type-31 modules; scripts name module `n` as script `0xE800 + n`, and the resource key is `type << 11 \| number`, so the two keys are the same 16 bits (CONFIRMED, script-format.md sections 6 and 9) | `convertSci0ResId` turns key type 31 into `ResourceId(kResourceTypeScript, 0xE800 \| n)`, used by `readResourceMapSCI0` and `readResourceInfo` | Class-table entries (`vocab.996`), `ModuleID` and `ModuleDispose` arguments then resolve with no translation layer, exactly as in LSCITV. No new `ResourceType` is needed |
| Views are SCI1 VGA (`03 80 ...`) although DCL-packed (CONFIRMED by data) | `detectSciVersion` skips the "DCL means SCI1.1 views" rule for LSCI | Otherwise `kViewVga11` would be chosen |
| The game object is export 0 of script 0, an item handle (CONFIRMED, interpreter.md section 5.3) | `findGameObject` returns `NULL_REG` for LSCI | The SCI0 export scan would `error()` on an item container |

Resource types seen: view, pic, script (including modules), text, sound, memory (SierraLand only),
vocab, font, patch, palette. All are existing ScummVM types.

## 5. Telling LSCI apart: `ResourceManager::isLsci()`

Options considered:

1. **A new `SciVersion` value.** The enum is ordered and the engine compares it 395 times in 76 files
   (`getSciVersion() <`, `<=`, `>=`, ...), plus the kernel signature ranges. Any position puts LSCI on
   one side of every comparison, which silently changes graphics, sound and VM paths. Rejected.
2. **`GameId` checks** (`GID_INN`, `GID_TSN`). They would spread `if (gameId == ...)` through the VM,
   which the conventions forbid, and they miss builds that are not in the detection table
   (`tsn_basic`) and the fallback detector, which has no engine instance. Rejected for behaviour; the
   IDs remain for script patches and workarounds.
3. **A data-detected predicate** (chosen). `ResourceManager::detectLsci()` is true when the map holds
   type-31 modules, which no SCI interpreter uses. It follows `isSci11Mac()`, the existing precedent
   for a format variant that the resource manager detects, and works in fallback detection too.

The version stays what ScummVM's detector derives from the data: an SCI0-format map with VGA views is
`SCI_VERSION_1_EARLY`. That matches the asset formats (SCI1 VGA views, `FE 02` pictures, SCI1 palette;
PLAN.md section 2a) and keeps the SCI1 graphics and sound code paths. Whether a later SCI1 sub-version
fits LSCI's graphics semantics better is open (section 9).

## 6. Kernel table

Facts: LSCITV dispatches `callk n` through a built-in table of 89 handlers, identical in all three
builds (CONFIRMED, interpreter.md section 3). `GRAPH256.DLL` and `NLNULL.DLL` append `Palette`, `Said`,
`Parse` and `SetSynonyms` at start-up (CONFIRMED, interpreter.md section 3.5); the numbers 0x59..0x5C
assume they load in that order (INFERRED), and scripts call exactly those numbers (kernel-usage.md).
There is no `vocab.999` (CONFIRMED).

- `Kernel::loadKernelNames` uses `s_lsciKernelNames` (89, CONFIRMED) followed by
  `s_lsciDllKernelNamesAssumed` (4, named so because the order is INFERRED). The SCI version
  adjustments are skipped for LSCI. ScummVM's reserved `ScummVMSleep` (0xE0) is kept for script
  patches; the ids in between are `Dummy`, which fails like LSCITV's "Kernel entry # too large".
- `Kernel::mapFunctions` looks names up in `s_lsciKernelMap` instead of `s_kernelMap`. A separate map
  is needed because several LSCI names collide with SCI names of different meaning (`Array`, `List`,
  `String` are SCI32 kernels; `Memory`, `Sound`, `FileSystem`, `Graph` have other sub-op tables), and
  the shared map would bind them, or abort on the version mismatch. The table lists only kernels with
  an implementation; `Kernel::mapFunctions` already binds every unlisted name to `kStub` and logs it
  as "unmapped" at start-up, so the log is the implementation status list.
- No version adjustment is needed after the LSCI names load: `loadKernelNames` has no case for
  `SCI_VERSION_1_EARLY`, the version detected for LSCI. A later sub-version choice must keep that true.
- `SciEngine::run` loads the LSCI table before `initGame`, because it needs no script-based feature
  detection; the debugger can therefore list it even if script loading fails.

Mapping rule: a kernel is mapped to an existing ScummVM function only when its result depends on its
integer arguments or on generic object queries, its argument counts in the scripts fit the ScummVM
signature (kernel-usage.md, Feb-94 counts), and SCI semantics are documented for the same name.
Kernels that read objects through LSCI property offsets (`vocab.994`), use LSCI sub-op numbering, or
draw through ports stay stubs until their semantics are traced.

| Status | Kernels |
|---|---|
| Mapped to the SCI kernel | `ObjectFree` (kDisposeClone), `ObjectRespondsTo` (kRespondsTo), `ModuleID` (kScriptID, CONFIRMED same semantics, script-format.md section 6), `GetFarText`, `DrawPic`, `GlobalToLocal`, `LocalToGlobal`, `MapKeyToDir`, `HaveMouse`, `GameIsRestarting`, `GetTime` (modes 1-3 encode like ScummVM's, kernel-usage.md), `Abs`, `CosDiv`, `CosMult` (kTimesCos), `GetAngle`, `GetDistance`, `Random`, `SinDiv`, `SinMult` (kTimesSin), `Joystick`, `Palette`, `GetPort`, `PicState` (kPicNotValid), `WindowDispose` (kDisposeWindow), `CelHigh`, `CelWide`, `Sound` (kDoSound) |
| Mapped through an LSCI sub-op table to SCI32 kernels | `Array`, `String`, `FileSystem`, `Graph` (rows that need LSCI arguments call an LSCI function) |
| LSCI functions (section 11) | `IsObject`, `ObjPropOffset`, `ObjOffsetProp`, `ObjectNew`, `Resource`, `GetEvent`, `SetCursor`, `ConfigStr`, `Animate`, `Display`, `TextSize`, `CelRect`, `DrawCel`, `Wait`, `SetPort`, `WindowNew`, `DrawControl`, `HiliteControl`, `EditControl`, `CanBeHere`, `OnControl`, `Block`, `ModuleDispose`, `NumLoops`, `NumCels`, `BaseRectSet`, `Memory`, `List`, `Seq`, `Long`, `SID`, `TSN` |
| `kEmpty` (LSCITV handler is a bare `retf`, CONFIRMED) | `CheckSaveGame`, `GetSaveDir`, `GetSaveFiles`, `DeviceInfo`, `Profiler`, `SetDebug`, `ShowFree`; also the DLL kernel `SetSynonyms`, which only the parser uses |
| `kStubNull` (LSCITV sets acc = 0, CONFIRMED) | `Debug` |
| `kStub` (unlisted) | `CoordPri`, `ShakeScreen`, `Show`, `AddToPic`, `DrawScaledCel`, `ScaledCelRect`, `Menu`, `Restart`, `Restore`, `Save`, `SetTimerFreq`, `Alert`, `Sqrt`, `StackUsage`, `Encrypt`, `Decrypt`, `ColorUp`, `ColorDown`, `InvokeMethod`, `Said`, `Parse` |

The LSCI rows live under `ENABLE_SCI32`. Without SCI32 the table keeps the plain `GetEvent` row and
the game stops at the first LSCI kernel, as before.

## 7. How far the game runs

Feb-1994 Clubhouse, offline executive, driven headless (section 12). Screens reached, each compared
with the DOSBox capture of the original (`work/dosbox/shots/`, online run):

| Screen | ScummVM (`work/scummvm-shots/`) | Original | Difference |
|---|---|---|---|
| Title | `inn-01-title.png` | `club_002.png` | none seen |
| Select Player, three personas | `inn-02-select-player.png` | `club_004.png` | persona colours: the `DrawCel` colour table is ignored |
| Dialing, after a click on Play | `inn-03-dialing-error-10.png` | `club_010.png` | the original is online and shows the Dialing picture; offline the game shows its "Error #10 There is no carrier" dialog over it |
| Redial prompt, after a click on OK | `inn-04-redial.png` | none (the original run was online) | "You've tried to call INN 2 times without logging on", with Redial and Quit INN buttons |

Kernels that stopped the boot, in the order they were met. The kernels of the first row were
implemented together, so their order was not recorded.

| # | Stop | Fix |
|---|---|---|
| 1 | `TSN(0)`; then the kernels implemented in the first batch, among them `Resource`, `Palette`, `SetCursor`, `ConfigStr`, `ObjectNew`, `Array`, `List`, `Seq`, `Memory` and `ModuleDispose` | `kTsn`, LSCI kernels and the SCI mappings |
| 2 | `NumCels` with (view, loop) | `kLsciNumCels`, `kLsciNumLoops` |
| 3 | `Str::indexOf` on a stub `String` result; `SID`; `Long` | `String` sub-op table on SCI32's kString functions; `kLsciSid`; `kLsciLong` |
| 4 | `Animate` given an LSCI list (an ID array); `CanBeHere` | `kLsciAnimate`; `kLsciCanBeHere` (LSCITV `0cdf:224c`) |
| 5 | `FileSystem`, `Block`, `GetPort`, `SetPort`, `Graph(10)` | `klsci_file.cpp`; `kLsciBlock`; mappings; `Graph` sub-op table |
| 6 | `TextSize`, `Display` with a `Str` object | `kLsciTextSize`, `kLsciDisplay` |
| 7 | assertion in `locateVarSelector` after `ObjectNew` of `PlayerRadioButton`, an instance used as a class | `Object::getClass` follows `-dict-` for LSCI |
| 8 | `CelRect`, `PicState`, `DrawCel`, `Wait` with a mode, `SetSynonyms` | `kLsciCelRect`, `kPicNotValid`, `kLsciDrawCel`, `kLsciWait`, `kEmpty` |
| 9 | `GlobalToLocal` | mapping |
| 10 | `Display(0, ...)` from `class_55::hide` | `kLsciDisplay` draws an empty text |
| 11 | `Block(0, string)` returned a fresh block where the scripts expect a copy | `Block` 0 is Copy (kernel-usage.md, Block) |
| 12 | `WindowNew`, `DrawControl`, `EditControl` with no event, `SetPort()` | `kLsciWindowNew`, `kLsciDrawControl`, `kLsciEditControl`, `kLsciSetPort` |

After OK the game offers to redial and waits. The other lands and builds were not run past start-up
in this round.

### 7.1 Online, against `innkeeperd`

A networked text-console build (section 10.1), the persona of the DOSBox setup (`guybrush`, the three
files of `work/dosbox/persona/` in the game directory), the online executive with `lsci_host` set
(section 12) and `innkeeperd` from master. Each screen
matches the DOSBox capture of the stock client (`docs/protocol/captures.md` sections 9 to 11):

| Screen | ScummVM (`work/scummvm-shots/`) | Original (`work/dosbox/shots/`) | Difference |
|---|---|---|---|
| Fall Map, after Play | `inn-net-1-fall-map.png` | `logon-1-fall-map.png` | none seen |
| Place list, after a click on the Clubhouse | `inn-net-2-place-list.png` | `logon-2-place-list.png` | none seen |
| Want To Play, after a click on Clubhouse | `inn-net-3-want-to-play.png` | `logon-3-want-to-play.png` | none seen |
| Clubhouse Waiting Room, after OK | `inn-net-4-clubhouse-waiting-room.png` | `logon-4-clubhouse-waiting-room.png` | the clock |

The message sequence in `innkeeperd`'s log is the stock client's, message for message: Login, joinNet
of the game object, the seven logon requests, land occupancy, joinNet again on entry, 40/4, joinNet of
the player, 26, setStr 5, 17 and 23, setInt, joinNet of the waiting room, GrpJoin, setInt 9 and 24.
Capture: `work/captures/scummvm-net-final/2026-10-05T125738Z-session1.hexlog` (20434 calls in 99 s:
Receive 7924, Poll 7910, Flush 3653, GetStatus 646, Service 276, Send 24, Connect 1).

What stopped the online run, in order, and the fix:

| # | Stop | Fix |
|---|---|---|
| 1 | `FileSystem(8)` from `script.101`, which asks whether the land's directory exists | `kLsciFileIsDirectory` (section 11) |
| 2 | the Login carried 10 password bytes, not 11, so the host read the persona as `uybrush` | the `a` code sends its full count (section 10) |
| 3 | the login Ack, addressed to SID 0, was dropped: `SID(3, 0)` returned 0 | `SID(3, 0)` returns the game object (section 11) |
| 4 | the `ObjID` reply was dropped: the scripts had sent the game object as a word and compare the echoed number with `self` | LSCI object handles (section 11) |
| 5 | setStr and setInt carried wrong property numbers (`ObjPropOffset` was a stub) | `ObjPropOffset`, `ObjOffsetProp` (section 11) |

## 8. Verification

- Build: `timeout 590 make -j28` in `scummvm/` is warning-free. Each of the seven proposed fork
  commits builds warning-free on its own (checked by applying them in order to a clean export of the
  fork's base and building after each).
- Detection: `SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy ./scummvm --detect --path=../work/ex/inn_cd`
  reports `sci:inn`; `inn_v2317` reports `sci:inn` "Dec 1993"; `tsn21_031293` reports `sci:tsn`.
- Start-up (2026-10-04, before the loader): `./scummvm --path=../work/ex/inn_cd inn` logged "Detected
  Early SCI1", "Detected VGA graphic resources", one "unmapped" warning for each of the 67 stubbed
  kernels, and stopped at the intentional stop. It now runs as section 7 describes.
- Resources: a build configured with `--enable-text-console` reads debugger commands from standard
  input. For each of the 9 set and land pairs (a target per pair with `lsci_land` set, and land
  directories without their own `RESOURCE.002`), `integrity_dump <file>` at the stop listed every
  resource of the map, and each one's size and MD5 equal the file `tools/dcl.py` produced in
  `work/res/<set>/<land>/` (type-31 module `n` compared as script `0xE800 + n`):

| Set | Clubhouse | SierraLand | CasinoLand |
|---|---|---|---|
| INN Feb-1994 | 338 / 338 | 373 / 373 | 297 / 297 |
| INN Dec-1993 | 316 / 316 | 353 / 353 | 277 / 277 |
| TSN 2.1 | 260 / 260 | 312 / 312 | 270 / 270 |

- Land fallback: `lsci_land=Yserbius` and a missing `SL/resource.map` both warn and start the Clubhouse.
- Kernels (2026-10-05): `timeout 590 make -j28` is warning-free after each of the seven proposed fork
  commits for the TSN executive and kernels; the files they touch outside the LSCI files also compile
  without `ENABLE_SCI32`. `make test` runs 413 tests, including `test/engines/sci/tsn_session.h`
  (binding, offline dial, the 20-tick service period, poll hooks and message queue).
- Boot: section 7, with the debugger script of section 12.
- Online executive (2026-10-05): the eleven proposed fork commits for it (`work/lsci-patches-w5/`) were
  replayed on a clone of the fork's base and each built warning-free in a build with basic networking;
  the last one also builds warning-free without `ENABLE_SCI32` and without basic networking, and in
  `scummvm/` (no basic networking). `make test` runs 425 tests; the new ones are
  `test/engines/sci/tsn_envelope.h` (the golden envelopes of `crates/int14h/tests/golden/envelopes.txt`
  written and parsed, malformed fields) and `test/engines/sci/tsn_online_executive.h` (the golden call
  sequence through a loopback transport, a stream split across reads, a lost link read as a lost carrier
  once, a reply with the wrong tag, an unreachable or unwelcoming host, a dial string the transport cannot
  carry).
- Online run: section 7.1.

## 9. The script loader

Built as designed in `docs/lsci/script-loader.md`, which replaces the plan that was here: each
container is converted in `Script::load` into an SCI0 block image, one block per item in item order,
with every item reference resolved at load time. That doc holds the layout, the code rewrites, the
engine touch points, the commit sequence and the measured results (byte parity with
`tools/lsci_image.py` in all nine set and land pairs, and disassembler parity with
`tools/lsci_disasm.py --image`).

After the loader came `kTsn` against a virtual TSNEXEC (section 10) and the boot-path kernels
(sections 7 and 11).

## 10. TSN executive

Facts: `docs/protocol/ktsn.md` (export table, binding, poll hooks, timer server, events) and
`docs/protocol/int14h-api.md` (the exports' semantics); their labels are not repeated here.

Design, following ktsn.md section 10:

- `TsnExecutive` (`tsn_executive.h`) is the resident executive: one pure virtual per export, in
  export order, and the `TsnExport` numbers. It is an interface because the executive differs at run
  time: offline, online, and a fake in the unit tests. The queries GetStatus, IsTransmitIdle and
  GetLineRate are not `const`, because the online executive asks the host.
- `OfflineTsnExecutive` behaves like TSNEXEC with a modem that never connects: driver id 3 (MODEM.DRV),
  line rate 1200, Connect returns 10 (no carrier), SwitchHost 1 (not connected), Send fails, nothing is
  received, Poll and Service report a good link, the transmitter is idle. The shared-data block keeps
  what the scripts store (256 bytes).
- `OnlineTsnExecutive` (`tsn_online_executive.*`) is the client of `docs/protocol/int14h-transport.md`:
  each export that touches the line is one CALL and its REPLY, tagged in sequence from 1 after HELLO
  (client name `scummvm`) and WELCOME. Connect opens the link and Disconnect closes it; while no link is
  open, an embedded `OfflineTsnExecutive` answers for the idle line. The executive-local exports (shared
  data, ack timeout, programs, callbacks) always stay with that embedded executive and are never sent,
  as transport section 1 allows, so the hand-off block survives a hang-up as it does in TSNEXEC.
- A REPLY that is malformed, carries another tag or export, or does not come within 10 s closes the link;
  the next Poll or Service then reports 1 (carrier lost) once, as a modem hang-up would, and the scripts
  show their carrier-lost error. A host that does not answer or does not welcome version 1 makes Connect
  return 10, the offline result. A field the transport cannot carry (a dial string over 127 bytes or with
  a control character) is not sent and also closes the link.
- `TsnEnvelopeWriter` and `TsnEnvelopeParser` (`tsn_envelope_*`) encode and parse the envelopes, length
  prefix included on write; the executive splits the stream by the prefix and refuses envelopes over
  `0x10100` bytes. `TsnTransport` (`tsn_transport.h`) is the byte stream underneath, with two
  implementations: `TsnSocketTransport` and the loopback of the unit test.
- `TsnSession` (`tsn_session.*`) is LSCITV's side of the executive. It binds on first use (ack
  timeout 300 ticks, callbacks installed) and unbinds when the engine ends. A successful Connect adds a
  poll hook and installs the timer server; the server runs Service every 20 ticks of the game clock
  (`advanceClock`, fed with ScummVM's tick count on each `TSN` and `GetEvent` call). Received bodies
  and link errors queue as network messages and errors.
- `kTsn` dispatches through `s_tsnSubops`, a 17-row table like LSCITV's jump table, instead of a ScummVM
  sub-op table, because the VM's sub-op dispatch makes an unknown sub-op a VM error, while LSCITV ignores
  sub-ops above 16 (CONFIRMED `16B1:0073`). Rows 5, 6, 10 and 15 and every sub-op above 16 bind the
  executive and return the accumulator unchanged. Send formats its arguments by the script's format
  string; the `a` code sends its full count even when the array is shorter, padding with zeros, because
  LSCITV copies `count` elements from element 0 without comparing with the array's size (CONFIRMED
  `16B1:0694`..`06C4`). The stock Login has 0 in its eleventh password byte, read past the ten-byte
  array of `PASS_SET.DTA` (captures.md section 4); that LSCITV's heap holds 0 there is INFERRED. A failed Connect posts its result as a network error.
- Received bodies become byte arrays when they are received, as LSCITV's Receive allocates them through
  the `alloc` callback, and wait in `EngineState::_lsciNetworkMessages`, which the GC treats as roots.
  `kLsciGetEvent` runs the poll hooks, then returns the oldest message (type `0x400`, the array in
  `message`, its length in `modifiers`) or error (`0x800`, code in `modifiers`) before falling back to
  `kGetEvent`. A peek returns the same array as the read that follows it. A mask of only `0x400` never
  reads input events.
- `SciEngine` owns the session; it exists only for LSCI games. `makeTsnExecutive` chooses the executive:
  online when the game option `lsci_host` is set, with `lsci_port` (default 2315, `innkeeperd`'s
  `--int14h-bind`), offline otherwise. The executives log dials, the host's name and `TSN(9)` program
  names on the `Network` debug channel (`--debugflags=network`).

### 10.1 Which networking facility

ScummVM's `backends/networking` offers three things, and the configured builds were checked
(`config.h` of `scummvm/` and `work/build-textcon`):

| Facility | What it is | In this build |
|---|---|---|
| `Networking::Socket` (`backends/networking/basic/socket.h`) | a TCP client: `connect(url)`, `send`, `recv`, `ready`; implemented with libcurl's connect-only mode, or Java on Android | needs `USE_BASIC_NET`, which `configure` sets when libcurl is found; off here, as no libcurl headers are installed |
| SDL_net (`backends/networking/sdl_net/`) | the local web server for the cloud and file-transfer GUI | `USE_SDL_NET` on, but it is a server inside the backend, with no client API for engines |
| ENet (`backends/networking/enet/`) | reliable UDP for SCUMM HE multiplayer | `USE_ENET` off; and the transport is a TCP byte stream |

The online executive uses `Networking::Socket`, the conventional choice: it is the engine-facing TCP
client, and the SCUMM HE lobby (`engines/scumm/he/net/net_lobby.cpp`) already reaches its server through it
with an `http://` URL that libcurl only connects. Calling SDL_net from an engine would tie the engine to one
backend library, which ScummVM's engines avoid. `TsnSocketTransport` is therefore built only with
`USE_BASIC_NET` (`engines/sci/module.mk`); without it, `lsci_host` logs a warning and the game stays offline.

For the runs in this document a networked text-console build was configured in `work/build-netcon`
against libcurl's headers unpacked from the Ubuntu package (`apt-get download libcurl4-openssl-dev`,
`dpkg -x` to `work/libcurl-dev/`, a `curl-config` there that points at those headers and the system's
`libcurl.so.4`), with `PATH=work/libcurl-dev/bin:$PATH ../../scummvm/configure --disable-all-engines
--enable-engine=sci --enable-engine=sci32 --disable-detection-full --enable-text-console --disable-debug
--enable-release-mode --enable-libcurl --disable-cloud`. Installing `libcurl4-openssl-dev` gives the
in-tree build the same.

## 11. LSCI kernels in ScummVM

LSCI's data kernels work on SCI32-style arrays: a script's byte strings, ID lists and integer arrays
become `SciArray`s (types string, byte, ID and int16), so the SCI32 array and string kernels are reused
through LSCI sub-op tables. LSCI therefore needs a build with SCI32. Facts about the original are in
kernel-usage.md (signatures) and interpreter.md (handler addresses).

Engine changes, each behind `isLsci()`:

| Change | Reason |
|---|---|
| `Object::getClass` returns the object named by `-dict-` | LSCI objects may inherit from an instance; `-dict-` names the object that holds the property layout (CONFIRMED by `PlayerRadioButton`, an instance used as a class by `ObjectNew`) |
| `nsTop`..`nsRight` selectors alias `top`..`right` | LSCI views keep rects in Rect objects (`lastSeen`, `baseRect`), so `GfxCompare::getNSRect`/`setNSRect` read and write Rect objects unchanged |
| `GfxAnimate` reads and writes the last-seen rect through the `lastSeen` Rect object | SCI keeps it in `lsTop`..`lsRight` on the view |
| `file_open` neither compresses nor makes files read-only | LSCI's data files are shared with the DOS original, and LSCI opens every file for reading and writing |
| The GC marks objects in the SID table | the table is the only reference to objects that the host names by id |
| The GC marks the byte arrays of waiting network messages | they are created on receipt, before any script holds them (section 10) |
| `send`, `eq?` and `ne?` resolve LSCI object handles (`vm.cpp`, `lsci_handle.*`) | see "Object handles" below |

Kernel notes (signatures from kernel-usage.md, Feb-94):

- `Animate(list, cycle)`: a non-null LSCI list gets a live `doit` pass (frozen views skipped), then a
  temporary SCI list feeds `GfxAnimate::kernelAnimate`.
- `CanBeHere(view, cast)`: false when the view's `baseRect` covers a control colour in its
  `illegalBits`; true when the view itself ignores actors or is being removed; otherwise false when
  its `baseRect` meets the `baseRect` of a cast member without IgnoreActor, RemoveView or NoUpdate
  (LSCITV `0cdf:224c`, CONFIRMED).
- `Wait(ticks)` waits as in SCI. `class_88::play`, the main loop, calls `Wait(period, 0)` once and
  then `Wait(period, 1)` each cycle, running `TSN` Poll and the idle handler in between (CONFIRMED).
  ScummVM returns 0 until the period has passed, then the game ticks elapsed (INFERRED from that loop).
- `Array` 7 (compare) and the other byte views of `lsci_array` also take a string literal in a script,
  which the boot path compares against an array about 50 times.
- `Block` 0 copies a block and 1 frees it (kernel-usage.md, Block).
- `Graph` 3 draws a line with (y, x) pairs, like SCI's `kGraphDrawLine`.
- `DrawControl` draws buttons (greyed when not enabled) and text through `kernelDrawText`, edit
  fields and icons through SCI's control code.
- `Long` works on objects with `highWord` and `lowWord`; `SID` keeps a table from host object ids to
  objects in `EngineState`.
- `SID(3, 0)` returns the game object. LSCITV returns the word at `1FCA` without searching the table
  (messages.md section 0); the only write to it, `0BAE:000D`, stores the value that `0BAE:000A` also
  stores as the current object (`DGROUP:0A52`, interpreter.md) at start-up (CONFIRMED code path; that
  the value is the game object is INFERRED, and it is what makes the login Ack reach `Dialing`).
- `FileSystem(8, path)` is IsDirectory: LSCITV runs a DOS find-first for `path` with attributes `0x12`
  into the DTA at `DS:3524` and returns 1 when one is found and its attribute byte (`DS:3539`) has bit
  `0x10` (CONFIRMED `0FAC:0231`..`0260`). ScummVM resolves the DOS path below the game directory, the
  working directory of LSCITV; an empty path after normalising (`.`) is the game directory itself.
- `ObjPropOffset(object, selector)` returns the slot of that property, counting from `-env-` as slot 0,
  and `ObjOffsetProp(object, slot)` the selector of a slot (CONFIRMED `1773:0002` searches the property
  dictionary, `1773:00CE` indexes it; both call LSCITV's error routine for a missing one, where
  ScummVM warns and returns -1 or 0). LSCI's slots are ScummVM's variable indices (`docs/lsci/script-loader.md`), so the kernels use
  `Object::locateVarSelector` and the class's selector list. setStr and setInt use them to name the
  properties they replicate, so they reach the host: name 5, looks 17, home 23, game 9, room 24.

Object handles. LSCITV names every object by a 16-bit handle, and the scripts send their own objects as
words: joinNet carries `self` as its cookie, and the host's `ObjID` reply gives the cookie back, which the
scripts compare with `self` (`Obj::handleMsg`) and use as an object (messages.md section 0 lists the four
replies that do this: `ObjID`, `NakMsg` for 55, `ObjFreeList`, `ObjNewList`). A ScummVM reference is a
segment and an offset, so the formatted send's `w` code (and `+` after it) turns any reference into a
handle from `EngineState::_lsciHandles`, numbered from `0x8000` so that small script numbers never name
an object. The VM resolves such a number back to its reference where LSCITV would use it as an object: as
the target of `send`, in `eq?` and `ne?` against a reference, and in `IsObject`. Handles are never
reused; the table grows by one entry per object sent (three on the way to the waiting room).

## 12. Driving the game headless

The text-console build (`--enable-text-console`) reads debugger commands from standard input. With
`--debugflags=onstartup` the debugger opens before the first script instruction, so a command file
can set kernel breakpoints (`bpk DrawPic`, `bpk GetEvent`), step with `go`, and use two fork commands:

- `screenshot` updates the screen and saves it through ScummVM's screenshot path (BMP, aspect-corrected
  to 640x480; convert with `ffmpeg`).
- `click x y [down|up]` queues a mouse move and a left-button press and/or release at game coordinates.
  Press and release are sent in separate commands with `go`s between them, because button tracking
  loops read the release in a later `GetEvent`.

Run with `SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy`, `stdbuf -o0` (the log is otherwise buffered
until exit) and `enable_unsupported_game_warning=false` in the ini, or the unsupported-game dialog
waits for a click. The section 7 run: `bpk DrawPic`, two `go`, `bc *`, `bpk GetEvent`, 100 `go`,
`click 285 60 down`, 20 `go`, `click 285 60 up`, 400 `go`, `bc *`, `screenshot`; then for the redial
prompt `bpk GetEvent`, `click 241 146 down`, 20 `go`, `click 241 146 up`, 400 `go`, `bc *`, `screenshot`.

The section 7.1 run needs a build with basic networking (section 10.1) and `innkeeperd` from master:

```
cargo run -p innkeeperd -- --bind 127.0.0.1:2514 --int14h-bind 127.0.0.1:2515 --capture-dir work/captures/scummvm-net-final
```

(ports beside the defaults, which other servers were using). The target's ini section adds
`lsci_host=127.0.0.1` and `lsci_port=2515` (2315 when omitted) and points `path` at an installed client
tree holding the persona files (`work/scummvm-game/INN`). The debugger commands were appended to a file that
`tail -f` feeds to ScummVM, so each step could wait for the screenshot of the one before:
`bpk DrawPic`, two `go`, `bc *`, `bpk GetEvent`, 100 `go`, Play as above, 1500 `go` (the Fall Map);
`click 185 150` (the Clubhouse on the map), 600 `go`; `click 95 102` (Clubhouse in the place list),
600 `go`; `click 203 160` (OK in Want To Play), 1500 `go`; each click as a press and a release 20 `go`s
apart, and a `screenshot` after each step. `logkernel *` traces every kernel call, which is how the stops
of section 7.1 were found.

## 13. Open questions

- The SCI1 sub-version whose graphics behaviour matches LSCI (`SCI_VERSION_1_EARLY` is a starting point).
- `GRAPH256.DLL` replaces 30 graphics routines through hook slots (interpreter.md section 3.5); which of
  them differ from SCI1 behaviour.
- SierraLand's `vocab.997` lacks 261 selector names (CONFIRMED). INFERRED: ScummVM's selector cache only
  needs names such as `x`, `y` and `view`, which are among the 561 present; to be checked against every
  selector the mapped kernels look up.
- When to restart for a land switch without losing the virtual TSNEXEC state (connection, hand-off block).
  `TSN(9)` only logs the program name so far.
- The online executive calls the host once per Poll, Receive and Flush, about 200 calls a second on the
  waiting-room screen, and blocks the game until each reply (at most 10 s). Transport version 1 has no
  "messages waiting" notice (int14h-transport.md section 7).
- A build without basic networking (no libcurl) stays offline. A `Networking::Socket` on SDL_net in the
  backend would bring the online executive to those builds; that is common code, outside this fork's scope.
- LSCI object handles are never released, and a handle of an object that was freed resolves to a stale
  reference, as a stale handle does in LSCITV. Only references sent with `w` become handles; a reference
  stored in a byte array or sent with `b` does not.
- Past the waiting room nothing was run online: Talk, Look, Invite, games, land switch.
- The persona colour table passed to `DrawCel` (argument 7) and the scaled paths of `CelRect` and
  `DrawCel`, which reuse SCI's scaling (INFERRED to match).
- `DrawControl` types 5 and 6 (not met on the boot path); edit controls use SCI's text handling.
- Unmapped sub-ops: `String` 6, 9, 10; `FileSystem` 4, 14, 15 (FindFirst/FindNext); `Graph` 0, 2,
  5, 8, 9, 13, 14, 15. A call to one is a VM error naming the sub-op.
- Save games do not hold the SID table or the polling `Wait` state; LSCI disables Save and Restore,
  so only ScummVM's own saves would need them.
- `Memory` reports an assumed 512 KB heap (`kLsciAssumedHeapBytes`).
- `ModuleDispose` disposes the scripts at once; LSCITV queues them for later disposal (CONFIRMED).
  No difference has shown on the boot path.

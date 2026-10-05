# LSCI in ScummVM: integration groundwork

How the ScummVM fork (`scummvm/`, branch `lsci`) recognises the LSCI clients, opens their resources,
selects LSCI's kernel table and loads LSCI scripts, and why it is done that way. The script loader
has its own document, `docs/lsci/script-loader.md`.
Original-software facts come from `docs/lsci/interpreter.md`, `docs/lsci/script-format.md`,
`docs/lsci/kernel-usage.md` and `docs/protocol/ktsn.md`; their labels are repeated here.

Status (2026-10-05): detection, resource loading for every land of the three builds, kernel-table
selection and script loading work. 70 of LSCI's 93 kernels are implemented or mapped, `kTsn` runs on
a virtual TSNEXEC with an offline executive (section 10), and the Feb-1994 Clubhouse boots headless
through the title and Select Player screens to the Dialing screen, where the offline executive's
"no carrier" result is shown as the game's Error #10 dialog; its OK button leads to the game's redial
prompt (section 7).

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
| TSN executive | `TsnExecutive` interface with TSNEXEC's 17 exports, `OfflineTsnExecutive`, and `TsnSession` for LSCITV's side (binding, poll hooks, timer server, event queues) | `engines/sci/engine/tsn_*`, `ktsn.cpp` (section 10) |
| LSCI kernels | LSCI-specific kernels and sub-op tables; built with SCI32, whose arrays and strings they use | `klsci.cpp`, `klsci_graphics.cpp`, `klsci_lists.cpp`, `klsci_file.cpp`, `lsci_array.*` (section 11) |
| Engine touch points | class lookup through `-dict-`, view rects in Rect objects, LSCI file modes, SID table in the GC root set; each behind `isLsci()` | `object.cpp`, `selector.cpp`, `animate.cpp`, `file.cpp`, `gc.cpp` (section 11) |
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
| Mapped to the SCI kernel | `IsObject`, `ObjectFree` (kDisposeClone), `ObjectRespondsTo` (kRespondsTo), `ModuleID` (kScriptID, CONFIRMED same semantics, script-format.md section 6), `GetFarText`, `DrawPic`, `GlobalToLocal`, `LocalToGlobal`, `MapKeyToDir`, `HaveMouse`, `GameIsRestarting`, `GetTime` (modes 1-3 encode like ScummVM's, kernel-usage.md), `Abs`, `CosDiv`, `CosMult` (kTimesCos), `GetAngle`, `GetDistance`, `Random`, `SinDiv`, `SinMult` (kTimesSin), `Joystick`, `Palette`, `GetPort`, `PicState` (kPicNotValid), `WindowDispose` (kDisposeWindow), `CelHigh`, `CelWide`, `Sound` (kDoSound) |
| Mapped through an LSCI sub-op table to SCI32 kernels | `Array`, `String`, `FileSystem`, `Graph` (rows that need LSCI arguments call an LSCI function) |
| LSCI functions (section 11) | `ObjectNew`, `Resource`, `GetEvent`, `SetCursor`, `ConfigStr`, `Animate`, `Display`, `TextSize`, `CelRect`, `DrawCel`, `Wait`, `SetPort`, `WindowNew`, `DrawControl`, `HiliteControl`, `EditControl`, `CanBeHere`, `OnControl`, `Block`, `ModuleDispose`, `NumLoops`, `NumCels`, `BaseRectSet`, `Memory`, `List`, `Seq`, `Long`, `SID`, `TSN` |
| `kEmpty` (LSCITV handler is a bare `retf`, CONFIRMED) | `CheckSaveGame`, `GetSaveDir`, `GetSaveFiles`, `DeviceInfo`, `Profiler`, `SetDebug`, `ShowFree`; also the DLL kernel `SetSynonyms`, which only the parser uses |
| `kStubNull` (LSCITV sets acc = 0, CONFIRMED) | `Debug` |
| `kStub` (unlisted) | `CoordPri`, `ShakeScreen`, `Show`, `AddToPic`, `DrawScaledCel`, `ScaledCelRect`, `Menu`, `Restart`, `Restore`, `Save`, `SetTimerFreq`, `Alert`, `Sqrt`, `StackUsage`, `Encrypt`, `Decrypt`, `ColorUp`, `ColorDown`, `ObjPropOffset`, `ObjOffsetProp`, `InvokeMethod`, `Said`, `Parse` |

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
  export order. It is an interface because the executive differs at run time: offline now, a network
  link to `innkeeperd` later, a fake in the unit test.
- `OfflineTsnExecutive` behaves like TSNEXEC with a modem that never connects: driver id 3 (MODEM.DRV),
  line rate 1200, Connect returns 10 (no carrier), SwitchHost 1 (not connected), Send fails, nothing is
  received, Poll and Service report a good link, the transmitter is idle. The shared-data block keeps
  what the scripts store (256 bytes).
- `TsnSession` (`tsn_session.*`) is LSCITV's side of the executive. It binds on first use (ack
  timeout 300 ticks, callbacks installed) and unbinds when the engine ends. A successful Connect adds a
  poll hook and installs the timer server; the server runs Service every 20 ticks of the game clock
  (`advanceClock`, fed with ScummVM's tick count on each `TSN` and `GetEvent` call). Received bodies
  and link errors queue as network messages and errors.
- `kTsn` dispatches through a 17-row sub-op table; rows 5, 6, 10 and 15 return the accumulator
  unchanged, as in LSCITV. Send formats its arguments by the script's format string. A failed Connect
  posts its result as a network error.
- `kLsciGetEvent` runs the poll hooks, then returns a queued message (type `0x400`, byte array in
  `message`, length in `modifiers`) or error (`0x800`, code in `modifiers`) before falling back to
  `kGetEvent`. A mask of only `0x400` never reads input events.
- `SciEngine` owns the session; it exists only for LSCI games. The offline executive logs dials and
  `TSN(9)` program names on the `Network` debug channel (`--debugflags=network`).

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

## 13. Open questions

- The SCI1 sub-version whose graphics behaviour matches LSCI (`SCI_VERSION_1_EARLY` is a starting point).
- `GRAPH256.DLL` replaces 30 graphics routines through hook slots (interpreter.md section 3.5); which of
  them differ from SCI1 behaviour.
- SierraLand's `vocab.997` lacks 261 selector names (CONFIRMED). INFERRED: ScummVM's selector cache only
  needs names such as `x`, `y` and `view`, which are among the 561 present; to be checked against every
  selector the mapped kernels look up.
- When to restart for a land switch without losing the virtual TSNEXEC state (connection, hand-off block).
  `TSN(9)` only logs the program name so far.
- An online executive: the virtual TSNEXEC over a socket to `innkeeperd`, in place of the offline one.
- The persona colour table passed to `DrawCel` (argument 7) and the scaled paths of `CelRect` and
  `DrawCel`, which reuse SCI's scaling (INFERRED to match).
- `DrawControl` types 5 and 6 (not met on the boot path); edit controls use SCI's text handling.
- Unmapped sub-ops: `String` 6, 9, 10; `FileSystem` 4, 8, 14, 15 (FindFirst/FindNext); `Graph` 0, 2,
  5, 8, 9, 13, 14, 15. A call to one is a VM error naming the sub-op.
- Save games do not hold the SID table or the polling `Wait` state; LSCI disables Save and Restore,
  so only ScummVM's own saves would need them.
- `Memory` reports an assumed 512 KB heap (`kLsciAssumedHeapBytes`).
- `ModuleDispose` disposes the scripts at once; LSCITV queues them for later disposal (CONFIRMED).
  No difference has shown on the boot path.
- A peeked network message (`GetEvent` mask with `0x8000` and `0x400`) gets a new byte array on each
  call, so a peek followed by a read leaves one array unfreed. No Feb-94 call site was found to peek
  the network queue.
- `TSN` sub-ops above 16 are a VM error here; ktsn.md says LSCITV returns the accumulator unchanged.

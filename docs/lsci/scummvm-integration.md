# LSCI in ScummVM: integration groundwork

How the ScummVM fork (`scummvm/`, branch `lsci`) recognises the LSCI clients, opens their resources,
selects LSCI's kernel table and loads LSCI scripts, and why it is done that way. The script loader
has its own document, `docs/lsci/script-loader.md`.
Original-software facts come from `docs/lsci/interpreter.md`, `docs/lsci/script-format.md`,
`docs/lsci/kernel-usage.md` and `docs/protocol/ktsn.md`; their labels are repeated here.

Status (2026-10-05): detection, resource loading for every land of the three builds, kernel-table
selection and script loading work. Every land boots into its game object's `init` and stops at the
first kernel whose stub breaks the script, `TSN(0)` (section 7).

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
| Mapped to the SCI kernel | `IsObject`, `ObjectRespondsTo` (kRespondsTo), `ModuleID` (kScriptID, CONFIRMED same semantics, script-format.md section 6), `GetFarText`, `GetEvent`, `MapKeyToDir`, `HaveMouse`, `GameIsRestarting`, `GetTime` (modes 1-3 encode like ScummVM's, kernel-usage.md), `Abs`, `CosDiv`, `CosMult` (kTimesCos), `GetAngle`, `GetDistance`, `Random`, `SinDiv`, `SinMult` (kTimesSin), `Joystick` |
| `kEmpty` (LSCITV handler is a bare `retf`, CONFIRMED) | `CheckSaveGame`, `GetSaveDir`, `GetSaveFiles`, `DeviceInfo`, `Profiler`, `SetDebug`, `ShowFree` |
| `kStubNull` (LSCITV sets acc = 0, CONFIRMED) | `Debug` |
| `kStub` (unlisted, so mapped by the fallback; logs the call, returns acc unchanged) | the other 67, including `TSN`, `SID`, `Array`, `List`, `String`, `Memory`, `Resource`, `Block`, `Graph`, `Display`, all drawing, port, window and control kernels, `Sound`, `FileSystem`, `ConfigStr`, the four DLL kernels |

Notable stubs: `Sqrt` and `Wait` are called with 2 arguments where ScummVM's take 1; `NumCels` with
2 where ScummVM's takes an object; `OnControl` with 2 or 3; `DrawCel` with up to 9. `ObjectNew` and
`ObjectFree` probably equal `kClone` and `kDisposeClone` (INFERRED), but wait for the loader.
`ModuleDispose` queues scripts for later disposal (CONFIRMED) unlike the immediate `kDisposeScript`.

## 7. How far the game runs

The intentional stop in `Script::load` (an `error()` for every LSCI script) is gone: the script loader
replaced it (`docs/lsci/script-loader.md`, commit F5). Script 0, its class modules and `script.003`
load, the game object is export 0 of script 0, and `play` runs. Unimplemented kernels are `kStub`,
which logs the call and leaves the accumulator unchanged. Boot log of the Feb-1994 Clubhouse
(`SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy ./scummvm --path=../work/ex/inn_cd inn`), every
`Dummy function` warning in order:

| # | Kernel | Calls | Arguments | Caller |
|---|---|---|---|---|
| 1 | `SetCursor` (0x2B) | 1 | `0x381, 0, 2` | `CC::setCursor`, sent by `play` after the mapped `HaveMouse` |
| 2 | `Resource` (0x09) | 8 | `0, 135, 0` / `0, 135, 3` / `0, 135, 999` / `2, 135, 0` / `2, 135, 3` / `2, 135, 999` / `0, 128, 897` / `2, 128, 897` | `CC::init` |
| 3 | `Palette` (0x59, DLL) | 16 | sub-op 5 and three colour indices each | export 0 of module 615 (`calle 0xEA67 0`), from `CC::init` |
| 4 | `TSN` (0x54) | 1 | `0` (GetStatus) | `CC::init` |

`CC::init` then shifts the `TSN(0)` result right by 8 to get the connection byte. The stub returned
the object left in the accumulator, so the VM stops with
`[inn 0 CC::init @ 03f7]: Invalid arithmetic operation (shift right - params: 0012:07d4 and 0000:0008)`
and opens the debugger, where `bt` shows `CC::play` calling `CC::init`. The other eight set and land
pairs log the same four kernels in the same order and stop at the same shift in `SL::init` or
`LL::init`. The next kernel to implement is therefore `kTsn` sub-op 0, then `Resource`, `Palette` and
`SetCursor`.

## 8. Verification

- Build: `timeout 590 make -j28` in `scummvm/` is warning-free. Each of the seven proposed fork
  commits builds warning-free on its own (checked by applying them in order to a clean export of the
  fork's base and building after each).
- Detection: `SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy ./scummvm --detect --path=../work/ex/inn_cd`
  reports `sci:inn`; `inn_v2317` reports `sci:inn` "Dec 1993"; `tsn21_031293` reports `sci:tsn`.
- Start-up (2026-10-04, before the loader): `./scummvm --path=../work/ex/inn_cd inn` logged "Detected
  Early SCI1", "Detected VGA graphic resources", one "unmapped" warning for each of the 67 stubbed
  kernels, and stopped at the intentional stop. Since the loader it runs to the stop in section 7.
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

## 9. The script loader

Built as designed in `docs/lsci/script-loader.md`, which replaces the plan that was here: each
container is converted in `Script::load` into an SCI0 block image, one block per item in item order,
with every item reference resolved at load time. That doc holds the layout, the code rewrites, the
engine touch points, the commit sequence and the measured results (byte parity with
`tools/lsci_image.py` in all nine set and land pairs, and disassembler parity with
`tools/lsci_disasm.py --image`).

After the loader: implement `kTsn` against a virtual TSNEXEC (ktsn.md section 10), then the
boot-path kernels in the order of kernel-usage.md section 8, starting with the four in section 7,
and the object kernels (`ObjectNew`, `ObjectFree`, `ObjPropOffset`, `ObjOffsetProp`, `InvokeMethod`).

## 10. Open questions

- The SCI1 sub-version whose graphics behaviour matches LSCI (`SCI_VERSION_1_EARLY` is a starting point).
- `GRAPH256.DLL` replaces 30 graphics routines through hook slots (interpreter.md section 3.5); which of
  them differ from SCI1 behaviour.
- SierraLand's `vocab.997` lacks 261 selector names (CONFIRMED). INFERRED: ScummVM's selector cache only
  needs names such as `x`, `y` and `view`, which are among the 561 present; to be checked against every
  selector the mapped kernels look up.
- When to restart for a land switch without losing the virtual TSNEXEC state (connection, hand-off block).

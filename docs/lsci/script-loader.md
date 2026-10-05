# LSCI script loader

How the ScummVM fork runs LSCI scripts and type-31 modules: each item container is converted into an
SCI0 block image inside `Script::load`, and ScummVM's existing SCI0/SCI1 script, object, VM, GC and
debugger code runs on that image. This doc replaces the plan in `scummvm-integration.md` section 9.
Original-software facts come from `script-format.md` and `interpreter.md` and keep their labels.
"Counted" numbers were measured on 2026-10-05 over all four sets (1,292 containers) with
`tools/lsci_format.py`, `tools/lsci_bytecode.py` and `tools/lsci_image.py`. "ScummVM code" facts
were read in the fork at branch `lsci`.

Status (2026-10-05): built through F6 (section 9). Every container of the three detected builds
converts with byte parity to `tools/lsci_image.py`, and all nine set and land pairs boot through
script loading into the game object's `init`; section 12 has the results. F7 and F8 are not built
yet.

## 1. Decision

Each LSCI container is converted, when its script is loaded, into an ordinary SCI0 block image: one
block per item in item order, and every item reference resolved to an image offset. ScummVM then
loads the image through its unchanged `SCI_VERSION_1_EARLY` path. The resource layer is not touched,
so resources stay byte-identical to `tools/dcl.py`. `vm.cpp` and `scriptdebug.cpp` do not change.
As built (F1-F6), upstream files change by 109 added and 10 removed lines, 43 of the added lines in
the debugger's `verify_scripts` branch.

This works because of one CONFIRMED fact (script-format.md 4.3): every word that holds an item
reference has a fixup, and no other word does. All handles can therefore be resolved at load time,
and after that ScummVM never needs to know that LSCI had handles.

Three candidate designs were scored, from 1 (worst) to 10 (best):

| Criterion | Translate in the resource layer | Native item table in `Script` | Minimal image in `Script::load` |
|---|---|---|---|
| Correctness risk against LSCITV | 7 | 8 | 8 |
| Invasiveness to upstream `engines/sci` | 7 | 3 | 9 |
| Fit with ScummVM conventions and SCI maintainers | 5 | 4 | 8 |
| Debugger and tooling parity | 9 | 10 | 6 |
| Path to full fidelity (all lands, network kernels) | 8 | 8 | 6 |
| Time to the first room | 6 | 4 | 9 |
| **Total** | **42** | **37** | **48** |

The minimal design wins. Most of its weaknesses are in tooling parity and fidelity, and the
runners-up fill those in:

- From the translate design: a separate parser and writer, a full decode of every Code item, the
  rewrite of `_line_`/`_file_` to SCI's `line` (no VM change), byte parity through a Python mirror
  writer and image MD5s, a cxxtest unit test, the stray-branch patches and `kModuleDispose`.
- From the native design: the finding that game code does arithmetic on `-env-`, the data fact on
  export bounds, the edge-case load list, the bulk counts per land, and a text-console build for
  debugger tests.

## 2. Conflicts between the designs and how they are resolved

| Question | Candidates | Chosen | Why |
|---|---|---|---|
| Where the conversion runs | `ResourceManager::loadResource` (translate) or `Script::load` (minimal) | `Script::load` | The resource layer stays free of script structures, and `integrity_dump` keeps its byte parity with `tools/dcl.py`. The cost is setting the game object in `initGame` and a `dissect_script` message. |
| Code-area order | ordered methods/procedures/methods (translate) or item order (minimal) | item order | ScummVM code: below SCI3, `reg_t::init` and `setOffset` store the offset in a `uint16`, so the `op_call` PC (`make_reg32(seg, pc + int16)`) wraps at 16 bits like the original's IP. The translate design's premise that `op_call` does not wrap is wrong. A call that is 42,135 bytes forward is encoded as a negative `int16` and lands correctly. |
| `_line_` 0x4C / `_file_` 0x4E | VM no-op in `op_info`/`op_superP` plus format rows (minimal) or rewrite to `line` 0x7E (translate) | rewrite | ScummVM code: `op_line` is a no-op with a word operand in every version, and the base formats for 0x26/0x27 are `Script_Invalid`. The rewrite needs no change in `vm.cpp` or `kernel.cpp`. It loses only the `_line_`/`_file_` distinction in `disasm`, and nothing reads those registers (CONFIRMED, interpreter.md 4). |
| Locals block | first n words (translate) or the whole payload of the last Variables item (minimal, native) | whole payload | LSCITV addresses local k at byte 2k of the block, slack included (CONFIRMED, `lal` `09c8:098C`). Local 0 holds the count and is unused. |
| Strings | one shared strings block (translate) or one block per String item (minimal) | one block per item | One rule (one block per item), `scrs` lists exactly the String items in order, and `identifyOffsets` cannot drop an empty string at the end of a shared block. Largest image 55,118 bytes (counted, `hub/script.406` in both INN sets). |
| Parser and writer | one class (minimal) or `LsciContainer` plus `LsciScriptImageWriter` (translate) | two | Follows the CONVENTIONS suffix vocabulary and one concept per header. `verify_scripts` reuses the parser. |
| Python mirror | layout map only (minimal) or a full-byte writer (translate) | full-byte writer | It allows an offline check of every container and MD5 parity with the engine. |
| `findSierraGameId` | name selector 5 for LSCI (translate) or unchanged (minimal) | unchanged | With conversion in `Script::load`, `findGameObject` keeps returning `NULL_REG` for LSCI, and LSCI builds are detected by MD5. |
| Stray branches, `ModuleDispose`, tests | in scope (translate) or deferred (minimal) | in scope, as late commits | They are not on the boot path, so they come after the switch-on commit. Each one is needed before a named milestone (section 9). |
| Feature detectors | lofs and DoSound (minimal) or also gfx functions and SetCursor (native) | lofs and DoSound only | ScummVM code: `detectGfxFunctionsType` and `detectSetCursorType` return fixed answers for `SCI_VERSION_1_EARLY` without scanning. |

## 3. Rejected alternatives

**Native item table inside `Script` (no rewrite).** Bytes stay as on disk, and a per-script item
table resolves `call`, `loadID` and `pushID` operands at run time. This design gives the best address
identity: a ScummVM offset equals the resource byte offset. It costs about 980 fork lines, including
branches in `run_vm` (`op_call`, `op_lea`, `op_lofss`), `Object::init`, `propertyOffsetToId`,
`getObject`, `uninstantiateScript`, a new `Script_Item` operand kind and about 15 upstream files, some
of them hot. An SCI maintainer would ask why a load-time conversion does not remove all of this. The
address identity it gains is recovered here by a deterministic layout and `lsci_disasm.py --image`.

**Conversion in the resource layer.** Every reader of script bytes (`findGameObject`, `dissect_script`,
`hexdump`) would see an SCI1 image, but the resource layer would then depend on engine script
structures, and resource MD5s would stop matching the volume files. It remains the fallback if a
reviewer prefers it; the writer moves there unchanged.

**SCI1.1 heap/hunk layout.** SCI1.1 reads the property and method dictionary offsets from vars 2 and
3, which are LSCI's `-dict-` and `-super-`. It would need version flips in a dozen places. In the
SCI0/1 layout, properties start at the object address, which is how LSCI addresses them.

**A segment per Code item.** This is the literal handle model. The Feb-1994 hub alone has 3,687 Code
items, which puts pressure on segment ids. It also needs a new segment type in `run_vm`, `ExecStack`,
the GC and savegames.

**`LsciScript : Script` subclass.** The loaders are not virtual, and `SegManager` constructs `Script`
directly. The conventions also forbid inheritance for code reuse.

**Filling `Script`/`Object` fields directly, with no image.** This needs branches in
`identifyOffsets`, `initializeObjects`, `Object::init`, relocation and the debugger: a larger upstream
diff than the writer.

## 4. Data structures (new fork files, `engines/sci/engine/`)

`lsci_container.h/.cpp` is the parser: bytes in, domain value out, no `g_sci`. As built, the item is
a nested type, so the header defines one concept (CONVENTIONS 2.6), and the enums `LsciItemTag` and
`LsciObjectSlot` (`-env-` .. `name`) sit beside it.

```cpp
struct LsciContainer {
	struct Item {
		LsciItemTag tag;
		SciSpan<const byte> payload;  // borrowed from the resource
		Common::Array<uint16> fixups; // payload offsets of words that hold item numbers
		uint16 getWord(uint32 payloadOffset) const;
		bool isReference(uint32 payloadOffset) const;
		uint16 getPropertyCount() const;
		uint16 getMethodCount() const;
	};
	Common::Array<Item> items;
};

bool parseLsciContainer(const SciSpan<const byte> &data, LsciContainer &container, Common::String &error);
```

The parser checks these at the boundary; each failure means "not an item container":

- the count, lengths and fixup bounds;
- that no bytes trail the last item;
- that every tag is known and every fixup target exists;
- that every Class is followed by its Property Dictionary;
- that every object has the six leading slots `-env-` .. `name` (the smallest in the data has 7,
  counted), because the writer and `Object` read up to `name`;
- that every object's length is `2*size + 2 + 4*methodCount`;
- that every String item has its header and ends with its NUL, and every Dispatch Table's length is
  `2 * (highest + 2)`.

Said Spec and Synonym Table items fail as "unsupported" (none exist, CONFIRMED). The 42 CasinoLand
view-format resources stored under script numbers fail here and are reported, never loaded.

`lsci_script_image.h/.cpp` is the writer: a domain value in, image bytes out. It is a pure function
and uses neither `g_sci` nor `_opcode_formats`. The reasons: script 0 loads before
`script_adjust_opcode_formats` runs, and cxxtest links the writer alone.

```cpp
class LsciScriptImageWriter {
public:
	struct StrayBranch { uint16 item; uint16 offset; int32 target; };
	bool write(const LsciContainer &container, Common::Array<byte> &image, Common::String &error);
	const Common::Array<StrayBranch> &getStrayBranches() const;
	...
};
```

It has two passes. The layout pass computes every block size and each item's target offset. The
emit pass then writes the blocks and rewrites operands. A static table
`s_lsciOperandFormats[128][4]` of `opcode_format` rows describes LSCI's source operand widths, and
the writer uses it to decode every Code item. It is the C++ copy of `OPCODE_TABLE` in
`tools/lsci_bytecode.py`; the MD5 parity check (section 8, stage C) catches any drift between them.

### 4.1 Image layout: the one rule shared by the engine and the tools

Blocks follow item order. Each block has the SCI0 header `{type, size}`, and every block is padded
to an even length. The size includes the header and the padding byte: `findBlockSCI0`,
`identifyOffsets` and `initializeObjectsSci0` step from block to block by that size (ScummVM code;
the first build wrote the unpadded size and `findBlockSCI0` walked off the image). The image ends with
the relocation block (no leading null entry) and a type-0 word. The largest image is 55,118 bytes
(counted), under the 64 KiB of a 16-bit segment.

| LSCI item | SCI0 block | Body | References resolve to |
|---|---|---|---|
| Object (2) | `SCI_OBJ_OBJECT` | `0x1234`, 0, function-area offset, property count; properties from slot 0; `{count, selectors, 0, code offsets}` | first property (block + 12) |
| Class (3) | `SCI_OBJ_CLASS` | as Object, plus the following Property Dictionary's selectors after the properties | first property |
| Property Dictionary (11) | none (merged into its class) | | |
| Code (4) | `SCI_OBJ_CODE` | payload with operands rewritten (4.2) | first instruction |
| String (7) | `SCI_OBJ_STRINGS` | text and NUL; LSCI's `{kind, size}` header dropped | first character |
| Variables (6), last one | `SCI_OBJ_LOCALVARS` | whole payload, so local k stays word k | local 0 |
| Variables (6), earlier ones | none (`findBlockSCI0` would take the first; CONFIRMED that LSCITV keeps the last, script-format.md 4.5) | | |
| Dispatch Table (9) | `SCI_OBJ_EXPORTS` | `count = highest + 1`, wide entries `{offset, 0}`; 0 where there is no fixup | |
| (end) | `SCI_OBJ_POINTERS`, then type 0 | image offsets of fixed-up property words and locals words | |

How slots and references are written:

- **`-dict-` (slot 2).** Classes keep their class number there. An instance gets its `-super-` class
  number. `Object::initSpecies` then stores the class address, which mirrors LSCITV copying the
  class's dictionary handle into each instance (CONFIRMED `0358:0511`). `-dict-` comparisons in
  `Obj::isMemberOf`/`isKindOf` keep their result.
- **`-super-`** stays a class number and resolves through `getClassAddress`. The root `Obj` has
  0xFFFF, which becomes `NULL_REG`.
- **`-env-` (slot 0)** stays 0 (see section 10).
- **Property and locals fixups** become target offsets with relocation entries. That covers `name`
  (7,386 counted) and other slots (1,286); `relocateSci0Sci21` gives them the segment.
- **Method and export words** become absolute code offsets. SCI0 does not relocate them.

### 4.2 Code rewrites (instruction lengths never change)

| LSCI bytes | Image bytes | Counted uses |
|---|---|---|
| `40 <item W> <frame B>` `call` | `40 <target - nextPc, int16> <frame>` | 13,072 (0x41: 0) |
| `41 <item W> <frame B>` (LSCITV reads a word in both forms) | `40`, as above, so the length stays 4 | 0 |
| `5A <item W>` `loadID` | `72 <target W>` `lofsa` | 38,480 (0x5B: 0) |
| `74 <item W>` `pushID` | `74 <target W>` `lofss` | 29,443 (0x75: 0) |
| `4C <W>` `_line_`, `4E <W>` `_file_` (and `4D`, `4F`) | `7E <W>` `line` | 1,080 / 104 (0 / 0) |
| `5A`/`74` without a fixup, and `5B`/`75` | `34`/`38` (`ldi`/`pushi`) in the same form, as LSCITV runs them (CONFIRMED) | 0 |
| `7D` `pushSelf` (CONFIRMED shared handler) | `7C`, because ScummVM reads `7D` as the debug `file` opcode with a string operand | 0 |
| `46`/`47` `calle`, `59` `&rest`, branches | unchanged | 748 + 16,542, 4,280 |
| invalid in LSCITV (`lofsa`, `line`, 0x26/0x27 slots, ...) | write fails, naming the item and offset | 0 |

Rewrites are driven by the fixups: every fixup in a Code item sits on the operand of 0x40, 0x5A or
0x74 (CONFIRMED by data). The decode checks that, finds `_line_`/`_file_`, and reports any branch
whose target falls outside its item through `debugC(kDebugLevelScripts)`. The write does not fail on
those branches, because patches fix them after conversion (section 6).

Two run-time operand rows are added to `script_adjust_opcode_formats` for LSCI:

- `calle` takes a **byte** export index (CONFIRMED `09c8:068B`). This row is needed immediately:
  `CC::init` +0x5D runs `calle 0xEA67 export 0` in the word form.
- `&rest` takes a byte (CONFIRMED `09c8:0068`). This row is for exactness only, because 0x58 never
  occurs.

## 5. Engine touch points (upstream files)

| File | Function | Change | Lines |
|---|---|---|---|
| `engine/object.h`, `object.cpp` | `Object()` | `_offset(speciesVarIndex())`: 0 for SCI0/1, 2 for LSCI, 5 for SCI1.1+ | +8 / -1 |
| `engine/script.cpp` | `initializeObjectsSci0` | read the class number at `12 + 2 * speciesVarIndex()` | +1 / -1 |
| `engine/seg_manager.cpp` | `uninstantiateScriptSci0` | read the superclass at `(speciesVarIndex() + 1) * 2` | +1 / -1 |
| `engine/script.cpp` | `Script::load` | replace the intentional stop: parse, write the image, then use it as the source span for size and `copyDataTo`; the patches then run on the image | +12 / -3 |
| `engine/features.cpp` | `detectLofsType`, `detectDoSoundType`, `detectMoveCountType` | LSCI: `SCI_VERSION_1_MIDDLE` (absolute lofs, so wide exports), `SCI_VERSION_1_EARLY` and `kIgnoreMoveCount`. The auto-detection would read the null game object, or `error()` in `getDetectionAddr("Sound")` / `("Motion")`: `nodePtr` exists but no object is named `Sound` or `Motion` (counted, Feb-1994 hub). The move count is read only by `kDoBresen`, which LSCI does not have; without the pin the debugger's `version` command stops with an error | +12 |
| `sci.cpp` | `initGame` | LSCI: `_gameObjectAddress = make_reg32(1, script0->validateExportFunc(0, false))` (export 0 of script 0, CONFIRMED `09c8:1E28`) | +5 |
| `engine/kernel.cpp` | `script_adjust_opcode_formats` | the two rows in 4.2 | +5 |
| `console.cpp` | `cmdVerifyScripts`, `cmdDissectScript` | LSCI branch: parse and convert every script and module, print failures, counts, out-of-item branches, and size plus MD5 per image; `dissect_script` points to `vo`/`scro` for LSCI | +43 |
| `util.h` | `SciSpanImpl` | drop the `friend class ::SpanTestSuite` under `CXXTEST_RUNNING`: cxxtest defines the macro empty, so the `#if` does not compile in a test build, and no test uses the friend | -3 |
| `test/module.mk` | | an `ENABLE_SCI` row linking `engines/sci/libsci.a` | +5 |
| `engine/script_patches.cpp` | `GID_INN`/`GID_TSN` tables | stray-branch patches (section 6) | +70 |
| `engine/kscripts.cpp`, `kernel.h`, `lsci_kernel_tables.h` | `kModuleDispose` | section 7 | +18 |

The predicate is the existing data-detected `ResourceManager::isLsci()`, through `g_sci->getResMan()`
outside the resource layer. There is no new `SciVersion` and no `GameId` check outside
`script_patches.cpp`. `vm.cpp`, `scriptdebug.cpp`, `segment.h` and `script.h` do not change.

## 6. Start-up, resolution and data anomalies

1. `SciEngine::run` calls `findGameObject`, which returns `NULL_REG` for LSCI. `initGame`
   instantiates script 0 and sets the game object from export 0. `detectLofsType` is pinned, so
   `validateExportFunc` does not reach the null game object.
2. Script 0's class chain loads through `initSpecies`/`initSuperClass`. `getClassAddress` uses
   `vocab.996` unchanged, and module n is script `0xE800 + n`. No instance precedes its own class
   (counted: 0), so SCI0's register-as-you-go order matches LSCITV (`0358:0763`). Class 60 is defined
   twice in SierraLand; the last load wins in both interpreters.
3. Globals are script 0's locals (CONFIRMED `[0A4E] = [1FCE]`). `runGame` sends `play` (0x1A, named
   in `vocab.997` of every land), as LSCITV does (CONFIRMED `09c8:1EC6`).
4. Out-of-item branches: 8 sites (`LL/script.1030` and `LL/script.1060` in INN and TSN 2.1,
   `hub/script.150` in TSN). After linearisation they would land in a neighbouring block, so
   `script_patches.cpp` entries retarget each one to the following `toss` (INFERRED intent). The
   patches match image bytes, which equal the disk bytes inside the item.
5. Exports: the data has `highest + 1` entries in every table (counted, 660 of 660), so ScummVM's
   `index >= count` check is exact. interpreter.md 5.3 says that `index == count` reads past the
   table; to be re-checked at `09c8:2061`.

## 7. Lifetime

- `ModuleID` is already `kScriptID`, which uses `validateExportFunc` on wide exports. Exports may
  name objects (402) or classes (12); those work as they do in SCI0.
- `ModuleDispose(n...)` gets a new `kModuleDispose` that runs `kDisposeScript`'s body for each
  argument. ScummVM marks the script deleted and the GC frees it once it is unreachable. That matches
  LSCITV's disposal queue (CONFIRMED queue `0358:019C`, INFERRED flush time). This kernel is needed
  before the second room, because a room that is entered again expects fresh locals.
- Lockers are released by `uninstantiateScriptSci0` through the species-slot fix.
- After a restart or a restore, scripts are converted again. The image is deterministic, so offsets
  are identical. Savegames are not offered (`GUIO_NOLAUNCHLOAD`; INN disables `Save`).
- `ObjectNew` must not map to `kClone`: ScummVM code sets a clone's species to the clone itself
  (`kscripts.cpp`), which would break `-dict-` identity. It needs its own kernel in the object-kernel
  wave.

## 8. Debugger and tooling parity

Because the image is an ordinary SCI0 script, these commands work with no LSCI code:

- `disasm`, `disasm_addr`: `lofsa`/`lofss` print the text or object name, and `callk` uses the LSCI
  kernel names.
- `vo`, `vr`, `bpx`, `bpe`, `bpk`, `bpa`.
- `scro`, `scrs`, `class_table`, `segment_table`.

Parity rests on the layout rule in 4.1, which is implemented twice:

- `tools/lsci_image.py` (new; imports `lsci_format` and `lsci_bytecode`) builds the identical image
  and an item-to-offset map. `--check DIR` converts every container and checks the invariants. `--md5`
  prints one line per script in the format that `verify_scripts` prints.
- `tools/lsci_disasm.py --image` prints each item's image offset, and prints instructions at image
  offsets in the column layout of ScummVM's `disasm`. A listing line and a `disasm_addr` line then
  share an address and can be diffed after the mnemonic map below. Because the layout is deterministic,
  the tool is the item map and `Script` stores no item numbers.
- Mnemonic map: `loadID` = `lofsa`, `pushID` = `lofss`, `_line_` and `_file_` = `line`, and `call`
  operands are relative in ScummVM.

## 9. Fork commit sequence

Each commit builds warning-free on its own (`timeout 590 make -j28`) and passes `check_comments.py`
for `upstream/master..HEAD`. The intentional stop is removed only in F5. Each commit carries the
`Assisted-by:` trailer.

| # | Subject | Content | Gate | State |
|---|---|---|---|---|
| F1 | `SCI: Name the species slot of SCI0 objects` | `Object::speciesVarIndex()` and its three uses; no behaviour change, so it can go upstream | builds | built |
| F2 | `SCI: Pin LSCI feature types and operands` | `detectLofsType`, `detectDoSoundType`, `detectMoveCountType`, the `calle`/`&rest` rows | builds; the stop is unchanged | built |
| F3 | `SCI: Parse LSCI item containers` | `lsci_container.*`, `module.mk` | builds | built |
| F3b | `SCI: Drop a stale test friend from SciSpanImpl` | `util.h`; needed before any SCI header can be compiled by cxxtest | builds; `make test` passes | built (added) |
| F4 | `SCI: Add the LSCI script image writer` | `lsci_script_image.*`, `module.mk`, `test/engines/sci/lsci_script_image.h`, `test/module.mk` | `make test` passes | built |
| F5 | `SCI: Load LSCI scripts as SCI0 script images` | `Script::load` switch-on, `speciesVarIndex()` = 2 for LSCI, game object in `initGame` | M1 | built |
| F6 | `SCI: Verify LSCI scripts in the debugger` | `verify_scripts` branch with MD5s, `dissect_script` message | M0, byte parity | built |
| F7 | `SCI: Fix LSCI branches that leave their item` | `script_patches.cpp` | the patched sites run in a CasinoLand room or in TSN `hub/script.150` | next: no stray site is on the boot path |
| F8 | `SCI: Add the LSCI ModuleDispose kernel` | `kscripts.cpp`, `kernel.h`, `lsci_kernel_tables.h` | lifetime test | next: the boot stops before the first `ModuleDispose` |

Innkeeper commits:

- T1 `tools: add lsci_image` goes before F4, so that the layout rule is proven offline first.
- T2 `tools: show image offsets in lsci_disasm`.
- D2 `docs: describe the LSCI script loader as built`: adds this doc with its results, and
  `scummvm-integration.md` sections 1, 7 and 9 point here.

## 10. Test plan

Builds: the fork as configured, plus an out-of-tree text-console build
(`--enable-text-console --disable-all-engines --enable-engine=sci`) under `work/` for debugger
commands on stdin. Targets: `inn` Feb-1994, the Dec-1993 extra and `tsn` 2.1, each with `lsci_land`
set to default, `SLand` or `LLand`.

| Stage | How | Success |
|---|---|---|
| A. Offline | `lsci_image.py --check work/res` | all containers convert; the 42 non-containers are reported; every image is under 64 KiB; exactly the 8 known stray branches are listed |
| B. Unit | cxxtest, hand-built containers only (no game data) | exact bytes for class + instance + Code with `call`/`loadID`/`pushID`/`_line_` + String + Variables + Dispatch Table; a parse failure for truncation, an unknown tag, a fixup off an operand, an invalid opcode, a class without a dictionary, an object without its leading slots |
| C. Byte parity | `verify_scripts` in all 9 targets | per-script MD5 equals `lsci_image.py --md5`; counts equal `lsci_disasm --stats` (for example Feb-1994 hub 82 + 62, 988 objects, 221 classes, 4,803 strings, 3,687 code items; LL 57 of 71 with 14 non-containers) |
| D. Start-up (M1) | `-d 1 --debugflags=OnStartup`; `segment_table`, `version`, `vo CC`, `scro 0`, `scrs 0` | script 0 is at segment 1 and `type31.608` is loaded; lofs type "SCI1 middle"; `CC`'s properties and 8 methods are at the offsets `--image` gives; 15 objects and 33 strings |
| E. Edge cases | `type31.630` and `646` (object at item 0); `script.415`, `775`, `790` (two Variables items); `type31.672` and `script.080` (exports to item 0); SL `type31.653` + `script.780` (class 60 twice); `hub/script.406` (largest, calls over 32 KiB apart: step one); SL `script.795` and `902` (`_line_`/`_file_`) | each loads; locals come from the second Variables item; `ModuleID` returns item 0's address; the far call lands on the right `disasm` line |
| F. Boot path (M2) | `bpk *` with logging, `bpx CC::newRoom`, `disasm CC init` diffed against `--image` | no VM-level error (bad opcode, invalid export, instantiation failure, "not a selector", locals or `lofsa` out of range, PC astray); every kernel reached is in kernel-usage.md section 8's start-up list; `newRoom` is reached |
| G. First room script (M3) | continue past `newRoom` | the room script is in `segment_table`; its `init` runs to the first drawing kernel. Drawing itself needs kernel waves 0a-0c |
| H. Other data | stages D-G for Dec-1993, TSN 2.1, SierraLand, CasinoLand | as above; LL and TSN log the stray-branch patches as applied |
| I. Lifetime (after F8) | break at a `ModuleDispose` site, step, `segment_table`, reload by `ModuleID` | marked deleted, freed by the GC, reloaded at identical offsets |
| J. Regression | review that every change sits behind `isLsci()`; F1 is behaviour-neutral; start any local SCI0/SCI1 game | unchanged |

The later oracle is the DOSBox harness (`tools/dosbox/run_client.py`): its screenshots of LSCITV
become the reference for the first drawn screen.

## 11. Open questions and deferred items

| Item | Status | Trigger or next step |
|---|---|---|
| `-env-` value | Stays 0, so it is a number and arithmetic never trips `requireUint16`. Game code does arithmetic on it: `OnLineSignup::advance` (hub `script.012` item 44) runs `pTos -env-; ldi 1; sub; lt?` like an element count. 38 reads counted. Meaning open | Trace `pToa` at `09c8:157B` (object segment `[0A2C]`) before the sign-up flow; the writer can then put a per-script value in slot 0 |
| `-dict-` holds the class address | CONFIRMED safe for `isMemberOf`/`isKindOf`; 58 other reads not inspected | Inspect them when a divergence points there |
| String header dropped | `size` is always `strlen + 2` for literals (CONFIRMED, 35,397 strings) | The LSCI `String`/`Array` kernels compute capacity from it, and strings they create must also point at the text |
| Locals on inherited methods | ScummVM uses the locals of the method owner's script. INFERRED that LSCITV does the same | Check `09c8:1961`..`1AEE` |
| `disasm` call target for calls over 32 KiB apart | The VM is correct (16-bit wrap). The `Script_SRelative` printer masks with `kOffsetMask` (0x7FFFF), and `bt` prints the unwrapped `debugLocalCallOffset`, so both show wrong targets for the 99 such calls in 14 containers (counted; Feb-1994 hub: `script.406`, `410`, `050`, `012`) | Mask to 16 bits below SCI3 in the printer if it gets in the way while debugging |
| LSCI mnemonics and item labels in ScummVM's disassembler | Cosmetic; `--image` covers them | When the mnemonic map starts to confuse |
| `-info-` bit 0x0004 (class 60) | Unknown; SCI0 code ignores it | When class 60's behaviour diverges |
| Detectors that look up named classes | `detectMoveCountType` (`Motion`) is pinned in F2, because the debugger's `version` command reached it. No other detector ran in the boot of the nine targets | Pin the next one when a kernel reaches it |
| `GuestAdditions::patchGameSaveRestore` | No log line at start-up in any of the nine targets (observed) | None |
| `logkernel` prints `k(null)` for stub kernels | ScummVM code: `Kernel::mapFunctions` leaves `name` unset for unmapped kernels; `kStub`'s own warning carries the name, so the boot log in section 12 uses it | Set the name in the fallback branch if the debugger output gets in the way |
| Where the boot stops | `CC::init` reads `TSN(0)` and shifts the result; the `TSN` stub leaves the accumulator unchanged (an object), so `shr` stops the VM (section 12) | Implement `kTsn` sub-op 0 against a virtual TSNEXEC (ktsn.md section 10) |

## 12. Results as built (2026-10-05)

Builds: the in-tree fork build and the text-console build in `work/build-textcon` are warning-free
after every commit F1-F6. Debugger runs use a scratch `scummvm.ini` with one target per set and land
(`lsci_land` = default, `SLand`, `LLand`) and `-d 1 --debugflags=OnStartup`, with commands on stdin.

| Stage | Result |
|---|---|
| A. Offline | `tools/lsci_image.py --check work/res`: 1,292 converted, the 42 CasinoLand view resources not converted ("fixup outside its payload" or "fixups run past the end"), largest image 55,118 bytes (`hub/script.406`, both INN sets), exactly the 8 known stray branches |
| B. Unit | `make test`: 409 tests pass, among them the 3 LSCI tests (image bytes of a hand-built container; truncation, unknown tag, class without dictionary and object without its leading slots rejected by the parser; fixup off an operand and invalid opcode rejected by the writer) |
| C. Byte parity | `verify_scripts` in all nine targets prints 1,165 image MD5s (144, 131, 120 per INN land; 129, 127, 119 for TSN 2.1), identical line for line to `lsci_image.py --md5`; LL reports its 14 view resources and 2 stray branches, TSN `hub` 1 |
| D. Start-up | `version`: lofs type "Middle SCI1", sound "Early SCI1", move count "ignore". `segment_table`: `script.000` at segment 1 with its locals at 2, then 15 modules and `script.003`. `vo CC`: 12 properties named from the dictionary, 8 own methods and 36 inherited ones down to `Obj`. `scro 0`: 15 objects; `scrs 0`: 33 strings |
| E. Edge cases | A temporary debugger loop (not committed) instantiated every container of all nine targets (1,165 scripts and modules) without an error; `script.415` gets 45 locals from its second Variables item, and `script.080` and `type31.672` load with their item-0 exports |
| F. Boot | All nine targets run `play` and the game object's `init` and stop at the same place (below) |

Disassembler parity (T2). `disasm` and `disasm_addr` in ScummVM were diffed against
`tools/lsci_disasm.py --image` on address and instruction bytes; mnemonics differ only by the map in
section 8.

| Method (Feb-1994 hub) | Image offset | Instructions | Same addresses and bytes |
|---|---|---|---|
| `CC::init` (`script.000` item 17) | `0372` | 466 | yes |
| `export_1` (`script.000` item 72, 3 `call`s) | `1816` | 245 | yes |
| `musicDone::doit` (`script.000` item 5, branches) | `0276` | 12 | yes |
| `Obj::perform`, `Obj::isKindOf`, `Obj::invokeMethod` (`type31.608`, `&rest`) | `0078`, `00c8`, `0226` | 7, 22, 13 | yes |

Boot log, Feb-1994 Clubhouse (`./scummvm --path=../work/ex/inn_cd inn`), stub kernels in call
order, as `kStub` logs them:

1. `SetCursor(0x381, 0, 2)` from `CC::setCursor`, which `play` sends after the mapped `HaveMouse`.
2. `Resource` 8 times from `CC::init`: `(0|2, 135, 0|3|999)` and `(0|2, 128, 897)`.
3. `Palette` 16 times, all sub-op 5, from the `calle 0xEA67 export 0` in `CC::init`.
4. `TSN(0)` (GetStatus, ktsn.md). `CC::init` shifts the result right by 8; the stub left an object
   in the accumulator, so the VM stops with `[inn 0 CC::init @ 03f7]: Invalid arithmetic operation
   (shift right - params: 0012:07d4 and 0000:0008)` and opens the debugger. `bt` shows
   `CC::play` (`script 60023`) calling `CC::init`.

SierraLand and CasinoLand, and the Dec-1993 and TSN 2.1 builds, hit the same four kernels in the same
order and stop at the same `TSN(0)` shift in their own game object's `init` (`SL::init`,
`LL::init`). No other VM error, warning or detector failure occurs before that point.

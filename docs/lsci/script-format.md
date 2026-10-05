# LSCI script and module resources

This document covers the resource format of LSCI scripts (type 2) and type-31 modules, the vocab
resources 994-998 that go with them, and how all of it compares with ScummVM's SCI loaders.
Opcode semantics and the VM register map are in `docs/lsci/interpreter.md`. Tools:
`tools/lsci_format.py` (parser), `tools/lsci_bytecode.py` (opcode table and decoder),
`tools/lsci_land.py` (class catalog), `tools/lsci_disasm.py` (disassembler and coverage report).

Addresses are `seg:off` in the unpacked Feb-1994 image `work/exe/LSCITV_inn_cd.EXE` (DGROUP =
`2074`). The file offset is `0x30E0 + seg*16 + off`.

## 1. Summary

- CONFIRMED: a script is **not** an SCI0/SCI1.1/SCI32 block or heap/hunk image. It is a counted list
  of independent **items**. Each item becomes its own memory block with its own handle, and the
  item's tag byte is the interpreter's memory-block type (`Object`, `Class`, `Code`, `Variables`,
  `String`, `Dispatch Table`, `Property Dictionary`, ...).
- CONFIRMED: words that refer to other items hold **item numbers** on disk. A per-item fixup list
  names those words, and the loader replaces each with the target item's run-time handle.
- CONFIRMED: a type-31 module uses the same container. Modules hold the class library (`Obj` and
  so on). The class table (`vocab.996`) refers to module `n` as script number `0xE800 + n`, and
  scripts pass the same numbers to `ModuleDispose`.
- CONFIRMED: `vocab.996` (class table), `vocab.997` (selector names) and `vocab.998` (opcode names)
  have the SCI layouts that ScummVM already reads. `vocab.994` (kernel property offsets) is loaded
  but ScummVM can ignore it, as it does for SCI.
- The disassembler decodes every one of the 1,292 containers in the four sets with no decode
  errors (2,207,389 instructions, section 10). 42 resources are not containers: they are
  view-format data stored under script numbers in CasinoLand.

## 2. The loader

| What | Where | File offset |
|---|---|---|
| Load script by number, create its module node (block type 13) | `0358:0057` | 0x066B7 |
| Load resource type 0x82 (script); word 0 = item count; create the handle array | `0358:00CA`, `0358:02B3` | 0x0672A, 0x06913 |
| Item loop: read tag and length, copy payload into a new block, bind it to handle `i` | `0358:0323`..`04E7` | 0x06983 |
| Tag dispatch (jump table, 9 words for tags 2..10) | `0358:0437`, table `0358:043C` | 0x06A9C |
| Apply fixups unless tag is 7, 8 or 10 | `0358:0495`, `0358:058F` | 0x06AF5, 0x06BEF |
| Register object or class; resolve superclass | `0358:0511`, `0358:0763` | 0x06B71, 0x06DC3 |
| Word-swap hook for tags 2, 3, 6, 9, 10, 11 (empty loop on DOS) | `0358:0417` calls `0358:0820` | 0x06E80 |
| Load class table from `vocab.996` | `0358:0708` | 0x06D68 |
| Copy `vocab.994` (70 bytes) to `DGROUP:0528`..`056D` | `0358:07A4` | 0x06E04 |
| Memory-block type names (far pointers, index = type) | `DGROUP:024A` | 0x23A6A |

- CONFIRMED (`0358:0391`..`03F9`): for item `i`, the loader reads `tag = byte`, `length = word`,
  allocates `length` bytes, copies the payload and binds the block to `handles[i]`. The handle array
  is allocated in advance with one fresh handle per item (`0358:02B3`), so forward references work.
- CONFIRMED (`0358:047E`): a class (tag 3) is typed 2 (`Object`) in the handle table after loading.
- CONFIRMED (`0358:0495`): tags 7 (String), 8 (Said Spec) and 10 (Synonym Table) have no fixup
  list on disk; every other tag is followed by one.

## 3. Container layout

```
word  itemCount
item[itemCount]:
    byte  tag
    word  length
    byte  payload[length]
    if tag not in {7, 8, 10}:
        word  fixupCount
        word  fixupOffset[fixupCount]     ; byte offsets inside payload
```

- CONFIRMED (`0358:058F`): for each fixup offset `o`, `payload[o..o+1] = handles[payload[o..o+1]]`.
  On disk the word holds an item number (0-based, in file order).
- CONFIRMED by data: the header count equals the number of items in all 1,292 containers, and no
  container has trailing bytes.
- Example, start of `type31.608`: `21 00` (33 items), then `07 08 00 | 01 00 05 00 4F 62 6A 00`
  (item 0: a String "Obj"), then `04 18 00 | 3F 01 ...` (item 1: Code, `link 1`).

| Tag | Block type name in LSCITV | Payload | Fixups | Seen in data |
|---|---|---|---|---|
| 2 | Object | instance (section 4.1) | yes | 7,295 |
| 3 | Class | class (section 4.1) | yes | 1,930 |
| 4 | Code | bytecode of one method or procedure | yes | 28,636 |
| 6 | Variables | locals; script 0's are the globals | yes | 334 |
| 7 | String | `word kind (=1)`, `word size`, NUL-terminated text | no | 35,397 |
| 8 | Said Spec | (parser) | no | 0 |
| 9 | Dispatch Table | exports | yes | 660 |
| 10 | Synonym Table | (parser) | no | 0 |
| 11 | Property Dictionary | selector of each class property | yes (always 0) | 1,930 |

Tags 0, 1, 5 (`Free`, `Allocated`, `Illegal: B_COMPILE`) and 12 and above (`Resource`, `Module
Node`, `List`, `Array`, ...) are run-time block types and never appear in resources.

## 4. Item kinds

### 4.1 Object and Class

```
word property[size]              ; property 1 is size itself
word methodCount
word methodSelector[methodCount]
word methodCode[methodCount]     ; item numbers of Code items (fixed up)
```

- CONFIRMED by data: `length == 2*size + 2 + 4*methodCount` for all 9,225 objects and classes.
  Unlike SCI0/1 there is no zero word between the selector list and the code list.
- CONFIRMED (send path, see interpreter.md section 5.3): method lookup reads `{count, selector[count],
  code handle[count]}` right after the properties.

| Slot | Name in vocab.997 | On disk | After loading |
|---|---|---|---|
| 0 | `-env-` | 0 | handle of the module node (`0358:0511`, CONFIRMED) |
| 1 | `-size-` | property count | unchanged |
| 2 | `-dict-` | class: class number; instance: 0xFFFF | handle of the property dictionary (CONFIRMED) |
| 3 | `-super-` | class number of the superclass; 0xFFFF for the root `Obj` | handle of that class (`0358:0763`, CONFIRMED) |
| 4 | `-info-` | 0x8000 for classes, 0 for instances | unchanged |
| 5 | `name` | item number of a String (fixed up), 0xFFFF, or 0 (no fixup, 741 items) | handle, or unchanged |

- CONFIRMED (`0358:0451`..`0460`, `0358:0511`): for tags 2 and 3 the loader calls the register
  routine with the handle of the **next** item. For a class, the routine first stores the class handle
  in the class table at index `property[2]`, then overwrites `property[2]` with that next handle,
  which is the class's Property Dictionary. For an instance, `property[2]` becomes a copy of its
  class's `property[2]`.
- CONFIRMED by data: every Class item is immediately followed by its Property Dictionary
  (1,930 of 1,930).
- CONFIRMED by data: class instance properties follow the class layout. Slots 0-5 always have the
  selector names `-env- -size- -dict- -super- -info- name` in every dictionary.
- CONFIRMED by data: only class 0x3C (`type31.653`, one per set) has `-info-` 0x8004; what bit 0x0004
  means is open.
- Many module classes have `name` = 0xFFFF, so their names are not in the data. The tools call them
  `class_<number>`.

### 4.2 Property Dictionary

- CONFIRMED: the first `size` words are the property selectors of the class that precedes it.
- INFERRED: the remaining 2-10 bytes are compiler slack. They are usually `00 00`, sometimes stale
  bytes such as `.. B4 00 00` or `.. B5 00 00`. The fixup count is always 0.

### 4.3 Code

- CONFIRMED: one Code item per method or procedure. Execution starts at offset 0 of the item's
  block, and branches are relative within it (interpreter.md section 5.3).
- CONFIRMED by data: every fixup inside a Code item sits on an operand of `call` (0x40, target Code
  item), `loadID` (0x5A, target Object or String) or `pushID` (0x74, target String). Conversely, every
  `call`, `loadID` and `pushID` in the data has a fixup, and all use the word form.
- CONFIRMED (handler table `DGROUP:0860`): `loadID` and `pushID` run the `ldi` and `pushi` handlers,
  so after fixup they simply load or push a handle. These are the vocab.998 names; ScummVM calls the
  same opcodes `lea` and `lofss`.
- `_file_` (0x4E) stores its operand in `[1EF6]` (`09c8:00E2`) and holds the script number of the
  resource itself (0x0386 in `script.902`, 0xEA2F in Dec-1993 `SL/type31.559`; CONFIRMED by data).
  `_line_` (0x4C, `09c8:00F3`) stores to `[1EF4]`.
  `_line_` (0x4C) operands are source line numbers. Only three resources use them: Dec-1993
  `SL/type31.559`, Feb-1994 `SL/script.795` and `SL/script.902`.

### 4.4 String

- CONFIRMED by data: `word 1`, then `word strlen+2`, then the text and its NUL. The second word is
  always the text length plus 2 (35,397 strings). INFERRED: it is a capacity field.

### 4.5 Variables

- CONFIRMED (`lal` handler `09c8:098C`, segment load at `09c8:099E`, file 0x0D6FE): local `k` is
  the word at byte `2k` of the block. Slot 0 is the count word itself.
- CONFIRMED by data: word 0 = `n`; code uses locals 1..n-1, never local 0 or global 0, and the
  highest local used is `n-1` in 266 of 286 resources that use locals (counting the last Variables
  item, see below) and lower in the other 20.
- INFERRED: the words from slot `n` on (at least one per item) are compiler slack. They often
  contain stale bytes.
- CONFIRMED (`0358:0465`): the loader stores the Variables handle in module node word 8, so the
  last Variables item of a container wins. The Variables of script 0 are the globals.
- CONFIRMED by data: `script.415` (hub), `script.775` and `script.790` (SierraLand) have two
  Variables items in every set. The first is 73 zero slots (156 bytes); code uses only the second.

### 4.6 Dispatch Table

- CONFIRMED (`09c8:2061`, file 0x0EDC1): word 0 = highest valid export number (a larger index is
  fatal "Dispatch number too large"), then export `k` at byte `2 + 2k`, holding a handle.
- CONFIRMED (`0358:046E`): its handle goes to module node word 6. Export 0 of script 0 is the game
  object, and in the Feb-1994 hub that is a class item (`CC`).
- CONFIRMED by data: unused entries are 0 with no fixup. The exceptions are `type31.672` (both
  entries) and `script.080` (export 8) in every set: their 0 entries do carry fixups, so they point
  at item 0.

### 4.7 Said Spec and Synonym Table

Never present in these sets. `0358:0477` stores a Synonym Table handle in module node word 10.

## 5. The module node

CONFIRMED (`0358:0057`, `0358:00CA`, `0358:0323`): every loaded script is a 12-byte block of type
13 (`Module Node`):

| Word (byte offset) | Content |
|---|---|
| 0 | script number |
| 2 | item count |
| 4 | handle of the handle array (one word per item) |
| 6 | Dispatch Table handle |
| 8 | Variables handle |
| 10 | Synonym Table handle |

## 6. Type-31 modules

- CONFIRMED by data: type-31 resources use the item container exactly as scripts do. 629 of 629
  parse.
- CONFIRMED by data: class-table entries name scripts `0xE800 + n` for module `n`, for example
  class 0 (`Obj`) = 0xEA60 = `type31.608`. All class-table entries of that form resolve to an
  existing module, except the modules listed in section 8.
- INFERRED: there is no separate module loader. Script loading asks for type 0x82, and the resource
  key is built as `type << 11 | number` (`1ad1:02C8`, file 0x1E0B8), so `(2 << 11) | 0xE800 + n` =
  `(31 << 11) | n`, which is the type-31 entry.
- CONFIRMED by data: scripts call `ModuleDispose(scriptNumber)` with both plain numbers (0x8D, 0x50,
  0x384) and module numbers (0xEAB1, 0xEA7F), and `ModuleID(scriptNumber, export)` (most often
  `(0x0B, 0)`, `(0x8D, 0)`, `(0, 4)`). "Module" is LSCI's word for any loaded script.
- CONFIRMED (`0358:0838`, `09c8:1FE6`): `ModuleID(script, export = 0)` loads the script if needed and
  returns that export's handle, which is `ScriptID`. CONFIRMED (`0358:019C`): `ModuleDispose` takes
  any number of scripts and queues each loaded one in a disposal list at `DGROUP:057E` (100
  entries), where `DisposeScript` frees at once. INFERRED: the interpreter flushes the list later.

## 7. Vocab resources

| Resource | Layout | Use | Status |
|---|---|---|---|
| `vocab.994` | 35 words | byte offsets of kernel-accessed properties. Entries 0-20 index the Actor layout (`x xLast y yLast z signal view loop cel priority ...`), the rest a rectangle-style layout | loaded at `0358:07A4` CONFIRMED; meaning INFERRED |
| `vocab.995` | `word, word, ...` then help text | built-in debugger help ("step across send/call", ...) | INFERRED |
| `vocab.996` | per class `{word 0, word script}` | class table; 236 classes in the Feb-1994 hub | CONFIRMED (`0358:0708`, `09c8:0800`) |
| `vocab.997` | `word n`, `word offset[n+1]`, names as `{word len, bytes}` | selector names, `n` = highest selector | CONFIRMED by data |
| `vocab.998` | `word 128`, `word offset[128]`, entries `{word len+2, word type, bytes}` | opcode names: LSCI's own (`loadID`, `pushID`); 0x26, 0x27, 0x29, 0x2F, 0x39 and 0x3F are blank (row = `opcode >> 1`; 0x29, 0x2F, 0x39, 0x3F have the bad-opcode handler, 0x26 and 0x27 are `_line_` and `_file_`) | CONFIRMED by data |
| `vocab.000`, `900`, `901` | SCI0 parser vocabularies | present, but no Said Spec items exist | INFERRED |

- CONFIRMED by data: `vocab.997` has `n + 1` entries. For example, in the Feb-1994 hub the count
  word is 906 and entry 906 is `pass`. ScummVM's `Kernel::loadSelectorNames` already makes this
  `+1` adjustment.
- CONFIRMED by data: the first 561 selectors are identical in every INN and TSN 2.1 land. The
  `tsn_basic` hub vocabulary shares only the first 517 (it has `nmpGameQueue` where the others have
  `nmpSystemQueue`), so selector numbers are not portable to it. The Feb-1994 and
  Dec-1993 `SL/vocab.997` hold only those 561, even though SierraLand code uses 261 selectors above
  them; the TSN 2.1 SierraLand vocab has 809. The Feb-1994 and Dec-1993 hub vocabularies lack 2
  selectors (907 and 908) that hub code uses.

## 8. Anomalies in the data

- **Non-containers.** CasinoLand `script.1904`, `1909`, `2004`..`2012`, `2014`, `2019` and `2020`
  (14 per CasinoLand set, 42 in total) start `01 80 00 00 00 00 00 00 0A 00` (`script.2014` starts
  `02 80 ... 0C 00`: two loops). This is the SCI1 view header, the same as `view.000`'s (loop count,
  flags 0x80, loop offsets), so INFERRED: these are view data under script numbers.
- **Branches outside their item** (8): `LL/script.1030` and `LL/script.1060` in each INN set
  and in TSN 2.1 (6), and `hub/script.150` in TSN 2.1 and basic (2). All have the same shape: the last
  `switch` case ends `dup; ldi N; eq?; bnt X; toss; ret`. X should reach the `toss` just after, but
  it points past the item (+797 in Feb-1994 `LL/script.1030` item 85) or before it (-4445 in basic
  `hub/script.150` item 22). INFERRED: a compiler offset bug. The jump is taken only when no case
  matched.
- **Missing class scripts.** The class tables name scripts that are not shipped in that land:
  modules 645, 647, 660, 661 and 688 everywhere; 624-626 and 690 in the hub and CasinoLand; 648 in
  the hub; 640 in SierraLand; script 8921 in INN SierraLand; scripts 480 and 481 in CasinoLand;
  script 781 (class 181) in every SierraLand set; and script 1061 (classes 188-192) in INN
  CasinoLand. CONFIRMED by data: classes 181 and 188-192 are defined in `script.780` and
  `script.1060`, one number lower, so the table entry is off by one and nothing is missing. The 16
  instances of class 191 in INN `LL/script.1060` follow their class item, so the loader finds the
  class already registered (`0358:0763`) and never asks for script 1061.
- **Class defined twice.** Class 60 is in `type31.653` in every land and also in `SL/script.780`;
  the SierraLand class table names script 780 for it, the other lands name `0xEA8D` (module 653).
- **Slack bytes.** Variables and Property Dictionary items carry stale bytes after their defined
  content (INFERRED compiler buffer reuse). A reimplementation must not read them as data.

## 9. ScummVM comparison

ScummVM's SCI loaders (`engine/script.cpp`: `Script::load`, `identifyOffsets`,
`relocateSci0Sci21`, `initializeLocals`, `initializeObjects*`; `engine/object.cpp`: `Object::init`;
`engine/seg_manager.cpp`: `createClassTable`, `getClassAddress`, `instantiateScript`) expect one
contiguous buffer per script, with offsets into it. LSCI has none: everything is an item number.

| ScummVM facility | LSCI | Verdict |
|---|---|---|
| `SegManager::createClassTable` (`vocab.996`, 4-byte entries, script at +2) | same layout | reuse as is |
| `SegManager::getClassAddress` (0xFFFF = null, load the class's script on demand) | same rule (`0358:0763`) | reuse; script numbers of 0xE800 and above need the type-31 mapping in the resource lookup |
| `Kernel::loadSelectorNames` (`vocab.997`, count + 1) | same layout | reuse as is |
| `Object` slot accessors (`_offset`: species, superclass, info, name at `_offset`..`_offset+3`) | species (`-dict-`) 2, super 3, info 4, name 5 | reuse with `_offset = 2` |
| `Object::init` (SCI0/1 header, method block with zero terminator) | properties from the item, selectors from the next item, method block without terminator | LSCI branch |
| `Script::load` / `identifyOffsets` (SCI0 blocks, SCI1.1 heap/hunk, SCI3) | item container | LSCI path: linearise the items into one buffer, word-aligned, and keep an item-number → offset table |
| `relocateSci0Sci21` (list of offsets whose values get the segment) | fixup list per item | LSCI path that fills the same relocation step: for each fixup, rewrite the item number as the target's offset in the linear buffer; for heap words, also mark them for relocation |
| `call` (SRelative) | word item number | at load, rewrite into a PC-relative offset, so the existing `op_call` works unchanged (all `call`s use the word form) |
| `lea` / `lofss` (`findOffset`, `detectLofsType`) | `loadID` / `pushID` hold item numbers | rewrite into linear offsets and treat them as absolute `lofsa`/`lofss` (the `SCI_VERSION_1_MIDDLE` lofs type) |
| Export table (`validateExportFunc`, SCI0 count word) | highest-index word, then handles | LSCI branch (off by one, entries are item numbers) |
| `initializeLocals` | slot 0 = count, locals 1..n-1 at byte 2k | reuse with the Variables item as the locals block; local 0 is never used |
| Branch opcodes | relative inside the Code item | unchanged after linearising; the 8 out-of-item branches need a script patch or a bounds check |
| Opcode formats (`g_base_opcode_formats`, `script_adjust_opcode_format`) | changes listed in interpreter.md section 5.3 | new adjustment for the LSCI version |

## 10. Disassembler and coverage

```
.venv/bin/python tools/lsci_disasm.py work/res/inn_feb94/hub --script 0      # one script
.venv/bin/python tools/lsci_disasm.py work/res/inn_feb94/hub --module 608    # one module
.venv/bin/python tools/lsci_disasm.py work/res --stats [--verbose]           # every land
```

The listing prints every item in order. Objects and classes show named properties, the superclass
name and the method dictionary. Code is disassembled with:

- item references resolved: strings quoted, objects and code by name;
- property operands named from the owning object's class dictionary (classes defined in the same
  module are looked up before the class table);
- `callk` with the kernel name. Kernel names are taken from `work/exe/LSCITV_inn_cd.kernel.txt` and
  printed as provisional, although interpreter.md now confirms the order;
- `pushi` annotated with the selector name, as ScummVM does.

Opcode mnemonics come from the land's `vocab.998`, falling back to the table in
`tools/lsci_bytecode.py`. Problems are reported as diagnostics and never abort the run.

Coverage over all four sets (`--stats`, 2026-10-04):

| Set/land | Scripts parsed | Modules parsed | Code items | Instructions | Objects | Classes | Strings | Selectors without a name | Diagnostics |
|---|---|---|---|---|---|---|---|---|---|
| inn_feb94/hub | 82/82 | 62/62 | 3,687 | 289,228 | 988 | 221 | 4,803 | 2 | none |
| inn_feb94/SL | 67/67 | 64/64 | 2,805 | 241,180 | 722 | 197 | 3,603 | 261 | none |
| inn_feb94/LL | 57/71 | 63/63 | 2,677 | 198,723 | 688 | 179 | 3,176 | 0 | 14 not containers, 2 out-of-item branches |
| inn_dec93/hub | 82/82 | 62/62 | 3,619 | 283,924 | 967 | 221 | 4,724 | 2 | none |
| inn_dec93/SL | 67/67 | 64/64 | 2,758 | 237,046 | 702 | 197 | 3,566 | 261 | none |
| inn_dec93/LL | 57/71 | 63/63 | 2,631 | 195,288 | 668 | 179 | 3,120 | 0 | 14 not containers, 2 out-of-item branches |
| tsn21/hub | 67/67 | 62/62 | 2,701 | 189,561 | 656 | 181 | 3,316 | 0 | 1 out-of-item branch |
| tsn21/SL | 63/63 | 64/64 | 2,548 | 202,779 | 628 | 196 | 2,959 | 0 | none |
| tsn21/LL | 56/70 | 63/63 | 2,595 | 190,419 | 651 | 178 | 2,979 | 0 | 14 not containers, 2 out-of-item branches |
| tsn_basic/hub | 65/65 | 62/62 | 2,615 | 179,241 | 625 | 181 | 3,151 | 0 | 1 out-of-item branch |
| **all** | **663/705** | **629/629** | **28,636** | **2,207,389** | **7,295** | **1,930** | **35,397** | 526 | 42 not containers, 8 out-of-item branches |

- No decode errors, no unknown opcodes, no unknown tags, no fixups off an operand, and no Code item
  that falls off its end without `ret` or `jmp`.
- 83 distinct opcodes are used. Never used: the unsigned comparisons, `sTop`, `ipTos`, `dpTos`,
  most indexed and stack forms of variable access, and the invalid slots.

## 11. Open questions

- When the interpreter flushes the `ModuleDispose` list at `DGROUP:057E` (track A3).
- Whether the SierraLand selector names above 560 from TSN 2.1 apply to the Feb-1994 SierraLand
  code. Their positions and counts differ, so this is unverified.
- Where the CasinoLand view-format "scripts" are used, and whether their real type in the resource
  map differs from the one `tools/sci_res.py` reports.
- Meaning of `-info-` bit 0x0004 and of the `vocab.995` header words.

## 12. Verification

An independent pass redid these checks from the binaries and resources, with a separate container
parser and a capstone disassembly of `LSCITV_inn_cd.EXE` (2026-10-04).

- CONFIRMED: the container layout parses all 1,292 scripts and modules with no trailing bytes. Item
  counts per tag match section 3 (2: 7,295; 3: 1,930; 4: 28,636; 6: 334; 7: 35,397; 9: 660; 11:
  1,930). All Property Dictionary fixup counts are 0.
- CONFIRMED in code: item loop (`0358:0391`..`04E7`), tag dispatch table at `0358:043C`
  (2,3 -> register; 6 -> node word 8; 9 -> word 6; 10 -> word 10), fixup skip for tags 7, 8, 10,
  word-swap hook set {2, 3, 6, 9, 10, 11}, hook body is an empty loop, class retyped 3 -> 2, fixup
  routine `0358:058F` (`payload[o] = handles[payload[o]]`), register routine `0358:0511`, class
  table lookup and on-demand load `0358:0763`, `vocab.994` copy of 70 bytes, `vocab.996` load.
- CONFIRMED in code: `DGROUP:024A` block-type names, `Dispatch number too large` bound check at
  `09c8:2061`, `loadID` and `pushID` sharing the `ldi` and `pushi` handlers, `call` always reading a
  word and a byte, `lal` reading `[locals_seg + 2k]`, `_line_` and `_file_` handlers and the
  `opcode >> 1` handler indexing, resource key `(type << 11) | number` at `1ad1:02C8`.
- CONFIRMED by data: object/class length formula (9,225 of 9,225), every class followed by its
  dictionary, class numbers match the class table, every `call`, `loadID` and `pushID` operand has a
  fixup and no other code word does, fixup targets are Code, Object or String as stated, strings
  (35,397 of 35,397), dispatch table length, the 8 out-of-item branches, 42 non-containers,
  the `_file_` and `_line_` users, `vocab.997` and `vocab.998` layouts, tool opcode names equal
  `vocab.998` and tool opcode validity equals the handler table.
- Corrected: `name` can be 0; two Variables items in three scripts; `script.080` dispatch entry;
  `lal` handler address; locals statistic (266 of 286); `ModuleID` and `ModuleDispose` handler
  semantics; the 561-selector claim (not true for `tsn_basic`); missing-class list (781, and the
  1061 and 781 entries are off-by-one table entries); the false "unresolved class" diagnostic
  (`tools/lsci_disasm.py` now resolves classes of the module being listed first); the non-container
  header (`script.2014` differs).
- Not independently reproduced: the `vocab.994` meaning, the `vocab.995` header, the `-info-` bit
  0x0004, and the instruction totals in section 10 (taken from the tool; item and tag counts match).

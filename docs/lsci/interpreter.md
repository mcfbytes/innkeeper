# LSCI interpreter map (LSCITV.EXE)

Kernel table, kernel calling convention, bytecode opcode map and build identifiers of the LSCI
interpreter, with the differences from ScummVM's SCI virtual machine.

Primary binary: `work/exe/LSCITV_inn_cd.EXE` (INN, Feb 1994), LZEXE-unpacked by `tools/unlzexe.py`.
Addresses are `seg:off` in the unpacked load image (segment 0 = first byte after the MZ header);
**file offset = 0x30E0 + seg*16 + off** for this binary. Every table in this document can be
regenerated with `tools/lsci_tables.py <exe>`.

## 1. Summary

- CONFIRMED: the kernel name order in `LSCITV_*.kernel.txt` **is** the dispatch order. Name table
  and handler table are two parallel 89-entry far-pointer arrays; `callk n` indexes the handler
  table directly with `n*4`. `TSN` is kernel **0x54**.
- CONFIRMED: kernel handlers are C far functions taking one far pointer to the argument block
  (`argv[0]` = argc, `argv[1..]` = parameters) and returning their result by writing the VM
  accumulator at `DGROUP:075A`.
- CONFIRMED: the opcode set is SCI's 128-opcode layout (low bit = byte/word operand form) with
  these differences: `call` takes a word code-handle, `calle` and `&rest` changed operand widths,
  `lea` and `lofss` are aliased to `ldi` and `pushi`, `lofsa` and `line` (0x7E) are invalid,
  and 0x4C / 0x4E are new word-operand debug-marker opcodes (absent in TSN 2.1).
- CONFIRMED: code runs from offset 0 of a per-procedure memory handle, not from an offset inside a
  script buffer. That is the deepest divergence from ScummVM's VM (section 5.3).
- CONFIRMED: kernel names, numbering and count (89) are identical in TSN 2.1, INN Dec-1993 and
  INN Feb-1994. No version-number string exists in any of the three builds.

## 2. Build identification

| Build | Binary | MZ header | DGROUP | LSCITV.EXE date on media |
|---|---|---|---|---|
| INN Feb-1994 | `LSCITV_inn_cd.EXE` | 0x30E0 | 2074 | 1994-02-14 (CD zip, also V2.3 floppy image) |
| INN Dec-1993 | `LSCITV_inn_v2317.EXE` | 0x2F80 | 1F6B | 1993-11-05 (V2.3.17 floppy image) |
| TSN 2.1 | `LSCITV_tsn21_031293.EXE` | 0x24B0 | 1B14 | 1993-02-26 (031293 floppy image) |

- CONFIRMED: DGROUP is loaded at entry, `0000:0085 mov dx,2074 / mov ds,dx` (file 0x3165).
- CONFIRMED: the only interpreter identification string is at `2074:3070` (file 0x26890):
  `Script Interpreter, Copyright (C) 1987-1992 The Sierra Network`, followed by an author line
  (Jeff Stephenson, Bob Heitman, Pablo Ghenis, Gary Butts, Conan Brink). The same string is in all
  three builds (Dec-1993 file 0x253E6, TSN 2.1 file 0x1F766).
- CONFIRMED: `Interpreter:  ` at `2074:00E9` (file 0x23909) is **not** a version banner; it is the
  prefix printed by the fatal-error routine `0000:0043` (file 0x3123) before the message text.
- CONFIRMED: `Version 1.03` (file 0x1E4E0 region, image 0x1B400) belongs to the linked PKWARE Data
  Compression Library copyright, not to the interpreter.
- CONFIRMED: default config name `lsci.cfg` at `2074:219A` (file 0x259BA).
- No `__DATE__`/`__TIME__` strings, no `x.yyy.zzz` SCI version string. The only build date
  evidence is the DOS file timestamp above (INFERRED as the build date).

## 3. Kernel dispatch

### 3.1 Tables

| Table | Feb-1994 | Dec-1993 | TSN 2.1 |
|---|---|---|---|
| Handler far pointers (89 x 4 bytes) | `2074:1CE6`, file 0x25506 | `1F6B:1A48`, file 0x24078 | `1B14:107A`, file 0x1E66A |
| Name far pointers (89 x 4 bytes) | `2074:310A`, file 0x2692A | `1F6B:2E50`, file 0x25480 | `1B14:2210`, file 0x1F800 |
| Highest valid kernel number (word) | `2074:1EC2` = 0x0058, file 0x256E2 | `1F6B:1C24` = 0x0058 | `1B14:1256` = 0x0058 |

- CONFIRMED: both arrays are runs of 89 consecutive relocated far pointers; the name array points
  into a separate string segment (`1C52:0004` = file 0x1F604 for `IsObject`) and is terminated by
  a null far pointer, as is the handler array.
- CONFIRMED: `callk` never reads the name array; only the handler array matters for execution.
  INFERRED: the names exist for the built-in debugger and error messages.

### 3.2 The `callk` opcode (0x42 word / 0x43 byte), handler `09c8:052C` (file 0xD28C)

```
09c8:052C  test al,1 / lodsb|lodsw          ; kernel number, byte if opcode is odd
09c8:0538  cmp ax,[1EC2] / jle ok           ; else fatal "Kernel entry # too large: %d"
09c8:0551  bx = number*4
09c8:0557  lodsb ; sub bp,ax                ; frame size in bytes, excludes argc
09c8:055D  sub bp,[0A44] ; [bp] += [0A44]/2 ; fold in words pushed by &rest
09c8:0579  push ds ; push bp ; lcall [bx+1CE6]
09c8:0649  sub bp,2                         ; drop the argc slot
```

- CONFIRMED: the VM stack grows **upward** in DGROUP and `BP` addresses the top word. The caller
  pushes argc, then the arguments; `callk`'s second operand is the byte size of the arguments.
  After `sub bp,framesize`, `DS:BP` points at the argc word and is passed as the only argument.
- CONFIRMED: kernel numbers are checked with a signed compare against 0x58, so 0..0x58 are valid.
- CONFIRMED: `&rest` adds its extra words to argc before the call (`09c8:0562`), as in SCI.
- CONFIRMED: handler signature is `void far kName(uint16 far *argv)`; handlers start with
  `les bx,[bp+6]` and read `es:[bx]` (argc) and `es:[bx+2*i]` (parameter i). Example `Abs`
  (`07d1:00FC`, file 0xAEEC): `mov ax,es:[bx+2] ; cwd ; xor/sub ; mov es:[075A],ax`.
- CONFIRMED: results are returned by storing into the accumulator `DGROUP:075A`; the AX value on
  return is ignored by the VM. INFERRED: handlers that store nothing leave the accumulator as
  it was before the call (no clearing is done in the `callk` path).
- CONFIRMED: after the call the VM reloads the globals segment (`[075C]` from handle `[0A4E]`), the
  locals segment (`[075E]` from handle `[0A50]`) and `ES` (current code block, handle `[0A4C]`),
  `09c8:05A4` onward, so a kernel may move or purge memory.

### 3.3 Kernel table (Feb-1994)

The argument column lists the `argv` slots the handler reads in its first ~90 instructions
(`argc` = slot 0). It is a lower bound from a mechanical scan, not a signature.

| # | Name | Handler (seg:off) | File offset | Args read in prologue (INFERRED) | Notes (CONFIRMED) |
|---|---|---|---|---|---|
| 00 | IsObject | 17ec:0213 | 0x1b1b3 | 1 |  |
| 01 | ObjectFree | 17ec:0202 | 0x1b1a2 | 1 |  |
| 02 | ObjectNew | 17ec:01a9 | 0x1b149 | argc, 1 |  |
| 03 | ObjectRespondsTo | 17ec:022c | 0x1b1cc | 1,2 |  |
| 04 | Array | 0bf4:0004 | 0x0f024 | 1,2,3,4,5,6 | sub-op args[1], 8 sub-ops (0-7) |
| 05 | Block | 0130:0512 | 0x048f2 | 1 |  |
| 06 | List | 01cc:0718 | 0x054b8 | 1,2 | sub-op args[1], 14 sub-ops (0-13) |
| 07 | ModuleDispose | 0358:019c | 0x067fc | argc |  |
| 08 | ModuleID | 0358:0838 | 0x06e98 | argc, 1,2 |  |
| 09 | Resource | 181c:05c9 | 0x1b869 | argc, 1,2,3,4 |  |
| 0a | Seq | 0276:03a6 | 0x05be6 | 1,2 | sub-op args[1], 7 sub-ops (0-6) |
| 0b | String | 02c4:000a | 0x05d2a | 1,2 | sub-op args[1], 12 sub-ops (0-11); TSN 2.1: 5 (0-4) |
| 0c | GetFarText | 062f:0a88 | 0x09e58 | argc, 1,2,3 |  |
| 0d | CoordPri | 08e1:0ab6 | 0x0c9a6 | argc, 1,2 |  |
| 0e | DrawPic | 08e1:0962 | 0x0c852 | argc, 1,2,3,4 |  |
| 0f | PicState | 08e1:0a41 | 0x0c931 | argc, 1 |  |
| 10 | ShakeScreen | 08e1:0aef | 0x0c9df | argc, 1,2 |  |
| 11 | Show | 08e1:0a87 | 0x0c977 | 1 |  |
| 12 | AddToPic | 0cdf:28c5 | 0x12795 | 1 |  |
| 13 | Animate | 0cdf:1d22 | 0x11bf2 | argc, 1,2 |  |
| 14 | CelHigh | 0cdf:2181 | 0x12051 | argc, 1,2,3,4 |  |
| 15 | CelRect | 0cdf:1dc6 | 0x11c96 | argc, 1,2,3,4,5,6,7 |  |
| 16 | CelWide | 0cdf:20b6 | 0x11f86 | argc, 1,2,3,4 |  |
| 17 | DrawCel | 0cdf:24ef | 0x123bf | argc, 1,2,3,4,5,6,7 |  |
| 18 | DrawScaledCel | 0cdf:264e | 0x1251e | argc, 1,2,3,4,5,6,7,8,9 |  |
| 19 | NumCels | 0cdf:1d8e | 0x11c5e | 1,2 |  |
| 1a | NumLoops | 0cdf:1d5d | 0x11c2d | 1 |  |
| 1b | ScaledCelRect | 0cdf:1ecd | 0x11d9d | argc, 1,2,3,4,8,9 |  |
| 1c | DrawControl | 05d6:04ae | 0x092ee | 1 |  |
| 1d | EditControl | 05d6:04d0 | 0x09310 | 1,2 |  |
| 1e | HiliteControl | 05d6:04bf | 0x092ff | 1 |  |
| 1f | GetPort | 04e0:0f29 | 0x08e09 | - |  |
| 20 | SetPort | 04e0:0f37 | 0x08e17 | argc, 1 |  |
| 21 | WindowDispose | 04e0:0f18 | 0x08df8 | 1 |  |
| 22 | WindowNew | 04e0:0e2c | 0x08d0c | 1 |  |
| 23 | Display | 062f:07b8 | 0x09b88 | argc, 1 |  |
| 24 | TextSize | 062f:0754 | 0x09b24 | argc, 3,4 |  |
| 25 | GetEvent | 0847:0798 | 0x0bce8 | 1,2 |  |
| 26 | GlobalToLocal | 04e0:0dcc | 0x08cac | 1 |  |
| 27 | LocalToGlobal | 04e0:0dfc | 0x08cdc | 1 |  |
| 28 | MapKeyToDir | 0847:07e5 | 0x0bd35 | 1 |  |
| 29 | Menu | 1008:0006 | 0x13166 | 1 |  |
| 2a | HaveMouse | 0847:0822 | 0x0bd72 | - |  |
| 2b | SetCursor | 0847:0833 | 0x0bd83 | argc, 1,2 |  |
| 2c | Memory | 18e5:0b26 | 0x1ca56 | 1 | sub-op args[1], 11 sub-ops (0-10) |
| 2d | CheckSaveGame | 1423:0006 | 0x17316 | - | stub: single `retf` |
| 2e | GameIsRestarting | 1414:006f | 0x1728f | argc, 1 |  |
| 2f | GetSaveDir | 1423:0005 | 0x17315 | - | stub: single `retf` |
| 30 | GetSaveFiles | 1423:0004 | 0x17314 | - | stub: single `retf` |
| 31 | Restart | 1414:0000 | 0x17220 | - |  |
| 32 | Restore | 1410:0034 | 0x17214 | - | fatal "Bad call - RESTORE kernel disabled" |
| 33 | Save | 1410:000c | 0x171ec | - | fatal "Bad call - SAVE kernel disabled" |
| 34 | Sound | 110a:05f1 | 0x14771 | argc, 1,2 | sub-op args[1], 15 sub-ops (0-14) |
| 35 | GetTime | 0f96:000a | 0x12a4a | argc, 1 |  |
| 36 | SetTimerFreq | 0cab:0023 | 0x0fbb3 | argc, 1,2,3 |  |
| 37 | Wait | 0f96:0030 | 0x12a70 | argc |  |
| 38 | Alert | 03de:002c | 0x06eec | 1 |  |
| 39 | CanBeHere | 0cdf:224c | 0x1211c | 1,2 |  |
| 3a | OnControl | 0cdf:2438 | 0x12308 | argc, 1,2,3 |  |
| 3b | FileSystem | 0fac:0008 | 0x12ba8 | 1,2,3 | sub-op args[1], 18 sub-ops (0-17); TSN 2.1: 16 |
| 3c | DeviceInfo | 1423:0007 | 0x17317 | - | stub: single `retf` |
| 3d | Abs | 07d1:00fc | 0x0aeec | 1 |  |
| 3e | CosDiv | 07d1:04c4 | 0x0b2b4 | 1,2 |  |
| 3f | CosMult | 07d1:0462 | 0x0b252 | 1,2 |  |
| 40 | GetAngle | 07d1:035c | 0x0b14c | 1,2,3,4 |  |
| 41 | GetDistance | 07d1:0397 | 0x0b187 | argc, 1,2,3,4,5 |  |
| 42 | Random | 07d1:0006 | 0x0adf6 | argc, 1,2 |  |
| 43 | SinDiv | 07d1:0498 | 0x0b288 | 1,2 |  |
| 44 | SinMult | 07d1:042c | 0x0b21c | 1,2 |  |
| 45 | Sqrt | 07d1:0115 | 0x0af05 | argc, 1,2 |  |
| 46 | Long | 07d1:0168 | 0x0af58 | 2 |  |
| 47 | Graph | 06e4:0bc8 | 0x0aae8 | argc, 1 |  |
| 48 | Debug | 17b2:02a1 | 0x1aea1 | - | stub: acc = 0 |
| 49 | Profiler | 17b2:02ad | 0x1aead | - | stub: calls 17b2:03ac with code 4; `03ac` is a single `retf`, so a no-op |
| 4a | SetDebug | 17b2:02b5 | 0x1aeb5 | - | stub: calls 17b2:03ac with code 5 (no-op) |
| 4b | ShowFree | 17b2:02bd | 0x1aebd | - | stub: calls 17b2:03ac with code 6 (no-op) |
| 4c | StackUsage | 09c8:21eb | 0x0ef4b | 1 |  |
| 4d | Joystick | 0847:0900 | 0x0be50 | 1,2 |  |
| 4e | BaseRectSet | 0cdf:28e9 | 0x127b9 | 1,2 |  |
| 4f | ConfigStr | 181c:08aa | 0x1bb4a | 1 |  |
| 50 | Encrypt | 142d:003c | 0x173ec | 1 |  |
| 51 | Decrypt | 142d:0053 | 0x17403 | 1 |  |
| 52 | ColorUp | 142d:006a | 0x1741a | argc, 1 |  |
| 53 | ColorDown | 142d:008f | 0x1743f | argc, 1 |  |
| 54 | TSN | 16b1:0004 | 0x19bf4 | 1 | sub-op args[1], 17 slots (0-16); 5, 6, 10, 15 fall to the default path 16b1:0318 |
| 55 | ObjPropOffset | 16b1:0b50 | 0x1a740 | 1,2 |  |
| 56 | ObjOffsetProp | 16b1:0b6f | 0x1a75f | 1,2 |  |
| 57 | SID | 16b1:0791 | 0x1a381 | argc, 1,2 | sub-op args[1], 6 sub-ops (0-5) |
| 58 | InvokeMethod | 16b1:0b13 | 0x1a703 | 1,2,3,4 |  |

### 3.4 Other builds

- CONFIRMED: Dec-1993 and TSN 2.1 have the same 89 names in the same order and the same 0x58 limit.
- CONFIRMED: Dec-1993 handler grouping by code segment is identical to Feb-1994 (every handler sits
  at the same relative place, shifted by a few bytes).
- CONFIRMED: TSN 2.1 differs in kernel layout: `String` has 5 sub-ops (0-4) instead of 12 and
  `FileSystem` 16 instead of 18. `TSN` already has 17 sub-op slots in all three builds.
- CONFIRMED: `Save` and `Restore` are **implemented** in TSN 2.1 (`1147:000E`, `1197:000C`, two
  separate segments). Each pushes `argv[1]`, calls the string helper `0243:01E3` and then a local
  worker. In both INN builds they are the disabled stubs described in 3.3
  (Dec-1993 `1401:000A`, `1401:0032`).

Handler addresses in the older builds (same index and name in every row):

| # | Name | Dec-1993 handler | file | TSN 2.1 handler | file |
|---|---|---|---|---|---|
| 00 | IsObject | 171b:020b | 0x1a33b | 1391:01d5 | 0x15f95 |
| 01 | ObjectFree | 171b:01fa | 0x1a32a | 1391:01c4 | 0x15f84 |
| 02 | ObjectNew | 171b:01a1 | 0x1a2d1 | 1391:016b | 0x15f2b |
| 03 | ObjectRespondsTo | 171b:0224 | 0x1a354 | 1391:01ee | 0x15fae |
| 04 | Array | 0be5:0002 | 0x0edd2 | 0993:0000 | 0x0bde0 |
| 05 | Block | 012d:051e | 0x0476e | 00de:04ce | 0x0375e |
| 06 | List | 01c5:0724 | 0x052f4 | 0166:0750 | 0x04260 |
| 07 | ModuleDispose | 0349:019c | 0x065ac | 027b:0175 | 0x04dd5 |
| 08 | ModuleID | 0349:0838 | 0x06c48 | 027b:06fa | 0x0535a |
| 09 | Resource | 174b:05c1 | 0x1a9f1 | 13be:0454 | 0x164e4 |
| 0a | Seq | 0267:03a6 | 0x05996 | 01f7:037e | 0x0479e |
| 0b | String | 02b5:000a | 0x05ada | 0243:0002 | 0x048e2 |
| 0c | GetFarText | 0620:0a8a | 0x09c0a | 050a:082c | 0x07d7c |
| 0d | CoordPri | 08d2:0ab4 | 0x0c754 | 0718:0a7b | 0x0a0ab |
| 0e | DrawPic | 08d2:0960 | 0x0c600 | 0718:0940 | 0x09f70 |
| 0f | PicState | 08d2:0a3f | 0x0c6df | 0718:0a1f | 0x0a04f |
| 10 | ShakeScreen | 08d2:0aed | 0x0c78d | 0718:0ab4 | 0x0a0e4 |
| 11 | Show | 08d2:0a85 | 0x0c725 | 0718:0a4c | 0x0a07c |
| 12 | AddToPic | 0ccf:28d3 | 0x12543 | 0a7e:2641 | 0x0f2d1 |
| 13 | Animate | 0ccf:1d30 | 0x119a0 | 0a7e:1b25 | 0x0e7b5 |
| 14 | CelHigh | 0ccf:218f | 0x11dff | 0a7e:1f31 | 0x0ebc1 |
| 15 | CelRect | 0ccf:1dd4 | 0x11a44 | 0a7e:1bc9 | 0x0e859 |
| 16 | CelWide | 0ccf:20c4 | 0x11d34 | 0a7e:1e7c | 0x0eb0c |
| 17 | DrawCel | 0ccf:24fd | 0x1216d | 0a7e:228c | 0x0ef1c |
| 18 | DrawScaledCel | 0ccf:265c | 0x122cc | 0a7e:23d5 | 0x0f065 |
| 19 | NumCels | 0ccf:1d9c | 0x11a0c | 0a7e:1b91 | 0x0e821 |
| 1a | NumLoops | 0ccf:1d6b | 0x119db | 0a7e:1b60 | 0x0e7f0 |
| 1b | ScaledCelRect | 0ccf:1edb | 0x11b4b | 0a7e:1cb6 | 0x0e946 |
| 1c | DrawControl | 05c7:04b0 | 0x090a0 | 04b1:04b2 | 0x07472 |
| 1d | EditControl | 05c7:04d2 | 0x090c2 | 04b1:04d4 | 0x07494 |
| 1e | HiliteControl | 05c7:04c1 | 0x090b1 | 04b1:04c3 | 0x07483 |
| 1f | GetPort | 04d1:0f2b | 0x08bbb | 03e7:0c69 | 0x06f89 |
| 20 | SetPort | 04d1:0f39 | 0x08bc9 | 03e7:0c77 | 0x06f97 |
| 21 | WindowDispose | 04d1:0f1a | 0x08baa | 03e7:0c58 | 0x06f78 |
| 22 | WindowNew | 04d1:0e2e | 0x08abe | 03e7:0ba0 | 0x06ec0 |
| 23 | Display | 0620:07ba | 0x0993a | 050a:055a | 0x07aaa |
| 24 | TextSize | 0620:0756 | 0x098d6 | 050a:0512 | 0x07a62 |
| 25 | GetEvent | 0838:0796 | 0x0ba96 | 06a6:0515 | 0x09425 |
| 26 | GlobalToLocal | 04d1:0dce | 0x08a5e | 03e7:0b36 | 0x06e56 |
| 27 | LocalToGlobal | 04d1:0dfe | 0x08a8e | 03e7:0b6b | 0x06e8b |
| 28 | MapKeyToDir | 0838:07e3 | 0x0bae3 | 06a6:0562 | 0x09472 |
| 29 | Menu | 0ff9:0004 | 0x12f14 | 0d5b:000e | 0x0fa6e |
| 2a | HaveMouse | 0838:0820 | 0x0bb20 | 06a6:059f | 0x094af |
| 2b | SetCursor | 0838:0831 | 0x0bb31 | 06a6:05b0 | 0x094c0 |
| 2c | Memory | 1813:0b2c | 0x1bbdc | 1461:0a2e | 0x174ee |
| 2d | CheckSaveGame | 1414:0004 | 0x170c4 | 1202:0006 | 0x144d6 |
| 2e | GameIsRestarting | 1404:007d | 0x1703d | 11f3:006f | 0x1444f |
| 2f | GetSaveDir | 1414:0003 | 0x170c3 | 1202:0005 | 0x144d5 |
| 30 | GetSaveFiles | 1414:0002 | 0x170c2 | 1202:0004 | 0x144d4 |
| 31 | Restart | 1404:000e | 0x16fce | 11f3:0000 | 0x143e0 |
| 32 | Restore | 1401:0032 | 0x16fc2 | 1197:000c | 0x13e2c |
| 33 | Save | 1401:000a | 0x16f9a | 1147:000e | 0x1392e |
| 34 | Sound | 10fb:05ef | 0x1451f | 0e57:05b4 | 0x10fd4 |
| 35 | GetTime | 0f87:0008 | 0x127f8 | 0cf7:0002 | 0x0f422 |
| 36 | SetTimerFreq | 0c9c:0021 | 0x0f961 | 0a4a:0025 | 0x0c975 |
| 37 | Wait | 0f87:002e | 0x1281e | 0cf7:0028 | 0x0f448 |
| 38 | Alert | 03cf:002c | 0x06c9c | 02ed:002e | 0x053ae |
| 39 | CanBeHere | 0ccf:225a | 0x11eca | 0a7e:1fe6 | 0x0ec76 |
| 3a | OnControl | 0ccf:2446 | 0x120b6 | 0a7e:21ce | 0x0ee5e |
| 3b | FileSystem | 0f9d:0006 | 0x12956 | 0d0d:0000 | 0x0f580 |
| 3c | DeviceInfo | 1414:0005 | 0x170c5 | 1202:0007 | 0x144d7 |
| 3d | Abs | 07c2:00fe | 0x0ac9e | 0630:00f8 | 0x088a8 |
| 3e | CosDiv | 07c2:04c6 | 0x0b066 | 0630:04c0 | 0x08c70 |
| 3f | CosMult | 07c2:0464 | 0x0b004 | 0630:045e | 0x08c0e |
| 40 | GetAngle | 07c2:035e | 0x0aefe | 0630:0358 | 0x08b08 |
| 41 | GetDistance | 07c2:0399 | 0x0af39 | 0630:0393 | 0x08b43 |
| 42 | Random | 07c2:0008 | 0x0aba8 | 0630:0002 | 0x087b2 |
| 43 | SinDiv | 07c2:049a | 0x0b03a | 0630:0494 | 0x08c44 |
| 44 | SinMult | 07c2:042e | 0x0afce | 0630:0428 | 0x08bd8 |
| 45 | Sqrt | 07c2:0117 | 0x0acb7 | 0630:0111 | 0x088c1 |
| 46 | Long | 07c2:016a | 0x0ad0a | 0630:0164 | 0x08914 |
| 47 | Graph | 06d5:0bca | 0x0a89a | 0599:0756 | 0x08596 |
| 48 | Debug | 16e0:02a9 | 0x1a029 | 1358:0273 | 0x15ca3 |
| 49 | Profiler | 16e0:02b5 | 0x1a035 | 1358:027f | 0x15caf |
| 4a | SetDebug | 16e0:02bd | 0x1a03d | 1358:0287 | 0x15cb7 |
| 4b | ShowFree | 16e0:02c5 | 0x1a045 | 1358:028f | 0x15cbf |
| 4c | StackUsage | 09b9:21e9 | 0x0ecf9 | 07f7:1928 | 0x0bd48 |
| 4d | Joystick | 0838:08fe | 0x0bbfe | 06a6:067d | 0x0958d |
| 4e | BaseRectSet | 0ccf:28f7 | 0x12567 | 0a7e:2665 | 0x0f2f5 |
| 4f | ConfigStr | 174b:08a2 | 0x1acd2 | 13be:0714 | 0x167a4 |
| 50 | Encrypt | 141e:003a | 0x1719a | 120c:003c | 0x145ac |
| 51 | Decrypt | 141e:0051 | 0x171b1 | 120c:0053 | 0x145c3 |
| 52 | ColorUp | 141e:0068 | 0x171c8 | 120c:006a | 0x145da |
| 53 | ColorDown | 141e:008d | 0x171ed | 120c:008f | 0x145ff |
| 54 | TSN | 15df:000c | 0x18d7c | 126a:000c | 0x14b5c |
| 55 | ObjPropOffset | 15df:0b58 | 0x198c8 | 126a:0afd | 0x1564d |
| 56 | ObjOffsetProp | 15df:0b77 | 0x198e7 | 126a:0b1c | 0x1566c |
| 57 | SID | 15df:0799 | 0x19509 | 126a:074e | 0x1529e |
| 58 | InvokeMethod | 15df:0b1b | 0x1988b | 126a:0ac0 | 0x15610 |

## 4. VM register map (Feb-1994)

| Role | Location | Evidence |
|---|---|---|
| Program counter | `ES:SI`, ES = segment of the current code handle | dispatch `09c8:0045 lodsb es:[si]` |
| Current code handle | `DGROUP:0A4C` | set at procedure entry `09c8:001A` |
| Accumulator | `DGROUP:075A` | every arithmetic handler, `09c8:009A` onward |
| prev (for `pprev`) | `CX` (the accumulator value before the last comparison, i.e. the second operand) | `09c8:0237 mov cx,[075A] / cmp cx,[bp]`, `pprev` `09c8:0951` |
| VM stack pointer | `BP` (SS = DGROUP), stack base `[1FDC]` = DGROUP:7580 (handle table 3580 + 0x4000), limit `[1FDE]` = base + 0x1000 | set at `0000:00A9`..`00B1`; `BP` init `09c8:1EAB`; overflow check in `link` `09c8:04EF` |
| Parameters base (argc word) | `DGROUP:0A3C` | `lap` `09c8:09EA`, call helper `09c8:1EF2` |
| Temporaries base | `DGROUP:0A40` | `link` `09c8:04D6`, `lat` `09c8:09C5` |
| Globals segment | `DGROUP:075C`; the block is script 0's locals (`[0A4E] = [1FCE]` after loading script 0) | `lag` `09c8:0975`, start-up `09c8:1E70` |
| Locals segment | `DGROUP:075E` | `lal` `09c8:099E` |
| Current object handle (`self`) | `DGROUP:0A52` | `selfID` `09c8:092C` |
| Current object segment | `DGROUP:0A2C` | `pToa` `09c8:157B` |
| `&rest` extra byte count | `DGROUP:0A44` | `&rest` `09c8:0073` |
| Send stack (object, selector pairs) | `DGROUP:095C`..`0A28`, pointer `[0A28]` | `09c8:1977` |
| Memory handle table | `DGROUP:3580`, 4-byte entries {segment, type byte} | `mov es,[si+3580]` throughout |
| Class table | far pointer at `DGROUP:056E`, 4-byte entries {object handle, script number} | `class` `09c8:0810` |
| Line / file registers | `DGROUP:1EF4` / `DGROUP:1EF6` | opcodes 0x4C / 0x4E, entry `09c8:000C`, `ret` `09c8:06B5`; no instruction in the image reads either one |
| Debugger-enabled flag | `DGROUP:1EF8` (0 in the image, never written) | tests at `09c8:00F8`, `06A4`, every store handler |

- CONFIRMED: a handle is a byte offset into the handle table. Segment value 0xF000 marks a
  non-resident block; every access then calls `16ac:0036` to bring it back (INFERRED: EMS/XMS
  swap-in; the EMS/XMS copy routines and their error strings are in the same binary).
- CONFIRMED: TSN 2.1 has no 0xF000 check (`07f7:000E mov es,[si+2690]` directly) and keeps its
  handle table at `DGROUP:2690`.

## 5. Bytecode opcodes

### 5.1 Dispatch

- CONFIRMED: threaded dispatch. Every handler ends with
  `lodsb es:[si] / mov bl,al / and bx,00FE / jmp [bx+0860]`
  (first instance `09c8:0045`, file 0xCDA5). The low opcode bit selects the operand width inside
  the handler (`test al,1`; set = byte operand), exactly as in SCI.
- CONFIRMED: `DGROUP:0860` (file 0x24080) is the live 128-word table of near handler offsets in
  VM code segment `09c8` (file base 0xCD60). `09c8:2123` copies the pristine table from
  `DGROUP:0760` (file 0x23F80) over it; `09c8:2100` fills the live table with the debugger trap
  `09c8:2144`, which calls the debugger and then jumps to the real handler from `[bx+0760]`.
- CONFIRMED: the debugger is compiled out of this build. `17b2:0296`, `0297`, `0298` and `03AC` are
  single `retf`, the `[1EF8]` flag is never set, and `17b2:0299` only calls `03AC`.
- CONFIRMED: bad opcodes go to `09c8:0051`, fatal error code 1, message `Bad opcode: $%x`
  (`2074:1F20`, file 0x25740).
- Dec-1993: live table `1F6B:0854`, static `1F6B:0754` (file 0x22D84), VM segment `09b9`.
  TSN 2.1: live table `1B14:0610`, static `1B14:0510` (file 0x1DB00), VM segment `07f7`,
  bad-opcode handler `07f7:0020`.

### 5.2 Opcode map (Feb-1994)

Operand notation: B/W = byte in the odd form, word in the even form; `s` = sign-extended; `B` or
`W` alone = fixed width in both forms. ScummVM format is `g_base_opcode_formats` in
`scummvm/engines/sci/engine/kernel_tables.h` as adjusted by `script_adjust_opcode_formats()` in
`engine/kernel.cpp` for SCI0/SCI1 (lofsa/lofss become `Script_Offset` there).

| Op | ScummVM name | LSCI | LSCI operands | ScummVM format | Handler | File | Divergence |
|---|---|---|---|---|---|---|---|
| 00/01 | bnot | bnot | - | - | 09c8:0211 | 0x0cf71 |  |
| 02/03 | add | add | - | - | 09c8:009a | 0x0cdfa |  |
| 04/05 | sub | sub | - | - | 09c8:00b0 | 0x0ce10 |  |
| 06/07 | mul | mul | - | - | 09c8:00c9 | 0x0ce29 |  |
| 08/09 | div | div | - | - | 09c8:0115 | 0x0ce75 |  |
| 0a/0b | mod | mod | - | - | 09c8:0136 | 0x0ce96 |  |
| 0c/0d | shr | shr | - | - | 09c8:0168 | 0x0cec8 |  |
| 0e/0f | shl | shl | - | - | 09c8:0183 | 0x0cee3 |  |
| 10/11 | xor | xor | - | - | 09c8:019e | 0x0cefe |  |
| 12/13 | and | and | - | - | 09c8:01b4 | 0x0cf14 |  |
| 14/15 | or | or | - | - | 09c8:01ca | 0x0cf2a |  |
| 16/17 | neg | neg | - | - | 09c8:01e0 | 0x0cf40 |  |
| 18/19 | not | not | - | - | 09c8:01f0 | 0x0cf50 |  |
| 1a/1b | eq | eq? | - | - | 09c8:0237 | 0x0cf97 |  |
| 1c/1d | ne | ne? | - | - | 09c8:0255 | 0x0cfb5 |  |
| 1e/1f | gt | gt? | - | - | 09c8:0273 | 0x0cfd3 |  |
| 20/21 | ge | ge? | - | - | 09c8:0291 | 0x0cff1 |  |
| 22/23 | lt | lt? | - | - | 09c8:02af | 0x0d00f |  |
| 24/25 | le | le? | - | - | 09c8:02cd | 0x0d02d |  |
| 26/27 | ugt | ugt? | - | - | 09c8:0300 | 0x0d060 |  |
| 28/29 | uge | uge? | - | - | 09c8:031e | 0x0d07e |  |
| 2a/2b | ult | ult? | - | - | 09c8:033c | 0x0d09c |  |
| 2c/2d | ule | ule? | - | - | 09c8:035a | 0x0d0ba |  |
| 2e/2f | bt | bt | srel B/W | SRelative | 09c8:0378 | 0x0d0d8 |  |
| 30/31 | bnt | bnt | srel B/W | SRelative | 09c8:0394 | 0x0d0f4 |  |
| 32/33 | jmp | jmp | srel B/W | SRelative | 09c8:03b0 | 0x0d110 |  |
| 34/35 | ldi | ldi | simm B/W | SVariable | 09c8:03f5 | 0x0d155 |  |
| 36/37 | push | push | - | - | 09c8:049d | 0x0d1fd |  |
| 38/39 | pushi | pushi | simm B/W | SVariable | 09c8:041c | 0x0d17c |  |
| 3a/3b | toss | toss | - | - | 09c8:04b2 | 0x0d212 |  |
| 3c/3d | dup | dup | - | - | 09c8:04c1 | 0x0d221 |  |
| 3e/3f | link | link | n B/W | Variable | 09c8:04d6 | 0x0d236 |  |
| 40/41 | call | call | handle W (both forms), frame B | SRelative, Byte | 09c8:0511 | 0x0d271 | **yes**: operand is always a word and is a code handle, not a PC-relative offset |
| 42/43 | callk | callk | kernel B/W, frame B | Variable, Byte | 09c8:052c | 0x0d28c |  |
| 44/45 | callb | callb | export B/W, frame B | Variable, Byte | 09c8:0658 | 0x0d3b8 |  |
| 46/47 | calle | calle | script B/W, export B (both forms), frame B | Variable, SVariable, Byte | 09c8:067b | 0x0d3db | **yes**: 0x46 reads a byte export index, ScummVM a word |
| 48/49 | ret | ret | - | End | 09c8:06a4 | 0x0d404 | semantics: also restores line/file registers |
| 4a/4b | send | send | frame B | Byte | 09c8:06be | 0x0d41e |  |
| 4c/4d | info | _line_ (INFERRED name) | W (both forms) | Invalid (SCI3: info, none) | 09c8:00f3 | 0x0ce53 | **yes**: new opcode |
| 4e/4f | superP | _file_ (INFERRED name) | W (both forms) | Invalid (SCI3: superP, none) | 09c8:00e2 | 0x0ce42 | **yes**: new opcode |
| 50/51 | class | class | class B/W | Variable | 09c8:0800 | 0x0d560 |  |
| 52/53 | dummy29 | (bad opcode) |  | Invalid | 09c8:0051 | 0x0cdb1 |  |
| 54/55 | self | self | frame B | Byte | 09c8:093e | 0x0d69e |  |
| 56/57 | super | super | class B/W, frame B | Variable, Byte | 09c8:06d1 | 0x0d431 |  |
| 58/59 | rest | &rest | param B (both forms) | SVariable | 09c8:0062 | 0x0cdc2 | **yes**: 0x58 reads one byte |
| 5a/5b | lea | = ldi | simm B/W | SVariable, Variable (lea) | 09c8:03f5 | 0x0d155 | **yes**: lea is gone; handler is ldi |
| 5c/5d | selfID | selfID | - | - | 09c8:092c | 0x0d68c |  |
| 5e/5f | dummy2f | (bad opcode) |  | Invalid | 09c8:0051 | 0x0cdb1 |  |
| 60/61 | pprev | pprev | - | - | 09c8:0951 | 0x0d6b1 |  |
| 62/63 | pToa | pToa | prop B/W | Property | 09c8:156b | 0x0e2cb |  |
| 64/65 | aTop | aTop | prop B/W | Property | 09c8:15be | 0x0e31e |  |
| 66/67 | pTos | pTos | prop B/W | Property | 09c8:1593 | 0x0e2f3 |  |
| 68/69 | sTop | sTop | prop B/W | Property | 09c8:15e6 | 0x0e346 |  |
| 6a/6b | ipToa | ipToa | prop B/W | Property | 09c8:1611 | 0x0e371 |  |
| 6c/6d | dpToa | dpToa | prop B/W | Property | 09c8:166a | 0x0e3ca |  |
| 6e/6f | ipTos | ipTos | prop B/W | Property | 09c8:163c | 0x0e39c |  |
| 70/71 | dpTos | dpTos | prop B/W | Property | 09c8:1695 | 0x0e3f5 |  |
| 72/73 | lofsa | (bad opcode) |  | SRelative / Offset (lofsa) | 09c8:0051 | 0x0cdb1 | **yes**: lofsa removed |
| 74/75 | lofss | = pushi | simm B/W | SRelative / Offset (lofss) | 09c8:041c | 0x0d17c | **yes**: lofss handler is pushi |
| 76/77 | push0 | push0 | - | - | 09c8:0449 | 0x0d1a9 |  |
| 78/79 | push1 | push1 | - | - | 09c8:045e | 0x0d1be |  |
| 7a/7b | push2 | push2 | - | - | 09c8:0473 | 0x0d1d3 |  |
| 7c/7d | pushSelf | pushSelf | - | - | 09c8:0488 | 0x0d1e8 | 0x7d is also pushSelf |
| 7e/7f | line | (bad opcode) |  | Word (line) | 09c8:0051 | 0x0cdb1 | **yes**: 0x7e/0x7f invalid |

Load/store block 0x80-0xFF, handler offsets in segment 09c8:

| Op | Mnemonic | Handler | Op | Mnemonic | Handler | Op | Mnemonic | Handler | Op | Mnemonic | Handler |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 80 | lag | 0963 | 82 | lal | 098c | 84 | lat | 09b5 | 86 | lap | 09da |
| 88 | lsg | 09ff | 8a | lsl | 0a2b | 8c | lst | 0a57 | 8e | lsp | 0a7f |
| 90 | lagi | 0aa7 | 92 | lali | 0ad4 | 94 | lati | 0b01 | 96 | lapi | 0b2a |
| 98 | lsgi | 0b53 | 9a | lsli | 0b83 | 9c | lsti | 0bb3 | 9e | lspi | 0bdf |
| a0 | sag | 0c0b | a2 | sal | 0c45 | a4 | sat | 0c7f | a6 | sap | 0cb5 |
| a8 | ssg | 0ceb | aa | ssl | 0d28 | ac | sst | 0d65 | ae | ssp | 0d9e |
| b0 | sagi | 0dd7 | b2 | sali | 0e1b | b4 | sati | 0e5f | b6 | sapi | 0e9f |
| b8 | ssgi | 0edf | ba | ssli | 0f20 | bc | ssti | 0f61 | be | sspi | 0f9e |
| c0 | plusag | 0fdb | c2 | plusal | 1006 | c4 | plusat | 1031 | c6 | plusap | 1058 |
| c8 | plussg | 107f | ca | plussl | 10ad | cc | plusst | 10db | ce | plussp | 1105 |
| d0 | plusagi | 112f | d2 | plusali | 115e | d4 | plusati | 118d | d6 | plusapi | 11b8 |
| d8 | plussgi | 11e3 | da | plussli | 1215 | dc | plussti | 1247 | de | plusspi | 1275 |
| e0 | minusag | 12a3 | e2 | minusal | 12ce | e4 | minusat | 12f9 | e6 | minusap | 1320 |
| e8 | minussg | 1347 | ea | minussl | 1375 | ec | minusst | 13a3 | ee | minussp | 13cd |
| f0 | minusagi | 13f7 | f2 | minusali | 1426 | f4 | minusati | 1455 | f6 | minusapi | 1480 |
| f8 | minussgi | 14ab | fa | minussli | 14dd | fc | minussti | 150f | fe | minusspi | 153d |

- CONFIRMED: the load/store block 0x80-0xFF has exactly SCI's bit layout: bits 1-2 select
  global/local/temp/param, bit 3 stack instead of accumulator, bit 4 accumulator-indexed,
  bits 5-6 load/store/increment/decrement. Operand is an unsigned byte/word variable index.
  Globals use the segment in `[075C]`, locals `[075E]`, temps `[0A40]`, params `[0A3C]`
  (param 0 = argc). Store forms call the debugger watch hook `17b2:0296` when `[1EF8]` is set.
- CONFIRMED: property opcodes (0x62-0x71) take a byte offset into the current object's segment,
  as SCI does; they read through `ES = [0A2C]`.

### 5.3 Divergences from ScummVM SCI (complete list)

| # | Opcode | LSCI behaviour | ScummVM behaviour | Status |
|---|---|---|---|---|
| 1 | 0x40/0x41 `call` | operand is **always a 16-bit word** (no byte form) naming a code handle; then frame size byte. `09c8:0511 lodsw / lodsb / mov si,handle / call 1EE1` | signed PC-relative offset, byte in 0x41 | CONFIRMED |
| 2 | 0x46 `calle` | script number B/W, **export index always one byte**, frame size byte (`09c8:068B`) | 0x46 reads a word export index | CONFIRMED |
| 3 | 0x58 `&rest` | always one unsigned byte, both forms (`09c8:0068`) | signed byte/word by opcode bit | CONFIRMED |
| 4 | 0x5A/0x5B `lea` | handler is `ldi` (`09c8:03F5`): load a signed immediate into acc, one operand | two operands (type, index), loads a variable address | CONFIRMED |
| 5 | 0x72/0x73 `lofsa` | bad opcode | load code/data offset into acc | CONFIRMED |
| 6 | 0x74/0x75 `lofss` | handler is `pushi` (`09c8:041C`): push a signed immediate | push code/data offset | CONFIRMED |
| 7 | 0x4C/0x4D | new: read a word (both forms) into `[1EF4]`; if `[1EF8]` is set call debugger step hook `17b2:0297`, a `retf` stub (`09c8:00F3`). Name `_line_` | invalid (SCI3 `info`, no operand) | CONFIRMED behaviour, INFERRED name |
| 8 | 0x4E/0x4F | new: read a word (both forms) into `[1EF6]` (`09c8:00E2`). Name `_file_` | invalid (SCI3 `superP`) | CONFIRMED behaviour, INFERRED name |
| 9 | 0x7E/0x7F | bad opcode | SCI2+ `line` (word) | CONFIRMED |
| 10 | 0x7D | `pushSelf` (shared handler) | `pushSelf` (low bit normally clear) | CONFIRMED |

Execution-model differences that are not visible in the operand table:

- CONFIRMED: **per-procedure code handles.** Procedure entry `09c8:000C` stores the target handle in
  `[0A4C]`, loads `ES` from the handle table and sets `SI = 0`; execution starts at offset 0 of that
  block. `call` operands, export entries (`09c8:2068 mov ax,es:[bx+di+2]`) and method dictionary
  entries (`09c8:1AEE mov si,es:[si]`) all hold such handles. Branch offsets stay relative within
  the block. INFERRED: the script loader splits code into blocks and writes the run-time handle
  numbers into the `call` operands and dispatch tables; this must be confirmed against the script
  container format (Track A2).
- CONFIRMED: **objects are memory blocks, not script offsets.** An object is a handle of type 2
  (`[handle+3582] == 2`, else `Not an object: $%x`). In the object segment: word 0 = handle of its
  script record (record word 8 = locals handle), word 2 = property count, word 4 = handle of the
  property-selector list, word 6 = superclass handle. The method dictionary follows the
  properties: `{count, selector[count], code handle[count]}` (`09c8:1ACD`..`1AEE`). INFERRED:
  the four header words double as properties 0-3, since property opcodes address from offset 0.
- CONFIRMED: `send` (`09c8:1961`) looks the selector up in the property-selector list first (argc
  0 = read into acc, otherwise write param 1), then walks method dictionaries up the superclass
  chain; failure is fatal `'%s' is not a selector for %s.`
- CONFIRMED: procedure entry pushes `[1EF6]`, `[1EF4]` and clears `[1EF4]`; `ret` (`09c8:06A4`)
  pops them back, then does a near `ret` into the caller's entry stub. TSN 2.1's `ret`
  (`07f7:059B`) is a bare `ret`, and its procedure entry has no line/file save.
- CONFIRMED: script record (from `0358:0024`, script number to handle): word 6 = export table
  handle `{count, handle[count]}`, word 8 = locals handle (`09c8:202D`..`2068`). `callb` is
  `calle` with script 0. Export index out of range is fatal `Dispatch number too large: %d`.
- CONFIRMED: start-up (`09c8:1E28`) takes export 0 of script 0 as the game object and sends it
  selector 0x1A on a fresh start or 0x1B on restart, with no arguments (`09c8:1EC6`).
- CONFIRMED fatal error codes passed to `17b2:02CF` (jump table `17b2:02EB`, 9 slots): 0 `Dispatch
  number too large: %d`, 1 `Bad opcode: $%x`, 2 `Kernel entry # too large: %d`, 3 `Can't load class
  %d`, 4 `Not an object: $%x`, 5 `'%s' is not a selector for %s.`, 6 no message, 7 `Attempt to
  divide by zero.`, 8 `Stack overflow.` (`link` and send-stack overflow).
- CONFIRMED: the export range check (`09c8:2061 cmp es:[bx],di / jl`) rejects only index > count, so
  index == count is not caught and reads one word past the table.

## 6. Cross-build opcode comparison

- CONFIRMED: Dec-1993 has the same opcode table shape as Feb-1994 (same handler-sharing pattern,
  0x4C/0x4E present).
- CONFIRMED: TSN 2.1 differs only in 0x4C/0x4E, which go to its bad-opcode handler. `call`,
  `calle`, `&rest`, `lea`, `lofsa`, `lofss` behave as in Feb-1994 (checked at `07f7:04AD`,
  `0572`, `0031`, `0391`, `03B8`).

## 7. Open questions

- How the script loader turns resource bytes into code handles and patches `call` operands (Track A2).
- What `[1EF6]` holds (INFERRED script/file number); the debugger that would print it is compiled out, so only the compiler's use of 0x4C/0x4E can tell.
- Kernel signatures beyond the prologue scan, and sub-op semantics (Track A3/A4).
- What 0x1A / 0x1B are in the selector vocabulary (INFERRED `play` / `replay`).

## 8. Verification

Independent re-check against the binaries (capstone disassembly and table parsing, no reuse of the
tool's output):

- CONFIRMED: kernel handler and name arrays at the stated addresses in all three builds, 89 entries
  plus a null terminator, limit word 0x58; names equal the `*.kernel.txt` order; all 89 handler
  addresses and file offsets in sections 3.3 and 3.4 reproduce.
- CONFIRMED: `callk`, `call`, `calle`, `callb`, `&rest`, `ret`, procedure entry, `send`, `class`,
  `pprev`, branch and load/store handlers behave as described; the 64 rows of the opcode map and the
  load/store block match the static table at `DGROUP:0760`; live and static tables are identical.
- CONFIRMED: sub-op limits (Array 8, List 14, Seq 7, String 12, Memory 11, Sound 15, FileSystem 18,
  SID 6, TSN 17 with slots 5, 6, 10, 15 on the default path); TSN 2.1 String 5 and FileSystem 16.
- CONFIRMED: Dec-1993 opcode shape equals Feb-1994; TSN 2.1 differs only in 0x4C/0x4E.
- CONFIRMED: object and script-record layout, method dictionary layout, send stack pairs, start-up
  selectors 0x1A / 0x1B on export 0 of script 0, copyright string, `Interpreter:  ` prefix routine,
  media dates (1994-02-14, 1993-11-05, 1993-02-26).
- Fixed in this pass: TSN 2.1 `Save`/`Restore` are implemented, not stubs; `pprev` register holds the
  old accumulator; the debugger hooks and `[1EF8]` flag are dead in this build; fatal code 4 and the
  message table were missing; `[1EF4]`/`[1EF6]` are never read; globals are script 0's locals; the
  post-kernel reload list was imprecise; export range check off-by-one noted.
- Still INFERRED: that the script loader writes run-time handle numbers into `call` operands, the
  `_line_` / `_file_` names, and the EMS/XMS meaning of segment 0xF000.

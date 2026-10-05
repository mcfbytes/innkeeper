# TSN / INN in ScummVM: plan of attack

End goal: the ImagiNation Network client playable in ScummVM, connected to a new
server (Rust, cloud-hosted, possibly distributed).

Status (2026-10-04): reconnaissance done. Nothing has been built or changed in ScummVM yet.

---

## 1. What recon found

### 1.1 Media

| Set | Source in `media/` | Data date | Notes |
|---|---|---|---|
| **INN 2.3, Feb 1994 + Twinion** | `INN_from_1995_Sierra_CD_sampler.zip` (taken from the ISO) | 1994-02-15 | **Primary target.** It matches the floppy "V2.3" set and also adds `TWINION/`. Everything comes in one `PART.1`. |
| INN 2.3, Feb 1994 | `003224_…7z` (was also `…V2.3[PC].zip`) | 1994-02-15 | Same SCI data as the CD. |
| INN, Dec 1993 | `003225_…7z` (was also `…V2.3.17[PC].zip`) | 1993-12-02 | **Older, even though it was labelled "2.3.17".** It has an older LSCITV and different resources. |
| TSN 2.1 | `Sierra Network, The (v2.1, Int. 031293) (1.44M).zip` | 1993-03-08 | The other dumps (003227, INT#031295, flux) had the same contents and were deleted. |
| TSN "basic" | `000513_sierra_network_basic.7z` | older | A single 1.2 MB disk that is different from the others. It has `_SNEXEC.EXE`, `_SCITV.EXE` (puff-packed) and `CCPATCH/141.SO`. |

The rest of the ISO (623 MB) was a 1995 Sierra demo sampler (SQ6, KQ7, Lode Runner and so on). The ISO, the flux dumps and the duplicate `[PC].zip` sets were deleted on 2026-10-04, which leaves `media/` at 26 MB. Media and extracted data are git-ignored and must never be committed.

### 1.2 Install format (tools in `tools/`)
- `PART.n` is Sierra's "defuse" archive. Each part opens with a 31-byte directory record. Every entry is `name[13] u16 flags(0x4000=dir) u32 total u32 chunk_len u32 chunk_off u32 dos_datetime`, and files can split across disks. `tools/defuse_list.py` extracts it.
- Files named `_XXXX` are additionally compressed with Sierra `puff`: `name[13] u32 crc u16 dostime u16 dosdate` followed by a PKWARE DCL stream. `tools/unpuff.py` decodes it, and all 251 such files decode cleanly into `work/games/`.
- `LSCITV.EXE` (the interpreter) and `TSNEXEC.EXE` are LZEXE 0.91. `tools/unlzexe.py` unpacks them to `work/exe/`.

### 1.3 The client architecture
```
INN.BAT -> TSNEXEC.EXE (resident "INN Executive"; opens the com driver MODEM.DRV/NOBRK.DRV;
                        runs TSN.PRG, a little script language that chains programs)
             ├─ lscitv.exe -cLSCI.CFG        Clubhouse   (root RESOURCE.*)
             ├─ cd SL; ..\lscitv.exe         SierraLand  (SL/RESOURCE.*)
             ├─ cd LL; ..\lscitv.exe         CasinoLand  (LL/RESOURCE.*)
             ├─ BARON\rb.exe, GOLF\golf.exe, YSERBIUS\cgenn.exe, TWINION\…, SHOPADV\…  (plain DOS games, not SCI)
```
- The modem driver dials with Hayes AT commands into a SprintNet X.25 PAD. `HOSTADDR` lists the hosts `Sierra7..14` at X.25 addresses `3110834202xx`. The connection lives in TSNEXEC and survives from one program launch to the next.
- The interpreter loads DLLs: `graph256.dll` and `nlnull.dll`. There are also `InstallServer`/`DisposeServer` entry points.

### 1.3a The shared seam: INT 14h
`TSNEXEC` hooks **INT 14h**, the BIOS serial-services interrupt, and every client has exactly one INT 14h call site. That covers LSCITV, Golf, Yserbius (`CGENN`/`DARKSTRT`), Twinion (`TWGENN`/`FATES`), Red Baron and Shoppers Advantage. The DOS games also link a common TSN client library (`src\tsn\hostcomm.c`, `gamecomm.c`, `dialog.c`). Their error strings include "TSN Executive not loaded!", "TSN_NetSend() failed on command %d", "TSN NAK Error" and "get host time timeout".

So the stack is: **program → INT 14h API → TSNEXEC → com driver → modem/PAD → host.** Reverse-engineering the TSNEXEC INT 14h API and the host protocol below it once covers every title.

### 1.3b The non-SCI titles
| Title | What it is | Tech | ScummVM fit |
|---|---|---|---|
| Shadow of Yserbius (`CGENN`, `DARKSTRT`) and Fates of Twinion (`TWGENN`, `FATES`) | First-person, grid-based multiplayer dungeon RPGs designed by Ybarra Productions. Both use one shared engine. | Borland C++ 1991, Miles AIL/XMIDI audio, custom `RESOURCE.0xx`/`MAIN.TSU`/`WALLS.DAT`/`IMAGES.DAT` files. The server side is rich: a BBS, map groups, parties and player objects. | **Plausible as one new ScummVM engine covering both games**, similar to the existing Might & Magic engine (`engines/mm`). A large but well-bounded job. |
| Red Baron (`RB.EXE`) | Dynamix's Red Baron (1990), a 3D flight sim built on the 3Space engine. "INN Conversion by Steve Luzietti and Dave Eaton." | Borland C++, LZEXE-packed, Dynamix `VOLUME.RMF`/`VOLUME.00x` archive. ScummVM's `dgds` engine already reads that archive format, but only that. | **Poor.** It would mean reimplementing a full 3D flight sim. Stay in DOSBox. |
| 3-D Golf (`GOLF.EXE`) | A 3D golf game. | Borland C++, TSN library. | Poor. Stay in DOSBox. |
| Shoppers Advantage (`SHOPADV.EXE`) | A text front end to a real third-party shopping service. | Borland C++. | Not worth reviving, beyond a stub that says "closed". |

The DOS titles will run under DOSBox against our server through the same seam, so none of them is blocked on ScummVM work.
, not "SCI plus a few kernels"
- **Kernel table.** The interpreter has its own built-in table of 89 functions; the list is in `work/exe/LSCITV_*.kernel.txt`. It is identical in TSN 2.1, INN Dec-93 and INN Feb-94, and **all networking goes through the one `TSN` kernel (0x54)**, which presumably takes sub-ops. The numbering is unrelated to ScummVM's SCI0/SCI1 tables. Several calls look like early versions of SCI32 calls (`Array`, `List`, `String`, `ObjectNew/Free`, `Module*`, `Seq`, `Block`).
  The numbers below assume the string order matches the dispatch order. **That still needs checking against the real dispatch table.**
  ```
  00 IsObject 01 ObjectFree 02 ObjectNew 03 ObjectRespondsTo 04 Array 05 Block 06 List
  07 ModuleDispose 08 ModuleID 09 Resource 0a Seq 0b String 0c GetFarText 0d CoordPri
  0e DrawPic 0f PicState 10 ShakeScreen 11 Show 12 AddToPic 13 Animate 14 CelHigh 15 CelRect
  16 CelWide 17 DrawCel 18 DrawScaledCel 19 NumCels 1a NumLoops 1b ScaledCelRect 1c DrawControl
  1d EditControl 1e HiliteControl 1f GetPort 20 SetPort 21 WindowDispose 22 WindowNew 23 Display
  24 TextSize 25 GetEvent 26 GlobalToLocal 27 LocalToGlobal 28 MapKeyToDir 29 Menu 2a HaveMouse
  2b SetCursor 2c Memory 2d CheckSaveGame 2e GameIsRestarting 2f GetSaveDir 30 GetSaveFiles
  31 Restart 32 Restore 33 Save 34 Sound 35 GetTime 36 SetTimerFreq 37 Wait 38 Alert 39 CanBeHere
  3a OnControl 3b FileSystem 3c DeviceInfo 3d Abs 3e CosDiv 3f CosMult 40 GetAngle 41 GetDistance
  42 Random 43 SinDiv 44 SinMult 45 Sqrt 46 Long 47 Graph 48 Debug 49 Profiler 4a SetDebug
  4b ShowFree 4c StackUsage 4d Joystick 4e BaseRectSet 4f ConfigStr 50 Encrypt 51 Decrypt
  52 ColorUp 53 ColorDown 54 TSN 55 ObjPropOffset 56 ObjOffsetProp 57 SID 58 InvokeMethod
  ```
- **No `vocab.999`.** The names exist only inside the interpreter.
- **Compression.** Almost every resource uses method **8**, which is PKWARE DCL. ScummVM only maps DCL to 18–20, so method 8 currently fails with "unknown compression". `tools/dcl.py` decodes all 338 resources.
- **Resource type 31.** There are about 62 of these, numbered 6xx. They hold bytecode and names such as `Act`, and look like separately loadable class or code modules (compare the `ModuleID`/`ModuleDispose` kernels).
- **Script container.** It is not the SCI0 block format. The bytecode itself looks like standard SCI opcodes (`pushi`, `push0`, `send`, `pToa`, `bnt`, `ret`…), but the header and object layout are new. Decompressed resources are in `work/res/inn_cd/`.
- **ScummVM's existing `inndemo`** is described in its detection table as a normal SCI 1.001.097 build. That means "the demo loads" says very little about whether the real LSCI client will run.

### 1.5 Prior art
- **INN Barn** (innbarn.com) is a working, closed-source revival server. It uses the original client under bundled DOSBox, with no ScummVM involved, and has said it has no plans to release source. It's still useful in two ways: as proof the original client works against a reimplemented host, and as a possible protocol oracle or collaborator. Ask before capturing traffic against their server.
- James Leiterman, an INN programmer from 1994–95, wrote INN's SDK: a "Pseudo Host" plus a virtual message router. That suggests the game protocol was mostly message routing between clients rather than server-side game logic.

---

## 2a. Host project decision: ScummVM `engines/sci`, LSCI as a version variant

The decision is to build inside ScummVM's SCI engine instead of a standalone reimplementation. The evidence:
- **The asset formats are SCI1 VGA, which ScummVM already decodes:**
  - views start `03 80 …` (loop count, flags, palette offset)
  - pics start with `FE 02` (the extended palette opcode)
  - `palette.999` is 1284 bytes, the SCI1 palette layout
  - fonts are the standard SCI layout
  - sounds look like SCI0/01
  - the sound drivers (ADL, MT32, SNDBLAST, TANDY) are the stock Sierra set
- **The bytecode looks like standard SCI opcodes.** If so, ScummVM's VM, object model, garbage collector, debugger (`bp`, `vo`, `disasm`, `bt`) and savegame layer all carry over.
- **The kernels mostly reuse existing ScummVM code.** Most are SCI0/1 kernels under new numbers, and the new ones (`Array`, `String`, `List`, `Object*`) resemble SCI32 calls that ScummVM already implements.
- **ScummVM already has online-multiplayer precedent.** `engines/scumm/he/net/` includes `net_lobby.cpp`, used for Backyard sports online against an external lobby server. `backends/networking/` also has `enet`, `sdl_net` and `http`.
- **A standalone build would redo all of the above, plus every platform backend, MIDI emulation and scaling.**

How the work is split up:
- **The ScummVM fork** holds only the client: detection, resources, LSCI script loader, kernel table, and a thin `kTSN` + transport layer.
- **This repo** holds the reverse-engineering tools, the documentation and `docs/protocol.md`.
- **A separate Rust workspace** holds the server and the `inn-proto` crate.

Because the protocol spec is independent of ScummVM, a DOSBox client or any future client can use it.

Fallback trigger: if Phase 1.1–1.2 shows LSCI's VM or object model diverging deeply from SCI, so that patching `engines/sci` becomes invasive, move LSCI to its own ScummVM engine (`engines/lsci`). It would still borrow the SCI graphics and sound classes. Leaving ScummVM entirely is not a goal.

---

## 2. Key strategic decision: where to put the network boundary

The boundary is **INT 14h** (section 1.3a). Every program reaches the network through TSNEXEC's INT 14h API, so the server accepts traffic two ways and feeds both into one internal message layer:

| Ingress | Who uses it | What goes over the wire |
|---|---|---|
| **Legacy link** | The original client running unmodified in DOSBox, using stock DOSBox modem emulation (`ATDT` to host:port, or a phonebook mapping). | The bytes that TSNEXEC and the com driver actually emit: Hayes, then the X.25 PAD dialogue, then TSN framing. The server pretends to be SprintNet plus the Sierra host. |
| **`int14h` transport** | ScummVM's built-in "virtual TSNEXEC". Optionally also a DOSBox-side **INT 14h helper**: either a replacement TSNEXEC TSR, or a DOSBox-X patch that services INT 14h on the host side. | The INT 14h API calls themselves (function, registers, payload) serialised over TCP or WebSocket. This avoids modem timing, the PAD and line noise. |

We start with the legacy link, because it needs no installs and gives us ground truth. The helper is an upgrade or fallback if the link layer turns out to be painful, and the ScummVM side is the same idea written in C++.

### Naming (proposed)
- **Server: `innkeeper`** (the daemon is `innkeeperd`). It keeps the INN and pours at Lefty's Bar.
- **Seam and transport crate: `int14h`.** README tagline: *"Look behind you, a three-headed modem!"* The Monkey Island humour is a nod to ScummVM's SCUMM roots.
- **X.25 PAD emulator module: `pad_thai`.** **Hayes AT handling for raw serial and null-modem links: `hayes_fever`.**
- **Startup log line:** `INT 14h hooked. Please wait while ImagiNation loads...`
- Alternatives if this doesn't land: `grog14` (Lefty's Bar meets the SCUMM Bar), or `fourteen-hex` ("the Voodoo Lady handles all hexes").

---

## 3. Phases

### Phase 0: Workspace and tooling
- [x] Extraction tools (`defuse_list.py`, `unlzexe.py`, `dcl.py`, `sci_res.py`, `unpuff.py`), plus all sets extracted under `work/`.
- [x] ScummVM shallow clone in `scummvm/`, and `git init` with media and game data ignored.
- [ ] Packages (needs the user, for sudo): `libsdl2-dev libsdl2-net-dev dosbox-x openjdk-21-jdk`, then Ghidra from its GitHub releases. Rust is already in `~/.cargo/bin`.
- [ ] Build ScummVM with `--disable-all-engines --enable-engine=sci`.
- [x] Install the Feb-94 client into a DOSBox-X directory, and confirm it reaches the dial screen offline (`tools/dosbox/`, `docs/dosbox.md`).

### Phase 1: Reverse-engineering, in two parallel tracks
**Track A: the LSCI client**
1. Find the kernel and opcode dispatch tables in LSCITV. This confirms the kernel numbering and the calling conventions, and shows how far the opcodes diverge from SCI.
2. Decode the script, module (type 31) and vocab formats, and build `tools/lsci_disasm.py`.
3. Produce the kernel usage report for every `callk` in every set: argc, argument provenance, how results are used, and sub-op histograms for `TSN`, `Array`, `String`, `List`, `Seq` and `Module`. Diff it across versions.
4. Work out the semantics of each kernel handler.

**Track B: the network seam**
1. TSNEXEC's INT 14h handler: the list of AH functions, the register and buffer conventions, and how child programs find it.
2. The com-driver API (MODEM.DRV and NOBRK.DRV), the TSN framing (checksums, acks, NAK and retry), and the login and handshake.
3. Dynamic capture. `innkeeper` v0 is a TCP listener that logs every byte and scripts just enough PAD responses (`pad_thai`) to coax out the first TSN frames from DOSBox. This is the first Rust code. **Done for the first frame**: the stock client dials, logs on to the PAD and sends its Login (`docs/protocol/captures.md`); the host's replies are the next capture.
4. Deliverable: `docs/protocol.md`, covering both the INT 14h API and the wire format.

### Phase 2: ScummVM LSCI bring-up, offline
Detection; resource fixes (method 8 → DCL, type 31, shared `RESOURCE.002`); the LSCI script loader and kernel table; land switching in place of TSN.PRG chaining; and `kTSN` routed to an offline virtual TSNEXEC.
**Milestone:** the Clubhouse renders and you can walk around. From there, use the ScummVM debugger for dynamic analysis.

### Phase 3: `innkeeper` and live play
- Crates: `int14h` (API and transport codecs), `inn-proto` (TSN messages), `pad_thai`, and `innkeeperd`.
- Server services: accounts and login, presence, lands and rooms, chat and mail, game-session routing, and persistence (SQLite for development, Postgres in the cloud).
- **Integration ladder:**
  1. A DOSBox client logs in.
  2. Two DOSBox clients chat.
  3. Two DOSBox clients play a card or board game.
  4. A ScummVM client does the same over `int14h`.
  5. ScummVM and DOSBox clients play together.
- Going distributed later means stateless gateways plus room or game shards over a bus.

### Phase 4: SCI coverage
All Clubhouse, SierraLand and CasinoLand games, the Dec-93 and TSN 2.1 variants, and any features later builds removed that are worth keeping.

### Phase 5: The Shadow of Yserbius
- **Server:** the Yserbius message set (BBS, map groups, parties, player objects, version control), first tested with the DOS client in DOSBox.
- **ScummVM:** a new engine for the Ybarra Productions engine (working name `engines/ybarra`), reimplemented from `DARKSTRT.EXE` and `CGENN.EXE`. It reuses ScummVM's XMIDI/Miles audio support and connects through `int14h`.

### Phase 6: The Fates of Twinion
The same engine extended to `FATES.EXE` and `TWGENN.EXE`. Diff the binaries and data formats against Yserbius first, and add the server-side differences.

### Deferred
- **Red Baron and 3-D Golf:** DOSBox only, through the same server, whenever we get to them. Their server needs are game-channel relay plus host time.
- **Shoppers Advantage, the joke edition:** a server-side stand-in for the old text shopping service that answers queries with Amazon search links. Don't scrape Amazon; plain public search URLs avoid any API terms. Amazon's product API needs an Associates account and has its own terms, so check those if we ever want real product data. Strictly optional.

---

## 4. Risks and open questions
- **How far LSCI diverges.** If the VM differs significantly, an `engines/sci` variant may be the wrong home. Track A.1–A.2 decides this.
- **Link-layer pain.** The PAD dialogue, modem timing and retries may be tough to emulate. That is why the INT 14h helper exists as a fallback.
- **Kernel numbering is assumed** from the string order until the dispatch table is located.
- **Compatibility pressure.** If INN Barn ever publishes its protocol, aligning with it makes sense.
- **Upstream acceptance.** ScummVM may be lukewarm about online-only titles, so keep a fork until then.

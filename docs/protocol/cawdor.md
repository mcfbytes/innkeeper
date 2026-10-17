# The Ruins of Cawdor: host message reference

What `innkeeperd` answers for the Cawdor programs (gallery `TWGENN.EXE`, game `RUINS.EXE`) beyond
[yserbius.md](yserbius.md) and [int14h-census/twinion.md](int14h-census/twinion.md). Cawdor is a
Twinion fork: the same joinNet kinds, object type `0x8B`, map and party groups, service lookup,
property read, game types 200–219 and bulletin-board subs. This page is only what differs. Notation
as in [messages.md](messages.md). Addresses are the load image (`seg:off`). Labels follow the
program survey: **CONFIRMED** was seen in that image, **INFERRED** was not.

## 1. What differs from Twinion

| Item | Twinion | Cawdor | Evidence | Status |
|---|---|---|---|---|
| Land type | 8 (`FATES` `DS:1966`) | **6** (`RUINS` `DS:195C`, gallery `DS:1A14`) | land join `11E2:10CC` pushes the land number at `DS:195A` (1) and the type byte, size `0x64`; gallery kind-5 caller `1933:121E` | CONFIRMED |
| Land joinNet | `b 5, b 8, w landNo, w 100` | `b 7, b 3, w 0, w player, b 5, b 6, w landNo, w 100` | same caller | CONFIRMED |
| Land GrpJoin 10/3 | 1.0.22 | **1.1.5** (`b 1, b 1, b 5` at bytes 6–8) | `11E2:10C7` pushes 5, then 1, then 1, then the land group and the player | CONFIRMED |
| Directory name | `Twinion` | `Cawdor` | one word, the same shape as the other two rows; the game's title is The Ruins of Cawdor | INFERRED (the hub shows the directory string) |
| Capacity | 100 | 100 | size word `0x64` | CONFIRMED |
| Player, map, party type | `0x8B` | `0x8B` | 41/2 at `11E2:0304` still pushes `0x8B`, not 6 | CONFIRMED |
| 17/2 | props 8, 9, 10, 11 | the same four, from `DS:196C` | sent right after the 41 reply; the client does not wait | CONFIRMED |
| Map group | size 80 | size 80 | the client sends the size; the host shares the group inside the player's place | CONFIRMED (size); scoping INFERRED, same as Twinion |
| Personal party | own group, param `−3`, size 4 | the same | kind 4, type `0x8B` | CONFIRMED |
| Game Send | `w map @6, b @8, b @9, msgType @0xA` | the same header at `11E2:1D5F` | types 200–219 (`msgType − 200` in `0..19`) | CONFIRMED (range); bodies INFERRED identical until a byte diff is shown |
| Shared block `+8` | skipped; type stays 8 | skipped; type stays 6 | two adds and no store, as Twinion `150F:01F3` | CONFIRMED |
| Next program | `TWINA` / `Twinion` | gallery menu word `DS:01E0` = 100 selects `CAWDA`, else `DEFAULT`; `RUINS` menu `0x4B5` → `DEFAULT`, `0x4B6` → NULL, `0x4B7` → `CAWDOR` (`093F:04F4`, `DS:00E2`, `DS:01CC`) | `TSN.PRG` blocks `Cawdor` → `twgenn.exe` and `Cawda` → `ruins.exe`. Lookup stays case-insensitive | CONFIRMED (names); case fold INFERRED |
| Characters | `TWINCHAR.DAT` on the client | `CAWCHAR.DAT` on the client | no upload command | CONFIRMED (no sender); host storage INFERRED none |

`RUINS.EXE` looks the export table up at `1741:000E` (file `0x1AE1E`), pointer `DS:1F9A`, DGROUP
`2178`. The gallery stub is `17D1:0004` (file `0x1B114`), pointer `DS:114C`, DGROUP `253B`. Neither
program calls Service or Connect. SetAckTimeout is 90.

## 2. What the host does

`RPG_TITLES` carries a third row: land type 6, version 1.1.5, capacity 100, description `Cawdor`.
`LandCatalog::stock()` lists it after Yserbius and before Twinion (place 1, any directory version,
stamp 2). A land GrpJoin is admitted only when the three version bytes are that row's
(`rpg::admits`): 1.1.5 on type 6, and not on 4 or 8; 1.0.22 and 1.1.7 are refused on type 6 with
Nak 10 code 6.

The login Ack's `rating` word has one bit per catalog land, bit `landType − 1`. Type 6 is bit 5
(`0x20`). An account whose stored rating is 0 therefore logs in with `0x00AF` (types 1, 2, 3, 4, 6
and 8) rather than the old `0x008F`.

Everything else is the Twinion path. The 41/2 lookup of type `0x8B` still returns `Sid(0x008B)`.
17/2 is absorbed. A map group (kind 2, type `0x8B`) is shared inside the place the land joinNet
set, so Cawdor's map 5 is not Yserbius's or Twinion's. A party is an own group and does not open a
game session. A `Send` to land type 6, or to type `0x8B`, does not come back to the sender. The
bulletin-board key is `landType << 24 | location << 8 | section`, so a tavern post in Cawdor is not
a tavern post in Twinion.

## 3. The extra bulletin-board word

Cawdor has Twinion's BBS send at `11E2:39A8` and a second one at `11E2:3AB1` (gallery `1933:3D41`).
The second stores one more word, at byte 8, from the argument at `[bp+0xE]`. Its two callers are
`11E2:387C` and `11E2:3977`. What that word means is not known: the survey did not name it, and the
callers are not in the census listing. The host still parses the Twinion-shaped command 27, which
is what list, index, read and post use. A message that puts a word at byte 8 would shift the body
the parser reads, and is left as that.

## 4. Open

- The word at byte 8 of the extended BBS send.
- Whether game types 200–219 differ in their bodies from Twinion.
- The SID a 41/2 lookup should return. Cawdor still looks up `0x8B` and does not read the value back.

## 5. Seen from the 2.4.46 hub

One headless logon, rating `0x00AF`, passed the HostInfo 36/7 wait and drew the Fall Map. The MedievalLand list named The Ruins of Cawdor next to Yserbius and Twinion. The place row used the directory string `Cawdor` and showed occupancy 0 of 100. PLAY on the gallery joined a land group of type 6, number 1, size 100, then GrpJoin 1.1.5; the host answered `GroupJoined`. The same call sent 41/2 for type `0x8B`, took back `Sid(0x008B)`, sent 17/2 with props 8 to 11, opened a party (parameter −3, size 4) and a map group of size 80, and left the picture on the ruins gate. That session sent no command 27. CONFIRMED for this run.

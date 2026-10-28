# The Ybarra character editor and game data (`ybarra`)

The operator portal ([innkeeperd.md](innkeeperd.md) section 9) can change the characters and automaps
that The Shadow of Yserbius, The Fates of Twinion and The Ruins of Cawdor keep in the synced save
files ([saves.md](saves.md)), and show the games' items and maps from the operator's copy (section 8).
Code: `crates/innkeeperd/src/ybarra/` (file layout, cipher, field maps, item and map files) and
`crates/innkeeperd/src/admin/ybarra.rs`, `ybarra_maps.rs`, `ybarra_warp.rs`, `gamedata.rs`,
`item_list.rs`, `map_grid.rs` (the pages).

## 1. The files

| Game | Characters | Automaps |
|---|---|---|
| Yserbius | `INN\YSERBIUS\MYCHARS.DAT` | `INN\YSERBIUS\AUTOMAPS.DAT` |
| Twinion | `INN\TWINION\TWINCHAR.DAT` | `INN\TWINION\AUTOMAPS.DAT` |
| Cawdor (INN 2.4.46) | `INN\CAWDOR\CAWCHAR.DAT` | `INN\CAWDOR\AUTOMAPS.DAT` |

A character file is 3076 bytes: a 64-byte header, then six slots of 502 (`0x1F6`) bytes at
`0x40 + slot * 0x1F6`. CONFIRMED: CGENN `17ca:0009` reads slot `DS:0498` (header byte 0) at that
offset, `17ca:02ec` writes it back the same way.

In each slot, bytes 8 to `8 + 0x1C4` are **encrypted**; bytes 0 to 7 and `0x1CC` to `0x1F5` are plain.
CONFIRMED: CGENN `288c:031f` calls the encryptor `1000:0acb` before every write and the decryptor
`1000:0cd9` after every read, over the record at `DS:623A`; TWGENN, DARKSTRT, FATES, Cawdor's TWGENN
and RUINS have the same pair (section 2).

## 2. The cipher

Inputs: a shuffle table `T` (in the program's data segment), 32 single-bit masks `M` (in its code
segment) and an 8-byte key `K`. CONFIRMED from the disassembly of CGENN `1000:0acb`; the decryptor
undoes each step in reverse.

Encryption of the 452 bytes `D`, with `K[2]` first set to `K[6]`:

1. For `i` from 0: `t = T[K[0] + i]`; swap `D[i]` and `D[t]`; `D[t] ^= t`.
2. For each 4-byte block, in order: rotate `M[0..=24]` left by one, `K[5] + 1` times; rotate
   `M[18..=28]` left by one, `K[2] + 1` times (the rotations accumulate across blocks). Read the block
   as a little-endian word `X`; output bit `31 - n` is set when `X & M[n]` is not 0; store the
   output big-endian. `K[2]` becomes the byte sum of the stored block, so every block depends on the
   ones before it.
3. For `i` from 0: swap `D[i]` and `D[T[K[7] + i]]`.

The three games use **one** table and **one** set of masks (all six programs agree, byte for byte, in
the part a key reaches); Yserbius and Twinion share a key, Cawdor has its own. The editor keeps the
`T` bytes the key reaches (`max(K[0], K[7]) + 0x1C4`) and zeroes the rest.

## 3. Importing the tables

The tables and keys are Sierra's data, so they are **not in this repository**. The operator imports
them from their own copy of a game program, once per game:

- **Portal:** Records > Saved games > "game tables" (`/admin/ybarra`): pick the program; the page reads
  it in the browser and posts it as base64url (this one form may be up to about 1.4 MiB). Audited as
  `ybarra.import`.
- **Command line:** `innkeeperd admin ybarra-import --data-dir DIR FILE...`, also while the server runs.

Accepted: `CGENN.EXE` or `DARKSTRT.EXE` (Yserbius), `TWGENN.EXE` or `FATES.EXE` (Twinion), Cawdor's
`TWGENN.EXE` or `RUINS.EXE`; the 1994 and 1995 builds alike. Only the extracted tables, the key and the
program's SHA-256 are kept, in `ybarra_ciphers` ([store.md](store.md) section 3.3); the program is not.

How a program is read (`ybarra/extract.rs`): the game is the character file it names; the routine is
found by its prologue (`push bp; mov bp,sp; push ds; push es; mov ax,DS; ...`), the data segment is
its `mov ax` immediate; a far call to it (`push KEY; push ds; push TABLE; push ds; call far`) gives the
code segment's base, the key's and the table's offsets; the key is the four word stores before the
call; the masks are the operand of the routine's `mov ax,cs:[bx+MASKS]`. A result whose masks are not
the 32 bits once each is refused.

## 4. Field maps

`ybarra/<game>.json` names the bytes of the header and of a decrypted slot. Every byte belongs to
exactly one field (the tests check this), so a byte nobody has named is still editable as raw hex
(`unknown_*`). A field has an `offset`, a `type` (`u8 i8 u16 i16 u32 i32 str bytes`), optionally
`count` and `stride` for arrays, `min`/`max`, `enum` (value names), `items` (element names),
`mirror_of` (a copy the editor keeps in step with another field instead of offering it),
`summary` (shown in the slot list), and `confidence` (`confirmed`, `inferred`, `unknown`) with its
`evidence`, which the form shows on hover. `invariants` lists what the games check or recompute.

Three more keys shape the form. `label` replaces the name as the field's caption (the quest flags use
it for what the flag records). `catalog: true` marks the item ids: the map's `catalog` lists the
game's items (`id`, `name` and, for an item that has charges, `charges`, what a new one holds), the
input takes a name from one shared list filtered as it is typed, and accepts a name, `name (id)` or
a number. `beside: "items"` puts the charges of each slot next to its item; with the script, they show
only for an item that has charges, and picking an item puts in its starting charges, or the
catalog's `no_charges` value (Yserbius 0, Twinion and Cawdor 255) for one without. A number typed past
a field's bounds is set back to the bound when the box is left; the server still refuses one sent
without the script.

The item names, charges and slot layout come from each game's item database (`RESOURCE.038`, 256
records of 46 bytes whose words are stored rotated; the games decode a record by rotating each word
left by 6) and the inventory code. In all three games slots 0 to 9 are the chest or backpack, 10 to 14
are unused (cleared whenever the inventory is built), 15 to 20 are worn (head, body, weapon hand,
shield hand, two accessories), 21 to 30 the quest item bag and 31 the holding box. The quest flags
(record `0xF6` to `0x1D5`, flag `i` at `0xF6 + i`) are named from the map scripts (`RESOURCE.001` to
`034`, one compiled module per map) that set and test them, grouped by the map that sets them; flags
no script references are kept as raw bytes. Most Yserbius and Twinion flags are CONFIRMED; most of
Cawdor's are INFERRED.

## 5. The pages

From an account's saved games, a character file links to its editor:

- `/admin/saves/<account>/ybarra/<game>`: the six slots with their summary fields, links to each
  slot's form, its automap, and the header's form.
- `/admin/saves/<account>/ybarra/<game>/<1-6|header>`: one form, fields grouped as the map groups
  them; arrays as a block of inputs named by `items`; enums as lists (a stored value with no name is
  kept and shown as such).

A POST stores the whole file as a **new current version** (`save_write`, `current`), so the version
before it stays in the history ([saves.md](saves.md) section 4; three are kept) and can be downloaded
from the saved-games page. Refused, with nothing stored:

- **409** while the account is online (its game would overwrite the change, as for flags and money);
- **409** when the file's current version is not the one the form was opened on (a sync happened);
- **422** when a value does not fit its field; the form comes back with what was sent.

An unchanged form stores nothing. Each stored change is audited as `save.edit`, naming the file,
the new version, the part and the changed fields (never the values), with the operator's reason.

The player gets the change at the next sign-on: the page sees a newer host version of a file it had
not changed, and downloads it ([saves.md](saves.md) section 5; also the HTTP prefetch, section 6).

## 6. Automaps

An automap file is 168000 bytes. Slot `s`, map `m` (0 to 33) is the 800 bytes at
`s * 27200 + m * 800` (the last 4800 bytes are unused). A block is 256 cells (`row * 16 + col`),
two 256-byte wall planes and two 16-byte edge strips; `0xFF` is unexplored. CONFIRMED on real saves
of all three games: a new character's blocks are all `0xFF`, and the games turn every kept cell
other than 12, 13 and 14 into 0 when they save. The map names and their codes come from the field
map's `automap` section (`maps`, with `names_confidence`).

`/admin/saves/<account>/ybarra/<game>/<1-6>/map` lists the 34 maps with how many cells are explored,
and `?map=N` adds a 16 by 16 grid of tickboxes (ticked = explored) and the block as hex. A POST names
one action:

- `forget_all`, `forget`: every map, or one, back to `0xFF`, cells and walls (each needs a yes);
- `cells`: a newly ticked cell becomes 0 (visited floor), an unticked one `0xFF`; a cell that stays
  ticked keeps its value, and the walls are left for the game to fill in;
- `raw`: the block's 800 bytes, as hex.

Each is refused, stored and audited as in section 5. The slot list marks a slot whose header byte
says it is not in use as vacant.

The slot list and each slot's form also link the slot's **warp** page (section 8.3).

## 7. Tests

`ybarra::cipher` (round trip, chaining, mask check), `ybarra::extract` (a program laid out like the
games; with `INNKEEPER_YBARRA_DIR` set to a folder holding `YSERBIUS`, `TWINION` and `CAWDOR` program
copies and a `saves` folder of character files, the real programs, each game's two programs giving the
same cipher, and each save opening and sealing back to the same bytes), `ybarra::schema` (bounds,
strings, raw bytes, coverage of every map), `ybarra::automap` (block layout, cells), `admin::ybarra` (forms, mirrors, escaping),
`admin::ybarra_maps` (the hex round trip),
`admin::tests::the_character_editor_imports_tables_edits_a_slot_and_refuses_stale_or_online_edits` and
`admin::tests::the_automap_editor_sets_cells_forgets_maps_and_takes_raw_bytes`.

Game data (section 8), all on made-up records: `ybarra::game_file` (word rotation, telling the files
apart; with `INNKEEPER_YBARRA_DIR` holding `RESOURCE.038` and `RESOURCE.035` in each game folder, the
real files parse, every item has a known type and the top row and left column of the maps are walled
north and west), `ybarra::items`, `ybarra::dungeon`, `admin::map_grid`, `admin::item_list`,
`admin::ybarra_warp` (every game names the position fields), `admin::item_pickers` (worn lists by
type and guild, weakest first, the current item kept, unused slots Empty only) and
`admin::tests::the_game_data_pages_import_files_show_items_and_maps_and_the_warp_moves_a_character`.

## 8. Game data: items and maps

The item and map files are Sierra's data, so they are not in this repository, nor are the tables
of what they hold. The operator imports them from their copy, like the cipher (section 3):

- **Portal:** Game data > "Game files and import" (`/admin/gamedata`): pick the game and the file;
  the page reads it in the browser and posts it as base64url. Audited as `ybarra.data`.
- **Command line:** `innkeeperd admin ybarra-data --game yserbius|twinion|cawdor --data-dir DIR FILE...`,
  also while the server runs.

The two files are told apart by size: `RESOURCE.038` (items) is 11,776 bytes and `RESOURCE.035`
(maps) 218,144 bytes in all three games. A file must parse before it is kept; it is kept whole, as it
lies in the game's folder, in `ybarra_resources` ([store.md](store.md) section 3), and parsed when a
page is opened. The short name lists the pages use (guilds, spells, skills, map names and codes)
come from the field maps (section 4).

Both files store every 16-bit little-endian word rotated right; the games read them with the
resource reader DARKSTRT `1C76:0333` (FATES `1CF7:0339`, RUINS `1E46:0326`), whose load mode `n`
rotates each word left by `n - 1`. CONFIRMED (`0000:7745` is `rol ax,cl`).

### 8.1 Items (`RESOURCE.038`, `ybarra::items`)

256 records of 46 (`0x2E`) bytes, record `id` at `id * 0x2E`, read with mode 7 (rotate left by 6).
The same layout in all three games.

| Offset | Field | Status | Evidence |
|---|---|---|---|
| `0x00` | name, NUL-terminated, at most 23 letters | CONFIRMED | the names the games show; every record of all three copies is printable text ending in a NUL before `0x18` |
| `0x18` | word, 0 to about 90 (high byte 0 in every record) | unknown | not traced; shown under "Unknown bytes" |
| `0x1A` | type (word) | CONFIRMED | equip routine RUINS `1005:087C` switches on `type - 1` through the table at `1005:0A11` (DARKSTRT `0C77:0899`, FATES `0CA8:0895`) |
| `0x1C` | charges a new one holds; 0 = none | CONFIRMED | inventory builder RUINS `0B1A:07AA`, DARKSTRT `0A7B:0733` (section 4) |
| `0x1D` | guilds barred: bit `0x80 >> guild` | CONFIRMED | guild check DARKSTRT `0BEA:04B8`, FATES `0C1B:04B4`, RUINS `0BA1:04BB` |
| `0x1E` | top three bits ORed into the wearer's status byte (`char+0x23`) while worn; low five bits unknown | CONFIRMED (where); meaning unknown | DARKSTRT `0A7B:06D9`, RUINS `0B1A:078B` |
| `0x1F`, `0x20` | two bytes, both 0 in most records, else small numbers | unknown | shown under "Unknown bytes" |
| `0x21` | worn bonus, a byte added to `char+0x01` | CONFIRMED (add); label "Strength" INFERRED | RUINS `1005:08D9` `add es:[bx+1],al`; strength is `char+0x00` |
| `0x22` | worn bonus, zero-extended and added to the word `char+0x08` | CONFIRMED (add); label "Dexterity" INFERRED | RUINS `1005:08EB` (`mov ah,0`; `add word es:[bx+8],ax`); dexterity is the word `char+0x06` |
| `0x23` | worn bonus, byte add to `char+0x04` | CONFIRMED (add); label "Defense" INFERRED | RUINS `1005:08FC`; defense is `char+0x03` |
| `0x24` | worn bonus, zero-extended word add to `char+0x12` | CONFIRMED (add); target unnamed | RUINS `1005:090B` |
| `0x25`, `0x26` | spell index (`0xFF` none) and its level | CONFIRMED | research notes for the item editor (section 4); names from the field map's `spell_ranks` |
| `0x27`, `0x28` | skill index (`0xFF` none) and level; level `0xFF` = the wearer's own rank | CONFIRMED | RUINS `1005:0997`..`09F6` reads `char+0x54 + skill` when the level is `0xFF` |
| `0x29` | worn bonus, byte add to `char+0x0D` | CONFIRMED (add); label "Agility" INFERRED | RUINS `1005:091C`; agility is `char+0x0C` |
| `0x2A` | worn bonus, byte add to `char+0x10` | CONFIRMED (add); label "Initiative" INFERRED | RUINS `1005:092B`; initiative is `char+0x0F` |
| `0x2B` | consumable: removed when its charges run out | CONFIRMED | RUINS `0B1A:02B1`, DARKSTRT `0A7B:01A3` |
| `0x2C` | tradeable | CONFIRMED | DARKSTRT `0F8A:03E1`, FATES `0FEB:03DE`, RUINS `0EE3:03EA` |
| `0x2D` | icon (20 to 38, 0 for most quest items) | INFERRED | values only; Cawdor's notes leave it unknown |

A byte add to a byte wraps, so the page shows those four bonuses as -128 to 127 (Cawdor's heavy
weapons have `0xFD` = -3 defense); the two word adds are 0 to 255. **No record holds a price, a damage
range or a race restriction**: no such field is known, so the page has none.

Types (`ybarra::items::ITEM_TYPES`): the worn slot of each is CONFIRMED by the equip switch; the names
are INFERRED from the items that have the type (`ITEM_TYPE_NAMES`).

| Type | Name (INFERRED) | Worn at |
|---|---|---|
| 0 | None (item 0 only) | |
| 1 | Blade, axe or club | weapon hand |
| 2 | Staff, spear or polearm | weapon hand |
| 3 | Bow or sling | weapon hand |
| 4 | Weapon (no item has it) | weapon hand |
| 6 | Shield (Twinion and Cawdor: or gauntlets) | shield hand |
| 7 | Head armour | head |
| 8 | Body armour | body |
| 9 | Accessory | either accessory slot |
| 11 | Usable: scroll, potion, wand | carried |
| 12 | Quest item, key or other | carried |

The page, `/admin/gamedata/<game>/items`, lists all 256 records: id, name, type and slot, the guilds
that may use it, charges, spell, skill, the six bonuses, icon, flags (used up, tradeable, worn status
bits) and the unknown bytes as hex. Filters (a GET form): type, a guild that may use it, and a power
(charges, a spell, a skill, a bonus, used up, tradeable). Each heading sorts by its column; a second
click reverses. "By type" counts the items of each type and links the filter. Without script it all
works the same.

### 8.2 Maps (`RESOURCE.035`, `ybarra::dungeon`)

34 blocks of `0x1910` bytes, block `n` for map `n` in the order of the game's map table (the
automap's order, section 6, whose names the page uses). The map loader reads the first `0xC10` bytes
of a block with mode 3 (rotate left by 2) into its map buffer. CONFIRMED: DARKSTRT file `0xD8F7`
(image `0xA0F7`; file = image + `0x3800`) pushes mode 3, `index * 0x1910` and `0xC10` for `ds:A63F`;
FATES file `0xDD09` (`ds:8E27`) and RUINS file `0xE4E2` (`ds:8FD9`) are the same code. The name
`resource.035` is a string in DARKSTRT (file `0x2319E`). The rest of each block is not read by that
call; its use is unknown.

A plane is 256 bytes, one per cell, cell = `row * 16 + col`.

| Offset in the block | What | Status |
|---|---|---|
| `0x000` | 16 header bytes (`01 nn nn 00...`) | unknown; shown as hex |
| `0x010` | walls: two bits per side, side `s` at bit `2s`; 0 is open | CONFIRMED: DARKSTRT image `0xA25E`..`0xA26C` shifts `ds:A64F[cell]` right by `2 * side` and tests `& 3` (FATES file `0xDE7C`, RUINS file `0xE61B`) |
| `0x110 + side * 0x100` | side kind, per side | CONFIRMED read at load (DARKSTRT image `0xA28A`..`0xA305`); kind 1 makes the side passable even where the wall bits are set (image `0xA31C`, `cmp byte [bx],1`; FATES file `0xDF2E`, RUINS file `0xE6CD`). Other kinds (2 to 7) unknown; most come with a side event |
| `0x510 + side * 0x100` | side event, plus one | CONFIRMED (flag research: dispatchers at DARKSTRT image `0xA427`, `0xA4E1`, `0xA65E`) |
| `0x910` | entry event, plus one | CONFIRMED (same) |
| `0xA10` | second cell event, plus one (mostly one-shot fights and treasure) | CONFIRMED (same) |
| `0xB10` | a byte per cell, 0 to 12 | CONFIRMED read at load (image `0xA214`, values 5 and 6 handled apart); meaning unknown |

**Which side is which is INFERRED** (`ybarra::dungeon::SIDE_ORDER_INFERRED`): side 0 west, 1 south,
2 east, 3 north. In all three games nearly every walled cell of the top row has its side 3 set and
of the left column its side 0 (Yserbius 418 of 422 and 407 of 415, Twinion and Cawdor 544 of 544);
a cell's side 2 equals its east neighbour's side 0 in 91 to 99 per cent of cells, and its side 3 its
north neighbour's side 1 likewise. The wall values 1 to 3 differ in something not traced (likely the
picture). That a passable wall is a door or a secret way is INFERRED.

The page, `/admin/gamedata/<game>/maps?map=N`, draws the chosen map (section 8.3) with its header
bytes, and lists the 34 maps with their codes and how many cells have a wall or an event.

### 8.3 The grid control and the warp

`admin::map_grid::map_grid` draws one map as a 16 by 16 table, north up, row and column numbers
round it: a solid edge is a wall, a dashed edge a passable wall, a square dot a cell with an event,
and an optional highlighted cell; each cell's tooltip names its walls, side kinds, events and cell
byte. The caller puts what it likes in each cell. Without an imported maps file it is a bare grid.

**Warp.** `/admin/saves/<account>/ybarra/<game>/<slot>/warp?map=N` shows the slot's saved position
(map, row, column, facing), a map chooser (the field map's 34 maps) and the grid of the chosen map,
the saved cell highlighted with an arrow when it is on that map. Each cell is a submit button of one
form that posts to the slot form's own address, `/admin/saves/<account>/ybarra/<game>/<slot>`, with
the record fields by their form names: `map_code` (`0xF2`, the chosen map's code), `position` (`0xF4`,
`row * 16 + col`, the clicked cell) and `facing` (`0xF5`, from a list), plus `version`, the optional
reason and `view=warp` so the answer is the warp page again. So it is the same save: refused while
the account is online (409), when the file changed since the page was opened (409), or for a value
outside its field (422); stored as a new version; audited as `save.edit` naming the changed fields
(for example `character slot 1: map_code, position, facing`). Enter in the reason box submits
nothing: the form's first button is a disabled one.

What the fields are: CONFIRMED for Yserbius and Twinion, INFERRED for Cawdor (section 4, the field
maps' evidence). They hold the position the game saved when the character left the dungeon
("Position (last teleport-out)"); the game loads them with the character. Whether a session then
starts at that cell or only the teleport back uses it is not traced, so test a warp in the game. The facing values are named
like the sides (0 west, 1 south, 2 east, 3 north), INFERRED: the new-character facings (Yserbius 2 at
row 3, column 0 beside a west wall; Twinion 3 at row 13, column 8 with walls south and east) fit it.
The warp does not touch the automap, nor check that the cell is inside the map's walls.

### 8.4 The item pickers of the slot form

The slot form's 32 item inputs each take a name from a list (section 4). With the game's
`RESOURCE.038` imported, a slot the game restricts gets a list of its own (`admin::item_pickers`);
without it, or with "Show all items in every picker" (`?all=1`, kept across a save), every slot uses
the full list. The lists only suggest: any item can still be typed, the save checks only the field's
bounds as before, and the item in a slot now is always in its list (first, labelled "in the slot now;
not usable here" when the rules exclude it), so odd gear is never changed by opening and saving.

| Slots | Area | Rule in the picker | Status |
|---|---|---|---|
| 0 to 9 | backpack (chest) | any item | no restriction is known (manual: any item may be moved between chest and bag) |
| 10 to 14 | unused | Empty only | CONFIRMED: the inventory builder empties them every time (section 4) |
| 15 | head | type 7 | type CONFIRMED (below) |
| 16 | body | type 8 | type CONFIRMED |
| 17 | weapon hand | types 1 to 4 | type CONFIRMED |
| 18 | shield hand | type 6 | type CONFIRMED |
| 19, 20 | accessories | type 9 | type CONFIRMED |
| 21 to 30 | quest item bag | any item | no restriction is known |
| 31 | holding box | any item | no restriction is known |

Types: RUINS `1005:087C`, called at load for each of the six worn positions (`1005:05CD`), switches
on `type - 1` through the table at `1005:0A11` (types 1 to 4 to the weapon position, 6 shield, 7
head, 8 body, 9 either accessory; 5 and every other type match nothing) and adds an item's worn
bonuses and skill only when its type matches the position (DARKSTRT `0C77:0899` and FATES `0CA8:0895`
the same). Whether the game's screen also refuses to put a wrong type there is not traced; a wrong
type gives nothing, so the picker leaves it out. Empty (item 0) is always offered.

Guild: worn slots also drop the items whose bar mask (`0x1D`) bars the character's guild (record
`0x21`). CONFIRMED that the game checks it: RUINS `0BA1:04BB` tests `0x80 >> guild` against the item
and prints "That item is not usable by your guild!" (`ds:105C`, file `0x261DC`) instead of going on
(DARKSTRT `0BEA:04B8`, FATES `0C1B:04B4`). That the refused action is wearing, and not only using, is
INFERRED (`GUILD_BAR_GATES_WEARING_INFERRED`). No level, race or alignment restriction is known: the
item record has no such field (section 8.1), so there is none in the picker.

**Power score** (`Item::power`), which orders each restricted list weakest first (ties by name, then
id; Empty first): the sum of each worn bonus times a weight, plus 100 points per level of the item's
spell and of its skill. The weights (`POWER_WEIGHTS_INFERRED`) give each attribute about 3,200 points
over its whole clamp range: strength 32 and defense 32 (0 to 99), dexterity 1 (0 to 3,200),
agility 64 and initiative 64 (0 to 50), the unnamed `+0x12` word 1 (range unknown). A skill that works
at the wearer's own rank counts as level 6 (`OWN_RANK_LEVEL_ASSUMED`, the middle of 0 to 12); 100 per
level is `POWER_PER_LEVEL_INFERRED`. Charges and the used-up flag do not count. All of it is a
judgement for sorting, INFERRED, not anything the games compute.

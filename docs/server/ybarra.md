# The Ybarra character editor (`ybarra`)

The operator portal ([innkeeperd.md](innkeeperd.md) section 9) can change the characters and automaps
that The Shadow of Yserbius, The Fates of Twinion and The Ruins of Cawdor keep in the synced save
files ([saves.md](saves.md)). Code: `crates/innkeeperd/src/ybarra/` (file layout, cipher, field maps)
and `crates/innkeeperd/src/admin/ybarra.rs`, `ybarra_maps.rs` (the pages).

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

## 7. Tests

`ybarra::cipher` (round trip, chaining, mask check), `ybarra::extract` (a program laid out like the
games; with `INNKEEPER_YBARRA_DIR` set to a folder holding `YSERBIUS`, `TWINION` and `CAWDOR` program
copies and a `saves` folder of character files, the real programs, each game's two programs giving the
same cipher, and each save opening and sealing back to the same bytes), `ybarra::schema` (bounds,
strings, raw bytes, coverage of every map), `ybarra::automap` (block layout, cells), `admin::ybarra` (forms, mirrors, escaping),
`admin::ybarra_maps` (the hex round trip),
`admin::tests::the_character_editor_imports_tables_edits_a_slot_and_refuses_stale_or_online_edits` and
`admin::tests::the_automap_editor_sets_cells_forgets_maps_and_takes_raw_bytes`.

# The Shadow of Yserbius: host message reference

What `innkeeperd` answers for the Yserbius programs (`CGENN.EXE` gallery, `DARKSTRT.EXE` game) and,
since they share the engine, for Twinion (`TWGENN.EXE`, `FATES.EXE`). The client-side evidence for every
layout is the census, [int14h-census/yserbius.md](int14h-census/yserbius.md) and
[int14h-census/twinion.md](int14h-census/twinion.md); this page says what the host does, where it lives in
the code and how sure each choice is. Notation as in [messages.md](messages.md): `b` byte, `w` LE word,
`s` NUL-terminated string, `@n` byte offset.

No captures of the original service exist for these titles. Every row below was established from the
client code (DARKSTRT addresses `seg:off` in the load image, file offset = `0x3800 + seg*16 + off`) and then
checked live: the stock client in `dosrt` (INN-Decomp, `build-ysv/inn --com1`) against `innkeeperd`
(section 7). Labels: **CONFIRMED** (code and live run agree), **CONFIRMED (code)** (read in the code, not
exercised live), **INFERRED** (a host choice the client does not pin down; each is a named constant in
`crates/innkeeper-world/src/assumptions.rs`).

## 1. Session and identity

| Fact | Host behaviour | Status |
|---|---|---|
| The programs never dial or log in; they inherit the land's session | `PlayerSession` stays logged in across the program switch; without a hub, `tools/dosbox/launcher` logs in and writes the shared block (`docs/dosbox.md` section 6) | CONFIRMED |
| Shared block `+0` user SID (joinNet cookie), `+2` userFlags, `+6` land number, `+8` land type | the hub writes them; the launcher's YSERBIUS / TWINION blocks name place 1 of land type 4 / 8 | CONFIRMED |
| `LAND.CFG`: Yserbius is land type 4, Cawdor 6, Twinion 8 | `LandCatalog::stock()` lists the Arena (type 5, place 1, maximum 64, description `Arena`) after the three LSCI lands, then these three (place 1, maximum 100, descriptions `Yserbius`, `Cawdor`, `Twinion`, any version), directory stamp 2. Cawdor's version gate is [cawdor.md](cawdor.md) | CONFIRMED (data); row values INFERRED |
| The hub lets a player into a land only if bit `landType - 1` of the login Ack's `rating` word is set (`hub/script.050 proc_310`; 0 counts as 1, the Clubhouse alone): otherwise "Sorry, but you do not have access rights for …" | an account whose stored rating is 0 logs in with one bit per land the catalog lists (`0x00BF` for the stock seven: types 1, 2, 3, 4, 5, 6 and 8) | CONFIRMED (code and live); the default INFERRED (`UNSET_RATING_GRANTS_EVERY_LAND_ASSUMED`) |
| Characters live in `MYCHARS.DAT` on the client; no message uploads one | nothing is stored per character; boards are the only persistent RPG state | CONFIRMED (code: no sender); INFERRED that the host never stored them |

## 2. Commands from the client

| Cmd/sub | Layout | Host answer | Code | Status |
|---|---|---|---|---|
| 7/0 kind 1 | `b 7, b 0, w 0, w cookie, b 1, b 0x8B, w -1, w 1` | `ObjId` with a new SID (player object) | `objects::ObjectStore::join_within` | CONFIRMED |
| 41/2 | `b 41, b 2, w 0, w player, b 0x8B, w 0` | `b 41, b 0, w player, w 0, w 0x008B`: the client stores w@6 and ends its 60 s resend loop | `rpg::locate_service` | CONFIRMED; the SID value INFERRED (`RPG_SERVICE_SID_ASSUMED`, nothing reads it back) |
| 17/2 (Twinion) | `b 17, b 2, w service, w player, w 8, w 9, w 10, w 11` | none; the client does not wait | `ServicePropertyRequest`, absorbed | CONFIRMED (no wait); meaning of the props unknown |
| 7/3 kind 5 | `b 7, b 3, w 0, w player, b 5, b landType, w landNo, w 100` | `ObjId`; one shared group per (land type, land number); it becomes the player's **place** | `rpg::place_of`, `PlayerSession::join_net` | CONFIRMED |
| 10/3 | `b 10, b 3, w landGroup, w player, b 1, b 1, b 7` | echo `b 10, b 0, w group, w player` to the joiner and the members, or Nak 10 code 6 unless the version is 1.1.7 (Twinion 1.0.22) | `presence::join_group`, `rpg::admits` | CONFIRMED; exact-version gate INFERRED (`RPG_LAND_ADMITS_ONLY_ITS_VERSION_ASSUMED`) |
| 7/0 kind 4 | `… b 4, b 0x8B, w -3, w 4` | `ObjId` of a group of the player's own (personal party, 4 members) | `objects` own group | CONFIRMED |
| 7/0 kind 2 | `… b 2, b 0x8B, w map, w 104` (Twinion 80) | `ObjId`; shared by map word **within the player's place** | `rpg::join`, `GroupKey::within` | CONFIRMED; scoping INFERRED (`MAP_GROUPS_PER_PLACE_ASSUMED`) |
| 10/0 | `b 10, b 0, w group, w member` | echo to the joiner and the members; Nak 10 codes 1 to 3 | `objects::join_group` | CONFIRMED |
| 11/0 | `b 11, b 0, w group, w member` | `GrpDel` to the remaining members | `objects::leave_group` | CONFIRMED |
| 9/0 | `b 9, b 0, w sid` (4 bytes, no trailing word) | release; the last holder frees, peers get `ObjFree` | `ClientMessage::LeaveNet` | CONFIRMED |
| 40/4 | `b 40, b 4, s characterName` | none; the session's persona becomes the character name | `SetPersona` | CONFIRMED |
| 2/0 | `b 2, b 0, w to, w from, b msgType @6, body` | relayed unchanged to the holders of `to` (an object) or of its members (a group), **not** to the sender | `router::send`, `rpg::echoes_sender` | CONFIRMED (relay); no echo INFERRED (`RPG_SENDS_ECHO_SENDER_ASSUMED`, section 4) |
| 27/x | section 3 | section 3 | `bbs`, `message::bbs` | see section 3 |

Commands the host never sends unprompted and the client tolerates: `setInt` 13 (the third pair's value
in 12..48 would become the party-step delay), `Unsolicited` 48 (operator text). Neither is sent today.

## 3. Bulletin board (command 27)

Requests are `b 27, b sub, w @2, w fromSID @4, w @6, body @8`; replies use the same header with the
requester's SID at `@4`. The board words `@2`/`@6` are whatever the directory handed out: `innkeeperd`
puts the location there and the section number at `@6`. Evidence: the `rpgbbs.cpp` builders
`10F7:2EAC` (list), `30B6` (index), `31B7` (read), `32D5` (post) and the handlers `2F14`, `315C`, `323D`.

| Sub | Request | Reply | Status |
|---|---|---|---|
| 13 directory | `@2 = 0`, `@6 = location` (8 bytes) | `b 27, b 13, w n @2, w to @4, w location @6`, n records of 17 bytes from `@8` (`b flags` bit 0 = readable, `w total`, `w location`, `w section`, `w indexed` ≤ 120, 8 unread zero bytes), then n names (≤ 21 chars, `_` for a space) | CONFIRMED |
| 14 index | `@2`, `@6` of a record; 27 bytes, the last 11 never filled | `b 27, b 14, header, 10 unread bytes, w message[indexed] @0x12` (the numbers 1..n) | CONFIRMED |
| 1 to 5 read | `@2`, `@6`, `w message @8` (10 bytes) | always sub 1: `header, w number @8, w position @10, 6 unread bytes, s "author\|text" @0x12`; a reader with userFlags 2 or 4 reads the text at `@0x16`, so the host inserts 4 zero bytes | CONFIRMED layout; step meanings INFERRED |
| 0 post | `@2`, `@6`, `"author\|text\0"` from `@8`, padded to 309 bytes | none; stored as `author\|text` | CONFIRMED |
| any | a request outside a land, or a store fault | Nak 27 (`b 1, …, b 27 @4`): "BBS Server offline.", ends the client's wait | CONFIRMED (code) |

Read steps (the reader's buttons, event ids `0x644..0x647` → subs 2, 3, 4, 5, table at overlay file
`0x4A696`): 1 this message, 2 next, 3 previous, 4 first (sent when a section opens), 5 last. The client
sends 2 only while position bit `0x4000` is clear and 3 only while bit `0x8000` is clear
(`10F7:2E3E`, `2E5B`), so the host sets `0x4000` on the last message and `0x8000` on the first.
Messages are numbered from 1, oldest first. The pairing of buttons and subs is INFERRED from the
order of the strings `FIRST LAST NEXT PREV` and the flag gates; it behaved as expected live.

Locations the clients ask for (both titles push the same values before `10F7:2D0E` / `1151:2EB2`):
`0x59` tavern posts (`TAVERN POSTS`), `0x5A` Hall of Heroes (read at open, posted to on entry), `0x63`
complaints (`Complain to TSN Host`, post only). The host offers one section per location: `Tavern_Posts`,
`Hall_of_Heroes` and `Complaints` (not readable). An unknown location gets an empty directory. Boards
are kept per title: the store's `BoardId` is `landType << 24 | location << 8 | section`. All of this
directory content is INFERRED; the original sections are unknown.

A client that posts while a section is open re-reads the index but not the directory, so the new
message appears the next time the section is opened (seen live; the client's own behaviour).

## 4. Game messages (msgType 200 to 219)

The host relays them unread (census section 5). It does **not** return a group `Send` to the sender's
own connection for RPG objects (land types `0x8B`, 4, 6 and 8): DARKSTRT prints its own chat line
locally after sending it (overlay dispatch at file `0x3C7EF`, print at `0x3C81C`), so an echo would
show every line twice, and with the echo the client answered its own 200/1 and 217/1 hellos (seen
live before the change). INFERRED; LSCI, GOLF and Red Baron keep the echo.

## 5. Twinion differences (handled by the same code)

Land type 8, land GrpJoin version 1.0.22, map capacity 80 (the client sends it), 17/2 after the
service lookup (absorbed), game type 219 (relayed). Separate land, map groups and boards from Yserbius.
Cawdor is the same engine on land type 6 at version 1.1.5 ([cawdor.md](cawdor.md)).
Live: TWGENN reaches its gallery against `innkeeperd`; FATES was not driven further.

## 6. Open questions

- What 41/2 and 17/2 asked the original host for, and whether 17/2 was answered with a `setInt`
  (prop 10 would be the party-step delay the 13 handler reads).
- The original boards, their names and how many sections each location had; what the 6 unread bytes
  of a message (perhaps a date) and the 4 privileged bytes held.
- Whether map groups were scoped by place, by land or host-wide.
- Whether the original host echoed group sends; section 4 argues it did not for these titles.

## 7. Live verification (2026-10-14)

`innkeeperd --bind 127.0.0.1:2570`, client tree copied from INN-Decomp `work/yserbius-c`, launcher
`LaunchYserbius`: CGENN joins its player and gets the 41 reply; Play starts DARKSTRT, which runs the whole
join sequence (player, 41/2, land, 10/3, party, 40/4, 217/1, map, 200/1) and shows the town; the tavern's
POSTS lists `Tavern Posts`, a posted message is read back as "Message 1/1 From Grunt" in a new session;
the volcano door enters the dungeon (map group re-joined for map `0x0101`). From the hub (`tsnexec
DEFAULT`, Play, town map, the Yserbius place, Play in the gallery) the same sequence follows the hub's
leaveNet of its game object, once the login Ack grants the land. Two clients at once share the
land and map groups and answer each other's 200/1 and 217/1.

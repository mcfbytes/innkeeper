# Top scores (27/13, 27/1, 27/0 and 27/6 on boards 10 to 13)

What the stock client sends and expects for the top-score boards: the SierraLand **Top Scores room**,
which shows a board, and the **score object** that posts a player's result when they come back to the
SierraLand waiting room from a game. Part 1 is the protocol only; it is what `srv-scores` implements.
Offsets in message tables are message bytes from 0; `w` is a little-endian word. Code offsets are
`+0x....` from the start of the named method, as `tools/lsci_disasm.py` prints them.

## Part 1. Protocol

### 1.1 Sources

| Code | Where | What |
|---|---|---|
| `ScoreObj` (class) and `baronScoreObj`, `golfScoreObj`, `pballScoreObj`, `wpsGolfScoreObj` | `SL/script.120` items 98 to 123 | reads a board, merges the player's score, posts it back |
| `WaitingRoom::init`, `PlayerLogin::changeState` | `SL/script.120` items 62 and 393 | start the right score object on entry |
| `HighScores` room, buttons `bRedBaron`, `bWPSGolf`, `bMiniGolf`, `bPaintBall`, `proc_52`, `ClearScoreMI` | room 8910, resource `SL/vocab.718` | shows one board |
| `export_2` | `SL/script.000` item 84 | turns the DOS games' result word into the land's score |
| `attachScript::changeState`, `proc_125` | `SL/script.101` items 52 and 125 | read the shared block on return |
| `class_72` (message 27), `class_64` (Ack), `class_62::wordAt` | `SL/type31.559` | how the client reads the replies |
| `class_61::lock`, `unlock` | `SL/type31.558` | lock (4) and unlock (6) |
| `Obj::joinNet`, `Obj::dispose` | `SL/type31.608` | joinNet (7) and leaveNet (9) |
| `class_84::handleMsg` (the player object) | `SL/type31.561` item 27 | turns a lock Ack into the second directory request |

Room 8910 is a script number above 2047. A SCI0 resource id keeps 11 bits of number, so the map
entry for script 8910 (`2 << 11 | 8910` = `0x32CE`) decodes as vocab 718, and the extracted file is
`SL/vocab.718`; `lsci_disasm.py --script 8910` reads it once the file is linked under that script number. The "Top Scores
Room" button of the waiting room's Go to panel is game number `0x22CE` (`SL/script.120 Go to::select
+0x0084`, `SL/script.001` item 35 "Best 10 Game Scores"), and `newRoom` starts the script of that
number. CONFIRMED (the room's buttons and help text are in that resource, and the room ran live,
section 1.11).

### 1.2 Who reads and posts

| Sender | Board read | Board written | Status |
|---|---|---|---|
| Top Scores room (SierraLand, Go to, Top Scores Room) | 27/13 then 27/1 of the board of the button pressed | never | CONFIRMED (`SL/vocab.718 proc_52 +0x006C`, `HighScores::handleMsg +0x025F`), live |
| Score object, on entering the SierraLand waiting room with a pending score ("Checking Top Scores...", then "Updating Top Scores...") | 27/13 then 27/1 | 27/0 of the whole table | CONFIRMED (`SL/script.120 ScoreObj::init`, `ScoreObj::handleMsg`) |
| "Clear Scores" (room menu, sysop only) | none | 27/6 message 1 of all four boards | CONFIRMED (`SL/vocab.718 ClearScoreMI::doit +0x001E`..`+0x0087`, shown when globals 165 and 82 are set, `HighScores::init +0x008A`) |
| CasinoLand (`LL`) and the hub | none | none | CONFIRMED by scan: no "Top Scores" text, no `ScoreObj`, and every `TSN` send of command 27 in `LL` and `hub` is the bulletin board, the complaint form, the conference and trivia topics or (`LL/script.1100 object_156::doit +0x0085`) a 27/1 of a chat board's random messages |

The Feb-94 SierraLand boards are the only top-score boards of this client.

### 1.3 Board keys and direction

The score objects are numbered 110 to 113 and use `number - 100` as the board key; the room uses
`10 + button index`. Both put the key in the word at byte 6 of every request (sections 1.5 to 1.10).

| Key | Game | Score object (`number`, `state`) | Room button (`proc_52` second argument) | Best is | Status |
|---|---|---|---|---|---|
| 10 | Red Baron | `baronScoreObj` (110, 0) | `bRedBaron` (0) | high | CONFIRMED (`SL/script.120` item 117; `SL/vocab.718 bRedBaron::doit +0x0000`) |
| 11 | MiniGolf | `golfScoreObj` (111, 1) | `bMiniGolf` (1) | low | CONFIRMED (item 119; `bMiniGolf::doit +0x0000`) |
| 12 | PaintBall ("PaintBall II" on the button) | `pballScoreObj` (112, 0) | `bPaintBall` (2) | high | CONFIRMED (item 121; `bPaintBall::doit +0x0051`) |
| 13 | 3-D Golf | `wpsGolfScoreObj` (113, 1) | `bWPSGolf` (3) | low | CONFIRMED (item 123; `bWPSGolf::doit +0x0000`) |

`state` 1 means low is best: the score object treats a 0 entry as empty (`0x7FFF`) while sorting and
sorts ascending (section 1.12). The key is a constant of the client, not something the host hands out.
The board key is per land only in that the requests come from a SierraLand connection; nothing in the
request names the land.

### 1.4 What score is posted

The pending score lives in the player record (global 81, getter selector `0x260`, setter `0x25F`). The
waiting room starts at most one score object when that score is non-zero (`WaitingRoom::init
+0x0251`..`+0x02B6`, the same test in `PlayerLogin::changeState +0x01B2`..`+0x020C`).

| Game | Score | Started when | Status |
|---|---|---|---|
| MiniGolf (`SL/script.779`) | the player's 18-hole stroke total; a finished round replaces the pending score when none is pending or it is lower, so the lowest of the visit is posted | the previous room (global 13) was 779 (`0x30B`) | CONFIRMED (`newHoleScript::changeState +0x0083`..`+0x0120`: only after hole 18) |
| PaintBall (`SL/script.780`) | the player object's points (selector `0x261`, raised by each award) at the end of the game, last game wins | the previous room was 780 (`0x30C`) | CONFIRMED (`PaintBall::dispose +0x0030`..`+0x003F`) |
| Red Baron (`RB.EXE`, a DOS program) | the word at shared block `+0x80` after the return | global 211 is `0x262` | CONFIRMED (path below) |
| 3-D Golf (`GOLF.EXE`, a DOS program, launched by `SL/script.910 StartGolfGame::doit +0x0076`) | the word at shared block `+0x80`; `0x7FFF` (no complete stroke-play round) is never posted | global 211 is `0x25D` and the score is not `0x7FFF` | CONFIRMED (path below) |

The DOS result word (the census documents [redbaron.md](../protocol/int14h-census/redbaron.md) section 4 and
[golf.md](../protocol/int14h-census/golf.md) section 5.2 say the games write it but no land reader was traced): on the return, `attachScript` state 1 calls
`proc_125` (`SL/script.101 attachScript::changeState +0x00C0`), which reads the shared block with
`TSN(1)` and returns a word array copied from block byte `0x80` on (`proc_125 +0x031F`..`+0x0334`,
returned at `+0x035E`). `SL/script.000 export_2` (called at `attachScript::changeState +0x0114`)
compares a land value of the arrival record (selector `0x279` of global 254) with 101 and 102: for
101 it stores element 0 as the score and sets global 211 to `0x262` (`+0x001B`..`+0x004C`), for 102
the same with `0x25D` (`+0x0052`..`+0x0083`). CONFIRMED. That `0x279` holds the block's land type
(101 is Red Baron's, 102 GOLF's joinNet `landType`) is INFERRED. So the land does read the result word
and posts it to board 10 or 13 the next time the waiting room starts.

A score of 0 is never posted (all four paths test it). Red Baron's counter can go negative
(census section 4: steps of -50); a negative word is posted as is. After any end of the score object
(posted, not placed, refused) the score is set back to 0 (`ScoreObj::dispose +0x0009`), so a score
is offered once.

### 1.5 joinNet (7) and ObjID (8) of the score object

`ScoreObj::init` shows "Checking Top Scores...", sets `signal` 1 and calls `joinNet(number)`
(`+0x0000`..`+0x0011`). `Obj::joinNet` (`SL/type31.608` item 17) sends:

| Offset | Width | Value | Status |
|---|---|---|---|
| 0 | w | 7 | CONFIRMED |
| 2 | w | 0 | CONFIRMED |
| 4 | w | cookie (the client's object handle) | CONFIRMED |
| 6 | b | kind 1 (`Object`) | CONFIRMED (`+0x000B`: one argument gives 1) |
| 7 | b | the land number, 2 for SierraLand (global 2 `number`) | CONFIRMED, live (the player object's joinNet carries `02`) |
| 8 | w | `number`: 110 to 113 | CONFIRMED |
| 10 | w | 13, the class size | CONFIRMED (`ScoreObj -size- 0x000D`) |

Example, MiniGolf, cookie `0x2BD4`: `07 00 00 00 d4 2b 01 02 6f 00 0d 00`.

The host answers with the ordinary ObjID ([messages.md](../protocol/messages.md) section 3): `08 00 d4 2b 00 00 45 01` gives the
score object SID `0x0145`. Only that reply starts the read (`ScoreObj::handleMsg +0x000A`). Whether
the original host gave every player's object of one key the same SID (which would make the lock in
section 1.8 a real board lock) is INFERRED and unknown; param 110 to 113 lets the host tell these
objects from a player object (param `0xFFFF`, size 30).

### 1.6 27/13: directory request

| Offset | Width | Room | Score object | Status |
|---|---|---|---|---|
| 0 | b | 27 | 27 | CONFIRMED |
| 1 | b | 13 | 13 | CONFIRMED |
| 2 | w | 0 | 0 | CONFIRMED |
| 4 | w | 0 (the reply goes to SID 0, the game object, which hands it to the current room) | the score object's SID | CONFIRMED (`proc_52 +0x0077`; `ScoreObj::handleMsg +0x0026`) |
| 6 | w | key | key | CONFIRMED |

Room, PaintBall: `1b 0d 00 00 00 00 0c 00` (live, section 1.11). Score object, MiniGolf, SID `0x0145`:
`1b 0d 00 00 45 01 0b 00`. The room disables its buttons until a 27/13 or 27/1 reply re-enables them
(`proc_52 +0x009D`; `HighScores::handleMsg +0x020F`, `+0x0287`), so every request must be answered; a
Nak is not handled by the Feb-94 room (it reaches `Rm::handleMsg`), and the buttons stay disabled.

### 1.7 27/13 reply, as the client reads it

The reply is the RPG directory reply ([yserbius.md](../protocol/yserbius.md) section 3). Both readers take only:

| Offset | Width | Field | Use | Status |
|---|---|---|---|---|
| 0 | b | 27 | | CONFIRMED |
| 1 | b | 13 (`whichCmd`) | selects the directory branch | CONFIRMED (`class_72::init +0x000B`) |
| 2 | w | record count | read and overwritten, never tested | CONFIRMED (`ScoreObj::handleMsg +0x0424`, `HighScores::handleMsg +0x022A`) |
| 4 | w | `toSID`: the request's word 4 | routing | CONFIRMED |
| 6 | w | key | not read | CONFIRMED |
| 8 | b | record 0 flags | not read | CONFIRMED |
| 9 | w | record 0 `total`: messages on the board | 0: the room says "Scores Not Available"; the score object writes a new table (section 1.13) | CONFIRMED (`+0x0433`; `+0x0239`) |
| 11 | w | record 0 `location` | echoed at word 2 of the 27/1 and 27/0 that follow (the score object keeps it as `selector_28b`) | CONFIRMED (`+0x0442`; `+0x0248`) |
| 13 to 24 | | section, indexed, 8 bytes | not read | CONFIRMED |
| 25 | | board names | not read | CONFIRMED |

The client never checks the count. With no record, words 9 and 11 lie past the end of the message;
the host must therefore always send exactly one record for keys 10 to 13. The reply may carry more
records; only record 0 is used.

Example, key 12 with a stored table, to the room: `1b 0d 01 00 00 00 0c 00` `01 01 00 0c 00 0c 00 01 00
00 00 00 00 00 00 00 00` `53 63 6f 72 65 73 00` ("Scores"). Record `location` is the key, so the
following requests carry the key at word 2 as well. An empty board has `total` 0 and the same record.

### 1.8 27/1: read of message 1

Sent only when `total` is non-zero.

| Offset | Width | Value | Status |
|---|---|---|---|
| 0 | b | 27 | CONFIRMED |
| 1 | b | 1 (read this message) | CONFIRMED |
| 2 | w | record 0 `location` from the reply | CONFIRMED (`HighScores::handleMsg +0x0269`; `ScoreObj::handleMsg +0x0479`) |
| 4 | w | 0 (room) or the score object's SID | CONFIRMED |
| 6 | w | key | CONFIRMED |
| 8 | w | 1: always message 1 | CONFIRMED (`+0x0272`; `+0x0483`) |

Room, PaintBall, location 12 in the record: `1b 01 0c 00 00 00 0c 00 01 00` (live). Score object,
MiniGolf: `1b 01 0b 00 45 01 0b 00 01 00`. No other read step is ever sent: a board is one message,
and the table in it is the whole board.

### 1.9 27/1 reply and the table text

The reply is the RPG message reply ([yserbius.md](../protocol/yserbius.md) section 3); for a key other than 67 the client
does not move its start (`class_72::init +0x0052`), so offsets are from byte 0.

| Offset | Width | Field | Use | Status |
|---|---|---|---|---|
| 0 | b | 27 | | CONFIRMED |
| 1 | b | 1 | selects the read branch | CONFIRMED |
| 2 | w | the request's word 2 | not read | CONFIRMED |
| 4 | w | `toSID` | routing | CONFIRMED |
| 6 | w | key (must not be 67) | not read | CONFIRMED |
| 8 | w | message number, 1 | not read | CONFIRMED |
| 10 | w | position flags, `0xC000` (first and last) | not read | CONFIRMED |
| 12 | 6 | zero | not read | CONFIRMED |
| 18 | | `author|table` and a NUL (4 more zero bytes before it for a privileged reader, as for every read) | the client searches the whole message for the first `|` | CONFIRMED (`HighScores::handleMsg +0x0055`; `ScoreObj::handleMsg +0x0051`) |

After the first `|` come 10 lines of exactly 18 bytes each (`"%6d %-10s\n"`, `SL/script.120` item 105):

| Line offset | Width | Field | Status |
|---|---|---|---|
| 0 | 6 | score, decimal, right-justified with spaces, `-` allowed; read with `asInteger` | CONFIRMED (`subStr 0 6`, `+0x00BF`; live) |
| 6 | 1 | space | CONFIRMED |
| 7 | 10 | name, left-justified, padded with spaces | CONFIRMED (`subStr 7 10`, `+0x00DE`; live) |
| 17 | 1 | `\n` (`0x0A`) | CONFIRMED (line stride 18, `+0x00FB`) |

Line `i` starts 1 + 18 `i` bytes after the `|`. The table is best first: line 0 is rank 1. The room
shows 10 rows, rank `%2d`, name and score (`HighScores::handleMsg +0x010C`..`+0x01DF`). Lines missing
at the end read as score 0 and an empty name. The author before the `|` is not shown.

Example, PaintBall, to the room, author "INN", first two lines:

```
1b 01 0c 00 00 00 0c 00 01 00 00 c0 00 00 00 00 00 00
49 4e 4e 7c                                      "INN|"
20 20 20 39 30 30 20 41 6c 70 68 61 20 20 20 20 20 0a   "   900 Alpha     \n"
20 20 20 38 30 30 20 42 72 61 76 6f 20 20 20 20 20 0a   "   800 Bravo     \n"
... 8 more lines of 18 bytes ...
00
```

203 bytes in all: 18 header, 4 author and `|`, 180 table, the NUL.

### 1.10 Lock (4), the lock Ack and the second read

When the first read shows the player's score would place (section 1.12), the score object sets
`signal` 2 and starts `scoreScript` (`ScoreObj::handleMsg +0x02C2`..`+0x02D8`). `scoreScript` state 0
locks the score object's SID on behalf of the player, flushes, and waits 5 seconds with state -1
(`scoreScript::changeState +0x000E`..`+0x003B`), so it locks again every 5 s until something clears
the script (the retry is INFERRED from the `Script` cue rule).

Lock, `class_61::lock` two-argument form (`SL/type31.558` item 9 `+0x0025`):

| Offset | Width | Value | Status |
|---|---|---|---|
| 0 | w | 4 | CONFIRMED |
| 2 | w | the player object's SID (global 125) | CONFIRMED |
| 4 | w | the score object's SID | CONFIRMED |

Example: `04 00 23 01 45 01` (player `0x0123`, score object `0x0145`).

The Ack goes to the player object, and `class_84::handleMsg` (`SL/type31.561` item 27 `+0x042B`..`+0x0490`)
reads **the word at byte 5** of an Ack with `whichCmd` 4, finds the networked object with that SID
(global 46), clears its script and sends a second 27/13 for it (`b 27, b 13, w 0, w that SID, w number
- 100`). That 27/13, its reply and the 27/1 after it are the second read, and the reply of that read
is what posts (section 1.13). The lock Ack the score object needs is therefore:

| Offset | Width | Value | Status |
|---|---|---|---|
| 0 | b | 0 (Ack) | CONFIRMED |
| 1 | b | 0 | unread |
| 2 | w | the player object's SID (the lock's word 2) | CONFIRMED |
| 4 | b | 4 (`whichCmd`) | CONFIRMED |
| 5 | w | the locked SID (the lock's word 4) | CONFIRMED (`wordAt: 5`, `+0x0448`) |

Example: `00 00 23 01 04 45 01`. `innkeeperd`'s lock Ack today ends at byte 5 with `whichSub` 0
(`00 00 23 01 04 00`): the word at 5 is then 0 plus a byte past the end, no object matches, and
`script:` is sent to no object (INFERRED: an LSCI "not an object" error). The invitation scripts lock
with `fromSID` 0, so their Ack goes to the game object and never reaches this code.

A Nak 4 (lock held elsewhere) also goes to the player object, which looks the SID up and does nothing
with it (`+0x0497`..`+0x04C5`); the score object keeps "Checking Top Scores..." up and the retry
locks again 5 s later (INFERRED). The score object's own Nak-4 branch (`ScoreObj::handleMsg
+0x0628`) is reached only by a Nak addressed to its SID, which no lock produces.

### 1.11 Live check (2026-10-25)

`tools/verify_lifted.sh` with `private/lifted/scenarios/inn_sierraland.txt` on drive
`work/srv-games-c`, 472 s. With the stock `innkeeperd` the room sent `1b 0d 00 00 00 00 0a 00` and
`... 0b 00`, got empty directories and drew no table. With a local, uncommitted change that answered
keys 10 to 13 with one record (`total` 1, `location` = key) and the table of section 1.9 (author "INN",
names Alpha to Juliet), the room sent, for the Red Baron, 3-D Golf, MiniGolf and PaintBall II buttons:

```
1b 0d 00 00 00 00 0a 00     then  1b 01 0a 00 00 00 0a 00 01 00
1b 0d 00 00 00 00 0d 00     then  1b 01 0d 00 00 00 0d 00 01 00
1b 0d 00 00 00 00 0b 00     then  1b 01 0b 00 00 00 0b 00 01 00
1b 0d 00 00 00 00 0c 00     then  1b 01 0c 00 00 00 0c 00 01 00
```

and drew "TOP 10 SCORES!" with ranks 1 to 10, the ten names and the ten scores in the order sent.
This confirms sections 1.6 to 1.9 for the room. The score object's path (sections 1.5, 1.10, 1.12 to
1.14) was not run live: it needs a finished MiniGolf round or a game result.

### 1.12 What the score object does with the table

`ScoreObj::handleMsg` whichCmd 1 (`+0x0051`..`+0x0309`), CONFIRMED unless marked:

1. Parse lines 0 to 9 as in section 1.9 into 11-entry arrays; entry 10 is the player's score and the
   player's name (`+0x008C`..`+0x00B5`).
2. Low-is-best keys (11, 13): every score 0 becomes `0x7FFF` (`+0x0133`..`+0x0160`).
3. Bubble sort of the 11 entries: high-is-best swaps when the upper is lower, low-is-best when the upper
   is higher (`+0x0162`..`+0x0234`). Equal scores do not swap, so a new score ties below an old one.
4. If entry 10 still has the player's name and score, the score did not place: the object unlocks its
   SID (`06 00 23 01 45 01`), sets `signal` 0 and ends (`+0x0237`..`+0x02BF`). No post.
5. Else, first read (`signal` 1): lock, section 1.10. Second read (`signal` 2): "Updating Top
   Scores...", low-is-best `0x7FFF` back to 0, and post entries 0 to 9 (`+0x030C`..`+0x0402`).

The client does not remove a second line with the player's name: a player can hold several places.

### 1.13 27/0: post of the table

Two senders: after the second read (`+0x03DC`..`+0x0402`), and at once after a directory reply with
`total` 0 (no read, no lock: `+0x048A`..`+0x05A7`), which writes the player's score as line 0 and
nine lines `0 INN` (`+0x04BB`..`+0x04E7`).

| Offset | Width | Value | Status |
|---|---|---|---|
| 0 | b | 27 | CONFIRMED |
| 1 | b | 0 | CONFIRMED |
| 2 | w | record 0 `location` from the last directory reply (whatever bytes 11 and 12 held) | CONFIRMED |
| 4 | w | the score object's SID | CONFIRMED |
| 6 | w | key | CONFIRMED |
| 8 | | `name|` and 10 lines of section 1.9, then a NUL; not padded | CONFIRMED (`"%s|%6d %-10s\n"` for line 0, item 104; the `a` length is the text length + 1) |

The name is the player's (global 125 `name`), `%-10s`, so a name longer than 10 characters makes a
longer line, which every later read misparses. Example, MiniGolf, first post of player "Sunny" with 41
strokes:

```
1b 00 0b 00 45 01 0b 00
53 75 6e 6e 79 7c                                      "Sunny|"
20 20 20 20 34 31 20 53 75 6e 6e 79 20 20 20 20 20 0a   "    41 Sunny     \n"
20 20 20 20 20 30 20 49 4e 4e 20 20 20 20 20 20 20 0a   "     0 INN       \n"   (x 9)
00
```

195 bytes. The score object then waits for an **Ack 27/0 to its SID** (`+0x05C3`..`+0x060F`): it
tests only `whichSub` (byte 5) for 0, closes the status box, sets `signal` 0, unlocks and disposes.

| Offset | Width | Value | Status |
|---|---|---|---|
| 0 | b | 0 | CONFIRMED |
| 1 | b | 0 | unread |
| 2 | w | the score object's SID | CONFIRMED |
| 4 | b | 27 | not tested |
| 5 | b | 0 | CONFIRMED (`class_64::init`: `whichSub` is byte 5) |

Example: `00 00 45 01 1b 00`. While `signal` is non-zero, `ScoreObj::doit` calls `SL/script.010` export 6
every cycle (`+0x000A`..`+0x0010`), which turns off input and control and shows the wait cursor; so
**without the Ack the player is left in the waiting room with no input**. That is today's behaviour
for an empty board, since `innkeeperd` drops the post and sends nothing (CONFIRMED by code; that the
networked-object list, global 46, gets a `doit` each cycle is INFERRED). A Nak to the score object
(any `whichCmd` but 4, for example Nak 27 "BBS Server offline") also ends it cleanly
(`+0x0642`..`+0x0674`): status box closed, input back, unlock, dispose.

### 1.14 Unlock (6) and leaveNet (9)

Every end path sends `class_61::unlock` with the same layout as the lock (`06 00 23 01 45 01`), also
when it never locked, and the object's dispose sends leaveNet for its SID (`Obj::dispose +0x000D`,
`09 00 45 01 00 00`). Neither waits for a reply. CONFIRMED.

### 1.15 27/6: Clear Scores

`ClearScoreMI::doit` (`SL/vocab.718` item 60) asks "Delete all of the top score tables on this host?"
and on YES sends four erases of message 1, one per key, from the game object:

| Offset | Width | Value | Status |
|---|---|---|---|
| 0 | b | 27 | CONFIRMED |
| 1 | b | 6 | CONFIRMED |
| 2 | w | 0 | CONFIRMED |
| 4 | w | the game object's SID (global 2) | CONFIRMED |
| 6 | w | key: 10, 11, 12, 13 in that order | CONFIRMED |
| 8 | w | 1 | CONFIRMED |
| 10 | b | 0 | CONFIRMED |

Example: `1b 06 00 00 31 01 0a 00 01 00 00`. No reply is read. The menu item exists only when the
room's menu (global 165) exists and global 82 is set; that global 82 is the sysop level is INFERRED.

### 1.16 What this means for the host

These follow from the sections above; each is a choice for `srv-scores`, not a fact about the original.

1. Key every score request by **word 6** (10 to 13). Word 2 is only an echo of the record, and 27/6
   sends 0 there. Write the key into record `location` so both words agree.
2. Answer 27/13 for a key with **one record** always, `total` 0 or 1, and answer every 27/1 of
   message 1 (an empty table when nothing is stored is safer than a Nak, which the room ignores).
3. Keep **one message per board**: a post replaces message 1. The client reads only message 1.
4. **Ack every 27/0** to a score board (`00 00 sid 1b 00`), or the poster has no input.
5. Put the **locked SID at bytes 5 and 6 of the lock Ack** (`00 00 to 04 lo hi`). The invitation
   locks do not read those bytes, so the longer Ack is safe for them.
6. Normalise the stored table: 10 lines of exactly 18 bytes, names cut to 10 characters, sorted by the
   key's direction, so one long name cannot shift every later read. A merge of the posted table with
   the stored one would also repair two players posting from the same old table, since the lock is
   only per score object unless all objects of a key share a SID.
7. The 18-byte header must hold no `0x7C` byte before the text: the client searches the whole
   message for `|`. A `toSID` or SID word with a `0x7C` byte (for example `0x017C`) would make it parse
   from the wrong place.
8. Treat 27/6 on keys 10 to 13 as privileged only; the poster of the last table is its "author".

### 1.17 For comparison: the 2.4.46 Arena (not this client)

Read from the INN Barn media (never committed). The Arena keeps the same `ScoreObj` and room code,
but the board and direction come from the game table the host sends (`GAMEINFO.CFG`, 36/8): the
Score column `s` gives `scoreType` 1 and board `s` when positive, `scoreType` -1 and board `-s` when
negative, none when 0 (Arena script 1, `export_11 +0x002B`..`+0x0059`). The score object is numbered
board + 100 and gets `state` 1 for `scoreType` -1 (script 1, `export_4 +0x001E`..`+0x0058`), so the
requests carry the board at word 6 exactly as here. Its Top Scores room (script 8904, stored as the Arena's
vocab 712) builds one button per game with a non-zero `scoreType` and handles a Nak ("The
Bulletin Board server is down"). `innkeeperd`'s Arena table uses boards 910, 911 and 912
([games.md](games.md) section 10). CONFIRMED layouts; the 2.4.46 SierraLand was not compared.

### 1.18 Not answered

- Whether the original host gave one SID per key to every player's score object (a true board lock)
  and what its directory record held at `location`.
- What the original host sent for the lock Ack beyond byte 6, and whether it acknowledged 27/0 with
  anything after `whichSub`.
- The meaning of global 82 and global 263 (the PaintBall button picks music from it), and the exact
  `Str compare` result the "did not place" test relies on (step 4 of section 1.12 assumes true when equal).

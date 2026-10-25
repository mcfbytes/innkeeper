# Game services: titles, service objects, hand-off and score boards

The contract between `innkeeperd` and every game the INN clients play: which titles the host serves
(section 1), the well-known service objects the DOS programs ask for (section 2), how a player goes from a
land to a DOS program and back (section 3), what a score board stores (section 4), and what the operator
sees (section 5). The code is `crates/innkeeper-world/src/titles.rs` (the registry) and `services.rs` (the
dispatch); the group and invitation model is [games.md](games.md), the byte layouts are
[messages.md](../protocol/messages.md). Labels as in [games.md](games.md): **CONFIRMED** (client code, client
data or a live run agree; the source is named) and **INFERRED** (a host choice or a reading the clients do not
pin down). Every INFERRED value has a named constant; a constant marked *(to add: package)* does not exist
yet and is the name that package gives it in `innkeeper-world/src/assumptions.rs`. A value nobody knows yet is
marked *to be found by package*, after the package of [fanout-plan.md](fanout-plan.md) section 5 that owns it.

Sources read for this document (2026-10-25): `GameInfo` instances in `SL/script.001`, `hub/script.001` and
`LL/script.001` of the Feb-94 client (`tools/lsci_disasm.py --script 1`); `GAME.CFG`, `TSN.PRG` and `LAND.CFG`
of the Feb-94 install (`INN-Decomp/work/srv-games-c`); `TSN.PRG`, `LAND.CFG` and `GAMEINFO.CFG` of the 2.4.46
install; the censuses under [int14h-census](../protocol/int14h-census/); the live run of section 3.2.

## 1. The title registry

`titles::TITLES` has one row per `TitleId`; a test (`every_title_variant_has_exactly_one_row`) fails to
compile until a new variant has its row. Lookups: `by_game_byte` (the land-type byte of a game group),
`by_dos_land_type` (the land type of a DOS program's objects and of its 41/2) and `by_name` (any case, `_` for
a space). `game_info.rs` writes the Arena's `GAMEINFO.CFG` rows from it, and the game-group log lines name
the title through `GameType::name`.

### 1.1 Columns

| Column | Meaning | Source |
|---|---|---|
| Offered | land type and place (land number) where a player starts it; every stock land has one place, land number 1 | `GameInfo` of that land, `LAND.CFG`; CONFIRMED |
| Kind | `LsciScript` (a script of the land's own `LSCITV`), `DosProgram` (the land chains to a `TSN.PRG` block), `Rpg` (a land of its own), `PmsServer` (a land script talking to a PMS server with command 54) | CONFIRMED per row |
| Game byte | the land-type byte of the own group (joinNet kind 2 or 4, parameter `0x8000` or more) the land makes for a game: `GameInfo` property `tID` (`selector_278` in SierraLand) | CONFIRMED as data; CONFIRMED live as the group's byte for 23, 26, 28 (2026-10-25), 101 (2026-10-15) and 102 (2026-10-25); for the rest INFERRED (`UNSEEN_GAME_BYTES_NAME_THEIR_TITLE_ASSUMED`) |
| Players | `GameInfo` `minimum` and `maximum`; for Football the 2.4.46 `GAMEINFO.CFG` row | CONFIRMED as data |
| Service | the service object a DOS program finds with 41/2 (section 2) | CONFIRMED per census |
| Score board | which board a result goes to and whether high or low wins (section 4) | filled in by `srv-scores` |
| Chain | the `TSN.PRG` block the land runs the program with | CONFIRMED (`TSN.PRG`; `RedBaron` and `Golf` are strings in `SL/script.120`) |

### 1.2 Rows

| Title (`TitleId`) | Offered | Kind | Script | Game byte | Players | Service | Score board | Chain |
|---|---|---|---|---|---|---|---|---|
| MiniGolf (`MiniGolf`) | SierraLand (2), 1 | LSCI | 779 | 23 (seen) | 1 to 2 | none | SierraLand Top Scores (`SL/script.120` names it); order to be found by `srv-scores-research` | none |
| PaintBall (`PaintBall`) | SierraLand, 1 | LSCI | 780 | 24 | 2 to 6 | none | SierraLand Top Scores (same string); order to be found by `srv-scores-research` | none |
| Boogers (`Boogers`) | SierraLand, 1 | LSCI | 790 | 25 | 2 to 4 | none | to be found by `srv-scores-research` | none |
| Stratego (`Stratego`) | SierraLand, 1 | LSCI | 800 | 26 (seen) | 1 to 2 | none | to be found by `srv-scores-research` | none |
| SneakATac (`SneakATac`) | SierraLand, 1 | LSCI | 775 | 27 | 2 to 2 | none | to be found by `srv-scores-research` | none |
| Graffiti (`Graffiti`) | SierraLand, 1 | LSCI | 530 | 28 (seen) | 1 to 4 | none | to be found by `srv-scores-research` | none |
| RocketQuiz (`RocketQuiz`) | SierraLand, 1 | LSCI | 555 | 29 | 2 to 3 | none | to be found by `srv-scores-research` | none |
| NTN Trivia (`NtnTrivia`) | Clubhouse (1, 2.4.46 rooms 16 to 18) and SierraLand (rooms 7 to 9), 1 | PMS server 4 | 8920 | 31 | 1 to 12 | PMS server 4 ([trivia.md](trivia.md)) | the show scores its rounds itself; a kept board to be found by `srv-scores-research` | none |
| Red Baron (`RedBaron`) | SierraLand, 1, and the Arena (5), 1 | DOS | 610, 615 | 101 (seen) | 2 to 4 | Red Baron, `0x65` | Arena 910, high (`GAMEINFO.CFG`), SierraLand Top Scores | `RedBaron` |
| 3-D Golf (`Golf3d`) | SierraLand, 1, and the Arena, 1 | DOS | 605 | 102 (seen) | 1 to 4 | none (GOLF's 41 builder is dead) | Arena 911, low; SierraLand Top Scores | `Golf` |
| Football (`Football`) | the Arena, 1 (2.4.46 only) | DOS | none in Feb-94 | to be found (no package of this wave) | 2 to 2 | to be found (no package of this wave) | Arena 912, low | `football` (2.4.46; also `halftime`, `ball2`, `endtime`) |
| Checkers (`Checkers`) | Clubhouse (1), 1 | LSCI | 400 | 4 | 2 to 2 | none | to be found by `srv-scores-research` | none |
| Chess (`Chess`) | Clubhouse, 1 | LSCI | 405, 406 | 5, 16 | 2 to 2 | none | same | none |
| BackGammon (`BackGammon`) | Clubhouse, 1 | LSCI | 410 | 3 | 2 to 2 | none | same | none |
| FlipFlop (`FlipFlop`) | Clubhouse, 1 | LSCI | 415 | 8 | 2 to 2 | none | same | none |
| Go (`Go`) | Clubhouse, 1 | LSCI | 420 | 9 | 2 to 2 | none | same | none |
| Spades (`Spades`) | Clubhouse, 1 | LSCI | 515 | 18 | 4 to 4 | none | same | none |
| Euchre (`Euchre`) | Clubhouse, 1 | LSCI | 520 | 19 | 4 to 4 | none | same | none |
| Bridge (`Bridge`) | Clubhouse, 1 | LSCI | 500, 503 | 14, 15 | 4 to 4 | none | same | none |
| Cribbage (`Cribbage`) | Clubhouse, 1 | LSCI | 505 | 6 | 2 to 2 | none | same | none |
| Hearts (`Hearts`) | Clubhouse, 1 | LSCI | 510, 511 | 7, 17 | 3 to 4 | none | same | none |
| BlackJack (`BlackJack`) | CasinoLand (3), 1 | LSCI | 911 | 32 | 1 to 4 | none | to be found by `srv-scores-research`; the bankroll (40/2, 40/3) is stored already | none |
| Slots (`Slots`) | CasinoLand, 1 | LSCI | 1030 | 33 | 1 to 7 | none | same | none |
| Roulette (`Roulette`) | CasinoLand, 1 | LSCI | 1060 | 34 | 1 to 4 | none | same | none |
| Roulette 00 (`Roulette00`) | CasinoLand, 1 | LSCI | 1061 | 35 | 1 to 4 | none | same | none |
| Poker (`Poker`) | CasinoLand, 1 | LSCI | 1040 | 36 | 2 to 5 | none | same | none |
| Lefty's Bar (`LeftysBar`) | CasinoLand, 1 | LSCI | 1100 | 37 | 2 to 6 | none | same | none |
| Shoppers Advantage (`ShoppersAdvantage`) | the map (Clubhouse program, `gotoCUCmall`) | DOS, PMS server 5 | none | none | n/a | PMS server 5 ([shopadv.md](../protocol/int14h-census/shopadv.md)) | none | `ShopAdv` |
| Yserbius (`Yserbius`) | its own land (4), 1 | RPG | none | none (own groups of `0x8B` are not games) | n/a | RPG, `0x8B` | none | `Yserbius` (`CGENN`), `Yserba` (`DARKSTRT`) |
| Cawdor (`Cawdor`) | its own land (6), 1 (2.4.46) | RPG | none | none | n/a | RPG, `0x8B` | none | `Cawdor`, `Cawda` (2.4.46) |
| Twinion (`Twinion`) | its own land (8), 1 | RPG | none | none | n/a | RPG, `0x8B` | none | `Twinion` (`TWGENN`), `Twina` (`FATES`) |

Notes on the rows:

1. Two script numbers for one title (Chess, Bridge, Hearts, Red Baron) are two catalogue entries of the same
   game; in the hub the second Chess, Bridge and Hearts entries have `loop` 4. INN-Decomp's `docs/redbaron.md`
   reads Red Baron's two as one for client version 3 or later and one for older clients; which client shows
   which entry is to be found (no package of this wave). The registry keeps both numbers and both game bytes
   on one row.
2. `GameInfo` rows that are places, not titles, stay out of the registry: SierraLand's `WaitingGame` (tID 2),
   `Chat Room` (21), `Crucial INN Host` (22), `Best 10 Game Scores` (script 8910, tID 30), `Watching` (0) and
   `Unknown` (1); the hub's `Chat` (10 and 21), `Bulletin Board`, `Hint Room`, `Mail Room`, `Conference Room`.
   CONFIRMED as data.
3. The Feb-94 `TSN.PRG` also has `LaunchGolf` (`LAUNCH\GOLF.COM`), the program-launch harness of fan-out item
   C11, not a title. The 2.4.46 `TSN.PRG` has `Torpedo` (`torp.exe`) with no `GAMEINFO.CFG` row and no
   `GameInfo`: not served.
4. The Arena rows (`ArenaSlot`) are the 2.4.46 `GAMEINFO.CFG` rows: land 5, relative position 0, 1, 2, queue 0,
   invite 910, 911, 912, score 910, -911, -912, add 0; the name, the players and the chain (101, 102, 104,
   the DOS land type) come from the title. CONFIRMED (the file on the 2.4.46 media is the same five columns of
   numbers); the `GAMEINFO.CFG` bytes this host sends are unchanged (`game_info.rs` test).
5. SierraLand Top Scores: `SL/script.120` holds "Select this button to view Top Scores from Red Baron,
   PaintBall, INN 3-D Golf, and MiniGolf", "Checking Top Scores...", "Updating Top Scores..." and the objects
   `golfScoreObj`, `wpsGolfScoreObj`. CONFIRMED strings; the read and post messages are `srv-scores-research`'s.

## 2. Service objects

`services.rs` is the one place that answers the commands addressed to a service object; `player.rs` hands
it commands 41, 17 and 50 and adds only the "absorbed" log lines. The bodies stay in `red_baron.rs` and
`rpg.rs`.

| `ServiceObject` | Land type of the 41/2 | SID returned | Titles | Status |
|---|---|---|---|---|
| `Rpg` | `0x8B` (`RPG_OBJECT_LAND_TYPE`) | `0x008B` (`RPG_SERVICE_SID_ASSUMED`) | Yserbius, Cawdor, Twinion | lookup CONFIRMED (yserbius.md 4.3, cawdor.md 1); the SID INFERRED |
| `RedBaron` | 101 (`RED_BARON_LAND_TYPE`) | `0x0065` (`RED_BARON_SERVICE_SID_ASSUMED`) | Red Baron | lookup CONFIRMED live (games.md section 6); the SID INFERRED |

Dispatch rules, each a test in `services.rs`:

1. **41/2** (`ObjExists::Service`): `ServiceObject::by_land_type` picks the object; its own module answers
   with command 41 and the SID at word 6 (`rpg::locate_service`, `red_baron::locate_service`). Any other land
   type is not answered and is logged "decoded, no handler yet: ignored". GOLF's 41 builder is dead
   ([golf.md](../protocol/int14h-census/golf.md) 6.1), so 102 needs nothing. CONFIRMED for the clients' waits
   (RB fatal at 119.6 s, the RPG titles resend every 60 s); test `each_service_object_is_found_by_its_land_type_and_its_sid`.
2. **41/0** (`ObjExists::Name`) is a conference name check, not a service: it goes to `conference::locate`
   (test `a_name_check_goes_to_the_conferences`).
3. **17/x** (`ServicePropertyRequest`): `ServiceObject::by_sid(request.service)` picks the object. Red Baron
   answers sub 0, properties 9 (`0x0201`) and `0xF` (`RED_BARON_GAME_MINUTES_ASSUMED`), one `setInt` each with
   the property as its first pair. Twinion's 17/2 (properties 8 to 11, to `0x8B`) is not waited for and gets
   nothing; the RPG object answers no property. CONFIRMED waits (redbaron.md 6, twinion.md 4.1); test
   `only_the_red_baron_service_answers_a_property_read`.
4. **50** (`PlayStatus`, state 0 playing, 1 leaving): logged "play status noted", never answered; only Red
   Baron sends it live (GOLF's builder is dead). It names no title, only the user SID. CONFIRMED no reply is
   read (redbaron.md 10 item 11). Section 3.3 rule H4 uses state 1.
5. **48** (`Notice`, host to client): operator text from the portal and the stop countdown
   ([innkeeperd.md](innkeeperd.md) section 11), sent to logged-in lines only. Who shows it: the LSCI hub
   (`RoomZeroHandler`), GOLF once joined (`gcOnUnsolicited`), Red Baron only while a game runs, Shop as "INN
   Notice:". CONFIRMED (coverage.md row 48). A notice that arrives during a program switch is lost on the legacy
   link unless rule H1 holds it; the countdown repeats, so one lost notice is harmless (INFERRED).

A new service object is one `ServiceObject` variant, its land type and SID, one arm in each `match` of
`services.rs`, and the `service` column of its title.

## 3. The hand-off: land, DOS program, land

### 3.1 States of one connection

| State | Entered on (host event) | Status |
|---|---|---|
| In a land | joinNet of the land's game object (kind 129, the land's type) and of its waiting room (kind 5) | CONFIRMED (messages.md 5.4) |
| Inviting | a granted lock (`GameBook` invitation) | CONFIRMED (games.md 2) |
| Game formed | the leader's own-group joinNet (kind 2 or 4, parameter `0xFFFE` or `0xFFFD`), command 30 from every player | CONFIRMED (games.md 2, 6) |
| Leaving the land | the land's leaveNet of its game object (kind 129): the last message `LSCITV` sends (`hub/script.101` export 1 step 2, then `TSN(9)`, `TSN(12)`, `TSN(8)`, `TSN(14)` and quit) | CONFIRMED code (messages.md 5.1) and live (section 3.2) |
| In a DOS program | the program's first message: GOLF's joinNet kind 1, type `0x66`; Red Baron's 41/2 type 101; Shop's JOIN 54/25 | CONFIRMED (golf.md 7.2, redbaron.md 6, shopadv.md 9) |
| Leaving the program | its last message: GOLF's leaveNet of its own object (after GrpDel); Red Baron's 50/1; Shop's EXIT 54/26 answered by 54/19 kind `0x1A` | GOLF and Shop CONFIRMED; Red Baron INFERRED (rule H4) |
| Back in the land | the land's leaveNet (6-byte form, `09 00 sid 00 00`) of the SID at block `+4` (the game group), then 47/1 and the land joinNets | CONFIRMED live (section 3.2) and code (messages.md 5.2 step 2) |

A Login is sent again only when the land the player returns to is on another host number (messages.md 5.2
step 3, after SwitchHost). On one host no Login follows a game: CONFIRMED live (section 3.2).

### 3.2 Captures

3-D Golf from SierraLand, one player, 2026-10-25: the stock Feb-94 client under `dosrt` (INN-Decomp public build,
`private/lifted/scenarios/inn_sierraland.txt`, drive `work/srv-games-c`) against this host. SIDs as seen:
land player object 260, waiting room 261, land game object 259, invitation object 269.

| # | Program | Bytes | Host |
|---|---|---|---|
| 1 | SL `LSCITV` | `07 00 00 00 5c 22 01 02 ff ff 19 00` joinNet kind 1 (the invitation object) | `ObjID` 269 |
| 2 | | `04 00 0d 01 04 01` lock `[260]`; multicast 57 | Ack 4 |
| 3 | | `07 00 00 00 64 24 02 66 fe ff 40 00` joinNet kind 2, type `0x66`, parameter `0xFFFE`, size 64 | `ObjID` 270; "game group formed ... game=102 title=3-D Golf" |
| 4 | | unlock, `1e 0e 01` register 270, `09 00 0d 01 00 00` leaveNet 269 | none |
| 5 | | `0d 00 04 01 ff ff 09 00 5d 02` setInt on 260, property 9 = 605 (the game's script) | relayed to replicas |
| 6 | | `09 00 03 01 00 00` leaveNet 259, the land's game object | none: **leaving the land** |
| 7 | `GOLF.EXE` | `07 00 00 00 00 00 01 66 ff ff 01 00` joinNet kind 1, type `0x66`, size 1 | `ObjID` 271 |
| 8 | | `0a 00 0e 01 0f 01` GrpJoin 270, 271; GrpMem; Send `CA` to 270 | echo, member list |
| 9 | | `0b 00 0e 01 0f 01` GrpDel; `09 00 0f 01` leaveNet 271 (4-byte form) | none: **leaving the program** |
| 10 | SL `LSCITV` | `09 00 0e 01 00 00` leaveNet 270 | "game group closed ... title=3-D Golf" |
| 11 | | `2f 01 00 00 00 00` 47/1; joinNet kind 5 type 2 land 1 size 128 | occupancy; `ObjID` 261 again (a second reference) |

260 stayed alive across the switch, in the waiting room, with property 9 = 605: the other players see where
the player went. CONFIRMED live.

Red Baron (games.md sections 6 and 8, `tests/games.rs` `red_baron_starts_up_joins_the_queue_and_trades_aircraft_state`):
the queue's joinNet `07 00 00 00 00 23 04 65 fd ff 04 00` (kind 4, type 101, parameter `0xFFFD`, size 4), then
in `RB.EXE` the first message `29 02 00 00 {user} 65 00 00` (41/2 type 101), then 17/0, 36/2, joinNet kind 3
type 101. CONFIRMED live 2026-10-15 and 2026-10-16; the return to the land was not run (`srv-rb-return`).

Shoppers Advantage ([shopadv.md](../protocol/int14h-census/shopadv.md) 9): `54/27` from the map, the host's
`54/19` (`MALL_LAUNCHES_SHOP_ASSUMED`), `LSCI` exits to `ShopAdv`, Shop's first message is JOIN `54/25`. Whether
the map sends a leaveNet of its game object between `54/19` and its exit is to be found by `srv-shopadv`.

### 3.3 Rules

Rules H1 to H6 decide the open C14 trigger. They are for `srv-handoff` to build; each names the test it adds
(`crates/innkeeper-world/tests/game_handoff.rs` unless another file is named).

1. **H1, the quiet window on the legacy link.** `Session::begin_program_switch` is armed by the connection
   after it has queued the replies to a message that ends a program, as the world reports it: H2 to H5. The
   window ends at the next program's first DATA frame or at `PROGRAM_SWITCH_QUIET_MAX_ASSUMED` (10 s), as
   built. Arming again while armed only extends the deadline (`hold_until_heard` takes the later one), so two
   markers for one exit are harmless. Test (innkeeperd): `a_land_exit_holds_host_frames_until_the_game_speaks`.
2. **H2, leaving a land.** A leaveNet of an object of kind 129 (the land's game object) that the connection
   holds ends the land program. CONFIRMED (section 3.1). Test: `leaving_the_land_game_object_ends_the_land_program`.
3. **H3, leaving GOLF and any DOS program with objects.** A leaveNet of the connection's last object whose land
   type is a title's `dos_land_type` (`titles::by_dos_land_type`) ends that program. CONFIRMED for GOLF
   (golf.md 4.3 and 9 item 9: GrpDel, then leaveNet, then exit). Red Baron's leaveNet arms too, its 50/1
   ends that window as a DATA frame, and H4 arms it again. Test:
   `golf_leaving_its_object_ends_the_program`.
4. **H4, Red Baron's 50/1.** A `PlayStatus` with `playing` false ends Red Baron. INFERRED that it is RB's last
   message (redbaron.md 8.3: `leaveOnFatal`'s GrpDel and ObjFree, then the clean exit with 50/1);
   `RED_BARON_LEAVING_IS_ITS_LAST_MESSAGE_ASSUMED` *(to add: srv-handoff)*. Test:
   `red_baron_leaving_status_ends_the_program`.
5. **H5, the mall.** The host's own 54/19 ends a program: kind 0 to the map's `54/27` ends `LSCITV`, kind
   `0x1A` to Shop's `54/26` ends Shop. CONFIRMED that each side exits on it (shopadv.md 9 steps 0 and 5a);
   INFERRED that nothing is sent after it, `MALL_REPLY_ENDS_THE_PROGRAM_ASSUMED` *(to add: srv-handoff)*. Test:
   `the_mall_open_and_exit_replies_end_a_program`.
6. **H6, the INT 14h transport.** No quiet window: the server's queue waits for the next program's Receive.
   `SetNextProgram(name)` is stored per session as `next`, and at each H2 to H5 marker the session does what
   TSNEXEC does (int14h-api.md 9.1 step 4): `previous = current`, `current = next`, `next = none`.
   `GetPreviousProgram` returns `previous`, so a GOLF or RB whose executive forwards that call is not
   orphaned ("Parent failure"). CONFIRMED TSNEXEC behaviour; today the server answers NULL. Test (innkeeperd):
   `the_transport_returns_the_program_that_ran_before`.

Group lifetime and orphans:

7. **G1.** A game group lives while any connection holds a reference: its joinNet, or command 30 (games.md
   rule 3). The land's reference survives the switch to the DOS program (section 3.2, rows 4 to 9). CONFIRMED
   live. Test: `a_game_group_outlives_the_switch_while_a_land_holds_it`.
8. **G2.** The returning land frees the group with its leaveNet of block `+4`; the group ends when the last
   connection lets go, and the members still in it hear `GrpDel` and `ObjFree`. CONFIRMED live (one player).
   Test: `the_last_land_to_return_closes_the_game_group`.
9. **G3, orphans.** A DOS program that dies without its own GrpDel and leaveNet (a fatal error with the link
   lost) leaves its object held. The returning land's leaveNet of the group (G2) also frees every object of a
   DOS title's land type the connection still holds, with `GrpDel`/`ObjFree` to the others.
   `RETURN_FREES_DOS_OBJECTS_ASSUMED` *(to add: srv-handoff)*. Test:
   `a_return_to_the_land_frees_what_the_dos_program_left`.
10. **G4, the land's own objects.** The land's player object and its waiting-room reference are kept across
    the switch (section 3.2). The land's second joinNet of its waiting room adds a reference that hang-up
    releases. Whether the returning land makes a new player object (so the old one is an orphan to free on
    return) is to be found by `srv-handoff`; until then nothing is freed. Test:
    `a_return_to_the_land_keeps_one_waiting_room_member`.
11. **G5, Login again.** A Login on a logged-in connection releases everything it holds, as a hang-up does
    (`objects.disconnect`, existing; [innkeeperd.md](innkeeperd.md) section 5), including game-group
    references, invitations and a trivia seat. It happens after a SwitchHost to another host number, never on
    the return from a game on this host. CONFIRMED. Test (exists): `a_second_login_releases_what_the_first_one_held`.
12. **G6.** A hang-up frees everything (existing, games.md rule 4). Test (exists in `tests/games.rs`).

## 4. The score-board contract

`srv-scores` builds it; the layouts of 27/1 (read) and 27/0 (post) on the score boards, and what each DOS
program leaves in the shared block, are `srv-scores-research`'s. The contract here is what the store keeps.

### 4.1 Record

| Field | Type | Meaning |
|---|---|---|
| `account` | `AccountId` | the player's account; never a persona |
| `title` | `TitleId` | stored as the title's `name` from the registry, which never changes for a row |
| `score` | `i32` | the result the land posts, as the client computes it |
| `date` | `i64` milliseconds since 1970, UTC | the host's clock at the post (`Clock::millis`) |

The persona a board shows is the account's stored persona when the board is read, not a copy kept with the
score, so a rename shows at once and a deleted account's rows go with it.

### 4.2 `Store` methods

```rust
fn post_score(&self, score: ScoreRecord) -> Result<ScorePosted, StoreError>;
fn top_scores(&self, title: TitleId, order: ScoreOrder, limit: usize) -> Result<Vec<ScoreRecord>, StoreError>;
fn scores_of(&self, account: AccountId) -> Result<Vec<ScoreRecord>, StoreError>;
fn delete_scores_of(&self, account: AccountId) -> Result<usize, StoreError>;
```

`ScorePosted` says whether the post entered the top `limit` (the client's "Updating Top Scores..." needs no
more). `scores_of` is the export, `delete_scores_of` runs with the account's hard purge. Both
`MemoryStore` and `SqliteStore` implement them, in the store's conformance suite.

### 4.3 Rules

| Rule | Value | Status |
|---|---|---|
| Order per title | `ScoreOrder::High` or `Low` on the title's row: Red Baron high, 3-D Golf and Football low (the sign of the `GAMEINFO.CFG` score column); MiniGolf and PaintBall to be found by `srv-scores-research` | CONFIRMED data, INFERRED meaning (games.md 10) |
| Ties | the earlier post ranks first | INFERRED, `SCORE_TIES_KEEP_THE_EARLIER_ASSUMED` *(to add: srv-scores)* |
| Top N | 10 rows per board ("Best 10 Game Scores", script 8910) | CONFIRMED name, INFERRED count: `TOP_SCORES_SHOWN_ASSUMED` *(to add: srv-scores)* |
| One account | keeps its best score per title, not every round | INFERRED, `BEST_SCORE_PER_ACCOUNT_ASSUMED` *(to add: srv-scores)* |
| No result | GOLF's word `0x7FFF` (not a full stroke-play round) is not posted | CONFIRMED (golf.md 5.2) |
| Quota | at most `SCORES_PER_DAY_ASSUMED` *(to add: srv-scores)* posts per account and title in 24 hours, like the board posts' `POSTS_PER_DAY_ASSUMED` | INFERRED |
| Privacy | under `--public` a post is logged with the account number, the title and nothing else; no persona, no score text ([privacy.md](privacy.md) section 2). Scores are kept while the account lives and are in its export; the admin rows are `srv-scores-admin`'s | operator choice |

## 5. Operator visibility

| Line | Fields | Under `--public` | Status |
|---|---|---|---|
| `game group formed` (start) | `group`, `game` (the byte), `title` (from the registry, absent for an unknown byte), `leader` connection, `invited` | the same: no names in it; the player span names the account number | built (`objects/games.rs`) |
| `game group closed` (end) | `group`, `game`, `title` | the same | built |
| end, still to add | `players` (distinct connections that held the group), `duration_s` (from the group's start, the world clock) | the same: counts only | to add: `srv-handoff`, which owns the group's lifetime |

One metrics counter: **`game_sessions`**, game groups formed, sent like `logins` (growth since the last push,
no dimensions, so nothing in it names a title or a player; [innkeeperd.md](innkeeperd.md) section 10.2,
[privacy.md](privacy.md) section 7a). To add with `srv-handoff`.

## 6. Open

- Football's game byte, service and the meaning of its four `TSN.PRG` blocks; Torpedo (section 1, note 3).
- Whether the second catalogue entry of Chess, Bridge, Hearts and Red Baron is ever offered by a Feb-94 client.
- Red Baron's exit order (rule H4) and its return to SierraLand: `srv-rb-return`.
- Whether the map leaves its game object before Shop starts: `srv-shopadv`.
- The RPG titles' program switches (CGENN to DARKSTRT, TWGENN to FATES and back) are not covered by H2 to H5.

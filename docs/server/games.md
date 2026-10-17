# Game groups and invitations

How `innkeeperd` lets two or more INN clients start a multiplayer game together: SierraLand's own games
(Stratego, Graffiti, MiniGolf, NTN Trivia, PaintBall, RocketQuiz, SneakATac and the hub's board and card
games) and Red Baron. MiniGolf is SierraLand's own LSCI game (`SL/script.779`); 3-D Golf is the DOS program
`GOLF.EXE` ([golf.md](../protocol/int14h-census/golf.md)). The code is `crates/innkeeper-world/src/objects/games.rs` (the model),
`objects.rs` (groups, locks, `register`), `presence.rs` (waiting rooms) and `red_baron.rs` (the DOS game's
service); the byte layouts are in [messages.md](../protocol/messages.md) sections 3.2 and 8. Labels as in
[yserbius.md](../protocol/yserbius.md): **CONFIRMED** (client code and a live two-client run agree),
**INFERRED** (a host choice the clients do not pin down; each is a named constant in
`assumptions.rs`).

## 1. What the host does and does not decide

The host never starts a game. The invitation, the answers, the start and the game itself are scripts and
messages between the clients (`SL/script.905` to `909`, `hub/script.120` and `121`, `SL/script.800`
Stratego and the rest); the host only offers five things, and each one is generic:

| Host service | Message | Used for |
|---|---|---|
| a lock table | lock (4) and unlock (6) | one invitation at a time per player: the inviter locks its own player SID and every guest's |
| objects with a SID | joinNet (7), leaveNet (9) | the invitation object each side makes (kind 1, size 19 or 25), the game group (kind 2 or 4, own group) |
| groups | GrpJoin (10), GrpDel (11), GrpMem (12) | who is in the waiting room, who is in the game |
| a relay | Send (2), multicast (28) | the invitation (type 12), the answers (13, 14, 56, 57, 58, 61, 63, 999), the game's own messages |
| property replicas | setInt (13), getProp (32), GrpGetProp (31) | the waiting room's player list and each player's current room number (property 9) |

Everything else a game needs is the clients' business. The host's job is to keep those five services
consistent while players come and go, and the model of section 3 is what lets it do so.

## 2. The flow, as seen on the wire (CONFIRMED live, 2026-10-15)

Two stock clients in the SierraLand waiting room (land type 2, place 1), A inviting B to Stratego or
Graffiti. SIDs are examples; the bytes are in `crates/innkeeper-world/tests/games.rs`.

| Step | Who | Message | Host |
|---|---|---|---|
| 1 | A | joinNet kind 1, size 25: the invitation object (SID `ia`) | `ObjID` |
| 2 | A | **lock** `from ia`, words `[A, B]` (player SIDs) | `Ack` (4) to `ia`; or a Nak naming a word another invitation holds ("%s is already engaged in an invitation") |
| 3 | A | Send to B, from A: type **12**, words: game script (800 Stratego, 530 Graffiti), 1, `ia`, B, A, 7, 42 | relayed to B |
| 4 | B | joinNet kind 1, size 25: B's invitation object (`ib`); Send to `ia`, from B: type **56** `[ib]` | relayed to A |
| 5 | A | Send to `ib`: type **30** array (the response list), twice | relayed |
| 6 | B | accepts or declines: Send to `ia`, type **13** (accept) or **14** (decline, with text) | relayed |
| 7 | A | "Start Game": multicast type **57** to the guests who have not answered (an empty list when all did); joinNet kind 2, param `0xFFFE` (an own group), size 64 (Stratego) or 4 (Graffiti): the game group `g`; Send to `ib`: type **58** `[g, number of players]`; **unlock** `[A, B]` | `ObjID`, relays |
| 8 | A | command 30 ("register") `[g]`, GrpJoin `g, A`, GrpMem `g` | echo, `GrpMem [A]` |
| 9 | B | the same: register `[g]`, GrpJoin `g, B`, GrpMem `g`; leaves its invitation object | echo to B and A, `GrpMem [A, B]` |
| 10 | both | each waits until its group holds the announced number of players, frees its invitation object, sets property 9 of its player to the game's script number (setInt) and loads the game | relays |
| 11 | both | game traffic: Send to `g` from the player's SID (Graffiti type 55, Stratego 100 to 104, 6, 8, 29, 30) | relayed to every connection holding a member of `g` |

Decline: B's type 14 ends A's wait for B; A may start with the others or cancel (multicast 57 to the
guests, unlock). Cancel by A while B is deciding: type 57 closes B's dialog ("This invitation has been
cancelled").

Two points the live run settled:

1. **The newcomer to a waiting room must be told who is already there.** A client's waiting room learns
   of a member from the `GrpJoin` that names it. The host used to send that only to the members already
   in the room, so the second player never saw the first, and could not invite them. The host now sends
   the newcomer one `GrpJoin` per member already in the room, right after its own echo
   (`presence::tell_who_is_here`). Sending one `GrpMem` with the whole list instead also shows the names,
   but the replicas it makes are not good enough for the game group: with that variant the player who
   had joined the waiting room *second* stayed in "Waiting to start...." for ever in four runs out of
   four (the group's member count never matched). INFERRED which path the original host took; the
   `GrpJoin`-per-member form works in every run.
2. **A guest does not join the game group with joinNet.** It only sends command 30 with the group's SID,
   then GrpJoin and GrpMem. The host therefore treats command 30 as "this connection now holds the
   group" (section 4, rule 3), so the group outlives its creator for as long as one registered player is
   in it. INFERRED meaning; the clients never wait for an answer.

## 3. The model

`GameBook` (inside `ObjectStore`, so its rules run in the same step as the group and lock changes):

| Record | Fields | Created | Ends |
|---|---|---|---|
| `GameSession` | `group` SID, `game` (the land-type byte of the joinNet: Red Baron 101; Stratego and Graffiti sent 26 and 28 on the test server), `leader` connection (the one whose joinNet made the group), `capacity` (the joinNet size), `phase` (`Forming` until members of two connections are in, then `Gathered`), `registered` connections | an **own group** (kind 2, 4 or 5 with a parameter of `0x8000` or more) on any land type but the RPG titles' `0x8B` | the group is destroyed (every holder let go: `leaveNet` or hang-up) |
| `Invitation` | the inviting connection, the object it locked from, the lock words still held | a **lock** that is granted | its last word is unlocked, or its connection hangs up (`release_all`) |

An invitation that is open when its inviter creates a game group is attached to the session
(`GameSession::invitation`), so a log line can say who invited whom to what; the host does not use it to
admit or refuse anything. `ObjectStore::game_sessions()` and `ObjectStore::invitation_of(connection)`
expose both, for tests and for the operator's log. Nothing here is persisted.

## 4. Rules (each one is a test in `tests/games.rs` or `objects/tests.rs`)

1. **Locks** are all-or-none and per connection, as before ([objects.md](objects.md) section 8). A hang-up
   releases them, so a player who drops during an invitation does not stay "engaged".
2. **GrpJoin** into a game group goes to the joiner (echo) and to every other connection with a member,
   never to the joiner's own other members twice; a group refuses a join only when its *connections*
   already fill its capacity. The size of a game group counts players, not objects: Red Baron's queue is
   created for 4 players, yet each player has a land object and a game object in it.
3. **Command 30 (register)** on the SID of a live game group adds a reference for the connection, as a
   joinNet would, so the creator can free its own reference at the hand-off to the game program and the
   group stays. `leaveNet` and hang-up give the reference back; the last one destroys the group, and the
   members still in it hear `GrpDel` and `ObjFree`. Command 30 on anything else is only logged.
4. **A player who leaves or drops** leaves every group it is in: the others get `GrpDel` for each of its
   members, then `ObjFree` for each object that goes with it ([objects.md](objects.md) section 4.2).
5. **A Send to a game group** goes once to every connection that holds a member, except the sender's
   (`GROUP_SEND_SKIPS_SENDER_ASSUMED`), with the real sender in `fromSID`. A Send to one of the sender's own
   objects does reach it. A multicast is one such relay per listed recipient, so Red Baron's `{group, own
   SID}` list gives the other pilots the group copy and the sender the own-SID copy (section 6). MiniGolf
   replays its own shots when its copy comes back (section 5.1). The rule holds for every group, not only
   game groups; the RPG lands skip the sender for any target (`RPG_SENDS_ECHO_SENDER_ASSUMED`).
6. **Waiting rooms** tell a newcomer who is there (section 2, point 1). Land groups of the RPG titles are
   exempt: DARKSTRT and FATES tell their own roster with game messages and were verified without it.

## 5. SierraLand games

Nothing beyond sections 3 and 4. Verified live (section 8) against `innkeeperd` with two headless `dosrt` clients
(accounts 100001 and 100002, players Ann and Bob):

| Game | Result |
|---|---|
| Stratego | invitation delivered, accepted, both clients load Stratego (the coin toss dialog) |
| Graffiti | both clients load the paint room |
| MiniGolf | alone and with two players: the start, the turn barrier and several strokes each (section 5.1) |
| Red Baron | section 6 |

The others use the same invite rooms (`slInviteRoom` 906 for Stratego, MiniGolf, Graffiti, SneakATac,
Boogers, NTN Trivia and RocketQuiz; `pbInviteRoom` 907 PaintBall; `WPSinviteRoom` 909) and the same
group calls, so they need nothing else from the host; only the three above were exercised.

### 5.1 MiniGolf (`SL/script.779`)

`GameInfo` "MiniGolf" in `SL/script.001`: script number `0x30B` (779), land type `0x17` (23, the `game` of
the session), one or two players. A player alone still makes a game group (kind 2, own, size 2) and sends
to it; with two the group is made for 64. Every message is a `Send` to the game group (`global119`) from
the player's SID (`global125`), except the barrier answer below. `MiniGolf::handleMsg` queues each command 2
that is not chat (types 1 and 36) and `proc_161` works the queue; neither compares `fromSID` with the
player's own SID. CONFIRMED (bytecode; sizes and order from live captures, 2026-10-16).

| Type | Size | Sent by, when | The sender itself | A receiver | What a copy back to the sender does |
|---|---|---|---|---|---|
| 23 | 16 | the leader, at the start (`PlayMiniGolfScript` state 2): up to four player SIDs, 0 for none | builds the hole and marks the game started, right after sending | the same, from the list | builds the hole again (INFERRED from code) |
| 40 | 10 | `aimBox`, when the player aims: the angle | applies it | applies it | nothing (the same value; INFERRED) |
| 41 | 10 | turn barrier (`proc_160`, `proc_161`), two players or more: 1 to the group by the player on the tee; 2 back to the sender of 1 (`to` = its `fromSID`); 3 to the group by the sender of 1 once its count equals the player count | counts itself (the count starts at 1) and moves its own script on after sending 3 | answers 1 with 2; counts 2; moves on at 3 | the sender of 1 answers its own 1, counts itself twice and sends 3 before the other player has answered; its own 3 moves it on a second time. CONFIRMED live |
| 42, 43 | array, 12 | `playGameScript`, by the player on the tee (INFERRED): positions on the hole | shows them | shows them | shows them again (INFERRED) |
| 44 | 32 | the shooter, once its ball stops (`movingScript`): twelve words, the shot and its result, the stroke count and the player number last | has played it | stores it for the replay | see 45 |
| 45 | 20 | the shooter, after 44: one per event of the shot (`GolfEvent`), then event 99 | nothing more | adds the event to its replay list; event 99 starts the replay (`movingScript`) when the player number is the one on the tee | the shooter replays its own shot; at the end of the replay it is the shooter again, so it sends 44 and its list once more, now with the copies added: one 45 more each time. CONFIRMED live |

So no MiniGolf message wants its own copy, and the host gives none (rule 5). The operator's log of a solo
round (one 16, then per stroke a 32 and a growing run of 20-byte Sends, the stroke counter rising, then a
hang-up) is the 44/45 loop. With two players the copies also desynchronise the courses and stop the turns
(section 8).

The same rule fits every other client that was read, which is why it is not a MiniGolf exception:

| Client | What it does | Status |
|---|---|---|
| PaintBall (`SL/script.780`) | `SendMyUpdate::doit` sends the update to the group, then a type 102 `Send` to its own player SID and sets a busy flag; `PaintBall::handleMsg` (`+0x0103`) clears the flag on 102, and `startGame::doit` (`+0x0469`) sends the next update only when it is clear | CONFIRMED code; not run live |
| GOLF (`GOLF.EXE`) | the `CC` update goes to `{group, own SID}`; `gcOnShot` (image `0x14E9B`) queues every `CB` shot with no test of the sender, so a copy of its own would be queued too (its use not traced) | CONFIRMED code |
| Red Baron | the `0xC9` update goes to `{group, own SID}` (section 6) | CONFIRMED live |
| land chat (`script.005` export 2) | drops a line whose `fromSID` is its own | CONFIRMED code; either rule works |

## 6. Red Baron

Land side (`SL/script.905 rbInviteRoom`, `908 rbInviteRoom23`, `120 RedBaronQueue`, `runRedBaronScript`):
the invitation of section 2 with the extra types 61 and 63 (sides and planes), then the queue's
joinNet of kind 4, land type 101, parameter `0xFFFD`, size 4 (an own group, so a game session of game 101),
and the hand-off to `RB.EXE` through the shared block (the group SID at `+4`).

Program side ([redbaron.md of the client project](../protocol/int14h-census/redbaron.md), sections 5 to 10):

| Client sends | Host answers | Code |
|---|---|---|
| 41/2 type 101 | cmd 41, service SID `RED_BARON_SERVICE_SID_ASSUMED` (0x65) at `w@6` | `red_baron::locate_service` |
| 17/0 service, user, prop 9 | setInt on the service SID, first pair (9, `0x0201`): the version the game demands | `red_baron::service_properties` |
| 17/0 prop 0xF | setInt, first pair (0xF, 10): the game length in minutes (5 to 30 accepted) | same |
| 36/2 | host time, as for every client | `host_time` |
| joinNet kind 3, type 101, size 1 | `ObjID` for the player's in-game object | `ObjectStore::join_within` |
| GrpJoin `group, player` | echo to every member's connection **including the joiner's** | `ObjectStore::join_group` |
| GrpMem | the member list in joining order (in the 2026-10-16 run the pilots' game objects only; the host also lists land objects that join) | `list_members` |
| Send 0xC8 to the group | to every other pilot's connection, `fromSID` the real sender | `router::send` |
| multicast `{group, own SID}` with 0xC9 | one Send per recipient: the group copy to the other pilots, the own-SID copy to the sender (the echo that releases the next update) | `router::multicast` |
| 50 (status 0 playing, 1 leaving) | none; logged | `PlayStatus` |
| GrpDel, ObjFree, or a hang-up | `GrpDel` to the others, `ObjFree` for the player's object | `ObjectStore::disconnect` |

Every wait in RB is fatal after 119.6 s, and none of its handlers reads a Nak, so a refusal is a
two-minute stall: the host answers each request above and refuses none of them (the capacity rule of
section 4 exists for this reason).

## 7. Verification

- `crates/innkeeper-world/tests/games.rs`: byte-exact transcripts with two and three clients: invitation,
  lock conflict, accept, decline, start, the newcomer's roster, relay in the game group, a player
  dropping mid-game, MiniGolf strokes alone and with two players (bytes of the 2026-10-16 runs), and the Red
  Baron program's whole start-up.
- Live: [section 8](#8-live-results).

## 8. Live results

2026-10-15. Two headless `dosrt` clients (accounts 100001 and 100002, players Ann and Bob, from the public
build of the client with no lifted code) against `innkeeperd --no-int14h` on a private port:

| Scenario | Result |
|---|---|
| both log in and enter SierraLand's waiting room | each sees the other (after the roster fix of section 2) |
| Stratego: Ann invites Bob, Bob accepts, Ann starts | both load Stratego |
| Graffiti: same | both load the paint room |
| Red Baron: Ann invites, Bob accepts, "Join Line" | both switch to RB.EXE and reach the cockpit ("Time Left 09:08"); pilot state is exchanged |

2026-10-16, MiniGolf and rule 5. The same setup on private ports with `--capture-dir`. The MiniGolf rows of
the right-hand column were run with both the client build with lifted code and a build with none; the rest
with the lifted build. A stroke is a drag of the club in the power bar.

| Scenario | Host before (sender's copy) | Host now (rule 5) |
|---|---|---|
| MiniGolf alone, one stroke | the stroke is sent again and again, the run of 20-byte Sends growing by one each time (1 to 6 within the run), the stroke counter in the 44 rising: the operator's report | one type 44 and one type 45 per stroke; four strokes, counter 0 to 3 |
| MiniGolf, Ann invites Bob, Bob accepts, Ann starts | Ann answers her own barrier 1 and sends 3 before Bob's 2; after one stroke each the two courses show the balls in different places and no further stroke is sent | barrier 1, 2, 3 in order; three strokes each, alternating, one 44 and one 45 each; both screens show the same balls |
| Stratego start (regression) | both load Stratego, coin toss | the same |
| Red Baron start (regression) | (2026-10-15 above) | both reach the cockpit (Bob sees Ann's aircraft); both pilots multicast `0xC9` until the run ends (about 370 in all), each update released by its own-SID copy |

Not exercised live: NTN Trivia, PaintBall, RocketQuiz, SneakATac (same invite rooms), the hub's board and
card games under rule 5, a MiniGolf hole played to the cup, Stratego past the coin toss, three or four Red
Baron pilots, a pilot dropping mid-flight, Red Baron's return to the land. Scripted live runs of
two emulated clients are timing sensitive: the processes run at different wall speeds, and one that ends
its run disconnects, which the other then sees as "just left the room".

## 9. Open

- What command 30 meant to the original host; the model treats it as a reference (rule 3).
- Whether the original host sent `GrpJoin` or `GrpMem` to a newcomer of a waiting room (section 2).
- The meaning of the type 12 words after the game number, and of the multicast type 57 (the guest side
  reads it as cancel or "started without you").
- Whether the host ever refused a second game on a busy group; no client path reads such a Nak.
- Whether the original host left out the sender's connection (rule 5) or only the member named in
  `fromSID`. They differ only for a connection with two members in the group, such as a land player and a
  DOS game's object; GOLF's `CB` handler (section 5.1) favours leaving out the connection.

## 10. Arena

Land type 5, place 1, description `Arena`, capacity 64, directory flags `0x01`. Bit `0x10` stays clear,
so a client that already has `GAMEINFO.CFG` does not ask the host for 36/8. `LandCatalog::stock` lists
the row after Clubhouse, SierraLand and CasinoLand and before the RPG titles. `lsci()` is still the
three-land capture the golden logon replays. An account whose stored rating is 0 logs in with `0x00BF`.
The waiting room is the ordinary kind-5 group, and a 1.4.46 GrpJoin is admitted because the row's
version range is any version. The place list is that one occupancy row.

Live, 2026-10-17, one headless 2.4.46 client (account 100001, hub persona guybrush) against this host.
The Fall Map showed the Arena and accepted the click into it. The place dialog listed Arena at
0 of 64. Choosing it started `LSCITV` in the Arena directory. Select Player has no shipped persona
(`PLAYER.LST`, `name.LND`, `name.PER`). Create took the name form (Guybrush) and the appearance screen,
and Go On drew Game Skills for Red Baron, 3-D Golf and Football. The Arena program then terminated with
DOS code 1 (`INN Exec: Error 1 in Arena`) and wrote no persona files. The want-to-play list, the waiting
room, an invite, the join line and Top Scores were not reached. A second client was not started.

Every client message on that path decoded. Two requests were absorbed, both the same as every other
hub logon: 34/4 rates (left unanswered) and 36/1 `HostAddressFile` (the client keeps its files).
Nothing was logged as not decoded or unexplained. The link ended when the program exited: a CR
between frames, which the PAD reads as escape.

The games themselves were not on the wire. Red Baron (invite 910, start 926, chain 101) and 3-D Golf
(invite 911, start 927, chain 102) use the same invite, queue and `Send` path as SierraLand. Football's
play channel is unicast 2/1, which the router already delivers; its invite is 912. Top Scores is an
LSCI command 27 read, and those boards are still empty ([coverage.md](coverage.md) section 3.2).

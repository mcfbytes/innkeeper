# Game groups and invitations

How `innkeeperd` lets two or more INN clients start a multiplayer game together: SierraLand's own games
(Stratego, Graffiti, MiniGolf, NTN Trivia, PaintBall, RocketQuiz, SneakATac and the hub's board and card
games) and Red Baron. The code is `crates/innkeeper-world/src/objects/games.rs` (the model),
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
5. **A Send to a game group** goes once to every connection that holds a member, the sender's included
   (`GROUP_SEND_ECHOES_SENDER_ASSUMED`), with the real sender in `fromSID`. A multicast is one such relay
   per listed recipient, so Red Baron's `{group, own SID}` list returns the update to its sender
   (section 6).
6. **Waiting rooms** tell a newcomer who is there (section 2, point 1). Land groups of the RPG titles are
   exempt: DARKSTRT and FATES tell their own roster with game messages and were verified without it.

## 5. SierraLand games

Nothing beyond sections 3 and 4. Verified live (section 8) against `innkeeperd` with two headless `dosrt` clients
(accounts 100001 and 100002, players Ann and Bob):

| Game | Result |
|---|---|
| Stratego | invitation delivered, accepted, both clients load Stratego (the coin toss dialog) |
| Graffiti | both clients load the paint room |
| Red Baron | section 6 |

The others use the same invite rooms (`slInviteRoom` 906 for Stratego, MiniGolf, Graffiti, SneakATac,
Boogers, NTN Trivia and RocketQuiz; `pbInviteRoom` 907 PaintBall; `WPSinviteRoom` 909) and the same
group calls, so they need nothing else from the host; only the two above were exercised.

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
| GrpMem | the member list (land objects and game objects both, in joining order) | `list_members` |
| Send 0xC8 to the group | to every connection with a member, `fromSID` the real sender | `router::send` |
| multicast `{group, own SID}` with 0xC9 | one Send per recipient; the own-SID copy is the echo that releases the next update | `router::multicast` |
| 50 (status 0 playing, 1 leaving) | none; logged | `PlayStatus` |
| GrpDel, ObjFree, or a hang-up | `GrpDel` to the others, `ObjFree` for the player's object | `ObjectStore::disconnect` |

Every wait in RB is fatal after 119.6 s, and none of its handlers reads a Nak, so a refusal is a
two-minute stall: the host answers each request above and refuses none of them (the capacity rule of
section 4 exists for this reason).

## 7. Verification

- `crates/innkeeper-world/tests/games.rs`: byte-exact transcripts with two and three clients: invitation,
  lock conflict, accept, decline, start, the newcomer's roster, relay in the game group, a player
  dropping mid-game, and the Red Baron program's whole start-up.
- Live: [section 8](#8-live-results-2026-10-15).

## 8. Live results (2026-10-15)

Two headless `dosrt` clients (accounts 100001 and 100002, players Ann and Bob, from the public build of the
client with no lifted code) against `innkeeperd --no-int14h` on a private port:

| Scenario | Result |
|---|---|
| both log in and enter SierraLand's waiting room | each sees the other (after the roster fix of section 2) |
| Stratego: Ann invites Bob, Bob accepts, Ann starts | both load Stratego |
| Graffiti: same | both load the paint room |
| Red Baron: Ann invites, Bob accepts, "Join Line" | both switch to RB.EXE and reach the cockpit ("Time Left 09:08"); pilot state is exchanged |

Not exercised live: MiniGolf, NTN Trivia, PaintBall, RocketQuiz, SneakATac (same invite rooms), three or
four Red Baron pilots, a pilot dropping mid-flight, Red Baron's return to the land. Scripted live runs of
two emulated clients are timing sensitive: the processes run at different wall speeds, and one that ends
its run disconnects, which the other then sees as "just left the room".

## 9. Open

- What command 30 meant to the original host; the model treats it as a reference (rule 3).
- Whether the original host sent `GrpJoin` or `GrpMem` to a newcomer of a waiting room (section 2).
- The meaning of the type 12 words after the game number, and of the multicast type 57 (the guest side
  reads it as cancel or "started without you").
- Whether the host ever refused a second game on a busy group; no client path reads such a Nak.

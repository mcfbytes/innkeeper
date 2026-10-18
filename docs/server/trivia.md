# NTN Trivia

What `innkeeperd` does so an unmodified INN client can sit in a trivia room, pick a handle, and play
Countdown. The code is `innkeeper-world::trivia` (the rooms and the show) and `message::trivia` (command
54 when the server word is 4). Shoppers Advantage uses the same command with server word 5
([coverage.md](coverage.md) section 3.1). Byte layouts were read from `vocab.728` (room `NTNTrivia`) and
`vocab.729` (`proc_237` / Feb-94 `proc_246`) in the 2.4.46 hub; SierraLand's copies differ by 15 bytes
that do not change the wire. `PMSData` is `script.012` in 2.4.46 and inside `vocab.729` in Feb-94; the
send format is `"bbwwwww"` in both.

## 1. The waiting room

`GotoTrivia` (`hub/script.120` item 320, the same method in `SL/script.120`) sends 27/1 for message 1 of
board class **85** in the 2.4.46 hub and class **84** in SierraLand and Feb-94 (`GotoTrivia::init`,
immediate `0x55` / `0x54`). A normal player reads the text with `subStr(18, -1)`; a privileged one with
`subStr(22, -1)` (`GotoTrivia::handleMsg` item 321). `proc_309` splits on bytes 1 and 2: the span after
`\x01` is the info line, each later `\x02` a topic. CONFIRMED.

Until a moderator writes, this host answers 84 and 85 with one message:

```
\x01Choose a room\x02Adventure games\x02General trivia\x02Games and screens\x02
```

Conference class 67 stays empty until someone with user flag `0x0004` posts. A moderator 27/0 on 84 or 85
replaces that board's text, as for 67 ([hub-services.md](hub-services.md) section 5).

The chosen button 1, 2 or 3 becomes a kind-2 `joinNet` (`GameGroup::joinNet`, `"wwwbbww"`). Hub parameters
are 16, 17, 18; SierraLand and Feb-94 use 7, 8, 9. Size is 20 in 2.4.46 and 12 in Feb-94. This host maps
both triples onto the same three shows. The `GrpJoin` that follows must echo with `who` equal to the
player's SID or `global144` stays 0 and room `0x22d8` never loads (`class_87::handleMsg`, CONFIRMED).

## 2. JOIN and the handle

Room `init` (`vocab.728` item 1) constructs `NTNData` with `server = 4` and a kind-1 `joinNet`
(parameter `0xFFFF`, size 13). ObjID then makes `PMSData::handleMsg` send sub 25:

`b 54, b 25, w 4, w 0, w 0, w 0, w sid, w mailboxLow, w mailboxHigh`

The host answers whichCmd 17 with the four routing words filled in (`server` 4, a per-session `number`,
`host` the stock host, `remote` 0) and `toSID` the PMS object's SID. Silence for 30 s is
"Timed out waiting for NTN game initialization." (`WaitForNTNInitMsg::init`, `ldi 30`). CONFIRMED.

The client then sends the version line `TRIVIA%V01.200\r\n` as sub 24. The logon the host sends is type 0
then type 1 (`vocab.729` `proc_237`). Type 0 with `local3 == 0` drops `WaitForNTNInitMsg`. Type 1 opens
the handle dialog because `local5` starts at 1 (`vocab.729` item 1 VARIABLES `[5]=0x0001`; Feb-94 the
same). A later type 0, once `local3 == 1`, is the reject dialog (`"because: %s"`). This host sends that
type 0 only when a handle is refused. A handle is 1..6 graphic characters plus CR LF, uppercased, with
leading and trailing spaces dropped. Names starting TSN or INN (spaces inside ignored) are refused, as the
client already refuses them for non-staff, and so is any other staff or service name
(`is_reserved_name`, [privacy.md](privacy.md) section 1). A name already used in the same room is
refused the same way, and two handles that differ only in spaces (`GUY`, `G UY`) count as the same name,
so nobody can sit beside a player under a look-alike. In `--public` the log line for an accepted handle
carries its length, not the handle. Type 39 carries the accepted handle (`subStr` of the whole payload)
and type 2 with help byte 0 unlocks question types (`local4` becomes 2). CONFIRMED the flag stores and
the local initialiser; the host does not implement a staff exception.

**A room with no questions.** When a room's pool is empty (no bank was found at start-up, or every
row was dropped), a JOIN is not seated: it is answered with whichCmd 18, the JOIN failure, with word 42
at byte 12, the client's "NTN Trivia has gone down" path, instead of whichCmd 17. Otherwise the player
would pick a handle and wait for a question that never comes. INFERRED that the client shows the
gone-down dialog for 18 with 42 (the codec's reading of the receive-side handlers); not run live.

Sub 26 (EXIT) and a hang-up drop the seat. The next JOIN on that connection is a new session number.

## 3. The show

One Countdown game per room, idle while empty. The first handle in an idle room waits two seconds so
type 39 and type 2 can land, then starts a game. A game is 15 questions. Per question the host sends,
to every seated player who has a handle:

| Type | Payload | Client |
|---|---|---|
| 3 | `b kind, w secs, w points, b clues` | stores `secs * 60` ticks (`proc_237` +0x025c) |
| 4 | `b number, b total`, four 24-byte rows | `proc_339` |
| 5 | `b count`, then `count` 24-byte rows | `proc_340`, buttons 1..count |
| 23 | `b iteration` | starts the on-screen clock from the type 3 values |

`secs` is 20, `points` 1000, the rest 0. Answers are shuffled per deal. A player who joins mid-question
waits for the next one.

The client answers once per question, as sub 24, framing `b 0, b 3, w number, b 2` then `"%d\r\n"` of
the 1-based button (`proc_243`, `vocab.729` +0x4fc8). Repeats and late lines are ignored. Chat is
command 28, not an answer.

When the 20 s host clock elapses:

1. type 8, the 1-based correct index, to everyone in the room
2. type 7 to each player who answered: their index and the points for that question
3. type 10, eight little-endian words, to everyone: this answer, round total, round rank, game total,
   total rank (`wordAt` 0, 2, 4, 6, 8, 10, 12, 14 as `vocab.729` reads them)

Then 5 s of quiet and the next question. After question 15: type 14 (up to three winners, 24-byte
records, stride `ldi 24` at `vocab.729` +0x0e52), type 24, type 22 with 1 minute, then a 1-minute
break. Type 22 is what builds the "stay with us" line. CONFIRMED the type numbers and the slices; the
payout is this host's (section 4).

`innkeeperd` looks at the clock every 100 ms (`World::tick_trivia`) and delivers through the switchboard.
Tests pass a fake millisecond time into `Shows::tick`.

## 4. Scoring

The manuals describe Countdown as a faster correct answer scoring more, and a wrong answer penalized.
The client prints the words it is given. This host:

- a correct answer pays linearly from 1000 at type 23 to 100 at the end of the 20 s
- a wrong answer scores 0 for the question and subtracts 250 from the game total, floored at 0
- no answer scores 0 and leaves the total

Round total and game total are the same running total for this game of 15. Rank is among players in
the room who have a handle, highest total first, earlier seat breaking ties. There is no career score.

## 5. Question banks

`--trivia-dir` (default `data/trivia`) holds `questions.json`, original GPL questions about adventure
games. `--opentdb` (default `third_party/opentdb/opentdb.json`) is a CC BY-SA 4.0 snapshot of the Open
Trivia Database. Both are loaded at start-up, fitted, and kept in memory.

A question is used only when, after transliterating accents, it is ASCII, the stem wraps into at most
four lines of 24 characters, and every answer is at most 24. Booleans become True/False. Counts of
kept and dropped rows are logged as `trivia question pools`.

| Room | Topic | Pool |
|---|---|---|
| 1 | Adventure games | the GPL bank |
| 2 | General trivia | OpenTDB categories that do not start `Entertainment:` |
| 3 | Games & screens | OpenTDB Entertainment: Video Games, Film, Television, Board Games, Comics, Cartoon & Animations, plus the GPL bank on every fourth draw |

Each pool is a shuffle-bag: it is drawn without replacement until empty, then reshuffled.

## 6. What this host does not do

Clues, leaders, net rankings, Showdown, Sports Challenge, category votes, and chat in the trivia room
are outside the Countdown path. Command 12 from the room is ignored. whichCmd 18 is sent only for an
empty room (section 2); 20/24/25/26 and inner command 42 are encoded and unused.

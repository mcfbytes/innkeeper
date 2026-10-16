# Hub services: complaints, the Hint Board, bulletin board erase and the sysop pages (27/0, 27/6, 27/9, 38, 37/20, 37/21)

What the hub (and the SL and LL land programs that share its scripts) send to the host for complaints, the
Hint Board room, the bulletin board's erase and the sysop mail pages, what each one waits on, and what
`innkeeperd` answers. The scripts are `hub/script.005` (the chat window and the complaint form),
`hub/script.130` and `hub/script.199` (the bulletin board classes, `BBS`, `NUKEMI`), `hub/script.135` (the
Hint Board room), `hub/script.120` and `SL/script.120` (conferences and trivia) and `hub/script.145` (mail).
`SL/script.005`, `LL/script.005`, `SL/script.199`, `LL/script.199` and the SL, LL copies of the others are the
same code. Code: `innkeeper-world::bbs` and `message::bbs`, `hint_board` and `message::hint`,
`message::mail`. Every text the server sends is its own wording. Offsets are message bytes from 0.

## 1. The audit

"Wait" is what the client sits on after sending. Each row was found by listing every `TSN` send in the hub, SL
and LL scripts whose first value is the command, and following the callers.

| Sender | Request | Wait | Before | Now |
|---|---|---|---|---|
| Chat window "Complain" (`hub/script.005`, a copy in `SL` and `LL`) with a player named | 27/9 `"bbwwwwa"`: `w 0 @2, w fromSID @4, w 99 @6, w aboutSID @8`, the text and its NUL | none: the client says "Your complaint has been forwarded." the moment it sends | `UnsupportedSub`, "not decoded", complaint lost | kept as a complaint (section 2), no reply |
| The same form with nobody named | 27/0 `"bbwwwa"`: `w 0 @2, w fromSID @4, w 99 @6`, text and NUL | none | parsed as an RPG post to board 0/99, which no directory lists, and dropped with a warning | kept as a complaint, no reply |
| Sketch Pad "Complain" (`SL/script.530` `BitchToTSNMI`) | 27/9 `"bbwwwws"`: the same, the text a string | none: the same message | "not decoded" | kept as a complaint |
| Bulletin board "Annihilate Messages" (`NUKEMI`, sysop level 3 only) and `BBS::erase` | 27/6 `"bbwwwwb"`: `w location @2, w fromSID @4, w section @6, w message @8, b 0 @10` | none: it loops on a send-queue flush call (`TSN` 12) between messages, not on a reply | `UnsupportedSub`, "not decoded" | erased for the author or a privileged reader (section 3), no reply |
| `script.199` export 2 (27/8 `"bbwww"`: three words) | none: nothing in the hub, SL or LL calls it | none | `UnsupportedSub` | unchanged, dead code (section 6) |
| Hint Board room (`hub/script.135`) | 38 `"bbwbbbb"`: `b 38, b step, w sid, b ×4` | a "Getting a hint / menu / answer, please wait" box until the reply, no timer | dropped, "not decoded", the room says "The Hint Board is disabled for some reason" after Nak, or sits on the wait | answered (section 4) |
| Conference list (`GotoConference`) | 27/1 `"bbw+"`: `w 0, w sid, w 67, w 1`, then 41/0 | "Requesting Conferences" until the reply | already answered: the message reply is empty (a board with no text), 41/0 free | the text is the moderators' topic text (section 5) |
| Conference "Change Topics" (moderators) | 27/0 `"bbwwwa"`: `w 0, w sid, w 67`, the topic text | none | dropped as a post to an unlisted board | stored when the sender has moderator rights |
| Trivia list (`GotoTrivia`, SL) | the same with `w 84` | "Requesting Trivia Info." | as the conference list | as the conference list |
| 37/20, 37/21 short forms (`script.145` exports 8 and 7, the sysop screens) | `"bbw+"`: `w sid, w boxLow, w boxHigh, w, w`, and for 37/20 `w 0x7530, w 0` | 37/20 sets a 30 s wait (the word 0x7530) | `Truncated`, "not decoded", no reply | no live sender; accepted, refused with fault 12 (section 7) |

A search for anything else in the hub, SL or LL that sends and waits turned up nothing new that is
unanswered (section 8).

## 2. Complaints (27/0 board 0/99 and 27/9)

The text is built by the client: it starts `Complaint|From: <name> in: <room>; about: <name>, who said "<line>"
Comments: <what the player typed>`, and the Sketch Pad's starts `<name>|Graffiti:` and lists the artists. The
word at byte 6 is the literal 99, which is the complaint board's location (`0x63`). The host:

- stores it as a post on board location `0x63`, land 0, section `COMPLAINT_SECTION_ASSUMED` (3), body
  `persona|Complaint: text`. The client's own `Complaint|` prefix is dropped; the post records the writing
  account (`posts.account`), so the account's export and delete find it (privacy.md sections 1 and 6);
- keeps at most `COMPLAINT_TEXT_LIMIT_ASSUMED` (2000) bytes of the text;
- records the account the complaint is about (`posts.about_account`) when it can tell, as below;
- is covered by the 365-day complaint retention (`--retain-complaint-days`) because the board location is
  `0x63`, and by a legal hold on the author's account or on the account it is about;
- logs only the account, the post id, the length, whether someone was named and the account resolved for
  them ("complaint kept"). The
  decoded request prints `ComplaintText(len N)`, never the text, in every mode. The raw hex line of the
  verbose mode still carries the bytes, as it does for mail; `--public` drops it;
- answers nothing, and needs no land: the hub has none for the player. A store failure is logged as a
  warning with the length and the account.

The SID of the player complained about (sub 9) is a connection-local object number, so it is not stored; the
client's text names the player after `about: `, up to the next `,` or `;` (`bbs::named_offender`, 1 to 16
printable characters). The host resolves that name once, at filing time, against the players online
(`bbs::offender`): when exactly one account other than the complainer's is playing under that persona,
case-insensitive and trimmed, that account is stored as `about_account`. No match, two accounts under the
name, or only the complainer leaves `about_account` NULL, and the name stays only in the text. Nothing reads
the name again later: the portal links the complaint to `about_account` and the author's account, never to an
account found by name. Section 2 of the board (Member Services) and section 3 (this) are listed by no
directory, so no game client can read either back.

## 3. Bulletin board erase (27/6)

The hub's own bulletin board (`theBBSs`, `BBS` in `hub/script.130`) is the same protocol as the RPG board
(yserbius.md section 3): 27/13 with location 66 lists boards, 27/14 an index, 27/1 a message, 27/0 a post of
`author|text`. This host lists no boards at location 66, so the Clubhouse's "Bulletin Board" says "There are
no bulletin boards available." Erase is only offered at sysop level 3, from the user flag `0x0004`.

27/6 removes message number `message` (the position in the board, counted from 1) of the board named by
`location` and `section`, in the player's land. The host deletes it when the requester's flags are privileged
(`0x0002` or `0x0004`, the same flags that move the text of a read) or the post was written by the requester's
account (`posts.account`; the name at the head of the body is not compared, so a player cannot erase a
same-named player's post; `ERASE_NEEDS_AUTHOR_OR_PRIVILEGE_ASSUMED`), and ignores it otherwise; a message that is not there is
ignored. No reply is sent. A deleted post leaves the board's numbering shifted, as every delete here does. Not
exercised live: nothing reaches it while the hub's directory is empty (the transcripts test it on the Tavern board).

## 4. The Hint Board (38)

The Hint Board is the Clubhouse's "Go to > Hint Board" room. It is a three-level menu then hints:

| Request step | Bytes after `sid` | Reply |
|---|---|---|
| 0 | four bytes the client does not clear (they held stack junk live) | 38/0: the series menu |
| 1 | series | 38/1 with `which` = series: the games of that series |
| 2 | series, game | 38/2 with `which` = game: the areas of that game |
| 3 | series, game, area, hint number | 38/3 `which` = number: the problem; 38/4, same `which`: the answer; or Nak 38/6 when the number is past the last hint, Nak 38/5 for a choice the menus do not have |

A reply is `b 38, b kind, w toSID, b which @4`, then text from byte 5 and its NUL. `toSID` is the room's own
object SID from the request; the room's object-id reply (`ObjId`, command 8) is what makes it send step 0. A
menu choice is the 1-based position of the line the player picked; a menu is lines separated by `\n`, ten
to a page, and the client draws a button per line and wraps the text (live: buttons hold about 24 characters,
the problem box shows two lines before the answer button and the answer box about six). The room keeps menus
by level (it asks again only when the choice changes) and hints by number, so a question and its answer are
sent together; `which` must equal the number asked or the room waits for it. Nak 38 reason 5 makes the room
say the menu is unavailable and go back to the first menu; reason 6 shows "There are no more hints for this
topic." with the way out. The request carries no room number, so one catalogue serves every room.

The text lives in one table, `SERIES` in `innkeeper-world/src/hint_board.rs`: series, games, areas and hints in
order, each hint a problem and an answer. Edit that table to change what the board says; a unit test keeps each
entry within the sizes the boxes took. Checked live (section 9).

## 5. Conference and trivia topic text (27/0 and 27/1, class 67 and 84)

`GotoConference` (hub) and `GotoTrivia` (SL) read message 1 of board 0/67 or 0/84 with an ordinary 27/1 and show
its text as the topic and info of the conference or trivia rooms (`\x01 info \x02 topic \x02 ...`, up to four
topics). A moderator's "Change Topics" (shown only at sysop level 3) writes that text back with a 27/0. The
host keeps one text per class, shared by every land (`TOPIC_SECTIONS`, land 0): a write from an account with
the user flag `0x0004` replaces the stored text (the old one is deleted, so message 1 is always the latest),
and any reader's 27/1 gets it. A write from anyone else is dropped with a warning. Until a moderator writes,
the read answers an empty board, as before. The conference name check (41/0) and the room itself are in
[coverage.md](coverage.md) section 3.1 and are unchanged. Not exercised live: the maintenance dialog is
reachable only with a debug-level client configuration, so the write is tested by transcript only
(`tests/hub_services.rs`), and a second client in a conference room (multiplayer chat) was not tried.

## 6. 27/8

`script.199` export 2 builds `b 27, b 8, w, w, w` (the board's location, the sender and the section, as for
27/6). Nothing in the hub, SL or LL calls that export, so no client sends it, and it stays unparsed. It is a
row in [coverage.md](coverage.md) section 3.3.

## 7. The short 37/20 and 37/21 (sysop pages)

The two short forms are `script.145` exports 8 (sub 20) and 7 (sub 21), `"bbw+"`: the object, a box
(`w low, w high`), two more words, and for 37/20 `w 0x7530, w 0`. They validate the box number first
("The box number must be between 1 and 4294901759.") and then send; nothing in the hub, SL or LL calls them, so
no stock screen sends them. The meaning of the two words is not in any script. They are not a letter and carry
no text, so nothing is stored. The host parses them (a send with fewer than the 46 envelope bytes after the
box) and answers Nak 37/sub with fault 12 ("you do not have rights to do that") to the box asked about, which
ends the 30 s wait (`SHORT_MAIL_FORM_IS_REFUSED_ASSUMED`). A real sysop screen that expected an Ack would show
that text.

## 8. Everything else that waits

Every other `TSN` send in the hub, SL and LL scripts maps to a row of [coverage.md](coverage.md): the
ones with "Please wait" boxes are the bulletin board (27/13, 27/14, 27/1, answered, board list empty), mail
(37/17 to 37/32), the phone book (49), the mall gate (54/27), the Town Hall (34, 37/30, 37/32) and this page's
38 and 27. The ones still worth a note:

- **Top scores** (SL "Checking Top Scores..." and "Updating Top Scores...", LL): a 27/1 read of a board the
  game names, then a 27/0 post. The read is answered (an empty board) and the post is dropped because no
  directory lists the board, so scores are not kept. The client does not freeze.
- **Bulletin board** (location 66): the directory is empty. A board listed there needs a 17-byte record whose
  last eight bytes the hub reads as a word (the last message number plus 10, between 26 and 16394) and a six
  byte time (`BBS::fromArray`, `script.199`); the RPG clients ignore them, and a record with zeros there is
  shown as "The ... BBS is invalid." (seen live with a trial board, then removed). Adding boards would
  need those two fields and a check of the read, index and post layouts.
- **37/31 services** remains an empty list ([mail.md](mail.md) section 8).

## 9. Checked against the stock client

Headless native runner, the Feb-94 client, a fresh data directory (2026-10-16). No "not decoded" line was logged
by any of these. Screenshots are under `/tmp/claude-1000/-mnt-source-INN-Decomp/8be7456b-11fb-4bca-94e9-86c80d59362c/scratchpad/gaps/shots/` (not in the repo).

- **Hint Board:** Clubhouse (the red building on the map), Go to, Hint Board. The first menu draws four series
  with arrow buttons; each choice draws the next menu; a hint draws its problem, and the answer button shows the
  answer; Next Hint steps to hint 2 and past the last hint draws "There are no more hints for this topic."; Prev
  Menu returns to the menu above; WaitRoom returns to the Clubhouse and its buttons answer.
- **Complaint:** two clients (accounts 100001 and 100002) in the Clubhouse; one talks to the other; the message
  box shows Complain, Mute, Reply and OK. Complain opens the "Complaint Form"; typing and "Send To INN" shows
  "Your complaint has been forwarded." and the room is not frozen afterwards (Go to opens). The server logged
  "complaint kept ... length ... names_someone=true" with no text, and the row in `innkeeper.db` is board 25347
  (`0x63 << 8 | 3`), body `guybrush|Complaint: From: ... about: ..., who said "..." Comments: ...`.
- **Conference:** Go to, A Conference: "Go to which Conference?" with -OPEN-; opening it reaches the Conference
  Room. The 27/1 read and the 41/0 check were answered. With the moderator flag the "Maintain Conferences"
  dialog did not appear, so "Change Topics" was not driven.
- **Sketch Pad complaint (27/9 with a string), 27/6 erase, the short mail pages:** no live path was found, as
  section 1 says; the golden rows and `tests/hub_services.rs` cover the bytes and the answers.

## 10. Assumptions

Named constants in `innkeeper-world/src/assumptions.rs`: `COMPLAINTS_ARE_KEPT_ASSUMED`,
`COMPLAINT_SECTION_ASSUMED`, `COMPLAINT_TEXT_LIMIT_ASSUMED`, `ERASE_NEEDS_AUTHOR_OR_PRIVILEGE_ASSUMED`,
`SHORT_MAIL_FORM_IS_REFUSED_ASSUMED`. In `bbs.rs`: `TOPIC_SECTIONS` and `MODERATOR_USER_FLAGS` (the topic text
boards and the flag that may write them).

## 11. Not answered

- Whether the original host forwarded a complaint to staff, to whom, and with what extra fields.
- The meaning of the two words in the short 37/20 and 37/21, and what a sysop screen expected back.
- The hub bulletin board's boards at location 66, and the top-score boards' keys and layouts (section 8).
- The moderator topic write and the erase live, and trivia (an SL land program), with the real client.

# Message coverage

What `innkeeperd` does with every client-to-host command any INN program can send, and which host-to-client
messages those programs handle. This page is an audit of the code against the catalogue. It does not change
the server. Where [innkeeperd.md](innkeeperd.md) section 7 still says `SetInt` and `getProp` are ignored, the
code below has moved on: this page follows the code.

Sources: [messages.md](../protocol/messages.md) sections 3, 9 and 11 (the catalogue), the per-program census
([int14h-census/](../protocol/int14h-census/README.md)), [yserbius.md](../protocol/yserbius.md),
[ktsn.md](../protocol/ktsn.md), and the server pages linked from section 5 of [innkeeperd.md](innkeeperd.md).
The codec is `crates/innkeeper-world/src/message/`. The handler is `PlayerSession::handle`
(`crates/innkeeper-world/src/player.rs`). Rust paths below are under `crates/innkeeper-world/src/` unless
they start with `innkeeperd/`.

## 0. How to read the tables

A citation of a doc section carries that section's own CONFIRMED or INFERRED label. A citation of a Rust
file is CONFIRMED: the line was read. A conclusion this page draws on top of those sources is marked
**INFERRED** in the cell. Nothing is filled in from memory.

`Host::answer` (`innkeeperd/src/host.rs:89`) parses the body with `ClientMessage::parse`
(`message/client.rs:140`). `Command::try_from` (`message/command.rs:87`) accepts only the 33 bytes in the
`Command` enum (`message/command.rs:6`). Any other command, an unknown sub-command, or a body the parser
cannot finish becomes `Exchange::Undecodable` and is logged as "client message not decoded"
(`innkeeperd/src/host.rs:94`, `:153`). No reply is sent.

**Before Login** every accepted message except Login itself is dropped: `when_logged_in` logs "message
before Login ignored" and returns nothing (`player.rs:209`). `join_net` does the same (`player.rs:193`).
The rows below assume a logged-in session. The DOS games never send Login; the land's Login, or the
DOSBox launcher, covers them (census README section 4, CONFIRMED). **INFERRED**: a game started with
neither would sit in its ObjID wait, because the join is discarded.

On the word-format commands the codec reads byte 1 and does not check it (`JoinNet::parse` at
`message/client.rs:331`, `GroupJoin::parse` at `:354`, and the same `flags` read on 4, 6, 9, 11, 12, 13,
14, 25, 26, 31 and 32). Yserbius's `7/3` and `10/3` therefore decode as the same variants as `7/0` and
`10/0`. A sub-command is checked, and rejected with `UnsupportedSub`, only where a row says so.

Status of a row, one value, from the live senders:

| Status | Meaning |
|---|---|
| DONE | The live bytes decode, and the handler does what the cited client depends on. That includes a deliberate absorb where the client reads no reply. |
| STUB | The live bytes decode, and the handler's reply is missing or a placeholder where a client reads an answer. |
| GAP | `ClientMessage::parse` rejects the live bytes. |
| N/A | Every sender is dead code. |

"Live?" is `yes` when a program sends it on a path the census or the catalogue calls live, `no` when every
sender is dead, `part` when both exist. Shoppers Advantage's `Element` joinNet, register and setInt are
**INFERRED** unreached (shopadv.md section 7.1), not proven dead.

Programs: **LSCI** is LSCITV (the Feb-1994 scripts unless a row says otherwise). **Ysv** is CGENN and
DARKSTRT. **Twn** is TWGENN and FATES. **Golf** is `GOLF.EXE`. **RB** is Red Baron. **Shop** is SHOPADV.
"Dead" means the census found the builder and no caller, or messages.md says the method is never called.

`kTSN` sub-op 4 is the only LSCI send (`ktsn.md` section 3, 381 sites, CONFIRMED). It adds no command
that section 3 of the catalogue does not already list. Section 9 of the catalogue is the same four Golf
bytes (joinNet, 36/2, 41, 50), not four new commands.

## 1. Client to host

### 1.1 Shared objects

Byte 1 of an LSCI object command is 0: `notify` is never set (messages.md section 2.2, CONFIRMED).

| Cmd/sub | Name | Senders | Live? | Decoded | Behaviour | If nothing comes back | Status |
|---|---|---|---|---|---|---|---|
| 2/0 | `Send` | LSCI, Ysv, Twn, Golf, RB | yes | `Send` | Relay one `HostMessage::Send` per holder of `to`, payload unread (`router.rs:15`). RPG land types skip the sender (`rpg.rs:59`, `RPG_SENDS_ECHO_SENDER_ASSUMED` false). Other targets echo the sender (router.md section 3). A SID nobody holds is dropped, no Nak. | Sender does not wait on the call (`ktsn.md` section 3). Peers miss the move. Golf `CC` and RB `0xC9` send the next update only after their own copy returns (golf.md 7.1, redbaron.md 7.2, CONFIRMED): updates stop, the join does not. | DONE |
| 3/0 | rSend | LSCI `class_61::rSend` | no | no parser | Command 3 is `UnsupportedCommand` (`command.rs:110` test). | Never sent (messages.md 3.2, CONFIRMED). | N/A |
| 4/0 | lock | LSCI `inviteScript`; Ysv, Twn, Golf, RB builders | part | `Lock` | All-or-none. Ack `whichCmd` 4 to `from`, or Nak `whichCmd` 4 with the held word at byte 5 (`objects/locks.rs:58`). | LSCI waits: the invite text comes from that Nak (messages.md 3.2, CONFIRMED). No timeout is cited. **INFERRED**: the invite dialog stays up. DOS builders are dead (yserbius.md 4.1, golf.md 3.3, redbaron.md 5.1). | DONE |
| 5/0 | lockQueue | LSCI `class_61::lockQueue` | no | no parser | Command 5 is not in `Command`. | Never sent (messages.md 3.2, CONFIRMED). | N/A |
| 6/0 | unlock | LSCI invite scripts; same DOS builders as 4 | part | `Unlock` | Frees the words this connection holds. No reply (`UNLOCK_IS_ACKNOWLEDGED_ASSUMED` false, `objects/locks.rs:82`). | No sender waits (messages.md 3.2, CONFIRMED). Harmless. | DONE |
| 7/0 | joinNet | LSCI (kinds 1, 2, 4, 129); Ysv and Twn (kinds 1, 2, 4); Golf kind 1 type `0x66`; RB kind 3 type 101; Shop `Element` | part | `JoinNet` | `ObjID`: cookie at word 2, new SID at word 6 (`objects.rs:217`). Kind 2/4/5 with parameter below `0x8000` is one shared group; a negative parameter is an own group (objects.md section 3). Kind 129 with the same cookie replaces the old object. Unknown kind fails the parse (`client.rs:434`). | LSCI state 2 waits for the game object's SID (messages.md 4.2, CONFIRMED); the Clubhouse room waits for the group SID (messages.md 5.4). Ysv/Twn: 60 s then a fatal dialog, matched by order not cookie (yserbius.md 4.3, twinion.md 4.3). Golf and RB: about 120 s then fatal (golf.md 7.2, redbaron.md 6). Shop: **INFERRED** unreached. | DONE |
| 7/3 | joinNet, land group | Ysv, Twn kind 5 | yes | `JoinNet` | Byte 1 is discarded. Kind 5 becomes the player's place (`rpg.rs:81`, `player.rs:196`) and a shared group. `ObjID` as for 7/0. | Same 60 s fatal wait as 7/0 (yserbius.md 4.3 step 3, twinion.md 4.3 step 4, CONFIRMED). | DONE |
| 9/0 | leaveNet | LSCI, Ysv, Twn, Golf, RB | yes | `LeaveNet` | Drops one reference. The last one frees the object: `GrpDel` and `ObjFree` to the peers (objects.md 4.2). Nothing to the sender. A 4-byte DOS body and LSCI's extra zero word both parse (`client.rs:152`). | No sender waits (messages.md 3.2, golf.md 9 item 9, CONFIRMED). Harmless. | DONE |
| 10/0 | add / GrpJoin | LSCI; Ysv and Twn map and party; Golf; RB | yes | `GroupJoin` | Echo `GrpJoin` to the joiner and to every other connection with a member. Nak `whichCmd` 10, code at byte 5: 1 unknown group, 2 full, 3 not the sender's object (objects.md 4.1). No version bytes, so the version gate does not run (`presence.rs:29`). | The joiner waits for its own echo. LSCI Clubhouse: stuck on "Entering Clubhouse..." (messages.md 5.4, CONFIRMED). Ysv/Twn: 60 s, or a Nak dialog for codes 1 to 6 (yserbius.md 4.3). Golf and RB: about 120 s then fatal; RB does not read a Nak, so a Nak is the same timeout (redbaron.md 6, 10, CONFIRMED). | DONE |
| 10/3 | add, with version | Ysv 1.1.7; Twn 1.0.22; LSCI waiting rooms send the same three bytes on **10/0** (messages.md 3.2) | yes | `GroupJoin` | Same echo as 10/0. A land group whose version fails `LandCatalog::admits` or `rpg::admits` gets Nak 10 code 6 and is not added (`presence.rs:37`). | Ysv/Twn land join: 60 s, or the code-6 text "Incompatible Land Version." (yserbius.md 4.3 step 4, CONFIRMED). | DONE |
| 11/0 | delete / GrpDel | LSCI, Ysv, Twn, Golf, RB | yes | `GroupLeave` | Removes the member. `GrpDel` to the connections that still have a member (objects.md section 4). Nothing to the sender. | No sender waits (golf.md 9 item 9, CONFIRMED). Peers drop a ghost only if this, or a hang-up, is delivered (yserbius.md 4.2, CONFIRMED handler). | DONE |
| 12/0 | `GrpMem` | LSCI conferences and invite rooms; Golf; RB. Ysv and Twn builders are dead | part | `GroupMembers` | Reply 12, member SIDs from byte 6 in joining order (objects.md section 4). | Golf and RB: about 120 s then fatal "get group mem timeout" (golf.md 7.2, redbaron.md 6, CONFIRMED). Ysv/Twn receive and ignore it (yserbius.md 4.2, CONFIRMED). | DONE |
| 13/0 | setInt | LSCI (live in the Clubhouse and invite rooms). Ysv, Twn, Golf, RB builders dead; Shop `Element` **INFERRED** unreached | part | `SetInt` | Mirrored, then the same message to every other replica, not the sender (`router.rs:65`, objects.md section 9). | The sender has already applied it locally (messages.md 3.2, CONFIRMED). No reply. Harmless for the sender. A peer that never gets it keeps a stale property. | DONE |
| 14/0 | setStr | LSCI (persona, looks, home). DOS builders dead | part | `SetStr` | Mirrored and relayed like setInt (`router.rs:77`). | Same as 13: the sender does not wait. The waiting room later reads the mirror with 32 (objects.md section 9). | DONE |
| 17/0 | service properties | RB (props 9 and `0xF`). Ysv, Twn and Golf builders dead | part | `ServiceProperties` | Only sub 0 addressed to `Sid(0x0065)`. One `SetInt` per known property, first pair only: prop 9 = `0x0201`, prop `0xF` = 10 (`red_baron.rs:38`). Any other target, sub or property: absorb, logged (`player.rs:129`). | RB waits for each setInt, fatal at 119.6 s (redbaron.md 6 steps 2 and 3, CONFIRMED). Prop 9 must be `0x0201` or RB exits with the incompatible-version text (redbaron.md 6 step 4, CONFIRMED). | DONE |
| 17/2 | service properties | Twn props 8 to 11, live. Ysv's 17/2 is dead (the key is compiled in) | part | `ServiceProperties` | Sub 2 is not the Red Baron arm, so absorb (`red_baron.rs:39`, `player.rs:129`). | Twn does not wait (twinion.md 4.1, yserbius.md section 2, CONFIRMED). Harmless. Whether the original host answered with a setInt is an open question (yserbius.md section 6). | DONE |
| 18/0 | getObj | LSCI `class_61` | no | no parser | Not in `Command`. | Never sent (messages.md 3.2, CONFIRMED). | N/A |
| 19/0 | add (object store) | LSCI `class_61` | no | no parser | Not in `Command`. The name is the catalogue's, distinct from command 10. | Never sent (messages.md 3.2, CONFIRMED). | N/A |
| 20/0 | setEq | LSCI `class_61` | no | no parser | Not in `Command`. | Never sent (messages.md 3.2, CONFIRMED). | N/A |
| 21/0 | setNe | LSCI `class_61` | no | no parser | Not in `Command`. | Never sent (messages.md 3.2, CONFIRMED). | N/A |
| 22/0 | login (old) | LSCI `class_61::login` | no | no parser | Not in `Command`. Login is 53; the Ack still says `whichCmd` 22. | Never sent (messages.md 3.2, CONFIRMED). | N/A |
| 25/0 | invokeMethod | LSCI `Obj::invokeMethod` | yes | `InvokeMethod` | Relayed to the other replicas, not the sender (`router.rs:89`). The sender already ran the method. | No reply to the sender (messages.md 3.2, CONFIRMED). Harmless for the sender. | DONE |
| 26/0 | player-object notice | LSCI, twice the new player SID, right after its `ObjID` | yes | `Unexplained` | Logged and absorbed (`player.rs:145`). | No reply is read (messages.md 3.2 and 11, CONFIRMED). The Clubhouse continues (innkeeperd.md section 5). Harmless. | DONE |
| 28/— | multicast | LSCI (conferences, msgType 50); Golf `CC`; RB `0xC9` | yes | `Multicast` | No sub-command: `from` is the word at byte 1 (`message/multicast.rs:22`). One `Send` of the body per listed SID, in order (`router.rs:21`). A group SID is not expanded (router.md section 2). | Golf and RB list their own SID so the echo comes back; without it position updates stop after the first (golf.md 7.1, redbaron.md 7.2, CONFIRMED). They do not hang the session. | DONE |
| 30/— | register | LSCI `script.120`, `script.408`; Shop `Element` **INFERRED** unreached | part | `Unexplained` | Words start at byte 1. On a live game group the connection takes a reference, so the group outlives the creator (`objects` register, games.md section 2). Anything else is only logged. | The clients do not wait (games.md section 2, CONFIRMED live). Harmless. | DONE |
| 31/1 | `GrpGetProp` | LSCI `WaitingRoomGroup::getMembers` | yes | `GetMemberProperties` | Byte 1 is read and not checked (`message/property_query.rs:117`); LSCI sends 1 (`w 0x011F`, messages.md 3.2). Reply 31: one column per offset, one row per member (objects.md section 9). Not a group: no reply. | The class applies the table to every member but the player's own (messages.md 3.2, CONFIRMED). A missing reply leaves those replicas blank. No timeout is cited. **INFERRED**: the room does not block on this one call; it also fetches names with 32. | DONE |
| 32/0 | getProp | LSCI `Obj::getProp`; the waiting room asks a newcomer's name, game and room | yes | `GetProperties` | `SetMsg` 33 for a live object, one record per mirrored property, in order. An unknown SID: no reply (objects.md section 9, `objects/mirror.rs:84`). | State 4 of the Clubhouse login waits until every member's name is known (messages.md 5.4, CONFIRMED). Those names arrive because 14 was mirrored and this reply carries them. No timeout is cited. **INFERRED**: with no `SetMsg` the room stays in that state. | DONE |
| 50/0 | playing | RB at game start. Golf's builder is dead | part | `PlayStatus` | State 0 or 1 only; any other state is `UnknownValue` and not decoded (`message/play_status.rs:18`). Logged, no reply (`player.rs:134`). | No reply is read (redbaron.md section 10 item 11, CONFIRMED). Harmless. | DONE |
| 50/1 | leaving | RB on a clean exit and on a fatal error. Golf's builder is dead | part | `PlayStatus` | Same as 50/0, `playing` false. | Same: no reply is read. Harmless. | DONE |
| 51/0 | `ObjNewList` | LSCI SierraLand `placeMen`, PaintBall | yes | no parser | Command 51 is not in `Command`. | The client has a receive class (messages.md 3.2, CONFIRMED layout). PaintBall was not driven (games.md section 8). **INFERRED**: `placeMen` waits for the host's list the way joinNet waits for `ObjID`. | GAP |
| 52/0 | `ObjFreeList` | LSCI `PaintBall::dispose` | yes | no parser | Command 52 is not in `Command`. | Sent from `dispose` (messages.md 3.2, CONFIRMED). **INFERRED**: like leaveNet, the sender does not wait; the H→C form is how a peer would hear the free. Not established. | GAP |

### 1.2 Host services

| Cmd/sub | Name | Senders | Live? | Decoded | Behaviour | If nothing comes back | Status |
|---|---|---|---|---|---|---|---|
| 27/0 | BBS post | Ysv, Twn post. LSCI also sends sub 0 (post/status, and conference/trivia with classes 67 and 84) | yes | `Bbs` `Post` | RPG shape only (`message/bbs.rs:144`). A post is stored and not answered (`bbs.rs:90`). Outside a land, a post still gets nothing (`bbs.rs:207` skips `Post`). | RPG does not wait (yserbius.md section 3, CONFIRMED). LSCI's own 27 is a different layout and "not decoded yet" (messages.md 3.2.1). A body that is not the RPG field widths fails the parse or is stored as an RPG post. **INFERRED**: the LSCI screens do not get the `BBSMsg` they read. | DONE |
| 27/1 | BBS read | Ysv, Twn read step "this". LSCI read, and conference/trivia 27/1 | yes | `Bbs` `Read` | RPG reply is always sub 1 (yserbius.md section 3). | RPG waits; a Nak 27 ends the 60 s wait with "BBS Server offline." (yserbius.md 4.2, CONFIRMED). LSCI: same caveat as 27/0. | DONE |
| 27/2 | BBS next | Ysv, Twn, only while the last-message flag is clear | yes | `Bbs` `Read` | Reply sub 1, number stepped (`bbs.rs:190`). | Same RPG wait as 27/1. | DONE |
| 27/3 | BBS previous | Ysv, Twn, only while the first-message flag is clear | yes | `Bbs` `Read` | Reply sub 1. | Same RPG wait as 27/1. | DONE |
| 27/4 | BBS first | Ysv, Twn, when a section opens | yes | `Bbs` `Read` | Reply sub 1, message 1. | Same RPG wait as 27/1. | DONE |
| 27/5 | BBS last | Ysv, Twn | yes | `Bbs` `Read` | Reply sub 1, last message. | Same RPG wait as 27/1. | DONE |
| 27/6 | BBS | LSCI `BitchToTSNMI` complaint | yes | no parser | Sub 6 is not a `ReadStep`, so parse returns `UnsupportedSub` (`message/bbs.rs:126`). | The complaint script is named (messages.md 3.3, CONFIRMED) and its wait is not. **INFERRED**: the dialog does not complete. | GAP |
| 27/8 | BBS | LSCI, sub seen with 6 and 9 | yes | no parser | `UnsupportedSub`, as 27/6. | Wait not stated (messages.md 3.3). | GAP |
| 27/9 | BBS | LSCI complaint family | yes | no parser | `UnsupportedSub`, as 27/6. | Wait not stated (messages.md 3.3). | GAP |
| 27/13 | BBS directory | Ysv, Twn list. LSCI `script.199` list | yes | `Bbs` `Directory` | RPG reply sub 13, 17-byte records and names (yserbius.md section 3). An unknown location is an empty directory. Outside a land, or a store fault: Nak 27 (`bbs.rs:207`). | RPG waits, 60 s (yserbius.md 4.4, CONFIRMED). LSCI's list is not a separate parser (messages.md 3.2.1). | DONE |
| 27/14 | BBS index | Ysv, Twn. LSCI `script.199` | yes | `Bbs` `Index` | RPG reply sub 14, message numbers from 1 (yserbius.md section 3). | Same RPG wait as 27/13. LSCI: same caveat. | DONE |
| 34/4 | rates | LSCI at logon, unless `ShutUp` | yes | no parser | Command 34 is not in `Command`. | Logon does not wait. A capture left 34/4 unanswered and the client reached the map (messages.md 4.2, captures.md section 9, CONFIRMED). The rates text is shown only when a reply arrives, and the client then sends 34/8 (messages.md 3.3, CONFIRMED). Harmless at logon. | GAP |
| 34/8 | rates seen | LSCI, after a 34/4 text | yes | no parser | Not in `Command`. | It is the client's own ack of the rates text (messages.md 3.3, CONFIRMED). This host never sends that text (section 2), so the client does not reach this send on the logon path. Harmless. | GAP |
| 36/1 | HOSTADDR file | LSCI at logon, with the stamp from `hostaddr.tim` | yes | `HostInfo::HostAddressFile` | Absorbed. "no reply: the client keeps what it has" (`player.rs:251`). | The client keeps its file and does not block logon (messages.md 4.2 step 7, captures.md section 9, CONFIRMED). Harmless. | DONE |
| 36/2 | host time | RB live. Ysv, Twn and Golf builders dead | part | `HostInfo::HostTime` | `HostInfo` type 2, six time bytes from `World.clock` (`player.rs:244`). | RB: fatal "get host time timeout" at 119.6 s (redbaron.md 6 step 6 and 8.2, CONFIRMED). Golf never asks (golf.md 9 item 13, CONFIRMED). | DONE |
| 36/5 | host number | LSCI at logon, and again on a land arrival | yes | `HostInfo::HostNumber` | `HostInfo` type 5, the world's host number (`player.rs:243`). | Logon state 4 waits on the land directory, not on this (messages.md 4.2, CONFIRMED). The number is what a later land switch compares (messages.md 5.2, CONFIRMED). With the reply, that compare works. | DONE |
| 36/6 | land directory | LSCI at logon, stamps from `landaddr.tim` | yes | `HostInfo::LandDirectory` | The stamp is not compared. Reply is `WaitGrpRequest` msgType 2 for the catalog (`player.rs:245`). | State 4 waits for that reply or a Nak 36/6. Without either, logon stays there (messages.md 4.2 steps 7 and 8, CONFIRMED). | DONE |
| 37/17 | boxes of an account | LSCI Mail Room | yes | `Mail` `Boxes` | The account's one box, or a Nak. Another account's number is fault 12 (mail.md section 2, `mail.rs:114`). | Until this reply existed the Mail Room said "You do not have a mail box attached to this ID" (mail.md section 7, CONFIRMED live). With it, the room opens. | DONE |
| 37/18 | new mail | LSCI at logon once `mail.cfg` exists, and from the map icon | yes | `Mail` `NewMail` | Status word: bits 0 and 1 while the box holds any letter (mail.md section 5). | Logon does not wait on it (messages.md 4.2, CONFIRMED). The icon stays off without the reply. Not a hang. | DONE |
| 37/19 | list a box | LSCI Mail Room | yes | `Mail` `List` | Up to 64 rows (mail.md section 3). | The empty box was shown live after this reply (mail.md section 7, CONFIRMED). **INFERRED**: without it the list does not draw. | DONE |
| 37/20 | send | LSCI letter, and a short sysop form | yes | `Mail` `Send` for an envelope; the short form is `Truncated` | Envelope: stored, Ack 37/20 with the receiving box. The six-word sysop form (`script.145` exports 7 and 8) fails the parse and gets nothing (mail.md section 2, CONFIRMED). | The letter sender shows "Letter received by box N." from the Ack (mail.md section 3, CONFIRMED). **INFERRED**: no Ack leaves that dialog up. The short form's wait is not stated. A new account has no stamps flag `0x200`, so the client refuses to send until the record has it (mail.md section 7); that is the login Ack, not this command. | DONE |
| 37/21 | send (claimed) | LSCI, same two shapes as 37/20 | yes | `Mail` `Send`; short form `Truncated` | Same as 37/20 with sub 21. | Same as 37/20. | DONE |
| 37/24 | send | LSCI compose | yes | `Mail` `Send` | Stored, Ack 37/24. | Same Ack text as 37/20 (mail.md section 3, CONFIRMED). | DONE |
| 37/25 | read | LSCI Mail Room | yes | `Mail` `Read` | The letter's text, or Nak fault 14. | The reader shows that text (mail.md section 7, the transcripts). **INFERRED**: silence leaves the read dialog empty. | DONE |
| 37/26 | delete | LSCI | yes | `Mail` `Delete` | Deleted, no reply. Nak only when the letter is missing (mail.md section 2). | No success reply is defined (messages.md 3.3, mail.md section 2). Harmless. | DONE |
| 37/27 | forward | LSCI | yes | `Mail` `Forward` | Copied, Ack 37/27. The client's note is not delivered (mail.md section 5). | The client says "Letter successfully forwarded." from that Ack (mail.md section 3, CONFIRMED). | DONE |
| 37/30 | service form | LSCI `"AcctUpdate"`, `"MemberServ"` | yes | `Mail` `Form` | Logged and dropped. No reply, including when the store is down (`mail.rs:92`; `MailRequest::refused` returns `None` for a form, `message/mail.rs:257`). | mail.md section 2 specifies nothing back. Whether the form screen blocks is not stated. The request is never carried out (mail.md section 8). | STUB |
| 37/31 | services | LSCI | yes | `Mail` `Services` | A listing reply (sub 19) with no rows (`mail.rs:104`). | A reply does arrive, so the client is not left in silence. The list is empty on purpose (mail.md section 8, INFERRED that the original list was not). The window shows no services. | STUB |
| 37/32 | system list | LSCI at logon | yes | `Mail` `SystemList` | Reply 32 with no rows. | An empty list is what the logon handler consumes, and logon continued live (mail.md section 7, messages.md 4.2, CONFIRMED). | DONE |
| 38/sub | `HintMsg` | LSCI hint room `script.135` | yes | no parser | Command 38 is not in `Command`. The sub-command byte is in the layout and no value is published (messages.md 3.3). | The room is named. No timeout is cited. **INFERRED**: a hint request that expects `b which @4` and the bytes at `@5` does not show the hint. | GAP |
| 40/2 | save money | LSCI CasinoLand, when it writes the bankroll | yes | `SaveMoney` | Stored, then Ack `whichCmd` 40 `whichSub` 2. A store fault is a Nak naming 40 (`money.rs:35`). | `MoneyLongInt::dispose` waits for an Ack or a Nak naming command 40, then gives up after about 420 `Wait(1)` turns (messages.md 3.3.1, CONFIRMED). Silence is a long stall, not a permanent hang. The Ack is sent. | DONE |
| 40/3 | read money | LSCI CasinoLand, only when `ll$$<account>.dat` failed to open | yes | `ReadMoney` | `HostMessage::Money`: command 40, sub 3, the balance at words 8 and 10. An account with no row gets 1,000,000. A failing store gets silence (`money.rs:18`). | The client applies the reply only when it has no local file (messages.md 3.3.1, CONFIRMED). Silence keeps the client's own seed. Not a hang. | DONE |
| 40/4 | set name | LSCI at logon and on entering a land; Ysv and Twn with the character name | yes | `SetPersona` | The session persona is renamed. Nothing is written to the store. No reply (`player.rs:86`). | No reply is read. The live client carried on (messages.md 3.3, captures.md 6.185, CONFIRMED). Ysv and Twn do not wait (yserbius.md section 2, CONFIRMED). Harmless. | DONE |
| 41/0 | name check | LSCI `GotoConference::init` | yes | `ObjExists::Name` | Logged "decoded, no handler yet: ignored" and no reply (`player.rs:152`). `rpg` and `red_baron` answer only a service lookup. | The client reads an H→C body at byte 6 (messages.md 3.3, CONFIRMED). No timeout is cited. What a free name looks like is an open question (fanout-plan.md section 4). **INFERRED**: conference entry waits on this the way the DOS 41/2 loop waits on command 41. | STUB |
| 41/2 | service lookup | Ysv and Twn land type `0x8B`; RB type 101. Golf's builder is dead | part | `ObjExists::Service` | Command 41, SID at word 6. RPG returns `Sid(0x008B)` (`rpg.rs:67`). RB returns `Sid(0x0065)` (`red_baron.rs:23`). Any other land type, including a name check, is the 41/0 path. | Ysv/Twn resend every 60 s forever until command 41 arrives (yserbius.md 4.3 step 2, CONFIRMED). RB is fatal at 119.6 s (redbaron.md 6 step 1, CONFIRMED). The SID values are host choices; neither client reads the SID back as a real object (yserbius.md section 2, INFERRED `RPG_SERVICE_SID_ASSUMED`). | DONE |
| 44/2 | new password | LSCI, after login status 11 or from the password UI | yes | `ChangePassword` | Byte 1 is not checked (`message/client.rs:316`); the client sends 2 (messages.md 3.3). Stored, expired flag cleared, Ack 44/1 to the SID. A refusing store gets a Nak (`logon.rs:28`). The open book acks and stores nothing (innkeeperd.md section 5). | The client stores the password in `PASS_SET.DTA` only on Ack 44/1 or 44/2 (messages.md 3.3, CONFIRMED). **INFERRED**: silence leaves the "password is out of date" path unfinished. | DONE |
| 45/1 | assign mailbox | LSCI at logon when `mail.cfg` is absent | yes | `NewMailbox` | Only sub 1 parses (`message/mail.rs:627`). Status 1 and the box number from the store. The open book answers status 2; a failing store answers status 4 (`mail.rs:27`). | Status 1 is written to `mail.cfg` (mail.md section 3, CONFIRMED live). Status 2 to 6 are errors by number; 4 is "busy" and the client tries again next logon. Logon does not wait (messages.md 4.2, mail.md section 6, CONFIRMED). | DONE |
| 47/1 | land occupancy | LSCI after the directory, and every 60 s (20 s on the map) | yes | `LandOccupancyRequest` | Only sub 1 parses (`message/client.rs:172`). `WaitGrpRequest` msgType 1. `current` is the waiting room's member count; `maximum` is the catalog's 64 (`player.rs:76`). | State 6 waits for this or a Nak 47/1 (messages.md 4.2 step 10, CONFIRMED). Silence holds logon. Later polls refresh the map; a missing land is drawn unavailable (messages.md section 6, CONFIRMED). | DONE |
| 49/1 | find by account | LSCI | yes | no parser | Command 49 is not in `Command`. | H→C is `body @4` and the field layout is unknown (messages.md 3.3, fanout-plan.md section 4, CONFIRMED that it is unknown). Whether the finder blocks is not stated. | GAP |
| 49/2 | find by name | LSCI | yes | no parser | Not in `Command`. | Same as 49/1. The request carries `w 0, s name` (messages.md 3.3, CONFIRMED). | GAP |
| 53/0 | Login | LSCI, first message after Connect. Shop links a sender that nothing calls | part | `Login` | Land type must be in the catalog, then the account book. Ack `whichCmd` 22 (flags, status, rating), or Nak 22 reason 9 (bad password, three tries), reason 1 (unknown account or a bad store), reason 2 (unknown land) (`logon.rs:15`, `player.rs:215`). A second Login releases the connection's objects first (`player.rs:55`). | 70 s, then error 999 and the session ends (messages.md 4.2, CONFIRMED). A Nak reason other than 9 shows error 100 + reason and hangs up. | DONE |
| 54/24 | TEXT | Shop, one typed line or a button command | yes | no parser | Command 54 is not in `Command`. | Any handled message within 636 ticks (about 35 s), or the modal "Send Time-Out" and a return to the map (shopadv.md section 9 step 4, CONFIRMED). Button commands do not start that timer. | GAP |
| 54/25 | JOIN | Shop, after the first dialog, and again when the host sends 54/19 kind `0x1B` | yes | no parser | Not in `Command`. | No timeout and no retry: silence hangs the client (shopadv.md section 9 step 2, CONFIRMED). | GAP |
| 54/26 | EXIT | Shop, once, at the end of `main` | yes | no parser | Not in `Command`. | A clean exit waits forever for 54/19 kind `0x1A` (or 54/20 kind `0x1A`). After an error the wait is at most 90 ticks (shopadv.md section 9 steps 5a and 5b, CONFIRMED). | GAP |
| 54/27 | STATUS / mall gate | LSCI `gotoCUCmall` before it launches Shop. Shop sends the same bytes after an Ack whose `whichCmd` is 22 | yes | no parser | Not in `Command`. Shop may not see the land's old login Ack: queued messages are dropped at a program switch (shopadv.md section 11 item 8, INFERRED). LSCI's send is the one that always happens. | LSCI leaves for Shop on 54/19 and shows the text from byte 15 on 54/20 (shopadv.md section 7.3 and section 9 step 0, CONFIRMED). No timeout is cited for that waiter. **INFERRED**: the mall entry stays on its wait. | GAP |
| 55/1 | signup | LSCI `MakeConnection` | yes | no parser | Command 55 is not in `Command`. Absent in TSN 2.1 (messages.md section 10, CONFIRMED). | Ack 55/1 carries the new account number at words 6 and 8 (messages.md 3.3, CONFIRMED). The failure mode with no Ack is not stated. **INFERRED**: signup does not finish. | GAP |
| 59/0 | Login, Prodigy id | LSCI when `LSCI.CFG` has `pFlag` | yes | `Login` | Same policy as 53. The extra string is stored on the `Login` and not read by `admit`. | Same 70 s timeout as 53 (messages.md 4.2, CONFIRMED). | DONE |

On the open book (no `--data-dir`) every 37 request except 37/30 is a Nak with fault 21, and 45/1 is status 2 (mail.md section 6, CONFIRMED). Those are still replies. Logon completes. The rows stay DONE.

Eight of the command-27 rows are DONE for Yserbius and Twinion only. LSCI's 27/0, 27/1, 27/13 and 27/14 are the same sub-commands with another layout (messages.md 3.2.1 and 3.3, CONFIRMED). They are not given a second row; they are a gap in section 3.

Sub 5 of command 34 (account status, a string at byte 4) matches the host-to-client layout. No client send site is cited. **INFERRED**: it is host-to-client only, so it is in section 2 and not in the table above. The detailed-summary reply (`w len @16`, text @18, `GetDetailedInfo`) has no sub-command number in the catalogue (messages.md 3.3). No row.

## 2. Host to client

These are the commands a client dispatches on. "Sends?" is whether any production path builds that `HostMessage`. `HostMessage::Notice` has an encoder (`message/host.rs:181`) and no constructor outside the parser: command 48 is never sent. The same is true of every command that is not in the `Command` enum.

| Cmd/sub | Name | Who handles it | Sends? | Evidence |
|---|---|---|---|---|
| 0 | `AckMsg` | LSCI matches `whichCmd` 7, 22, 34, 36, 37, 40, 44, 47, 55 (messages.md 3.1). DOS games act on 4 and 27; other Acks are ignored (yserbius.md 4.2). Shop sends 54/27 when `whichCmd` is 22 (shopadv.md 7.2). | Yes. Login (`whichCmd` 22, `player.rs:227`); lock 4 (`objects/locks.rs:58`); password 44/1 (`logon.rs:37`); money 40/2 (`money.rs:44`); mail 37/20, 37/21, 37/24, 37/27 (`message/mail.rs:561`). | CONFIRMED |
| 1 | `NakMsg` | LSCI shows the text; reason 9 retries Login (messages.md 4.2). DOS games: Nak 10 is a join error, Nak 27 is "BBS offline", other `whichCmd` values are "TSN NAK Error." (yserbius.md 4.2). Shop ignores a Nak (shopadv.md 7.2). | Yes. Login (`logon.rs:21`); lock (`objects/locks.rs:62`); password (`logon.rs:45`); money (`money.rs:53`); GrpJoin (objects.md 4.1); mail (`mail.rs:59`); BBS offline (`bbs.rs:214`). | CONFIRMED |
| 2 | `Send` | Every program. LSCI delivers it to the object. DOS games switch on the type byte (census section 4 of each game doc). | Yes, as a relay (`router.rs:58`). The host never writes a game payload. | CONFIRMED |
| 8 | `ObjID` | LSCI registers the cookie. Ysv and Twn assign it to the pending create, ignoring the cookie (yserbius.md 4.2). Golf checks the cookie (golf.md 6.2). RB takes the first (redbaron.md 5.2). Shop parses it and does nothing (shopadv.md 7.2). | Yes, the reply to joinNet (`objects.rs:217`). | CONFIRMED |
| 9 | `ObjFree` | LSCI disposes the object (messages.md 3.2). Ysv, Twn, Golf and RB ignore it (each census 4.2 / 6.2 / 5.2). | Yes, to peers when the last reference drops (objects.md 4.2). Not to the sender of leaveNet. | CONFIRMED |
| 10 | `GrpJoin` | LSCI `addMember`. DOS games count it only when `who` is themselves, and Golf/RB also count other members (golf.md 6.2, redbaron.md 5.2). | Yes. The joiner's echo, the other members, and one extra `GrpJoin` per existing member to an LSCI newcomer (`presence.rs:62`). | CONFIRMED |
| 11 | `GrpDel` | LSCI removes the member. DOS games drop the remote player (yserbius.md 4.2, golf.md 7.3, redbaron.md 5.2). | Yes, to the connections that still have a member (objects.md section 4). | CONFIRMED |
| 12 | `GrpMem` | LSCI and Golf and RB install the list. Ysv and Twn ignore it. | Yes, the reply to 12 (`objects.rs:305`). | CONFIRMED |
| 13 | `SetIntMsg` | LSCI applies every pair. Ysv/Twn read the third pair's value as a step delay if it is 12 to 48 (yserbius.md 4.2). RB reads only the first pair, and only on the service SID (redbaron.md 5.2). Shop parses it and does nothing. | Yes. Relayed from a client's 13 (`router.rs:71`), and built for RB's 17/0 (`red_baron.rs:56`). The RPG step-delay push is never sent (yserbius.md section 2, CONFIRMED). | CONFIRMED |
| 14 | `SetStrMsg` | LSCI applies it. The DOS handlers are empty (yserbius.md 4.2, golf.md 6.2, redbaron.md 5.2). | Yes, relayed from a client's 14 (`router.rs:83`). | CONFIRMED |
| 23 | `GameRights` | LSCI has the class and no script handler (messages.md 3.3 and 11). | No. | CONFIRMED |
| 25 | `RMI` | LSCI runs the method on the local copy (messages.md 3.2). | Yes, relayed from a client's 25 (`router.rs:98`). Never invented by the host. | CONFIRMED |
| 27 | `BBSMsg` | Ysv and Twn switch on byte 1 (subs 13, 14, 1, or an Ack `whichCmd` 27). LSCI's class reads a different body (messages.md 3.3). | Yes, the RPG replies from `bbs.rs:64`. Not the LSCI shape. | CONFIRMED |
| 29 | `GameVersion` | LSCI class, no script handler (messages.md 11). DOS games do not list it. | No. | CONFIRMED |
| 31 | `GrpGetProp` | LSCI applies the table (messages.md 3.2). | Yes, the reply to 31 (objects.md section 9). | CONFIRMED |
| 33 | `SetMsg` | LSCI applies the records. Any other record type does not shorten the body, so the loop would not end (messages.md 3.2, CONFIRMED). | Yes, the reply to 32. Record types are only 0 and 1 (`message/property_query.rs:13`). | CONFIRMED |
| 34 | `AccountMsg` | LSCI shows sub 4 when the text changed, sub 5 as account status, and the long summary from `GetDetailedInfo` (messages.md 3.3). | No. | CONFIRMED |
| 36/0 | `HostInfo` | Golf's sub 0 handler is empty (golf.md 6.2). | No. | CONFIRMED |
| 36/1 | `HostInfo` file | LSCI saves the bytes as HOSTADDR (messages.md section 6). | No. The 36/1 request is absorbed (section 1.2). | CONFIRMED |
| 36/2 | `HostInfo` time | RB stores all six bytes; the month and day are the scenario date (redbaron.md 8.2). Golf's dead sender has the same reader (golf.md 6.2). Ysv and Twn ignore type 2 (yserbius.md 4.2). | Yes, the reply to 36/2 (`player.rs:244`). | CONFIRMED |
| 36/5 | `HostInfo` host number | LSCI stores the word at byte 2 (messages.md 3.3). | Yes, the reply to 36/5 (`player.rs:243`). | CONFIRMED |
| 37 | `EMMsg` | LSCI Mail Room and `RoomZeroHandler` (mail.md section 3). | Yes: subs 17, 18, 19, 25, 32 and the Acks and Naks in section 1.2. Not a push when a letter arrives (mail.md section 8). | CONFIRMED |
| 38 | `HintMsg` | LSCI hint room (messages.md 3.3). | No. | CONFIRMED |
| 39 | `HostFile` | LSCI class, no script handler (messages.md 11). | No. | CONFIRMED |
| 40 | `UserInfoMsg` | CasinoLand reads sub 3, and only when the local file did not open (messages.md 3.3.1). | Yes, the reply to 40/3 (`money.rs:28`). No reply is defined for 40/4. | CONFIRMED |
| 41 | `ObjExists` | Ysv, Twn and RB store the word at byte 6 and end the lookup (yserbius.md 4.2, redbaron.md 5.2). LSCI's name check reads a body at byte 6 (messages.md 3.3). Golf's handler is empty (golf.md 6.2). | Yes, for 41/2 of land type `0x8B` or 101 (`rpg.rs:71`, `red_baron.rs:27`). Not for 41/0. | CONFIRMED |
| 45 | `NewBoxHandler` | LSCI writes status 1 to `mail.cfg` (mail.md section 3). | Yes, the reply to 45/1 (`mail.rs:47`). | CONFIRMED |
| 47/1 | occupancy | LSCI `ProcessLandInfo` (messages.md section 6). | Yes, the reply to 47/1 (`player.rs:78`). | CONFIRMED |
| 47/2 | directory | LSCI logon state 4. This is the reply to **36/6**, not to a 47 request (messages.md 3.3 and 4.2). | Yes (`player.rs:245`). | CONFIRMED |
| 48 | `Unsolicited` | LSCI `RoomZeroHandler`. Every DOS game shows the text (census README section 4). Shop shows "INN Notice:" (shopadv.md 7.2). RB shows it only once the game is running (redbaron.md 5.2). | No. The encoder exists; nothing constructs a `Notice`. | CONFIRMED |
| 49 | `UserFindMsg` | LSCI, body at byte 4, layout unknown (messages.md 3.3). | No. | CONFIRMED |
| 51 | `ObjNewList` | LSCI PaintBall, word at byte 4 used as an object handle (messages.md 2.1). | No. | CONFIRMED |
| 52 | `ObjFreeList` | LSCI, word at byte 2 used as an object handle (messages.md 2.1). | No. | CONFIRMED |
| 54/16 | OUTPUT | Shop: kind 1 appends text, kind 2 is "Mall Down" and quits, any other kind is "Mall Notice:" (shopadv.md 7.2). | No. | CONFIRMED |
| 54/17 | JOIN OK | Shop stores three words and goes idle (shopadv.md 7.2). | No. | CONFIRMED |
| 54/18 | JOIN FAILURE | Shop: modal, then quit with the error flag (shopadv.md 7.2). | No. | CONFIRMED |
| 54/19 | CONTROL | Shop: kind `0x1A` acknowledges exit, kind `0x1B` sends JOIN again. LSCI `gotoCUCmall` treats 54/19 as "go" and starts Shop (shopadv.md 7.2 and 7.3). | No. | CONFIRMED |
| 54/20 | NAK | Shop quits, with a dialog for kinds `0x18`, `0x19` and `0x1B`. LSCI shows the text from byte 15 as " S.A. Unavailable " (shopadv.md 7.2 and 7.3). | No. | CONFIRMED |
| 147 | `SysOpCmd` | LSCI delivers it to the SysOp object (messages.md 2.1 and 3.3). | No. | CONFIRMED |
| 149 | raw text | Shop, only when a mode word is nonzero, which never happens (shopadv.md 7.2, CONFIRMED dead). | No. | CONFIRMED |

Commands 23, 29, 39, 48 and 147 are the unsolicited messages the user list names. None is sent. No cited play path requires them: 48 is operator text, and 23, 29 and 39 have no script handler (messages.md section 11, CONFIRMED). 147's script behaviour beyond "operator commands" is not described.

## 3. Gaps, in the order a player would hit them

The smallest behaviour is the least the client reads, not a guess at the original host. **INFERRED** marks a byte the catalogue does not pin down.

### 3.1 A live client waits, or shows an error

1. **54/25 JOIN** (GAP). Shop hangs. There is no timeout and no retry (shopadv.md section 9 step 2, CONFIRMED). Smallest reply: `54/17` with the three token words at bytes 4, 6 and 8, and `w@2` below `0x1000` (shopadv.md section 7.2 and section 11 item 3, CONFIRMED). Any token value works.
2. **54/26 EXIT** (GAP). A clean exit waits forever for the acknowledgement (shopadv.md section 9 step 5a, CONFIRMED). Smallest reply: `54/19` with kind `0x1A` at byte 12, body at least 13 bytes (shopadv.md section 7.2, CONFIRMED).
3. **54/24 TEXT** (GAP). After 636 ticks Shop shows "Send Time-Out" and returns to the map (shopadv.md section 9 step 4, CONFIRMED). Smallest reply: one `54/16` kind 1, text at byte 13, inside that window (shopadv.md section 11 item 5, CONFIRMED). Any handled message stops the timer; the text is what the screen shows.
4. **54/27** (GAP). LSCI starts Shop only on `54/19`, and shows the byte-15 text on `54/20` (shopadv.md section 9 step 0 and section 11 item 2, CONFIRMED). No timeout is written down. **INFERRED**: the mall gate stays where it is. Smallest reply that satisfies LSCI without running Shop: `54/20`, text at byte 15, the body long enough for that byte (shopadv.md section 11 item 2, CONFIRMED). `54/19` is the reply that launches the program, which then needs items 1 to 3.
5. **55/1 signup** (GAP). The client reads the new account number at words 6 and 8 of Ack 55/1 (messages.md 3.3, CONFIRMED). The failure mode is not stated. **INFERRED**: signup does not finish. Smallest reply: Ack, `whichCmd` 55, `whichSub` 1, the account's low and high words at bytes 6 and 8. The Nak-for-55 path delivers word 2 as a raw object handle (messages.md 2.1, CONFIRMED), so a refusal is not a normal Nak.
6. **41/0 conference name** (STUB). Decoded and ignored (`player.rs:152`). The client reads a body at byte 6 (messages.md 3.3, CONFIRMED). No timeout is cited. **INFERRED**: `GotoConference::init` waits, by analogy with the DOS command-41 loop, and the word at byte 6 is a SID the way `ObjectLocated` already writes it (`message/service_lookup.rs:75`). What a free name looks like is not known (fanout-plan.md section 4). Smallest reply that matches the only documented read: command 41, the looked-up SID at word 6, when the name is taken.
7. **51/0 `ObjNewList`** (GAP). PaintBall `placeMen` sends it and the receive class reads word 4 as an object handle and a body at byte 8 (messages.md 2.1 and 3.2, CONFIRMED). The wait itself was not traced (games.md section 8 did not run PaintBall). **INFERRED**: the script waits for that reply, as joinNet waits for `ObjID`. Smallest reply: command 51 with the caller's cookie at word 4 and one object record per requested object in the body. The record size is not in the catalogue.

### 3.2 Live, and a block is not established

8. **49/1 and 49/2** (GAP). The finder sends them (messages.md 3.3, CONFIRMED). The reply is "body @4" and the project has no field layout (fanout-plan.md section 4, CONFIRMED). No smallest body can be stated without reading `script` that this audit did not re-disassemble. Whether the screen blocks is not stated.
9. **LSCI command 27** (GAP for that sender). Subs 6, 8 and 9 are `UnsupportedSub`. Subs 0, 1, 13 and 14 are parsed only as the RPG board (section 1.2). The LSCI class reads `whichCmd`, a number at word 2, and a body at byte 4 or 18 (messages.md 3.3, CONFIRMED). The complaint and conference scripts are named; their waits are not. **INFERRED**: those screens do not complete. The smallest LSCI reply is a `BBSMsg` of the sub the script switches on. The field values are not in the catalogue, so they are not invented here. The RPG rows stay DONE.
10. **38 hint** (GAP). `script.135` sends `b 38, b sub, w roomSID, b ×4` and the reply class reads `which` at byte 4 and bytes at byte 5 (messages.md 3.3, CONFIRMED). The sub-command value and the wait are not published. **INFERRED**: the hint is not shown. Smallest reply: command 38 with those two fields. The bytes are **INFERRED** to be the hint text; the catalogue does not say.
11. **52/0 `ObjFreeList`** (GAP). Sent from `PaintBall::dispose` (messages.md 3.2, CONFIRMED). **INFERRED**: dispose does not wait, by analogy with leaveNet. The receive class exists so a peer can be told (messages.md 2.1). Smallest behaviour that matches leaveNet: free the listed objects and send command 52 to the peers, word 2 the object handle. **INFERRED**.
12. **34/4 and 34/8** (GAP). 34/4 is unanswered at logon and the client still reaches the map (captures.md section 9, CONFIRMED). The rates text is shown only when it changed, and only then does the client send 34/8 (messages.md 3.3, CONFIRMED). Smallest behaviour that matches today's logon: keep absorbing 34/4. Smallest behaviour that also fills the rates screen: `AccountMsg` sub 4, text at byte 4, and absorb the 34/8 that comes back.
13. **37/30** (STUB). The form is dropped and no reply is defined (mail.md sections 2 and 8, CONFIRMED). A wait is not stated. Smallest behaviour the document already claims is enough: keep dropping it. Doing the Member Services work is out of this audit; the client read that would prove a reply was not found.
14. **37/31** (STUB). A real reply arrives, with no rows (`mail.rs:104`). The client does not sit in silence. The services window is empty (mail.md section 8). Smallest further step, **INFERRED**: the same sub-19 rows the mailbox list uses (mail.md section 3), one per service. The service record's fields are not separated from a letter row in the catalogue.
15. **37/20 and 37/21, short sysop form** (the envelope form is DONE). Six words, `Truncated`, no reply (mail.md section 2, CONFIRMED). The sysop screen's wait is not stated. Smallest behaviour: accept that short body. What it should answer is not in mail.md.

### 3.3 Dead code

These have no live sender. Decoding them would not change a session. The DOS copies of 4, 6, 12, 13, 14, 17, 36/2, 41 and 50 are dead in some programs and live in others; the live row covers them, and they are not repeated here.

| Cmd/sub | Where it sits | Codec | Smallest behaviour if it ever ran |
|---|---|---|---|
| 3/0 rSend | LSCI, never called (messages.md 3.2) | no parser | Unknown. The layout is a `Send` without a type word (messages.md 3.2). **INFERRED**: relay it as a `Send`. |
| 5/0 lockQueue | LSCI, never called | no parser | Unknown. No client read is described. |
| 18/0, 19/0, 20/0, 21/0 | LSCI, never called | no parser | Unknown. "host object-store operations" (messages.md 3.2). No reply is described. |
| 22/0 old login | LSCI, never called; replaced by 53 | no parser | The live Login path. Not worth a second parser. |
| Shop `Element` 7, 13, 30 | **INFERRED** unreached (shopadv.md 7.1) | already decoded for LSCI | Nothing. The live LSCI rows cover the bytes. |
| Shop Login 53 | dead, vtable slot never called (shopadv.md section 6, CONFIRMED) | already decoded | Nothing. Do not send Shop an Ack `whichCmd` 22 unless a 54/27 reply is intended (shopadv.md section 11 item 9, CONFIRMED). |
| Golf/Ysv/Twn dead builders | 4, 6, 13, 14, 17/0, 36/2, 41, 50, and Ysv 17/2 | decoded | Nothing beyond the live rows. Golf does not need host time (golf.md 9 item 13, CONFIRMED). |

## 4. Counts

Client-to-host rows in section 1: **78**.

| Status | Rows |
|---|---|
| DONE | 53 |
| STUB | 3 (37/30, 37/31, 41/0) |
| GAP | 15 |
| N/A | 7 (3, 5, 18, 19, 20, 21, 22) |

The 15 GAPs are 27/6, 27/8, 27/9, 34/4, 34/8, 38, 49/1, 49/2, 51/0, 52/0, 54/24, 54/25, 54/26, 54/27 and 55/1.

Not in those 15, and still open for a live sender: LSCI's command 27 on the subs the RPG parser accepts (section 3 item 9), and the short sysop form of 37/20 and 37/21 (section 3 item 15).

Of the 53 DONE rows, the ones that deliberately send nothing are 6, 9, 11, 13, 14, 17/2, 25, 26, 30, 36/1, 37/26, 40/4, 50/0 and 50/1. Each cites a client that does not read a reply.

Host-to-client rows in section 2: **40**. **22** are sent (0, 1, 2, 8, 9, 10, 11, 12, 13, 14, 25, 27, 31, 33, 36/2, 36/5, 37, 40, 41, 45, 47/1, 47/2). **18** are never sent, including 48, 147, 23, 29 and 39.

A logged-in LSCI client can finish logon, enter the Clubhouse, invite a SierraLand game and play Red Baron on the messages those paths send (games.md sections 5 and 6, yserbius.md section 7, CONFIRMED live for Yserbius, the Clubhouse, Stratego, Graffiti and Red Baron). The holes that stop a player are the mall (54), signup (55), a conference name check (41/0), PaintBall's object list (51) and the finder (49).

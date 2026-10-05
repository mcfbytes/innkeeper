# TSN application messages

The messages the LSCI scripts send and receive through `kTSN`, which is what a replacement host must
speak. Below this layer every message is an opaque, length-prefixed byte string carried in TSNEXEC
frames (`docs/protocol/link-layer.md` section 6.8); the `kTSN` sub-ops that move them are in
`docs/protocol/ktsn.md`. This document covers the message header, the catalog of commands in both
directions, the logon and land-switch exchanges, and the open questions.

The catalog is regenerated with:

```
.venv/bin/python tools/tsn_messages.py work/res            # every set, one report per set
.venv/bin/python tools/tsn_messages.py work/res/inn_feb94  # one set
```

`tools/tsn_messages.py` evaluates every code item symbolically (`tools/lsci_eval.py`) and prints the
receive dispatch, the fields each receive class reads, every `TSN(4, ...)` send with its format string
paired to its values, the connection sub-ops, and every handler that compares a message's `command`,
`msgType`, `whichCmd` or `whichSub` with a constant. The output is deterministic and has no code
offsets, so two sets can be compared with `diff`.

## 0. Conventions

- Scripts are cited as `land/resource method +offset`, where the offset is inside that code item as
  `tools/lsci_disasm.py` prints it, for example `hub/script.101 dialScript::changeState +0x0140`.
  Unless a set is named, the citation is the Feb-1994 set (`work/res/inn_feb94`).
- **C→H** is client to host (transmit), **H→C** is host to client (receive), following
  `docs/CONVENTIONS.md`.
- Layouts give byte offsets in the message body. `b` = byte, `w` = 16-bit little-endian word, `s` =
  NUL-terminated string, `a[n]` = n raw bytes, `…` = to the end of the message. These are the codes of
  the formatted send (`docs/protocol/ktsn.md` section 3.1).
- Class names in parentheses (`AckMsg`, `Send`, ...) are the original names. Every set strips them except
  Dec-1993 SierraLand, whose `type31.559` keeps them, and the tool carries them over by command number.
  CONFIRMED (`inn_dec93/SL/type31.559`, class items).
- SID = the host's 16-bit object identifier. The client maps SIDs to objects with kernel 0x57 `SID`
  (`16b1:0791`, file `0x1A381`), a 6-way jump table at `16b1:07B1` (file `0x1A3A1`): 0 allocates the
  table, 1 `(sid, object)` registers, 2 unregisters, 3 `(sid)` looks up, 4 and 5 return arrays of the
  registered entries. CONFIRMED (jump table; scripts use 0 to 3, for example
  `hub/type31.608 Obj::handleMsg +0x005F` and `hub/type31.558 class_61::lookup +0x0005`).
- `SID(3, 0)` does not search the table: it returns a fixed interpreter variable (word `1FCA` in the
  segment held at `DS:33DC`, code `16b1:0972`..`0982`, file `0x1A562`). CONFIRMED.

## 1. Summary

- CONFIRMED: every message starts with a **command byte** at offset 0. The base receive class `Msg`
  reads it and a **target SID word at offset 2** (`hub/type31.559 class_62::init`); 10 of the 31
  receive classes replace that layout (section 2.1). Byte 1 is a sub-command, a message type or 0.
- CONFIRMED: incoming messages are classified by the command byte alone. `Msg::with` maps 31 command
  values to 31 subclasses; anything else is reported as "Unsupported message." and dropped
  (`hub/type31.559 class_62::with`). The table is identical in every land of every set, except that
  TSN 2.1 and TSN basic lack command 54.
- CONFIRMED: the commands form two families that share one number space:
  1. **Host services**, built with byte-oriented formats (`"bb…"`), byte 1 = sub-command: logon,
     account, mail, host and land tables, bulletin boards, user lookup. Commands 22 to 59.
  2. **Shared objects**, built with word formats (`"w…"`), byte 1 = 0 (or a flag): create and free
     networked objects, groups, property updates, remote method calls, and `Send`, the routed
     object-to-object message that carries all chat and game traffic. Commands 2 to 33.
- CONFIRMED: **the client speaks first.** Right after Connect it sends Login (command 53, or 59 for an
  account with a Prodigy ID) and starts a 70-second timeout. The host answers with `AckMsg` for
  command 22 (not 53) or with `NakMsg`. The live client accepted such an Ack and went on to the map and
  the Clubhouse (`docs/protocol/captures.md` sections 9 to 11).
- CONFIRMED: a game's own traffic is `Send` (command 2) to the SID of the game's group object, with a
  game-specific `msgType` word. The host relays it; it never interprets game state.
- CONFIRMED: changing land is "write the 256-byte shared block, name the next program, exit". The new
  LSCITV reads the block back, frees the previous land's object, switches X.25 host only when the land
  lives on another host, and then logs in again.
- INFERRED: the host is a message router plus a small object store (SIDs, groups, locks, property
  values), with service handlers for accounts, mail, boards and the land directory. This matches the
  "Pseudo Host plus virtual message router" description in `docs/PLAN.md` section 1.5.

## 2. Message header and routing

### 2.1 Base fields

| Field | Offset | Read by | Status |
|---|---|---|---|
| `command` | b@0 | `Msg::init` (`class_62::init`), also every "own layout" class | CONFIRMED |
| `toSID` | w@2 | `Msg::init`; `to = SID(3, toSID)` through `class_61::lookup` | CONFIRMED |
| byte 1 | b@1 | per command: `whichCmd` (sub-command echo), `msgType`, `type`, or unread | CONFIRMED |

Every class ends with a `to` object, which is where the message is delivered (section 2.2). The tool
prints it as "routed to". CONFIRMED (`hub/type31.559`, the `init` of each class):

| `to` comes from | Classes |
|---|---|
| `SID(3, w@2)` (base `Msg::init`) | all classes that call `Msg::init` and do not override `to`, plus `HintMsg` (38), `WaitGrpRequest` (47) |
| `SID(3, w@4)` | `BBSMsg` (27) |
| `SID(3, w@1)` | `GameVersion` (29) |
| `SID(3, w@10)` | `PMSMsg` (54) |
| `SID(3, 0)` | `UserInfoMsg` (40), `Unsolicited` (48) |
| the game object (global 2) | `HostInfo` (36), `HostFile` (39) |
| the SysOp object (global 89) | `SysOpCmd` (147) |
| w@2 used directly as an object handle | `ObjID` (8, the cookie from joinNet), `NakMsg` when `whichCmd` is 55, `ObjFreeList` (52) |
| w@4 used directly as an object handle | `ObjNewList` (51) |

Replies addressed to SID 0 reach the game object, so `CC` and through it the current room and
`RoomZeroHandler` (`hub/script.011`), whose name fits. This matters before the game object has a SID:
the login Ack (section 4.2) can only be delivered with `toSID` 0. CONFIRMED live: an Ack, a `HostInfo`
and two `WaitGrpRequest` replies with `toSID` 0 all reached `Dialing` (`docs/protocol/captures.md`
section 9). That the variable behind `SID(3, 0)` is the game object itself remains INFERRED.

Several classes call `move` in `init` to drop their header, so offsets used later by handlers are
relative to the new start ("body rebased" in the tool output). For example `Send` rebases at 8, so a
game handler's `at: 0` is message byte 8. CONFIRMED (`class_63::init +0x0029`).

### 2.2 How a message reaches a script

1. The `kTSN` pump posts a `0x400` event (`docs/protocol/ktsn.md` section 4.1).
2. The network user object `class_87` (`hub/type31.565`, installed by `hub/script.101 proc_7`) takes
   network events first (`class_87::handleEvent +0x0030`, `nextEvent: 0x400`) and builds the message
   with `Msg with: data len` (`+0x0061`).
3. `Send` with `msgType` 1 goes to the chat hook `passChatTo`; other messages go to `passGameTo` or
   `passSystemTo` when those are set. Unclaimed messages are queued (`putMsg`). CONFIRMED (`+0x00A3`..`+0x0123`).
4. A message whose `to` is not an object is disposed at once (`class_87::handleEvent +0x0133`).
   `class_87::handleMsg` takes queued messages, resolves `from = SID(3, fromSID)` for `Send`, resolves
   `to = SID(3, toSID)` again, and calls `to handleMsg: msg` (`+0x0062`..`+0x00B6`). CONFIRMED.
5. Messages delivered to the game object reach `CC::handleMsg` (`hub/script.000`). It offers each
   one to its superclass and to the current room (global 3, for example `Dialing` during logon), then routes chat (`Send` msgType 1 and 36) to `script.005` export 2, and system replies
   (`Unsolicited`, `AccountMsg`, `NewBoxHandler`, `EMMsg` 18/32, Ack/Nak for 37/40/44) to
   `script.011 RoomZeroHandler`, and land occupancy (`WaitGrpRequest` msgType 1) to `ProcessLandInfo`.
   CONFIRMED (`CC::handleMsg +0x002A`..`+0x016B`).

The `notify` property of `class_61` is ORed into the command word of every `class_61` send. No script
sets it, so byte 1 of those messages is 0. CONFIRMED (`hub/type31.558` class item, no `notify:` sender);
its purpose is unknown.

## 3. Catalog

Status applies to the layout. "Meaning" is INFERRED from names, strings and handler behaviour unless it
says otherwise. Sender and receiver scripts are examples; the tool lists them all.

### 3.1 Replies

| Cmd | Name | Dir | Layout | Meaning | Status |
|---|---|---|---|---|---|
| 0 | `AckMsg` | H→C | `b 0, b ?, w toSID, b whichCmd @4, b whichSub @5, …` | positive reply; `whichCmd`/`whichSub` repeat the request's command and sub-command, in both families (Ack/Nak handlers test 7, 22, 34, 36, 37, 40, 44, 47 and 55) | CONFIRMED layout; echo CONFIRMED by handlers |
| 1 | `NakMsg` | H→C | as `AckMsg`, plus `b numTries @7`; text from 7 or 8 in some handlers | negative reply. For `whichCmd` 55 the target is w@2 used directly as an object (no SID lookup) | CONFIRMED (`class_65::init`) |

Reply payloads after byte 5 are per request; the known ones are in sections 4 and 5. A lock failure
puts the holder's SID at w@5 (`hub/script.121 inviteScript::handleMsg +0x00AD`). CONFIRMED.

### 3.2 Shared objects (word formats)

| Cmd | Name | Dir | Layout | Sender / receiver | Meaning | Status |
|---|---|---|---|---|---|---|
| 2 | `Send` | both | `w 2, w toSID, w fromSID, w msgType, …body` | `class_61::send`/`sendArray`, `script.005` export 0 (chat); receivers' `handleMsg` | routed message to the object (or group) `toSID`. `fromSID` may be 0. All chat and game moves | CONFIRMED |
| 3 | rSend | C→H | `w 3, w toSID, w fromSID, …` | `class_61::rSend` (never called) | unknown variant of `Send` | CONFIRMED layout, unused |
| 4 | lock | C→H | `w 4, w fromSID, w lockId, w…` or `…, a[n]` | `class_61::lock`, invite scripts | take a named host lock; Ack on success, Nak with holder SID at w@5 | CONFIRMED |
| 5 | lockQueue | C→H | `w 5, w toSID, w fromSID` | `class_61::lockQueue` (never called) | unknown | CONFIRMED layout, unused |
| 6 | unlock | C→H | as lock | `class_61::unlock`, `LL/script.905 inviteRoom::init` | release a lock | CONFIRMED |
| 7 | joinNet | C→H | `w 7, w 0, w cookie, b kind, b landType, w param, w size` (12 bytes) | `Obj::joinNet`, `GameGroup::joinNet`, `WaitingRoomGroup::joinNet` | create a networked object; `cookie` is the client's object handle, `kind` is one of 1 to 5 or 129 (the role names and per-land meaning are in section 3.2.1) | CONFIRMED layout; kinds from callers; 129, 1 (param `0xFFFF`, size 30) and 5 seen live |
| 8 | `ObjID` | H→C | `b 8, b ?, w cookie @2, w ? @4, w sid @6` | `class_66::init`; `Obj::handleMsg +0x003F` | reply to joinNet: the object whose handle is `cookie` gets SID `sid` and registers it with `SID(1, …)` | CONFIRMED, live with bytes 1 and 4..5 zero |
| 9 | leaveNet / `ObjFree` | both | C→H `w 9, w sid, w 0`; H→C header only | `Obj::leaveNet`; `Obj::handleMsg +0x0068` | free an object; on receive the target object disposes itself | CONFIRMED |
| 10 | add / `GrpJoin` | both | C→H `w 10, w groupSID, w memberSID` (+ `b major, b minor, b revision` from waiting rooms); H→C `b 10, b 0, w groupSID @2, w who @4` | `class_83::add`, `WaitingRoomGroup::add`; `class_83::handleMsg` | add a member to a group / a member joined; the H→C form is delivered to the group, which calls `addMember: who` | CONFIRMED, both directions live (`captures.md` section 11) |
| 11 | delete / `GrpDel` | both | C→H `w 11, w groupSID, w memberSID`; H→C `…, w who @4` | `class_83::delete`; `class_83::handleMsg` | remove a member / a member left | CONFIRMED |
| 12 | `GrpMem` | both | C→H `w 12, w groupSID, w userSID`; H→C `…, w member[n] @6…` | `Conference::init`, SierraLand invite rooms; `class_83::handleMsg` | request / deliver the member list (n = (len-6)/2) | CONFIRMED |
| 13 | setInt / `SetIntMsg` | both | `w 13, w targetSID, w fromSID, {w propOffset, w value}…` | `Obj::setInt`; `SetIntMsg::doit` | set integer properties of the replicas of an object; on receive each pair is applied with `ObjOffsetProp` | CONFIRMED; C→H live |
| 14 | setStr / `SetStrMsg` | both | `w 14, w targetSID, w fromSID, w propOffset, s value` | `Obj::setStr`; `SetStrMsg::doit` | set a string or array property | CONFIRMED; C→H live (persona name, looks, home) |
| 18–21 | getObj, add, setEq, setNe | C→H | `w n, w toSID, w fromSID, …` | `class_61` (never called) | host object-store operations | CONFIRMED layout, unused |
| 22 | login (old) | C→H | `b 22, b 0, w 0, w idLow, w idHigh, s name, b 2, b flag, a[10] password` | `class_61::login` (never called) | the original login, replaced by 53 | CONFIRMED layout, unused |
| 25 | invokeMethod / `RMI` | both | `w 25, w sid, w selector, w args…`; H→C `…, w selector @4, args @6…` | `Obj::invokeMethod`; `RMI::doit` (`InvokeMethod` kernel) | remote method call on an object's replicas | CONFIRMED |
| 26 | — | C→H | `w 26, w sid, w sid` | `type31.561 class_84::handleMsg` | unknown; live: sent with the new player object's SID twice, right after its `ObjID` | CONFIRMED layout |
| 28 | multicast | C→H | `b 28, w fromSID @1, w n @3, a[n] recipients @5, w msgType, b fg, b bg, b flag, a text` | `script.005` export 1, `script.155 proc_60` (msgType 50), `script.010` exports 14/20 | one message to a list of SIDs; recipients presumably receive a `Send` | CONFIRMED layout; delivery INFERRED |
| 30 | register | C→H | `b 30, w sid…` | `class_61::register` (`script.120 proc_208`, `script.408 Clock::setSid`) | unknown | CONFIRMED layout |
| 31 | getMembers / `GrpGetProp` | both | C→H `w 0x011F, w groupSID, w groupSID, w propOffset…` (byte 1 = 1); H→C words at 6 and 8, then per-member property values | `WaitingRoomGroup::getMembers`; `GrpGetProp::init` | read chosen properties of every member of a group | CONFIRMED (H→C layout partly) |
| 32 | getProp | C→H | `w 32, w sid, w sid, w propOffset…` | `Obj::getProp` | read properties of an object | CONFIRMED layout |
| 33 | `SetMsg` | H→C | body rebased at 6; records of `b type, …` | `SetMsg::doit`, `getProp` | property values (reply to 32) applied to the object | CONFIRMED in part |
| 51 | `ObjNewList` | both | C→H `b 51, b 0, w 0, w cookie, b 1, b landType, w -2, w size, w count`; H→C `w to @4, w ? @6, body @8` | `SL/script.780 placeMen`; PaintBall | create a list of objects | CONFIRMED layout |
| 52 | `ObjFreeList` | both | C→H `b 52, b 0, w 0, w n, a[n]`; H→C `w to @2, body @6` | `SL/script.780 PaintBall::dispose` | free a list of objects | CONFIRMED layout |

#### 3.2.1 Codec names and joinNet kinds

`innkeeper-world::message` types the shared-object commands as follows. Both header shapes are the same
bytes: LSCI writes `w cmd`, the DOS libraries `b cmd, b 0`, and the codec ignores byte 1 on receive and
writes 0.

| Cmd | Client to host | Host to client | Notes |
|---|---|---|---|
| 2 | `ClientMessage::Send(SendMessage)` | `HostMessage::Send` | `to`, `from`, then the payload from byte 6 kept raw. LSCI's `msgType` is a word, the DOS libraries' a byte, so only `msg_type_word()` interprets it, and only for LSCI traffic |
| 9 | `LeaveNet(Sid)` | `ObjectFreed(Sid)` | the host form is the header only |
| 10 | `GroupJoin` | `GroupJoined` | unchanged |
| 11 | `GroupLeave` | `GroupLeft` | `group`, `member` |
| 12 | `GroupMembers(GroupMembersRequest)` | `GroupMembers` | the list is every word from byte 6; the word at byte 4 is unread (INFERRED 0) |
| 13 | `SetInt` | `SetInt` | `target`, `from`, then `(propOffset, value)` pairs to the end |
| 14 | `SetStr` | `SetStr` | `target`, `from`, `propOffset`, then the value verbatim: arrays are not text |
| 28 | `Multicast` | none | `from`, `n` word SIDs, the body raw (`5 + 2n + body`); the body's first word is LSCI's `msgType` |
| 41 | `ObjExists::Name` (sub 0, raw name tail), `ObjExists::Service` (sub 2: `w 0, w userSID, b landType, w 0`) | `ObjectLocated` | the located SID is at w@6; the word at byte 4 is unread (INFERRED 0) |
| 48 | none | `Notice` | text from byte 2, byte 1 unread; the NUL that ends the text is INFERRED |
| 0 | none | `Ack` (`to`, `whichCmd`, `whichSub`, tail) | `whichCmd` 22 stays `LoginAccepted` |

The joinNet `kind` byte says how the host scopes the object; the codec names it by that role and the land
decides what it stands for (`ObjectKind`). Whether a kind is a group is a host decision per land, not a
property of the codec. CONFIRMED layouts, per-land meaning from the census documents:

| Kind | Role name | LSCI | Yserbius and Twinion | Red Baron | GOLF |
|---|---|---|---|---|---|
| 1 | `Object` | player object (param `0xFFFF`, size 30) | player object (type `0x8B`, size 1) | none | player object (type `0x66`, size 1) |
| 2 | `Group` | game group (`GameGroup::joinNet`) | map group (type `0x8B`, param map number, size 104, Twinion 80) | linked, never called | none |
| 3 | `SoloObject` | none seen | none (the client's own enum calls its party slot 3, never sent) | the player's in-game object (type 101, size 1) | none |
| 4 | `PrivateGroup` | `SL/script.120 runRedBaronScript` | personal party (type `0x8B`, param `0xFFFD`, size 4) | none | none |
| 5 | `LandGroup` | waiting-room group (param land number, size = maximum) | land group (param land number, size 100) | none | none |
| 129 | `GameObject` | the game object at logon | none | none | none |

Kind 6 and every other value are errors.

### 3.3 Host services (byte formats)

| Cmd | Name | Dir | Layout | Sub-commands seen | Status |
|---|---|---|---|---|---|
| 23 | `GameRights` | H→C | body rebased at 4 | — (no handler found) | CONFIRMED layout |
| 27 | `BBSMsg` | both | C→H `b 27, b sub, w …`; H→C `b 27, b whichCmd, w number @2, w toSID @4, w type @6, body @4 or @18` | 0 post/status, 1 read, 6, 8, 9 complaint (`SL/script.530 BitchToTSNMI`), 13, 14 list (`script.199`); conference and trivia directories use 27/1 and 27/0 with class 67 and 84 (`GotoConference`, `GotoTrivia`) | CONFIRMED layouts |
| 29 | `GameVersion` | H→C | `b 29, w toSID @1, b number @3, b major @4, b minor @5` | — | CONFIRMED |
| 34 | `AccountMsg` | both | C→H `b 34, b sub, w sid`; H→C `b 34, b whichCmd, w toSID, text @4` | 4 current rates (text, shown when it changed, answered with 34/8), 5 account status (`s` at 4), 8 rates seen, detailed summary (`script.050 GetDetailedInfo`: `w len @16`, text @18) | CONFIRMED |
| 36 | `HostInfo` | both | C→H `b 36, b sub [, w stampLow, w stampHigh]`; H→C `b 36, b type, …` | 1 HOSTADDR file (reply type 1: `w stamp @2, w stamp @4`, file text @6), 5 host number (reply type 5: `w host @2`, at most 200), 6 land directory (reply is `WaitGrpRequest` msgType 2, or Nak 36/6), 2 host time (reply type 2: `b year-1900 @2, b month0 @3, b mday @4, b hour @5, b minute @6, b second @7`; DOS games only) | CONFIRMED |
| 37 | `EMMsg` | both | C→H `b 37, b sub, w sid, w…`; H→C `b 37, b whichCmd, w toSID, w lowWord @4, w highWord @6, w status @8, …` | 17, 18 new-mail check (mailbox low/high), 20, 21, 24 send letter (`"bbwwwawwawaaa"` envelope), 25 read, 26 delete, 27 forward, 30 form to a service box (`"AcctUpdate"`, `"MemberServ"`), 31 services, 32 system list (H→C records from 8, saved as `syslst.dat`) | CONFIRMED layouts |
| 38 | `HintMsg` | both | C→H `b 38, b sub, w roomSID, b ×4`; H→C `b 38, b whichCmd, w toSID, b which @4, a @5` | hint room (`script.135`) | CONFIRMED |
| 39 | `HostFile` | H→C | `b 39, b whichCmd, body @2` | — (no handler found) | CONFIRMED layout |
| 40 | `UserInfoMsg` | both | C→H `b 40, b sub, …`; H→C `b 40, b whichCmd, w msgType @4, w whichSub @6` | 4 set name (`s` at 2), 3 read money, 2 save money (`w sid, w 1, w 1, w low, w high`, CasinoLand `script.000`) | CONFIRMED |
| 41 | `ObjExists` | both | C→H `b 41, b 0, w sid, w sid, a name`; H→C body @6 | conference name check (`GotoConference::init`) | CONFIRMED |
| 44 | password | C→H | `b 44, b 2, w sid, a[10] encoded password` | change password (`script.095` export 5); Ack 44/1 or 44/2 makes `RoomZeroHandler` store it in `PASS_SET.DTA`. Host: `ClientMessage::ChangePassword`; `innkeeperd` stores it, clears the expired flag and answers Ack 44/1 to the SID (`golden/logon_policy.txt`) | CONFIRMED layout; host reply INFERRED |
| 45 | `NewBoxHandler` | both | C→H `b 45, b 1, w sid`; H→C `b 45, b whichCmd, w toSID, b status @4, w low @5, w high @7` | 1 assign mailbox: status 1 = mailbox number low/high (written to `mail.cfg`), 2–6 errors | CONFIRMED |
| 47 | `WaitGrpRequest` | both | C→H `b 47, b 1, w 0, w 0`; H→C `b 47, b msgType, w toSID, w ? @4`, body rebased at 6 | msgType 1 land occupancy, msgType 2 land directory (section 6) | CONFIRMED |
| 48 | `Unsolicited` | H→C | `b 48, b ?, text @2` | operator notice (`script.011 RoomZeroHandler`) | CONFIRMED layout |
| 49 | `UserFindMsg` | both | C→H `b 49, b sub, w 0, …`; H→C `b 49, b whichCmd, w toSID, body @4` | 1 by account number (`w low, w high`), 2 by name (`w 0, s name`) | CONFIRMED |
| 53 | Login | C→H | section 4.2 | — | CONFIRMED |
| 54 | `PMSMsg` | both | C→H `b 54, b 27, w 5, w -1, w 0xEEEE, w 0xDDDD, w 0`; H→C `b 54, b whichCmd, …, w toSID @10` | shopping mall gateway (`script.050 gotoCUCmall`); INN only | CONFIRMED layout |
| 55 | signup | C→H | `b 55, b 1, w cookie, b 1, a form` | online signup form (`script.012 MakeConnection`); Ack 55/1 returns the new account number at w@6/w@8 | CONFIRMED |
| 59 | Login with Prodigy ID | C→H | Login plus `s prodigyId` | — | CONFIRMED |
| 147 | `SysOpCmd` | H→C | `b 0x93, b msgType, body @2`; target is always the SysOp object | operator commands | CONFIRMED layout |

## 4. Logon

### 4.1 Before the first byte

All CONFIRMED in `hub/script.101` unless noted.

1. `dialScript::changeState` state 0 loads the password with `proc_57` (`hub/script.101 proc_57`):

   | `PASS_SET.DTA` starts with | Action |
   |---|---|
   | `0x4241` ("AB") | the next 10 bytes are the encoded password; global 184 = 1 |
   | `0x4342` ("BC"), or the file is missing | write `0x4644` + `"THPPPHHHHT"`, ask for a new password twice (`script.095` export 1) |
   | anything else (`0x4644` after the first run) | nothing stored; state 0 asks for the password (`script.095` export 0) |

   CONFIRMED (`proc_57 +0x0028`..`+0x00AE`). The installer writes the file (`INSTALL.SCR` line
   `echo %5>%1:%6\PASS_SET.DTA`); that "BC" marks a fresh install is INFERRED.
2. The encoding (`script.095` export 4) uppercases the password (6 to 10 characters, A–Z and 0–9),
   XORs byte i with `"M34546788S"[i]`, zero-pads to 10 bytes, then for i = 0..n-1 XORs byte
   `(i+1) mod n` with byte i. CONFIRMED (code); that the host stores the same encoding is INFERRED.
3. The account number comes from the `LSCI.CFG` key `id` (decimal, split into words held in globals 262
   and 261) and a name from key `name` (`hub/script.004` export 0 `+0x003A`, `+0x0063`). The live client sent the
   **persona** name in the Login, with or without a `name` key (`docs/protocol/captures.md` section 4), so that key
   is not where the Login's name comes from. INFERRED: it is an operator or debug setting.
4. `proc_80` composes the dial string and calls Connect (section 7).

### 4.2 The exchange

| # | Dir | Message | Evidence | Status |
|---|---|---|---|---|
| 1 | C→H | **Login** `b 53, b 0, w 0, b landType, b major, b minor, b revision, w idLow, w idHigh, b fromFile, a[11] password, s name` (59 adds `s prodigyId` from `C:\prodtsn.pid` when `LSCI.CFG` has `pFlag`) | `script.196` export 2 `+0x00DA`/`+0x0092`, called by `proc_7 +0x001B` right after Connect | CONFIRMED |
| 2 | — | client starts `LoginTimeout` (70 s); on expiry it shows error 999 "There seems to be a problem logging in. If you still have a problem, call 1-800-IMAGIN-1." (`proc_116`). "We are not receiving any messages from the network." belongs to a different path, `dialScript::changeState +0x008E`. | `LoginTimeout::changeState +0x000C`..`+0x001E`; live run: `docs/protocol/captures.md` section 5 | CONFIRMED |
| 3 | H→C | **AckMsg** `b 0, b ?, w 0, b 22, w userFlags @5, b status @7, w rating @8` (target SID 0, section 2.1) | `Dialing::handleMsg +0x02F3`..`+0x039A`; live: `00 00 00 00 16 00 00 00 00 00` accepted | CONFIRMED layout; field names INFERRED |
| 3' | H→C | or **NakMsg** `b 1, b ?, w 0, b 22, b reason @5, b ?, b numTries @7, text @8` | `+0x03A2`..`+0x0427`: the text is shown; reason 9 counts a retry and the client hangs up after `numTries` | CONFIRMED |
| 4 | C→H | joinNet for the game object: `w 7, w 0, w cookie, b 129, b landType, w 0, w propertyCount` | `dialScript::changeState +0x00BE` → `Obj::joinNet +0x0027`; live: `07 00 00 00 60 01 81 01 00 00 0c 00`, 125 ms after the Ack | CONFIRMED |
| 5 | H→C | **ObjID** `b 8, b ?, w cookie, w ?, w sid` | `Obj::handleMsg +0x003F`; the script waits until the game object has a SID (state 2) | CONFIRMED, live |
| 6 | C→H | `34/4 sid` current rates (skipped with `LSCI.CFG ShutUp`), `36/5` host number, `45/1 sid` mailbox (only without `mail.cfg` and outside demo mode), `36/1` and `36/6` with the stamps from `hostaddr.tim` and `landaddr.tim`, `37/18 sid box` new mail (with a mailbox), `37/32 0 0 0` system list, `40/4 name` | `dialScript::changeState` state 3, `+0x010C`..`+0x0231`; live: all but 37/18 in one frame, in this order | CONFIRMED order |
| 7 | H→C | replies in any order: `AccountMsg` 4, `HostInfo` type 5, `NewBoxHandler` 1, `HostInfo` type 1 (only when HOSTADDR changed, INFERRED), `WaitGrpRequest` msgType 2 or `NakMsg` 36/6, `EMMsg` 18, `EMMsg` 32 | `Dialing::handleMsg`, `RoomZeroHandler::handleMsg` | CONFIRMED handlers |
| 8 | — | state 4 waits for the land directory (`WaitGrpRequest` msgType 2 or `NakMsg` 36/6) | `+0x0242` (local 4) | CONFIRMED |
| 9 | C→H | `47/1 0 0` land occupancy | state 5 `+0x0264` | CONFIRMED |
| 10 | H→C | `WaitGrpRequest` msgType 1 or `NakMsg` 47/1; state 6 waits for either | `+0x0275` (local 3) | CONFIRMED |
| 11 | — | when no land is known yet (land number 0, the normal first logon) the player is sent to the map, room 50 (`proc_5`); otherwise, for example with the debug keys `landNum`/`hostNum` (`proc_48`), a land descriptor is built and `attachScript` runs (section 5.2, step 3) | state 7 `+0x028B`..`+0x0300`; live: "Fall Map" | CONFIRMED |

Fields in step 3: bit `0x04` of `userFlags` gives level 3, `0x80` level 2, `0x02` level 1 (global 82);
`status` 11 means "Your password is out of date", which prompts for a new one and sends command 44
(`+0x0320`..`+0x0374`); `rating` 0 is stored as 1 (global 253). CONFIRMED (code); meanings INFERRED.

Host notes (`innkeeper-world::logon`, INFERRED): the Ack's three fields come from the stored account, and
a record whose password is expired logs in with `status` 11, so the client sends command 44 and the host
clears the flag. A Login whose land type the host does not run is refused with Nak 22 reason 2 (any reason
but 9 shows error 100 + reason). A host with a data directory creates an unknown account from the encoded
password the first Login presents.

The Login's 11-byte password field is copied from a 10-byte array (`a` with count 11), so its last byte
is whatever follows the array in the interpreter's heap. CONFIRMED (`script.196` export 2,
`hub/script.101 proc_57 +0x0036`); a server must ignore byte 11. INFERRED.

What a server must produce, in order: Ack(22) for Login, `ObjID` for the joinNet, `HostInfo` type 5 and
a land directory (or `NakMsg` 36/6) for step 6, then a reply to 47/1. Replies to requests that carry no
SID (Login, 36, 47) are addressed to SID 0. Missing replies stall the client in states 2, 4 or 6, and no
Ack within 70 s ends the session. CONFIRMED live: exactly these five replies, with 34/4, 45/1, 36/1,
37/32 and 40/4 left unanswered, took the stock client to the map (`docs/protocol/captures.md`
sections 9 and 10). That a land directory Nak works as well is INFERRED from the code.

## 5. Land switch

### 5.1 Leaving (`hub/script.101` export 1)

1. Build the shared block (section 5.3) and store it with `TSN(2, block, len)` (`+0x01DE`).
2. Release the current group and free the game object: `leaveNet` (`w 9, w sid, w 0`, `+0x0216`).
3. `TSN(9, program)`: the program name comes from `land.cfg` by land type, else `"Default"`
   (`+0x0264`, `+0x0271`).
4. `TSN(12)`, `TSN(8)`, `TSN(14)`, then the game quits (global 5). TSNEXEC runs the next `TSN.PRG`
   block (`docs/protocol/int14h-api.md` section 9). CONFIRMED.

### 5.2 Arriving

1. `Dialing::init` reads `TSN(0) >> 8`; when the line is up it runs `attachScript`, otherwise
   `dialScript` (`+0x0056`..`+0x0094`). CONFIRMED.
2. `attachScript` state 0 clears all SIDs and installs a new `class_87`. State 1 runs `proc_125`:
   `TSN(1)` reads the shared block, restores identity, frees the previous land's game object by SID
   (`w 9, w oldSid, w 0`, `proc_125 +0x020B`), then clears the block with `TSN(2, 0, 0)`. It then sends
   `37/18` and `47/1`. CONFIRMED (`attachScript::changeState +0x00C0`..`+0x0102`).
3. State 3 compares the target host number with the current one (global 251). When they differ it calls
   SwitchHost (section 7.2) and then `proc_7`, which **sends Login again** and restarts the timeout
   (`+0x01E1`..`+0x0226`). When they match, nothing is sent. A failed SwitchHost falls back to a full
   redial (`dialScript`). CONFIRMED.
4. State 4 sends joinNet for the game object (`+0x0335`); state 6 sends `36/5` (`+0x0373`); state 9
   sends `40/4 name` (`+0x04A4`). CONFIRMED.

### 5.3 Shared block (written by export 1, read by `proc_125`)

| Offset | Size | Content | Status |
|---|---|---|---|
| 0 | w | user SID | CONFIRMED |
| 2 | w | `userFlags` from the login Ack | CONFIRMED |
| 4 | w | SID of the game object to free | CONFIRMED |
| 6 / 8 / 10 | w each | target land number, land type, host number | CONFIRMED |
| 12 | b | `rating` (global 253) | CONFIRMED |
| 13 | b | land flags | CONFIRMED |
| 14 | w | 2 when the password came from `PASS_SET.DTA` | CONFIRMED |
| 16 | 11 + NUL | user name | CONFIRMED |
| 28 | 15 + NUL | host name or address (global 168) | CONFIRMED |
| 44 | 10 + NUL | encoded password | CONFIRMED |
| 128 | rest | parameters for the next program | CONFIRMED |

The DOS games read words at +0 and +4 and an 11-byte string at +10 (`docs/protocol/int14h-api.md`
section 9.3), which does not match this layout; see section 9.

### 5.4 Entering a land on the same host (live)

Choosing the Clubhouse on the map while logged in to the host that runs it sent no Login, no leaveNet,
no BREAK and no `c <host>`; the client stayed in the same `LSCITV`. Its messages, in order
(`docs/protocol/captures.md` section 11, CONFIRMED on the wire):

1. joinNet for the game object again, same cookie, which gets a new SID.
2. After the player answers the "Want To Play" dialog: `40/4 name`, joinNet for the player object
   (kind 1, param `0xFFFF`, size 30), command 26 with that SID twice, `setStr` name (offset 5), looks
   (offset `0x11`) and home (offset `0x17`), one `setInt` with eight pairs.
3. joinNet for the waiting-room group (kind 5, land type, land number, maximum 128), then `add` of the
   player to it with the version bytes.
4. The room stays at "Entering Clubhouse..." until the group has a SID and contains the player:
   `hub/script.120 PlayerLogin::changeState` state 3 (`+0x00C3`..`+0x00DC`). A `GrpJoin` naming the player
   ends the wait; state 4 then waits for every member's name, and state 5 sends `setInt` with the
   current game and room.

Which script sends step 1 is INFERRED (the place list of `hub/script.055`, not `attachScript`, since
no 36/5 followed).

## 6. Land tables

Both arrive as `WaitGrpRequest` (47); offsets are after the rebase at message byte 6. CONFIRMED
(`Dialing::handleMsg +0x0023`..`+0x01FB`, `hub/script.000 ProcessLandInfo::handleMsg`).

| msgType | Body | Client use |
|---|---|---|
| 1 occupancy | `w ?, w ?, w n`, then n × `b host, b landType, b landNumber, b maximum, b current` | updates the place list; polled every 60 s (20 s on the map) by `ProcessLandInfo`, both intervals seen live |
| 2 directory | `w stampLow, w stampHigh, w n`, then n × `b host, b landType, b landNumber, b ?, b ?, b min[3], b max[3], b flags, s description` | stamp saved to `landaddr.tim`, records written to the `LandAddr` file as `%20s %3d %3d %3d %03d.%03d.%03d %03d.%03d.%03d %3d` |

The two bytes after the land number are read into locals that nothing uses (`Dialing::handleMsg`
`+0x00BE`..`+0x00D4`, temps 9 and 10). CONFIRMED by the code and live: without them the client shifted
every later field (`docs/protocol/captures.md` section 10). Whether the host put a maximum and a count
there, as in an occupancy row, is unknown; the server writes zeros.

`min` and `max` are interpreter versions (major, minor, revision) allowed in that land. INFERRED from
the `minVer`/`maxVer` properties (`hub/script.055 proc_20`); the range 0.0.0 to 255.255.255 was accepted
live. `HostInfo` type 1 carries the `HOSTADDR` file itself; the client saves it verbatim and the stamp
to `hostaddr.tim`. CONFIRMED.

Reading the `LandAddr` file back (`hub/script.055` export 1 and `proc_20`), CONFIRMED by the code:

- Fields are split at space, `.` and tab, so the description is one word; `_` is shown as a space and
  the description is cut to 16 characters (export 1 `+0x00D2`..`+0x0108`).
- A row with land type 0, land number 0 or **flags 0** is dropped (`proc_20` `+0x0072`..`+0x01C1`); live,
  a directory with zero flags gave "There are no places available right now for this land." The default
  of the land object (`class_97`) is 1.
- Flag bits `0x0C` choose the CasinoLand disclaimer: 4 "Unrestricted", 0 "Restricted" (`hub/script.055
  proc_37`). Other bits are unknown.

`ProcessLandInfo` sets every known land's maximum to -2 before applying an occupancy reply, and
`PlaceButton::draw` shows a special cel for -2, so a land missing from the reply is shown as not
available. The first two words of the occupancy body are not read. CONFIRMED (`hub/script.000
ProcessLandInfo::handleMsg`, `hub/script.055 PlaceButton::draw +0x00EF`).

## 7. Connect and SwitchHost arguments

### 7.1 Connect (`hub/script.101 proc_80`)

CONFIRMED (`proc_80 +0x0029`..`+0x0282`):

| Driver (`TSN(0) & 0xFF`) | Argument |
|---|---|
| 3 (modem) | `prefix` + `"t"` + host + dial command |
| 2 (Novell) | `LSCI.CFG novell`, or a prompted "Host name:" |
| 1 (serial) | `"foo"` |

- host = `proc_84(hostID)`: the second column of the `HOSTADDR` line whose first column equals the
  first token of `hostID` (default `"SIERRA"`), or `hostID` unchanged when no line matches. So
  `hostID = Sierra7` dials `t311083420207` and `hostID = SIERRA` dials `tSIERRA`.
- dial command = `modem` on the first try, `modem2` on the second, and from the third try (or when both
  are absent) `"ATDT" + <number typed at "Number to dial:"> + "!~"`. A dial string containing `ATDT1`
  shows a long-distance warning first.
- The signup path (`hub/script.012 MakeConnection::doit`) uses the same composition through its own
  copy of the lookup (`proc_446`) and the format `"%st%s%s"`.

This settles the grammar left open in `docs/protocol/link-layer.md` section 3.2: the order is
`prefix`, `t`, host, `modem`, as that section assumed.

### 7.2 SwitchHost (`attachScript` state 3)

- The target host number becomes an address through `hub/script.055` export 3: the second column of
  the `HOSTADDR` line whose **fourth** column is that number (`proc_24`). So host 7 gives
  `"311083420207"`, which the driver turns into `c 83420207`. CONFIRMED.
- Modem and serial drivers pass that address to `TSN(13)` (`proc_81 +0x0047`). The Novell driver maps it
  back to the first column, the host name, before calling `TSN(13)` (`proc_82 +0x0126`). CONFIRMED.
- The third `HOSTADDR` column (`CC` in every line) is skipped by both readers. Its meaning is unknown.

## 8. Game traffic

All game messages are `Send` (command 2) to the group SID (global 119), from the player's SID
(global 81 or 125), with a game-defined `msgType`. The host only relays. CONFIRMED by the send sites
below; "only relays" is INFERRED (no host-side game logic is visible to the client).

| Game | Script | msgType values sent → handled | Notes |
|---|---|---|---|
| Checkers | `hub/script.400` | sent: 2 (`w fromSquare, w toSquare, w local13`), 6, 8, 18, 19, 20, 26, 29, 30; handled: 2, 6, 8, 18, 19, 20, 26, 29, 30 | `class_ab::move +0x0131`: `send: 2 (self number:) (param1 number:) local13` |
| Hearts | `hub/script.510`, `511` | sent: 24 (`sendArray: 24 52 deck`), 30; handled: 1, 23, 24, 25, 26, 29, 30, 38 (511 also 4, 11) | `HeartsPlay::changeState +0x0282` deals all 52 cards in one `Send` |
| Invitations | `hub/script.120`, `121` | 12, 13, 14, 27, 53, 60, 999 | `lock`/`unlock` serialise invitations; `GrpMem` lists the waiting room |
| Chat | `script.005` | 1: `b fg, b bg, b flag, a text`; 36 | multicast (28) for conferences, with msgType 50 |

Watching a game uses msgType 30 with an array (`proc_67`, `proc_79`, `waitForWatch::cue`). The tool's
handler section lists every msgType per game.

## 9. Shared host messages in the DOS games

GOLF's `src\tsn\hostcomm.c` builds messages with the same header (command byte, byte 1, word 2):

| GOLF message | Bytes | LSCI equivalent | Status |
|---|---|---|---|
| `07 00 00 00 …` (12 bytes), "create object timeout" | image `0x13872` | joinNet (7) | CONFIRMED layout match |
| `24 02` (2 bytes), "get host time timeout" | image `0x13744` | `HostInfo` sub 2 | CONFIRMED bytes; "host time" from the string, INFERRED |
| `29 00 00 00 w …`, "join group timeout" | image `0x13696` | `ObjExists` (41) | CONFIRMED bytes; meaning INFERRED |
| `32 b w w` | image `0x135F9` | none (50 is not used by LSCI) | CONFIRMED bytes |

The DOS games never call Connect; they inherit the session and identity through the shared block
(`docs/protocol/int14h-api.md` section 10). They do not log in again (census README section 4); a test
launcher logs in on their behalf (`docs/dosbox.md` section 6).

## 10. Version differences

From `tools/tsn_messages.py work/res` (2026-10-04). CONFIRMED.

- Dec-1993 and Feb-1994 send and receive the same commands. The send layouts differ in one SierraLand
  `Send` site and in property names inside `script.145`/`199` (the class dictionaries differ).
- TSN 2.1 lacks commands 54 (`PMSMsg`, both directions), 55 (signup) and 59 (Prodigy login).
- TSN basic also lacks 6, 12, 51 and 52 on the send side; its receive table equals TSN 2.1's.
- Login is command 53 and the login Ack is `whichCmd` 22 in every set, including TSN basic.

## 11. Unknowns

- Byte 1 of host replies (`AckMsg`, `ObjID`) and word 4 of `ObjID`: never read by the client.
- What `rSend` (3), `lockQueue` (5), commands 18–21, 26 and `register` (30) do; only their layouts are
  known, and 3, 5, 18–21 and 22 are never sent.
- `notify`: a `class_61` flag ORed into byte 1 of object commands; never set by any script.
- Whether the host fans a `Send` to a group SID out to every member, and whether it echoes it to the
  sender. Game handlers suggest every member including the sender gets it; a capture would settle it.
- Whether a `GrpJoin` reaches the other members of the group as well as the joiner, and a `GrpDel` the
  remaining members. The waiting room fetches a newcomer's name in `addMember`
  (`hub/script.003 WaitingRoomGroup::addMember`) and the DOS games drop a remote player on `GrpDel`
  (`int14h-census/yserbius.md` section 4.2), so `innkeeperd` tells every member (INFERRED,
  `docs/server/objects.md` section 6).
- How the host scopes groups. `innkeeperd` shares kinds 2, 4 and 5 by kind, land type and parameter, and
  gives a negative parameter (the personal party's -3) a group of the requester's own (INFERRED,
  `docs/server/objects.md` section 3).
- Whether a second joinNet with the same cookie replaces the first object. The Clubhouse entry re-joins the
  game object with no leaveNet (section 5.4); `innkeeperd` frees the old one and its group memberships
  (INFERRED).
- The meaning of `userFlags`, `status` and `rating` in the login Ack beyond the bits the client tests;
  all zero works.
- The two unread bytes of a land directory row, and the land flags beyond bits `0x0C`.
- What command 26 asks for; the client sends it for its player object and needs no reply.
- Which object the interpreter variable behind `SID(3, 0)` holds; the game object is INFERRED.
- Field meanings of `GrpGetProp` (31) and `SetMsg` (33) replies, and the record format of the system
  list (`EMMsg` 32).
- `GameRights` (23), `HostFile` (39) and `GameVersion` (29) have classes but no handler in any land.
- How the host derives the password check value from what the client sends, and what byte 11 of the
  Login password field holds.
- The DOS games' shared-block offsets (section 5.3) disagree with the LSCI layout.
- The third `HOSTADDR` column (`CC`).

## 12. Verification

A second pass (2026-10-04) checked the tool's results against the raw disassembly from
`tools/lsci_disasm.py`:

- The 31-row dispatch in `hub/type31.559 class_62::with` (instruction pattern `dup; ldi N; eq?; bnt;
  class C`), the base-class reads, and the reads of `class_63`, `class_64`, `class_65`, `class_66`,
  `class_72`, `class_73` against their `init` bytecode.
- The Login send in `script.196` export 2, its caller `proc_7`, and the order of the state-3 sends in
  `dialScript::changeState` (`+0x00E9`..`+0x0231`), read instruction by instruction.
- The dial-string branches of `proc_80` (`+0x00BB`..`+0x01C8`) and the `HOSTADDR` readers `proc_84`,
  `proc_82` and `script.055 proc_24`.
- The login Ack handling (`Dialing::handleMsg +0x0319`..`+0x039C`), including the level bits.
- GOLF's message bytes, disassembled with capstone at the four image offsets in section 9.

Live (2026-10-05, `docs/protocol/captures.md` sections 9 to 11): the logon of section 4.2, the land
tables of section 6 and the Clubhouse entry of section 5.4 against the stock client. No capture of the
original host exists, so meanings of host-side behaviour (fan-out, unanswered requests) remain INFERRED.

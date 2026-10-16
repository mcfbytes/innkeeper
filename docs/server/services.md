# Town Hall services (commands 34 and 37/30, 37/32)

What the map's Town Hall (`TownHall`, map selection 13) sends and waits for, and what `innkeeperd` answers.
The scripts are `TSNServices`, `TSNServicesDialog`, `GetInfo`, `GetDetailedInfo`, `TSNMemberServices`,
`UpdateAccountInfo`, `ChangePassword` and `ProcessServices` in `hub/script.050`, the password dialogs in
`hub/script.095`, and the logon handler `RoomZeroHandler` in `hub/script.011`. Code:
`innkeeper-world::town_hall` (the behaviour) and `message::account` (the codec of command 34). All offsets
are message bytes from 0 (the Ack and Account classes do not rebase; `class_64::init`, `class_71::init`).
Every text the server sends is its own wording.

## 1. Why it froze

`GetInfo` (60 s) and `GetDetailedInfo` (240 s) send a command 34 request, open no dialog, and wait for a
reply in a `changeState` timer; when it fires they say "Sorry, but this service is not currently
available." and return to the menu. This host dropped command 34 as "no parser", so every billing item sat
out the wait. Neither class has a Nak path for `GetInfo`, so those two must be answered with text.

## 2. The audit

"Wait" is what the client sits on. Replies are command 34 unless stated.

| Menu item | Request | Wait | Reply layout | Before | Now |
|---|---|---|---|---|---|
| (open Town Hall) | state 0 reads local `syslst.dat`; if missing, 37/32 `"bbw+"` and "Please wait one moment..." until a 37/32 reply or a Nak 1/32 sets `local16` | until that reply | 37/32 (section 4) | empty list: the menu opened, "Ask Member Services" said "temporarily not available" | list with the two service boxes |
| New Member Orientation | none: `help` on the map | none | none | local | local |
| Ask Member Services | 37/30 `"bbwwwawwawaaa"` to the type 100 box, addressee `MemberServ` | none (the form box closes at once) | Ack 37/30 with the box words at 7 and 9, or Nak 1/37 with the same words | dropped | Ack; text kept as a complaint (section 5) |
| Billing > Update Account Information | 37/30 to the type 101 box, addressee `AcctUpdate` | none | as above, "sent to Accounting" | dropped | Ack; text discarded (section 5) |
| Billing > Check My Account Status | 34/5 `"bbw"`, sid word 0 | 60 s | `b 34, b 5, w toSID, s text` at 4, shown in "Account Status" | dropped, 60 s wait | text reply |
| Billing > Check My Current Rates | 34/4 `"bbw"`, sid = game object | 60 s | `b 34, b 4, w toSID, s text` at 4, shown in "Current Rates" only when it differs from the copy the client keeps | dropped, 60 s wait | text reply |
| Billing > Usage Summary / Billing Summary | 34/12 or 34/15 `"bbwwwbw"`: sid, account low, account high, byte 0, word 100, via TSN call 18 | 240 s | `b 34, b sub, w toSID`, 12 unread bytes, `w length` at 16, text from 18; shown in room 42 as "Account Summary". Nak 1/34 with that sub shows "not available" at once | dropped, 240 s wait | text reply |
| Change Password > Change INN Password | the client checks the old password against its own copy, then sends 44/2 `"bbwa"`: sid and the 10 encoded bytes of the new one | none | Ack 44/1 or 44/2 writes `PASS_SET.DTA` and says "New password accepted"; Nak 44/1 or 44/2 says it was not changed | Ack 44/1 worked; the refusal Nak named sub 0, which the client ignores and leaves "Wait for the previous password to come back" set | Nak now names sub 1 |
| Change Password > Change CasinoLand Password | local `llsystem.dat`, no send | none | none | local | local |
| Change Password > I forgot my password | a text | none | none | local | local |
| Print Documentation | local `doc.lst` and `PRN`, no send | none | none | local | local |
| Get Help about INN | `GotoHelpArea`: searches the land directory for a land whose description holds "help"; none: "New Member Orientation is not available right now" | none | none | local | local |
| Services from `syslst.dat` (type 3) | `ProcessServices` sends a 37 list for the service box | a reply | | the list has no type 3 records, so no such items exist | unchanged |

The reply to a request arrives while the modal Town Hall menu is open, and the client handles it after the
player leaves the menu: the Ack's "sent to Member Services" text and the password messages appear on the map
after Cancel. The billing replies are handled at once because `GetInfo` and `GetDetailedInfo` are the room's
script. A wrong old password never leaves the client ("The password you entered is incorrect.").

## 3. Command 34

| Request | Reply |
|---|---|
| 34/4 | text. The first 34/4 after a Login is the logon's and is not answered (`FIRST_RATES_REQUEST_IS_THE_LOGON_ASSUMED`): the logon handler would show a modal Quit / OK box with the text on every logon (it compares with a copy kept only in memory) and send 34/8. A client with `ShutUp` sends none at logon, so its first Town Hall request goes unanswered and times out |
| 34/5 | account number, persona, status and a no-charges line |
| 34/8 | none; absorbed |
| 34/12, 34/15 | `SUMMARY_UNREAD_BYTES_ASSUMED` (12 zero bytes), length, text with CR LF line ends. The account words in the request are not used; the session's account is |

The rates text has no version word: the client compares the whole text with its copy. Texts are short
enough for their boxes (checked live, section 7).

## 4. The system list (37/32) and its boxes

`RoomZeroHandler` copies each record of the 37/32 reply to `syslst.dat`, and `TSNServices` reads it. A
record is `w boxLow, w boxHigh, b type` and a name ending in NUL (the client turns `_` into a space);
records start at byte 8. Types: 100 Member Services, 101 Accounting, 3 a menu service. This host lists
types 100 and 101 only, with the boxes `MEMBER_SERVICES_BOX_ASSUMED` and `ACCOUNTING_BOX_ASSUMED`.

**Client quirk, found live.** The handler stores each box word as its low byte (`syslst.dat` held
`01 00 fe 00` for 0x0001 and 0xFFFE) while the Ack's words are compared whole, so both words of a service
box must be below 256. The boxes are `0x00FE_0001` and `0x00FE_0002`; account mailboxes count up from 1.
Without a type 100 record the locals stay `0xFFFF` and `TSNMemberServices` says the service is temporarily
not available. The list is answered on the open book too. 37/31 (services) is still an empty listing.

## 5. What 37/30 does with the text

The Ack carries the form's target box at words 7 and 9, which the map matches with the boxes it loaded.
An unknown box gets Nak fault 6 (`NoSuchBox`); a store failure on the Member Services path gets fault 2.

- **Member Services**: the question is the player's own message to the operator. It is kept as a post on
  the complaint board location (`0x63`, section 2, land 0), body `persona|Member Services: text`, up to
  `MEMBER_SERVICES_MESSAGE_LIMIT_ASSUMED` bytes, so it follows the 365-day complaint retention and the
  account export and delete find it by the account recorded on the post (privacy.md sections 1 and 6). Section 2 is listed by no
  directory, so no game client reads it. `MEMBER_SERVICES_MESSAGES_ARE_KEPT_ASSUMED`.
- **Accounting (Update Account Information)**: the form is free text about billing, so a player may type
  an address or a card number. It is acknowledged and thrown away unread; only the account number and the
  fact that a form arrived are logged (`ACCOUNT_FORMS_ARE_DISCARDED_ASSUMED`). There is no billing here.

`Envelope`'s `Debug` prints the text length only, so no log line carries a letter or form text.

## 6. Passwords

The old password is checked by the client against its stored copy, so the host never sees it and cannot
verify it against the stored hash. The host stores the new one (`Admission::Stored`) and answers Ack 44/1.
A refusal (unknown account, store failure) is Nak 44 with sub `PASSWORD_REFUSED_SUB_ASSUMED` (1); with
sub 0 the client ignored it. The CasinoLand password is a local file, `llsystem.dat`, that no message
reaches, and the account stores no separate one. Not exercised live: the Nak (needs a failing store).

## 7. Checked against the stock client

Headless native runner, the Feb-94 client, account 100001, fresh data directory (2026-10-16). Every item was
clicked through; no "not decoded" line was logged; each returned to the menu or the map.

- Ask Member Services and Update Account Information: the form opens, Send, Cancel; the map says "Your
  message has been sent to Member Services." / "...to Accounting." A Nak showed "could not be processed".
- Account Status, Current Rates (box), Usage Summary, Billing Summary (room 42; Map returns to the map), and
  the Town Hall reopens afterwards.
- Change Password: with the right old password and a new one, `PASS_SET.DTA` holds the new encoding and the
  map says "New password accepted"; a wrong old password is refused locally.
- New Member Orientation says it is not available (no help land in the directory); Print Documentation is local.

## 8. Not answered

- 37/31 services (empty), type 3 menu services, and a `ShutUp` client's first 34/4 (section 3).
- The Nak for a refused password change was not seen live.

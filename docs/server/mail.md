# Mail (`EMMsg` 37 and `NewBoxHandler` 45)

The host side of the client's mailbox: `innkeeper-world::mail` (the behaviour, over the `Store` calls
`assign_mailbox`, `append_letter`, `letters` and `delete_letter`) and `innkeeper-world::message::mail`
(the codec of both commands in both directions). Layouts were decoded from the Feb-1994 hub scripts:
`script.145` (`class_c8`, the box class and its send, read and delete), `script.140` (`MailRoom`,
`EMailBox`), `script.011` (`RoomZeroHandler`) and `script.050` (`TSNMap`, `ServicesMailBox`).
The catalog rows are in [messages.md](../protocol/messages.md) section 3.3; the golden bytes are
`crates/innkeeper-world/tests/golden/mail.txt`, the transcripts `tests/mail.rs`.

## 1. Numbers and boxes

A client keeps every 32-bit value as a low word and a high word, little-endian: a mailbox number, a
letter id and an account number (`MailboxNumber`, `LetterId`, `AccountId`). The client accepts boxes
"between 1 and 4294901759" (its own error text), so the host refuses a send to 0 or above `0xFFFEFFFF`
with fault 9. Mailbox numbers come from the store: the first `assign_mailbox` for an account assigns one,
later calls return it, so the number is stable across logons and across a second 45/1.

## 2. Requests (client to host)

Every command 37 request is `b 37, b sub, w sid`; the host answers to `sid`.

| Sub | Fields after `sid` | Host answer |
|---|---|---|
| 17 | `w low, w high` of the account number | reply 17: the account's boxes (one), or Nak |
| 18 | box | reply 18: new-mail status |
| 19 | box | reply 19: listing |
| 20, 21, 24 | box of the addressee, then the envelope (section 4) | Ack 37/sub with the box that received it, or Nak |
| 25 | box, letter id | reply 25: the letter's text, or Nak |
| 26 | box, letter id | nothing, or Nak when the letter is not there |
| 27 | box, letter id, addressee's box, `a[10]` new addressee name, `s` note | Ack 37/27 ("Letter successfully forwarded."), or Nak |
| 30 | box of the service, then the envelope | nothing: the form is accepted and dropped |
| 31 | box | reply 19 with no rows |
| 32 | `w 0, w 0` | reply 32 with no rows |

Sub 17 is the Mail Room asking which boxes the signed-on account owns (`script.140` export 4 with the
account number); the answer opens the box directly when there is one. Sub 20 and 21 also exist as short
`bbw+` forms (`script.145` exports 7 and 8, six words, used by the sysop screens); they are not envelopes,
fail to parse (`Truncated`) and get no answer.

`45/1` is `b 45, b 1, w sid`.

## 3. Replies (host to client)

Replies to command 37 are `b 37, b whichCmd, w toSID, w low @4, w high @6`, then the sub-command's own
fields. The client matches the box words with the box it asked about and ignores a reply for another box.

| Reply | After the box words | Source |
|---|---|---|
| 17 | `w low, w high` per box from byte 8 (`script.145` export 5) | CONFIRMED layout |
| 18 | `w status` at 8: bit 0 lights the map's mail icon (`HaveMailIcon`) and the logon notice, bit 1 marks the box in the window | CONFIRMED layout; the meaning of the bits INFERRED |
| 19 | rows of 47 bytes from byte 8 (`class_c7::ackMsgStr`): `b status, w idLow, w idHigh, w fromLow, w fromHigh, a[10] subject, a[10] sender, w flags, a[6] time, a[10] addressee`; at most 64 rows are read | CONFIRMED layout |
| 25 | `w idLow, w idHigh` at 8, the text from byte 12 to the end | CONFIRMED layout |
| 32 | nothing: the loop in `RoomZeroHandler` reads records from byte 8 and stops at the end, so an empty list is the 8-byte header | CONFIRMED layout; empty list INFERRED |
| Ack 37/24, 37/27 | `b 0, w low, w high` of the box that received the letter (the client shows "Letter received by box N.") | CONFIRMED layout |
| Nak 37/sub | `b fault, w low, w high` of the box the request named | CONFIRMED layout |

Faults are the client's own error numbers, which it turns into text (`script.145` export 10): 2 "the
host had some kind of disk error" (a failing store), 6 "it does not exist" (an addressee's box nobody
has), 9 box number out of range, 12 "you do not have rights to do that" (another account's box), 14 "that
letter does not exist", 21 "the post office is closed" (see section 6).

The reply to command 45 is `b 45, b 1, w toSID, b status @4, w low @5, w high @7`. Status 1 carries the
mailbox number, which the client writes to `MAIL.CFG` (two words) and shows as "Your MailBox is: N"; from
the next logon it sends 18 instead of 45/1. Status 2, 3, 5 and 6 are errors the client shows by number,
and 4 prints "ImagiNation is busy creating a mail box for you" and tries again at the next logon.

## 4. Letters are the client's bytes

A send is `b 37, b sub, w sid, w boxLow, w boxHigh` and then the envelope, which is the formatted send
`"bbwwwawwawaaa"` after those fields:

| Offset in the envelope | Field | Notes |
|---|---|---|
| 0 | `a[10]` reserved | the client fills it from an empty string; nothing reads it |
| 10 | `w low, w high` | the sender's own box (`isMe`, `formatTo`) |
| 14 | `a[10]` subject | shown under the box line |
| 24 | `w flags` | `allocate` |
| 26 | `a[10]` addressee name | "To:" |
| 36 | `a[10]` sender name | "From:" |
| 46 | `s` text | the body with its NUL, as written |

The host reads the addressee's box and nothing else from a send. It stores the envelope unchanged behind
six bytes of its own, the delivery time (`HostInfo` type 2 layout), because the listing shows a time and the
store keeps none: the body in the store is `year-1900, month0, mday, hour, minute, second` and then the
envelope. Reading a letter returns the text bytes exactly as sent (tested with a high-bit byte, a CR LF and
the NUL); a listing row is cut from the same bytes and the letter's id. Deleting removes the letter and
answers nothing.

## 5. Assumptions

Each is a named constant in `innkeeper-world/src/assumptions.rs`, labelled INFERRED in
[messages.md](../protocol/messages.md) 3.3.

- Only the requester's own box may be checked, listed, read, deleted from or forwarded from
  (`OTHER_BOXES_ARE_PRIVATE_ASSUMED`); anyone may send to any assigned box. The refusal is fault 12.
- A new-mail check says bits 0 and 1 together while the box holds any letter: the store has no read
  marker, so a letter stays "new" until it is deleted.
- A forwarded letter keeps its text and takes the new addressee name; the client's note is not delivered.
- Replies keep the fields the client never reads at zero: the status byte of a listing row, the byte
  before the box words of an Ack, the words after the header of the empty system list and the box words
  of a refused 45/1 (`LISTED_LETTER_STATUS_ASSUMED`, `ACK_UNREAD_BYTE_ASSUMED`,
  `SYSTEM_LIST_UNREAD_WORDS_ASSUMED`, `REFUSED_MAILBOX_ASSUMED`).
- 45/1 answers status 2 for an account the store does not know (`NO_ACCOUNT_MAILBOX_STATUS_ASSUMED`) and
  status 4 when the store fails (`STORE_FAULT_MAILBOX_STATUS_ASSUMED`).
- The delivery time uses the host clock and the `HostInfo` type 2 byte order; the date shown by the client was
  not checked.

## 6. The open book

`World::stock()` has no stored accounts: the account book admits anyone but the store knows nobody, so
`assign_mailbox` fails with `UnknownAccount`. 45/1 answers status 2, and every 37 request is refused with
fault 21 ("the post office is closed"), except a form (30), which is dropped as always. A host started
without `--data-dir` therefore has no mail, and a logon still completes, because the client carries on
without the mailbox.

## 7. Checked against the stock client

DOSBox-X with `innkeeperd --data-dir` (2026-10-05, the Feb-94 CD client, account 100001):

- Logon: 45/1 gets status 1 with box 1 and the client writes `MAIL.CFG` (`01 00 00 00`); 37/32 is answered
  by the empty system list and the logon goes on to the map.
- The next logon sends 37/18 with box 1 and the reply is accepted; entering the Post Office on the map sends
  37/17 with the account number. Until the reply was added the client printed "You do not have a mail box
  attached to this ID"; with it the Mail Room opens "Mailbox 1" and, after 37/19, shows "Your mail box is
  empty." with the Read, Write, Address Book, Map and Options buttons.
- The Mail Room first said "You don't have any INN Stamps. Without them you can only receive mail." That
  is bit `0x200` of the login Ack's `userFlags` (global 86): an account enrolled on first login has no flags,
  so it cannot send until its record is given `0x200`. The harness test set it in the SQLite file. A default
  for enrolled accounts belongs to the logon policy, not to this module.

The key script `mailbox` (`tools/dosbox/key_scripts.py`) reaches "Mailbox 1". The compose, send and read
dialogs were not driven: the Mail Room's buttons are reached by tabbing (after dismissing the empty-box
notice `tab` visits Map, Options, Return to Mailboxes, Write a Letter, Address Book), but each screen takes
10 to 20 s to appear at 30000 cycles, the address and text entry need typed boxes and the two accounts need
two client trees, and a run in which the Post Office did not open was seen once. The acceptance criterion
for sending and reading between two accounts is therefore the transcripts in `tests/mail.rs` (A sends B a
letter; B's check shows new mail; the read returns the bytes unchanged; the delete returns the box to empty; the
mailbox number is stable) and the golden rows, which replay the layouts the client's own code uses.

## 8. Not done

- A sysop's view of other accounts' boxes (`sysopIDToBoxMI`), mailing lists and the short sub-20 and
  sub-21 forms.
- Services (31) is an empty list; the service forms (30) are dropped, so "Member Services" requests are
  accepted and never answered.
- Push delivery: a client learns of a letter at its next 18, not when it arrives.

# Live captures: the stock client against innkeeperd

Sections 1 to 8 are the first capture, which stopped at the Login; sections 9 to 13 the second, in which
`innkeeperd` answers the logon and the client enters the Clubhouse.

The Feb-1994 INN client (CD set, `MODEM.DRV`, COM1, 2400 bps) ran in DOSBox-X 2026.01.02 against
`innkeeperd` v0 on 2026-10-05. The harness is described in `docs/dosbox.md`; the server in
`docs/server/innkeeperd.md`. Everything below was seen on the wire or on the screen, so it is CONFIRMED unless
a line says INFERRED. Times are seconds since the TCP connection opened and come from the server's capture
file (`work/captures/*.hexlog`, not committed).

## 1. How far it got

| Layer | Result |
|---|---|
| Client start, persona, "Select Player" | works; `Play` starts the dial (a persona has to be created once) |
| Hayes modem and dial string | `\rAT\r`, init string, `ATDT5551234` seen; answered by DOSBox-X or by `hayes_fever` |
| PAD wake-up and call | `@`, `D`, CR, CR, then `c SIERRA`; answered `TERMINAL=`, `@`, `SIERRA CONNECTED` |
| Link layer | the client's first DATA frame arrived with a good CRC and was ACKed; the ACK `81 49 62 90 82` was accepted (no resend, no NAK) |
| First message | a **Login** message, decoded in section 4 |
| Everything after | blocked: the server has no application logic, so no reply came |

What blocked: the client waits for the Ack of its Login. After about 70 s it shows the alert "Error #999 -
There seems to be a problem logging in. If you still have a problem, call 1-800-IMAGIN-1." and stops. No
second frame and no resend of the first one followed.

Screenshots of this run (`work/dosbox/shots/play_*.png`, not committed): the startup logo, "Select Player" with
the persona, the "Dialing" and "Modem Connect" screens, then the Error #999 alert.

## 2. The dial, frame by frame (modem emulation)

DOSBox-X dialed `5551234`, mapped by its phonebook to `127.0.0.1:2314`. `tx` is what the client sent, `rx`
what it received.

| t | Dir | Bytes | Meaning |
|---|---|---|---|
| 1.551 | tx | `40` | `@`: the client's first byte after `CONNECT`; the rate is above 1200 so it sends `@` and not CR |
| 2.102 | tx | `44` | `D` (0.55 s later) |
| 2.651 | tx | `0d` | CR |
| 2.651 | rx | `0d 0a` `TERMINAL=` | server answers the first CR |
| 3.201 | tx | `0d` | second CR |
| 3.201 | rx | `0d 0a 40` | `\r\n@`, the prompt the client waits for |
| 3.753 | tx | `63 20 53 49 45 52 52 41 0d` | `c SIERRA\r`: the host name is `hostID` |
| 3.784 | rx | `0d 0a` `SIERRA CONNECTED` `0d 0a` | server confirms the call; both sequence numbers are 0 |
| 5.437 | tx | 39 bytes, section 3 | the first DATA frame, 1.65 s after ` CONNECTED` |
| 5.594 | rx | `81 49 62 90 82` | ACK 0 |

The pauses are the 9-tick waits of the driver (about 0.55 s) and the 1 s pause after ` CONNECTED`, as in
`docs/protocol/link-layer.md` section 4 (steps 7 to 10). Bytes arrive one at a time about 4.2 ms apart (2400 bps,
10 bits per byte), so the 39-byte frame takes 157 ms to arrive; `innkeeperd` now writes bytes that follow each
other within 50 ms as one capture line.

## 3. The first DATA frame

```
81 | 58 d8 | 00 | 21 35 00 00 00 01 02 03 12 a1 86 01 00 01 1d 7a 01 66 16 66 18 73 03 00 00 67 75 79 62 72 75 73 68 00 | 82
SOF  CRC     ctrl  length + message (34 bytes)                                                                       EOF
```

| Field | Value | Check |
|---|---|---|
| SOF, EOF | `81`, `82` | |
| CRC | `58 d8`, low byte first: `0xD858` | recomputed by `tsn_link::encode_frame`: byte for byte equal, now a golden row in `crates/tsn-link/tests/golden/frames.txt` |
| `ctrl` | `00`: DATA, sequence 0 | the first frame after a connect |
| payload | 34 bytes, none of them `81`, `82` or `1B`, so nothing was escaped | |
| `21` | message length 33 (one-byte prefix, below `FF`) | `docs/protocol/link-layer.md` section 6.8 |

## 4. The Login message

The 33 message bytes match `docs/protocol/messages.md` section 4.2 row 1 (`b 53, b 0, w 0, b landType, b major,
b minor, b revision, w idLow, w idHigh, b fromFile, a[11] password, s name`) field for field.

| Offset | Bytes | Field | Value and source |
|---|---|---|---|
| 0 | `35` | opcode | 53 = Login |
| 1 | `00` | byte | 0 |
| 2 | `00 00` | word | 0 (target SID 0) |
| 4 | `01` | landType | 1 = Clubhouse (`LAND.CFG`: `Sierr` is land 1) |
| 5 | `02 03 12` | version | 2.3.18, the "ImagiNation v2.3" of the title bar (revision 18 is INFERRED; the Dec-93 set is labelled 2.3.17) |
| 8 | `a1 86 01 00` | idLow, idHigh | `0x000186A1` = 100001, the `id` of `LSCI.CFG` |
| 12 | `01` | fromFile | 1: the password came from `PASS_SET.DTA` (no password prompt appeared) |
| 13 | `1d 7a 01 66 16 66 18 73 03 00` | password, 10 bytes | the 10 bytes of `PASS_SET.DTA` after `AB`, sent unchanged |
| 23 | `00` | password byte 11 | 0; `messages.md` says a server must ignore it |
| 24 | `67 75 79 62 72 75 73 68 00` | name | `guybrush\0`: the **persona** name |

New facts:

- The name is the persona name chosen in the menus. The harness `LSCI.CFG` had a `name` key in its first runs
  and the Login still carried the persona; `messages.md` section 4.1 item 3 says the name comes from the
  `name` key. That key is read while `hub/script.004` export 1 sets up the SysOp object, so it is probably an
  operator or debug setting (INFERRED). A later run without the `name` key produced the identical Login.
- The password field is the stored form and is not decoded by the client. What the host does with it is
  still open.

## 5. After the Login

The client stays on the "Modem Connect" screen. `LoginTimeout` (`hub/script.101`, 70 s timer in state 0)
calls `proc_116(0x3E7)` in state 1, which shows the text of error 999. This differs from the note in
`messages.md` section 4.2 row 2 that the expiry shows "We are not receiving any messages from the network":
that message belongs to another path in `dialScript::changeState`. The alert needs the mouse; the keyboard
did not dismiss it.

## 6. The null-modem variant (what the client says to a modem)

With `serial1 = nullmodem` the server plays the modem. `hayes_fever` echoed each command and answered `OK`;
the client sent, with times from the connection start:

| t | Client sent | Reply |
|---|---|---|
| 31.155 | `\rAT\r` | `OK` (the first 31 s are the client booting and the player clicking `Play`) |
| 32.209 | `+++` (1.05 s later) | echo only; the server treats it as ordinary text in command mode |
| 33.854 | `ATZ\r` (1.65 s later: three `~` pauses) | `OK` |
| 36.055 | `AT&D2\r` | `OK` |
| 36.603 | `AT&C1\r` | `OK` |
| 37.155 | `ATV1\r` | `OK` |
| 37.704 | `ATDT5551234\r` | `CONNECT 2400` |
| 39.317 | `@`, then `D`, CR, CR, `c SIERRA\r` as in section 2 | as in section 2 |

This is the `prefix` string of `LSCI.CFG` (`+++~~~ATZ!~~~~AT&D2!~AT&C1!~ATV1!~`) followed by the `ATD` part of the
`modem` string. The `t SIERRA` part of the dial string is not sent to the modem: the driver takes the host
name out of it and uses it in `c SIERRA`. This confirms the composition `prefix`, `t`, `hostID`, `modem` of
`link-layer.md` section 3.2. The dialogue and the frame were identical with `NOBRK.DRV`, and for both drivers in
both serial modes.

## 7. Corrections made to the docs

- `docs/protocol/messages.md` section 4.2 row 2 and section 4.1 item 3 (timeout alert, persona name).
- `docs/protocol/link-layer.md` sections 3.2, 6.9 and 8 (composition confirmed, first message known, DOSBox-X
  modem emulation tested for the dial).
- `docs/server/innkeeperd.md` sections 3, 6 and 7 as numbered now (the settings work; capture lines are grouped).

No change to `pad_thai`, `tsn-link` or `innkeeper-session` was needed: the PAD dialogue, the ACK and the
wake-up guesses held on the first try. The INFERRED `DIRECT` detection and the CR-between-frames escape were
not exercised.

## 8. Next steps (after the first capture)

1. **Answer the Login**: done in the second capture (section 9).
2. **Keep it alive** until the map: done (section 10), and further into the Clubhouse (section 11).
3. **Test the escape to the PAD**: still open. Entering the Clubhouse from the map stays on the same host
   and in the same `LSCITV`, so it sent no BREAK, no `SET?` and no second `c <host>` (section 11).
4. **Dismiss the alert**: no longer needed for the logon; a failed logon still ends at the alert.
5. **Other lands and hosts**: still open.

## 9. Second capture: the logon answered

Same client, harness and modem emulation as above; `innkeeperd` now runs the host side of
`crates/innkeeper-world` (`docs/server/innkeeperd.md`). Key script `clubhouse` of `tools/dosbox/key_scripts.py`;
capture `work/captures/clubhouse-logon.hexlog` (not committed); screenshots `work/dosbox/shots/logon-1-fall-map.png`,
`logon-2-place-list.png`, `logon-3-want-to-play.png` and `logon-4-clubhouse-waiting-room.png`. The logon part (sections 9
and 10) was identical in every run after the directory fix. The bytes are the golden file `crates/innkeeper-world/tests/golden/logon.txt`;
the two framed replies are in `crates/innkeeper-session/tests/golden/host_replies.txt`.

| t | Dir | Message | Note |
|---|---|---|---|
| 5.630 | C→H | Login (section 4) | unchanged from the first capture |
| 5.630 | H→C | `00 00 00 00 16 00 00 00 00 00` | Ack, `toSID` 0, `whichCmd` 22, `userFlags` 0, `status` 0, `rating` 0; sent in the same millisecond as the link ACK, as DATA 0 |
| 5.755 | C→H | `07 00 00 00 60 01 81 01 00 00 0c 00` | joinNet for the game object: cookie `0x0160`, kind 129, land type 1, 12 properties; 125 ms after the Ack, so the Ack ended the logon timeout |
| 5.755 | H→C | `08 00 60 01 00 00 00 01` | `ObjID`: cookie echoed, SID `0x0100` |
| 6.185 | C→H | one frame of seven messages: `22 04 00 01`, `24 05`, `2d 01 00 01`, `24 01 00 00 00 00`, `24 06 s s s s`, `25 20 00 00 00 00 00 00`, `28 04 "guybrush\0"` | 34/4 rates, 36/5 host number, 45/1 mailbox, 36/1 and 36/6 with the stored stamps, 37/32 system list, 40/4 name; the SID in 34/4 and 45/1 is the one just assigned |
| 6.185 | H→C | `24 05 07 00` | host number 7 |
| 6.185 | H→C | `2f 02 …` (80 bytes) | land directory: Clubhouse, SierraLand, CasinoLand on host 7 (section 12) |
| 6.506 | C→H | `2f 01 00 00 00 00` | land occupancy request (dialScript state 5) |
| 6.506 | H→C | `2f 01 00 00 00 00 00 00 00 00 03 00` + 3 rows of `07 tt 01 40 00` | occupancy: maximum 64, current 0 |
| 7.400 | C→H | `2f 01 00 00 00 00` | the map's `ProcessLandInfo` asks again, then every 20 s |

The four requests the host left unanswered (34/4, 45/1, 37/32, 40/4) did not hold anything up. No 37/18 was
sent; INFERRED: it needs the mailbox number of `mail.cfg`, which a reply to 45/1 would have written. The stamp in 36/6 was 0 on the first run
and 1, the value the host sent, on every later run, so `landaddr.tim` round-trips.

## 10. The map

The client went to room 50, the town map ("Fall Map" in the title bar, `logon-1-fall-map.png`), right after
the occupancy reply, as `dialScript` state 7 does when no land is known yet. Keyboard play on the map (observed):

- `tab` walks the places in the order the room adds them, starting after the Clubhouse: MedievaLand, Phone,
  Post Office, Town Hall, School, Help, Airport, Exit, the water tower (credits), CasinoLand, The Mall,
  SierraLand, Clubhouse. Thirteen tabs come back to the Clubhouse; `enter` opens the place under the cursor.
  This is the order in which `TSNMap` adds its features, taken cyclically: the `add:` at `hub/script.050 TSNMap::init`
  `+0x05D7` lists SierraLand, `CCLand` (the Clubhouse, land type 1), MedievaLand and so on to the water tower,
  and CasinoLand and The Mall come later. `MapKeyCursor` starts on `CCLand` (`+0x0888`). CONFIRMED.
- Choosing the Clubhouse opens a list of the directory rows for land type 1 ("Please select where you would
  like to go:", `logon-2-place-list.png`), one button per land number with its occupancy, `0 / 64`.

The directory took two attempts, both visible only on the screen and in the client's `LANDADDR` file:

1. The first layout (no bytes between land number and versions) shifted every field by two bytes. The file
   read `ubhouse 7 1 1 000.255.255 255.000.067 108`: two bytes after the land number belong to the record.
2. With zero flags the place list said "There are no places available right now for this land.": a row whose
   flags are 0 is dropped. Flags 1 made the row appear.

## 11. Entering the Clubhouse

| t | Dir | Message | Note |
|---|---|---|---|
| 236.900 | C→H | `07 00 00 00 60 01 81 01 00 00 0c 00` | the place was chosen: joinNet for the game object again, same cookie, no leaveNet, no Login |
| 236.900 | H→C | `08 00 60 01 00 00 01 01` | SID `0x0101` |
| — | — | — | the dialog "Want To Play: Select the games which you are interested in playing this morning" (`logon-3-want-to-play.png`); `enter` pressed OK |
| 295.332 | C→H | `28 04 "guybrush\0"` | 40/4 name again |
| 296.207 | C→H | `07 00 00 00 8c 09 01 01 ff ff 1e 00` | joinNet for the player: kind 1, parameter `0xFFFF`, 30 properties |
| 296.207 | H→C | `08 00 8c 09 00 00 02 01` | SID `0x0102` |
| 296.301 | C→H | `1a 00 02 01 02 01` | command 26 for the player (meaning unknown) |
| 296.757 | C→H | `0e 00 02 01 02 01 05 00 "guybrush\0"`, `… 11 00 03 03 … 00`, `… 17 00 "maine\0"`, `0d 00 02 01 02 01` + 8 offset/value pairs | setStr name, looks and home, setInt: the persona is published as properties of its object |
| 296.840 | C→H | `07 00 00 00 c0 01 05 01 01 00 80 00` | joinNet for the waiting room: kind 5, land type 1, land number 1, at most 128 members |
| 296.840 | H→C | `08 00 c0 01 00 00 03 01` | SID `0x0103` |
| 296.944 | C→H | `0a 00 03 01 02 01 02 03 12` | add the player to the waiting room, with the version 2.3.18 |
| 296.944 | H→C | `0a 00 03 01 02 01` | `GrpJoin` to the group: member `0x0102` joined |
| 297.099 | C→H | `0d 00 02 01 02 01 09 00 78 00 18 00 12 03` | setInt: the player's current game and room |

Before the `GrpJoin` reply existed, the client sat in the Clubhouse waiting room with "Entering Clubhouse..."
and the watch cursor. With it, the status line cleared and the room became usable: title "Clubhouse | Clubhouse
v2.3 | Waiting Room", buttons Talk, Look, Invite, Watch, All, None, Go to, Options, and a people count of 0
(`logon-4-clubhouse-waiting-room.png`). That wait is `PlayerLogin::changeState` state 3 in `hub/script.120`
(`+0x00C3`..`+0x00DC`): the waiting-room group needs a SID and must contain the player. Occupancy requests went
on every 60 s instead of 20 (`hub/script.000 ProcessLandInfo::changeState +0x0025`).

Link layer: the host's DATA sequence numbers wrapped from 7 to 0 during the run and the client kept
acknowledging; no frame was resent in either direction.

## 12. Corrections from the second capture

- `docs/protocol/messages.md` section 6: a directory row has two bytes after the land number, and its flags
  must be nonzero. Sections 2.1, 3, 4.2 and 6 promote the logon facts the run proved to CONFIRMED; section 5.4
  is new (entering a land on the same host).
- `docs/server/innkeeperd.md`: the host answers the logon; the capture file shows decoded messages.
- `docs/dosbox.md`: the `clubhouse` key script, the map keyboard and the real length of a key-script pause.

## 13. Next steps

1. Answer the remaining logon requests (34/4 rates, 45/1 mailbox, 37/32 system list, 40/4 name) and see what
   the client does with the answers (`mail.cfg`, 37/18 on the next logon).
2. A second player: a world task that routes `Send`, fans `GrpJoin` out to the members and answers `getProp`,
   so two DOSBox clients meet in the waiting room.
3. A land on another host number, to exercise SwitchHost and the escape to the PAD.

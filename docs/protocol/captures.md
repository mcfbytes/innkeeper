# First live capture: the stock client against innkeeperd

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
- `docs/server/innkeeperd.md` sections 3, 5 and 6 (the settings work; capture lines are grouped).

No change to `pad_thai`, `tsn-link` or `innkeeper-session` was needed: the PAD dialogue, the ACK and the
wake-up guesses held on the first try. The INFERRED `DIRECT` detection and the CR-between-frames escape were
not exercised.

## 8. Next steps

1. **Answer the Login.** Send Ack(22) (`messages.md` section 4.2 row 3) as the first host DATA frame, after the
   1.5 s quiet period or after the client's first frame. Capture what follows: the joinNet message, then
   `34/4`, `36/5`, `45/1`, `36/1`, `36/6`, `37/18`, `37/32`, `40/4` (row 6). That checks the message catalog
   against a real client.
2. **Keep it alive**: a reply to each request in rows 5 to 10 until the client reaches the map (room 50),
   which is the first screen that needs no further host answers.
3. **Test the escape to the PAD**: a land switch makes the client send a BREAK or `+++ AT\B ATO`, then
   `SET? 0:0,32:0`, `D` and `c <host>`. This needs step 1. It settles the open question of how DOSBox-X's
   modem emulation passes these (`link-layer.md` section 8) and whether the CR heuristic is enough.
4. **Dismiss the alert** in the harness: find a keyboard route, or move the mouse. The alert blocks scripted
   runs of failed logons.
5. **Other lands and hosts**: `hostID = Sierra7` (a numeric NTN), `DIRECT`, and the SL and LL clients.

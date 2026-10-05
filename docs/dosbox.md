# DOSBox-X harness for the stock INN client

`tools/dosbox/` installs the Feb-1994 CD client into a scratch directory, runs it in DOSBox-X without a visible
window, drives its menus from a script, and saves screenshots while `innkeeperd` records the serial traffic.
Its first result is in `docs/protocol/captures.md`. Nothing here is committed except the scripts: the installed
tree, the persona, the videos and the screenshots live under `work/dosbox/`, which git ignores.

Statements marked "observed" come from runs of DOSBox-X 2026.01.02 (SDL2, Linux) on 2026-10-05.

## 1. Quick start

Needs the extracted CD set (`work/sets/inn_cd`, `work/ex/inn_cd`, see `docs/formats/install-archives.md`),
`dosbox-x` and `ffmpeg` on the path, the project virtualenv, and a built server.

```
cargo build -p innkeeperd
.venv/bin/python tools/dosbox/install_client.py --force
.venv/bin/python tools/dosbox/run_client.py --script create-persona     # once, about 4.5 minutes
.venv/bin/python tools/dosbox/run_client.py --script play               # about 2 minutes
```

`run_client.py` starts `innkeeperd` (log in `work/dosbox/innkeeperd.log`, captures in `work/captures/`), runs
DOSBox-X, stops the server and cuts the screenshots into `work/dosbox/shots/<label>_NNN.png`. A reinstall with
`--force` keeps the persona: `create-persona` saves its three files to `work/dosbox/persona/` and
`install_client.py` copies them back.

| Option | Meaning |
|---|---|
| `--serial modem\|nullmodem` | what DOSBox-X attaches to COM1 (section 5) |
| `--script NAME`, `--keys`, `--keys-wait`, `--keys-pace`, `--seconds` | a named key script, or your own (section 4) |
| `--shot-interval N` | seconds between screenshots (default 5) |
| `--label NAME` | screenshot prefix (default: the script name) |
| `--no-server`, `--server HOST:PORT` | use a server that is already running, or one on another address |
| `install_client.py --com-driver NOBRK.DRV` | install the no-break modem driver instead of `MODEM.DRV` |

To look at a run, tile the screenshots: `ffmpeg -pattern_type glob -i 'play_*.png' -vf scale=300:-1,tile=6x5
-frames:v 1 montage.png` in `work/dosbox/shots/`.

## 2. The installed client (`install_client.py`)

The tree is `work/dosbox/c/INN`, mounted as `C:\INN`. It follows `work/sets/inn_cd/INSTALL.SCR`, the script
the CD installer runs. CONFIRMED from that file; the CD's `INN.BAT` is only the installer menu.

| INSTALL.SCR step | Harness |
|---|---|
| copy executive, drivers, `TSN.PRG`, `HOSTADDR`, `LAND.CFG`, `GAME.CFG`, `LSCITV.EXE` | copied from `work/sets/inn_cd` (list in `ROOT_FILES`) |
| `puff /D _LNULL.DLL`, `_RAPH256.DLL` | decoded with `tools/unpuff.py` into `NLNULL.DLL` and `GRAPH256.DLL` |
| `defuse` of `PART.1` | root `RESOURCE.001`/`.MAP`, `SL/` and `LL/` taken from `work/ex/inn_cd` |
| `copy resource.002` into root, `SL`, `LL` | one `RESOURCE.002` copied three times |
| `echo tsnexec DEFAULT > INN.BAT` | same |
| `lsci.cfg`, `tsn.cfg`, `PASS_SET.DTA` | generated (below) |
| `lsciget lsci.cfg sl\lsci.cfg ...` | `render_land_cfg`: the keys `mouseDrv prefix id modem pFlag prodPath virtualDir swapSize LOGONVOL SEASONS` copied, driver paths prefixed with `..\`, `pathStr = ..\` and `hostID` appended |
| `VMF` directories | created in root, `SL`, `LL` (`virtualDir = VMF`) |

The games under `BARON`, `GOLF`, `YSERBIUS`, `TWINION` and `SHOPADV` are not installed; the harness only needs
the SCI client.

Generated values (a `ClientChoices` in `install_client.py`; what the installer's menus would have stored):

| File | Content | Why |
|---|---|---|
| `tsn.cfg` | `comm = MODEM.DRV : b2400 c1` | COM1, 2400 bps; format from `INSTTSN.EXE` (`docs/protocol/link-layer.md` section 3.1) |
| `LSCI.CFG` | the shipped four lines, then `video = vga320.drv`, `keyboard = ibmkbd.drv`, `music = adl.drv`, `prefix = +++~~~ATZ!~~~~AT&D2!~AT&C1!~ATV1!~`, `modem = ATDT5551234!~`, `id = 100001`, then `dll = graph256.dll`, `dll = nlnull.dll`, `pathStr = .\`, `hostID = SIERRA`, `virtualDir = VMF`, `swapSize = 100` | VGA, keyboard, AdLib, a modem init string from `MODEM.TXT`, a made-up number that the phonebook maps to the server |
| `PASS_SET.DTA` | `AB` + the 10-byte encoding of the password `SWORDFISH` | the "stored password" form, so the logon script does not ask for one (`docs/protocol/messages.md` section 4.1) |

No `mouseDrv` line is written; the client ran with the keyboard as its only input. The `name` key is not
written either: the Login message carries the persona name (`docs/protocol/captures.md` section 4).

## 3. Running without a window (`run_client.py`, `dosbox-inn.conf.in`)

- `SDL_VIDEODRIVER=offscreen` and `SDL_AUDIODRIVER=dummy` keep DOSBox-X off the desktop and off the sound
  card. Both work (observed). The settings in `dosbox-inn.conf.in`: `machine = vgaonly` (the client needs
  VGA mode 13h), `cycles = 30000`, `nosound = true`, `mpu401 = none` and `mididevice = none` (silences
  the missing-ROM warnings).
- `-silent -time-limit N` runs the `[autoexec]` section and exits. Use `-silent`: with `-fastlaunch -nogui
  -nomenu` together DOSBox-X never reached `[autoexec]` in the offscreen setup. The harness adds a 30 s margin
  and kills DOSBox-X if `-time-limit` is ignored, which happened once in about 20 runs.
- **Screenshots come from DOSBox-X's own video capture**, because the hotkeys do not exist without a window.
  `dx-capture /v /-a command /c inn.bat` records everything the client shows to
  `work/dosbox/capture/*.mts`, and `ffmpeg -vf fps=1/N` cuts that into PNGs. The capture format must be
  `mpegts-h264`: the default AVI+ZMBV capture aborts DOSBox-X with `double free or corruption` when the
  program starts a child (`TSNEXEC` runs `LSCITV`). `skip encoding unchanged frames = true` and 4:2:0 chroma
  keep the files small. A frame shows the 720 x 400 DOSBox-X output, not the raw 320 x 200.
- Key scripts are tuned by trial, in seconds from the start of the autotype command; the screenshot times
  are seconds of video.
- `ffmpeg` prints `error while decoding MB ...` for the last frame, because the video is cut off when the
  time limit hits. Harmless.

## 4. Driving the menus (key scripts)

`autotype -w WAIT -p PACE buttons...` types into the running client. Only one autotype can be active and the
wait is limited to 30 s, so `run_client.py` turns a script into a single command:

| Token | Meaning |
|---|---|
| `enter`, `tab`, `right`, `esc`, `a`, `3`, ... | one DOSBox-X button name (`autotype -list`) |
| `=text` | one button per character of `text` |
| `Ns` | a pause of N seconds: `N / pace` of the `,` buttons that autotype reads as "wait one pace" |

`tools/dosbox/key_scripts.py` holds the named scripts. They are `KeyScript(keys, wait, pace, seconds)` rows:

| Script | What it does | Run time |
|---|---|---|
| `create-persona` | welcome `OK`, "INN Guide" `No`, Create, name `guybrush`, age 30, `maine`, `Go On` through appearance, skills and hobbies, then `Save` | 260 s |
| `play` | `Play` on the "Select Player" screen; the client then dials | 120 s |

What the menus need (all observed on the Feb-94 client, no mouse):

- `enter` presses the first button of the right-hand panel (`OK`, `Yes`, `Go On`, `Play`). On the summary page
  that would be `Go Back`, so the script uses `tab` instead.
- `tab` moves the pointer to the next button; `right` moves between `Yes` and `No` in a two-button dialog.
  From the "Summary" page, three tabs reach `Save` (the order was Cancel, Options, Save).
- A first run has no persona, so "Select Player" has only `Create` enabled. The persona is stored as
  `GUYBRUSH.DTA`, `PLAYER.DIR` and `GUIDE.CFG` in the client directory. Names that start with `INN` or `TSN`
  are refused ("only ImagiNation Network employees can have names that start with ...").
- The persona's looks are random, so screenshots of it differ from run to run.
- Screens take 10 to 20 s to appear after a keypress at 30000 cycles. Scripts wait generously rather than
  reading the screen.
- The "Error #999" alert after a login timeout does not react to `enter` or `tab`; it wants the mouse. A
  script cannot dismiss it, and a run simply ends at the time limit.

Flakiness: once in about 20 runs the client stopped in the middle of typing (the screen froze and the
remaining keys had no effect). Re-run it.

## 5. COM1 and `innkeeperd`: modem emulation or null modem

Both reach the same logon. Pick with `--serial`.

| | `modem` (default) | `nullmodem` |
|---|---|---|
| DOSBox-X setting | `serial1 = modem` and `phonebookfile` mapping `5551234` to `127.0.0.1:2314` | `serial1 = nullmodem server:127.0.0.1 port:2314 transparent:1` |
| Who plays the Hayes modem | DOSBox-X: it answers `AT`, dials the mapped address over TCP and prints `CONNECT` | `innkeeperd` (`pad_thai::hayes_fever`) |
| What the server first sees | `@`, the PAD wake-up, 1.55 s after the connection | `\rAT\r` and the init string, then the dial |
| `innkeeperd --line` | `pad` or `auto` | `hayes` or `auto` |
| Driver | `MODEM.DRV` and `NOBRK.DRV` both worked | both worked |

Recommendation: use `modem`. It is the setup a player has (DOSBox-X's phonebook), the server needs no modem
play-acting, and the traffic starts at the PAD. Use `nullmodem` to test `hayes_fever` and the dial strings:
it shows the client's complete modem dialogue (`docs/protocol/captures.md` section 6).

Why they work: DOSBox-X emulates a 16550 UART in both cases, so the client's driver sees the same registers.
`MODEM.DRV` never checks the `OK` of its init commands and only waits for `CONNECT`, and the server's
`CONNECT 2400` and `@` prompts are what the driver searches for. `transparent:1` stops DOSBox-X from
inserting its own handshake text into a null-modem stream. DOSBox-X's modem telnet mode stays off, so `FF`
bytes in frames are not taken for Telnet commands.

Notes:

- `Serial1: Modem could not open TCP port 23` in `work/dosbox/dosbox-x.log` is the modem's incoming-call
  listener failing without root. It does not matter for dialing out.
- In `modem` mode the escapes `+++`, `AT H0` and a UART BREAK stay inside DOSBox-X, so the server never sees
  them (not yet exercised: the client has not tried to switch land).
- `--dial-number` changes the number in `LSCI.CFG` and the phonebook together.

#!/usr/bin/env python3
"""Run the installed INN client in headless DOSBox-X against innkeeperd and save screenshots.
See docs/dosbox.md for the setup, the key script format and what each serial mode does."""
import argparse
import os
import re
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from string import Template

from install_client import (
    CLIENT_DIR_NAME, DRIVE_C_DIR, HARNESS_DIR, PERSONA_DIR, REPO_ROOT, ClientChoices,
    install_game, launch_program_name, save_persona)
from key_scripts import KEY_SCRIPTS, KeyScript
from launcher.games import GAME_LAUNCHES

CONFIG_TEMPLATE = Path(__file__).resolve().parent / "dosbox-inn.conf.in"
DOSBOX_CAPTURE_DIR = HARNESS_DIR / "capture"
SHOTS_DIR = HARNESS_DIR / "shots"
SERVER_BINARY = REPO_ROOT / "target" / "debug" / "innkeeperd"
SERVER_ADDRESS = "127.0.0.1:2314"
SERVER_LOG = HARNESS_DIR / "innkeeperd.log"
SERVER_READY_LINE = "INT 14h hooked"
SERVER_START_TIMEOUT_SECONDS = 10.0
SERVER_POLL_SECONDS = 0.1
SERVER_STOP_WAIT_SECONDS = 5
DOSBOX_GRACE_SECONDS = 30
TIMED_OUT_STATUS = -1
DEFAULT_KEYS_WAIT_SECONDS = 20.0
DEFAULT_KEYS_PACE_SECONDS = 1.0
DEFAULT_RUN_SECONDS = 90
MAX_AUTOTYPE_WAIT_SECONDS = 30
CLIENT_COMMAND = "inn.bat"
SECONDS_TOKEN = re.compile(r"(\d+(?:\.\d+)?)s")
TEXT_PREFIX = "="
PAUSE_BUTTON = ","
SERIAL_MODES = {
    "modem": "modem",
    "nullmodem": "nullmodem server:{host} port:{port} transparent:1",
}


@dataclass(frozen=True)
class RunPlan:
    serial_mode: str
    server_address: str
    keys: str
    keys_wait: float
    keys_pace: float
    seconds: int
    label: str
    shot_interval: float
    command: str


def serial_setting(mode: str, address: str) -> str:
    host, port = address.rsplit(":", 1)
    return SERIAL_MODES[mode].format(host=host, port=port)


def expand_key_script(script: str, pace: float) -> str:
    """Turn `=text` into one button per character and `Ns` into the commas that wait N seconds."""
    buttons = []
    for token in script.split():
        wait = SECONDS_TOKEN.fullmatch(token)
        if wait:
            buttons.extend([PAUSE_BUTTON] * round(float(wait.group(1)) / pace))
        elif token.startswith(TEXT_PREFIX):
            buttons.extend("space" if char == " " else char for char in token[len(TEXT_PREFIX):])
        else:
            buttons.append(token)
    return " ".join(buttons)


def autotype_line(plan: RunPlan) -> str:
    if not plan.keys:
        return "rem no keys scripted"
    wait = min(plan.keys_wait, MAX_AUTOTYPE_WAIT_SECONDS)
    return f"autotype -w {wait} -p {plan.keys_pace} {expand_key_script(plan.keys, plan.keys_pace)}"


def write_config(plan: RunPlan, dial_number: str) -> Path:
    HARNESS_DIR.mkdir(parents=True, exist_ok=True)
    phonebook = HARNESS_DIR / "phonebook.txt"
    phonebook.write_text(f"{dial_number} {plan.server_address}\n")
    settings = {
        "capture_dir": DOSBOX_CAPTURE_DIR,
        "drive_c_dir": DRIVE_C_DIR,
        "serial1": serial_setting(plan.serial_mode, plan.server_address),
        "phonebook_file": phonebook,
        "autotype": autotype_line(plan),
        "command": plan.command,
    }
    config = HARNESS_DIR / "dosbox-inn.conf"
    config.write_text(Template(CONFIG_TEMPLATE.read_text()).substitute(settings))
    return config


def server_is_ready(server: subprocess.Popen) -> bool:
    return SERVER_READY_LINE in SERVER_LOG.read_text(errors="replace") and server.poll() is None


def start_server(address: str) -> subprocess.Popen:
    """Launch innkeeperd and wait for its startup line, which it logs once it is listening."""
    command = [str(SERVER_BINARY), "--bind", address]
    with SERVER_LOG.open("wb") as log:
        server = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, cwd=REPO_ROOT)
    deadline = time.monotonic() + SERVER_START_TIMEOUT_SECONDS
    while not server_is_ready(server):
        if server.poll() is not None or time.monotonic() > deadline:
            stop_server(server)
            raise RuntimeError(f"innkeeperd did not start listening on {address}; see {SERVER_LOG}")
        time.sleep(SERVER_POLL_SECONDS)
    return server


def stop_server(server: subprocess.Popen) -> None:
    server.terminate()
    try:
        server.wait(SERVER_STOP_WAIT_SECONDS)
    except subprocess.TimeoutExpired:
        server.kill()


def run_dosbox(config: Path, seconds: int) -> int:
    """DOSBox-X's exit status, or TIMED_OUT_STATUS when it ignored its time limit."""
    environment = {**os.environ, "SDL_VIDEODRIVER": "offscreen", "SDL_AUDIODRIVER": "dummy"}
    command = ["dosbox-x", "-conf", str(config), "-silent", "-time-limit", str(seconds)]
    with (HARNESS_DIR / "dosbox-x.log").open("wb") as log:
        try:
            completed = subprocess.run(
                command, env=environment, stdout=log, stderr=subprocess.STDOUT,
                timeout=seconds + DOSBOX_GRACE_SECONDS, check=False)
        except subprocess.TimeoutExpired:
            return TIMED_OUT_STATUS
    return completed.returncode


def extract_screenshots(plan: RunPlan) -> list[Path]:
    """One PNG every shot_interval seconds from the newest video capture."""
    videos = sorted(DOSBOX_CAPTURE_DIR.glob("*.mts"), key=lambda path: path.stat().st_mtime)
    if not videos:
        return []
    SHOTS_DIR.mkdir(parents=True, exist_ok=True)
    pattern = SHOTS_DIR / f"{plan.label}_%03d.png"
    command = ["ffmpeg", "-v", "error", "-y", "-i", str(videos[-1]),
               "-vf", f"fps=1/{plan.shot_interval}", str(pattern)]
    subprocess.run(command, check=False)
    return sorted(SHOTS_DIR.glob(f"{plan.label}_*.png"))


def clear_previous_run(label: str) -> None:
    for old in DOSBOX_CAPTURE_DIR.glob("*.mts"):
        old.unlink()
    for old in SHOTS_DIR.glob(f"{label}_*.png"):
        old.unlink()


def parse_arguments(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", choices=sorted(SERIAL_MODES), default="modem")
    parser.add_argument("--server", default=SERVER_ADDRESS, help="innkeeperd host:port")
    parser.add_argument("--no-server", action="store_true", help="innkeeperd is already running")
    parser.add_argument("--script", choices=sorted(KEY_SCRIPTS), help="named key script")
    parser.add_argument("--keys", default="", help="DOSBox-X autotype buttons, space separated")
    parser.add_argument("--keys-wait", type=float, help="seconds before the first key")
    parser.add_argument("--keys-pace", type=float, help="seconds between keys")
    parser.add_argument("--seconds", type=int, help="run time before DOSBox-X is stopped")
    parser.add_argument("--label", help="screenshot file name prefix (default: the script name)")
    parser.add_argument("--shot-interval", type=float, default=5.0,
                        help="seconds between screenshots")
    parser.add_argument("--dial-number", default=ClientChoices.dial_number)
    parser.add_argument("--launch", choices=sorted(GAME_LAUNCHES),
                        help="start this DOS game through the launcher instead of the client")
    return parser.parse_args(argv)


def client_command(launch: str | None) -> str:
    """The stock client's batch file, or TSNEXEC started on the launcher's program block."""
    if launch is None:
        return CLIENT_COMMAND
    return f"tsnexec {launch_program_name(GAME_LAUNCHES[launch])}"


def launch_label(launch: str | None) -> str:
    return f"launch-{launch.lower()}" if launch else "run"


def plan_from_arguments(args: argparse.Namespace) -> RunPlan:
    """A named script supplies the defaults; explicit options override them."""
    script = KEY_SCRIPTS.get(args.script, KeyScript(keys="", wait=DEFAULT_KEYS_WAIT_SECONDS,
                                                    pace=DEFAULT_KEYS_PACE_SECONDS,
                                                    seconds=DEFAULT_RUN_SECONDS))
    return RunPlan(
        serial_mode=args.serial,
        server_address=args.server,
        keys=args.keys or script.keys,
        keys_wait=args.keys_wait or script.wait,
        keys_pace=args.keys_pace or script.pace,
        seconds=args.seconds or script.seconds,
        label=args.label or args.script or launch_label(args.launch),
        shot_interval=args.shot_interval,
        command=client_command(args.launch),
    )


def main(argv: list[str] | None = None) -> int:
    args = parse_arguments(argv)
    client = DRIVE_C_DIR / CLIENT_DIR_NAME
    if not client.exists():
        print("no client tree; run tools/dosbox/install_client.py first", file=sys.stderr)
        return 1
    if args.launch:
        install_game(client, args.launch, ClientChoices(dial_number=args.dial_number), None)
    if not args.no_server and not SERVER_BINARY.exists():
        print("no innkeeperd binary; run `cargo build -p innkeeperd` first", file=sys.stderr)
        return 1
    plan = plan_from_arguments(args)
    DOSBOX_CAPTURE_DIR.mkdir(parents=True, exist_ok=True)
    clear_previous_run(plan.label)
    config = write_config(plan, args.dial_number)
    try:
        server = None if args.no_server else start_server(plan.server_address)
    except RuntimeError as error:
        print(error, file=sys.stderr)
        return 1
    try:
        status = run_dosbox(config, plan.seconds)
    finally:
        if server is not None:
            stop_server(server)
    shots = extract_screenshots(plan)
    if args.script and KEY_SCRIPTS[args.script].creates_persona and save_persona(client):
        print(f"persona saved under {PERSONA_DIR}")
    print(f"dosbox-x exit status {status}; {len(shots)} screenshots in {SHOTS_DIR}")
    return 0 if status == 0 else 1


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Assemble an installed INN client under work/dosbox/c/INN from the extracted CD install set.
Follows work/sets/inn_cd/INSTALL.SCR; the harness is described in docs/dosbox.md."""
import argparse
import shutil
import sys
from dataclasses import dataclass
from pathlib import Path, PureWindowsPath

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from key_scripts import PERSONA_NAME  # noqa: E402
from launcher.games import GAME_LAUNCHES, GameLaunch  # noqa: E402
from launcher.messages import Login, encode_login, encode_shared_block  # noqa: E402
from launcher.program import LaunchPlan, build_launcher  # noqa: E402
from unpuff import unpuff  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
WORK_DIR = REPO_ROOT / "work"
HARNESS_DIR = WORK_DIR / "dosbox"
PERSONA_DIR = HARNESS_DIR / "persona"
DRIVE_C_DIR = HARNESS_DIR / "c"
GAMES_DIR = WORK_DIR / "games"
CLIENT_DIR_NAME = "INN"
LAUNCHER_DIR_NAME = "LAUNCH"
PROGRAM_FILE = "TSN.PRG"
PROGRAM_FILE_LAST_BLOCK = b"program DummyDontRemove"
CRLF = "\r\n"

ROOT_FILES = (
    "TSNEXEC.EXE", "TSN.PRG", "MODEM.DRV", "NOBRK.DRV", "HOSTADDR", "TSNVER", "LSCITV.EXE",
    "GAME.CFG", "LAND.CFG", "LLSYSTEM.DAT", "TWEAKER.DRV", "MT32.DRV", "MT32.NL", "ADL.DRV",
    "SNDBLAST.DRV", "TANDYXL.DRV", "IBMKBD.DRV", "VGA320.DRV", "EGA640.DRV", "READ.ME",
)
PUFFED_DLLS = ("_RAPH256.DLL", "_LNULL.DLL")
LAND_DIRS = ("SL", "LL")
RESOURCE_FILES = ("RESOURCE.001", "RESOURCE.MAP")
SHARED_RESOURCE = "RESOURCE.002"
VIRTUAL_DIR = "VMF"
DLLS = ("graph256.dll", "nlnull.dll")
PASSWORD_KEY = b"M34546788S"
PASSWORD_FIELD_LENGTH = len(PASSWORD_KEY)
PASSWORD_FILE_MAGIC = b"AB"
PASSWORD_MIN_LENGTH = 6
SIERRA_PAD_ADDRESS = "SIERRA"
PERSONA_FILES = (f"{PERSONA_NAME.upper()}.DTA", "PLAYER.DIR", "GUIDE.CFG")


@dataclass(frozen=True)
class ClientChoices:
    """What the installer's menus would have recorded in LSCI.CFG and tsn.cfg."""

    video: str = "vga320.drv"
    keyboard: str = "ibmkbd.drv"
    music: str = "adl.drv"
    com_driver: str = "MODEM.DRV"
    com_port: int = 1
    baud: int = 2400
    modem_prefix: str = "+++~~~ATZ!~~~~AT&D2!~AT&C1!~ATV1!~"
    dial_number: str = "5551234"
    host_id: str = SIERRA_PAD_ADDRESS
    member_id: int = 100001
    password: str = "SWORDFISH"


def encode_password(password: str) -> bytes:
    """The 10-byte stored form of script.095 export 4 (docs/protocol/messages.md section 4.1)."""
    plain = password.upper().encode("ascii")
    if not PASSWORD_MIN_LENGTH <= len(plain) <= PASSWORD_FIELD_LENGTH:
        raise ValueError(
            f"password must have {PASSWORD_MIN_LENGTH} to {PASSWORD_FIELD_LENGTH} characters")
    field = bytearray(plain.ljust(PASSWORD_FIELD_LENGTH, b"\0"))
    for index, key_byte in enumerate(PASSWORD_KEY[: len(plain)]):
        field[index] ^= key_byte
    target = 1
    for source in range(len(plain)):
        field[target] ^= field[source]
        target = (target + 1) % len(plain)
    return bytes(field)


def modem_command(choices: ClientChoices) -> str:
    return f"ATDT{choices.dial_number}!~"


def dial_string(choices: ClientChoices) -> str:
    """Connect's argument as LSCI composes it (docs/protocol/messages.md section 7.1)."""
    return f"{choices.modem_prefix}t{choices.host_id}{modem_command(choices)}"


def render_tsn_cfg(choices: ClientChoices) -> str:
    return f"comm = {choices.com_driver} : b{choices.baud} c{choices.com_port}{CRLF}"


def render_lsci_cfg(choices: ClientChoices, template: str) -> str:
    """Root LSCI.CFG: the shipped template, the menu choices, then what INSTALL.SCR appends."""
    chosen = (
        f" video = {choices.video}",
        f" keyboard = {choices.keyboard}",
        f" music = {choices.music}",
        f" prefix = {choices.modem_prefix}",
        f" modem = {modem_command(choices)}",
        f" id = {choices.member_id}",
    )
    appended = (
        *(f" dll = {dll}" for dll in DLLS),
        " pathStr = .\\",
        f" hostID = {choices.host_id}",
        f" virtualDir = {VIRTUAL_DIR}",
        " swapSize = 100",
    )
    return CRLF.join((*template.splitlines(), *chosen, *appended)) + CRLF


def render_land_cfg(root_cfg: str) -> str:
    """Land LSCI.CFG: root keys that LSCIGET copies, with driver paths moved one level up."""
    path_keys = ("video", "keyboard", "music", "dll")
    carried = ("mouseDrv", "prefix", "id", "modem", "pFlag", "prodPath", "virtualDir",
               "swapSize", "LOGONVOL", "SEASONS")
    lines = []
    for line in root_cfg.splitlines():
        key = line.split("=")[0].strip()
        value = line.split("=", 1)[-1].strip()
        if key in path_keys:
            lines.append(f" {key} = ..\\{value}")
        elif key in carried:
            lines.append(line)
    lines += [" pathStr = ..\\", f" hostID = {SIERRA_PAD_ADDRESS}"]
    return CRLF.join(lines) + CRLF


def install_puffed_dlls(sets_dir: Path, client: Path) -> None:
    for packed in PUFFED_DLLS:
        name, data, _ = unpuff((sets_dir / packed).read_bytes())
        (client / name).write_bytes(data)


def install_land(extracted_dir: Path, sets_dir: Path, land: str, cfg: str, client: Path) -> None:
    source = extracted_dir / land
    target = client / land
    target.mkdir()
    for entry in source.iterdir():
        if entry.name != SHARED_RESOURCE:
            shutil.copy2(entry, target / entry.name)
    shutil.copy2(sets_dir / SHARED_RESOURCE, target / SHARED_RESOURCE)
    (target / VIRTUAL_DIR).mkdir()
    (target / "LSCI.CFG").write_text(cfg, newline="")


def install_client(
    sets_dir: Path, extracted_dir: Path, client: Path, choices: ClientChoices
) -> None:
    client.mkdir(parents=True)
    for name in ROOT_FILES:
        shutil.copy2(sets_dir / name, client / name)
    install_puffed_dlls(sets_dir, client)
    for name in RESOURCE_FILES:
        shutil.copy2(extracted_dir / name, client / name)
    shutil.copy2(sets_dir / SHARED_RESOURCE, client / SHARED_RESOURCE)
    (client / VIRTUAL_DIR).mkdir()
    (client / "INN.BAT").write_text(f"@echo off{CRLF}tsnexec DEFAULT{CRLF}", newline="")
    (client / "tsn.cfg").write_text(render_tsn_cfg(choices), newline="")
    (client / "PASS_SET.DTA").write_bytes(PASSWORD_FILE_MAGIC + encode_password(choices.password))
    template = (sets_dir / "LSCI.CFG").read_text(encoding="ascii")
    root_cfg = render_lsci_cfg(choices, template)
    (client / "LSCI.CFG").write_text(root_cfg, newline="")
    for land in LAND_DIRS:
        install_land(extracted_dir, sets_dir, land, render_land_cfg(root_cfg), client)


def launch_program_name(game: GameLaunch) -> str:
    return f"Launch{game.program}"


def launcher_dos_path(game_name: str) -> PureWindowsPath:
    return PureWindowsPath(LAUNCHER_DIR_NAME, f"{game_name}.COM")


def launcher_path(client: Path, game_name: str) -> Path:
    return client.joinpath(*launcher_dos_path(game_name).parts)


def add_launch_block(client: Path, game_name: str) -> None:
    """Insert a block that runs the launcher before the shipped file's closing dummy block."""
    program_file = client / PROGRAM_FILE
    shipped = program_file.read_bytes()
    header = f"program {launch_program_name(GAME_LAUNCHES[game_name])}"
    if header.encode("ascii") in shipped:
        return
    if PROGRAM_FILE_LAST_BLOCK not in shipped:
        raise ValueError(f"{program_file} has no closing {PROGRAM_FILE_LAST_BLOCK.decode()} block")
    block = f"{header}{CRLF}\t{launcher_dos_path(game_name)}{CRLF}{CRLF}".encode("ascii")
    program_file.write_bytes(
        shipped.replace(PROGRAM_FILE_LAST_BLOCK, block + PROGRAM_FILE_LAST_BLOCK, 1))


def install_game(client: Path, game_name: str, choices: ClientChoices,
                 shared_block: bytes | None) -> None:
    """Copy the game beside the client and build a launcher that logs in and starts it."""
    game = GAME_LAUNCHES[game_name]
    target = client / game.directory
    if not target.exists():
        shutil.copytree(GAMES_DIR / game.directory, target)
    login = Login(account=choices.member_id, encoded_password=encode_password(choices.password),
                  name=PERSONA_NAME)
    plan = LaunchPlan(
        dial=dial_string(choices),
        login=encode_login(login),
        shared_block=(encode_shared_block(game.shared_block(PERSONA_NAME))
                      if shared_block is None else shared_block),
        next_program=game.program,
    )
    launcher = launcher_path(client, game_name)
    launcher.parent.mkdir(exist_ok=True)
    launcher.write_bytes(build_launcher(plan))
    add_launch_block(client, game_name)


def save_persona(client: Path) -> bool:
    """Keep the persona files the client wrote, so a rebuilt tree can skip persona creation."""
    present = [client / name for name in PERSONA_FILES if (client / name).exists()]
    if len(present) != len(PERSONA_FILES):
        return False
    PERSONA_DIR.mkdir(parents=True, exist_ok=True)
    for path in present:
        shutil.copy2(path, PERSONA_DIR / path.name)
    return True


def restore_persona(client: Path) -> bool:
    saved = [PERSONA_DIR / name for name in PERSONA_FILES if (PERSONA_DIR / name).exists()]
    for path in saved:
        shutil.copy2(path, client / path.name)
    return bool(saved)


def parse_arguments(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--set", default="inn_cd", help="set name under work/sets and work/ex")
    parser.add_argument("--dest", type=Path, default=DRIVE_C_DIR / CLIENT_DIR_NAME)
    parser.add_argument("--dial-number", default=ClientChoices.dial_number)
    parser.add_argument("--com-driver", default=ClientChoices.com_driver,
                        choices=("MODEM.DRV", "NOBRK.DRV"))
    parser.add_argument("--force", action="store_true", help="replace an existing client tree")
    parser.add_argument("--game", choices=sorted(GAME_LAUNCHES),
                        help="also install this game from work/games and a launcher for it")
    parser.add_argument("--block", type=Path,
                        help="raw shared block to use instead of the game's hand-made one")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    """Build the client tree unless it exists; with --game, add a game to the tree either way."""
    args = parse_arguments(argv)
    choices = ClientChoices(dial_number=args.dial_number, com_driver=args.com_driver)
    if args.dest.exists() and args.force:
        shutil.rmtree(args.dest)
    if not args.dest.exists():
        install_client(WORK_DIR / "sets" / args.set, WORK_DIR / "ex" / args.set, args.dest, choices)
        restored = restore_persona(args.dest)
        print(f"installed {args.set} into {args.dest}" + ("; persona restored" if restored else ""))
    elif not args.game:
        print(f"{args.dest} exists; pass --force to rebuild it", file=sys.stderr)
        return 1
    if args.game:
        block = args.block.read_bytes() if args.block else None
        install_game(args.dest, args.game, choices, block)
        print(f"installed {args.game} and {launcher_path(args.dest, args.game)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

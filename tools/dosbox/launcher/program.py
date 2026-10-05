"""The launcher .COM: dial, log in, wait for the Ack, leave a shared block, name a game and exit.
Exports and arguments: docs/protocol/int14h-api.md section 3; the flow: docs/dosbox.md section 6."""
from dataclasses import dataclass
from enum import IntEnum

from .assembler import Data, Item, Label, Operand, assemble, branch, op

COM_ORIGIN = 0x100
ARENA_END = 0xE000
EXPORT_SLOT_SIZE = 4
BIOS_DATA_SEGMENT = 0x40
BIOS_TICK_LOW = 0x6C
BIOS_TICK_HIGH = 0x6E
BIOS_TICKS_PER_SECOND = 18.2
LOGIN_TIMEOUT_SECONDS = 70
ACK_TIMEOUT_TICKS = round(LOGIN_TIMEOUT_SECONDS * BIOS_TICKS_PER_SECOND)
DOS_INT = 0x21
DOS_PRINT = 0x09
DOS_EXIT = 0x4C
EXECUTIVE_INT = 0x14
ACK_COMMAND = 0
NAK_COMMAND = 1
WHICH_CMD_OFFSET = 4
LOGIN_WHICH_CMD = 22
SEND_QUEUED = 1
LINK_ACK_TIMEOUT_TICKS = 90


class Export(IntEnum):
    SET_SHARED_DATA = 2
    CONNECT = 3
    SEND = 4
    RECEIVE = 5
    SET_ACK_TIMEOUT = 6
    POLL = 8
    SET_NEXT_PROGRAM = 9
    GET_PREVIOUS_PROGRAM = 11
    FLUSH = 14
    SET_CALLBACKS = 15


@dataclass(frozen=True)
class Failure:
    label: str
    exit_code: int
    text: str


FAILURES = (
    Failure("no_executive", 1, "launch: TSN executive not loaded"),
    Failure("connect_failed", 2, "launch: Connect failed"),
    Failure("send_refused", 3, "launch: Send refused the Login"),
    Failure("link_failed", 4, "launch: the link failed"),
    Failure("login_refused", 5, "launch: the host refused the Login"),
    Failure("no_reply", 6, "launch: no Login Ack within 70 s"),
)


@dataclass(frozen=True)
class LaunchPlan:
    dial: str
    login: bytes
    shared_block: bytes
    next_program: str


@dataclass(frozen=True)
class FarPointer:
    label: str


Argument = int | FarPointer


def call_export(export: Export, *arguments: Argument) -> list[Item]:
    """A far cdecl call through the export table: push right to left, call, pop."""
    pushes: list[Item] = []
    words = 0
    for argument in reversed(arguments):
        if isinstance(argument, FarPointer):
            pushes += [op("push cs"), op("mov ax, {iw}", argument.label), op("push ax")]
            words += 2
        else:
            pushes += [op("mov ax, {iw}", argument), op("push ax")]
            words += 1
    call = [op("les bx, ptr [{mw}]", "table_offset"),
            op("lcall es:[bx + {ib}]", export * EXPORT_SLOT_SIZE)]
    cleanup = [op("add sp, {ib}", 2 * words)] if words else []
    return pushes + call + cleanup


def word_variables(*names: str) -> list[Item]:
    return [item for name in names for item in (Label(name), Data(bytes(2)))]


def text(label: str, value: str, terminator: str) -> list[Item]:
    return [Label(label), Data(value.encode("ascii") + terminator.encode("ascii"))]


def find_executive() -> list[Item]:
    """INT 14h returns the export table in DX:AX; a zero segment means no TSNEXEC."""
    return [
        op("xor dx, dx"),
        op("int {ib}", EXECUTIVE_INT),
        op("or dx, dx"),
        branch("je", "no_executive"),
        op("mov word ptr [{mw}], ax", "table_offset"),
        op("mov word ptr [{mw}], dx", "table_segment"),
        op("mov ax, {iw}", "arena"),
        op("mov word ptr [{mw}], ax", "arena_next"),
        *call_export(Export.SET_CALLBACKS,
                     FarPointer("allocate"), FarPointer("dereference"), FarPointer("release")),
        *call_export(Export.SET_ACK_TIMEOUT, LINK_ACK_TIMEOUT_TICKS),
        op("mov word ptr [{mw}], ax", "tick_offset"),
        op("mov word ptr [{mw}], dx", "tick_segment"),
    ]


def log_in(plan: LaunchPlan) -> list[Item]:
    """Connect as LSCI does, send the Login, then pump until the Ack or Nak for command 22."""
    return [
        *call_export(Export.CONNECT, FarPointer("dial")),
        op("or ax, ax"),
        branch("jne", "connect_failed"),
        *call_export(Export.SEND, FarPointer("login"), len(plan.login)),
        op("cmp ax, {iw}", SEND_QUEUED),
        branch("jne", "send_refused"),
        *call_export(Export.FLUSH),
        op("call {rw}", "stamp_clock"),
        op("mov word ptr [{mw}], ax", "login_sent_at"),
        Label("await_reply"),
        op("call {rw}", "poll"),
        op("or ax, ax"),
        branch("jne", "link_failed"),
        *call_export(Export.RECEIVE, FarPointer("received_offset")),
        op("or ax, ax"),
        op("jne {rb}", "inspect_reply"),
        op("call {rw}", "stamp_clock"),
        op("sub ax, word ptr [{mw}]", "login_sent_at"),
        op("cmp ax, {iw}", ACK_TIMEOUT_TICKS),
        branch("ja", "no_reply"),
        op("jmp {rb}", "await_reply"),
        Label("inspect_reply"),
        op("les bx, ptr [{mw}]", "received_offset"),
        op("cmp byte ptr es:[bx + {ib}], {ib}", WHICH_CMD_OFFSET, LOGIN_WHICH_CMD),
        op("jne {rb}", "await_reply"),
        op("cmp byte ptr es:[bx], {ib}", NAK_COMMAND),
        branch("je", "login_refused"),
        op("cmp byte ptr es:[bx], {ib}", ACK_COMMAND),
        op("jne {rb}", "await_reply"),
    ]


def hand_over(plan: LaunchPlan) -> list[Item]:
    """The first Poll acknowledges the Ack's frame; the last sends anything still queued."""
    return [
        op("call {rw}", "poll"),
        *call_export(Export.SET_SHARED_DATA, FarPointer("shared_block"), len(plan.shared_block)),
        *call_export(Export.SET_NEXT_PROGRAM, FarPointer("next_program")),
        op("call {rw}", "poll"),
        *call_export(Export.FLUSH),
    ]


def main_program(plan: LaunchPlan) -> list[Item]:
    """Run only as the first program, so a game that returns here ends the session."""
    return [
        *find_executive(),
        *call_export(Export.GET_PREVIOUS_PROGRAM),
        op("or ax, dx"),
        branch("jne", "finished"),
        *log_in(plan),
        *hand_over(plan),
        Label("finished"),
        op("mov ax, {iw}", DOS_EXIT << 8),
        op("int {ib}", DOS_INT),
    ]


def failure_exits() -> list[Item]:
    """Each failure prints its text and exits with its own code, which TSNEXEC reports."""
    items: list[Item] = []
    for failure in FAILURES:
        items += [Label(failure.label),
                  op("mov dx, {iw}", f"{failure.label}_text"),
                  op("mov al, {ib}", failure.exit_code),
                  op("jmp {rw}", "fail")]
    return items + [
        Label("fail"),
        op("push ax"),
        op("mov ah, {ib}", DOS_PRINT),
        op("int {ib}", DOS_INT),
        op("pop ax"),
        op("mov ah, {ib}", DOS_EXIT),
        op("int {ib}", DOS_INT),
    ]


def clock_routines() -> list[Item]:
    """TSNEXEC's retransmit clock only moves when the client copies the BIOS tick into it."""
    return [
        Label("stamp_clock"),
        op("push es"),
        op("mov ax, {iw}", BIOS_DATA_SEGMENT),
        op("mov es, ax"),
        op("mov dx, word ptr es:[{mw}]", BIOS_TICK_HIGH),
        op("mov ax, word ptr es:[{mw}]", BIOS_TICK_LOW),
        op("les bx, ptr [{mw}]", "tick_offset"),
        op("mov word ptr es:[bx], ax"),
        op("mov word ptr es:[bx + {ib}], dx", 2),
        op("pop es"),
        op("ret"),
        Label("poll"),
        op("call {rw}", "stamp_clock"),
        *call_export(Export.POLL),
        op("ret"),
    ]


def far_argument(word_index: int) -> Operand:
    """BP offset of an argument word once a far-called routine has pushed BP."""
    return 6 + 2 * word_index


def memory_callbacks() -> list[Item]:
    """TSNEXEC calls these with its own DS; a handle is a far pointer into the launcher's arena."""
    return [
        Label("allocate"),
        op("push bp"),
        op("mov bp, sp"),
        op("push ds"),
        op("push cs"),
        op("pop ds"),
        op("mov bx, word ptr [{mw}]", "arena_next"),
        op("mov ax, word ptr [bp + {ib}]", far_argument(0)),
        op("add ax, bx"),
        op("jb {rb}", "arena_wrap"),
        op("cmp ax, {iw}", ARENA_END),
        op("ja {rb}", "arena_wrap"),
        op("jmp {rb}", "arena_take"),
        Label("arena_wrap"),
        op("mov bx, {iw}", "arena"),
        op("mov ax, word ptr [bp + {ib}]", far_argument(0)),
        op("add ax, bx"),
        Label("arena_take"),
        op("mov word ptr [{mw}], ax", "arena_next"),
        op("mov ax, bx"),
        op("mov dx, cs"),
        op("pop ds"),
        op("pop bp"),
        op("retf"),
        Label("dereference"),
        op("push bp"),
        op("mov bp, sp"),
        op("mov ax, word ptr [bp + {ib}]", far_argument(0)),
        op("mov dx, word ptr [bp + {ib}]", far_argument(1)),
        op("pop bp"),
        op("retf"),
        Label("release"),
        op("retf"),
    ]


def data(plan: LaunchPlan) -> list[Item]:
    items = word_variables("table_offset", "table_segment", "tick_offset", "tick_segment",
                           "received_offset", "received_segment", "login_sent_at", "arena_next")
    items += text("dial", plan.dial, "\0")
    items += [Label("login"), Data(plan.login), Label("shared_block"), Data(plan.shared_block)]
    items += text("next_program", plan.next_program, "\0")
    for failure in FAILURES:
        items += text(f"{failure.label}_text", failure.text + "\r\n", "$")
    return items + [Label("arena")]


def launcher_items(plan: LaunchPlan) -> list[Item]:
    return [*main_program(plan), *failure_exits(), *clock_routines(), *memory_callbacks(),
            *data(plan)]


def build_launcher(plan: LaunchPlan) -> bytes:
    image = assemble(launcher_items(plan), COM_ORIGIN)
    if COM_ORIGIN + len(image) >= ARENA_END:
        raise ValueError("launcher leaves no room for its message arena")
    return image

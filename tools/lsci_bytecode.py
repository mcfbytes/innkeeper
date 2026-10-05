#!/usr/bin/env python3
"""LSCI bytecode: the 128-row opcode table and a linear decoder for one code item.
Operand layouts are ScummVM's SCI table with LSCI changes from docs/lsci/interpreter.md."""
import enum
import struct
from dataclasses import dataclass

SIGNED_BYTE = struct.Struct("<b")
SIGNED_WORD = struct.Struct("<h")
WORD = struct.Struct("<H")
SHORT_FORM_BIT = 1


class Operand(enum.Enum):
    BYTE = "byte"
    WORD = "word"
    VARIABLE = "variable"
    SIGNED = "signed"
    RELATIVE = "relative"
    PROPERTY = "property"
    GLOBAL = "global"
    LOCAL = "local"
    TEMP = "temp"
    PARAM = "param"


class Evidence(enum.Enum):
    CONFIRMED = "confirmed"
    SCI_BASELINE = "sci-baseline"
    INVALID = "invalid"


@dataclass(frozen=True)
class OpcodeRow:
    name: str
    operands: tuple[Operand, ...]
    evidence: Evidence


def row(name: str, *operands: Operand, evidence: Evidence = Evidence.CONFIRMED) -> OpcodeRow:
    return OpcodeRow(name, operands, evidence)


def invalid(name: str) -> OpcodeRow:
    return OpcodeRow(name, (), Evidence.INVALID)


O = Operand
STACK_OPERATORS = (
    "bnot add sub mul div mod shr shl xor and or neg not "
    "eq? ne? gt? ge? lt? le? ugt? uge? ult? ule?"
).split()
VARIABLE_KINDS = (O.GLOBAL, O.LOCAL, O.TEMP, O.PARAM)

CONTROL_OPCODES = (
    row("bt", O.RELATIVE),
    row("bnt", O.RELATIVE),
    row("jmp", O.RELATIVE),
    row("ldi", O.SIGNED),
    row("push"),
    row("pushi", O.SIGNED),
    row("toss"),
    row("dup"),
    row("link", O.VARIABLE),
    row("call", O.WORD, O.BYTE),
    row("callk", O.VARIABLE, O.BYTE),
    row("callb", O.VARIABLE, O.BYTE),
    row("calle", O.VARIABLE, O.BYTE, O.BYTE),
    row("ret"),
    row("send", O.BYTE, evidence=Evidence.SCI_BASELINE),
    row("_line_", O.WORD),
    row("_file_", O.WORD),
    row("class", O.VARIABLE),
    invalid("dummy29"),
    row("self", O.BYTE, evidence=Evidence.SCI_BASELINE),
    row("super", O.VARIABLE, O.BYTE),
    row("&rest", O.BYTE),
    row("loadID", O.SIGNED),
    row("selfID"),
    invalid("dummy2f"),
    row("pprev"),
    *(row(name, O.PROPERTY) for name in "pToa aTop pTos sTop ipToa dpToa ipTos dpTos".split()),
    invalid("lofsa"),
    row("pushID", O.SIGNED),
    row("push0"),
    row("push1"),
    row("push2"),
    row("pushSelf"),
    invalid("line"),
)


def variable_access_rows() -> tuple[OpcodeRow, ...]:
    """0x40-0x7F: operation (load, store, increment, decrement) x form x variable kind."""
    rows = []
    for operation in ("l", "s", "+", "-"):
        for form in ("a{}", "s{}", "a{}i", "s{}i"):
            for kind, letter in zip(VARIABLE_KINDS, "gltp"):
                rows.append(row(operation + form.format(letter), kind))
    return tuple(rows)


OPCODE_TABLE = (
    tuple(row(name) for name in STACK_OPERATORS) + CONTROL_OPCODES + variable_access_rows()
)
assert len(OPCODE_TABLE) == 128



def opcode_number(name: str) -> int:
    return next(number for number, entry in enumerate(OPCODE_TABLE) if entry.name == name)


BRANCH_OPCODES = frozenset(opcode_number(name) for name in ("bt", "bnt", "jmp"))
TERMINATING_OPCODES = frozenset(opcode_number(name) for name in ("jmp", "ret"))


@dataclass(frozen=True)
class Instruction:
    offset: int
    opcode: int
    short_form: bool
    operands: tuple[int, ...]
    operand_offsets: tuple[int, ...]
    size: int

    @property
    def row(self) -> OpcodeRow:
        return OPCODE_TABLE[self.opcode]

    @property
    def next_offset(self) -> int:
        return self.offset + self.size

    def branch_target(self) -> int | None:
        return self.next_offset + self.operands[0] if self.opcode in BRANCH_OPCODES else None


class DecodeError(ValueError):
    def __init__(self, offset: int, message: str):
        super().__init__(f"0x{offset:04x}: {message}")
        self.offset = offset


def operand_width(kind: Operand, short_form: bool) -> int:
    if kind is Operand.BYTE:
        return 1
    if kind is Operand.WORD:
        return 2
    return 1 if short_form else 2


def read_operand(code: bytes, position: int, kind: Operand, short_form: bool) -> int:
    width = operand_width(kind, short_form)
    signed = kind in (Operand.SIGNED, Operand.RELATIVE)
    if width == 1:
        return SIGNED_BYTE.unpack_from(code, position)[0] if signed else code[position]
    return (SIGNED_WORD if signed else WORD).unpack_from(code, position)[0]


def decode_instruction(code: bytes, offset: int) -> Instruction:
    opcode, short_form = code[offset] >> 1, bool(code[offset] & SHORT_FORM_BIT)
    entry = OPCODE_TABLE[opcode]
    if entry.evidence is Evidence.INVALID:
        raise DecodeError(offset, f"invalid opcode 0x{opcode:02x} ({entry.name})")
    position, values, positions = offset + 1, [], []
    for kind in entry.operands:
        if position + operand_width(kind, short_form) > len(code):
            raise DecodeError(offset, f"{entry.name} operand runs past the end of the item")
        values.append(read_operand(code, position, kind, short_form))
        positions.append(position)
        position += operand_width(kind, short_form)
    size = position - offset
    return Instruction(offset, opcode, short_form, tuple(values), tuple(positions), size)


def decode_code(code: bytes) -> tuple[list[Instruction], DecodeError | None]:
    """Decode a whole code item; on a bad byte stop and return what decoded plus the error."""
    instructions, offset = [], 0
    while offset < len(code):
        try:
            instruction = decode_instruction(code, offset)
        except DecodeError as error:
            return instructions, error
        instructions.append(instruction)
        offset = instruction.next_offset
    return instructions, None

#!/usr/bin/env python3
"""List every TSN kernel call in decompressed LSCI scripts and modules, with its sub-op.
A byte scan checked by a forward decode; the sub-ops are described in docs/protocol/ktsn.md."""
import argparse
import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path

from lsci_tables import opcode_mnemonic

TSN_KERNEL = 0x54
CODE_RESOURCE_TYPES = ("script", "type31")
SEND_FORMATTED_SUBOP = 4
MAX_ARGUMENT_BYTES = 160
TSN_CALLK = re.compile(b"\\x43%c(.)|\\x42%c\\x00(.)" % (TSN_KERNEL, TSN_KERNEL), re.S)

NO_OPERANDS = ""
OPERAND_LAYOUTS = {
    "bt": "v", "bnt": "v", "jmp": "v", "ldi": "v", "pushi": "v", "link": "v", "class": "v",
    "rest": "b", "lofss": "v", "lea": "v", "call": "wb", "callk": "vb", "callb": "vb",
    "calle": "vbb", "send": "b", "self": "b", "super": "vb", "info": "w", "superP": "w",
    "pToa": "v", "aTop": "v", "pTos": "v", "sTop": "v", "ipToa": "v", "dpToa": "v",
    "ipTos": "v", "dpTos": "v",
}
INVALID_IN_LSCI = {"dummy29", "dummy2f", "lofsa", "line"}
CONSTANT_PUSHES = {"push0": 0, "push1": 1, "push2": 2}
STACK_PUSHERS = {"push", "pushi", "push0", "push1", "push2", "pushSelf", "lofss", "dup", "pTos",
                 "ipTos", "dpTos"}
BINARY_OPERATORS = {"add", "sub", "mul", "div", "mod", "shr", "shl", "xor", "and", "or", "eq", "ne",
                    "gt", "ge", "lt", "le", "ugt", "uge", "ult", "ule"}
SENDS = {"send", "self", "super"}
CALLS = {"call", "callk", "callb", "calle"}
BRANCHES = {"bt", "bnt", "jmp"}


@dataclass(frozen=True)
class Instruction:
    offset: int
    mnemonic: str
    operand: int | None
    argument_bytes: int
    length: int


@dataclass(frozen=True)
class CallSite:
    resource: Path
    offset: int
    argc: int
    subop: int | None
    arguments: tuple[Instruction, ...]


def operand_width(kind: str, opcode: int) -> int:
    if kind == "v":
        return 1 if opcode & 1 else 2
    return 1 if kind == "b" else 2


def decode_instruction(code: bytes, offset: int) -> Instruction | None:
    opcode = code[offset]
    mnemonic = opcode_mnemonic(opcode)
    if mnemonic in INVALID_IN_LSCI:
        return None
    is_variable_access = (opcode >> 1) >= 0x40
    layout = "v" if is_variable_access else OPERAND_LAYOUTS.get(mnemonic, NO_OPERANDS)
    length, values = 1, []
    for kind in layout:
        width = operand_width(kind, opcode)
        values.append(int.from_bytes(code[offset + length:offset + length + width], "little"))
        length += width
    operand = values[0] if values else None
    return Instruction(offset, mnemonic, operand, values[-1] if values else 0, length)


def stack_effect(instruction: Instruction) -> int:
    """Net words pushed; variable ops are named l/s/plus/minus + a/s (acc or stack) + kind."""
    name = instruction.mnemonic
    if name in STACK_PUSHERS:
        return 1
    if name in BINARY_OPERATORS or name == "toss":
        return -1
    if name in SENDS:
        return -(instruction.argument_bytes // 2)
    if name in CALLS:
        return -(instruction.argument_bytes // 2) - 1
    if re.fullmatch(r"(l|plus|minus)s[gltp]i?", name):
        return 1
    return -1 if re.fullmatch(r"ss[gltp]i?", name) else 0


def balances_to(instructions: list[Instruction], depth: int) -> bool:
    """True when the sequence never consumes earlier stack words and leaves exactly depth."""
    if any(i.mnemonic in BRANCHES for i in instructions):
        return True
    running = 0
    for instruction in instructions:
        running += stack_effect(instruction)
        if running < 0:
            return False
    return running == depth


def decode_range(code: bytes, start: int, end: int) -> list[Instruction] | None:
    """Decode start..end and succeed only when the last instruction ends exactly at end."""
    instructions, offset = [], start
    while offset < end:
        instruction = decode_instruction(code, offset)
        if instruction is None:
            return None
        instructions.append(instruction)
        offset += instruction.length
    return instructions if offset == end else None


def constant_value(instruction: Instruction) -> int | None:
    if instruction.mnemonic in CONSTANT_PUSHES:
        return CONSTANT_PUSHES[instruction.mnemonic]
    return instruction.operand if instruction.mnemonic == "pushi" else None


def subop_of(argc_push: Instruction, subop_push: Instruction) -> int | None:
    if subop_push.mnemonic == "dup":
        return constant_value(argc_push)
    return constant_value(subop_push)


def parse_call_site(resource: Path, code: bytes, callk: int, argc: int) -> CallSite:
    """Closest preceding 'push argc, push sub-op' that decodes cleanly up to the callk."""
    for start in range(callk - 2, max(callk - MAX_ARGUMENT_BYTES, 0) - 1, -1):
        decoded = decode_range(code, start, callk)
        if not decoded or len(decoded) < 2 or constant_value(decoded[0]) != argc:
            continue
        if not balances_to(decoded, argc + 1):
            continue
        subop = subop_of(decoded[0], decoded[1])
        if subop is not None:
            return CallSite(resource, callk, argc, subop, tuple(decoded[2:]))
    return CallSite(resource, callk, argc, None, ())


def find_call_sites(resource: Path) -> list[CallSite]:
    code = resource.read_bytes()
    sites = []
    for match in TSN_CALLK.finditer(code):
        argument_bytes = (match.group(1) or match.group(2))[0]
        sites.append(parse_call_site(resource, code, match.start(), argument_bytes // 2))
    return sites


def message_code(site: CallSite) -> int | None:
    """First value after the format string of a formatted send, when it is a constant."""
    if site.subop != SEND_FORMATTED_SUBOP or len(site.arguments) < 2:
        return None
    return constant_value(site.arguments[1])


def describe_argument(instruction: Instruction) -> str:
    constant = constant_value(instruction)
    if constant is not None:
        return f"{constant:#x}"
    operand = "" if instruction.operand is None else f" {instruction.operand:#x}"
    return f"<{instruction.mnemonic}{operand}>"


def print_site(site: CallSite) -> None:
    subop = "?" if site.subop is None else f"{site.subop:#04x}"
    arguments = ", ".join(describe_argument(i) for i in site.arguments)
    print(f"{site.resource.parent.name}/{site.resource.name}:{site.offset:05x} "
          f"argc={site.argc} subop={subop} [{arguments}]")


def code_resources(root: Path) -> list[Path]:
    return sorted(path for kind in CODE_RESOURCE_TYPES for path in root.glob(f"*/{kind}.*"))


def print_message_codes(codes: Counter) -> None:
    print("\nformatted-send message codes (first value after the format string)")
    for code, count in sorted(codes.items()):
        print(f"  {code:#04x}  {count}")


def print_summary(histogram: Counter, resources: defaultdict) -> None:
    print("\nsub-op  argc  sites  resources")
    unresolved_last = sorted(histogram.items(), key=lambda item: (item[0][0] is None, item[0]))
    for (subop, argc), count in unresolved_last:
        label = "?" if subop is None else f"{subop:#04x}"
        names = sorted(resources[(subop, argc)])
        shown = ", ".join(names[:6]) + (" ..." if len(names) > 6 else "")
        print(f"{label:>6}  {argc:>4}  {count:>5}  {shown}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("set_root", type=Path, help="work/res/<set> directory holding <land>/ dirs")
    parser.add_argument("--summary-only", action="store_true")
    arguments = parser.parse_args()
    if not arguments.set_root.is_dir():
        print(f"{arguments.set_root}: not a directory", file=sys.stderr)
        return 1
    histogram, resources, codes = Counter(), defaultdict(set), Counter()
    for resource in code_resources(arguments.set_root):
        for site in find_call_sites(resource):
            histogram[(site.subop, site.argc)] += 1
            resources[(site.subop, site.argc)].add(f"{resource.parent.name}/{resource.name}")
            codes.update([message_code(site)] if message_code(site) is not None else [])
            if not arguments.summary_only:
                print_site(site)
    print_summary(histogram, resources)
    print_message_codes(codes)
    return 0


if __name__ == "__main__":
    sys.exit(main())

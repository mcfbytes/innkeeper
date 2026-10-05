#!/usr/bin/env python3
"""List every far call through the TSNEXEC INT 14h export table in a client EXE.
Finds the lookup stub and cached pointer. Exports: docs/protocol/int14h-api.md."""
import argparse
import json
import re
import struct
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path

import capstone
from capstone import x86

PARAGRAPH = 16
RELOCATION_ENTRY = 4
SEGMENT_LIMIT = 0x10000
MAX_FUNCTION_BYTES = 0x4000
MAX_INSTRUCTION_BYTES = 15
LOOKBACK_INSTRUCTIONS = 4
JUMP_TABLE_LOOKBACK = 8
CONTEXT_INSTRUCTIONS = 10
TABLE_ENTRY_SIZE = 4
LAST_EXPORT_OFFSET = 0x44
FAR_CALL_OPCODE = 0x9A
PUSH_IMMEDIATE_OPCODE = 0x68
FAR_CALL_PATTERN = re.compile(b"(?=" + bytes([FAR_CALL_OPCODE]) + b"..)", re.DOTALL)
STUB_PATTERN = re.compile(
    rb"\xa1(..)\x0b\x06(..)\x74\x01\xcb\xcd\x14\xa3(..)\x89\x16(..)\xcb", re.DOTALL)
PROLOGUE_PATTERNS = (re.compile(rb"\x55\x8b\xec"), re.compile(rb"\x55\x89\xe5"),
                     re.compile(rb"\xc8..\x00", re.DOTALL))
EXPORT_NAMES = (
    "GetStatus", "GetSharedData", "SetSharedData", "Connect", "Send", "Receive", "SetAckTimeout",
    "Disconnect", "Poll", "SetNextProgram", "Service", "GetPreviousProgram", "IsTransmitIdle",
    "SwitchHost", "Flush", "SetCallbacks", "GetLineRate")
TABLE_BASE_REGISTERS = (x86.X86_REG_BX, x86.X86_REG_SI, x86.X86_REG_DI, x86.X86_REG_BP)
POINTER_LOAD_MNEMONICS = ("mov", "push", "lea")
POP_MNEMONIC = "pop"
RETURN_MNEMONICS = ("ret", "retf", "iret", "retn")
JUMP_TABLE_MNEMONIC = "jmp"
BYTES_PER_POP = 2


def export_name(index: int) -> str:
    return EXPORT_NAMES[index] if index < len(EXPORT_NAMES) else "?"


@dataclass
class Image:
    header_size: int
    data: bytes
    relocated_words: frozenset[int]

    @staticmethod
    def load(path: Path) -> "Image":
        raw = path.read_bytes()
        (_, _, _, relocation_count, header_paragraphs, *_rest) = struct.unpack("<7H", raw[:14])
        relocation_offset = struct.unpack("<H", raw[24:26])[0]
        header_size = header_paragraphs * PARAGRAPH
        words = set()
        for index in range(relocation_count):
            entry = relocation_offset + index * RELOCATION_ENTRY
            offset, segment = struct.unpack_from("<HH", raw, entry)
            words.add(segment * PARAGRAPH + offset)
        return Image(header_size, raw[header_size:], frozenset(words))

    def file_offset(self, image_offset: int) -> int:
        return self.header_size + image_offset

    def word(self, offset: int) -> int:
        return struct.unpack_from("<H", self.data, offset)[0]


@dataclass
class SegmentMap:
    """Code segment bases found from relocated far-call targets and pushed segment:offset pairs."""
    bases: list[int]

    @staticmethod
    def from_image(image: Image) -> "SegmentMap":
        found = {segment for segment, _ in far_call_targets(image).values()}
        found.update(segment for segment, _ in address_taken_targets(image).values())
        return SegmentMap(sorted(found))

    def locate(self, image_offset: int) -> tuple[int, int]:
        paragraph = image_offset // PARAGRAPH
        candidates = [base for base in self.bases if base <= paragraph]
        base = candidates[-1] if candidates else 0
        return base, image_offset - base * PARAGRAPH

    def label(self, image_offset: int) -> str:
        segment, offset = self.locate(image_offset)
        return f"{segment:04X}:{offset:04X}"


def far_call_targets(image: Image) -> dict[int, tuple[int, int]]:
    """Image offset of each relocated `call seg:off` -> (segment, offset)."""
    targets = {}
    for match in re.finditer(FAR_CALL_PATTERN, image.data):
        site = match.start()
        if site + 3 + 2 <= len(image.data) and site + 3 in image.relocated_words:
            targets[site] = (image.word(site + 3), image.word(site + 1))
    return targets


def address_taken_targets(image: Image) -> dict[int, tuple[int, int]]:
    """Image offset of each relocated `push seg; push off` pair -> (segment, offset)."""
    targets = {}
    for word_offset in image.relocated_words:
        before = word_offset - 1
        after = word_offset + 2
        if before < 0 or after + 3 > len(image.data):
            continue
        if PUSH_IMMEDIATE_OPCODE == image.data[before] == image.data[after]:
            targets[before] = (image.word(word_offset), image.word(after + 1))
    return targets


@dataclass
class TableCall:
    site: int
    export_offset: int
    argument_bytes: int | None
    function: int | None
    context: list[str]
    via: str = "direct"

    @property
    def export_index(self) -> int:
        return self.export_offset // TABLE_ENTRY_SIZE


@dataclass
class Function:
    """Instructions are keyed by segment-relative address; `base` converts to an image offset."""
    start: int
    base: int
    instructions: dict[int, capstone.CsInsn] = field(default_factory=dict)
    touches_pointer: bool = False

    def image_offset(self, address: int) -> int:
        return self.base + address


def find_stub(image: Image) -> tuple[int, int] | None:
    for match in STUB_PATTERN.finditer(image.data):
        low, high, store_low, store_high = (struct.unpack("<H", match.group(n))[0]
                                            for n in range(1, 5))
        if low == store_low and high == store_high == low + 2:
            return match.start(), low
    return None


def find_stub_callers(image: Image, stub_offset: int) -> list[int]:
    segment, offset = stub_offset // PARAGRAPH, stub_offset % PARAGRAPH
    return [site for site, target in far_call_targets(image).items()
            if target[0] * PARAGRAPH + target[1] == segment * PARAGRAPH + offset]


def function_starts(image: Image, stub_offset: int) -> list[int]:
    starts = {stub_offset}
    for pattern in PROLOGUE_PATTERNS:
        starts.update(match.start() for match in pattern.finditer(image.data))
    code_pointers = list(far_call_targets(image).values())
    code_pointers += list(address_taken_targets(image).values())
    starts.update(segment * PARAGRAPH + offset for segment, offset in code_pointers)
    return sorted(start for start in starts if start < len(image.data))


def new_disassembler() -> capstone.Cs:
    disassembler = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_16)
    disassembler.detail = True
    return disassembler


def preceding_instructions(function: Function, address: int, count: int) -> list[capstone.CsInsn]:
    earlier = [function.instructions[a] for a in sorted(function.instructions) if a < address]
    return earlier[-count:]


def jump_table_size(function: Function, address: int) -> int:
    for instruction in reversed(preceding_instructions(function, address, JUMP_TABLE_LOOKBACK)):
        if instruction.mnemonic == "cmp" and instruction.operands[-1].type == x86.X86_OP_IMM:
            return instruction.operands[-1].imm + 1
    return 0


def jump_table_targets(image: Image, function: Function, instruction: capstone.CsInsn) -> list[int]:
    operand = instruction.operands[0]
    if operand.type != x86.X86_OP_MEM or operand.mem.segment != x86.X86_REG_CS:
        return []
    table = function.base + (operand.mem.disp & 0xFFFF)
    size = jump_table_size(function, instruction.address)
    return [image.word(table + 2 * index) for index in range(size)]


def branch_target(instruction: capstone.CsInsn) -> int | None:
    is_branch = instruction.mnemonic.startswith("j") or instruction.mnemonic in ("loop", "jcxz")
    if not is_branch or instruction.operands[0].type != x86.X86_OP_IMM:
        return None
    return instruction.operands[0].imm


def trace_function(image: Image, disassembler: capstone.Cs, start: int, limit: int,
                   segments: SegmentMap) -> Function:
    base = segments.locate(start)[0] * PARAGRAPH
    function = Function(start, base)
    pending = [start - base]
    while pending:
        address = pending.pop()
        while base + address < limit and address not in function.instructions:
            code = image.data[base + address:base + address + MAX_INSTRUCTION_BYTES]
            decoded = list(disassembler.disasm(code, address, 1))
            if not decoded or address - (start - base) > MAX_FUNCTION_BYTES:
                break
            instruction = decoded[0]
            function.instructions[address] = instruction
            pending.extend(follow_targets(image, function, instruction))
            if instruction.mnemonic in RETURN_MNEMONICS + (JUMP_TABLE_MNEMONIC,):
                break
            address += instruction.size
    return function


def follow_targets(image: Image, function: Function, instruction: capstone.CsInsn) -> list[int]:
    target = branch_target(instruction)
    if target is not None:
        return [target]
    is_jump = instruction.mnemonic == JUMP_TABLE_MNEMONIC
    if is_jump and instruction.operands[0].type == x86.X86_OP_MEM:
        return jump_table_targets(image, function, instruction)
    return []


def is_direct_memory(operand: x86.X86Op, address: int) -> bool:
    plain = operand.type == x86.X86_OP_MEM and operand.mem.base == 0 and operand.mem.index == 0
    return plain and (operand.mem.disp & 0xFFFF) == address


def touches_pointer(instruction: capstone.CsInsn, pointer: int) -> bool:
    loads_immediate = instruction.mnemonic in POINTER_LOAD_MNEMONICS
    for operand in instruction.operands:
        if is_direct_memory(operand, pointer):
            return True
        if loads_immediate and operand.type == x86.X86_OP_IMM and operand.imm == pointer:
            return True
    return False


def table_export_offset(instruction: capstone.CsInsn) -> int | None:
    if instruction.mnemonic != "lcall" or instruction.operands[0].type != x86.X86_OP_MEM:
        return None
    memory = instruction.operands[0].mem
    offset = memory.disp & 0xFFFF
    in_table = offset <= LAST_EXPORT_OFFSET and offset % TABLE_ENTRY_SIZE == 0
    based = memory.base in TABLE_BASE_REGISTERS and memory.index == 0
    return offset if in_table and based else None


def is_virtual_call(function: Function, instruction: capstone.CsInsn) -> bool:
    """A C++ virtual call reloads its base register first: `mov bx, es:[bx]; lcall [bx+N]`."""
    earlier = preceding_instructions(function, instruction.address, 1)
    if not earlier or earlier[0].mnemonic != "mov" or len(earlier[0].operands) != 2:
        return False
    destination, source = earlier[0].operands
    base = instruction.operands[0].mem.base
    return (destination.type == x86.X86_OP_REG and destination.reg == base
            and source.type == x86.X86_OP_MEM and source.mem.base == base)


def argument_bytes_after(function: Function, instruction: capstone.CsInsn) -> int | None:
    following = function.instructions.get(instruction.address + instruction.size)
    total = 0
    while following is not None and following.mnemonic == POP_MNEMONIC:
        total += BYTES_PER_POP
        following = function.instructions.get(following.address + following.size)
    if total:
        return total
    if following is not None and following.mnemonic == "add":
        return int(following.op_str[4:], 16) if following.op_str.startswith("sp, ") else None
    return None


def pointer_origin(function: Function, instruction: capstone.CsInsn, pointer: int) -> str:
    earlier = preceding_instructions(function, instruction.address, LOOKBACK_INSTRUCTIONS)
    for candidate in reversed(earlier):
        if candidate.mnemonic == "les":
            direct = is_direct_memory(candidate.operands[1], pointer)
            return "direct" if direct else "pointer-to-pointer"
    return "other"


def describe(instruction: capstone.CsInsn, base: int) -> str:
    return f"{base + instruction.address:05x} {instruction.mnemonic} {instruction.op_str}"


def find_functions(image: Image, pointer: int, stub_offset: int,
                   segments: SegmentMap) -> list[Function]:
    disassembler = new_disassembler()
    starts = function_starts(image, stub_offset)
    functions = []
    for index, start in enumerate(starts):
        limit = starts[index + 1] if index + 1 < len(starts) else len(image.data)
        function = trace_function(image, disassembler, start, limit, segments)
        function.touches_pointer = any(touches_pointer(instruction, pointer)
                                       for instruction in function.instructions.values())
        functions.append(function)
    return functions


def find_table_calls(image: Image, pointer: int, stub_offset: int | None = None) -> list[TableCall]:
    stub = stub_offset if stub_offset is not None else (find_stub(image) or (0, 0))[0]
    segments = SegmentMap.from_image(image)
    calls = []
    for function in find_functions(image, pointer, stub, segments):
        if not function.touches_pointer:
            continue
        for address in sorted(function.instructions):
            instruction = function.instructions[address]
            export_offset = table_export_offset(instruction)
            if export_offset is None or is_virtual_call(function, instruction):
                continue
            earlier = preceding_instructions(function, address, CONTEXT_INSTRUCTIONS)
            context = [describe(instruction, function.base) for instruction in earlier]
            calls.append(TableCall(function.image_offset(address), export_offset,
                                   argument_bytes_after(function, instruction),
                                   function.start, context,
                                   pointer_origin(function, instruction, pointer)))
    return calls


def callers_of(image: Image, functions: list[Function], target: int) -> list[str]:
    """Every call or address-taking reference to the function at image offset `target`."""
    segments = SegmentMap.from_image(image)
    found = []
    for site, (segment, offset) in far_call_targets(image).items():
        if segment * PARAGRAPH + offset == target:
            found.append(f"far call at {segments.label(site)}")
    for site, (segment, offset) in address_taken_targets(image).items():
        if segment * PARAGRAPH + offset == target:
            found.append(f"address taken at {segments.label(site)}")
    for function in functions:
        for address, instruction in function.instructions.items():
            if near_call_target(instruction, function.base) == target:
                found.append(f"near call at {segments.label(function.image_offset(address))}")
    return sorted(found)


def near_call_target(instruction: capstone.CsInsn, segment_base: int) -> int | None:
    if instruction.mnemonic != "call" or instruction.operands[0].type != x86.X86_OP_IMM:
        return None
    return segment_base + instruction.operands[0].imm


def group_by_export(calls: list[TableCall]) -> dict[int, list[TableCall]]:
    grouped = defaultdict(list)
    for call in calls:
        grouped[call.export_index].append(call)
    return grouped


def summarise(calls: list[TableCall], image: Image) -> None:
    segments = SegmentMap.from_image(image)
    print("\nexport  name                 sites  argbytes  containing functions")
    for index, group in sorted(group_by_export(calls).items()):
        sizes = sorted({call.argument_bytes for call in group if call.argument_bytes is not None})
        functions = sorted({call.function for call in group if call.function is not None})
        shown = " ".join(segments.label(start) for start in functions[:8])
        more = " ..." if len(functions) > 8 else ""
        print(f"+{index * TABLE_ENTRY_SIZE:02x}     {export_name(index):<20} {len(group):>5}  "
              f"{sizes}  {shown}{more}")


def report_json(image: Image, stub_offset: int, pointer: int, calls: list[TableCall]) -> str:
    segments = SegmentMap.from_image(image)
    return json.dumps({
        "stub": {"image": stub_offset, "file": image.file_offset(stub_offset), "pointer": pointer},
        "calls": [{"site": segments.label(call.site), "file": image.file_offset(call.site),
                   "export": call.export_index, "name": export_name(call.export_index),
                   "argument_bytes": call.argument_bytes,
                   "function": None if call.function is None else segments.label(call.function),
                   "via": call.via} for call in calls]}, indent=2)


def print_report(image: Image, stub_offset: int, pointer: int, calls: list[TableCall],
                 show_context: bool, show_wrappers: bool) -> None:
    segments = SegmentMap.from_image(image)
    callers = find_stub_callers(image, stub_offset)
    located = segments.locate(stub_offset)
    segment, offset = far_call_targets(image)[callers[0]] if callers else located
    print(f"stub: {segment:04X}:{offset:04X} (file {image.file_offset(stub_offset):#x}), "
          f"cached pointer DS:{pointer:#06x}")
    for caller in callers:
        print(f"stub caller: {segments.label(caller)} (file {image.file_offset(caller):#x})")
    for call in calls:
        start = "?" if call.function is None else segments.label(call.function)
        print(f"call: {segments.label(call.site)} (file {image.file_offset(call.site):#07x}) "
              f"+{call.export_offset:02x} {export_name(call.export_index)} "
              f"argbytes={call.argument_bytes} via={call.via} function={start}")
        for line in call.context if show_context else []:
            print(f"    {line}")
    summarise(calls, image)
    if show_wrappers:
        print_wrappers(image, stub_offset, pointer, calls)


def print_wrappers(image: Image, stub_offset: int, pointer: int, calls: list[TableCall]) -> None:
    segments = SegmentMap.from_image(image)
    functions = find_functions(image, pointer, stub_offset, segments)
    print("\nfunctions that call the table, and what reaches them")
    for start in sorted({call.function for call in calls if call.function is not None}):
        reached = ", ".join(callers_of(image, functions, start)) or "no static caller"
        print(f"  {segments.label(start)}: {reached}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path)
    parser.add_argument("--context", action="store_true", help="show instructions before each call")
    parser.add_argument("--wrappers", action="store_true", help="show what reaches each caller")
    parser.add_argument("--json", action="store_true", help="print the call list as JSON")
    arguments = parser.parse_args()
    image = Image.load(arguments.exe)
    stub = find_stub(image)
    if stub is None:
        print("no INT 14h lookup stub found", file=sys.stderr)
        return 1
    stub_offset, pointer = stub
    calls = find_table_calls(image, pointer, stub_offset)
    if arguments.json:
        print(report_json(image, stub_offset, pointer, calls))
    else:
        print_report(image, stub_offset, pointer, calls, arguments.context, arguments.wrappers)
    return 0


if __name__ == "__main__":
    sys.exit(main())

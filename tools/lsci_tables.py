#!/usr/bin/env python3
"""Locate and dump the kernel and bytecode-opcode dispatch tables of an unpacked LSCITV.EXE.
Findings and their meaning are documented in docs/lsci/interpreter.md."""
import argparse
import re
import struct
import sys
from collections import defaultdict, namedtuple
from dataclasses import dataclass
from pathlib import Path

MZ_HEADER = struct.Struct("<2s13H")
MzHeader = namedtuple(
    "MzHeader",
    "magic last_page_bytes pages relocation_count header_paragraphs min_alloc max_alloc "
    "ss sp checksum ip cs relocation_table overlay",
)
RELOCATION = struct.Struct("<HH")
FAR_POINTER = struct.Struct("<HH")
OPCODE_COUNT = 128
MIN_KERNEL_TABLE_LENGTH = 64

DGROUP_LOAD = re.compile(rb"\xba(..)\x8e\xda", re.S)
OPCODE_DISPATCH = re.compile(rb"\x26\xac\x8a\xd8\x81\xe3\xfe\x00\xff\xa7(..)", re.S)
OPCODE_TABLE_COPY = rb"\xbe(..)\xbf%s\xb9\x80\x00\xf3\xa5"

SCI_OPCODE_NAMES = (
    "bnot add sub mul div mod shr shl xor and or neg not eq ne gt ge lt le ugt uge ult ule "
    "bt bnt jmp ldi push pushi toss dup link call callk callb calle ret send info superP "
    "class dummy29 self super rest lea selfID dummy2f pprev pToa aTop pTos sTop ipToa dpToa "
    "ipTos dpTos lofsa lofss push0 push1 push2 pushSelf line"
).split()
VARIABLE_OPERATIONS = ("l", "s", "plus", "minus")
VARIABLE_KINDS = ("g", "l", "t", "p")


@dataclass(frozen=True)
class MzImage:
    header_size: int
    image: bytes
    relocations: list[int]
    entry: int

    def file_offset(self, linear: int) -> int:
        return self.header_size + linear


@dataclass(frozen=True)
class FarPointer:
    segment: int
    offset: int

    @property
    def linear(self) -> int:
        return self.segment * 16 + self.offset


@dataclass(frozen=True)
class KernelEntry:
    number: int
    name: str
    handler: FarPointer


@dataclass(frozen=True)
class OpcodeEntry:
    opcode: int
    mnemonic: str
    handler_offset: int


def parse_mz(path: Path) -> MzImage:
    data = path.read_bytes()
    header = MzHeader(*MZ_HEADER.unpack_from(data))
    relocations = []
    for index in range(header.relocation_count):
        position = header.relocation_table + index * RELOCATION.size
        offset, segment = RELOCATION.unpack_from(data, position)
        relocations.append(segment * 16 + offset)
    header_size = header.header_paragraphs * 16
    entry = header.cs * 16 + header.ip
    return MzImage(header_size, data[header_size:], sorted(relocations), entry)


def read_far_pointer(image: bytes, linear: int) -> FarPointer:
    offset, segment = FAR_POINTER.unpack_from(image, linear)
    return FarPointer(segment, offset)


def read_c_string(image: bytes, linear: int) -> str:
    return image[linear:image.index(b"\0", linear)].decode("latin-1")


def find_dgroup(mz: MzImage) -> int:
    match = DGROUP_LOAD.search(mz.image, mz.entry, mz.entry + 16)
    return struct.unpack("<H", match.group(1))[0]


def find_far_pointer_tables(mz: MzImage, min_length: int) -> list[int]:
    """Relocated segment words spaced 4 bytes apart mark far-pointer arrays."""
    tables, run = [], [mz.relocations[0]]
    for relocation in mz.relocations[1:]:
        if relocation - run[-1] == 4:
            run.append(relocation)
            continue
        if len(run) >= min_length:
            tables.append(run[0] - 2)
        run = [relocation]
    return tables


def read_kernel_table(mz: MzImage) -> list[KernelEntry]:
    names_table = handlers_table = None
    for table in find_far_pointer_tables(mz, MIN_KERNEL_TABLE_LENGTH):
        target = read_far_pointer(mz.image, table).linear
        if mz.image[target:target + 9] == b"IsObject\0":
            names_table = table
        else:
            handlers_table = table
    entries = []
    while True:
        handler = read_far_pointer(mz.image, handlers_table + 4 * len(entries))
        if handler.linear == 0:
            return entries
        name_pointer = read_far_pointer(mz.image, names_table + 4 * len(entries))
        name = read_c_string(mz.image, name_pointer.linear)
        entries.append(KernelEntry(len(entries), name, handler))


def opcode_mnemonic(opcode: int) -> str:
    index = opcode >> 1
    if index < len(SCI_OPCODE_NAMES):
        return SCI_OPCODE_NAMES[index]
    operation = VARIABLE_OPERATIONS[(index >> 4) & 3]
    target = "s" if index & 4 else "a"
    indexed = "i" if index & 8 else ""
    return f"{operation}{target}{VARIABLE_KINDS[index & 3]}{indexed}"


def read_opcode_table(mz: MzImage, dgroup: int) -> list[OpcodeEntry]:
    """The VM copies a static table over the live one; the static copy is read."""
    live_table = OPCODE_DISPATCH.search(mz.image).group(1)
    copy = re.search(OPCODE_TABLE_COPY % re.escape(live_table), mz.image, re.S)
    static_table = dgroup * 16 + struct.unpack("<H", copy.group(1))[0]
    offsets = struct.unpack_from(f"<{OPCODE_COUNT}H", mz.image, static_table)
    return [OpcodeEntry(i * 2, opcode_mnemonic(i * 2), offset) for i, offset in enumerate(offsets)]


def print_kernel_table(mz: MzImage, kernels: list[KernelEntry]) -> None:
    print(f"kernel table: {len(kernels)} entries")
    for kernel in kernels:
        handler = kernel.handler
        file_offset = mz.file_offset(handler.linear)
        print(f"  {kernel.number:02x} {kernel.name:<18} "
              f"{handler.segment:04x}:{handler.offset:04x}  file {file_offset:05x}")


def print_opcode_table(opcodes: list[OpcodeEntry]) -> None:
    sharers = defaultdict(list)
    for entry in opcodes:
        sharers[entry.handler_offset].append(entry.opcode)
    print("opcode table (even opcode; odd byte-operand form shares the handler)")
    for entry in opcodes:
        others = [f"{op:02x}" for op in sharers[entry.handler_offset] if op != entry.opcode]
        shared = f"  shared with {','.join(others)}" if others else ""
        print(f"  {entry.opcode:02x} {entry.mnemonic:<9} vm:{entry.handler_offset:04x}{shared}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path, help="LZEXE-unpacked LSCITV.EXE")
    arguments = parser.parse_args()
    try:
        mz = parse_mz(arguments.exe)
        dgroup = find_dgroup(mz)
        kernels = read_kernel_table(mz)
        opcodes = read_opcode_table(mz, dgroup)
    except (OSError, AttributeError, TypeError, ValueError, struct.error) as error:
        print(f"{arguments.exe}: tables not found ({error})", file=sys.stderr)
        return 1
    print(f"DGROUP {dgroup:04x}, header {mz.header_size:#x} bytes")
    print_kernel_table(mz, kernels)
    print_opcode_table(opcodes)
    return 0


if __name__ == "__main__":
    sys.exit(main())

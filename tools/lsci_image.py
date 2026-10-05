#!/usr/bin/env python3
"""Build the SCI0 script image that the ScummVM fork makes from an LSCI item container.
Byte copy of the fork's LsciScriptImageWriter; layout in docs/lsci/script-loader.md."""
import argparse
import enum
import hashlib
import struct
import sys
from dataclasses import dataclass
from pathlib import Path

import lsci_bytecode as bytecode
import lsci_format as fmt

BLOCK_HEADER = struct.Struct("<HH")
OBJECT_HEADER = struct.Struct("<HHHH")
WORD = struct.Struct("<H")
OBJECT_MAGIC = 0x1234
IMAGE_LIMIT = 0x10000


class BlockType(enum.IntEnum):
    """ScummVM's SCI0 block types (ScriptObjectTypes in engines/sci/engine/script.h)."""

    TERMINATOR = 0
    OBJECT = 1
    CODE = 2
    STRINGS = 5
    CLASS = 6
    EXPORTS = 7
    POINTERS = 8
    LOCALVARS = 10


class ImageOpcode(enum.IntEnum):
    """ScummVM opcodes that LSCI instructions are rewritten to."""

    LDI = 0x1A
    PUSHI = 0x1C
    CALL = 0x20
    LOFSA = 0x39
    LOFSS = 0x3A
    PUSH_SELF = 0x3E
    LINE = 0x3F


LINE_MARKERS = frozenset(bytecode.opcode_number(name) for name in ("_line_", "_file_"))
CALL = bytecode.opcode_number("call")
LOAD_ID = bytecode.opcode_number("loadID")
PUSH_ID = bytecode.opcode_number("pushID")
PUSH_SELF = bytecode.opcode_number("pushSelf")
OBJECT_PROPERTIES_OFFSET = BLOCK_HEADER.size + OBJECT_HEADER.size
STRING_TEXT_OFFSET = fmt.STRING_HEADER.size


class ImageError(ValueError):
    pass


@dataclass(frozen=True)
class StrayBranch:
    item: int
    offset: int
    target: int


@dataclass(frozen=True)
class ScriptImage:
    data: bytes
    item_offsets: tuple[int | None, ...]
    block_offsets: tuple[int | None, ...]
    stray_branches: tuple[StrayBranch, ...]


def padded(size: int) -> int:
    return size + (size & 1)


def last_variables_index(module: fmt.Module) -> int | None:
    indices = [item.index for item in module.items_of(fmt.ItemTag.VARIABLES)]
    return indices[-1] if indices else None


def object_body_size(item: fmt.Item) -> int:
    record = fmt.parse_object(item)
    if len(record.properties) <= fmt.ObjectSlot.NAME:
        raise ImageError(f"item {item.index}: object without the leading slots up to name")
    property_words = len(record.properties) * (2 if record.is_class else 1)
    method_words = 2 + 2 * len(record.method_selectors)
    return OBJECT_HEADER.size + WORD.size * (property_words + method_words)


def body_size(item: fmt.Item, locals_index: int | None) -> int | None:
    """Size of the block body an item becomes, or None when it has no block of its own."""
    tag = item.kind
    if tag in (fmt.ItemTag.OBJECT, fmt.ItemTag.CLASS):
        return object_body_size(item)
    if tag in (fmt.ItemTag.CODE, fmt.ItemTag.VARIABLES):
        return len(item.payload) if tag is fmt.ItemTag.CODE or item.index == locals_index else None
    if tag is fmt.ItemTag.STRING:
        return len(item.payload) - STRING_TEXT_OFFSET
    if tag is fmt.ItemTag.DISPATCH_TABLE:
        return WORD.size * (1 + 2 * (item.word(0) + 1))
    if tag is fmt.ItemTag.PROPERTY_DICTIONARY:
        return None
    raise ImageError(f"item {item.index}: unsupported {item.tag_name}")


def reference_offset(tag: fmt.ItemTag | None) -> int:
    """Distance from a block's start to the address that references to its item resolve to."""
    if tag in (fmt.ItemTag.OBJECT, fmt.ItemTag.CLASS):
        return OBJECT_PROPERTIES_OFFSET
    return BLOCK_HEADER.size


class ImageWriter:
    def __init__(self, module: fmt.Module):
        self.module = module
        self.locals_index = last_variables_index(module)
        self.block_offsets: list[int | None] = []
        self.item_offsets: list[int | None] = []
        self.relocations: list[int] = []
        self.stray_branches: list[StrayBranch] = []

    def write(self) -> ScriptImage:
        end = self.layout()
        image = bytearray()
        for item, block_offset in zip(self.module.items, self.block_offsets):
            if block_offset is not None:
                image += self.block(item, block_offset)
        relocation_body = struct.pack(f"<H{len(self.relocations)}H", len(self.relocations),
                                      *self.relocations)
        image += BLOCK_HEADER.pack(BlockType.POINTERS, BLOCK_HEADER.size + len(relocation_body))
        image += relocation_body + WORD.pack(BlockType.TERMINATOR)
        assert len(image) == end + BLOCK_HEADER.size + len(relocation_body) + WORD.size
        if len(image) > IMAGE_LIMIT:
            raise ImageError(f"image of {len(image)} bytes exceeds a 16-bit segment")
        return ScriptImage(bytes(image), tuple(self.item_offsets), tuple(self.block_offsets),
                           tuple(self.stray_branches))

    def layout(self) -> int:
        position = 0
        for item in self.module.items:
            size = body_size(item, self.locals_index)
            if size is None:
                self.block_offsets.append(None)
                self.item_offsets.append(None)
                continue
            self.block_offsets.append(position)
            self.item_offsets.append(position + reference_offset(item.kind))
            position += padded(BLOCK_HEADER.size + size)
        return position

    def target(self, item: fmt.Item, payload_offset: int) -> int:
        number = item.word(payload_offset)
        if number >= len(self.item_offsets) or self.item_offsets[number] is None:
            raise ImageError(f"item {item.index}: reference at 0x{payload_offset:x} to item "
                             f"{number}, which has no block")
        return self.item_offsets[number]

    def resolved(self, item: fmt.Item, payload_offset: int, image_offset: int | None) -> int:
        """A payload word with its item reference resolved; relocated when image_offset is set."""
        if not item.is_reference(payload_offset):
            return item.word(payload_offset)
        if image_offset is not None:
            self.relocations.append(image_offset)
        return self.target(item, payload_offset)

    def block(self, item: fmt.Item, block_offset: int) -> bytes:
        body_offset = block_offset + BLOCK_HEADER.size
        tag = item.kind
        if tag in (fmt.ItemTag.OBJECT, fmt.ItemTag.CLASS):
            body = self.object_body(item, body_offset)
            block_type = BlockType.CLASS if tag is fmt.ItemTag.CLASS else BlockType.OBJECT
        elif tag is fmt.ItemTag.CODE:
            body, block_type = self.code_body(item, body_offset), BlockType.CODE
        elif tag is fmt.ItemTag.STRING:
            body, block_type = item.payload[STRING_TEXT_OFFSET:], BlockType.STRINGS
        elif tag is fmt.ItemTag.VARIABLES:
            body, block_type = self.variables_body(item, body_offset), BlockType.LOCALVARS
        else:
            body, block_type = self.exports_body(item), BlockType.EXPORTS
        size = padded(BLOCK_HEADER.size + len(body))
        return BLOCK_HEADER.pack(block_type, size) + body + bytes(size - BLOCK_HEADER.size - len(body))

    def object_body(self, item: fmt.Item, body_offset: int) -> bytes:
        record = fmt.parse_object(item)
        size = len(record.properties)
        properties_offset = body_offset + OBJECT_HEADER.size
        words = []
        for slot in range(size):
            if slot == fmt.ObjectSlot.DICT and not record.is_class:
                words.append(record.super_class)
            else:
                words.append(self.resolved(item, WORD.size * slot,
                                           properties_offset + WORD.size * slot))
        if record.is_class:
            dictionary = self.module.items[item.index + 1]
            words += fmt.parse_property_dictionary(dictionary, size)
        function_area = WORD.size * (len(words) + 1)
        words.append(len(record.method_selectors))
        words += record.method_selectors
        words.append(0)
        code_offset = WORD.size * (size + 1 + len(record.method_selectors))
        words += [self.resolved(item, code_offset + WORD.size * n, None)
                  for n in range(len(record.method_selectors))]
        header = OBJECT_HEADER.pack(OBJECT_MAGIC, 0, function_area, size)
        return header + struct.pack(f"<{len(words)}H", *words)

    def variables_body(self, item: fmt.Item, body_offset: int) -> bytes:
        count = len(item.payload) // WORD.size
        words = [self.resolved(item, WORD.size * k, body_offset + WORD.size * k)
                 for k in range(count)]
        return struct.pack(f"<{count}H", *words)

    def exports_body(self, item: fmt.Item) -> bytes:
        count = item.word(0) + 1
        words = [count]
        for export in range(count):
            offset = WORD.size * (1 + export)
            words += [self.target(item, offset) if item.is_reference(offset) else 0, 0]
        return struct.pack(f"<{len(words)}H", *words)

    def code_body(self, item: fmt.Item, body_offset: int) -> bytes:
        instructions, error = bytecode.decode_code(item.payload)
        if error:
            raise ImageError(f"item {item.index}: {error}")
        code = bytearray(item.payload)
        rewritten: list[int] = []
        for instruction in instructions:
            rewritten += self.rewrite(item, instruction, code, body_offset)
            target = instruction.branch_target()
            if target is not None and not 0 <= target < len(code):
                self.stray_branches.append(StrayBranch(item.index, instruction.offset, target))
        if len(rewritten) != len(item.fixups):
            raise ImageError(f"item {item.index}: {len(item.fixups) - len(rewritten)} fixups are "
                             f"not operands of call, loadID or pushID")
        return bytes(code)

    def rewrite(self, item: fmt.Item, instruction: bytecode.Instruction, code: bytearray,
                body_offset: int) -> list[int]:
        """Rewrite one instruction in place; returns the payload offsets of the references used."""
        opcode, at = instruction.opcode, instruction.offset
        operand = at + 1
        if opcode == CALL:
            if not item.is_reference(operand):
                raise ImageError(f"item {item.index}: call at 0x{at:x} names no item")
            relative = self.target(item, operand) - (body_offset + instruction.next_offset)
            code[at] = ImageOpcode.CALL << 1
            WORD.pack_into(code, operand, relative & 0xFFFF)
            return [operand]
        if opcode in (LOAD_ID, PUSH_ID):
            if not instruction.short_form and item.is_reference(operand):
                code[at] = (ImageOpcode.LOFSA if opcode == LOAD_ID else ImageOpcode.LOFSS) << 1
                WORD.pack_into(code, operand, self.target(item, operand))
                return [operand]
            immediate = ImageOpcode.LDI if opcode == LOAD_ID else ImageOpcode.PUSHI
            code[at] = immediate << 1 | instruction.short_form
        elif opcode in LINE_MARKERS:
            code[at] = ImageOpcode.LINE << 1
        elif opcode == PUSH_SELF:
            code[at] = ImageOpcode.PUSH_SELF << 1
        return []


def build_image(module: fmt.Module) -> ScriptImage:
    return ImageWriter(module).write()


def script_number(prefix: str, number: int) -> int:
    return number + fmt.MODULE_SCRIPT_BASE if prefix == fmt.MODULE_PREFIX else number


@dataclass(frozen=True)
class Conversion:
    land: Path
    number: int
    image: ScriptImage | None
    problem: str | None


def convert_land(land: Path) -> list[Conversion]:
    resources = fmt.ResourceDir(land)
    results = []
    for prefix in (fmt.SCRIPT_PREFIX, fmt.MODULE_PREFIX):
        for number in resources.numbers(prefix):
            results.append(convert_one(land, script_number(prefix, number),
                                       resources.read(prefix, number)))
    return sorted(results, key=lambda result: result.number)


def convert_one(land: Path, number: int, data: bytes) -> Conversion:
    try:
        module = fmt.parse_module(data)
        if module.trailing_bytes:
            raise fmt.FormatError(f"{module.trailing_bytes} bytes after the last item")
        return Conversion(land, number, build_image(module), None)
    except (fmt.FormatError, ImageError) as error:
        return Conversion(land, number, None, str(error))


def land_dirs(root: Path) -> list[Path]:
    if fmt.is_resource_dir(root):
        return [root]
    return sorted(path.parent for path in root.rglob(f"{fmt.VOCAB_PREFIX}.{fmt.SELECTOR_NAMES_VOCAB}"))


def print_check(conversions: list[Conversion]) -> int:
    converted = [result for result in conversions if result.image]
    for result in conversions:
        if result.problem:
            print(f"{result.land}: script {result.number}: not converted: {result.problem}")
        for stray in result.image.stray_branches if result.image else ():
            print(f"{result.land}: script {result.number}: item {stray.item} branch at "
                  f"0x{stray.offset:04x} leaves its item (target {stray.target:+d})")
    largest = max(converted, key=lambda result: len(result.image.data))
    print(f"{len(converted)} converted, {len(conversions) - len(converted)} not converted; "
          f"largest image {len(largest.image.data)} bytes ({largest.land} script {largest.number})")
    return 0


def print_md5(conversions: list[Conversion]) -> int:
    for result in conversions:
        if result.image:
            digest = hashlib.md5(result.image.data).hexdigest()
            print(f"{result.number} {len(result.image.data)} {digest}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("path", type=Path, help="land directory or a tree of them")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="convert all and report problems")
    mode.add_argument("--md5", action="store_true",
                      help="print 'script size md5' per image, as verify_scripts does")
    args = parser.parse_args()
    lands = land_dirs(args.path)
    if not lands:
        print(f"{args.path}: no land directory found", file=sys.stderr)
        return 1
    conversions = [result for land in lands for result in convert_land(land)]
    return print_check(conversions) if args.check else print_md5(conversions)


if __name__ == "__main__":
    sys.exit(main())

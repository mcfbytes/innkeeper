#!/usr/bin/env python3
"""Parse LSCI script and module resources (the item container) and the vocab resources they use.
The format and its evidence are described in docs/lsci/script-format.md."""
import enum
import struct
from dataclasses import dataclass
from functools import cached_property
from pathlib import Path

WORD = struct.Struct("<H")
ITEM_HEADER = struct.Struct("<BH")
STRING_HEADER = struct.Struct("<HH")
OPCODE_NAME_HEADER = struct.Struct("<HH")
CLASS_TABLE_ENTRY = struct.Struct("<HH")

SCRIPT_PREFIX = "script"
MODULE_PREFIX = "type31"
VOCAB_PREFIX = "vocab"
KERNEL_PROPERTY_OFFSETS_VOCAB = 994
CLASS_TABLE_VOCAB = 996
SELECTOR_NAMES_VOCAB = 997
OPCODE_NAMES_VOCAB = 998

# Script numbers 0xE800 + n name type-31 module n; how the key is formed is INFERRED.
MODULE_SCRIPT_BASE = 0xE800
NO_SUPER_CLASS = 0xFFFF


class ItemTag(enum.IntEnum):
    """Item tags are the interpreter's memory-block types (name table at DGROUP:024A)."""

    OBJECT = 2
    CLASS = 3
    CODE = 4
    VARIABLES = 6
    STRING = 7
    SAID_SPEC = 8
    DISPATCH_TABLE = 9
    SYNONYM_TABLE = 10
    PROPERTY_DICTIONARY = 11


TAGS_WITHOUT_FIXUPS = frozenset({ItemTag.STRING, ItemTag.SAID_SPEC, ItemTag.SYNONYM_TABLE})


class ObjectSlot(enum.IntEnum):
    """Leading property slots shared by every object and class."""

    ENV = 0
    SIZE = 1
    DICT = 2
    SUPER = 3
    INFO = 4
    NAME = 5


class FormatError(ValueError):
    pass


@dataclass(frozen=True)
class Item:
    index: int
    tag: int
    payload: bytes
    fixups: tuple[int, ...]
    file_offset: int

    @property
    def kind(self) -> ItemTag | None:
        return ItemTag(self.tag) if self.tag in ItemTag._value2member_map_ else None

    @property
    def tag_name(self) -> str:
        return self.kind.name if self.kind else f"TAG_{self.tag:02X}"

    def word(self, byte_offset: int) -> int:
        return WORD.unpack_from(self.payload, byte_offset)[0]

    def words(self, count: int, byte_offset: int = 0) -> tuple[int, ...]:
        return struct.unpack_from(f"<{count}H", self.payload, byte_offset)

    def is_reference(self, byte_offset: int) -> bool:
        return byte_offset in self.fixups


@dataclass(frozen=True)
class Module:
    items: tuple[Item, ...]
    trailing_bytes: int

    def items_of(self, tag: ItemTag) -> list[Item]:
        return [item for item in self.items if item.tag == tag]


def parse_module(data: bytes) -> Module:
    """Split a resource into its numbered items; raises FormatError if it is not a container."""
    if len(data) < WORD.size:
        raise FormatError("resource shorter than the item count")
    count = WORD.unpack_from(data, 0)[0]
    position = WORD.size
    items = []
    for index in range(count):
        item, position = parse_item(data, position, index)
        items.append(item)
    return Module(tuple(items), len(data) - position)


def parse_item(data: bytes, header_offset: int, index: int) -> tuple[Item, int]:
    """Item: tag byte, payload length word, payload, then fixups unless the tag has none."""
    if header_offset + ITEM_HEADER.size > len(data):
        raise FormatError(f"item {index}: header past end at 0x{header_offset:x}")
    tag, length = ITEM_HEADER.unpack_from(data, header_offset)
    payload_start = header_offset + ITEM_HEADER.size
    payload = data[payload_start : payload_start + length]
    if len(payload) != length:
        raise FormatError(f"item {index}: payload of {length} bytes runs past end")
    fixups, next_offset = parse_fixups(data, payload_start + length, tag, length, index)
    return Item(index, tag, payload, fixups, header_offset), next_offset


def parse_fixups(
    data: bytes, position: int, tag: int, length: int, index: int
) -> tuple[tuple[int, ...], int]:
    """Fixups: count, then payload offsets of words that hold item numbers."""
    if tag in TAGS_WITHOUT_FIXUPS:
        return (), position
    if position + WORD.size > len(data):
        raise FormatError(f"item {index}: fixup count past end")
    count = WORD.unpack_from(data, position)[0]
    end = position + WORD.size * (count + 1)
    if end > len(data):
        raise FormatError(f"item {index}: {count} fixups run past end")
    offsets = struct.unpack_from(f"<{count}H", data, position + WORD.size)
    if any(offset + WORD.size > length for offset in offsets):
        raise FormatError(f"item {index}: fixup outside its {length}-byte payload")
    return offsets, end


@dataclass(frozen=True)
class ObjectRecord:
    item: Item
    properties: tuple[int, ...]
    method_selectors: tuple[int, ...]
    method_code_items: tuple[int, ...]

    @property
    def is_class(self) -> bool:
        return self.item.tag == ItemTag.CLASS

    @property
    def species(self) -> int:
        return self.properties[ObjectSlot.DICT]

    @property
    def super_class(self) -> int:
        return self.properties[ObjectSlot.SUPER]

    def property_is_reference(self, index: int) -> bool:
        return self.item.is_reference(index * WORD.size)


def parse_object(item: Item) -> ObjectRecord:
    """Object and class items: properties, then {count, selectors[count], code items[count]}."""
    size = item.word(ObjectSlot.SIZE * WORD.size)
    method_offset = size * WORD.size
    method_count = item.word(method_offset)
    expected = method_offset + WORD.size * (1 + 2 * method_count)
    if expected != len(item.payload):
        raise FormatError(f"item {item.index}: object layout needs {expected} bytes")
    selectors = item.words(method_count, method_offset + WORD.size)
    code_items = item.words(method_count, method_offset + WORD.size * (1 + method_count))
    return ObjectRecord(item, item.words(size), selectors, code_items)


def parse_property_dictionary(item: Item, property_count: int) -> tuple[int, ...]:
    """Selector of each property; INFERRED: bytes after them are compiler slack."""
    if property_count * WORD.size > len(item.payload):
        raise FormatError(f"item {item.index}: dictionary shorter than {property_count} entries")
    return item.words(property_count)


def parse_string(item: Item) -> str:
    _kind, _declared_size = STRING_HEADER.unpack_from(item.payload, 0)
    return item.payload[STRING_HEADER.size :].split(b"\0", 1)[0].decode("latin-1")


@dataclass(frozen=True)
class WordCell:
    number: int
    value: int
    is_reference: bool


def word_cells(
    item: Item, first_offset: int, count: int, first_number: int
) -> tuple[WordCell, ...]:
    if first_offset + WORD.size * count > len(item.payload):
        raise FormatError(f"item {item.index}: {count} words do not fit")
    offsets = range(first_offset, first_offset + WORD.size * count, WORD.size)
    return tuple(
        WordCell(first_number + n, item.word(offset), item.is_reference(offset))
        for n, offset in enumerate(offsets)
    )


def parse_variables(item: Item) -> tuple[WordCell, ...]:
    """Slot 0 holds the slot count n; returns slots 1..n-1. Words after slot n-1 are slack."""
    return word_cells(item, WORD.size, item.word(0) - 1, 1)


def parse_dispatch_table(item: Item) -> tuple[WordCell, ...]:
    """Highest export number, then one item reference per export."""
    return word_cells(item, WORD.size, item.word(0) + 1, 0)


@dataclass(frozen=True)
class Vocabulary:
    selector_names: tuple[str, ...]
    class_scripts: tuple[int, ...]
    opcode_names: tuple[str, ...]
    kernel_property_offsets: tuple[int, ...]

    def selector_name(self, selector: int) -> str:
        if selector < len(self.selector_names):
            return self.selector_names[selector]
        return f"selector_{selector:x}"


def parse_name_table(data: bytes) -> tuple[str, ...]:
    """vocab.997: highest selector number n, offsets[n + 1], each name stored as {length, bytes}."""
    count = WORD.unpack_from(data, 0)[0] + 1
    offsets = struct.unpack_from(f"<{count}H", data, WORD.size)
    names = []
    for offset in offsets:
        length = WORD.unpack_from(data, offset)[0]
        names.append(data[offset + WORD.size : offset + WORD.size + length].decode("latin-1"))
    return tuple(names)


def parse_opcode_names(data: bytes) -> tuple[str, ...]:
    """vocab.998: count, offsets[count], each entry {length + 2, type, name}."""
    count = WORD.unpack_from(data, 0)[0]
    offsets = struct.unpack_from(f"<{count}H", data, WORD.size)
    names = []
    for offset in offsets:
        stored_length, _type = OPCODE_NAME_HEADER.unpack_from(data, offset)
        start = offset + OPCODE_NAME_HEADER.size
        names.append(data[start : start + stored_length - WORD.size].decode("latin-1"))
    return tuple(names)


def parse_class_table(data: bytes) -> tuple[int, ...]:
    """vocab.996: per class {handle slot (0 on disk), script number}."""
    return tuple(script for _slot, script in CLASS_TABLE_ENTRY.iter_unpack(data))


def parse_word_table(data: bytes) -> tuple[int, ...]:
    return tuple(value for (value,) in WORD.iter_unpack(data[: len(data) & ~1]))


def resource_for_script_number(script_number: int) -> tuple[str, int]:
    if script_number >= MODULE_SCRIPT_BASE:
        return MODULE_PREFIX, script_number - MODULE_SCRIPT_BASE
    return SCRIPT_PREFIX, script_number


class ResourceDir:
    """One land directory of decompressed resources named <type>.<number>."""

    def __init__(self, path: Path):
        self.path = path

    def numbers(self, prefix: str) -> list[int]:
        return sorted(int(file.suffix[1:]) for file in self.path.glob(f"{prefix}.*"))

    def read(self, prefix: str, number: int) -> bytes | None:
        file = self.path / f"{prefix}.{number:03d}"
        return file.read_bytes() if file.exists() else None

    @cached_property
    def vocabulary(self) -> Vocabulary:
        return Vocabulary(
            self._parsed(SELECTOR_NAMES_VOCAB, parse_name_table),
            self._parsed(CLASS_TABLE_VOCAB, parse_class_table),
            self._parsed(OPCODE_NAMES_VOCAB, parse_opcode_names),
            self._parsed(KERNEL_PROPERTY_OFFSETS_VOCAB, parse_word_table),
        )

    def _parsed(self, number: int, parser) -> tuple:
        data = self.read(VOCAB_PREFIX, number)
        return parser(data) if data else ()


def is_resource_dir(path: Path) -> bool:
    return (path / f"{VOCAB_PREFIX}.{SELECTOR_NAMES_VOCAB}").exists()

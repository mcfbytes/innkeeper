#!/usr/bin/env python3
"""Disassemble LSCI scripts and type-31 modules: objects, classes, properties and method bytecode.
Container format: docs/lsci/script-format.md. Opcode semantics: docs/lsci/interpreter.md."""
import argparse
import sys
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path

from lsci_bytecode import OPCODE_TABLE, TERMINATING_OPCODES, Instruction, Operand, decode_code
from lsci_format import (
    MODULE_PREFIX,
    NO_SUPER_CLASS,
    SCRIPT_PREFIX,
    FormatError,
    Item,
    ItemTag,
    Module,
    ObjectRecord,
    ObjectSlot,
    ResourceDir,
    WordCell,
    is_resource_dir,
    parse_dispatch_table,
    parse_module,
    parse_object,
    parse_property_dictionary,
    parse_string,
    parse_variables,
    resource_for_script_number,
)
from lsci_land import Land, classes_in, object_name

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_KERNEL_NAMES = REPO_ROOT / "work/exe/LSCITV_inn_cd.kernel.txt"
KERNEL_NUMBERING_NOTE = "kernel names follow the LSCITV name table order (provisional)"
CODE_RESOURCE_PREFIXES = (SCRIPT_PREFIX, MODULE_PREFIX)
MAX_QUOTED_STRING = 60
SMALL_NUMBER = 255
VARIABLES_PER_LINE = 8
HEX_BYTES_PER_LINE = 16
VARIABLE_PREFIXES = {
    Operand.GLOBAL: "global",
    Operand.LOCAL: "local",
    Operand.TEMP: "temp",
    Operand.PARAM: "param",
}


@dataclass(frozen=True)
class Diagnostic:
    kind: str
    where: str
    detail: str


@dataclass
class Coverage:
    resources: Counter = field(default_factory=Counter)
    items: Counter = field(default_factory=Counter)
    opcodes: Counter = field(default_factory=Counter)
    diagnostics: list[Diagnostic] = field(default_factory=list)
    unnamed_selectors: set[tuple[str, int]] = field(default_factory=set)

    def report(self, kind: str, where: str, detail: str) -> None:
        self.diagnostics.append(Diagnostic(kind, where, detail))

    def merge(self, other: "Coverage") -> None:
        self.resources.update(other.resources)
        self.items.update(other.items)
        self.opcodes.update(other.opcodes)
        self.diagnostics.extend(other.diagnostics)
        self.unnamed_selectors |= other.unnamed_selectors


def describe_script_number(number: int) -> str:
    prefix, resource_number = resource_for_script_number(number)
    return f"{prefix}.{resource_number:03d}"


def quoted(text: str) -> str:
    shown = text if len(text) <= MAX_QUOTED_STRING else text[:MAX_QUOTED_STRING] + "..."
    return '"' + shown.encode("unicode_escape").decode("ascii").replace('"', '\\"') + '"'


@dataclass(frozen=True)
class MethodOwner:
    record: ObjectRecord
    selector: int


class ModuleListing:
    """Renders one parsed module; problems found while rendering go to the coverage record."""

    def __init__(self, land: Land, module: Module, where: str, coverage: Coverage):
        self.land, self.module, self.where, self.coverage = land, module, where, coverage
        self.vocabulary = land.vocabulary
        self.local_classes = classes_in(module)
        self.objects = self._parse_objects()
        self.owners = self._method_owners()
        self.exports = self._export_numbers()
        self.property_name_cache: dict[int, list[str]] = {}

    def _parse_objects(self) -> dict[int, ObjectRecord]:
        objects = {}
        for item in self.module.items:
            if item.tag in (ItemTag.OBJECT, ItemTag.CLASS):
                try:
                    objects[item.index] = parse_object(item)
                except FormatError as error:
                    self.coverage.report("bad-object", self.where, str(error))
        return objects

    def _method_owners(self) -> dict[int, MethodOwner]:
        owners = {}
        for record in self.objects.values():
            for selector, code_item in zip(record.method_selectors, record.method_code_items):
                owners[code_item] = MethodOwner(record, selector)
        return owners

    def _export_numbers(self) -> dict[int, int]:
        exports = {}
        for table in self.module.items_of(ItemTag.DISPATCH_TABLE):
            for entry in parse_dispatch_table(table):
                if entry.is_reference:
                    exports.setdefault(entry.value, entry.number)
        return exports

    def selector_name(self, selector: int) -> str:
        if selector >= len(self.vocabulary.selector_names):
            self.coverage.unnamed_selectors.add((str(self.land.resources.path), selector))
        return self.vocabulary.selector_name(selector)

    def lines(self) -> list[str]:
        out = []
        for item in self.module.items:
            self.coverage.items[item.tag_name] += 1
            out.extend(self.item_lines(item))
        return out

    def item_lines(self, item: Item) -> list[str]:
        header = f"item {item.index} {item.tag_name} @0x{item.file_offset:04x}"
        if item.kind is None:
            self.coverage.report("unknown-tag", self.where, header)
        renderer = {
            ItemTag.OBJECT: self.object_lines,
            ItemTag.CLASS: self.object_lines,
            ItemTag.CODE: self.code_lines,
            ItemTag.STRING: lambda i: [f"  {quoted(parse_string(i))}"],
            ItemTag.VARIABLES: self.variable_lines,
            ItemTag.DISPATCH_TABLE: self.dispatch_lines,
            ItemTag.PROPERTY_DICTIONARY: self.dictionary_lines,
        }.get(item.kind, hex_lines)
        return [f"{header}  {self.item_label(item.index)}"] + renderer(item) + [""]

    def item_label(self, index: int) -> str:
        if index >= len(self.module.items):
            return f"<item {index} out of range>"
        item = self.module.items[index]
        if index in self.objects:
            return object_name(self.module, self.objects[index])
        if item.tag == ItemTag.STRING:
            return quoted(parse_string(item))
        if item.tag == ItemTag.CODE:
            return self.code_label(index)
        return f"{item.tag_name.lower()}_{index}"

    def code_label(self, index: int) -> str:
        owner = self.owners.get(index)
        if owner:
            name = object_name(self.module, owner.record)
            return f"{name}::{self.selector_name(owner.selector)}"
        if index in self.exports:
            return f"export_{self.exports[index]}"
        return f"proc_{index}"

    def object_lines(self, item: Item) -> list[str]:
        record = self.objects.get(item.index)
        if record is None:
            return hex_lines(item)
        names = self.property_names(record)
        out = [f"  {'class' if record.is_class else 'instance'} of {self.super_text(record)}"]
        for index, value in enumerate(record.properties):
            out.append(f"    {names[index]:<20} {self.property_value(record, index, value)}")
        for selector, code_item in zip(record.method_selectors, record.method_code_items):
            method = self.selector_name(selector)
            out.append(f"    method {method:<13} -> item {code_item}")
        return out

    def super_text(self, record: ObjectRecord) -> str:
        if record.is_class and record.super_class == NO_SUPER_CLASS:
            return "(root)"
        return self.land.class_name(record.super_class)

    def property_names(self, record: ObjectRecord) -> list[str]:
        if record.item.index not in self.property_name_cache:
            self.property_name_cache[record.item.index] = self.lookup_property_names(record)
        return self.property_name_cache[record.item.index]

    def lookup_property_names(self, record: ObjectRecord) -> list[str]:
        selectors = self.property_selectors(record)
        if selectors is None or len(selectors) != len(record.properties):
            return [f"prop_{index}" for index in range(len(record.properties))]
        return [self.selector_name(selector) for selector in selectors]

    def property_selectors(self, record: ObjectRecord) -> tuple[int, ...] | None:
        if record.is_class:
            dictionary = self.module.items[record.item.index + 1]
            return parse_property_dictionary(dictionary, len(record.properties))
        species = record.super_class
        info = self.local_classes.get(species) or self.land.classes.get(species)
        if info is None:
            self.coverage.report("unresolved-class", self.where, f"class {record.super_class}")
            return None
        return info.property_selectors

    def property_value(self, record: ObjectRecord, index: int, value: int) -> str:
        if record.property_is_reference(index):
            return f"-> {self.item_label(value)}"
        if index == ObjectSlot.SUPER and value != NO_SUPER_CLASS:
            return f"0x{value:04x} ({self.land.class_name(value)})"
        return f"0x{value:04x}"

    def variable_lines(self, item: Item) -> list[str]:
        slots = parse_variables(item)
        cells = [f"[{slot.number}]={self.cell_text(slot)}"
                 for slot in slots if slot.value or slot.is_reference]
        rows = range(0, len(cells), VARIABLES_PER_LINE)
        header = f"  variables 1..{len(slots)}, zero unless listed"
        return [header] + ["    " + "  ".join(cells[row : row + VARIABLES_PER_LINE])
                           for row in rows]

    def dispatch_lines(self, item: Item) -> list[str]:
        return [f"  export {entry.number}: {self.cell_text(entry)}"
                for entry in parse_dispatch_table(item)]

    def dictionary_lines(self, item: Item) -> list[str]:
        owner = self.objects.get(item.index - 1)
        count = len(owner.properties) if owner else len(item.payload) // 2
        selectors = parse_property_dictionary(item, count)
        return ["  " + " ".join(self.selector_name(s) for s in selectors)]

    def cell_text(self, cell: WordCell) -> str:
        return f"-> {self.item_label(cell.value)}" if cell.is_reference else f"0x{cell.value:04x}"

    def code_lines(self, item: Item) -> list[str]:
        instructions, error = decode_code(item.payload)
        self.check_code(item, instructions)
        owner = self.owners.get(item.index)
        out = [self.instruction_line(item, owner, ins) for ins in instructions]
        if error:
            self.coverage.report("decode-error", f"{self.where} item {item.index}", str(error))
            out.extend(hex_lines(item, error.offset))
        return out

    def check_code(self, item: Item, instructions: list[Instruction]) -> None:
        where = f"{self.where} item {item.index}"
        starts = {ins.offset for ins in instructions}
        operand_offsets = {offset for ins in instructions for offset in ins.operand_offsets}
        for ins in instructions:
            self.coverage.opcodes[ins.opcode] += 1
            target = ins.branch_target()
            if target is not None and target not in starts:
                self.coverage.report("branch-outside", where, f"0x{ins.offset:04x} -> {target:#x}")
        for offset in item.fixups:
            if offset not in operand_offsets:
                self.coverage.report("fixup-not-operand", where, f"payload offset 0x{offset:x}")
        if instructions and instructions[-1].opcode not in TERMINATING_OPCODES:
            self.coverage.report("falls-off-end", where, f"last opcode {instructions[-1].row.name}")

    def instruction_line(self, item: Item, owner: MethodOwner | None, ins: Instruction) -> str:
        raw = item.payload[ins.offset : ins.next_offset].hex(" ")
        operands, comment = self.operand_text(item, owner, ins)
        text = f"  {ins.offset:04x}: {raw:<15} {self.mnemonic(ins):<9} {operands}"
        return f"{text.rstrip():<56} ; {comment}" if comment else text.rstrip()

    def mnemonic(self, ins: Instruction) -> str:
        """The game's own name from vocab.998 where it has one, else the table name."""
        names = self.vocabulary.opcode_names
        return names[ins.opcode] if ins.opcode < len(names) and names[ins.opcode] else ins.row.name

    def operand_text(self, item: Item, owner: MethodOwner | None, ins: Instruction):
        """Return (operand text, comment) for one instruction."""
        if any(item.is_reference(offset) for offset in ins.operand_offsets):
            return self.reference_operands(item, ins)
        special = OPERAND_FORMATTERS.get(ins.row.name)
        if special:
            return special(self, ins)
        texts = [self.plain_operand(owner, kind, value)
                 for kind, value in zip(ins.row.operands, ins.operands)]
        return ", ".join(texts), self.operand_comment(ins)

    def reference_operands(self, item: Item, ins: Instruction):
        texts = []
        for offset, value in zip(ins.operand_offsets, ins.operands):
            texts.append(self.item_label(value) if item.is_reference(offset) else str(value))
        return ", ".join(texts), ""

    def plain_operand(self, owner: MethodOwner | None, kind: Operand, value: int) -> str:
        if kind is Operand.RELATIVE:
            return f"{value:+d}"
        if kind is Operand.PROPERTY:
            return self.property_operand(owner, value)
        if kind in VARIABLE_PREFIXES:
            return f"{VARIABLE_PREFIXES[kind]}{value}"
        return str(value) if -SMALL_NUMBER <= value <= SMALL_NUMBER else f"0x{value & 0xFFFF:x}"

    def property_operand(self, owner: MethodOwner | None, offset: int) -> str:
        if owner is None or offset % 2:
            return f"prop[0x{offset:x}]"
        names = self.property_names(owner.record)
        index = offset // 2
        return f"{names[index]}" if index < len(names) else f"prop[0x{offset:x}]"

    def operand_comment(self, ins: Instruction) -> str:
        target = ins.branch_target()
        if target is not None:
            return f"-> {target:04x}"
        if ins.row.name == "pushi" and 0 <= ins.operands[0] < len(self.vocabulary.selector_names):
            return self.selector_name(ins.operands[0])
        return ""


def format_callk(listing: ModuleListing, ins: Instruction):
    number, frame = ins.operands
    return f"{listing.land.kernel_name(number)}, {frame}", f"kernel 0x{number:02x}"


def format_class(listing: ModuleListing, ins: Instruction):
    texts = [listing.land.class_name(ins.operands[0])] + [str(v) for v in ins.operands[1:]]
    return ", ".join(texts), f"class 0x{ins.operands[0]:x}"


def format_callb(listing: ModuleListing, ins: Instruction):
    export, frame = ins.operands
    return f"export {export}, {frame}", "script.000"


def format_calle(listing: ModuleListing, ins: Instruction):
    script, export, frame = ins.operands
    return f"{script:#x} export {export}, {frame}", describe_script_number(script)


def format_file(listing: ModuleListing, ins: Instruction):
    return f"0x{ins.operands[0]:04x}", describe_script_number(ins.operands[0])


OPERAND_FORMATTERS = {
    "callk": format_callk,
    "class": format_class,
    "super": format_class,
    "callb": format_callb,
    "calle": format_calle,
    "_file_": format_file,
}


def hex_lines(item: Item, start: int = 0) -> list[str]:
    rows = range(start, len(item.payload), HEX_BYTES_PER_LINE)
    return [f"  {row:04x}: {item.payload[row : row + HEX_BYTES_PER_LINE].hex(' ')}" for row in rows]


def list_resource(land: Land, prefix: str, number: int, coverage: Coverage) -> list[str]:
    where = f"{land.resources.path.name}/{prefix}.{number:03d}"
    data = land.resources.read(prefix, number)
    if data is None:
        coverage.report("missing", where, "no such resource file")
        return [f"; {where}: no such resource file"]
    coverage.resources[f"{prefix} total"] += 1
    try:
        module = parse_module(data)
    except FormatError as error:
        coverage.report("not-a-container", where, str(error))
        return [f"; {where}: not an LSCI item container ({error})"]
    coverage.resources[f"{prefix} parsed"] += 1
    if module.trailing_bytes:
        coverage.report("trailing-bytes", where, f"{module.trailing_bytes} bytes")
    header = f"; {where}: {len(module.items)} items; {KERNEL_NUMBERING_NOTE}"
    return [header, ""] + ModuleListing(land, module, where, coverage).lines()


def load_kernel_names(path: Path) -> tuple[str, ...]:
    return tuple(path.read_text().split()) if path.exists() else ()


def find_lands(root: Path) -> list[Path]:
    if is_resource_dir(root):
        return [root]
    return sorted(path.parent for path in root.rglob("vocab.997"))


def selected_resources(land: Land, args: argparse.Namespace) -> list[tuple[str, int]]:
    if args.script is not None:
        return [(SCRIPT_PREFIX, args.script)]
    if args.module is not None:
        return [(MODULE_PREFIX, args.module)]
    numbers = land.resources.numbers
    return [(prefix, number) for prefix in CODE_RESOURCE_PREFIXES for number in numbers(prefix)]


def print_coverage(title: str, coverage: Coverage, verbose: bool) -> None:
    kinds = Counter(diagnostic.kind for diagnostic in coverage.diagnostics)
    print(f"== {title}")
    print("  resources: " + ", ".join(f"{k} {v}" for k, v in sorted(coverage.resources.items())))
    print("  items: " + ", ".join(f"{k} {v}" for k, v in coverage.items.most_common()))
    instructions = sum(coverage.opcodes.values())
    print(f"  instructions: {instructions}, distinct opcodes {len(coverage.opcodes)}")
    unused = [OPCODE_TABLE[op].name for op in range(len(OPCODE_TABLE)) if not coverage.opcodes[op]]
    print(f"  opcodes never used: {' '.join(unused) or 'none'}")
    print(f"  selectors used but missing from vocab.997: {len(coverage.unnamed_selectors)}")
    print("  diagnostics: " + (", ".join(f"{k} {v}" for k, v in kinds.most_common()) or "none"))
    for diagnostic in coverage.diagnostics if verbose else []:
        print(f"    {diagnostic.kind}: {diagnostic.where}: {diagnostic.detail}")


def run_land(path: Path, args: argparse.Namespace, kernel_names: tuple[str, ...]) -> Coverage:
    land = Land(ResourceDir(path), kernel_names)
    coverage = Coverage()
    for prefix, number in selected_resources(land, args):
        lines = list_resource(land, prefix, number, coverage)
        if not args.stats:
            print("\n".join(lines))
    return coverage


def parse_arguments(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("path", type=Path, help="land directory (with vocab.997) or a tree of them")
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--script", type=int, help="only this script number")
    selection.add_argument("--module", type=int, help="only this type-31 module number")
    parser.add_argument("--stats", action="store_true", help="print coverage statistics only")
    parser.add_argument("--verbose", action="store_true", help="list every diagnostic")
    parser.add_argument("--kernel-names", type=Path, default=DEFAULT_KERNEL_NAMES)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_arguments(argv)
    lands = find_lands(args.path)
    if not lands:
        print(f"no resource directory with vocab.997 under {args.path}", file=sys.stderr)
        return 1
    kernel_names = load_kernel_names(args.kernel_names)
    total = Coverage()
    for path in lands:
        coverage = run_land(path, args, kernel_names)
        if args.stats:
            print_coverage(str(path), coverage, args.verbose)
        total.merge(coverage)
    if args.stats and len(lands) > 1:
        print_coverage("all lands", total, verbose=False)
    return 0


if __name__ == "__main__":
    sys.exit(main())

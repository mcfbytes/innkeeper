#!/usr/bin/env python3
"""Extract the TSN application message catalog from LSCI scripts: sends, receive classes, handlers.
One deterministic text report per set, for diffing versions; the catalog is docs/protocol/messages.md."""
import argparse
import sys
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

from lsci_bytecode import decode_code
from lsci_disasm import (
    CODE_RESOURCE_PREFIXES,
    DEFAULT_KERNEL_NAMES,
    Coverage,
    ModuleListing,
    find_lands,
    load_kernel_names,
)
from lsci_eval import Comparison, Const, Event, KernelCall, Message, Name, Send, SendResult
from lsci_eval import Store, Text, Value, evaluate
from lsci_format import FormatError, Item, ItemTag, ResourceDir, is_resource_dir, parse_module
from lsci_format import parse_string
from lsci_land import Land

TSN_KERNEL = "TSN"
SEND_FORMATTED = 4
SUBOP_NAMES = {
    0: "GetStatus", 1: "GetSharedData", 2: "SetSharedData", 3: "Connect", 4: "Send",
    7: "Disconnect", 8: "Poll", 9: "SetNextProgram", 11: "GetPreviousProgram",
    12: "IsTransmitIdle", 13: "SwitchHost", 14: "Flush", 16: "GetLineRate",
}
PLUMBING_SUBOPS = frozenset({0, 7, 8, 12, 14, 16})
FIELD_READERS = {"at": "b", "wordAt": "w", "stringAt": "s", "arrayAt": "a"}
BODY_READERS = frozenset({"move"})
MESSAGE_SELECTORS = ("command", "msgType", "whichCmd", "whichSub")
CLASS_SWITCH = ("dup", "ldi", "eq?", "bnt", "class")
MIN_DISPATCH_CASES = 5
BASE_CLASS_KEY = -1
UNNAMED_CLASS_PREFIX = "class_"
SELF = Name("self")
ROUTING_PROPERTY = "to"


class ItemContext:
    """Names for the evaluator, taken from the disassembler's listing of one module."""

    def __init__(self, listing: ModuleListing, item: Item):
        self.listing = listing
        self.owner = listing.owners.get(item.index)

    def selector_name(self, selector: int) -> str:
        return self.listing.selector_name(selector)

    def class_name(self, species: int) -> str:
        info = self.listing.local_classes.get(species)
        return info.name if info else self.listing.land.class_name(species)

    def kernel_name(self, number: int) -> str:
        return self.listing.land.kernel_name(number)

    def property_name(self, offset: int) -> str:
        return self.listing.property_operand(self.owner, offset)

    def item_value(self, index: int) -> Value:
        items = self.listing.module.items
        if index < len(items) and items[index].tag == ItemTag.STRING:
            return Text(parse_string(items[index]))
        return Name(self.listing.item_label(index))


@dataclass(frozen=True)
class CodeUnit:
    """One code item with its evaluated events and where it lives."""

    where: str
    label: str
    owner_class: str | None
    super_class: str | None
    item: Item
    context: ItemContext
    events: tuple[Event, ...]

    @property
    def method(self) -> str:
        return self.label.split("::", 1)[-1]

    @property
    def site(self) -> str:
        return f"{self.where} {self.label}"


def module_units(land: Land, prefix: str, number: int) -> list[CodeUnit]:
    data = land.resources.read(prefix, number)
    try:
        module = parse_module(data) if data else None
    except FormatError:
        module = None
    if module is None:
        return []
    where = f"{land.resources.path.name}/{prefix}.{number:03d}"
    listing = ModuleListing(land, module, where, Coverage())
    units = []
    for item in module.items_of(ItemTag.CODE):
        owner = listing.owners.get(item.index)
        owner_name = listing.item_label(owner.record.item.index) if owner else None
        super_name = listing.super_text(owner.record) if owner else None
        context = ItemContext(listing, item)
        events = tuple(evaluate(item, context))
        label = listing.code_label(item.index)
        units.append(CodeUnit(where, label, owner_name, super_name, item, context, events))
    return units


def land_units(land: Land) -> list[CodeUnit]:
    return [unit for prefix in CODE_RESOURCE_PREFIXES
            for number in land.resources.numbers(prefix)
            for unit in module_units(land, prefix, number)]


@dataclass(frozen=True)
class TsnCall:
    unit: CodeUnit
    subop: int | None
    args: tuple[Value, ...]
    passes_rest: bool

    @property
    def opcode(self) -> int | None:
        if self.subop != SEND_FORMATTED or len(self.args) < 2:
            return None
        return self.args[1].value if isinstance(self.args[1], Const) else None


def tsn_calls(units: list[CodeUnit]) -> list[TsnCall]:
    calls = []
    for unit in units:
        for event in unit.events:
            if isinstance(event, KernelCall) and event.name == TSN_KERNEL:
                first = event.args[0] if event.args else None
                subop = first.value if isinstance(first, Const) else None
                calls.append(TsnCall(unit, subop, event.args[1:], event.passes_rest))
    return calls


def format_fields(format_text: str, values: tuple[Value, ...]) -> list[str]:
    """Pair formatted-send codes with values; the codes are in docs/protocol/ktsn.md (sub-op 4)."""
    fields, position, offset, previous = [], 0, 0, "w"
    for code in format_text:
        if code == "+":
            repeated = "b" if previous == "b" else "w"
            return fields + [f"+{repeated}={value}" for value in values[position:]]
        taken = 2 if code == "a" else 1
        chunk = ",".join(str(value) for value in values[position : position + taken])
        place = "?" if offset is None else str(offset)
        fields.append(f"{place}:{code}={chunk or '&rest'}")
        position += taken
        width = {"b": 1, "w": 2}.get(code)
        offset = offset + width if offset is not None and width else None
        previous = code
    return fields


def send_layout(call: TsnCall) -> str:
    format_value = call.args[0] if call.args else None
    rest = " +&rest" if call.passes_rest else ""
    if isinstance(format_value, Text):
        fields = " ".join(format_fields(format_value.text, call.args[1:]))
        return f"{format_value} {fields}{rest}"
    return "(format not constant) " + " ".join(str(arg) for arg in call.args) + rest


def class_switch(unit: CodeUnit) -> list[tuple[int, str]]:
    """Cases of a 'dup; ldi N; eq?; bnt; class C' chain, as (N, class name)."""
    instructions, _error = decode_code(unit.item.payload)
    names = [ins.row.name for ins in instructions]
    cases = []
    for start in range(len(instructions) - len(CLASS_SWITCH) + 1):
        if tuple(names[start : start + len(CLASS_SWITCH)]) == CLASS_SWITCH:
            value = instructions[start + 1].operands[0]
            species = instructions[start + 4].operands[0]
            cases.append((value & 0xFFFF, unit.context.class_name(species)))
    return cases


def body_reads(unit: CodeUnit) -> list[str]:
    """Reads a message class makes from its own body, and where it sends itself."""
    reads = []
    for event in unit.events:
        if isinstance(event, Store) and is_self_read(event.value):
            reads.append(read_text(unit, event.value.message, event.target))
        elif isinstance(event, Store) and event.target == ROUTING_PROPERTY:
            reads.append(f"{unit.method}: routed to {event.value}")
        elif isinstance(event, Send):
            reads.extend(send_reads(unit, event))
    return reads


def is_self_read(value: Value) -> bool:
    return (isinstance(value, SendResult) and value.receiver == SELF
            and value.message.selector in FIELD_READERS)


def read_text(unit: CodeUnit, message: Message, target: str) -> str:
    offset = " ".join(str(arg) for arg in message.args)
    return f"{unit.method}: {FIELD_READERS[message.selector]}@{offset} -> {target}"


def send_reads(unit: CodeUnit, event: Send) -> list[str]:
    reads = []
    for message in event.messages:
        args = " ".join(str(arg) for arg in message.args)
        if event.receiver == SELF and len(message.args) == 1 and is_self_read(message.args[0]):
            reads.append(read_text(unit, message.args[0].message, message.selector))
        elif event.receiver == SELF and message.selector == ROUTING_PROPERTY:
            reads.append(f"{unit.method}: routed to {args}")
        elif event.receiver == SELF and message.selector in BODY_READERS:
            reads.append(f"{unit.method}: body rebased by move: {args}")
        elif message.selector == "copyToFrom" and SELF in message.args:
            reads.append(f"{unit.method}: array copy {args}")
    return reads


@dataclass(frozen=True)
class MessageClass:
    command: int
    name: str
    super_name: str
    calls_super_init: bool
    reads: tuple[str, ...]


def message_classes(units: list[CodeUnit], cases: list[tuple[int, str]]) -> list[MessageClass]:
    by_class = defaultdict(list)
    for unit in units:
        if unit.owner_class:
            by_class[unit.owner_class].append(unit)
    found = []
    for command, name in sorted(cases):
        methods = by_class.get(name, [])
        reads = tuple(read for unit in methods for read in body_reads(unit))
        init = next((unit for unit in methods if unit.method == "init"), None)
        calls_super = init is None or any(sends_super_init(event) for event in init.events)
        super_name = next((unit.super_class for unit in methods if unit.super_class), "?")
        found.append(MessageClass(command, name, super_name, calls_super, reads))
    return found


def sends_super_init(event: Event) -> bool:
    return (isinstance(event, Send) and event.receiver == Name("super")
            and any(message.selector == "init" for message in event.messages))


def message_tests(units: list[CodeUnit]) -> dict[tuple[str, int], set[str]]:
    """Places that compare a message's command, msgType, whichCmd or whichSub with a constant."""
    tests = defaultdict(set)
    for unit in units:
        for event in unit.events:
            if isinstance(event, Comparison) and isinstance(event.right, Const):
                selector = tested_selector(event.left)
                if selector:
                    tests[(selector, event.right.value)].add(unit.site)
    return tests


def tested_selector(value: Value) -> str | None:
    if isinstance(value, SendResult) and not value.message.args:
        return value.message.selector if value.message.selector in MESSAGE_SELECTORS else None
    return None


@dataclass
class SetCatalog:
    """Everything extracted from the lands of one set, keyed for stable printing."""

    dispatch: dict[str, list[tuple[int, str]]]
    classes: list[tuple[MessageClass, str]]
    sends: dict[int | None, dict[str, list[str]]]
    connection: dict[str, list[str]]
    tests: dict[tuple[str, int], set[str]]


def catalog_set(land_dirs: list[Path], kernel_names: tuple[str, ...]) -> SetCatalog:
    catalog = SetCatalog({}, [], defaultdict(lambda: defaultdict(list)), defaultdict(list),
                         defaultdict(set))
    for path in land_dirs:
        land = Land(ResourceDir(path), kernel_names)
        units = land_units(land)
        add_receive_side(catalog, units, path.name)
        add_send_side(catalog, tsn_calls(units))
        for key, sites in message_tests(units).items():
            catalog.tests[key] |= sites
    return catalog


def add_receive_side(catalog: SetCatalog, units: list[CodeUnit], land_name: str) -> None:
    for unit in units:
        cases = class_switch(unit)
        if len(cases) < MIN_DISPATCH_CASES:
            continue
        catalog.dispatch[unit.site] = sorted(cases)
        base = [(BASE_CLASS_KEY, unit.owner_class)] if unit.owner_class else []
        catalog.classes += [(found, land_name) for found in message_classes(units, base + cases)]


def recovered_names(catalogs: list[SetCatalog]) -> dict[int, str]:
    """Class names that survive in some build (Dec-93 SierraLand keeps them), by command."""
    names = {}
    for catalog in catalogs:
        for found, _land in catalog.classes:
            if not found.name.startswith(UNNAMED_CLASS_PREFIX):
                names.setdefault(found.command, found.name)
    return names


def class_text(found: MessageClass, names: dict[int, str]) -> str:
    inherits = "calls super init" if found.calls_super_init else "own layout"
    key = "base" if found.command == BASE_CLASS_KEY else f"{found.command}"
    lines = [f"command {key}: {named(found.name, names.get(found.command))} < "
             f"{found.super_name}, {inherits}"]
    return "\n".join(lines + [f"    {read}" for read in found.reads])


def named(name: str, recovered: str | None) -> str:
    return f"{name} ({recovered})" if recovered and recovered != name else name


def add_send_side(catalog: SetCatalog, calls: list[TsnCall]) -> None:
    for call in calls:
        if call.subop == SEND_FORMATTED:
            catalog.sends[call.opcode][send_layout(call)].append(call.unit.site)
        elif call.subop not in PLUMBING_SUBOPS:
            name = SUBOP_NAMES.get(call.subop, f"sub-op {call.subop}")
            args = ", ".join(str(arg) for arg in call.args)
            catalog.connection[f"{name}({args})"].append(call.unit.site)


def print_catalog(title: str, catalog: SetCatalog, names: dict[int, str]) -> None:
    print(f"# TSN message catalog: {title}")
    print("\n## Receive dispatch (command byte -> message class)")
    for site, cases in sorted(catalog.dispatch.items()):
        print(f"{site}: " + " ".join(f"{command}={named(name, names.get(command))}"
                                     for command, name in cases))
    print("\n## Receive classes (fields read from the message body)")
    grouped = defaultdict(list)
    for found, land_name in catalog.classes:
        grouped[(found.command, class_text(found, names))].append(land_name)
    for (_command, text), lands in sorted(grouped.items()):
        print(f"{text}\n    lands: {' '.join(lands)}")
    print("\n## Sends by first value (TSN sub-op 4)")
    for opcode in sorted(catalog.sends, key=lambda value: (value is None, value or 0)):
        label = "not constant" if opcode is None else f"{opcode} (0x{opcode:02x})"
        print(f"opcode {label}")
        for layout, sites in sorted(catalog.sends[opcode].items()):
            print(f"  {layout}\n" + "".join(f"      {site}\n" for site in sorted(set(sites))), end="")
    print("\n## Connection sub-ops")
    for text, sites in sorted(catalog.connection.items()):
        print(f"{text}\n" + "".join(f"      {site}\n" for site in sorted(set(sites))), end="")
    print("\n## Handlers (a received message's selector compared with a constant)")
    for (selector, value), sites in sorted(catalog.tests.items()):
        print(f"{selector} == {value}: " + ", ".join(sorted(sites)))


def set_roots(root: Path) -> list[Path]:
    """A set directory holds land directories; a land directory is accepted on its own."""
    if is_resource_dir(root):
        return [root]
    return sorted({land.parent for land in find_lands(root)})


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("path", type=Path, help="work/res, a set directory or one land directory")
    parser.add_argument("--kernel-names", type=Path, default=DEFAULT_KERNEL_NAMES)
    args = parser.parse_args(argv)
    if not find_lands(args.path):
        print(f"no resource directory with vocab.997 under {args.path}", file=sys.stderr)
        return 1
    kernel_names = load_kernel_names(args.kernel_names)
    roots = set_roots(args.path)
    catalogs = [catalog_set(find_lands(root), kernel_names) for root in roots]
    names = recovered_names(catalogs)
    for root, catalog in zip(roots, catalogs):
        print_catalog(root.name, catalog, names)
        print()
    return 0


if __name__ == "__main__":
    sys.exit(main())

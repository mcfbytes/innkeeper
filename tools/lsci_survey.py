#!/usr/bin/env python3
"""Kernel-call survey over resource sets: tallies per kernel and sub-op, plus the start-up closure.
Method and caveats: docs/lsci/kernel-usage.md."""
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterator

from lsci_bytecode import decode_code
from lsci_callflow import KernelCall, Source, UseKind, find_kernel_calls, item_sends
from lsci_disasm import Coverage, ModuleListing
from lsci_format import (
    MODULE_PREFIX,
    NO_SUPER_CLASS,
    SCRIPT_PREFIX,
    ItemTag,
    ResourceDir,
    parse_dispatch_table,
    parse_object,
    resource_for_script_number,
)
from lsci_kernels import SUBOP_TABLES
from lsci_land import Land

STARTUP_RESOURCE = f"{SCRIPT_PREFIX}.000"
GAME_OBJECT_EXPORT = 0
START_SELECTOR = 0x1A
SCRIPT_REFERENCE_OPCODES = frozenset({"class", "super"})
MODULE_KERNEL = "ModuleID"
SELECTOR_ARGUMENTS = {"ObjectRespondsTo": (1,), "ObjPropOffset": (1,), "InvokeMethod": (1,)}


@dataclass(frozen=True)
class LocatedCall:
    resource: str
    caller: str
    call: KernelCall


def land_kernel_calls(land: Land, kernel_names: tuple[str, ...]) -> Iterator[LocatedCall]:
    """Every callk in every script and type-31 module of one land, with its caller's name."""
    for prefix in (SCRIPT_PREFIX, MODULE_PREFIX):
        for number in land.resources.numbers(prefix):
            module = land.module(prefix, number)
            if module is None:
                continue
            resource = f"{prefix}.{number:03d}"
            listing = ModuleListing(land, module, resource, Coverage())
            for call in find_kernel_calls(module, kernel_names, listing.item_label):
                yield LocatedCall(resource, listing.code_label(call.item), call)


@dataclass
class Tally:
    """Everything counted about one kernel, or one sub-op of it, inside one resource set."""

    calls: int = 0
    lands: Counter = field(default_factory=Counter)
    argc: Counter = field(default_factory=Counter)
    callers: Counter = field(default_factory=Counter)
    uses: Counter = field(default_factory=Counter)
    stored_in: Counter = field(default_factory=Counter)
    sources: defaultdict = field(default_factory=lambda: defaultdict(Counter))
    variadic: int = 0
    unverified: int = 0

    def add(self, land: str, located: LocatedCall, kernel: str) -> None:
        call = located.call
        self.calls += 1
        self.lands[land] += 1
        rest = any(argument.source is Source.REST for argument in call.arguments)
        self.variadic += rest
        self.unverified += not call.block_verified and not rest
        self.argc[f"{call.argc}+" if rest else str(call.argc)] += 1
        self.callers[(located.caller.split("::")[0], located.resource)] += 1
        for position, argument in enumerate(call.arguments):
            is_selector = position in SELECTOR_ARGUMENTS.get(kernel, ())
            immediate = argument.source is Source.IMMEDIATE
            source = Source.SELECTOR if is_selector and immediate else argument.source
            self.sources[position][source] += 1
        self._add_uses(call)

    def _add_uses(self, call) -> None:
        kinds = {use.kind for use in call.uses}
        self.uses.update(kinds if kinds else {None})
        for use in call.uses:
            if use.kind is UseKind.STORED:
                self.stored_in[use.detail.split()[0]] += 1


@dataclass
class SetSurvey:
    name: str
    lands: list[str] = field(default_factory=list)
    boot_kernels: dict[str, set[int]] = field(default_factory=dict)
    kernels: dict[int, Tally] = field(default_factory=lambda: defaultdict(Tally))
    subops: dict[tuple[int, int | None], Tally] = field(default_factory=lambda: defaultdict(Tally))
    startup: dict[str, set[int]] = field(default_factory=dict)
    startup_resources: dict[str, set[str]] = field(default_factory=dict)

    def kernel_total(self) -> int:
        return sum(tally.calls for tally in self.kernels.values())


def startup_closure(land: Land, calls: list[LocatedCall]) -> set[str]:
    """Resources reachable from script 0 by calle, class loads, superclasses and ModuleID."""
    module_ids = defaultdict(set)
    for located in calls:
        call = located.call
        if land.kernel_name(call.kernel) == MODULE_KERNEL and call.arguments:
            first = call.arguments[0]
            if first.source is Source.IMMEDIATE:
                module_ids[located.resource].add(first.detail)
    seen, frontier = set(), [STARTUP_RESOURCE]
    while frontier:
        resource = frontier.pop()
        if resource in seen:
            continue
        seen.add(resource)
        frontier.extend(_references(land, resource, module_ids[resource]))
    return seen


def _references(land: Land, resource: str, module_ids: set[int]) -> set[str]:
    prefix, number = resource.split(".")
    module = land.module(prefix, int(number))
    if module is None:
        return set()
    class_scripts = land.vocabulary.class_scripts
    targets = set(module_ids)
    for item in module.items_of(ItemTag.CODE):
        instructions, _error = decode_code(item.payload)
        for ins in instructions:
            if ins.row.name == "calle":
                targets.add(ins.operands[0])
            elif ins.row.name in SCRIPT_REFERENCE_OPCODES and ins.operands[0] < len(class_scripts):
                targets.add(class_scripts[ins.operands[0]])
    for item in module.items_of(ItemTag.OBJECT) + module.items_of(ItemTag.CLASS):
        super_class = parse_object(item).super_class
        if super_class != NO_SUPER_CLASS and super_class < len(class_scripts):
            targets.add(class_scripts[super_class])
    return {_resource_name(land, script) for script in targets} - {None}


def _resource_name(land: Land, script: int) -> str | None:
    prefix, number = resource_for_script_number(script)
    return f"{prefix}.{number:03d}" if land.resources.read(prefix, number) is not None else None


ItemId = tuple[str, int]


@dataclass
class ClassChain:
    """A class and its superclasses, nearest first, as selector -> code item."""

    levels: list[dict[int, "ItemId"]] = field(default_factory=list)

    def lookup(self, selector: int | None) -> "ItemId | None":
        return next((level[selector] for level in self.levels if selector in level), None)


def game_object_reach(land: Land) -> set[ItemId]:
    """Code run from the start-up selector of the game object, following static resolution."""
    game = _chain_from(land, _export_item(land, 0, GAME_OBJECT_EXPORT))
    start = game.lookup(START_SELECTOR)
    frontier = [(start, game)] if start else []
    seen: set[ItemId] = set()
    while frontier:
        item_id, chain = frontier.pop()
        if item_id in seen:
            continue
        seen.add(item_id)
        frontier.extend((callee, None) for callee in _direct_callees(land, *item_id))
        frontier.extend(_send_targets(land, item_id, chain))
    return seen


def _send_targets(land: Land, item_id: ItemId, chain: ClassChain | None):
    """Methods reached by sends to self or a literal class with a constant selector."""
    module = _module_of(land, item_id[0])
    if module is None or item_id[1] >= len(module.items):
        return
    for send in item_sends(module, module.items[item_id[1]]):
        receiver = send.receiver
        if receiver.source is Source.SELF:
            target_chain = chain
        elif (receiver.source is Source.CLASS
              and receiver.detail < len(land.vocabulary.class_scripts)):
            target_chain = _chain_from(land, _class_item(land, receiver.detail))
        else:
            continue
        target = target_chain.lookup(send.selector) if target_chain else None
        if target:
            yield target, target_chain


def _module_of(land: Land, resource: str):
    prefix, number = resource.split(".")
    return land.module(prefix, int(number))


def _export_item(land: Land, script: int, export: int) -> ItemId | None:
    prefix, number = resource_for_script_number(script)
    module = land.module(prefix, number)
    tables = module.items_of(ItemTag.DISPATCH_TABLE) if module else []
    entries = parse_dispatch_table(tables[0]) if tables else ()
    entry = next((e for e in entries if e.number == export and e.is_reference), None)
    return (f"{prefix}.{number:03d}", entry.value) if entry else None


def _chain_from(land: Land, where: ItemId | None) -> ClassChain:
    """Method tables of the object or class item and of its superclasses."""
    chain, seen = ClassChain(), set()
    classes = land.vocabulary.class_scripts
    while where is not None:
        record = parse_object(_module_of(land, where[0]).items[where[1]])
        chain.levels.append({selector: (where[0], code) for selector, code in
                             zip(record.method_selectors, record.method_code_items)})
        number = record.super_class
        if number == NO_SUPER_CLASS or number in seen or number >= len(classes):
            break
        seen.add(number)
        where = _class_item(land, number)
    return chain


def _class_item(land: Land, class_number: int) -> ItemId | None:
    if class_number >= len(land.vocabulary.class_scripts):
        return None
    prefix, number = resource_for_script_number(land.vocabulary.class_scripts[class_number])
    module = land.module(prefix, number)
    for item in module.items_of(ItemTag.CLASS) if module else []:
        if parse_object(item).species == class_number:
            return (f"{prefix}.{number:03d}", item.index)
    return None


def _direct_callees(land: Land, resource: str, item_index: int) -> list[ItemId]:
    module = _module_of(land, resource)
    callees: list[ItemId | None] = []
    if module is None or item_index >= len(module.items):
        return []
    instructions, _error = decode_code(module.items[item_index].payload)
    for ins in instructions:
        if ins.row.name == "call":
            callees.append((resource, ins.operands[0]))
        elif ins.row.name == "callb":
            callees.append(_export_item(land, 0, ins.operands[0]))
        elif ins.row.name == "calle":
            callees.append(_export_item(land, ins.operands[0], ins.operands[1]))
    return [callee for callee in callees if callee is not None]


def survey_set(root: Path, name: str, kernel_names: tuple[str, ...]) -> SetSurvey:
    survey = SetSurvey(name)
    for land_path in sorted(path.parent for path in (root / name).rglob("vocab.997")):
        land = Land(ResourceDir(land_path), kernel_names)
        calls = list(land_kernel_calls(land, kernel_names))
        survey.lands.append(land_path.name)
        closure = startup_closure(land, calls)
        survey.startup_resources[land_path.name] = closure
        survey.startup[land_path.name] = {c.call.kernel for c in calls if c.resource in closure}
        reached = game_object_reach(land)
        survey.boot_kernels[land_path.name] = {
            c.call.kernel for c in calls if (c.resource, c.call.item) in reached}
        for located in calls:
            kernel = kernel_names[located.call.kernel]
            survey.kernels[located.call.kernel].add(land_path.name, located, kernel)
            if kernel in SUBOP_TABLES:
                key = (located.call.kernel, located.call.subop)
                survey.subops[key].add(land_path.name, located, kernel)
    return survey


def survey_sets(root: Path, names: tuple[str, ...], kernel_names: tuple[str, ...]):
    return {name: survey_set(root, name, kernel_names) for name in names}

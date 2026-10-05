#!/usr/bin/env python3
"""A land: one resource directory with its vocabulary, parsed-module cache and class catalog.
Class lookup follows the class table (vocab.996) as described in docs/lsci/script-format.md."""
from dataclasses import dataclass
from functools import cached_property

from lsci_format import (
    FormatError,
    ItemTag,
    Module,
    ObjectRecord,
    ObjectSlot,
    ResourceDir,
    parse_module,
    parse_object,
    parse_property_dictionary,
    parse_string,
    resource_for_script_number,
)


@dataclass(frozen=True)
class ClassInfo:
    record: ObjectRecord
    property_selectors: tuple[int, ...]
    name: str


class Land:
    """One resource directory with its vocabulary, parsed-module cache and class catalog."""

    def __init__(self, resources: ResourceDir, kernel_names: tuple[str, ...]):
        self.resources = resources
        self.kernel_names = kernel_names
        self.vocabulary = resources.vocabulary
        self._modules: dict[tuple[str, int], Module | None] = {}

    def module(self, prefix: str, number: int) -> Module | None:
        key = (prefix, number)
        if key not in self._modules:
            data = self.resources.read(prefix, number)
            self._modules[key] = parse_or_none(data)
        return self._modules[key]

    @cached_property
    def classes(self) -> dict[int, ClassInfo]:
        catalog = {}
        for script_number in sorted(set(self.vocabulary.class_scripts)):
            module = self.module(*resource_for_script_number(script_number))
            if module:
                catalog.update(classes_in(module))
        return catalog

    def class_name(self, species: int) -> str:
        info = self.classes.get(species)
        return info.name if info else f"class_{species:x}"

    def kernel_name(self, number: int) -> str:
        if number < len(self.kernel_names):
            return self.kernel_names[number]
        return f"kernel_{number:x}"


def parse_or_none(data: bytes | None) -> Module | None:
    if data is None:
        return None
    try:
        return parse_module(data)
    except FormatError:
        return None


def classes_in(module: Module) -> dict[int, ClassInfo]:
    found = {}
    for item in module.items_of(ItemTag.CLASS):
        try:
            record = parse_object(item)
            dictionary = module.items[item.index + 1]
            selectors = parse_property_dictionary(dictionary, len(record.properties))
        except (FormatError, IndexError):
            continue
        found[record.species] = ClassInfo(record, selectors, object_name(module, record))
    return found


def object_name(module: Module, record: ObjectRecord) -> str:
    name_value = record.properties[ObjectSlot.NAME]
    if record.property_is_reference(ObjectSlot.NAME) and name_value < len(module.items):
        return parse_string(module.items[name_value])
    if record.is_class:
        return f"class_{record.species:x}"
    return f"object_{record.item.index}"

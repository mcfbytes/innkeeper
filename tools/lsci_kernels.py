#!/usr/bin/env python3
"""Kernel numbering and sub-op tables of LSCITV. Evidence and sources: docs/lsci/kernel-usage.md."""
from dataclasses import dataclass
from pathlib import Path

from lsci_bytecode import Evidence

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_KERNEL_NAMES = REPO_ROOT / "work/exe/LSCITV_inn_cd.kernel.txt"
STATIC_KERNEL_COUNT = 89

# Kernels registered at start-up by the DLLs in LSCI.CFG, in load order (GRAPH256 then NLNULL).
DLL_KERNEL_NAMES = ("Palette", "Said", "Parse", "SetSynonyms")


@dataclass(frozen=True)
class SubOp:
    number: int
    name: str
    signature: str
    evidence: Evidence


def confirmed(number: int, name: str, signature: str) -> SubOp:
    return SubOp(number, name, signature, Evidence.CONFIRMED)


def inferred(number: int, name: str, signature: str) -> SubOp:
    return SubOp(number, name, signature, Evidence.INFERRED)


def load_kernel_names(path: Path = DEFAULT_KERNEL_NAMES) -> tuple[str, ...]:
    """The 89 static names in dispatch order, then the DLL-registered ones (empty if no file)."""
    static = tuple(path.read_text().split()) if path.exists() else ()
    return static + DLL_KERNEL_NAMES if len(static) == STATIC_KERNEL_COUNT else static


SUBOP_TABLES: dict[str, tuple[SubOp, ...]] = {
    "TSN": (
        confirmed(0, "GetStatus", "() -> status"),
        confirmed(1, "GetSharedData", "() -> array or 0"),
        confirmed(2, "SetSharedData", "(array, length)"),
        confirmed(3, "Connect", "(dial string) -> 1 ok"),
        confirmed(4, "Send", "(format, values...) -> queued"),
        confirmed(5, "(reserved)", "no-op"),
        confirmed(6, "(reserved)", "no-op"),
        confirmed(7, "Disconnect", "()"),
        confirmed(8, "Poll", "()"),
        confirmed(9, "SetNextProgram", "(name or 0)"),
        confirmed(10, "(reserved)", "no-op"),
        confirmed(11, "GetPreviousProgram", "() -> string or 0"),
        confirmed(12, "IsTransmitIdle", "() -> 1 idle"),
        confirmed(13, "SwitchHost", "(host string) -> 1 ok"),
        confirmed(14, "Flush", "()"),
        confirmed(15, "(reserved)", "no-op"),
        confirmed(16, "GetLineRate", "() -> rate"),
    ),
    "Array": (
        confirmed(0, "New", "(count, type) -> handle"),
        confirmed(1, "Dispose", "(handle)"),
        confirmed(2, "At", "(handle, index) -> value, 0 out of range"),
        confirmed(3, "AtPut", "(handle, index, values...) -> handle"),
        confirmed(4, "Size", "(handle) -> count"),
        confirmed(5, "Fill", "(handle, start, count, value) -> handle, grows the array"),
        confirmed(6, "Copy", "(dest, destIndex, src, srcIndex, count or -1), grows dest"),
        confirmed(7, "Compare", "(a, aIndex, b, bIndex, count) -> 0 when equal"),
    ),
    "List": (
        confirmed(0, "New", "(capacity = 10) -> handle"),
        confirmed(1, "Dispose", "(list)"),
        confirmed(2, "Count", "(list) -> n"),
        confirmed(3, "At", "(list, index) -> value, Alert when out of range"),
        confirmed(4, "Delete", "(list, value) -> value or 0"),
        confirmed(5, "Contains", "(list, value) -> value or 0"),
        confirmed(6, "AddToFront", "(list, value) -> value"),
        confirmed(7, "AddToEnd", "(list, value) -> value"),
        confirmed(8, "AddAfter", "(list, key, value) -> value"),
        confirmed(9, "AddBefore", "(list, key, value) -> value"),
        confirmed(10, "MoveToFront", "(list, value) -> value or 0"),
        confirmed(11, "MoveToEnd", "(list, value) -> value or 0"),
        confirmed(12, "IndexOf", "(list, value) -> index or -1"),
        confirmed(13, "Release", "(list), count = 0"),
    ),
    "Seq": (
        confirmed(0, "New", "(list, mode = 1) -> iterator"),
        confirmed(1, "Dispose", "(iterator)"),
        confirmed(2, "Begin", "(iterator), position -1"),
        confirmed(3, "End", "(iterator), position = count"),
        confirmed(4, "Next", "(iterator) -> value or 0 at the end"),
        confirmed(5, "Prev", "(iterator) -> value or 0 at the start"),
        inferred(6, "Over", "(iterator, list), rebinds the iterator to another list"),
    ),
    "String": (
        confirmed(0, "Size", "(string) -> length"),
        confirmed(1, "Format", "(format, values...) -> new string"),
        confirmed(2, "FormatInto", "(dest, format, values...)"),
        inferred(3, "Terminate", "(string), grows by one and writes the NUL"),
        confirmed(4, "AsInteger", "(string) -> number"),
        inferred(5, "Compare", "(a, b) -> -1, 0, 1"),
        inferred(6, "Parse", "(string, option codes...)"),
        inferred(7, "ToUpper", "(string), in place"),
        inferred(8, "ToLower", "(string), in place"),
        inferred(9, "Search", "(string, ...) -> result"),
        inferred(10, "Search2", "(string, ...) -> result"),
        confirmed(11, "GetToken", "(dest, source, delimiters) -> token"),
    ),
    "Memory": (
        inferred(0, "Compact", "()"),
        inferred(1, "Collect", "()"),
        inferred(2, "FreeBytesLow", "() -> low word"),
        inferred(3, "FreeBytesHigh", "() -> high word"),
        inferred(4, "LargestBlockLow", "() -> low word"),
        inferred(5, "LargestBlockHigh", "() -> high word"),
        inferred(6, "HandleCount", "() -> word at DGROUP:0242 region"),
        confirmed(7, "Require", "(kilobytes), purges resources until that much is free"),
        confirmed(8, "FreeHandle", "(handle)"),
        confirmed(9, "Dump", "(), debug text"),
        inferred(10, "HeapKilobytes", "() -> configured heap size / 1024"),
    ),
    "Sound": (
        confirmed(0, "MasterVolume", "(volume or none) -> volume"),
        confirmed(1, "Mute", "(flag or none) -> state"),
        confirmed(2, "Restore", "(), no-op in LSCITV"),
        confirmed(3, "GetPolyphony", "() -> voices"),
        confirmed(4, "Update", "(sound object)"),
        confirmed(5, "Init", "(sound object)"),
        confirmed(6, "Dispose", "(sound object)"),
        confirmed(7, "Play", "(sound object, flag)"),
        confirmed(8, "Stop", "(sound object)"),
        confirmed(9, "Pause", "(sound object, flag)"),
        confirmed(10, "Fade", "(sound object, volume, ticks, step, stop)"),
        confirmed(11, "UpdateCues", "(sound object)"),
        confirmed(12, "SendMidi", "(sound object, channel, command, value)"),
        confirmed(13, "Reverb", "(mode or none) -> mode"),
        confirmed(14, "SetHold", "(sound object, hold)"),
    ),
    "FileSystem": (
        confirmed(0, "Open", "(name, mode) -> handle or -1"),
        confirmed(1, "Close", "(handle)"),
        confirmed(2, "WriteString", "(handle, string) -> bytes written"),
        confirmed(3, "ReadString", "(buffer, maxLength, handle) -> buffer or 0"),
        inferred(4, "GetCurrentDir", "(string, ...)"),
        confirmed(5, "PutByte", "(handle, byte)"),
        confirmed(6, "GetByte", "(handle) -> byte or -1"),
        confirmed(7, "(reserved)", "no-op"),
        inferred(8, "IsDirectory", "(path) -> 1 when the path names a directory"),
        confirmed(9, "Seek", "(file object, low, high * 10000, whence)"),
        confirmed(10, "Unlink", "(name)"),
        confirmed(11, "ReadToArray", "(file object, array, count) -> status"),
        confirmed(12, "WriteFromArray", "(file object, array, count) -> status"),
        confirmed(13, "Size", "(file object)"),
        confirmed(14, "FindFirst", "(dest string, pattern, attributes) -> 0 none"),
        confirmed(15, "FindNext", "(dest string) -> 0 none"),
        inferred(16, "SizeLow", "(handle) -> low word"),
        inferred(17, "SizeHigh", "(handle) -> high word"),
    ),
    "SID": (
        confirmed(0, "Init", "(slots), allocates the id table"),
        confirmed(1, "Bind", "(id, object) -> 1 stored, 0 table full"),
        confirmed(2, "Unbind", "(id, ...)"),
        confirmed(3, "Lookup", "(id) -> bound object, 0 unknown; id 0 gives the local object"),
        inferred(4, "List", "(kind) -> array of live entries"),
        inferred(5, "List2", "(kind) -> array of live entries"),
    ),
    "Graph": (
        confirmed(0, "LoadBits", "(...), DLL hook slot 16"),
        confirmed(1, "GetColorCount", "() -> 256"),
        confirmed(2, "SetResPal", "(a, b), DLL hook slot 18"),
        confirmed(3, "DrawLine", "(x1, y1, x2, y2, color, priority, control), DLL Line"),
        confirmed(4, "(reserved)", "no-op"),
        confirmed(5, "DrawBrush", "(6 values), DLL hook slot 20"),
        inferred(6, "SaveBits", "(rect, flags) -> handle"),
        inferred(7, "RestoreBits", "(handle)"),
        inferred(8, "Graph8", "(a, b), unused by scripts"),
        inferred(9, "Graph9", "(a, b), unused by scripts"),
        confirmed(10, "FillRect", "(rect object, a, b, c, d), DLL FillRect"),
        inferred(11, "ShowBits", "(rect object, flags), DLL ShowBits path"),
        inferred(12, "RedrawBox", "(rect object), cel-list redraw"),
        confirmed(13, "InitPri", "(a, b), DLL InitPri"),
        inferred(14, "QueryBits", "(rect object, flags) -> value"),
        inferred(15, "Graph15", "(handle), second restore-style call"),
    ),
    "Long": (
        confirmed(0, "Add", "(long object, operand)"),
        confirmed(1, "Subtract", "(long object, operand)"),
        confirmed(2, "Multiply", "(long object, operand)"),
        confirmed(3, "Divide", "(long object, operand)"),
        inferred(4, "SquareRoot", "(long object)"),
        confirmed(5, "Negate", "(long object)"),
        confirmed(6, "GreaterThan", "(long object, operand) -> 0 or 1"),
        confirmed(7, "LessThan", "(long object, operand) -> 0 or 1"),
        confirmed(8, "Equals", "(long object, operand) -> 0 or 1"),
        confirmed(9, "GreaterOrEqual", "(long object, operand) -> 0 or 1"),
        confirmed(10, "LessOrEqual", "(long object, operand) -> 0 or 1"),
        confirmed(11, "AsString", "(long object, base flag) -> string"),
        confirmed(12, "FromString", "(long object, string)"),
    ),
    "Resource": (
        confirmed(0, "Load", "(type, numbers...)"),
        confirmed(1, "Unload", "(type, numbers...)"),
        inferred(2, "Lock", "(type, number, state)"),
        inferred(3, "Reserve", "(bytes)"),
        inferred(4, "SetRoom", "(room), purges until memory suffices"),
    ),
    "Block": (
        confirmed(0, "New", "(size) -> handle"),
        confirmed(1, "Free", "(handle)"),
        inferred(2, "Lock", "(handle, flag) -> result"),
    ),
    "GetTime": (
        confirmed(1, "Time12", "() -> hour<<12 | minute<<6 | second"),
        confirmed(2, "Time24", "() -> hour<<11 | minute<<5 | second/2"),
        confirmed(3, "Date", "() -> year-1980<<9 | month<<5 | day"),
    ),
    "Palette": (
        inferred(1, "SetFromResource", "(resource, flags)"),
        inferred(5, "Find", "(red, green, blue) -> palette index"),
    ),
}


def subop_name(kernel: str, number: int | None) -> str:
    """The table name of a sub-op, or its number when the kernel has no table row for it."""
    for row in SUBOP_TABLES.get(kernel, ()):
        if row.number == number:
            return row.name
    return "?" if number is None else str(number)

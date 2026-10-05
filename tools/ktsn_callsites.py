#!/usr/bin/env python3
"""List every TSN kernel call in decompressed LSCI scripts and modules, with its sub-op.
Calls come from tools/lsci_callflow.py; the sub-ops are described in docs/protocol/ktsn.md."""
import argparse
import sys
from collections import Counter, defaultdict
from pathlib import Path

from lsci_callflow import KernelCall, Source
from lsci_disasm import find_lands
from lsci_format import ResourceDir
from lsci_kernels import DEFAULT_KERNEL_NAMES, load_kernel_names
from lsci_land import Land
from lsci_survey import LocatedCall, land_kernel_calls

TSN_KERNEL = 0x54
SEND_FORMATTED_SUBOP = 4
FORMAT_ARGUMENT = 1


def message_code(call: KernelCall) -> int | None:
    """First value after the format string of a formatted send, when it is a constant."""
    if call.subop != SEND_FORMATTED_SUBOP or len(call.arguments) <= FORMAT_ARGUMENT + 1:
        return None
    code = call.arguments[FORMAT_ARGUMENT + 1]
    return code.detail if code.source is Source.IMMEDIATE else None


def print_site(land: Path, located: LocatedCall) -> None:
    call = located.call
    subop = "?" if call.subop is None else f"{call.subop:#04x}"
    arguments = ", ".join(argument.describe() for argument in call.arguments[1:])
    print(f"{land.name}/{located.resource}:{call.file_offset:05x} "
          f"argc={call.argc} subop={subop} [{arguments}]")


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
    lands = find_lands(arguments.set_root)
    if not lands:
        print(f"{arguments.set_root}: no land directories", file=sys.stderr)
        return 1
    kernel_names = load_kernel_names(DEFAULT_KERNEL_NAMES)
    histogram, resources, codes = Counter(), defaultdict(set), Counter()
    for path in lands:
        for located in land_kernel_calls(Land(ResourceDir(path), kernel_names), kernel_names):
            call = located.call
            if call.kernel != TSN_KERNEL:
                continue
            histogram[(call.subop, call.argc)] += 1
            resources[(call.subop, call.argc)].add(f"{path.name}/{located.resource}")
            if message_code(call) is not None:
                codes[message_code(call)] += 1
            if not arguments.summary_only:
                print_site(path, located)
    print_summary(histogram, resources)
    print_message_codes(codes)
    return 0


if __name__ == "__main__":
    sys.exit(main())

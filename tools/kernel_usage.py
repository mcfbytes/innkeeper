#!/usr/bin/env python3
"""Report how LSCI scripts use the kernels: call volume, sub-ops, arguments, results, versions.
Prints Markdown; docs/lsci/kernel-usage.md is written from this output."""
import argparse
import sys
from pathlib import Path

from lsci_callflow import UseKind
from lsci_kernels import SUBOP_TABLES, load_kernel_names, subop_name
from lsci_survey import SetSurvey, Tally, survey_sets

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_ROOT = REPO_ROOT / "work/res"
ALL_SETS = ("inn_feb94", "inn_dec93", "tsn21", "tsn_basic")
PRIMARY_SET = "inn_feb94"
LAND_ORDER = ("hub", "SL", "LL")
COMMON_LAND = "hub"
SECTIONS = ("summary", "kernels", "subops", "arguments", "versions", "priority")
TOP_CALLERS = 3
TOP_ARGC = 4
TOP_USES = 3
ARGUMENT_COLUMNS = 3
ARGUMENT_KERNELS = 30
USE_NAMES = {None: "unseen", **{kind: kind.value for kind in UseKind}}
TIERS = ("boot path", "boot path in some lands", "loaded at start-up", "room code only")


def table(headers: list[str], rows: list[list[str]]) -> str:
    lines = ["| " + " | ".join(headers) + " |", "|" + "|".join("---" for _ in headers) + "|"]
    escaped = [[cell.replace("|", "\\|") for cell in row] for row in rows]
    return "\n".join(lines + ["| " + " | ".join(row) + " |" for row in escaped])


def lands_cell(tally: Tally) -> str:
    return " / ".join(f"{tally.lands[land]:,}" for land in LAND_ORDER)


def argc_cell(tally: Tally) -> str:
    ranked = sorted(tally.argc.items(), key=lambda item: (-item[1], item[0]))
    shown = ", ".join(f"{value}x{count:,}" for value, count in ranked[:TOP_ARGC])
    return shown + (f", +{len(ranked) - TOP_ARGC}" if len(ranked) > TOP_ARGC else "")


def callers_cell(tally: Tally, limit: int = TOP_CALLERS) -> str:
    return "; ".join(f"{owner} ({resource}) {count}"
                     for (owner, resource), count in tally.callers.most_common(limit))


def uses_cell(tally: Tally) -> str:
    """Share of call sites with at least one use of each kind; kinds can overlap."""
    ranked = [(kind, count) for kind, count in tally.uses.most_common()
              if kind is not UseKind.RECEIVER][:TOP_USES]
    stores = tally.stored_in.most_common(2)
    where = " in " + "/".join(kind for kind, _count in stores) if stores else ""
    cells = []
    for kind, count in ranked:
        label = USE_NAMES[kind] + (where if kind is UseKind.STORED else "")
        cells.append(f"{label} {100 * count // tally.calls}%")
    return ", ".join(cells)


def kernel_label(names: tuple[str, ...], number: int) -> str:
    return f"{number:02x} {names[number]}" if number < len(names) else f"{number:02x}"


def summary_section(surveys: dict[str, SetSurvey]) -> str:
    rows = []
    for survey in surveys.values():
        tallies = survey.kernels.values()
        rows.append([survey.name, ", ".join(survey.lands), f"{survey.kernel_total():,}",
                     str(len(survey.kernels)), f"{sum(t.variadic for t in tallies):,}",
                     str(sum(t.unverified for t in tallies))])
    return table(["Set", "Lands", "callk sites", "Kernels used", "With &rest", "Argc unverified"],
                 rows)


def kernels_section(survey: SetSurvey, names: tuple[str, ...]) -> str:
    rows = []
    for number, tally in sorted(survey.kernels.items(), key=lambda item: -item[1].calls):
        rows.append([kernel_label(names, number), f"{tally.calls:,}", lands_cell(tally),
                     argc_cell(tally), callers_cell(tally), uses_cell(tally)])
    unused = [kernel_label(names, n) for n in range(len(names)) if n not in survey.kernels]
    headers = ["Kernel", "Calls", "hub / SL / LL", "argc", "Typical callers", "Result use"]
    return table(headers, rows) + "\n\nNever called: " + ", ".join(unused) + "."


def subop_rows(survey: SetSurvey, names: tuple[str, ...], kernel: str) -> list[list[str]]:
    number = names.index(kernel)
    documented = {row.number: row for row in SUBOP_TABLES[kernel]}
    seen = {key[1] for key in survey.subops if key[0] == number and key[1] is not None}
    rows = []
    for subop in sorted(set(documented) | seen):
        tally = survey.subops.get((number, subop), Tally())
        row = documented.get(subop)
        used = tally.calls > 0
        rows.append([f"{subop} {row.name}" if row else str(subop),
                     f"`{row.signature}`" if row else "-",
                     row.evidence.value.upper() if row else "-", f"{tally.calls:,}",
                     argc_cell(tally) if used else "-", callers_cell(tally, 2) if used else "-",
                     uses_cell(tally) if used else ""])
    computed = survey.subops.get((number, None))
    if computed:
        rows.append(["(first argument not constant)", "-", "-", f"{computed.calls:,}",
                     argc_cell(computed), callers_cell(computed, 2), uses_cell(computed)])
    return rows


def subops_section(survey: SetSurvey, names: tuple[str, ...]) -> str:
    headers = ["Sub-op", "Signature", "Evidence", "Calls", "argc", "Typical callers", "Result use"]
    parts = [f"### {kernel}\n\n" + table(headers, subop_rows(survey, names, kernel))
             for kernel in SUBOP_TABLES if kernel in names]
    return "\n\n".join(parts)


def source_cell(tally: Tally, position: int) -> str:
    counter = tally.sources.get(position)
    if not counter:
        return "-"
    total = sum(counter.values())
    return ", ".join(f"{source.value} {100 * count // total}%"
                     for source, count in counter.most_common(3))


def arguments_section(survey: SetSurvey, names: tuple[str, ...]) -> str:
    ranked = sorted(survey.kernels.items(), key=lambda item: -item[1].calls)[:ARGUMENT_KERNELS]
    rows = [[names[number], f"{tally.calls:,}"]
            + [source_cell(tally, position) for position in range(ARGUMENT_COLUMNS)]
            for number, tally in ranked]
    return table(["Kernel", "Calls"] + [f"Argument {p + 1}" for p in range(ARGUMENT_COLUMNS)], rows)


def calls_in(survey: SetSurvey, number: int, land: str | None = None) -> int:
    tally = survey.kernels.get(number)
    if tally is None:
        return 0
    return tally.calls if land is None else tally.lands[land]


def gaps_table(surveys: dict[str, SetSurvey], names: tuple[str, ...]) -> str:
    rows = []
    for number in range(len(names)):
        called = [s for s, survey in surveys.items() if calls_in(survey, number)]
        if called and len(called) < len(surveys):
            rows.append([kernel_label(names, number), ", ".join(called),
                         ", ".join(s for s in surveys if s not in called)])
    return table(["Kernel", "Called in", "Never called in"], rows)


def changes_table(surveys: dict[str, SetSurvey], names: tuple[str, ...]) -> str:
    rows = []
    for number in range(len(names)):
        counts = [calls_in(survey, number, COMMON_LAND) for survey in surveys.values()]
        if len(set(counts)) > 1 and any(counts):
            rows.append([kernel_label(names, number)] + [f"{count:,}" for count in counts])
    return table(["Kernel"] + list(surveys), rows)


def subop_changes(surveys: dict[str, SetSurvey], names: tuple[str, ...]) -> str:
    rows = []
    for kernel in (k for k in SUBOP_TABLES if k in names):
        number = names.index(kernel)
        per_set = {s: {k[1] for k in v.subops if k[0] == number} for s, v in surveys.items()}
        for subop in sorted(set().union(*per_set.values()) - {None}):
            used = [s for s in surveys if subop in per_set[s]]
            if len(used) < len(surveys):
                rows.append([kernel, f"{subop} {subop_name(kernel, subop)}", ", ".join(used),
                             ", ".join(s for s in surveys if s not in used)])
    return table(["Kernel", "Sub-op", "Used in", "Not used in"], rows)


def versions_section(surveys: dict[str, SetSurvey], names: tuple[str, ...]) -> str:
    return "\n\n".join([
        "Kernels called in some sets and never in others:\n\n" + gaps_table(surveys, names),
        f"Kernels whose call count in the {COMMON_LAND} land differs between sets:\n\n"
        + changes_table(surveys, names),
        "Sub-ops used in some sets and not in others:\n\n" + subop_changes(surveys, names),
    ])


def tier_of(number: int, survey: SetSurvey) -> int:
    lands = survey.lands
    if all(number in survey.boot_kernels[land] for land in lands):
        return 0
    if any(number in survey.boot_kernels[land] for land in lands):
        return 1
    return 2 if any(number in survey.startup[land] for land in lands) else 3


def resource_count(tally: Tally) -> int:
    return len({resource for _owner, resource in tally.callers})


def priority_section(survey: SetSurvey, names: tuple[str, ...]) -> str:
    by_volume = sorted(survey.kernels, key=lambda n: -survey.kernels[n].calls)
    volume_rank = {number: rank + 1 for rank, number in enumerate(by_volume)}
    ordered = sorted(survey.kernels, key=lambda n: (tier_of(n, survey), -survey.kernels[n].calls))
    rows = []
    for position, number in enumerate(ordered, start=1):
        tally = survey.kernels[number]
        rows.append([str(position), names[number], f"{tally.calls:,}", str(volume_rank[number]),
                     str(resource_count(tally)), TIERS[tier_of(number, survey)]])
    return table(["Order", "Kernel", "Calls", "Volume rank", "Resources", "Needed by"], rows)


def boot_lists(survey: SetSurvey, names: tuple[str, ...]) -> str:
    lines = []
    for land in survey.lands:
        boot = sorted(names[n] for n in survey.boot_kernels[land])
        lines.append(f"- {land}: {len(boot)} kernels on the start-up path ({', '.join(boot)}); "
                     f"{len(survey.startup[land])} kernels in the "
                     f"{len(survey.startup_resources[land])} resources loaded at start-up.")
    return "\n".join(lines)


def render(section: str, surveys: dict[str, SetSurvey], names: tuple[str, ...]) -> str:
    primary = surveys[PRIMARY_SET]
    if section == "summary":
        return summary_section(surveys)
    if section == "kernels":
        return kernels_section(primary, names)
    if section == "subops":
        return subops_section(primary, names)
    if section == "arguments":
        return arguments_section(primary, names)
    if section == "versions":
        return versions_section(surveys, names)
    return priority_section(primary, names) + "\n\n" + boot_lists(primary, names)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT, help="directory of the sets")
    parser.add_argument("--section", choices=SECTIONS, action="append",
                        help="section to print (repeatable, default all)")
    arguments = parser.parse_args()
    available = tuple(s for s in ALL_SETS if (arguments.root / s).is_dir())
    if PRIMARY_SET not in available:
        print(f"{arguments.root / PRIMARY_SET}: not found", file=sys.stderr)
        return 1
    names = load_kernel_names()
    surveys = survey_sets(arguments.root, available, names)
    for section in arguments.section or SECTIONS:
        print(f"## {section}\n\n{render(section, surveys, names)}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())

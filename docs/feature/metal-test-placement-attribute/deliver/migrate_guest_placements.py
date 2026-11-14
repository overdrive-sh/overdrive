#!/usr/bin/env python3
"""Moves guest-group placements from infra/metal/test-placement.json onto the
tests they place (temporary migration tool; deleted at retirement).

For each selected table entry the script finds the test function by binary id
and module path, adds `#[overdrive_testing::overdrive_test(guest = "<group>")]`
directly above the function, adds `overdrive-testing` to the owning crate's
dev-dependencies when the crate lacks it, and deletes the table entry.

It reads the table by parsing the JSON. It never adds a table entry, never
changes a group (the group written on the test is the group the entry carried),
and reports, without touching, any entry it cannot place unambiguously: a
missing function, two functions of one name, a function inside a macro, a
binary outside the workspace. Reported entries stay in the table.

Usage: migrate_guest_placements.py --groups core,probe,... [--acknowledge FILE]
       [--dry-run]

`--acknowledge` names a JSON object of "<binary>::<test>" -> reason: entries
held back on purpose (reported to the user, left in the table, never migrated).
Any other report makes the exit status 3 and nothing changes.
"""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
TABLE = ROOT / "infra/metal/test-placement.json"
ATTRIBUTE = '#[overdrive_testing::overdrive_test(guest = "{group}")]'


def workspace_members():
    meta = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"],
            cwd=ROOT,
        )
    )
    return {p["name"]: Path(p["manifest_path"]).parent for p in meta["packages"]}


def module_file(root_file, segments):
    """The file and the remaining inline-module depth holding `segments`."""
    file = root_file
    base = root_file.parent
    text = file.read_text()
    for segment in segments:
        if re.search(rf"^\s*(pub(\([a-z]+\))?\s+)?mod\s+{re.escape(segment)}\s*\{{", text, re.M):
            base = base / segment
            continue
        candidate = base / f"{segment}.rs"
        if candidate.is_file():
            file = candidate
            text = file.read_text()
            base = base / segment
            continue
        candidate = base / segment / "mod.rs"
        if candidate.is_file():
            file = candidate
            text = file.read_text()
            base = base / segment
            continue
        return None
    return file


def inside_macro(lines, index):
    """True when line `index` is inside a macro_rules! body."""
    depth = 0
    in_macro = False
    macro_depth = 0
    for number, line in enumerate(lines):
        stripped = line.split("//")[0]
        if re.search(r"\bmacro_rules!", stripped) and not in_macro:
            in_macro = True
            macro_depth = depth
        if number == index and in_macro:
            return True
        depth += stripped.count("{") - stripped.count("}")
        if in_macro and depth <= macro_depth and "}" in stripped and number > 0:
            if depth <= macro_depth:
                in_macro = False
    return False


def locate(members, binary, test):
    """(file, line index of `fn`) or a string describing why not."""
    package, _, target = binary.partition("::")
    if package not in members:
        return f"binary {binary} is not a workspace package"
    directory = members[package]
    root_file = directory / "tests" / f"{target}.rs"
    if not root_file.is_file():
        return f"no test target file {root_file.relative_to(ROOT)}"
    *modules, leaf = test.split("::")
    file = module_file(root_file, modules)
    if file is None:
        return f"module path {'::'.join(modules)} does not resolve from {root_file.relative_to(ROOT)}"
    lines = file.read_text().split("\n")
    pattern = re.compile(rf"\bfn\s+{re.escape(leaf)}\s*[(<]")
    hits = [i for i, line in enumerate(lines) if pattern.search(line)]
    if not hits:
        return f"no fn {leaf} in {file.relative_to(ROOT)}"
    if len(hits) > 1:
        return f"{len(hits)} fns named {leaf} in {file.relative_to(ROOT)}"
    index = hits[0]
    if inside_macro(lines, index):
        return f"fn {leaf} is inside a macro_rules! body in {file.relative_to(ROOT)}"
    above = index - 1
    attributes = []
    while above >= 0 and (
        lines[above].lstrip().startswith("#[") or lines[above].lstrip().startswith(")")
        or lines[above].lstrip().startswith("//")
    ):
        attributes.append(lines[above])
        above -= 1
    if not any(re.match(r"\s*#\[(tokio::)?test\b", line) for line in attributes):
        return f"fn {leaf} in {file.relative_to(ROOT)} carries no #[test]"
    if any("overdrive_test" in line for line in attributes):
        return f"fn {leaf} in {file.relative_to(ROOT)} already carries a placement"
    return file, index


def ensure_dev_dependency(directory):
    manifest = directory / "Cargo.toml"
    text = manifest.read_text()
    section = re.search(r"^\[dev-dependencies\]\n(.*?)(?=^\[|\Z)", text, re.M | re.S)
    dependencies = re.search(r"^\[dependencies\]\n(.*?)(?=^\[|\Z)", text, re.M | re.S)
    for found in (section, dependencies):
        if found and re.search(r"^overdrive-testing\b", found.group(1), re.M):
            return False
    line = "# `#[overdrive_test]` places the guest-placed tests.\noverdrive-testing.workspace = true\n"
    if section:
        end = section.end(1)
        body = section.group(1)
        text = text[:end] + ("" if body.endswith("\n") else "\n") + line + text[end:]
    else:
        text = text.rstrip("\n") + "\n\n[dev-dependencies]\n" + line
    manifest.write_text(text)
    return True


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--groups", required=True)
    parser.add_argument("--acknowledge")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    groups = set(args.groups.split(","))
    acknowledged = json.loads(Path(args.acknowledge).read_text()) if args.acknowledge else {}

    original = TABLE.read_text()
    table = json.loads(original)
    assert json.dumps(table, indent=2, ensure_ascii=False) + "\n" == original, "table is not canonical"
    members = workspace_members()

    selected = [
        (binary, test, descriptor)
        for binary, tests in table["placements"].items()
        for test, descriptor in tests.items()
        if descriptor["kind"] == "guest-group" and descriptor["group"] in groups
    ]
    findings = []
    plan = []
    for binary, test, descriptor in selected:
        if f"{binary}::{test}" in acknowledged:
            findings.append((binary, test, "held: " + acknowledged[f"{binary}::{test}"]))
            continue
        if set(descriptor) != {"kind", "group"}:
            findings.append((binary, test, f"descriptor has unexpected fields {sorted(descriptor)}"))
            continue
        where = locate(members, binary, test)
        if isinstance(where, str):
            findings.append((binary, test, where))
        else:
            plan.append((binary, test, descriptor["group"], where))

    unacknowledged = [f for f in findings if f"{f[0]}::{f[1]}" not in acknowledged]
    for binary, test, reason in findings:
        mark = "acknowledged" if f"{binary}::{test}" in acknowledged else "STOP"
        print(f"{mark}: {binary}::{test}: {reason}", file=sys.stderr)
    if unacknowledged:
        print(f"{len(unacknowledged)} unacknowledged finding(s); nothing changed", file=sys.stderr)
        return 3

    edits = {}
    for binary, test, group, (file, index) in plan:
        edits.setdefault(file, []).append((index, group))
    for file, entries in edits.items():
        lines = file.read_text().split("\n")
        for index, group in sorted(entries, reverse=True):
            indent = re.match(r"\s*", lines[index]).group(0)
            lines.insert(index, indent + ATTRIBUTE.format(group=group))
        if not args.dry_run:
            file.write_text("\n".join(lines))
    if not args.dry_run:
        for file in edits:
            owner = next(
                directory for directory in members.values() if directory in file.parents
            )
            if ensure_dev_dependency(owner):
                print(f"added overdrive-testing to {owner.relative_to(ROOT)}/Cargo.toml")
        for binary, test, _, _ in plan:
            del table["placements"][binary][test]
            if not table["placements"][binary]:
                del table["placements"][binary]
        TABLE.write_text(json.dumps(table, indent=2, ensure_ascii=False) + "\n")
    per_group = {}
    for _, _, group, _ in plan:
        per_group[group] = per_group.get(group, 0) + 1
    print(json.dumps({"migrated": per_group, "left": len(findings)}))
    return 0


if __name__ == "__main__":
    sys.exit(main())

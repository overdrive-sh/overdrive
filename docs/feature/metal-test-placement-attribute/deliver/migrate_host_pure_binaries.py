#!/usr/bin/env python3
"""Moves host-pure placements from infra/metal/test-placement.json onto a
binary default in the test source (temporary migration tool; deleted at
retirement).

For each table binary of the selected packages the script requires every table
entry of the binary to be host-pure and the binary to hold no placement record
of another kind (`--listing`, the output of `cargo xtask placement-lint --emit`
taken before the batch, lists the records the sources already declare).
Such a host-only binary takes one `overdrive_test_default!(host_pure);` at its
crate root (a lib or bin target's under `#[cfg(test)]`: dev-dependencies are not built into it), the entries of the binary are
deleted from the table, and `overdrive-testing-macros` or `overdrive-testing`
is added to the owning crate's dev-dependencies when it lacks both.

It never adds a table entry. A binary whose listing holds anything but
host-pure, a binary without a
unique target, or a root that already declares is reported, left alone, and
makes the exit status 3 with nothing changed.

Usage: migrate_host_pure_binaries.py --packages a,b --listing FILE
       [--macro-crate overdrive_testing] [--dry-run]
"""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
TABLE = ROOT / "infra/metal/test-placement.json"
MACRO = "overdrive_test_default"


def cargo_metadata():
    return json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"], cwd=ROOT
        )
    )


def root_file(packages, binary):
    package, _, target = binary.partition("::")
    meta = packages.get(package)
    if meta is None:
        return None
    if target == "":
        wanted = ("lib",)
        name = None
    elif target.startswith("bin/"):
        wanted = ("bin",)
        name = target[4:]
    else:
        wanted = ("test",)
        name = target
    hits = [
        t for t in meta["targets"]
        if t["kind"][0] in wanted and (name is None or t["name"] == name)
    ]
    if len(hits) != 1:
        return None
    return Path(hits[0]["src_path"])


def module_file_exact(root, segments):
    """The file whose top-level module is exactly `segments`, else None (the
    module is inline, or does not resolve). Inline `mod x {` segments shift the
    lookup base the way rustc does."""
    file = root
    base = root.parent
    text = file.read_text()
    resolved_as_file = not segments
    for segment in segments:
        if re.search(rf"^\s*(pub(\([a-z]+\))?\s+)?mod\s+{re.escape(segment)}\s*\{{", text, re.M):
            base = base / segment
            resolved_as_file = False
            continue
        for candidate in (base / f"{segment}.rs", base / segment / "mod.rs"):
            if candidate.is_file():
                file = candidate
                text = file.read_text()
                base = base / segment
                resolved_as_file = True
                break
        else:
            return None
    return file if resolved_as_file else None


def header_end(lines):
    end = 0
    index = 0
    while index < len(lines):
        line = lines[index].strip()
        if line.startswith("#!["):
            balance = line.count("[") - line.count("]")
            while balance > 0:
                index += 1
                balance += lines[index].count("[") - lines[index].count("]")
            end = index + 1
        elif line.startswith("//!"):
            end = index + 1
        elif line == "" or line.startswith("//"):
            pass
        else:
            break
        index += 1
    return end


def insertion_point(lines):
    """Index at which the import and the declaration go: after the last
    top-level `use`, but ahead of a top-level `#[cfg(test)] mod`, else after
    the header."""
    last_use = None
    index = 0
    while index < len(lines):
        if re.match(r"(pub(\([a-z]+\))?\s+)?use ", lines[index]):
            end = index
            while not lines[end].rstrip().endswith(";"):
                end += 1
            last_use = end + 1
            index = end + 1
        else:
            index += 1
    point = last_use if last_use is not None else header_end(lines)
    for i in range(len(lines) - 1):
        if lines[i].strip() == "#[cfg(test)]" and re.match(r"(pub\s+)?mod\s+\w+\s*\{", lines[i + 1]):
            if i < point:
                point = i
            break
    return point


def declare(file, macro_crate, macro, lib):
    lines = file.read_text().split("\n")
    if any(re.search(rf"\b{macro}!", line) for line in lines):
        return False
    gate = ["#[cfg(test)]"] if lib else []
    block = (
        gate + [f"use {macro_crate}::{macro};", ""] + gate + [f"{macro}!(host_pure);"]
    )
    point = insertion_point(lines)
    before = lines[point - 1] if point > 0 else ""
    after = lines[point] if point < len(lines) else ""
    block = ([""] if before.strip() else []) + block + ([""] if after.strip() else [])
    lines[point:point] = block
    file.write_text("\n".join(lines))
    return True


def ensure_dev_dependency(directory, package):
    manifest = directory / "Cargo.toml"
    text = manifest.read_text()
    section = re.search(r"^\[dev-dependencies\]\n(.*?)(?=^\[|\Z)", text, re.M | re.S)
    dependencies = re.search(r"^\[dependencies\]\n(.*?)(?=^\[|\Z)", text, re.M | re.S)
    for found in (section, dependencies):
        if found and re.search(r"^(overdrive-testing|overdrive-testing-macros)\b", found.group(1), re.M):
            return False
    line = f"# `overdrive_test_default!` places the host-pure test binaries.\n{package}.workspace = true\n"
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
    parser.add_argument("--packages", required=True)
    parser.add_argument("--listing", required=True)
    parser.add_argument("--macro-crate", default="overdrive_testing_macros")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    selected = set(args.packages.split(","))
    macro_package = args.macro_crate.replace("_", "-")

    original = TABLE.read_text()
    table = json.loads(original)
    assert json.dumps(table, indent=2, ensure_ascii=False) + "\n" == original, "table is not canonical"
    listing = json.loads(Path(args.listing).read_text())["placements"]
    meta = cargo_metadata()
    packages = {p["name"]: p for p in meta["packages"]}

    findings = []
    plan = []
    mixed = []
    for binary, tests in table["placements"].items():
        package = binary.partition("::")[0]
        if package not in selected:
            continue
        records = listing.get(binary, {})
        other = sorted({d["kind"] for d in records.values()} - {"host-pure"})
        if {d["kind"] for d in tests.values()} != {"host-pure"}:
            findings.append((binary, "table entries of another kind"))
            continue
        file = root_file(packages, binary)
        if file is None or not file.is_file():
            findings.append((binary, "no unique target root file"))
            continue
        if not other:
            if any(MACRO in line for line in file.read_text().split("\n")):
                findings.append((binary, f"{file.relative_to(ROOT)} already declares a default"))
                continue
            plan.append((binary, file, not binary.partition("::")[2] or binary.partition("::")[2].startswith("bin/"), len(tests), len(records)))
            continue
        modules = {}
        for test in tests:
            *path, _ = test.split("::")
            modules.setdefault(tuple(path), 0)
            modules[tuple(path)] += 1
        files = {}
        failed = False
        for path, count in modules.items():
            module = module_file_exact(file, list(path))
            if module is None or module == file:
                findings.append((binary, f"module {'::'.join(path)} is not an out-of-line module file"))
                failed = True
            elif any(re.search(r"\bplacement!", l) for l in module.read_text().split("\n")):
                findings.append((binary, f"{module.relative_to(ROOT)} already declares a placement"))
                failed = True
            else:
                files[module] = count
        if not failed:
            mixed.append((binary, files, len(tests)))
    for binary, reason in findings:
        print(f"STOP: {binary}: {reason}", file=sys.stderr)
    if findings:
        print(f"{len(findings)} finding(s); nothing changed", file=sys.stderr)
        return 3
    if not args.dry_run:
        for binary, file, lib, _, _ in plan:
            declare(file, args.macro_crate, MACRO, lib)
            owner = Path(packages[binary.partition("::")[0]]["manifest_path"]).parent
            if ensure_dev_dependency(owner, macro_package):
                print(f"added {macro_package} to {owner.relative_to(ROOT)}/Cargo.toml")
            del table["placements"][binary]
        for binary, files, _ in mixed:
            for module in files:
                declare(module, args.macro_crate, "placement", False)
            owner = Path(packages[binary.partition("::")[0]]["manifest_path"]).parent
            if ensure_dev_dependency(owner, macro_package):
                print(f"added {macro_package} to {owner.relative_to(ROOT)}/Cargo.toml")
            del table["placements"][binary]
        TABLE.write_text(json.dumps(table, indent=2, ensure_ascii=False) + "\n")
    for binary, file, lib, listed, resolved in plan:
        print(f"{binary}: default in {file.relative_to(ROOT)} (table {listed}, records {resolved})")
    for binary, files, listed in mixed:
        print(f"{binary}: placement!(host_pure) in {len(files)} module files (table {listed})")
    print(json.dumps({"default_binaries": len(plan), "default_entries": sum(p[3] for p in plan),
                      "module_binaries": len(mixed), "module_declarations": sum(len(m[1]) for m in mixed),
                      "module_entries": sum(m[2] for m in mixed)}))
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Moves host-pure placements from infra/metal/test-placement.json onto
declarations in the test source (temporary migration tool; deleted at
retirement).

For each table binary of the selected packages the script requires every table
entry of the binary to be host-pure. `--listing` (the output of
`cargo xtask placement-lint --emit` taken before the batch) lists the records
the sources already declare.

- A host-only binary (the listing holds nothing but host-pure for it) takes one
  `overdrive_test_default!(host_pure);` at its crate root (a lib or bin
  target's under `#[cfg(test)]`: dev-dependencies are not built into it).
- A mixed binary (the listing holds a guest, lane or launcher record) takes one
  `placement!(host_pure);` in each module that holds a table test, whether the
  module is a file or an inline `mod x { ... }`. A child module declares its
  own; none inherits. A guest test of the module keeps its own attribute,
  which wins over the module declaration.

The entries of the binary are deleted from the table, and
`overdrive-testing-macros` is added to the owning crate's dev-dependencies when
it has neither that nor `overdrive-testing`. A package that has
`overdrive-testing` imports the macros from it, the rest from the macro crate.

It never adds a table entry. A binary without a unique target, a module that
does not resolve to one declaration, or a root or module that already declares
is reported, left alone, and makes the exit status 3 with nothing changed.

Usage: migrate_host_pure_binaries.py --packages a,b --listing FILE [--dry-run]
"""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
TABLE = ROOT / "infra/metal/test-placement.json"
DEFAULT_MACRO = "overdrive_test_default"
MODULE_MACRO = "placement"


def cargo_metadata():
    return json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"], cwd=ROOT
        )
    )


def root_target(packages, binary):
    """(root file, target kind) of the binary id, or None."""
    package, _, target = binary.partition("::")
    meta = packages.get(package)
    if meta is None:
        return None
    if target == "":
        wanted, name = "lib", None
    elif target.startswith("bin/"):
        wanted, name = "bin", target[4:]
    else:
        wanted, name = "test", target
    hits = [
        t for t in meta["targets"]
        if t["kind"][0] == wanted and (name is None or t["name"] == name)
    ]
    if len(hits) != 1:
        return None
    return Path(hits[0]["src_path"]), wanted


def mask(text):
    """`text` with comments and the contents of string and char literals
    replaced by spaces (newlines kept), so braces and keywords found in it are
    code."""
    out = list(text)
    n = len(text)
    i = 0

    def blank(a, b):
        for k in range(a, b):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = text[i]
        two = text[i:i + 2]
        if two == "//":
            j = text.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif two == "/*":
            depth = 1
            j = i + 2
            while j < n and depth:
                if text[j:j + 2] == "/*":
                    depth += 1
                    j += 2
                elif text[j:j + 2] == "*/":
                    depth -= 1
                    j += 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif c in "rb" and re.match(r'(br|r)(#*)"', text[i:i + 80]) and (
            i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")
        ):
            m = re.match(r'(?:br|r)(#*)"', text[i:])
            close = '"' + m.group(1)
            j = text.find(close, i + m.end())
            j = n if j < 0 else j + len(close)
            blank(i + m.end(), j - len(close))
            i = j
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            blank(i + 1, j)
            i = j + 1
        elif c == "'":
            m = re.match(r"'(\\.[^']*|[^\\'])'", text[i:])
            if m:
                blank(i + 1, i + m.end() - 1)
                i += m.end()
            else:
                i += 1
        else:
            i += 1
    return "".join(out)


MOD = re.compile(r"\bmod\s+(\w+)\s*([{;])")
USE = re.compile(r"\buse\b")


class Source:
    """One source file: its text, the masked text and the brace depths."""

    def __init__(self, path):
        self.path = path
        self.text = path.read_text()
        self.masked = mask(self.text)
        self.depth = []
        depth = 0
        for c in self.masked:
            self.depth.append(depth)
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
        self.edits = []

    def base_depth(self, lo):
        return self.depth[lo] if lo < len(self.depth) else 0

    def children(self, lo, hi, name):
        base = self.base_depth(lo)
        return [
            m for m in MOD.finditer(self.masked, lo, hi)
            if m.group(1) == name and self.depth[m.start()] == base
        ]

    def attributes(self, position):
        before = self.masked[:position]
        boundary = max(before.rfind(";"), before.rfind("{"), before.rfind("}"))
        return before[boundary + 1:]

    def body_end(self, brace):
        want = self.depth[brace]
        for k in range(brace + 1, len(self.masked)):
            if self.masked[k] == "}" and self.depth[k] == want + 1:
                return k
        raise ValueError("unbalanced braces")

    def statement_end(self, start):
        """Offset just past the `;` that ends the statement at `start`."""
        k = start
        nest = 0
        while k < len(self.masked):
            c = self.masked[k]
            if c in "{[(":
                nest += 1
            elif c in "}])":
                nest -= 1
            elif c == ";" and nest == 0:
                return k + 1
            k += 1
        raise ValueError("unterminated statement")

    def line_start(self, offset):
        return self.text.rfind("\n", 0, offset) + 1

    def line_end(self, offset):
        j = self.text.find("\n", offset)
        return len(self.text) if j < 0 else j + 1


def gated_by_test(attributes):
    stripped = re.sub(r"not\(\s*test\s*\)", "", attributes)
    return bool(re.search(r"cfg\s*\(.*\btest\b", stripped, re.S))


FILE_GATE = re.compile(r"#!\[cfg\([^\]]*\btest\b")


class Resolved:
    def __init__(self, source, lo, hi, inline, gated, mod_start):
        self.source, self.lo, self.hi = source, lo, hi
        self.inline, self.gated, self.mod_start = inline, gated, mod_start


def resolve_module(sources, root, segments, root_gated):
    """Where the module `segments` of the crate rooted at `root` lives: a
    Resolved, or a string saying why it does not resolve to one place."""
    source = sources.setdefault(root, Source(root))
    lo, hi = 0, len(source.text)
    base = root.parent
    gated = root_gated or bool(FILE_GATE.search(source.masked))
    inline, mod_start = False, None
    for segment in segments:
        hits = source.children(lo, hi, segment)
        if len(hits) != 1:
            return f"module {segment} resolves to {len(hits)} declarations in {source.path.relative_to(ROOT)}"
        hit = hits[0]
        attributes = source.attributes(hit.start())
        if re.search(r"\bpath\s*=", attributes):
            window = source.text[max(0, hit.start() - len(attributes) - 1):hit.start()]
            named = re.search(r'\bpath\s*=\s*"([^"]+)"', window)
            if hit.group(2) != ";" or named is None or segment != segments[-1]:
                return f"module {segment} carries a path attribute this tool does not resolve"
            target = source.path.parent / named.group(1)
            if not target.is_file():
                return f"module {segment} path attribute names no file"
            gated = gated or gated_by_test(attributes)
            source = sources.setdefault(target, Source(target))
            lo, hi = 0, len(source.text)
            inline, mod_start = False, None
            gated = gated or bool(FILE_GATE.search(source.masked))
            continue
        gated = gated or gated_by_test(attributes)
        if hit.group(2) == "{":
            lo, hi = hit.end(), source.body_end(hit.end() - 1)
            inline, mod_start = True, hit.start()
            base = base / segment
            continue
        for candidate in (base / f"{segment}.rs", base / segment / "mod.rs"):
            if candidate.is_file():
                source = sources.setdefault(candidate, Source(candidate))
                lo, hi = 0, len(source.text)
                inline, mod_start = False, None
                gated = gated or bool(FILE_GATE.search(source.masked))
                base = base / segment
                break
        else:
            return f"module {segment} has no file under {base.relative_to(ROOT)}"
    return Resolved(source, lo, hi, inline, gated, mod_start)


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
    """Line index at which a file-level import and declaration go: after the
    last top-level `use`, but ahead of a top-level `#[cfg(test)] mod`, else
    after the header."""
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


def has_import(source, lo, hi, name):
    base = source.base_depth(lo)
    for m in USE.finditer(source.masked, lo, hi):
        if source.depth[m.start()] != base:
            continue
        statement = source.masked[m.start():source.statement_end(m.start())]
        if re.search(rf"\b{name}\b", statement):
            return True
    return False


def conflicts(source, lo, hi, name):
    base = source.base_depth(lo)
    pattern = re.compile(rf"\b(fn|mod|struct|enum|const|static|type|trait)\s+{name}\b")
    return any(source.depth[m.start()] == base for m in pattern.finditer(source.masked, lo, hi))


def declares(source, lo, hi, macro):
    base = source.base_depth(lo)
    return any(
        source.depth[m.start()] == base
        for m in re.compile(rf"\b{macro}\s*!").finditer(source.masked, lo, hi)
    )


def declaration_lines(macro_crate, macro, gate, with_import, indent):
    gate_line = [f"{indent}#[cfg(test)]"] if gate else []
    lines = []
    if with_import:
        lines += gate_line + [f"{indent}use {macro_crate}::{macro};", ""]
        call = f"{indent}{macro}!(host_pure);"
    else:
        call = f"{indent}{macro_crate}::{macro}!(host_pure);"
    return lines + gate_line + [call]


def plan_file_level(source, macro_crate, macro, gate):
    """Queue the declaration at the top level of `source`."""
    lines = source.text.split("\n")
    import_needed = not has_import(source, 0, len(source.text), macro)
    if conflicts(source, 0, len(source.text), macro):
        import_needed = False
    block = declaration_lines(macro_crate, macro, gate, import_needed, "")
    point = insertion_point(lines)
    before = lines[point - 1] if point > 0 else ""
    after = lines[point] if point < len(lines) else ""
    block = ([""] if before.strip() else []) + block + ([""] if after.strip() else [])
    offset = sum(len(l) + 1 for l in lines[:point])
    source.edits.append((offset, "\n".join(block) + "\n"))


def plan_inline(source, resolved, macro_crate, macro, gate):
    """Queue the declaration inside the inline module `resolved`."""
    text, masked = source.text, source.masked
    lo, hi = resolved.lo, resolved.hi
    rest_of_line = text[lo:source.line_end(lo)]
    if rest_of_line.strip():
        return f"module at {source.path.relative_to(ROOT)} has items on the line of its opening brace"
    indent = re.match(r"[ \t]*", text[source.line_start(resolved.mod_start):]).group(0) + "    "
    cursor = source.line_end(lo)
    last_end = None
    documented = False
    while text.startswith("//!", cursor) or re.match(r"[ \t]*//!", text[cursor:source.line_end(cursor)]):
        last_end = source.line_end(cursor) - 1
        cursor = source.line_end(cursor)
        documented = True
    while True:
        m = re.compile(r"\s*").match(masked, cursor)
        cursor = m.end()
        if cursor >= hi:
            break
        if masked.startswith("#![", cursor):
            close = masked.index("]", cursor)
            cursor = close + 1
            last_end = cursor
        elif masked.startswith("#[", cursor):
            nest = 0
            k = cursor + 1
            while True:
                if masked[k] == "[":
                    nest += 1
                elif masked[k] == "]":
                    nest -= 1
                    if nest == 0:
                        break
                k += 1
            after = re.compile(r"\s*").match(masked, k + 1).end()
            if re.match(r"(pub(\([a-z:]+\))?\s+)?use\b", masked[after:after + 20]):
                cursor = source.statement_end(after)
                last_end = cursor
            else:
                break
        elif re.match(r"(pub(\([a-z:]+\))?\s+)?use\b", masked[cursor:cursor + 20]):
            cursor = source.statement_end(cursor)
            last_end = cursor
        else:
            break
    if last_end is None:
        offset = source.line_end(lo)
    else:
        offset = source.line_end(last_end)
    import_needed = not has_import(source, lo, hi, macro) and not conflicts(source, lo, hi, macro)
    block = declaration_lines(macro_crate, macro, gate, import_needed, indent)
    following = text[offset:source.line_end(offset)] if offset < len(text) else ""
    preceded_by_use = last_end is not None
    prefix = ["", ] if ((preceded_by_use and not import_needed) or documented) else []
    block = prefix + block
    block_text = "\n".join(block) + "\n"
    if following.strip() and not following.strip().startswith("}"):
        block_text += "\n"
    source.edits.append((offset, block_text))
    return None


def apply_edits(sources):
    for source in sources.values():
        if not source.edits:
            continue
        text = source.text
        for offset, block in sorted(source.edits, key=lambda e: -e[0]):
            text = text[:offset] + block + text[offset:]
        source.path.write_text(text)


def macro_crate_for(directory):
    """The crate a package imports the placement macros from."""
    text = (directory / "Cargo.toml").read_text()
    if re.search(r"^overdrive-testing\s*[.=]", text, re.M):
        return "overdrive_testing"
    return "overdrive_testing_macros"


def ensure_dev_dependency(directory, package):
    manifest = directory / "Cargo.toml"
    text = manifest.read_text()
    section = re.search(r"^\[dev-dependencies\]\n(.*?)(?=^\[|\Z)", text, re.M | re.S)
    dependencies = re.search(r"^\[dependencies\]\n(.*?)(?=^\[|\Z)", text, re.M | re.S)
    for found in (section, dependencies):
        if found and re.search(r"^(overdrive-testing|overdrive-testing-macros)\b", found.group(1), re.M):
            return False
    line = f"# `overdrive_test_default!` and `placement!` place the host-pure tests.\n{package}.workspace = true\n"
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
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    selected = set(args.packages.split(","))

    original = TABLE.read_text()
    table = json.loads(original)
    assert json.dumps(table, indent=2, ensure_ascii=False) + "\n" == original, "table is not canonical"
    listing = json.loads(Path(args.listing).read_text())["placements"]
    meta = cargo_metadata()
    packages = {p["name"]: p for p in meta["packages"]}

    findings = []
    defaults = []
    mixed = []
    sources = {}
    for binary, tests in table["placements"].items():
        package = binary.partition("::")[0]
        if package not in selected:
            continue
        owner = Path(packages[package]["manifest_path"]).parent
        macro_crate = macro_crate_for(owner)
        records = listing.get(binary, {})
        other = sorted({d["kind"] for d in records.values()} - {"host-pure"})
        if {d["kind"] for d in tests.values()} != {"host-pure"}:
            findings.append((binary, "table entries of another kind"))
            continue
        target = root_target(packages, binary)
        if target is None or not target[0].is_file():
            findings.append((binary, "no unique target root file"))
            continue
        root, kind = target
        if not other:
            source = sources.setdefault(root, Source(root))
            if declares(source, 0, len(source.text), DEFAULT_MACRO):
                findings.append((binary, f"{root.relative_to(ROOT)} already declares a default"))
                continue
            defaults.append((binary, source, kind != "test", macro_crate, owner, len(tests), len(records)))
            continue
        modules = {}
        for test in tests:
            *path, _ = test.split("::")
            modules[tuple(path)] = modules.get(tuple(path), 0) + 1
        placed = []
        failed = False
        for path, count in modules.items():
            resolved = resolve_module(sources, root, list(path), kind == "test")
            if isinstance(resolved, str):
                findings.append((binary, resolved))
                failed = True
                continue
            if declares(resolved.source, resolved.lo, resolved.hi, MODULE_MACRO):
                findings.append((binary, f"module {'::'.join(path) or '(root)'} already declares a placement"))
                failed = True
                continue
            placed.append((path, resolved, count))
        if not failed:
            mixed.append((binary, placed, macro_crate, owner, len(tests)))

    # Queue the edits; a module resolved twice (two tests, one module) is
    # declared once by construction of `modules`.
    inline_findings = []
    for binary, source, lib, macro_crate, owner, _, _ in defaults:
        plan_file_level(source, macro_crate, DEFAULT_MACRO, lib)
    for binary, placed, macro_crate, owner, _ in mixed:
        for path, resolved, _ in placed:
            gate = not resolved.gated
            if resolved.inline:
                problem = plan_inline(resolved.source, resolved, macro_crate, MODULE_MACRO, gate)
                if problem:
                    inline_findings.append((binary, problem))
            else:
                plan_file_level(resolved.source, macro_crate, MODULE_MACRO, gate)
    for binary, reason in findings + inline_findings:
        print(f"STOP: {binary}: {reason}", file=sys.stderr)
    if findings or inline_findings:
        print(f"{len(findings) + len(inline_findings)} finding(s); nothing changed", file=sys.stderr)
        return 3
    gated_modules = sum(1 for _, placed, _, _, _ in mixed for _, r, _ in placed if not r.gated)
    if not args.dry_run:
        apply_edits(sources)
        for binary, _, _, macro_crate, owner, _, _ in defaults:
            if macro_crate == "overdrive_testing_macros" and ensure_dev_dependency(owner, "overdrive-testing-macros"):
                print(f"added overdrive-testing-macros to {owner.relative_to(ROOT)}/Cargo.toml")
            del table["placements"][binary]
        for binary, _, macro_crate, owner, _ in mixed:
            if macro_crate == "overdrive_testing_macros" and ensure_dev_dependency(owner, "overdrive-testing-macros"):
                print(f"added overdrive-testing-macros to {owner.relative_to(ROOT)}/Cargo.toml")
            del table["placements"][binary]
        TABLE.write_text(json.dumps(table, indent=2, ensure_ascii=False) + "\n")
    for binary, source, lib, macro_crate, owner, listed, resolved in defaults:
        print(f"{binary}: default in {source.path.relative_to(ROOT)} (table {listed}, records {resolved}, import {macro_crate})")
    for binary, placed, macro_crate, owner, listed in mixed:
        inline = sum(1 for _, r, _ in placed if r.inline)
        print(f"{binary}: placement!(host_pure) in {len(placed)} modules ({inline} inline) (table {listed}, import {macro_crate})")
    print(json.dumps({"default_binaries": len(defaults), "default_entries": sum(d[5] for d in defaults),
                      "module_binaries": len(mixed), "module_declarations": sum(len(m[1]) for m in mixed),
                      "declarations_needing_their_own_cfg_test": gated_modules,
                      "module_entries": sum(m[4] for m in mixed)}))
    return 0


if __name__ == "__main__":
    sys.exit(main())

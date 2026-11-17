#!/usr/bin/env python3
"""Reads cargo --message-format=json (stdin or file) and writes one TSV row per
compiler-artifact: kind, package, target, fresh flag, executable sha256 and the
sha256 of its overdrive_test_placement ELF section (if any).
Usage: manifest.py ARTIFACTS_JSONL > manifest.tsv
"""
import hashlib
import json
import os
import subprocess
import sys
import tempfile


def sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def section_bytes(path):
    with tempfile.NamedTemporaryFile() as out:
        r = subprocess.run(
            ["objcopy", "-O", "binary", "--only-section=overdrive_test_placement", path, out.name],
            capture_output=True,
        )
        if r.returncode != 0 or os.path.getsize(out.name) == 0:
            return None
        return open(out.name, "rb").read()


def parse_records(raw):
    """NUL-terminated fields: kind, test, argc, then argc arguments."""
    f = raw.split(b"\0")
    recs, i = [], 0
    while i + 2 < len(f):
        if f[i] == b"":
            i += 1
            continue
        try:
            argc = int(f[i + 2])
        except ValueError:
            return None
        recs.append(tuple(f[i:i + 3 + argc]))
        i += 3 + argc
    return recs


def section_sha(path):
    """raw section sha256 and order-independent sha256 of the parsed record set"""
    raw = section_bytes(path)
    if raw is None:
        return "-", "-"
    recs = parse_records(raw)
    rs = "unparsed" if recs is None else hashlib.sha256(repr(sorted(recs)).encode()).hexdigest()
    return hashlib.sha256(raw).hexdigest(), rs


def main():
    src = open(sys.argv[1]) if len(sys.argv) > 1 else sys.stdin
    print("kind\tpackage\ttarget\tfresh\tfile\tsha256\tplacement_section_sha256\tplacement_records_sha256")
    for line in src:
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("reason") != "compiler-artifact":
            continue
        t = m["target"]
        pkg = m["package_id"].split("#")[-1].split(" ")[0]
        files = [m["executable"]] if m.get("executable") else m.get("filenames", [])
        for f in files:
            if not os.path.isfile(f):
                continue
            is_exe = bool(m.get("executable"))
            raw_sha, rec_sha = section_sha(f) if is_exe else ("-", "-")
            print("\t".join([
                ",".join(t["kind"]), pkg, t["name"], str(m.get("fresh")).lower(),
                f, sha(f), raw_sha, rec_sha,
            ]))


main()

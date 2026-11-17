#!/usr/bin/env python3
"""Local analysis of a ladder/conc evidence directory (never runs on the box).
Usage: analyze.py <evidence/<run>/bus> [vm ...]
Prints, per VM: sample timings, and per state the kache event results grouped by
output kind (deps, ws-lib, proc-macro, bin, test, build-script-compile,
build-script-run, other).
"""
import collections
import csv
import gzip
import json
import os
import statistics
import sys
from datetime import datetime


def ts(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()


def samples(vm_dir):
    rows = []
    p = os.path.join(vm_dir, "samples.tsv")
    if not os.path.exists(p):
        return rows
    for line in open(p):
        f = line.rstrip("\n").split("\t")
        if len(f) < 8:
            continue
        rows.append({"end": ts(f[0]), "label": f[3], "rc": f[4], "wall": float(f[5]), "cpu": f[6], "rss": f[7]})
    return rows


def kinds(vm_dir):
    """crate name -> category, from any artifacts-*.jsonl.gz of the VM."""
    m = {}
    for fn in sorted(os.listdir(vm_dir)):
        if not (fn.startswith("artifacts-") and fn.endswith(".jsonl.gz")):
            continue
        for line in gzip.open(os.path.join(vm_dir, fn), "rt"):
            try:
                a = json.loads(line)
            except ValueError:
                continue
            t = a["target"]
            name = t["name"].replace("-", "_")
            ws = a["package_id"].startswith("path+") or "(path+" in a["package_id"]
            k = t["kind"]
            if "proc-macro" in k:
                cat = "proc-macro"
            elif "custom-build" in k:
                cat = "build-script-compile"
            elif "bin" in k:
                cat = "bin"
            elif "test" in k:
                cat = "test"
            elif "example" in k or "bench" in k:
                cat = "example/bench"
            else:
                cat = "ws-lib" if ws else "deps"
            m.setdefault(name, cat)
    return m


def events(vm_dir):
    out = []
    for fn in ("kache-events.jsonl",):
        p = os.path.join(vm_dir, fn)
        if os.path.exists(p):
            for line in open(p):
                try:
                    e = json.loads(line)
                except ValueError:
                    continue
                if "result" in e:
                    out.append(e)
    return out


def report(vm_dir, vm):
    s = samples(vm_dir)
    print(f"\n=== {vm}")
    for r in s:
        if r["label"].startswith("edit-") and r["label"] != "edit-setup":
            continue
        print(f"  {r['label']:<20} rc={r['rc']:<4} wall={r['wall']:>8.1f}s cpu={r['cpu']:<16} rss_kb={r['rss']}")
    loop = [r["wall"] for r in s if r["label"].startswith("edit-") and r["label"] != "edit-setup"]
    if loop:
        t = sorted(loop[2:]) if len(loop) > 4 else sorted(loop)
        print(f"  edit loop n={len(loop)} (first 2 discarded): median={statistics.median(t):.1f}s "
              f"p95={t[max(0, int(0.95 * len(t)) - 1)]:.1f}s min={t[0]:.1f}s max={t[-1]:.1f}s")
    ev = events(vm_dir)
    if not ev:
        return
    km = kinds(vm_dir)
    wins = []
    prev = None
    for r in s:
        if r["label"].startswith("edit-") or r["label"].startswith("s7") or r["label"].startswith("seed"):
            continue
        start = prev if prev is not None else 0
        wins.append((r["label"], start, r["end"]))
        prev = r["end"]
    for label, a, b in wins:
        c = collections.defaultdict(collections.Counter)
        for e in ev:
            t = ts(e["ts"])
            if not (a < t <= b + 1):
                continue
            name = e["crate_name"]
            if name == "build_script_run":
                cat = "build-script-run"
            elif name == "build_script_build":
                cat = "build-script-compile"
            else:
                cat = km.get(name, "other")
            res = e["result"]
            if res == "passthrough":
                res = "pass:" + (e.get("passthrough_reason") or "").split("|")[0][:24] + ":" + (e.get("passthrough_reason") or "").split("|")[-1][:20]
            c[cat][res] += 1
        print(f"  [{label}] events by kind:")
        for cat in sorted(c):
            print(f"      {cat:<22} {dict(c[cat])}")


def main():
    root = sys.argv[1]
    vms = sys.argv[2:] or sorted(d for d in os.listdir(root) if os.path.isdir(os.path.join(root, d)) and d != "cache")
    for vm in vms:
        report(os.path.join(root, vm), vm)


main()

#!/usr/bin/env python3
"""SPIKE helper: locate the diverging quint-connect trace and print spec states around the divergence.

usage: scripts/triage_mbt.py <mbt raw log> <test-name-substring> <out/qc dir> [nodes=0,1,2,3] [ctx=8]

The quint-connect log lists each trace's `Action taken:` names; the matching ITF file (regenerated with
the same `quint run` seed into <out/qc dir>) is found by comparing action-name sequences.
"""
import glob
import importlib.util
import json
import re
import sys

spec = importlib.util.spec_from_file_location("itf", __file__.replace("triage_mbt.py", "itf.py"))
itf = importlib.util.module_from_spec(spec)
spec.loader.exec_module(itf)

raw, test, qc = sys.argv[1], sys.argv[2], sys.argv[3]
nodes = [int(x) for x in (sys.argv[4] if len(sys.argv) > 4 else "0,1,2,3").split(",")]
ctx = int(sys.argv[5]) if len(sys.argv) > 5 else 8

text = re.sub(r"\x1b\[[0-9;]*m", "", open(raw).read())
sect = text[text.index(f"START") :]
sect = sect[sect.index(test) :]
end = sect.find("replay result =")
sect = sect[: sect.find("\n", end) + 1] if end >= 0 else sect
blocks = sect.split("[Trace ")
last = blocks[-1]
acts = re.findall(r"Action taken: (\w+)", last)
diff = [l for l in last.splitlines() if re.match(r"^   [-+] {5,}", l)]
err = re.search(r"replay result = DIVERGED: (.*)", sect)
print(f"trace #{len(blocks) - 1}: {len(acts)} steps; error: {err.group(1) if err else '?'}")
print("diff lines:", *diff, sep="\n  ")

best = None
for f in glob.glob(f"{qc}/run_*.itf.json"):
    d = json.load(open(f))
    tags = []
    for r in d["states"]:
        s = {itf.short(k): itf.dec(v) for k, v in r.items() if k != "#meta"}
        tags.append(s["lastAction"]["tag"])
    if tags[: len(acts)] == acts:
        best = (f, d)
        break
if not best:
    print("no matching ITF trace found")
    sys.exit(1)
f, d = best
print(f"matching ITF: {f}")
lo = max(0, len(acts) - 1 - ctx)
for j in range(lo, len(acts)):
    s = {itf.short(k): itf.dec(v) for k, v in d["states"][j].items() if k != "#meta"}
    la = s["lastAction"]
    v = la["value"] if isinstance(la["value"], dict) else {}
    desc = {k: x for k, x in v.items() if k != "m"}
    if "m" in v:
        m = v["m"]
        desc["m"] = f"{m['kind']} {m['src']}->{m['dst']} v{m['view']} e{m['epoch']} op{m['op']} c{m['commit']}"
    print(f"{j} {la['tag']} {desc}")
    for n, ns in sorted(s["st"].items()):
        if n in nodes:
            print(
                f"   n{n} {'' if ns['up'] else 'DOWN '}{'' if ns['started'] else 'unstarted '}{ns['status']} "
                f"e{ns['cfg']['epoch']} V{ns['cfg']['voters']} L{ns['cfg']['learners']} v{ns['view']} "
                f"lv{ns['logView']} svcT{ns['svcTarget']} svcFrom{sorted(ns['svcFrom'])} idle={ns['idle']} "
                f"log=[{' '.join(itf.entry(e) for e in ns['log'])}] holes={sorted(ns['holes'])} "
                f"commit={ns['commit']} cmax={ns['cmax']} dcommit={ns['dcommit']}"
            )

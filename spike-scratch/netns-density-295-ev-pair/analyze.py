#!/usr/bin/env python3
"""Tabulate evidence/*/run-results.jsonl (+ peer-summary.json) into markdown. Usage: analyze.py [evidence-dir-glob-prefix...]"""
import glob, json, sys, os
pat = sys.argv[1:] or ["evidence/vm-job*"]
rows = []
for p in pat:
    for d in sorted(glob.glob(p)):
        f = os.path.join(d, "run-results.jsonl")
        if not os.path.exists(f): continue
        peer = {}
        pf = os.path.join(d, "peer-summary.json")
        if os.path.exists(pf):
            try:
                for r in json.load(open(pf))["peer_runs"]: peer[r["run"]] = r
            except Exception: pass
        for l in open(f):
            if l.strip():
                r = json.loads(l); r["_dir"] = os.path.basename(d); r["_peer"] = peer.get(r["run"]); rows.append(r)
def pct(a, b): return f"{100*a/b:.0f}%" if b else "-"
print("| run | mode | shape | trials | ok | guest early(any) | early bytes A/B | early END A/B | host early(any) | peer ok/fail | wall s | evidence |")
print("|---|---|---|---:|---:|---:|---|---|---:|---|---:|---|")
for r in rows:
    g = r["guest_attach"]; h = r.get("host_attach")
    peer = r["_peer"]; ps = f'{peer["ok"]}/{peer["fail"]}' if peer else "-"
    print(f'| {r["run"]} | {r["mode"]} | {r["shape"]} | {r["lanes"]}x{r["trials_per_lane"]}={r["trials"]} | {r["ok"]} | {g["any_early"]} ({pct(g["any_early"], g["attached"])}) | '
          f'{g["early_bytes_A"]}/{g["early_bytes_B"]} | {g["early_end_A"]}/{g["early_end_B"]} | {h["any_early"] if h else "-"} | {ps} | {r["wall_s"]} | {r["_dir"][-22:]} |' + (f' reasons={r["reasons"]}' if r["reasons"] else ""))

#!/usr/bin/env python3
"""Cross-role check of the drain goals from the sampled flow events of one loop run: the survivor's own forwarded
count (goal of a vsock-source direction) must equal the peer's final-hop count (goal of the matching TCP-source
direction, tcpi_bytes_received-1). Usage: analyze_goals.py <events-prefix> (reads <prefix>.guest.jsonl, <prefix>.host.jsonl)"""
import json, sys, collections
p = sys.argv[1]
def load(role):
    d = collections.defaultdict(dict)
    try:
        for l in open(f"{p}.{role}.jsonl"):
            try: e = json.loads(l)
            except Exception: continue
            if e.get("ev") == "eof": d[e["flow"]][e["dir"]] = e
    except FileNotFoundError: pass
    return d
g, h = load("guest"), load("host")
common = sorted(set(g) & set(h))
n = ok = 0; bad = []
for f in common:
    for gd, hd in ((0, 1), (1, 0)):   # guest TCP-source dir0 <-> host vsock-source dir1 ; guest vsock-source dir1 <-> host TCP-source dir0
        a, b = g[f].get(gd), h[f].get(hd)
        if a is None or b is None: continue
        n += 1
        if a["goal"] == b["goal"]: ok += 1
        else: bad.append((f, gd, a["goal"], b["goal"]))
print(f"{p}: flows_in_both={len(common)} directions_compared={n} equal={ok} unequal={len(bad)} {bad[:5]}")

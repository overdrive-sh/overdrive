#!/usr/bin/env python3 -I
"""Pool the raw BENCH samples of one or more run logs per (cell, workload).
Usage: aggregate_bench.py log1 [log2 ...]
Warm-up = iteration 1 of each guest run (dropped). Prints a markdown table of medians and min..max."""
import re, sys, statistics as st, collections
rows = []  # (ts, kind, payload)
for path in sys.argv[1:]:
    seen = set()
    for line in open(path, errors="replace"):
        m = re.search(r"NLPV-H (\d+\.\d+) (.*)$", line)
        if not m: continue
        ts, msg = float(m.group(1)), m.group(2)
        if (ts, msg) in seen: continue
        seen.add((ts, msg))
        rows.append((ts, path, msg))
rows.sort(key=lambda r: r[0])
pending = []  # raw samples since last BENCHRESULT group
samples = collections.defaultdict(list)  # (cell, w) -> list of dict
cells_order = []
last_cell = None
for ts, path, msg in rows:
    if msg.startswith("raw BENCH "):
        kv = dict(t.split("=", 1) for t in msg[10:].split() if "=" in t)
        pending.append(kv)
    elif msg.startswith("BENCHRESULT "):
        kv = dict(t.split("=", 1) for t in msg[12:].split() if "=" in t)
        cell = kv["cell"]
        if last_cell != cell:
            # first BENCHRESULT of a cell: bind the pending raw samples to it
            for s in pending:
                samples[(cell, s["w"])].append(s)
            pending = []
            last_cell = cell
            cells_order.append((cell, kv["substrate"], kv["backing"], kv["direct"]))
    # a new test starts: reset last_cell lazily by seeing another cell name later
        # (cells alternate; same cell name in 2 runs is handled by pending being empty on 2nd group)
    elif msg.startswith("raw ") is False and msg.startswith("guest:") and "BENCHREADY" in msg:
        last_cell = None
def med(v): return st.median(v) if v else float("nan")
print("| substrate | backing | host direct | workload | n | median ms | min..max ms | median p50/p99 us |")
print("|---|---|---|---|---|---|---|---|")
done = set()
for cell, sub, bk, dr in cells_order:
    for w in ["smallfiles_nofsync", "smallfiles_fsync", "randsync", "tree"]:
        key = (cell, w)
        if key in done: continue
        done.add(key)
        keep = [s for s in samples[key] if int(s["iter"]) >= 2]
        ms = [float(s["ms"]) for s in keep]
        p50 = [float(s["p50_us"]) for s in keep if "p50_us" in s]
        p99 = [float(s["p99_us"]) for s in keep if "p99_us" in s]
        lat = f"{med(p50):.0f}/{med(p99):.0f}" if p50 else "-"
        print(f"| {sub} | {bk} | {dr} | {w} | {len(ms)} | {med(ms):.0f} | {min(ms):.0f}..{max(ms):.0f} | {lat} |")

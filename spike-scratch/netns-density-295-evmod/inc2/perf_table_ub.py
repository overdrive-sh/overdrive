#!/usr/bin/env python3
"""Aggregate the bench cohorts into the comparison tables (median [min-max] over cohorts). Usage: perf_table.py <evidence-dir>... """
import glob, json, re, statistics, sys
from analyze import events, window_rows, mem

def med(xs, f="{:.3f}"):
    xs = [x for x in xs if x is not None]
    if not xs: return "-"
    return f"{f.format(statistics.median(xs))} [{f.format(min(xs))}-{f.format(max(xs))}]"

cells = {}   # variant -> list of (path, rep)
for d in sys.argv[1:]:
    for p in sorted(glob.glob(d + "/*/native-serial.log")):
        m = re.search(r"/(\d+)-(kernel|aya|evmodub|evmod)-tcp-r(\d+)/", p)
        if m: cells.setdefault(m.group(2), []).append((p, int(m.group(3))))
names = {"kernel": "shmproxy.ko (2 kthreads/peer)", "aya": "Aya sockhash stream verdict", "evmod": "evmod.ko per-CPU wq", "evmodub": "evmod.ko unbound=1"}
order = ["aya", "evmod", "evmodub"]
print("cohorts:", {v: [r for _, r in cells.get(v, [])] for v in order})

def table(pop, active, size):
    print(f"\n#### population {pop}, active {active}, {size}-byte request/reply, unrestricted closed loop (4 outstanding)")
    print("| proxy | cohorts | GiB/s | busy cores | core-s/GiB | p50 ms | p99 ms |")
    print("|---|---:|---:|---:|---:|---:|---:|")
    for v in order:
        rows = []
        for p, rep in cells.get(v, []):
            rows += [r for r in window_rows(p, "evmod" if v=="evmodub" else v) if r["population"] == pop and r["active"] == active and r["size"] == size]
        if not rows: continue
        print(f"| {names[v]} | {len(rows)} | {med([r['gibps'] for r in rows])} | {med([r['busy'] for r in rows], '{:.2f}')} | {med([r['core_s_per_gib'] for r in rows], '{:.2f}')} | {med([r['p50_ms'] for r in rows])} | {med([r['p99_ms'] for r in rows])} |")

for pop, active in [(64, 64), (16384, 16384), (16384, 64)]:
    for size in (65536, 1024, 64):
        table(pop, active, size)

print("\n#### kernel memory and tasks (idle, population held, immediately before traffic; increase over the loaded baseline = before any peer)")
print("| proxy | population | all tasks | proxy-owned kernel threads | SUnreclaim increase MiB | KernelStack increase MiB | Slab increase MiB | kworker threads |")
print("|---|---:|---:|---|---:|---:|---:|---:|")
for v in order:
    for pop in (64, 16384):
        vals = []
        for p, rep in cells.get(v, []):
            base = idle = None
            for e in events(p):
                if e.get("event") == "loaded": base = e["snapshot"]
                if e.get("event") == "idle_resources" and e.get("population") == pop: idle = e["before"]
            if base and idle:
                kw = idle["kernel_workers"]
                own = {k: n for k, n in kw.items() if k not in ("kworker/", "vhost-")}
                vals.append((idle["all_tasks"], own, (mem(idle, "SUnreclaim") - mem(base, "SUnreclaim")) / 1024, (mem(idle, "KernelStack") - mem(base, "KernelStack")) / 1024,
                             (mem(idle, "Slab") - mem(base, "Slab")) / 1024, kw.get("kworker/", 0), kw.get("vhost-", 0)))
        if vals:
            print(f"| {names[v]} | {pop} | {med([x[0] for x in vals], '{:.0f}')} | {vals[0][1]} | {med([x[2] for x in vals], '{:.0f}')} | {med([x[3] for x in vals], '{:.0f}')} | {med([x[4] for x in vals], '{:.0f}')} | {med([x[5] for x in vals], '{:.0f}')} |")

#!/usr/bin/env python3
"""Throughput / latency table from evidence/bench-*/results.json. Usage: analyze_bench.py <evidence-dir>"""
import json, sys, pathlib, statistics

d = pathlib.Path(sys.argv[1])
r = json.loads((d / "results.json").read_text())
rows = {}
for k, v in r.items():
    if not isinstance(v, dict) or "t_total_us" not in v: continue
    parts = k.split("-")
    if parts[-1].startswith("p") and parts[-1][1:].isdigit(): parts = parts[:-1]
    if parts[-1].startswith("r") and parts[-1][1:].isdigit(): parts = parts[:-1]
    cfg, size, direction = "-".join(parts[:-2]), parts[-2], parts[-1]
    ts = sorted(v["t_total_us"])
    if not ts: continue
    med = ts[len(ts) // 2]; p99 = ts[min(len(ts) - 1, int(len(ts) * 0.99))]
    nbytes = int(size) * (2 if direction == "both" else 1)
    key = (cfg, int(size), direction)
    x = dict(ok=v["ok"], fail=v["fail"], med_ms=med / 1000, p99_ms=p99 / 1000, mbps=nbytes / (med / 1e6) / 1e6, n=len(ts))
    if key in rows:
        o = rows[key]
        rows[key] = dict(ok=o["ok"] + x["ok"], fail=o["fail"] + x["fail"], med_ms=statistics.median(o["_med"] + [x["med_ms"]]), p99_ms=max(o["p99_ms"], x["p99_ms"]),
                         mbps=statistics.median(o["_mb"] + [x["mbps"]]), n=o["n"] + x["n"], _med=o["_med"] + [x["med_ms"]], _mb=o["_mb"] + [x["mbps"]])
    else:
        x["_med"] = [x["med_ms"]]; x["_mb"] = [x["mbps"]]
        rows[key] = x
cfgs = sorted({k[0] for k in rows}); sizes = sorted({k[1] for k in rows})
for direction in ("up", "down", "both"):
    print(f"\n== {direction}  (median ms / p99 ms / MB/s of payload; fail count)")
    print("config".ljust(22) + "".join(f"{s:>30}" for s in sizes))
    for c in cfgs:
        line = c.ljust(22)
        for s in sizes:
            x = rows.get((c, s, direction))
            line += (f"{x['med_ms']:9.3f}/{x['p99_ms']:9.3f}/{x['mbps']:8.1f} f{x['fail']}".rjust(30) if x else "-".rjust(30))
        if direction in ("up", "down", "both") and False: pass
        print(line)

#!/usr/bin/env python3
"""Aggregate the retained V-13 evidence: per-cell tallies (EOF-class = EOF or EPIPE-first), latency of
host-clock observers, and the survivor-side ordering gap between the flow-vsock RDHUP and the
control-session loss (from the timestamped owner logs)."""
import json, glob, os, statistics, collections, sys
def cls(v): return "RESET" if v == "RESET" else ("EOF-class" if v in ("EOF", "OTHER_ERR") else v)
def load(pattern):
    rows = []
    for d in sorted(glob.glob(pattern)):
        for l in open(d + "/results.jsonl"):
            r = json.loads(l); r["_dir"] = d; rows.append(r)
    return rows
def tally(rows, cellprefix=""):
    t = collections.defaultdict(lambda: collections.Counter())
    for r in rows:
        if not r["cell"].startswith(cellprefix): continue
        for k, v in r.get("verdicts", {}).items(): t[(r["cell"], k)][cls(v)] += 1
    return t
def show(title, t):
    print("==", title)
    for (cell, obs), c in sorted(t.items()):
        n = sum(c.values()); print(f"  {cell:34s} {obs:32s} n={n:3d}  RESET={c['RESET']:3d}  EOF-class={c['EOF-class']:3d}  other={n-c['RESET']-c['EOF-class']}")
def evs(path):
    out = []
    if os.path.exists(path):
        for l in open(path):
            l = l.strip()
            if l.startswith("{"):
                try: out.append(json.loads(l))
                except Exception: pass
    return out
base = [r for r in load("evidence/matrix-*") if "t_ns" not in "" ]
show("BASELINE matrix (3 runs pooled: N=30+40+40 per cell)", tally(base))
show("CONTROL: no SO_LINGER anywhere (N=15)", tally(load("evidence/controls-*")))
show("ORDERS: all three flows at once, both owners killed", tally(load("evidence/orders-*")))
show("VARIANT B (EPOLLHUP discriminator), N=30", tally(load("evidence/variantB-*")))
show("VARIANT C (explicit half-close message), N=30", tally(load("evidence/variantC-*")))
# kTLS
print("== kTLS edge case")
for r in [json.loads(l) for d in sorted(glob.glob("evidence/ktls-*")) for l in open(d + "/results.jsonl")]:
    pass
for d in sorted(glob.glob("evidence/ktls-*")):
    t = collections.defaultdict(collections.Counter)
    for l in open(d + "/results.jsonl"):
        r = json.loads(l); t[r["cell"]][r["verdict"]] += 1
    print("  ", d, {k: dict(v) for k, v in t.items()})
# latency of host observers, killing the owner (time from first kill to first event)
print("== host-clock observers: first-event latency after the kill (ms), RESET trials")
lat = collections.defaultdict(list)
for r in base:
    for k, o in r.get("observers", {}).items():
        if o and k.split(":")[1] != "guest_app" and o.get("rx_verdict") == "RESET" and "first_event_after_first_kill_ms" in o:
            lat[(r["cell"], k)].append(o["first_event_after_first_kill_ms"])
for k, v in sorted(lat.items()):
    v.sort(); print(f"  {k[0]:22s} {k[1]:30s} n={len(v):3d} p50={statistics.median(v):6.2f} max={max(v):6.2f}")
# survivor ordering: flow RDHUP -> ctl_closed gap on the host owner (guest owner killed) in matrix runs with timestamps
print("== survivor host owner after kill -9 of the guest owner: flow-vsock RDHUP -> ctl_closed gap (ms)")
for cell in ("out_nonmesh-killG", "out_mesh-killG", "in-killG"):
    gaps, first = [], collections.Counter()
    for r in base:
        if r["cell"] != cell: continue
        f = f"{r['_dir']}/trial-logs/{cell}-{r['trial']:03d}/host-owner.log"
        es = evs(f); hup = [e for e in es if e.get("ev") == "leg_hup" and e.get("t_ns")]; ctl = [e for e in es if e.get("ev") == "ctl_closed" and e.get("t_ns")]
        if hup and ctl: gaps.append((int(ctl[0]["t_ns"]) - int(hup[0]["t_ns"])) / 1e6); first["flow RDHUP first" if gaps[-1] > 0 else "ctl loss first"] += 1
        elif ctl and not hup: first["ctl loss only"] += 1
    if gaps:
        gaps.sort(); print(f"  {cell:20s} n={len(gaps)} gap p50={statistics.median(gaps):.2f} min={gaps[0]:.2f} max={gaps[-1]:.2f}  {dict(first)}")
print("== survivor guest owner after kill -9 of the host owner: flow-vsock RDHUP -> ctl_closed gap (ms)")
for cell in ("out_nonmesh-killH", "out_mesh-killH", "in-killH"):
    gaps, first = [], collections.Counter()
    for r in base:
        if r["cell"] != cell: continue
        f = f"{r['_dir']}/trial-logs/{cell}-{r['trial']:03d}/guest-owner.log"
        es = evs(f); hup = [e for e in es if e.get("ev") == "leg_hup" and e.get("t_ns")]; ctl = [e for e in es if e.get("ev") == "ctl_closed" and e.get("t_ns")]
        if hup and ctl: gaps.append((int(ctl[0]["t_ns"]) - int(hup[0]["t_ns"])) / 1e6); first["flow RDHUP first" if gaps[-1] > 0 else "ctl loss first"] += 1
        elif ctl and not hup: first["ctl loss only"] += 1
    if gaps:
        gaps.sort(); print(f"  {cell:20s} n={len(gaps)} gap p50={statistics.median(gaps)*1000:.0f}us min={gaps[0]*1000:.0f}us max={gaps[-1]*1000:.0f}us  {dict(first)}")
# guest-clock observers: the harness's per-trial offset (guest clock - host clock, +-~1-2 ms: bracketed agent round trip);
# the in-trial field first_event_after_first_kill_ms for guest_app has the offset applied with the wrong sign and is NOT used.
print("== guest_app first event, host-clock-corrected (ms after first kill), RESET trials; offset error +-2 ms")
g = collections.defaultdict(list)
for r in base:
    off = r.get("guest_clock_offset_ns")
    for k, o in r.get("observers", {}).items():
        if o and k.endswith("guest_app") and o.get("rx_verdict") == "RESET" and off is not None and o.get("first_event"):
            g[(r["cell"], k)].append((int(o["first_event"]["t_ns"]) - off - r["kills"][0]["t_ns"]) / 1e6)
for k, v in sorted(g.items()):
    v.sort(); print(f"  {k[0]:22s} {k[1]:24s} n={len(v):3d} p50={statistics.median(v):7.2f} min={v[0]:7.2f} max={v[-1]:7.2f}")

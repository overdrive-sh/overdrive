#!/usr/bin/env python3
"""Tables for the host-role V-3 run (bench-v3 events in native-serial.log). Usage: v3_table.py <evidence dir>"""
import json, glob, sys, statistics, re
def ev(path):
    for l in open(path, errors="replace"):
        i = l.find('{"')
        if i >= 0 and l.rstrip().endswith("}"):
            try: yield json.loads(l[i:])
            except Exception: pass
d = sys.argv[1]
p = glob.glob(d + "/*/native-serial.log")[0]
trials = [e for e in ev(p) if e.get("event") == "v3_trial"]
by = {}
for t in trials: by.setdefault(t["label"], []).append(t)
MiB = 1048576
def r(xs, f="{:.2f}"):
    xs = [x for x in xs if x is not None]
    return "-" if not xs else f"{f.format(min(xs))}/{f.format(statistics.median(xs))}/{f.format(max(xs))}"
print("| label | dir | trials | stream ok | peak pipeline MiB (a+b+c) | SUnreclaim rise max MiB | MemAvailable drop max MiB | longest sender block ms | accepted before first block MiB | detach ms |")
print("|---|---|---:|---:|---|---:|---:|---|---|---:|")
for lab, ts in by.items():
    ok = sum(1 for t in ts if t["ok"] and not t["timed_out"])
    blocks = []
    for t in ts:
        b = t.get("sender_blocks_ms") or []
        blocks.append(max([x[1] for x in b], default=0))
    acc = [(t["sent_at_first_block"] or 0) / MiB for t in ts if t.get("sent_at_first_block") is not None]
    print(f"| {lab} | {ts[0]['dir']} | {len(ts)} | {ok}/{len(ts)} | {r([t['peak_pipe']/MiB for t in ts])} | {max((t['sunrec_max_kb']-t['sunrec_start_kb'])/1024 for t in ts):.1f} | {max((t['avail_start_kb']-t['avail_min_kb'])/1024 for t in ts):.1f} | {r(blocks, '{:.0f}')} | {r(acc)} | {r([t['detach_ms'] for t in ts], '{:.0f}')} |")
print()
# resume latency (blocked sender unblocked after the reader woke) and flatness inside the stall window
wr = {}
for e in ev(p):
    if e.get("event") == "v3_writer": wr[(e["label"], e["trial"])] = e
print("| label | resume ms after the reader wakes (min/p50/max) | in-stall window: SUnreclaim range KiB (max over trials) | in-stall window: MemAvailable range MiB (max over trials) | sender throughput MiB/s (slow only) |")
print("|---|---|---|---|---|")
for lab, ts in by.items():
    res, sr, ar, thr = [], [], [], []
    for t in ts:
        ends = t.get("block_end_mono_ns") or (wr.get((lab, t["trial"])) or {}).get("block_end_mono_ns") or []
        st = (t.get("stall_ends") or (wr.get((lab, t["trial"])) or {}).get("stall_ends") or [])
        blocks = t.get("sender_blocks_ms") or []
        if ends and st and blocks:
            i = max(range(len(blocks)), key=lambda k: blocks[k][1])
            if i < len(ends): res.append((ends[i] - st[0][0]) / 1e6)
        ser = t["series"]
        stall_ms = int(re.match(r"s(\d+)", t["schedule"]).group(1)) if t["schedule"].startswith("s") else 0
        if stall_ms:
            w = [r for r in ser if 400 <= r[0] <= stall_ms - 400]
            if w:
                sr.append(max(r[6] for r in w) - min(r[6] for r in w)); ar.append((max(r[7] for r in w) - min(r[7] for r in w)) / 1024)
        if t["schedule"].startswith("r"): thr.append(t["total"] / MiB / t["secs"])
    print(f"| {lab} | {r(res)} | {max(sr) if sr else '-'} | {('%.2f' % max(ar)) if ar else '-'} | {r(thr, '{:.1f}')} |")
print()
for lab in by:
    t = by[lab][0]
    print(lab, "trial0 series cols:", t["series_cols"])
    for row in t["series"][:: max(1, len(t["series"]) // 14)]:
        print("   ", row)

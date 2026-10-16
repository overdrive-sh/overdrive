#!/usr/bin/env python3
"""Parse harness serial logs (JSON event lines) into the comparison tables. Usage: analyze.py <label>=<native-serial.log> ..."""
import json, sys, statistics, re

def events(path):
    for line in open(path, errors="replace"):
        i = line.find('{"')
        if i < 0 or not line.rstrip().endswith("}"):
            continue
        try:
            yield json.loads(line[i:])
        except Exception:
            pass

def cpu(s):
    v = [int(x) for x in s.splitlines()[0].split()[1:]]
    return v[0] + v[1] + v[2] + v[5] + v[6]  # user nice system irq softirq (ticks); idle/iowait/steal excluded

def mem(snap, key):
    m = re.search(rf"^{key}:\s+(\d+) kB", snap["meminfo"], re.M)
    return int(m.group(1)) if m else 0

def window_rows(path, label):
    rows = []
    for e in events(path):
        if e.get("event") != "window" or e["unlimited_closed_loop"] is False or e["repeat"] >= 99:
            continue
        dur = e["duration_s"]
        ticks = cpu(e["cpu_after"]) - cpu(e["cpu_before"])
        busy = ticks / 100.0 / dur
        gib = e["application_bytes_delivered"] / 2**30
        rows.append(dict(label=label, population=e["population"], active=e["active"], size=e["application_bytes_each_direction"], rep=e["repeat"],
                         gibps=gib / dur, busy=busy, core_s_per_gib=(ticks / 100.0) / gib if gib else None,
                         p50_ms=e["p50_ns"] / 1e6, p99_ms=e["p99_ns"] / 1e6, rt=e["delivered_roundtrips"]))
    return rows

def resources(path, label):
    out = []
    for e in events(path):
        if e.get("event") in ("loaded", "idle_resources", "traffic_resources", "setup") or "snapshot" in e:
            snap = e.get("snapshot") or e.get("after")
            if not snap or "meminfo" not in snap:
                continue
            out.append(dict(label=label, event=e["event"], population=e.get("population"), tasks=snap["all_tasks"], kworkers=snap["kernel_workers"],
                            sunreclaim_mib=mem(snap, "SUnreclaim") / 1024, slab_mib=mem(snap, "Slab") / 1024, kstack_mib=mem(snap, "KernelStack") / 1024,
                            memfree_mib=mem(snap, "MemFree") / 1024, self_tasks=snap["self_tasks"], fds=snap["self_fds"]))
    return out

if __name__ == "__main__":
    args = [a.split("=", 1) for a in sys.argv[1:]]
    mode = "windows"
    for label, path in args:
        print(f"### {label}")
        for r in window_rows(path, label):
            if r["size"] == 65536 or r["size"] == 64:
                print(f"pop={r['population']:6d} active={r['active']:6d} size={r['size']:6d} rep={r['rep']} {r['gibps']:.3f} GiB/s busy={r['busy']:.2f} cores {r['core_s_per_gib']:.2f} core-s/GiB p50={r['p50_ms']:.3f}ms p99={r['p99_ms']:.3f}ms n={r['rt']}")
        seen = set()
        for r in resources(path, label):
            k = (r['event'], r['population'])
            print(f"  res {r['event']:18s} pop={r['population']} tasks={r['tasks']} sunreclaim={r['sunreclaim_mib']:.1f}MiB slab={r['slab_mib']:.1f}MiB kstack={r['kstack_mib']:.1f}MiB kworkers={r['kworkers']}")

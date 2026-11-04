#!/usr/bin/env python3
"""Summarise wcdriver output (WCJSON lines). usage: analyze.py a|b|c|d <raw.log> [...]"""
import json, re, statistics, sys
from collections import Counter, defaultdict


def load(paths):
    rows, fails, env, sub = [], [], [], []
    for p in paths:
        for line in open(p, errors="replace"):
            if line.startswith("WCJSON "):
                rows.append(json.loads(line[7:]))
            elif line.startswith("WCFAIL"):
                fails.append(line.strip()[:200])
            elif line.startswith("WCENV"):
                env.append(line.strip())
            elif line.startswith("SUBSTRATE") or line.startswith("CH "):
                sub.append(line.strip())
    return rows, fails, env, sub


def kv(line):
    return dict(re.findall(r"(\w+)=(\S+)", line))


def parse(row):
    r = {"variant": row["variant"], "round": row["round"], "warm": row["warm"], "exit": row["exit"], "wc_end": row["wc_end"],
         "stream": row["stream_hex"], "ev": [], "wl": [], "marks": {}, "sum": {}, "corr": [], "origin": [], "child": []}
    for l in row["lines"]:
        if l.startswith("WC_EV"):
            r["ev"].append(kv(l))
        elif l.startswith("WL role"):
            r["wl"].append(kv(l))
        elif l.startswith("WC_T "):
            r["marks"] = {k: int(v) for k, v in kv(l).items() if v.isdigit()}
        elif l.startswith("WC_SUM"):
            r["sum"] = kv(l)
        elif l.startswith("WL_CORR"):
            r["corr"].append(l)
        elif l.startswith("WL_ORIGIN"):
            r["origin"].append(l)
        elif l.startswith("WL_CHILD"):
            r["child"].append(l)
    return r


def pct(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round((len(xs) - 1) * p)))]


def check_a(r):
    """Return list of violated expectations for a workload-cgroup boot (child placement)."""
    bad = []
    s = r["sum"]
    wl = {w["role"]: w for w in r["wl"]}
    wlmode = r["variant"].split("_", 1)[1] if "_" in r["variant"] else r["variant"]
    for k in ("init_tcp", "init_udp", "init_other"):
        if s.get(k) != "0":
            bad.append(f"{k}={s.get(k)}")
    if r["exit"] != 0 or not r["wc_end"]:
        bad.append(f"exit={r['exit']} wc_end={r['wc_end']}")
    exp = {  # (proto kind, port) -> role that must own it
        "single": [("tcp_listen", "18080", "main"), ("udp_bind", "18081", "main")],
        "childonly": [("tcp_listen", "18082", "child"), ("udp_bind", "18083", "child")],
        "parentchild": [("tcp_listen", "18080", "main"), ("udp_bind", "18081", "main"),
                        ("tcp_listen", "18082", "child"), ("udp_bind", "18083", "child")],
    }[wlmode]
    got = sorted((e["kind"], e["port"], e["pid"]) for e in r["ev"])
    want = sorted((k, p, wl[role]["pid"]) for k, p, role in exp if role in wl)
    if got != want:
        bad.append(f"events {got} != {want}")
    n_tcp = sum(1 for e in r["ev"] if e["kind"] == "tcp_listen")
    n_udp = sum(1 for e in r["ev"] if e["kind"] == "udp_bind")
    if s.get("tcp_seen") != str(n_tcp) or s.get("udp_seen") != str(n_udp) or s.get("tcp_drops") != "0" or s.get("udp_drops") != "0":
        bad.append(f"accounting {s.get('tcp_seen')}/{s.get('udp_seen')}/{s.get('tcp_drops')}/{s.get('udp_drops')} vs events {n_tcp}/{n_udp}")
    for role, w in wl.items():
        if w.get("cg") != "0::/workload":
            bad.append(f"{role} cg={w.get('cg')}")
    return bad


def mode_a(paths):
    rows, fails, env, sub = load(paths)
    rs = [parse(x) for x in rows if not x["warm"]]
    print("\n".join(sub[:2])); print("\n".join(env[:1]))
    by = defaultdict(list)
    for r in rs:
        by[r["variant"]].append(r)
    tot = ok = 0
    for v, lst in sorted(by.items()):
        good = [r for r in lst if not check_a(r)]
        print(f"{v}: {len(good)}/{len(lst)} boots exactly as predicted")
        tot += len(lst); ok += len(good)
        for r in lst:
            b = check_a(r)
            if b:
                print("  MISMATCH round", r["round"], b)
    ev = Counter()
    for r in rs:
        for e in r["ev"]:
            ev[(e["kind"], e["port"])] += 1
    print(f"TOTAL {ok}/{tot}; init-originated reports across all boots: " +
          str(sum(int(r["sum"].get("init_tcp", 0)) + int(r["sum"].get("init_udp", 0)) + int(r["sum"].get("init_other", 0)) for r in rs)))
    print("workload report counts by (kind,port):", dict(ev))
    print(f"WCFAIL lines: {len(fails)}"); [print(" ", f) for f in fails[:5]]


def mode_b(paths):
    rows, fails, env, sub = load(paths)
    rs = [parse(x) for x in rows if not x["warm"]]
    print("\n".join(sub[:2]))
    by = defaultdict(list)
    for r in rs:
        by[r["variant"]].append(r)
    for v, lst in sorted(by.items()):
        agg = Counter()
        for r in lst:
            for k in ("init_tcp", "init_udp", "init_other", "wl_tcp", "wl_udp", "wl_other"):
                agg[k] += int(r["sum"].get(k, 0))
        shapes = Counter((r["sum"].get("init_tcp"), r["sum"].get("init_udp"), r["sum"].get("wl_tcp"), r["sum"].get("wl_udp")) for r in lst)
        cgs = Counter(w.get("cg") for r in lst for w in r["wl"])
        rng = Counter(r["sum"].get("init_udp_range") for r in lst)
        print(f"{v}: n={len(lst)} totals={dict(agg)}")
        print(f"   per-boot (init_tcp,init_udp,wl_tcp,wl_udp) shapes: {dict(shapes)}")
        print(f"   workload cgroups: {dict(cgs)}  init udp port range(s): {dict(rng)}")
        print(f"   accounting tcp_seen/udp_seen/drops: {Counter((r['sum'].get('tcp_seen'), r['sum'].get('udp_seen'), r['sum'].get('tcp_drops'), r['sum'].get('udp_drops')) for r in lst)}")
    print(f"WCFAIL lines: {len(fails)}")


def mode_c(paths):
    rows, fails, env, sub = load(paths)
    rs = [parse(x) for x in rows if not x["warm"]]
    print("\n".join(sub[:2]))
    by = defaultdict(list)
    for r in rs:
        by[r["variant"]].append(r)
    for v, lst in sorted(by.items()):
        streams = Counter(tuple(r["stream"]) for r in lst)
        origins = Counter(tuple(r["origin"]) for r in lst)
        corr = Counter(re.sub(r"cookie=\S+", "cookie=*", c) for r in lst for c in r["corr"])
        cgs = Counter(w.get("cg") for r in lst for w in r["wl"])
        child = Counter(tuple(r["child"]) for r in lst)
        print(f"{v}: n={len(lst)} workload cgroup={dict(cgs)}")
        print(f"   guest-owner stream header seen by host: {dict(streams)}")
        print(f"   ORIGIN map entry for the workload socket: {dict(origins)}")
        print(f"   connect result: {dict(corr)}")
        print(f"   CHILD_CORRELATION entries seen by workload poll: {dict(child)}")
        print(f"   listen reports (wl_tcp,wl_udp,init_tcp,init_udp): {dict(Counter((r['sum'].get('wl_tcp'), r['sum'].get('wl_udp'), r['sum'].get('init_tcp'), r['sum'].get('init_udp')) for r in lst))}")
    print(f"WCFAIL lines: {len(fails)}")


def mode_d(paths):
    rows, fails, env, sub = load(paths)
    rs = [parse(x) for x in rows if not x["warm"] and x["variant"].startswith("d_")]
    print("\n".join(sub[:2])); print("\n".join(env[:1]))
    by = defaultdict(dict)
    for r in rs:
        m = r["marks"]
        by[r["variant"]][r["round"]] = (m["ready_sent"] - m["start"]) / 1e6, m
    base = by["d_base"]
    copyb = by.get("d_copybase")
    print(f"{'variant':16} {'n':>3} {'p50':>8} {'p90':>8} {'mean':>8} {'min':>8} {'max':>8} | delta vs d_base: {'p50(unpaired)':>13} {'paired p50':>10} {'paired p90':>10} {'paired max':>10}   (ms, init start -> READY sent)")
    for v in sorted(by):
        xs = [t for t, _ in by[v].values()]
        bs = [t for t, _ in base.values()]
        paired = [by[v][r][0] - base[r][0] for r in by[v] if r in base]
        print(f"{v:16} {len(xs):3} {pct(xs,.5):8.2f} {pct(xs,.9):8.2f} {statistics.mean(xs):8.2f} {min(xs):8.2f} {max(xs):8.2f} | {pct(xs,.5)-pct(bs,.5):13.2f} {pct(paired,.5):10.2f} {pct(paired,.9):10.2f} {max(paired):10.2f}")
    print("\nmedian stage durations (ms) from the in-guest marks:")
    stages = [("start", "cfg_begin"), ("cfg_begin", "programs_done"), ("programs_done", "wc_cgroup_made"), ("wc_cgroup_made", "wc_loaded"),
              ("wc_loaded", "wc_attached"), ("wc_attached", "wc_done"), ("programs_done", "wc_done"), ("wc_done", "ready_sent")]
    for v in sorted(by):
        out = []
        for a, b in stages:
            xs = [(m[b] - m[a]) / 1e6 for _, m in by[v].values() if a in m and b in m]
            if xs:
                out.append(f"{a}->{b} {statistics.median(xs):.2f}")
        print(f"  {v:16} " + "; ".join(out))
    print("\npaired per-round differences (ms, same round index = adjacent in the interleaved schedule):")
    for a, b, what in [("d_sameobj", "d_copybase", "THE DESIGN (listen programs in the guest object, attached to the workload cgroup, workload placed) vs the identical guest object without them"),
                       ("d_sameobjidle", "d_copybase", "carrying the listen programs and their 2 maps in the guest object, nothing attached"),
                       ("d_copybase", "d_base", "spike-built guest-object copy vs production-built guest object (object-build difference, not a cost of the design)"),
                       ("d_sameobj", "d_sameobjidle", "same object: programs attached to the workload cgroup vs merged object idle"),
                       ("d_child", "d_cgonly", "separate Ebpf::load + attach vs cgroup+placement only"),
                       ("d_cgonly", "d_base", "workload cgroup mkdir + clone3 CLONE_INTO_CGROUP placement vs baseline"),
                       ("d_child", "d_outside", "placement via clone3 vs not placed (both load+attach)"),
                       ("d_childnobtf", "d_child", "loader.btf(None) vs default loader")]:
        if a in by and b in by:
            diffs = [by[a][r][0] - by[b][r][0] for r in by[a] if r in by[b]]
            print(f"  {a} - {b}: p50 {pct(diffs,.5):.2f}  p90 {pct(diffs,.9):.2f}  max {max(diffs):.2f}  mean {statistics.mean(diffs):.2f}   [{what}]")
    print(f"WCFAIL lines: {len(fails)}")


if __name__ == "__main__":
    {"a": mode_a, "b": mode_b, "c": mode_c, "d": mode_d}[sys.argv[1]](sys.argv[2:])

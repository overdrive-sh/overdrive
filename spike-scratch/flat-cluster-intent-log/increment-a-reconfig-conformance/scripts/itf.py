#!/usr/bin/env python3
"""SPIKE helper: decode a Quint ITF trace of spec/vr_sc.qnt into a compact per-step table.

usage: scripts/itf.py <trace.itf.json> [--msgs]
"""
import json
import sys


def dec(x):
    if isinstance(x, dict):
        if "#bigint" in x:
            return int(x["#bigint"])
        if "#set" in x:
            return [dec(y) for y in x["#set"]]
        if "#tup" in x:
            return tuple(dec(y) for y in x["#tup"])
        if "#map" in x:
            return {dec(k): dec(v) for k, v in x["#map"]}
        return {k: dec(v) for k, v in x.items()}
    if isinstance(x, list):
        return [dec(y) for y in x]
    return x


def short(k):
    return k.split("::")[-1]


def entry(e):
    if e["kind"] == "client":
        return f"c{e['label']}"
    c = e["cfg"]
    return f"R(e{c['epoch']}:V{c['voters']}L{c['learners']}{'!' if e['reduces'] else ''})"


def node(s):
    c = s["cfg"]
    flags = ("" if s["up"] else "DOWN ") + ("" if s["started"] else "unstarted ")
    return (f"{flags}{s['status']} e{c['epoch']} V{c['voters']} L{c['learners']} "
            f"v{s['view']} lv{s['logView']} commit={s['commit']} "
            f"log=[{' '.join(entry(e) for e in s['log'])}]")


def main():
    path = sys.argv[1]
    show_msgs = "--msgs" in sys.argv
    d = json.load(open(path))
    for i, raw in enumerate(d["states"]):
        s = {short(k): dec(v) for k, v in raw.items() if k != "#meta"}
        act = s.get("actionTaken", "")
        picks = s.get("nondetPicks", {})
        picks = {k: v.get("value") if isinstance(v, dict) else v for k, v in picks.items()}
        picks = {k: (v[0] if isinstance(v, list) and len(v) == 1 else v)
                 for k, v in picks.items() if v not in (None, [], {})}
        print(f"--- state {i}  action={act} {picks if picks else ''}")
        for n, ns in sorted(s["st"].items()):
            print(f"   n{n}: {node(ns)}")
        flags = {k: s[k] for k in ("ghostOk", "promoteOk", "adoptOk", "commitStarOk") if k in s}
        print(f"   ghost=[{' '.join(entry(e) for e in s['ghostLog'])}] committers={sorted(s['committers'])} {flags}")
        if show_msgs:
            for m in s["msgs"]:
                print(f"     msg {m['kind']} {m['src']}->{m['dst']} v{m['view']} e{m['epoch']} "
                      f"op{m['op']} c{m['commit']} lv{m['logView']} voter={m['srcVoter']}")


if __name__ == "__main__" and "--check" not in sys.argv:
    main()


def check(path, max_view):
    """Re-evaluate the spec invariants on every state of an ITF trace; print the first failing ones."""
    d = json.load(open(path))
    for i, raw in enumerate(d["states"]):
        s = {short(k): dec(v) for k, v in raw.items() if k != "#meta"}
        st = s["st"]
        g = s["ghostLog"]
        gat = s.get("ghostAt", [])

        def committed(ns):
            return ns["log"][: min(ns["commit"], len(ns["log"]))]

        def is_prefix(a, b):
            return len(a) <= len(b) and all(a[k] == b[k] for k in range(len(a)))

        def primary_self(ns):
            c = ns["cfg"]
            return ns["id"] in c["voters"] and c["voters"][ns["view"] % len(c["voters"])] == ns["id"]

        fails = []
        if not (s["ghostOk"] and all(is_prefix(committed(ns), g) for ns in st.values())):
            fails.append("agreement")
        for n, ns in st.items():
            if ns["started"] and ns["up"] and ns["status"] == "normal" and primary_self(ns) and ns["logView"] == ns["view"]:
                ev = ns["cfg"]["epoch"] * (max_view + 1) + ns["view"]
                for k in range(len(g)):
                    if gat[k] < ev and not (k < len(ns["log"]) and ns["log"][k] == g[k]):
                        fails.append(f"completeness(n{n} ev={ev} lacks ghost[{k}]={entry(g[k])} committed@ev={gat[k]})")
                        break
        cm = s["committers"]
        for a in cm:
            for b in cm:
                if a[0] == b[0] and a[1] == b[1] and a[2] != b[2]:
                    fails.append(f"singlePrimary{a}{b}")
        if not s["promoteOk"]:
            fails.append("promotionCaughtUp")
        if any(m["kind"] in ("prepareOk", "svc", "dvc") and not m["srcVoter"] for m in s["msgs"]):
            fails.append("learnerNoVote")
        if not (s["adoptOk"] and s["commitStarOk"]):
            fails.append(f"noFailStop(adoptOk={s['adoptOk']} commitStarOk={s['commitStarOk']})")
        if fails:
            print(f"first violation at state {i}: {fails}")
            return
    print("no violation in trace")


if __name__ == "__main__" and "--check" in sys.argv:
    check(sys.argv[1], int(sys.argv[sys.argv.index("--check") + 1]))

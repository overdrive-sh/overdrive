#!/usr/bin/env python3
"""Condense a TLC counterexample (from evidence/tlc-*.txt) into per-step action + changed variables.
Added on resume; reads raw evidence only, writes nothing but stdout."""
import re, sys

def states(text):
    body = text.split("Error: The behavior up to this point is:", 1)
    if len(body) < 2:
        return []
    out, cur, var = [], None, None
    for line in body[1].splitlines():
        m = re.match(r"State (\d+): <(.*?)(?: line .*)?>", line)
        if m:
            cur = {"n": int(m.group(1)), "action": m.group(2).split("_gate_seal_")[-1], "vars": {}}
            out.append(cur); var = None; continue
        if cur is None:
            continue
        m = re.match(r"/\\ \w+?_gate_seal_(\w+) = (.*)", line)
        if m:
            var = m.group(1); cur["vars"][var] = m.group(2).strip(); continue
        if var and line.startswith(" ") and line.strip():
            cur["vars"][var] += " " + line.strip()
        elif not line.strip() or line.startswith(("Error", "State", "1", "2", "3", "4", "5", "6", "7", "8", "9")):
            var = None
    return out

for path in sys.argv[1:]:
    text = open(path).read()
    st = states(text)
    print(f"### {path.split('/')[-1]}  ({len(st)} states; BFS => shortest counterexample)")
    prev = {}
    for s in st:
        changed = {k: v for k, v in s["vars"].items() if prev.get(k) != v}
        if s["n"] == 1:
            print("  State 1 <init>")
        else:
            kv = "; ".join(f"{k}={re.sub(r'\\s+', ' ', v)}" for k, v in sorted(changed.items()))
            print(f"  State {s['n']} <{s['action']}>: {kv}")
        prev = s["vars"]
    print()

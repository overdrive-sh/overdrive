#!/usr/bin/env python3
"""SPIKE helper: what did a set of replayed ITF traces actually exercise?

usage: scripts/coverage.py <dir-with-run_*.itf.json> [more dirs...]

Reports, per directory: traces, steps, the action histogram, minted reconfiguration kinds (from the
Propose action args), the highest epoch reached, and how many traces reached each churn milestone.
"""
import collections
import glob
import importlib.util
import json
import sys

spec = importlib.util.spec_from_file_location("itf", __file__.replace("coverage.py", "itf.py"))
itf = importlib.util.module_from_spec(spec)
spec.loader.exec_module(itf)


def states(path):
    d = json.load(open(path))
    for raw in d["states"]:
        yield {itf.short(k): itf.dec(v) for k, v in raw.items() if k != "#meta"}


def report(dirname):
    files = sorted(glob.glob(f"{dirname}/run_*.itf.json"))
    acts = collections.Counter()
    minted = collections.Counter()
    milestones = collections.Counter()
    steps = 0
    max_epoch = 0
    for f in files:
        seen = set()
        genesis_voters = None
        for s in states(f):
            la = s["lastAction"]
            tag = la["tag"]
            acts[tag] += 1
            if tag != "Init":
                steps += 1
            if tag == "Propose":
                minted[la["value"]["kind"]] += 1
                seen.add(f"minted:{la['value']['kind']}")
            nodes = s["st"]
            if genesis_voters is None:
                genesis_voters = max(len(ns["cfg"]["voters"]) for ns in nodes.values())
            for ns in nodes.values():
                if not ns["started"]:
                    continue
                e = ns["cfg"]["epoch"]
                max_epoch = max(max_epoch, e)
                if e >= 1:
                    seen.add("epoch>=1")
                if e >= 2:
                    seen.add("epoch>=2")
                if e >= 3:
                    seen.add("epoch>=3")
                if len(ns["cfg"]["voters"]) > genesis_voters:
                    seen.add("grew")
                if len(ns["cfg"]["voters"]) < genesis_voters:
                    seen.add("shrank")
                if ns["status"] == "retired":
                    seen.add("retired")
                if ns["status"] == "normal" and ns["logView"] >= 1:
                    seen.add("viewChanged")
                if ns["status"] == "rhead":
                    seen.add("recoveringHead")
            if tag == "SyncFrom":
                seen.add("crossed(syncFrom)")
            if tag == "Bootstrap":
                seen.add("bootstrapped")
            if tag == "Crash":
                seen.add("crash")
            if len(s["ghostLog"]) >= 2:
                seen.add("committed>=2")
        for m in seen:
            milestones[m] += 1
    print(f"== {dirname}: traces={len(files)} steps={steps} max_epoch={max_epoch}")
    print("   actions: " + ", ".join(f"{k}={v}" for k, v in sorted(acts.items())))
    print("   minted reconfigs: " + (", ".join(f"{k}={v}" for k, v in sorted(minted.items())) or "none"))
    print("   traces reaching: " + ", ".join(f"{k}={v}" for k, v in sorted(milestones.items())))


for d in sys.argv[1:]:
    report(d)

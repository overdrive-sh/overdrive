#!/usr/bin/env python3
"""Tabulate evidence/<run>/results.json (the V-3 probe output). Usage: analyze.py evidence/full-<ts>/results.json"""
import json
import statistics as st
import sys

d = json.load(open(sys.argv[1]))


def fmt(xs):
    return f"{st.mean(xs):.3f} sd {st.pstdev(xs):.3f} [{min(xs):.3f}..{max(xs):.3f}]"


for r in d:
    print("=== N", r["n"], "targets", r["targets_total"], r["targets_per_reconciler"],
          "enum ms", round(r["enumeration_proxy_ms"]["mean"], 3))
    for k, v in r["per_evaluation_serial_uncontended"].items():
        h, t = v["hydrate_desired_plus_actual_ms"], v["full_tick_ms"]
        print(f"  {k:22s} targets {v['targets']:5d} hydrate mean {h['mean']:.3f} sd {h['sd']:.3f} p95 {h['p95']:.3f} "
              f"| tick mean {t['mean']:.3f} sd {t['sd']:.3f} p95 {t['p95']:.3f}")
    ie = r["idle_edge_wake"]
    print("  idle: admission job", round(ie["job_admission_delay_ms"]["mean"], 2), "svc",
          round(ie["service_admission_delay_ms"]["mean"], 2), "| handler job",
          round(ie["job_deploy_handler_ms"]["mean"], 2), "svc", round(ie["service_deploy_handler_ms"]["mean"], 2),
          "| lock wait us p50/max", ie["allocator_lock_wait_us_idle"]["p50"], ie["allocator_lock_wait_us_idle"]["max"])
    ps = r["pass_samples"]
    col = lambda f: [f(p) for p in ps]
    print("  evals", ps[0]["evals_submitted"], "..", ps[-1]["evals_submitted"])
    print("  pass wall ms", fmt(col(lambda p: p["pass_wall_ms"])))
    print("  cpu s", fmt(col(lambda p: p["cpu_s_during_window"])))
    print("  duty wall %", fmt(col(lambda p: p["duty_cycle_wall_pct_of_30s"])),
          "cpu %", fmt(col(lambda p: p["duty_cycle_cpu_pct_of_30s_one_core"])))
    print("  sum tick elapsed ms", fmt(col(lambda p: p["sum_tick_elapsed_ms"])))
    print("  drain_pending total ms", fmt(col(lambda p: p["drain_pending_total_ms"])),
          "max call ms", fmt(col(lambda p: p["drain_pending_max_ms"])))
    for kind in ("job", "service"):
        e = [p["edge"] for p in ps if p["edge"]["kind"] == kind]
        if e:
            print("  edge", kind, "admission ms", [round(x["admission_delay_ms"], 1) for x in e],
                  "handler ms", [round(x["deploy_handler_wall_ms"], 1) for x in e])
    for q in ("p50", "p95", "max"):
        print("  lock wait in pass us", q, fmt(col(lambda p: p["allocator_lock_wait_us_in_pass"][q])))

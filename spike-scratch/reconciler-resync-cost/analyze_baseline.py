#!/usr/bin/env python3
"""Side-by-side: baseline (router relist + resync_schedule) vs proposed pass.
Usage: analyze_baseline.py <new results.json> [<prior results.json>]
Columns: baseline (new run), proposed same-run control (new run), proposed prior run (full-20261115T180650Z)."""
import json, statistics as st, sys

new = json.load(open(sys.argv[1]))
prior = {r["n"]: r for r in json.load(open(sys.argv[2]))} if len(sys.argv) > 2 else {}

def m(xs):
    xs = [x for x in xs if x is not None]
    return (st.mean(xs), min(xs), max(xs)) if xs else (float("nan"),) * 3

def f(t, d=1):
    return f"{t[0]:.{d}f} [{t[1]:.{d}f}..{t[2]:.{d}f}]"

def col(samples, g):
    return m([g(s) for s in samples])

def edge(samples, kind, key):
    return m([s["edge"][key] for s in samples if s["edge"]["kind"] == kind])

rows = []
for r in new:
    n = r["n"]
    ps = [p for p in r["pass_samples"] if not p.get("warmup")]
    by = {"baseline": [p for p in ps if p["mode"] == "baseline"], "proposed": [p for p in ps if p["mode"] == "proposed"]}
    pr = [dict(p, mode="prior") for p in prior[n]["pass_samples"]] if n in prior else []
    print(f"=== N={n}  warmup baseline: " + str([ (p['evals_submitted'], round(p['pass_wall_ms'])) for p in r['pass_samples'] if p.get('warmup')]))
    for label, sm in (("baseline", by["baseline"]), ("proposed(same run)", by["proposed"]), ("proposed(prior run)", pr)):
        if not sm:
            continue
        print(f"  {label:20s} n={len(sm)}")
        print("    E            ", f(col(sm, lambda p: p["evals_submitted"]), 0), " by reconciler:", sm[0].get("evals_by_reconciler"))
        print("    never-completed keys", [p.get("keys_never_completed") for p in sm], " extra evals", [p["extra_evals_beyond_pass"] for p in sm])
        print("    wall ms      ", f(col(sm, lambda p: p["pass_wall_ms"]), 0))
        print("    cpu s        ", f(col(sm, lambda p: p["cpu_s_during_window"]), 2))
        print("    duty wall %  ", f(col(sm, lambda p: p["duty_cycle_wall_pct_of_30s"]), 2), "cpu %", f(col(sm, lambda p: p["duty_cycle_cpu_pct_of_30s_one_core"]), 2))
        print("    list->first submit ms", f(col(sm, lambda p: p.get("list_to_first_submit_ms", 0.0)), 2))
        print("    edge admission ms job", f(edge(sm, "job", "admission_delay_ms"), 0), " svc", f(edge(sm, "service", "admission_delay_ms"), 0))
        print("    svc handler ms", f(edge(sm, "service", "deploy_handler_wall_ms"), 1), " job handler ms", f(edge(sm, "job", "deploy_handler_wall_ms"), 1))
        print("    lock wait us p50", f(col(sm, lambda p: p["allocator_lock_wait_us_in_pass"]["p50"]), 0), " max", f(col(sm, lambda p: p["allocator_lock_wait_us_in_pass"]["max"]), 0))
        print("    sum tick ms  ", f(col(sm, lambda p: p["sum_tick_elapsed_ms"]), 0), " drain_pending total ms", f(col(sm, lambda p: p["drain_pending_total_ms"]), 0))
        pb = sm[0]["tick_elapsed_ms_by_reconciler"]
        print("    tick mean ms by reconciler (first sample)", {k: round(v["mean"], 2) for k, v in pb.items()})
    # markdown row
    def cell(sm):
        if not sm: return ["-"] * 8
        return [f"{st.mean([p['evals_submitted'] for p in sm]):.0f}",
                f"{st.mean([p['pass_wall_ms'] for p in sm]):.0f}",
                f"{st.mean([p['cpu_s_during_window'] for p in sm]):.2f}",
                f"{st.mean([p['duty_cycle_wall_pct_of_30s'] for p in sm]):.2f}",
                f"{edge(sm,'job','admission_delay_ms')[0]:.0f}",
                f"{edge(sm,'service','admission_delay_ms')[0]:.0f}",
                f"{edge(sm,'service','deploy_handler_wall_ms')[0]:.1f}",
                f"{st.mean([p['allocator_lock_wait_us_in_pass']['p50'] for p in sm])/1000:.1f}"]
    for label, sm in (("baseline", by["baseline"]), ("proposed same-run", by["proposed"]), ("proposed prior", pr)):
        rows.append(f"| {n} | {label} | " + " | ".join(cell(sm)) + " |")
print()
print("| N | pass | E | wall ms | CPU s | duty % (wall/30s) | edge admit ms (Job) | edge admit ms (Svc) | Svc handler ms | lock wait p50 ms |")
print("|---|---|---|---|---|---|---|---|---|---|")
print("\n".join(rows))

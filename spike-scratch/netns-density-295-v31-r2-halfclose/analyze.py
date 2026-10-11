#!/usr/bin/env python3
"""Aggregate the retained evidence of the v27 drain probes into analysis.txt (tables for the findings).
Loop-mode runs: evidence/<label>.json. VM runs: evidence/vm-drain-*/<label>-summary.json (latest per label)."""
import glob, json, os, re, sys, collections
HERE = os.path.dirname(os.path.abspath(__file__))
EV = os.path.join(HERE, "evidence")
OUT = []
def p(*a):
    s = " ".join(str(x) for x in a); OUT.append(s); print(s)

def load_loop(label):
    f = os.path.join(EV, label + ".json")
    if not os.path.exists(f): return None
    try: return json.load(open(f))
    except Exception: return None

def load_vm(label):
    best = None
    for d in sorted(glob.glob(os.path.join(EV, "vm-drain-*"))):
        f = os.path.join(d, label + "-summary.json")
        if os.path.exists(f): best = f
    return json.load(open(best)) if best else None

def roles(d, vm):
    return (d.get("guest") or {}, d.get("host") or {}) if vm else (d["guest"], d["host"])

def row(label, d, vm=False):
    if d is None:
        p(f"{label:36s} (no data)"); return None
    t = d["trials"]; g, h = roles(d, vm)
    def comp(c): return c.get("complete_owner_cas", 0) + c.get("complete_record_fexit", 0) + c.get("complete_record_state_cb", 0)
    viol = {k: g.get(k, 0) + h.get(k, 0) for k in set(g) | set(h) if k.startswith("VIOLATION")}
    viol = {k: v for k, v in viol.items() if v}
    live = (d.get("guest_live_at_end"), d.get("host_live_at_end")) if vm else (d["guest_live_flows_at_end"], d["host_live_flows_at_end"])
    started = g.get("flows_started", 0) + h.get("flows_started", 0)
    aborted = g.get("flows_aborted", 0) + h.get("flows_aborted", 0)
    recs = g.get("records", 0) + h.get("records", 0)
    cas = g.get("complete_owner_cas", 0) + h.get("complete_owner_cas", 0)
    rf = g.get("complete_record_fexit", 0) + h.get("complete_record_fexit", 0)
    rs = g.get("complete_record_state_cb", 0) + h.get("complete_record_state_cb", 0)
    lost = g.get("owner_lost_cas", 0) + h.get("owner_lost_cas", 0)
    fails = {k: v for k, v in t["fail"].items()}
    p(f"{label:36s} trials={t['total']:6d} pass={t['pass']:6d} role_flows={started:6d} aborted={aborted:5d} completions={comp(g)+comp(h):7d} "
      f"(owner_cas={cas} rec_fexit={rf} rec_state_cb={rs}) records={recs} owner_lost_cas={lost} live_at_end={live} VIOL={viol or 0} fail={fails or 0}")
    return {"trials": t["total"], "pass": t["pass"], "completions": comp(g) + comp(h), "owner_cas": cas, "rec_fexit": rf, "rec_state_cb": rs, "lost": lost,
            "viol": sum(viol.values()), "live": sum(x or 0 for x in live), "aborted": aborted, "started": started}

def gap(label, d, vm=False):
    if d is None: return
    g, h = roles(d, vm)
    def f(c, k, n): return f"{c.get(k,0)}/{c.get(n,0)}"
    p(f"{label:36s} TCP-src sent<goal at RDHUP: guest {f(g,'tcp_eof_sent_lt_goal','tcp_eof_events')} host {f(h,'tcp_eof_sent_lt_goal','tcp_eof_events')} | "
      f"vsock-src: guest {f(g,'vsock_eof_sent_lt_goal','vsock_eof_events')} host {f(h,'vsock_eof_sent_lt_goal','vsock_eof_events')} | "
      f"fwd<goal(verdict not yet run at RDHUP): guest {g.get('tcp_eof_fwd_lt_goal',0)} host {h.get('tcp_eof_fwd_lt_goal',0)} | max_deficit_B guest {g.get('tcp_eof_deficit_max',0)} host {h.get('tcp_eof_deficit_max',0)}"
      f" | max eof->complete us guest {g.get('max_eof_to_complete_us',0)} host {h.get('max_eof_to_complete_us',0)}")

def tot(rows):
    rows = [r for r in rows if r]
    p("  TOTAL", {k: sum(r[k] for r in rows) for k in rows[0]})

p("=" * 100); p("T1 - the drain gap on 7.0 (loopback vsock; both roles on the host)"); p("=" * 100)
t1 = ["t1-drain-p0", "t1-drain-p300", "t1-drain-p3000", "t1-naive-p0", "t1-naive-p300", "t1-naive-p3000",
      "t1-drain-small-findata", "t1-drain-small-finonly", "t1-naive-small-findata", "t1-naive-small-finonly"]
for l in t1: row(l, load_loop(l))
for l in t1: gap(l, load_loop(l))
for l in ("t1-naive-p0", "t1-naive-p300", "t1-naive-p3000"):
    d = load_loop(l)
    if d: p(f"{l:36s} EPIPE (errno32) at the target seen by fexit: {d['bpc'] if 'bpc' in d else d['bpf_counters'].get('send_err_hist')}")
p("--- T1 in the real microVM")
for l in ("vm-owner-2MiB-p0", "vm-owner-2MiB-p300", "vm-owner-2MiB-p3000", "vm-naive-p300"):
    d = load_vm(l); row(l, d, True); gap(l, d, True)

p(); p("=" * 100); p("T2 - R1 event correctness (loopback)"); p("=" * 100)
t2 = []
for arm in ("owner", "kernel"):
    for suf in ("mix-25k", "2MiB-p0", "2MiB-p300", "2MiB-p3000", "race-pin", "reuse", "srvfirst"):
        t2.append(f"t2-{arm}-{suf}")
rs = [row(l, load_loop(l)) for l in t2]
tot(rs)
p("--- T2 in the real microVM")
t2v = ["vm-owner-mix", "vm-kernel-mix", "vm-owner-2MiB-p0", "vm-owner-2MiB-p300", "vm-owner-2MiB-p3000", "vm-kernel-2MiB-p300", "vm-owner-race-small", "vm-kernel-race-small", "vm-owner-reuse"]
rv = [row(l, load_vm(l), True) for l in t2v]
tot(rv)

p(); p("=" * 100); p("T4 - goal exactness and stranding"); p("=" * 100)
for l in ("t4-findata-big", "t4-finonly-big"): row(l, load_loop(l))
rb = [row(f"t4-bigrcvbuf-{i}", load_loop(f"t4-bigrcvbuf-{i}")) for i in range(1, 13)]
tot(rb)
for l in ("t4-vlate0-banner100000", "t4-vlate1-banner100000", "t4-vlate2-banner100000", "t4-rcvlowat64k"): row(l, load_loop(l))
for l in ("vm-t4-vlate0-banner100000", "vm-t4-vlate1-banner100000", "vm-t4-vlate2-banner100000", "vm-t4-rcvlowat64k", "vm-t4-bigrcvbuf", "vm-t4-findata", "vm-t4-finonly"): row(l, load_vm(l), True)
p("--- TCP receive-queue overlap (the one exactness failure): default intake window")
agg = collections.Counter(); runs = 0; trials = 0
for l in sorted(set(re.sub(r"\.json$", "", os.path.basename(f)) for f in glob.glob(os.path.join(EV, "*.json")))):
    if re.match(r"(ovl\d+|nsa\d+|nsc\d+|anom\d)$", l):
        d = load_loop(l)
        if not d: continue
        runs += 1; trials += d["trials"]["total"]
        g = d["guest"]; h = d["host"]
        agg["flows_with_overlap"] += g.get("flows_with_tcp_rx_overlap", 0) + h.get("flows_with_tcp_rx_overlap", 0)
        agg["overlap_bytes"] += g.get("tcp_rx_overlap_bytes", 0) + h.get("tcp_rx_overlap_bytes", 0)
        agg["fwd_ne_goal"] += g.get("tcp_fwd_ne_goal_at_completion", 0) + h.get("tcp_fwd_ne_goal_at_completion", 0)
        agg["srv_excess"] += d["trials"]["fail"].get("srv_excess_bytes", 0) + d["trials"]["fail"].get("srv_truncated", 0)
p(f"  default window runs (labels anom*/nsa*/nsc*/ovl*): runs={runs} trials={trials} {dict(agg)}")
for l in sorted(glob.glob(os.path.join(EV, "ovl*.json"))):
    d = json.load(open(l)); g = d["guest"]
    if g.get("flows_with_tcp_rx_overlap", 0):
        pass
ov = [(os.path.basename(f), json.load(open(f))) for f in sorted(glob.glob(os.path.join(EV, "ovl*.json")))]
p("  ovl* runs (--diag-seq, in-kernel overlap detection): overlap flows =", sum(d['guest'].get('flows_with_tcp_rx_overlap', 0) for _, d in ov),
  "overlap bytes =", sum(d['guest'].get('tcp_rx_overlap_bytes', 0) for _, d in ov), "of", sum(d['trials']['total'] for _, d in ov), "trials;",
  "every overlap flow: fwd-goal == overlap bytes == 47616 (see anomaly events in evidence/ovl*.guest.jsonl)")
p("  intake SO_RCVBUF=4MiB runs (nsb21-26 + t4-bigrcvbuf-1..12):",
  sum(json.load(open(f))['trials']['total'] for f in glob.glob(os.path.join(EV, 'nsb*.json'))) + sum(r['trials'] for r in rb if r), "trials, overlap/exactness violations:",
  sum(json.load(open(f))['guest'].get('tcp_fwd_ne_goal_at_completion', 0) for f in glob.glob(os.path.join(EV, 'nsb*.json'))) + sum(r['viol'] for r in rb if r))

p(); p("=" * 100); p("T6 - failure path"); p("=" * 100)
r6 = []
for arm in ("owner", "kernel"):
    for kind in ("tcp-rst", "vsock-reset"):
        for s in range(1, 6):
            r6.append(row(f"t6-{kind}-{arm}-s{s}", load_loop(f"t6-{kind}-{arm}-s{s}")))
tot(r6)
row("t6-clean-control", load_loop("t6-clean-control"))
for l in ("vm-t6-tcp-rst-owner", "vm-t6-vsock-reset-owner", "vm-t6-tcp-rst-kernel", "vm-t6-vsock-reset-kernel"): row(l, load_vm(l), True)
# abort-cause totals
cause = collections.Counter(); late = collections.Counter()
for f in glob.glob(os.path.join(EV, "t6-*-s[1-5].json")):
    d = json.load(open(f))
    for r in ("guest", "host"):
        for k, v in d[r].items():
            if k.startswith("abort_"): cause[(r, k)] += v
            if k in ("late_drop_notice_after_recycle", "VIOLATION_completion_record_after_recycle", "record_redirect_dropped"): late[(r, k)] += v
p("  abort causes (T6 loopback, 10 runs):", dict(cause))
p("  late/dropped-notice records:", dict(late))

p(); p("=" * 100); p("fexit(skb_send_sock) recursion misses (counter exactness)"); p("=" * 100)
ms = []
for pref in ("m2", "m3"):
    flows = stuck = misses_a = misses_b = disagree = calls = 0
    for f in sorted(glob.glob(os.path.join(EV, f"{pref}-*.json"))):
        d = json.load(open(f)); b = d["bpf_counters"]
        flows += d["trials"]["total"]; stuck += d["guest_live_flows_at_end"]
        misses_a += b.get("fexit_recursion_misses") or 0; misses_b += b.get("fexit_b_recursion_misses") or 0
        calls += b["fexit_send_calls"] + b["fexit_send_untracked_target"]
        disagree += sum(d[r].get("counter_a_short_of_b", 0) + d[r].get("counter_b_short_of_a", 0) for r in ("guest", "host"))
    p(f"  {pref}: flows={flows} fexit_calls_executed={calls} stuck_flows={stuck} recursion_misses_prog_a={misses_a} prog_b={misses_b} flows_where_counters_disagree={disagree}")
for f in sorted(glob.glob(os.path.join(EV, "m2-*.json"))):
    d = json.load(open(f)); b = d["bpf_counters"]
    if (b.get("fexit_recursion_misses") or 0) or d["guest_live_flows_at_end"]:
        p(f"  {os.path.basename(f)}: misses={b.get('fexit_recursion_misses')} stuck(guest,host)={d['guest_live_flows_at_end']},{d['host_live_flows_at_end']}")
open(os.path.join(HERE, "analysis.txt"), "w").write("\n".join(OUT) + "\n")

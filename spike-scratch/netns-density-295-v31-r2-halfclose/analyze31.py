#!/usr/bin/env python3
"""Aggregate the retained evidence of the v31 probes into analysis.txt. Loop-mode runs: evidence/<label>.json.
Split-process runs: evidence/vm-<env>-<label>-*/<scenario>-<rep>-result.json (latest per env+label)."""
import glob, json, os, re, sys, collections
HERE = os.path.dirname(os.path.abspath(__file__)); EV = os.path.join(HERE, "evidence")
OUT = []
def p(*a):
    s = " ".join(str(x) for x in a); OUT.append(s); print(s)
def load(label):
    f = os.path.join(EV, label + ".json")
    try: return json.load(open(f))
    except Exception: return None
def tot(d, k): return d["guest"].get(k, 0) + d["host"].get(k, 0)
def viol(d): return {k: tot(d, k) for k in set(d["guest"]) | set(d["host"]) if k.startswith("VIOLATION") and tot(d, k)}

def loop_row(label, d):
    t = d["trials"]; b = d["bpf_counters"]
    comp = tot(d, "complete_owner_cas") + tot(d, "complete_record_fexit") + tot(d, "complete_record_poke")
    return (f"{label:34s} trials={t['total']:6d} pass={t['pass']:6d} reset={t.get('reset',0):6d} fail={t['fail']} | started={tot(d,'flows_started')} clean={tot(d,'flows_clean')} "
            f"aborted={tot(d,'flows_aborted')} completions={comp} (owner_cas={tot(d,'complete_owner_cas')} rec_fexit={tot(d,'complete_record_fexit')} rec_poke={tot(d,'complete_record_poke')}) "
            f"stale_epoch_discards={tot(d,'epoch_discarded_stale_completion')} | viol={viol(d)} | live_end={d['guest_live_flows_at_end']}/{d['host_live_flows_at_end']}")

p("## A. cumulative counters vs per-call counters, level backstop ablation (injected recursion misses; loopback; 3000 trials of 256 KiB unless noted)")
for lab in ["a1-incr-skip30-off", "a2-cum-skip30-off", "a5b-incr-skipA70-off", "a5-cum-skipA70-off", "a3-cum-skip30-rise", "a8-cum-skip30-rise-c1", "a4-cum-skip30-full", "a6-cum-skip70-full", "a7-cum-skip90-full", "a9-cum-skip90-full-big", "a10-cum-noinj-off", "a11-cum-noinj-full"]:
    d = load(lab)
    if not d: p(lab, "(no data)"); continue
    a = d["args"]; b = d["bpf_counters"]
    p(loop_row(lab, d))
    p(f"{'':34s}   args: incr={a.get('incr')} skip_a={a.get('skip-a')} skip_b={a.get('skip-b')} backstop={a.get('backstop')} conc={a.get('conc')} n={a.get('n')} | injected_misses={b.get('injected_misses')} poke_runs={b.get('poke_runs')} fire_poke={b.get('fire_poke')} fire_fexit={b.get('fire_fexit')} "
      f"arm_pokes={tot(d,'arm_pokes')} backstop_pokes={tot(d,'backstop_pokes')} rises={tot(d,'backstop_miss_rises')} owner_waits={tot(d,'owner_waits_for_record')}")

p("\n## B. clean half-close volume, loopback (owners in one process, flow vsocks over vsock_loopback, no control-session half-close)")
tp = tc = 0; agg = collections.Counter()
for f in sorted(glob.glob(os.path.join(EV, "b-*.json"))):
    lab = os.path.basename(f)[:-5]; d = load(lab)
    if not d: continue
    p(loop_row(lab, d))
    g = os.popen(f"python3 {HERE}/analyze_goals.py {os.path.join(EV, lab)}").read().strip(); p(f"{'':34s}   {g.split(': ',1)[-1]}")
    tp += d["trials"]["total"]; tc += d["trials"]["pass"]
    for k in ("shut_0x2", "shut_0x3", "shut_rst", "shut_other", "rdhup_v_without_record", "shut_late_after_finish", "flows_aborted", "flows_clean", "flows_started"): agg[k] += tot(d, k)
    for k in viol(d): agg[k] += viol(d)[k]
p(f"TOTAL loopback clean: trials={tp} pass={tc} | {dict(agg)}")

p("\n## C. control-session loss at seeded random points (loopback, in-process control channel)")
for pref in ("c2-loss-", "c2-lossall-", "c-loss-"):
    runs = [load(os.path.basename(f)[:-5]) for f in sorted(glob.glob(os.path.join(EV, pref + "*.json")))]
    runs = [r for r in runs if r]
    if not runs: continue
    A = collections.Counter()
    for d in runs:
        t = d["trials"]; A["runs"] += 1; A["trials"] += t["total"]; A["pass"] += t["pass"]; A["reset"] += t.get("reset", 0)
        for k, v in t["fail"].items(): A["FAIL:" + k] += v
        for k in ("ctl_lost", "paired_at_ctl_loss", "paired_flows_clean_after_ctl_loss", "intake_after_ctl_loss", "flows_clean", "flows_aborted", "abort_ctl_lost_unpaired", "abort_peer_close_while_open", "abort_ctl_lost_all", "epoch_discarded_stale_completion"): A[k] += tot(d, k)
        for k, v in viol(d).items(): A[k] += v
        A["live_end_nonzero"] += int(d["guest_live_flows_at_end"] + d["host_live_flows_at_end"] > 0)
    p(f"{pref}*: {dict(A)}")

p("\n## D. real recursion misses at volume (4000 flows x 2 MiB x 2 directions per run)")
for pref in ("d-full-", "d-off-", "d-incr1-"):
    runs = [load(os.path.basename(f)[:-5]) for f in sorted(glob.glob(os.path.join(EV, pref + "*.json")))]
    runs = [r for r in runs if r]
    if not runs: continue
    A = collections.Counter()
    for d in runs:
        b = d["bpf_counters"]; t = d["trials"]
        A["runs"] += 1; A["flows"] += t["total"]; A["pass"] += t["pass"]; A["stuck_or_failed"] += t["total"] - t["pass"]
        A["fexit_calls"] += b["fexit_send_calls"] + b["fexit_send_untracked_target"]
        A["recursion_misses_A"] += b["fexit_recursion_misses"] or 0; A["recursion_misses_B"] += b["fexit_b_recursion_misses"] or 0
        A["counter_a_short_of_b"] += tot(d, "counter_a_short_of_b"); A["counter_b_short_of_a"] += tot(d, "counter_b_short_of_a")
        A["rec_poke"] += tot(d, "complete_record_poke"); A["backstop_pokes"] += tot(d, "backstop_pokes"); A["backstop_rises"] += tot(d, "backstop_miss_rises")
        A["live_end"] += d["guest_live_flows_at_end"] + d["host_live_flows_at_end"]
        for k, v in viol(d).items(): A[k] += v
    p(f"{pref}*: {dict(A)}")

p("\n## E. flows larger than 2^32 bytes per direction")
for lab in ("e-wrap-5GiB-x4", "e-wrap-5GiB-pause"):
    d = load(lab)
    if d: p(loop_row(lab, d))

p("\n## K. split-process matrices (kill / control loss / app crash / destination reset); one line per scenario family")
def fam(tag): return re.sub(r"-\d+$", "", tag)
for envlabel in sorted({re.match(r"vm-(\w+)-(\w+?)-\d{8}T", os.path.basename(d)).groups() for d in glob.glob(os.path.join(EV, "vm-*-????????T*Z")) if re.match(r"vm-(\w+)-(\w+?)-\d{8}T", os.path.basename(d))}):
    env, lab = envlabel
    dirs = sorted(glob.glob(os.path.join(EV, f"vm-{env}-{lab}-*Z")))
    d = dirs[-1]
    res = []
    for f in sorted(glob.glob(os.path.join(d, "*-result.json"))):
        try: res.append(json.load(open(f)))
        except Exception: pass
    if not res: continue
    p(f"### env={env} plan={lab} dir={os.path.basename(d)} reps={len(res)}")
    byfam = collections.defaultdict(list)
    for r in res: byfam[fam(r["tag"])].append(r)
    for fm, rs in sorted(byfam.items()):
        cats = collections.Counter(); ends = collections.Counter(); ab = collections.Counter(); viol_ = collections.Counter(); live = 0; err = 0
        for r in rs:
            for k, v in (r.get("cats") or {}).items(): cats[k] += v
            for k, v in (r.get("ends") or {}).items(): ends[k] += v
            for who in ("guest_owner", "host_owner"):
                o = r.get(who)
                if o:
                    for k, v in o["aborts"].items(): ab[f"{who[:5]}:{k}"] += v
                    for k, v in o["viol"].items(): viol_[k] += v
                    live += o["live_at_end"] or 0
            err += 1 if r.get("error") else 0
        nv = sum(v for k, v in cats.items() if "violation" in k)
        p(f"  {fm:34s} reps={len(rs)} trials={sum(r.get('trials',0) for r in rs)} violations={nv} errors={err} live_at_end_total={live}\n      cats={dict(cats)}\n      ends={dict(ends)}\n      survivor_aborts={dict(ab)} owner_VIOLATIONs={dict(viol_)}")
open(os.path.join(HERE, "analysis.txt"), "w").write("\n".join(OUT) + "\n")

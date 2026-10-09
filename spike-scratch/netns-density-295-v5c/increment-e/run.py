#!/usr/bin/env python3
"""increment-e (derived from increment-d / increment-c): the V-5(c) evidence run on a private destination fixture (netns + veth, unframe on the veth only),
with tokenised epoll registrations in the owners. increment-b is preserved as evidence of the shared-`lo` interference.
Pre-registration: ../increment-b/PREREGISTRATION.md (unchanged)."""
import json, pathlib, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H
import v5c as V

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
CELLS = ("none", "empty", "recreate")


def acts(ev):
    return {"associate": V.count(ev, "udp_associate"), "teardown": V.count(ev, "udp_teardown")}


def a1(cells, connect=True, n=200):
    label = f"A1{'' if connect else 'u'}-{cells}"
    def fn():
        x = V.dest(label, "X")
        rc, ev, raw = V.app(f"udp-cycle --dst {V.addr('X')} {'--connect' if connect else ''} --cycles {n} --gap-ms 250 --tmo-ms 1500 --tag A --size 64", timeout=400)
        V.save_raw(label, "cycles", raw)
        time.sleep(3)  # idle: no association may open without an application send
        V.stop_dest(x)
        summ = V.summary_of(ev, "udp_cycle")
        bad = [e["r"] for e in ev if e.get("ev") == "cycle" and e["r"].get("ok") is not True]
        tg = V.tags(V.rec_entries(x))
        return {"summary": summ, "bad_cycles": bad[:20], "dest_received": len(tg), "dest_unique": len(set(tg)), "expected_associations": n}
    return V.scenario(label, cells, fn, host_extra=["--abort-dst", V.addr("X"), "--abort-after-ms", "80"])


def r1(cells, with_="refused"):
    label = f"R1-{with_}-{cells}"
    def fn():
        x = V.dest(label, "X")
        rc, ev, raw = V.app(f"udp-burst --dst {V.addr('X')} --connect --count 1 --size 64 --tag A --hold-ms 3000", timeout=60)
        time.sleep(0.5)
        V.stop_dest(x)
        return {"app": ev, "dest_received": V.tags(V.rec_entries(x))}
    return V.scenario(label, cells, fn, host_extra=["--refuse-dst", V.addr("X"), "--refuse-n", "1000000000", "--refuse-with", with_])


def r2(cells):
    out = []
    for kind, args in (("paced10", "--count 10 --gap-us 200000"), ("burst500", "--count 500 --gap-us 0")):
        label = f"R2-{kind}-{cells}"
        def fn():
            x = V.dest(label, "X")
            rc, ev, raw = V.app(f"udp-burst --dst {V.addr('X')} --connect {args} --size 64 --tag A --hold-ms 1500", timeout=60)
            time.sleep(0.5)
            V.stop_dest(x)
            return {"app": ev, "dest_received": V.tags(V.rec_entries(x))}
        out.append(V.scenario(label, cells, fn, host_extra=["--refuse-dst", V.addr("X"), "--refuse-n", "1000000000"]))
    return out


def r3(cells):
    label = f"R3-{cells}"
    def fn():
        x = V.dest(label, "X")
        rc, ev, raw = V.app(f"udp-cycle --dst {V.addr('X')} --connect --cycles 8 --gap-ms 300 --tmo-ms 250 --tag R --size 64", timeout=60)
        time.sleep(0.5)
        V.stop_dest(x)
        return {"summary": V.summary_of(ev, "udp_cycle"), "per_cycle_ok": [e["r"].get("ok") for e in ev if e.get("ev") == "cycle"], "dest_received": V.tags(V.rec_entries(x))}
    return V.scenario(label, cells, fn, host_extra=["--refuse-dst", V.addr("X"), "--refuse-n", "3"])


def b1(cells, how, reps):
    label = f"B1-{how}-{cells}"
    extra = {"refused": ["--refuse-with", "refused"], "abort": ["--refuse-with", "abort"], "silent": ["--refuse-with", "silent"]}[how]
    gextra = ["--udp-slots", "2"] + (["--pair-deadline-ms", "800"] if how == "silent" else [])
    hold = 1500 if how == "silent" else 700
    def fn():
        x = V.dest(label, "X")
        per = []
        for k in range(reps):
            rc, ev, raw = V.app(f"udp-burst --dst {V.addr('X')} --connect --tag A{k:02d} --count 5 --size 200 --hold-ms {hold} --then-cycles 3 --then-tag B{k:02d} --size 200 --tmo-ms 1500", timeout=60)
            per.append({"k": k, "then": V.summary_of(ev, "udp_then"), "rc": rc})
            time.sleep(0.15)
        time.sleep(0.5)
        V.stop_dest(x)
        ents = V.rec_entries(x)
        tg = V.tags(ents)
        a_leaked = [t for t in tg if t.startswith("A")]
        b_by_rep = {}
        for t in tg:
            if t.startswith("B"):
                b_by_rep.setdefault(t[1:3], []).append(t[:9])
        exact = sum(1 for k in range(reps) if b_by_rep.get(f"{k:02d}") == [f"B{k:02d}{i:06d}" for i in range(3)])
        return {"reps": reps, "A_frames_delivered": len(a_leaked), "A_sample": a_leaked[:10], "reps_B_exactly_once_in_order": exact,
                "reps_then_all_echoed": sum(1 for p in per if p["then"] and p["then"]["ok"] == 3), "reps_then_mismatches": sum((p["then"] or {}).get("mismatches", 0) for p in per),
                "per_rep": per, "dest_total": len(tg)}
    return V.scenario(label, cells, fn, host_extra=["--refuse-dst", V.addr("X"), "--refuse-n", "1000000000", "--fault-odd", "yes"] + extra, guest_extra=gextra)


def b2(cells, reps=30):
    label = f"B2-{cells}"
    def fn():
        x = V.dest(label, "X")
        y = V.dest(label, "Y")
        per = []
        for k in range(reps):
            rc1, ev1, raw1 = V.app(f"udp-burst --dst {V.addr('X')} --connect --tag A{k:02d} --count 5 --size 200 --hold-ms 200", timeout=60)
            time.sleep(1.0)
            rc2, ev2, raw2 = V.app(f"udp-cycle --dst {V.addr('Y')} --connect --cycles 3 --tag B{k:02d} --size 200 --tmo-ms 2500", timeout=60)
            per.append({"k": k, "cycle": V.summary_of(ev2, "udp_cycle")})
            time.sleep(0.2)
        time.sleep(1.0)
        V.stop_dest(x)
        V.stop_dest(y)
        ex, ey = V.tags(V.rec_entries(x)), V.tags(V.rec_entries(y))
        a_at_y = [t for t in ey if t.startswith("A")]
        b_by_rep = {}
        for t in ey:
            if t.startswith("B"):
                b_by_rep.setdefault(t[1:3], []).append(t[:9])
        exact = sum(1 for k in range(reps) if b_by_rep.get(f"{k:02d}") == [f"B{k:02d}{i:06d}" for i in range(3)])
        return {"reps": reps, "X_received": len(ex), "X_sample": ex[:10], "Y_received": len(ey), "A_frames_at_Y": len(a_at_y), "A_at_Y_sample": a_at_y[:10],
                "reps_B_exactly_once_in_order_at_Y": exact, "reps_cycle_all_ok": sum(1 for p in per if p["cycle"] and p["cycle"]["ok"] == 3),
                "reps_mismatches": sum((p["cycle"] or {}).get("mismatches", 0) for p in per), "per_rep": per}
    return V.scenario(label, cells, fn, host_extra=["--pair-delay-ms", "700", "--pair-delay-dst", V.PEER_IP], guest_extra=["--udp-slots", "1"])


def b3(cells, reps=25):
    label = f"B3-{cells}"
    def fn():
        z = V.dest(label, "Z", no_echo_tag="P")
        per = []
        for k in range(reps):
            rc, ev, raw = V.app(f"udp-burst --dst {V.addr('Z')} --connect --tag P{k:02d} --count 1 --size 100 --hold-ms 900 --then-cycles 3 --then-tag Q{k:02d} --size 100 --tmo-ms 1500", timeout=60)
            per.append({"k": k, "then": V.summary_of(ev, "udp_then"), "first_bad": [e["r"] for e in ev if e.get("ev") == "cycle" and e["r"].get("ok") is not True][:3]})
            time.sleep(0.15)
        time.sleep(0.5)
        V.stop_dest(z)
        return {"reps": reps, "reps_then_all_echoed": sum(1 for p in per if p["then"] and p["then"]["ok"] == 3),
                "then_timeouts": sum((p["then"] or {}).get("timeouts", 0) for p in per), "then_mismatches": sum((p["then"] or {}).get("mismatches", 0) for p in per), "per_rep": per}
    return V.scenario(label, cells, fn, host_extra=["--partial-dst", V.addr("Z"), "--fault-odd", "yes"])


def b4(cells, ks=(0, 1, 2, 4, 8), n=60, order=None):
    out = []
    for kms in ks:
        label = f"B4-k{kms}-{cells}" + (f"-{order}" if order else "")
        def fn():
            x = V.dest(label, "X")
            rc, ev, raw = V.app(f"udp-cycle --dst {V.addr('X')} --connect --cycles {n} --gap-ms 300 --tmo-ms 500 --size 30000 --tag L", timeout=300)
            V.save_raw(label, "cycles", raw)
            time.sleep(1.0)
            # quiescent probe: a clean association after the storm must echo byte-exact
            rc2, ev2, raw2 = V.app(f"udp-cycle --dst {V.addr('X')} --connect --cycles 1 --tag F --size 64 --tmo-ms 1500", timeout=30)
            V.stop_dest(x)
            return {"summary": V.summary_of(ev, "udp_cycle"), "final_probe": V.summary_of(ev2, "udp_cycle"),
                    "mismatch_cycles": [e["r"] for e in ev if e.get("ev") == "cycle" and e["r"].get("mismatch")][:10]}
        out.append(V.scenario(label, cells, fn, host_extra=["--abort-dst", V.addr("X"), "--abort-after-ms", str(kms)], guest_extra=(["--order", order] if order else None)))
    return out


def b5(cells, reps=10):
    label = f"B5-{cells}"
    def fn():
        x = V.dest(label, "X")
        per = []
        for k in range(reps):
            rc, ev, raw = V.app(f"udp-burst --dst {V.addr('X')} --connect --tag A{k:02d} --count 200 --size 1000 --hold-ms 2500", timeout=60)
            b = V.summary_of(ev, "udp_burst")
            per.append({"k": k, "local": b and b["local"], "sent_ok": b and b["sent_ok"]})
            time.sleep(0.3)
        time.sleep(1.0)
        V.stop_dest(x)
        ge = V.glog(label)
        by_port = {}
        for e in ge:
            if e.get("ev") == "udp_associate":
                by_port[e["app"]] = by_port.get(e["app"], 0) + 1
        for p in per:
            p["associations"] = by_port.get(p["local"], 0)
        cl = [e for e in ge if e.get("ev") == "cells_cleared"]
        return {"reps": reps, "delivered_at_X": len(V.rec_entries(x)), "associations_per_rep": [p["associations"] for p in per],
                "reps_with_exactly_one": sum(1 for p in per if p["associations"] == 1), "total_associations": sum(by_port.values()),
                "first_clear_per_rep_p2_inq_before": [e["p2_inq_before"] for e in cl][:12], "per_rep": per}
    return V.scenario(label, cells, fn, host_extra=["--refuse-dst", V.addr("X"), "--refuse-n", "1000000000", "--refuse-delay-ms", "500"],
                      guest_extra=["--cell-bufs", "4096", "--udp-slots", "2"])


def probes():
    cid = H.CTX["cid"]
    H.record("substrate", "guest_uname", H.gx(cid, "uname -a; cat /proc/modules | cut -d' ' -f1 | tr '\\n' ' '")[1])
    H.record("substrate", "ch_version", H.sh(f"{H.CTX['ch']} --version", quiet=True))
    for cells in CELLS:
        b5(cells)
    for cells in CELLS:
        r1(cells)
        r1(cells, "abort")
        r2(cells)
        r3(cells)
    for cells in CELLS:
        b1(cells, "refused", 40)
        b1(cells, "abort", 20)
        b1(cells, "silent", 15)
    for cells in CELLS:
        b2(cells)
    for cells in CELLS:
        b3(cells)
    for cells in CELLS:
        b4(cells)


def pre():
    H.record("substrate", "fixture", V.setup_fixture())
    H.CTX["host_extra"] = V.BASE_HOST


H.run_increment(probes, V.MODE + ["--cells", "none"], pre=pre)

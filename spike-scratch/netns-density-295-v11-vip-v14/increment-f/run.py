#!/usr/bin/env python3
"""increment-f: (1) slot-recycle hypothesis behind increment-e's E-first failures, with an
explicit host pairing-delay fault and an untraced control; (2) untraced first-datagram
stresses for tc-frag and hybrid; (3) V-14 re-run unchanged from increment-e with the
inet_diag/tcp_diag modules present. Pre-registration: spike/v11-vip-v14-findings.md."""
import json, pathlib, sys, threading, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H
import probes as P

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
MODE = P.OOB + P.DECLARE


def timeline(port, stop, out, every=0.05):
    t0 = time.monotonic()
    while not stop.is_set():
        out.append((round(time.monotonic() - t0, 3), P.probe(port, timeout=0.3)))
        time.sleep(every)


def stress(cid, dst, n, size, connect=False, tmo=1500):
    c = "--connect" if connect else ""
    return P.J(H.gx(cid, f"/vfwd udp-stress --dst {dst} --iterations {n} --size {size} --timeout-ms {tmo} {c}", timeout=1500, quiet=True)[1], "udp_stress")


def ns_marks():
    p = H.OUT / f"proc-{H.INC.name}-ns-udp7210.log"
    return sum(1 for l in p.read_text().splitlines() if '"len":777' in l) if p.exists() else -1


def slot_case(cid, fault):
    PD = f"{P.PEER}:7210"
    r = {"ns_777_before": ns_marks()}
    r["cells_recvq_before"] = H.gx(cid, "/usr/bin/ss -tn | awk 'NR==1 || $2 != 0'", quiet=True)[1]
    r["senders"] = [P.J(H.gx(cid, f"/vfwd udp-send --dst {P.TESTNET}:9 --size 777 --count 3", quiet=True)[1], "udp_send") for _ in range(8)]
    time.sleep(2)
    r["cells_recvq_after_senders"] = H.gx(cid, "/usr/bin/ss -tn | awk 'NR==1 || $2 != 0'", quiet=True)[1]
    r["stress_200"] = stress(cid, PD, 200, 64)
    time.sleep(1)
    r["ns_777_after"] = ns_marks()
    r["guest_owner_assoc_log"] = H.gx(cid, f"grep -E 'udp_associate|udp_teardown|udp_paired' /tmp/owner.log | grep -B2 -A2 '{P.TESTNET}' | tail -40", quiet=True)[1]
    r["guest_stats"] = H.gx(cid, "cat /tmp/owner-stats.json", quiet=True)[1]
    r["host_stats"] = H.host_stats()
    return r


def probes():
    cid, pub = H.CTX["cid"], H.CTX["pub"]
    gx, rec = H.gx, H.record
    phys = P.phys_dev()
    rec("substrate", "guest_uname", gx(cid, "uname -a; cat /proc/modules | cut -d' ' -f1 | tr '\\n' ' '")[1])
    rec("substrate", "offhost_topology", P.setup_offhost())
    D23 = ["--d23", f"{pub}:2730=7300", "--d23", f"{pub}:2731=7301", "--d23", f"{pub}:2732=7302", "--intake", f"{pub}:2713=7303"]
    EG = ["--egress-if", P.HV, "--egress-if", phys]
    PD = f"{P.PEER}:7210"
    # ============================================================= slot-recycle hypothesis
    H.set_modes(MODE, D23 + ["--unframe", "hybrid"] + EG)
    rec("F-slot", "control", slot_case(cid, False))
    H.set_modes(MODE, D23 + ["--unframe", "hybrid", "--pair-delay-ms", "300", "--pair-delay-dst", P.TESTNET] + EG)
    rec("F-slot", "fault", slot_case(cid, True))
    H.gsave("/tmp/owner.log", "guest-owner-slot-fault.log")
    # ============================================================= untraced first datagram
    for opt in ("tc-frag", "hybrid"):
        H.set_modes(MODE, D23 + ["--unframe", opt] + EG)
        rec("F-first-untraced", opt, {"unconn_64_1000": stress(cid, PD, 1000, 64), "conn_64_500": stress(cid, PD, 500, 64, True),
                                      "empty_300": stress(cid, PD, 300, 0), "s4097_200": stress(cid, PD, 200, 4097),
                                      "s59000_50": stress(cid, PD, 50, 59000, tmo=3000)})
    # ============================================================= V-14
    H.set_modes(MODE, D23 + ["--unframe", "hybrid"] + EG)
    w = {}
    w["refuse_d23_before_listen"] = P.probe(2730)
    t0 = time.monotonic()
    w["refuse_alwaysbound"] = P.probe_read(2713)
    w["refuse_alwaysbound_ms"] = (time.monotonic() - t0) * 1000
    hst = None
    gx(cid, "strace -f -qq -yy -s 0 -e trace=" + P.STRACE + " -p $(cat /tmp/owner.pid) -o /tmp/strace-v14.txt >/tmp/strace.err 2>&1 </dev/null & echo $! > /tmp/strace.pid; sleep 1", quiet=True)
    lags = {"up": [], "down": []}
    def on_line(line, t):
        j = P.J(line)
        if not j:
            return
        if j.get("ev") == "L":
            e, n, last = P.wait_for(2730, "ok")
            lags["up"].append(None if e is None else e * 1000)
        elif j.get("ev") == "C":
            e, n, last = P.wait_for(2730, "refused")
            lags["down"].append(None if e is None else e * 1000)
    H.gx_stream(cid, "/vfwd listen-cycle --port 7300 --cycles 20 --up-ms 100 --down-ms 100", on_line)
    gx(cid, "kill -INT $(cat /tmp/strace.pid); sleep 1", quiet=True)
    st = H.gsave("/tmp/strace-v14.txt", "strace-guest-owner-v14.txt")
    w["kp_guest_strace"] = H.strace_summary(st)
    w["kp_guest_socket_io_sizes"] = sorted(set(int(m.group(5)) for m in map(H.RE_LINE.match, st.splitlines()) if m and "socket" in m.group(4)))
    H.gx_stream(cid, "/vfwd listen-cycle --port 7300 --cycles 180 --up-ms 100 --down-ms 100", on_line, timeout=600)
    w["lag_ms"] = {k: {"n": len(x), "missing": sum(1 for y in x if y is None), "p50": P.pct(x, .5), "p90": P.pct(x, .9), "p99": P.pct(x, .99), "max": P.pct(x, 1)} for k, x in lags.items()}
    w["lag_raw_ms"] = lags
    tl, stop = [], threading.Event()
    th = threading.Thread(target=timeline, args=(2730, stop, tl)); th.start()
    marks = []
    t_start = time.monotonic()
    time.sleep(1.0)
    H.gx_stream(cid, "/vfwd listen-cycle --port 7300 --cycles 2 --up-ms 1000 --down-ms 1000", lambda l, t: marks.append((round(t - t_start, 3), l)))
    stop.set(); th.join()
    w["probe_timeline"] = tl
    w["probe_timeline_marks"] = marks
    for sig in ("9", "TERM"):
        gx(cid, f"/vfwd tcp-server --bind 0.0.0.0:7301 > /tmp/k.log 2>&1 </dev/null & echo $! > /tmp/k.pid")
        w[f"app_{sig}_up"] = P.wait_for(2731, "ok")
        w[f"app_{sig}_e2e"] = P.J(H.sh(f"timeout 20 {H.CTX['vfwd']} tcp-client --dst {pub}:2731 --size 1000", check=False))
        gx(cid, f"kill -{sig} $(cat /tmp/k.pid)")
        w[f"app_{sig}_down"] = P.wait_for(2731, "refused")
    sent0 = H.sh(f"grep -c 'd23_' {H.CTX['host_owner'].owner_log} || true", check=False, quiet=True).strip()
    rc, lines = H.gx_stream(cid, "/vfwd listen-churn --port 7302 --iterations 2000 --final open --hold-ms 4000 & sleep 0.5; echo PROBE", lambda l, t: None, timeout=120)
    w["churn_open_lines"] = lines[-3:]
    w["churn_open_probe_after"] = P.probe_read(2732)
    time.sleep(4)
    gx(cid, "/vfwd listen-churn --port 7302 --iterations 2000 --final closed")
    time.sleep(0.5)
    w["churn_closed_probe_after"] = P.probe(2732)
    w["churn_host_d23_events"] = H.sh(f"grep 'd23_.*7302' {H.CTX['host_owner'].owner_log} | cut -c1-160", check=False, quiet=True)
    gx(cid, "/vfwd listen-cycle --reuseport --port 7302 --cycles 1 --up-ms 8000 --down-ms 0 > /tmp/rp.log 2>&1 </dev/null & echo $! > /tmp/rp.pid")
    w["reuseport_up"] = P.wait_for(2732, "ok")
    closes_before = int(H.sh(f"grep -c 'd23_close.*7302' {H.CTX['host_owner'].owner_log} || true", check=False, quiet=True).strip() or 0)
    tl2, stop2 = [], threading.Event()
    th2 = threading.Thread(target=timeline, args=(2732, stop2, tl2, 0.002)); th2.start()
    w["reuseport_churn"] = gx(cid, "/vfwd listen-churn --reuseport --port 7302 --iterations 2000 --final closed")[1]
    time.sleep(0.5)
    stop2.set(); th2.join()
    closes_after = int(H.sh(f"grep -c 'd23_close.*7302' {H.CTX['host_owner'].owner_log} || true", check=False, quiet=True).strip() or 0)
    w["reuseport_window_probe_results"] = {r: sum(1 for _, x in tl2 if x == r) for r in set(x for _, x in tl2)}
    w["reuseport_window_host_closes"] = closes_after - closes_before
    gx(cid, "kill $(cat /tmp/rp.pid)")
    w["reuseport_after_steady_exit"] = P.wait_for(2732, "refused")
    gx(cid, "/vfwd listen-cycle --port 7300 --cycles 1 --up-ms 60000 --down-ms 0 > /tmp/hold.log 2>&1 </dev/null & echo $! > /tmp/hold.pid")
    w["restart1_hold_up"] = P.wait_for(2730, "ok")
    gx(cid, "kill -9 $(cat /tmp/owner.pid)")
    w["restart1_owner_killed"] = P.wait_for(2730, "refused")
    H.guest_owner_restart(MODE)
    w["restart1_owner_back"] = P.wait_for(2730, "ok", deadline_s=10)
    w["restart1_e2e"] = P.probe_read(2730)
    gx(cid, "kill -9 $(cat /tmp/owner.pid)")
    w["restart2_owner_killed"] = P.wait_for(2730, "refused")
    gx(cid, "kill $(cat /tmp/hold.pid)")
    H.guest_owner_restart(MODE)
    time.sleep(1)
    w["restart2_probe_after"] = P.probe(2730)
    w["restart2_state_sent"] = gx(cid, "grep listen_state_sent /tmp/owner.log | head -3", quiet=True)[1]
    gx(cid, "/vfwd listen-cycle --port 7300 --cycles 1 --up-ms 60000 --down-ms 0 > /tmp/hold.log 2>&1 </dev/null & echo $! > /tmp/hold.pid")
    w["restart3_hold_up"] = P.wait_for(2730, "ok")
    H.stop_host_owner()
    w["restart3_host_down_probe"] = P.probe(2730)
    H.start_host_owner(MODE)
    w["restart3_host_back"] = P.wait_for(2730, "ok", deadline_s=10)
    w["restart3_e2e"] = P.probe_read(2730)
    w["guest_owner_stats"] = gx(cid, "cat /tmp/owner-stats.json; grep -E 'ctl_open|ctl_closed|ctl_connect_failed' /tmp/owner.log | tail -6", quiet=True)[1]
    w["guest_listen_log"] = gx(cid, "grep -E 'listen_state_sent|listen_event' /tmp/owner.log | tail -12", quiet=True)[1]
    rec("V14", "result", w)
    H.gsave("/tmp/owner.log", "guest-owner-v14.log")


H.run_increment(probes, MODE)

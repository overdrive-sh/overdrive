#!/usr/bin/env python3
"""increment-e: the pre-registered evidence run (findings: spike/v11-vip-v14-findings.md).
V-11 for every D5a option, the missing-TC failure mode, VIP, V-14."""
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


def probes():
    cid, pub = H.CTX["cid"], H.CTX["pub"]
    gx, rec = H.gx, H.record
    phys = P.phys_dev()
    rec("substrate", "guest_uname", gx(cid, "uname -a; /usr/sbin/ip -br link; /usr/sbin/ip -4 addr show dummy0")[1])
    rec("substrate", "offhost_topology", P.setup_offhost())
    rec("substrate", "phys", H.sh(f"ip -br link show {phys}; ip -4 route get 1.1.1.1; ip -4 route get {P.PEER}", check=False))
    D23 = ["--d23", f"{pub}:2730=7300", "--d23", f"{pub}:2731=7301", "--d23", f"{pub}:2732=7302", "--intake", f"{pub}:2713=7303"]
    EG = ["--egress-if", P.HV, "--egress-if", phys]
    PD = f"{P.PEER}:7210"
    P.bpf_stats(True)
    H.CLEANUPS.append(lambda: P.bpf_stats(False))
    # ============================================================= V-11 / D5a
    for opt in ["tc", "tc-frag", "verdict", "verdict-naive", "hybrid"]:
        H.set_modes(MODE, D23 + ["--unframe", opt] + EG)
        rec("V11-programs", opt, H.sh(f"grep -o '\"programs\":.*' {H.CTX['host_owner'].owner_log} | head -c 3000", check=False, quiet=True))
        b = H.host_stats()
        candidate = opt in ("tc-frag", "hybrid")
        hst = P.strace_start(cid, opt) if candidate else None
        r = {}
        r["matrix_P"] = P.udp_matrix(cid, PD, timeout_ms=1500)
        r["same_assoc_unconn_P"] = P.udp_same_assoc(cid, PD)
        r["framelike_P"] = P.framelike(cid, PD)
        r["dig_1111"] = gx(cid, "dig @1.1.1.1 example.com +notcp +tries=1 +time=3 +short; echo rc=$?", quiet=True)[1]
        r["wire_nic"] = P.wire_probe(cid, phys, P.TESTNET, f"{opt}-nic")
        r["wire_veth_4096"] = P.wire_probe(cid, P.HV, P.PEER, f"{opt}-veth", sizes=(4096,))
        r["unrelated_framelike"] = P.host_udp_unrelated(True)
        r["unrelated_plain"] = P.host_udp_unrelated(False)
        if opt in ("verdict", "verdict-naive"):
            r["first_64_unconn_1000"] = stress(cid, PD, 1000, 64)
            r["first_empty_30"] = stress(cid, PD, 30, 0, tmo=300)
        else:
            r["first_64_unconn_1000"] = stress(cid, PD, 1000, 64)
            r["first_64_conn_500"] = stress(cid, PD, 500, 64, True)
            r["first_empty_300"] = stress(cid, PD, 300, 0)
            if candidate:
                r["first_4097_200"] = stress(cid, PD, 200, 4097)
                r["first_59000_50"] = stress(cid, PD, 50, 59000, tmo=3000)
        if hst:
            r["strace"] = P.strace_stop(cid, hst, opt)
        if opt in ("tc", "tc-frag", "hybrid"):
            r["tc_cost_unregistered"] = P.tc_cost(P.HV)
            r["tcx_stats"] = P.tcx_prog_stats(P.HV)
        r["counters"] = P.counters_delta(b, H.host_stats())
        rec("V11", opt, r)
    # ------------------------------------------------------------- missing TC on the egress interface
    for opt in ["tc-frag", "hybrid", "verdict"]:
        H.set_modes(MODE, D23 + ["--unframe", opt, "--egress-if", phys])
        rec("V11-missing-tc", opt, {"matrix_P": P.udp_matrix(cid, PD, sizes=[0, 1, 1431], timeout_ms=1500),
                                    "tcx_on_v11h": H.sh(f"bpftool net show dev {P.HV}", check=False, quiet=True)})
    # ============================================================= VIP
    VIPS = ["--vip", f"{P.VIP}:7200/tcp={P.PEER}:7200", "--vip", f"{P.VIP}:7201/tcp={P.PEER}:7201", "--vip", f"{P.VIP}:7210/udp={P.PEER}:7210",
            "--vip", f"{P.VIP2}:7000/tcp={pub}:7000", "--vip", f"{P.VIP2}:7010/udp={pub}:7010"]
    v = {}
    H.set_modes(MODE, D23 + ["--unframe", "hybrid"] + EG + VIPS)
    v["programs"] = H.sh(f"grep -o 'vip_connect4[^}}]*}}\\|\"vipmap\":[^]]*]' {H.CTX['host_owner'].owner_log}", check=False, quiet=True)
    v["tcp_vip_offhost"] = P.J(gx(cid, f"/vfwd tcp-client --dst {P.VIP}:7200 --size 100000")[1])
    v["tcp_vip_offhost_banner"] = P.J(gx(cid, f"/vfwd tcp-client --dst {P.VIP}:7201 --size 1000 --banner --timeout-ms 3000")[1])
    v["tcp_vip_hostlocal"] = P.J(gx(cid, f"/vfwd tcp-client --dst {P.VIP2}:7000 --size 1000")[1])
    v["sockname_vip"] = gx(cid, f"/vfwd sockname --dst {P.VIP}:7200")[1]
    v["tcp_vip_stress_300"] = P.J(gx(cid, f"/vfwd tcp-stress --dst {P.VIP}:7200 --iterations 300 --max-size 4096", timeout=600)[1], "tcp_stress")
    v["udp_vip_offhost"] = P.udp_matrix(cid, f"{P.VIP}:7210")
    v["udp_vip_hostlocal"] = P.udp_matrix(cid, f"{P.VIP2}:7010", sizes=[0, 1431, 59000])
    v["udp_vip_first_300"] = stress(cid, f"{P.VIP}:7210", 300, 64)
    v["host_log"] = H.sh(f"grep -E 'host_dg_installed|host_tcp_connect' {H.CTX['host_owner'].owner_log} | head -8", check=False, quiet=True)
    v["vip_hits"] = (H.host_stats() or {}).get("counters", [0] * 40)[34]
    H.set_modes(MODE, D23 + ["--unframe", "hybrid"] + EG)
    v["neg1_no_map_tcp"] = P.J(gx(cid, f"/vfwd tcp-client --dst {P.VIP}:7200 --size 100 --timeout-ms 7000", timeout=60)[1])
    v["neg1_no_map_udp"] = P.udp_matrix(cid, f"{P.VIP}:7210", sizes=[1], timeout_ms=1500)
    H.set_modes(MODE, D23 + ["--unframe", "tc-frag", "--tuple-from", "requested"] + EG + VIPS)
    v["neg2_tuple_requested_tcfrag"] = P.udp_matrix(cid, f"{P.VIP}:7210", sizes=[0, 1, 1431], timeout_ms=1500)
    rec("VIP", "result", v)
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

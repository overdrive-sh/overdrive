#!/usr/bin/env python3
"""increment-b (development): first pass over V-11 (all D5a options), VIP and V-14.
Exploration run: shakes out the mechanism and the harness before the pre-registered
evidence increment. Results here are not the verdict."""
import json, pathlib, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H
import probes as P

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
MODE = P.OOB + P.DECLARE


def probes():
    cid, pub = H.CTX["cid"], H.CTX["pub"]
    gx, rec = H.gx, H.record
    phys = P.phys_dev()
    rec("topo", "offhost", P.setup_offhost())
    rec("topo", "phys", phys)
    D23 = ["--d23", f"{pub}:2730=7300", "--d23", f"{pub}:2731=7301", "--d23", f"{pub}:2732=7302", "--intake", f"{pub}:2713=7303"]
    EG = ["--egress-if", P.HV, "--egress-if", phys]
    # ------------------------------------------------------------- V-11 per option
    P.bpf_stats(True)
    H.CLEANUPS.append(lambda: P.bpf_stats(False))
    for opt in ["tc", "tc-frag", "verdict", "verdict-naive", "hybrid"]:
        H.set_modes(MODE, D23 + ["--unframe", opt] + EG)
        b = H.host_stats()
        r = {}
        r["matrix_netns"] = P.udp_matrix(cid, f"{P.PEER}:7210")
        r["same_assoc_unconn"] = P.udp_same_assoc(cid, f"{P.PEER}:7210")
        r["dig_1111"] = gx(cid, "dig @1.1.1.1 example.com +notcp +tries=1 +time=3 +short; echo rc=$?", quiet=True)[1]
        r["wire_phys"] = P.wire_probe(cid, phys, P.TESTNET, f"{opt}-phys")
        r["wire_veth_4096"] = P.wire_probe(cid, P.HV, P.PEER, f"{opt}-veth", sizes=(4096,))
        r["unrelated_framelike"] = P.host_udp_unrelated(True)
        r["unrelated_plain"] = P.host_udp_unrelated(False)
        r["stress_unconn_500"] = P.J(gx(cid, f"/vfwd udp-stress --dst {P.PEER}:7210 --iterations 500 --size 64 --timeout-ms 1500", timeout=900)[1], "udp_stress")
        r["stress_empty_200"] = P.J(gx(cid, f"/vfwd udp-stress --dst {P.PEER}:7210 --iterations 200 --size 0 --timeout-ms 1000", timeout=900)[1], "udp_stress")
        r["tcx_stats_phys"] = P.tcx_prog_stats(phys)
        r["tcx_stats_veth"] = P.tcx_prog_stats(P.HV)
        r["counters"] = P.counters_delta(b, H.host_stats())
        rec("V11", opt, r)
        H.sh(f"cp {H.CTX['host_owner'].owner_log} {H.EV}/host-owner-v11-{opt}.log", check=False, quiet=True)
    # ------------------------------------------------------------- VIP
    VIPS = ["--vip", f"{P.VIP}:7200/tcp={P.PEER}:7200", "--vip", f"{P.VIP}:7210/udp={P.PEER}:7210",
            "--vip", f"{P.VIP2}:7000/tcp={pub}:7000", "--vip", f"{P.VIP2}:7010/udp={pub}:7010"]
    v = {}
    H.set_modes(MODE, D23 + ["--unframe", "hybrid"] + EG + VIPS)
    v["tcp_vip_offhost"] = P.J(gx(cid, f"/vfwd tcp-client --dst {P.VIP}:7200 --size 100000")[1])
    v["tcp_vip_hostlocal"] = P.J(gx(cid, f"/vfwd tcp-client --dst {P.VIP2}:7000 --size 1000")[1])
    v["sockname_vip"] = gx(cid, f"/vfwd sockname --dst {P.VIP}:7200")[1]
    v["udp_vip_offhost"] = P.udp_matrix(cid, f"{P.VIP}:7210")
    v["udp_vip_hostlocal"] = P.udp_matrix(cid, f"{P.VIP2}:7010", sizes=[0, 1431, 59000])
    v["udp_vip_stress"] = P.J(gx(cid, f"/vfwd udp-stress --dst {P.VIP}:7210 --iterations 300 --size 64 --timeout-ms 1500", timeout=600)[1], "udp_stress")
    v["host_log_tail"] = H.sh(f"grep -E 'host_dg_installed|host_tcp_connect' {H.CTX['host_owner'].owner_log} | tail -12", check=False, quiet=True)
    H.set_modes(MODE, D23 + ["--unframe", "hybrid"] + EG)
    v["neg_no_vipmap_tcp"] = P.J(gx(cid, f"/vfwd tcp-client --dst {P.VIP}:7200 --size 100 --timeout-ms 3000", timeout=60)[1])
    v["neg_no_vipmap_udp"] = P.udp_matrix(cid, f"{P.VIP}:7210", sizes=[1])
    H.set_modes(MODE, D23 + ["--unframe", "tc", "--tuple-from", "requested"] + EG + VIPS)
    v["neg_tuple_requested_tc"] = P.udp_matrix(cid, f"{P.VIP}:7210", sizes=[0, 1, 1431])
    rec("VIP", "result", v)
    # ------------------------------------------------------------- V-14
    H.set_modes(MODE, D23 + ["--unframe", "hybrid"] + EG)
    w = {}
    w["before_listen_d23"] = P.probe(2730)
    w["before_listen_alwaysbound"] = P.probe_read(2713)
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
    H.gx_stream(cid, "/vfwd listen-cycle --port 7300 --cycles 20 --up-ms 300 --down-ms 300", on_line)
    w["cycle_lag_ms"] = {k: {"n": len(x), "missing": sum(1 for y in x if y is None), "p50": P.pct(x, .5), "p99": P.pct(x, .99), "max": P.pct(x, 1)} for k, x in lags.items()}
    w["cycle_raw_ms"] = lags
    # kill -9
    gx(cid, "/vfwd tcp-server --bind 0.0.0.0:7301 > /tmp/k9.log 2>&1 </dev/null & echo $! > /tmp/k9.pid")
    w["k9_up"] = P.wait_for(2731, "ok")
    w["k9_e2e"] = P.J(H.sh(f"timeout 20 {H.CTX['vfwd']} tcp-client --dst {pub}:2731 --size 1000", check=False))
    gx(cid, "kill -9 $(cat /tmp/k9.pid)")
    w["k9_down"] = P.wait_for(2731, "refused")
    # churn
    rc, lines = H.gx_stream(cid, "/vfwd listen-churn --port 7302 --iterations 2000 --final open --hold-ms 3000", lambda l, t: None, timeout=120)
    time.sleep(0.3)
    w["churn_open_lines"] = lines[-3:]
    w["churn_open_probe_after"] = P.probe_read(2732)
    gx(cid, "/vfwd listen-churn --port 7302 --iterations 2000 --final closed")
    time.sleep(0.3)
    w["churn_closed_probe_after"] = P.probe(2732)
    # guest owner restart while the app listens
    gx(cid, "/vfwd listen-cycle --port 7300 --cycles 1 --up-ms 30000 --down-ms 0 > /tmp/hold.log 2>&1 </dev/null & echo $! > /tmp/hold.pid")
    w["hold_up"] = P.wait_for(2730, "ok")
    gx(cid, "kill -9 $(cat /tmp/owner.pid)")
    w["owner_killed_down"] = P.wait_for(2730, "refused")
    t0 = time.monotonic()
    H.guest_owner_restart(MODE)
    w["owner_restarted_up"] = P.wait_for(2730, "ok", deadline_s=10)
    w["owner_restarted_e2e"] = P.probe_read(2730)
    # host owner restart: guest reconnects and re-sends
    H.stop_host_owner()
    w["host_owner_down_probe"] = P.probe(2730)
    H.start_host_owner(MODE)
    w["host_restarted_up"] = P.wait_for(2730, "ok", deadline_s=10)
    w["host_restarted_e2e"] = P.probe_read(2730)
    w["guest_owner_listen_log"] = gx(cid, "grep -E 'listen_state_sent|ctl_open|ctl_closed' /tmp/owner.log | tail -40; cat /tmp/owner-stats.json", quiet=True)[1]
    w["host_owner_d23_log"] = H.sh(f"grep -hE 'd23_|listen_state' {H.CTX['host_owner'].owner_log} | tail -20", check=False, quiet=True)
    rec("V14", "result", w)


H.run_increment(probes, MODE)

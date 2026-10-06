#!/usr/bin/env python3
"""increment-d (development): V-14 after the fwmark_reflect / fd-budget fixes; VIP with the
complete map. Not the verdict."""
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


def probes():
    cid, pub = H.CTX["cid"], H.CTX["pub"]
    gx, rec = H.gx, H.record
    phys = P.phys_dev()
    rec("topo", "offhost", P.setup_offhost())
    D23 = ["--d23", f"{pub}:2730=7300", "--d23", f"{pub}:2731=7301", "--d23", f"{pub}:2732=7302", "--intake", f"{pub}:2713=7303"]
    EG = ["--egress-if", P.HV, "--egress-if", phys]
    VIPS = ["--vip", f"{P.VIP}:7200/tcp={P.PEER}:7200", "--vip", f"{P.VIP}:7201/tcp={P.PEER}:7201", "--vip", f"{P.VIP}:7210/udp={P.PEER}:7210",
            "--vip", f"{P.VIP2}:7000/tcp={pub}:7000", "--vip", f"{P.VIP2}:7010/udp={pub}:7010"]
    v = {}
    H.set_modes(MODE, D23 + ["--unframe", "tc-frag"] + EG + VIPS)
    v["tcp_vip_offhost"] = P.J(gx(cid, f"/vfwd tcp-client --dst {P.VIP}:7200 --size 100000")[1])
    v["tcp_vip_offhost_banner"] = P.J(gx(cid, f"/vfwd tcp-client --dst {P.VIP}:7201 --size 1000 --banner --timeout-ms 3000")[1])
    v["tcp_vip_hostlocal"] = P.J(gx(cid, f"/vfwd tcp-client --dst {P.VIP2}:7000 --size 1000")[1])
    v["tcp_vip_stress"] = P.J(gx(cid, f"/vfwd tcp-stress --dst {P.VIP}:7200 --iterations 300 --max-size 4096", timeout=600)[1], "tcp_stress")
    v["udp_vip_offhost"] = P.udp_matrix(cid, f"{P.VIP}:7210")
    v["udp_vip_hostlocal"] = P.udp_matrix(cid, f"{P.VIP2}:7010", sizes=[0, 1431, 59000])
    rec("VIP", "result", v)
    # ------------------------------------------------------------- V-14
    H.set_modes(MODE, D23 + ["--unframe", "tc-frag"] + EG)
    w = {}
    w["before_listen_d23"] = P.probe(2730)
    t0 = time.monotonic()
    w["before_listen_alwaysbound"] = P.probe_read(2713)
    w["before_listen_alwaysbound_ms"] = (time.monotonic() - t0) * 1000
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
    H.gx_stream(cid, "/vfwd listen-cycle --port 7300 --cycles 50 --up-ms 200 --down-ms 200", on_line)
    w["cycle_lag_ms"] = {k: {"n": len(x), "missing": sum(1 for y in x if y is None), "p50": P.pct(x, .5), "p99": P.pct(x, .99), "max": P.pct(x, 1)} for k, x in lags.items()}
    w["cycle_raw_ms"] = lags
    # readiness-probe timeline: down 1 s, up 1 s, down 1 s, up 1 s, down
    tl, stop = [], threading.Event()
    th = threading.Thread(target=timeline, args=(2730, stop, tl)); th.start()
    marks = []
    time.sleep(1.0)
    H.gx_stream(cid, "/vfwd listen-cycle --port 7300 --cycles 2 --up-ms 1000 --down-ms 1000", lambda l, t: marks.append((round(t, 3), l)))
    stop.set(); th.join()
    w["probe_timeline"] = tl
    w["probe_timeline_marks"] = marks
    # kill -9 and SIGTERM of the application
    for sig in ("9", "TERM"):
        gx(cid, f"/vfwd tcp-server --bind 0.0.0.0:7301 > /tmp/k.log 2>&1 </dev/null & echo $! > /tmp/k.pid")
        w[f"app_{sig}_up"] = P.wait_for(2731, "ok")
        w[f"app_{sig}_e2e"] = P.J(H.sh(f"timeout 20 {H.CTX['vfwd']} tcp-client --dst {pub}:2731 --size 1000", check=False))
        gx(cid, f"kill -{sig} $(cat /tmp/k.pid)")
        w[f"app_{sig}_down"] = P.wait_for(2731, "refused")
    # churn
    rc, lines = H.gx_stream(cid, "/vfwd listen-churn --port 7302 --iterations 2000 --final open --hold-ms 4000 & sleep 0.5; echo PROBE", lambda l, t: None, timeout=120)
    w["churn_open_lines"] = lines[-3:]
    w["churn_open_probe_after"] = P.probe_read(2732)
    time.sleep(4)
    gx(cid, "/vfwd listen-churn --port 7302 --iterations 2000 --final closed")
    time.sleep(0.5)
    w["churn_closed_probe_after"] = P.probe(2732)
    # SO_REUSEPORT: a steady listener while a second one churns -> no flap
    gx(cid, "/vfwd listen-cycle --reuseport --port 7302 --cycles 1 --up-ms 8000 --down-ms 0 > /tmp/rp.log 2>&1 </dev/null & echo $! > /tmp/rp.pid")
    w["reuseport_up"] = P.wait_for(2732, "ok")
    tl2, stop2 = [], threading.Event()
    th2 = threading.Thread(target=timeline, args=(2732, stop2, tl2, 0.002)); th2.start()
    gx(cid, "/vfwd listen-churn --reuseport --port 7302 --iterations 2000 --final closed")
    time.sleep(0.5)
    stop2.set(); th2.join()
    w["reuseport_churn_probe_results"] = {r: sum(1 for _, x in tl2 if x == r) for r in set(x for _, x in tl2)}
    w["reuseport_host_closes"] = H.sh(f"grep -c 'd23_close.*7302' {H.CTX['host_owner'].owner_log} || true", check=False, quiet=True)
    gx(cid, "kill $(cat /tmp/rp.pid)")
    w["reuseport_after_holder_exit"] = P.wait_for(2732, "refused")
    # guest-owner restart, app listening throughout
    gx(cid, "/vfwd listen-cycle --port 7300 --cycles 1 --up-ms 60000 --down-ms 0 > /tmp/hold.log 2>&1 </dev/null & echo $! > /tmp/hold.pid")
    w["hold_up"] = P.wait_for(2730, "ok")
    gx(cid, "kill -9 $(cat /tmp/owner.pid)")
    w["owner_killed_down"] = P.wait_for(2730, "refused")
    t0 = time.monotonic()
    H.guest_owner_restart(MODE)
    w["owner_restarted_up"] = P.wait_for(2730, "ok", deadline_s=10)
    w["owner_restarted_e2e"] = P.probe_read(2730)
    # guest-owner restart, app stops while the owner is down
    gx(cid, "kill -9 $(cat /tmp/owner.pid)")
    w["owner2_killed_down"] = P.wait_for(2730, "refused")
    gx(cid, "kill $(cat /tmp/hold.pid)")
    H.guest_owner_restart(MODE)
    time.sleep(1)
    w["owner2_restarted_probe"] = P.probe(2730)
    w["owner2_guest_state_sent"] = gx(cid, "grep listen_state_sent /tmp/owner.log | tail -5", quiet=True)[1]
    # host-owner restart: the guest reconnects and re-sends
    gx(cid, "/vfwd listen-cycle --port 7300 --cycles 1 --up-ms 60000 --down-ms 0 > /tmp/hold.log 2>&1 </dev/null & echo $! > /tmp/hold.pid")
    w["hold2_up"] = P.wait_for(2730, "ok")
    H.stop_host_owner()
    w["host_owner_down_probe"] = P.probe(2730)
    H.start_host_owner(MODE)
    w["host_restarted_up"] = P.wait_for(2730, "ok", deadline_s=10)
    w["host_restarted_e2e"] = P.probe_read(2730)
    w["guest_owner_stats"] = gx(cid, "cat /tmp/owner-stats.json; grep -E 'ctl_open|ctl_closed|ctl_connect_failed' /tmp/owner.log | tail -10", quiet=True)[1]
    rec("V14", "result", w)
    H.gsave("/tmp/owner.log", "guest-owner-v14.log")


H.run_increment(probes, MODE)

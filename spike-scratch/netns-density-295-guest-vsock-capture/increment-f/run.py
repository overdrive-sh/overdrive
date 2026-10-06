#!/usr/bin/env python3
"""increment-f: increment-e found intake children installed at PASSIVE_ESTABLISHED lose their
data_ready until accept() (no struct socket -> sk_psock_verdict_data_ready returns early);
fix: SO_RCVLOWAT re-arm right after accept(). FIONREAD is not used for psock TCP sockets
(tcp_bpf_ioctl reports the psock msg queue). Same probes, then 400-iteration stresses
with the 8-cell pools (reuse every 8 flows)."""
import pathlib, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
MODE = ["--ack", "oob", "--intake-mode", "park", "--arm", "yes", "--flush", "yes", "--udp", "park", "--park-pool", "8"]


def probes():
    cid, pub, V, W = H.CTX["cid"], H.CTX["pub"], H.CTX["vfwd"], H.W
    gx, rec, sh = H.gx, H.record, H.sh
    rc, t = gx(cid, f"/vfwd udp-client --dst {pub}:7010 --sizes 0,1,1431,4096,4097,59000")
    rec("v5", "udp_unconnected", H.last_json(t))
    rc, t = gx(cid, f"/vfwd udp-client --dst {pub}:7010 --sizes 59000,0,8192 --connect")
    rec("v5", "udp_connected", H.last_json(t))
    for n in range(4):
        rc, t = gx(cid, f"/vfwd tcp-client --dst {pub}:7000 --size 1000")
        rec("v3", f"reuse_seq_{n}", H.last_json(t))
    rc, t = gx(cid, f"/vfwd tcp-stress --dst {pub}:7000 --iterations 40 --max-size 4096", timeout=600)
    rec("v3", "stress40_egress_pool8", H.last_json(t, "tcp_stress"))
    H.gsave("/tmp/owner.log", "guest-owner-after-stress.log")
    rec("owner", "guest_stats", gx(cid, "cat /tmp/owner-stats.json")[1])
    rec("owner", "host_stats", H.host_stats())
    r = sh(f"timeout 300 {V} tcp-stress --dst {pub}:2711 --iterations 40 --banner --max-size 4096", check=False)
    rec("v4", "inbound_stress40_banner_pool8", r)
    rc, t = gx(cid, f"/vfwd tcp-stress --dst {pub}:7000 --iterations 400 --max-size 4096", timeout=900)
    rec("v3", "stress400_egress_pool8", H.last_json(t, "tcp_stress"))
    rc, t = gx(cid, f"/vfwd tcp-stress --dst {pub}:7001 --iterations 400 --max-size 4096 --banner", timeout=900)
    rec("v3", "stress400_egress_banner_pool8", H.last_json(t, "tcp_stress"))
    r = sh(f"timeout 600 {V} tcp-stress --dst {pub}:2710 --iterations 400 --max-size 4096", check=False)
    rec("v4", "inbound_stress400_early_pool8", r)
    rec("owner", "guest_stats_end", gx(cid, "cat /tmp/owner-stats.json")[1])
    rec("owner", "host_stats_end", H.host_stats())


H.run_increment(probes, MODE)

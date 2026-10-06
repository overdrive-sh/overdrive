#!/usr/bin/env python3
"""increment-d: smoke after the establishment-install redesign (increment-c showed:
FIN-only skb redirect -> target EPIPE; FIN-before-install -> SockHash insert EOPNOTSUPP).
Intake children are installed at PASSIVE_ESTABLISHED by sock_ops and park in a kernel
TCP cell until Paired; acceptor active legs are armed for ACTIVE_ESTABLISHED install."""
import pathlib, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
OOB = ["--ack", "oob", "--intake-mode", "park", "--arm", "yes", "--flush", "yes", "--udp", "park"]


def probes():
    cid, pub, V, W = H.CTX["cid"], H.CTX["pub"], H.CTX["vfwd"], H.W
    gx, rec, sh = H.gx, H.record, H.sh
    rc, t = gx(cid, f"/vfwd sockname --dst {pub}:7000; /vfwd sockname --bind {W}:0 --dst {pub}:7000")
    rec("d15", "sockname", t)
    rc, t = gx(cid, f"/vfwd tcp-client --dst {pub}:7000 --size 1048576")
    rec("v2", "tcp_1mib", H.last_json(t))
    rc, t = gx(cid, "cat /tmp/owner.out; tail -c 4000 /tmp/owner.log")
    rec("owner", "after_first_tcp", t)
    rc, t = gx(cid, f"/vfwd tcp-client --dst {pub}:7001 --size 4096 --banner")
    rec("v3", "banner_once", H.last_json(t))
    rc, t = gx(cid, f"/vfwd tcp-client --dst {pub}:7002 --size 100 --expect-rst")
    rec("v2", "rst_once", H.last_json(t))
    rc, t = gx(cid, f"/vfwd tcp-client --dst {pub}:7003 --size 4194304")
    rec("v2", "k2_server_withholds_rx_300ms", H.last_json(t))
    rc, t = gx(cid, f"/vfwd tcp-client --dst {pub}:7000 --size 4194304 --read-delay 300")
    rec("v2", "k2_client_withholds_rx_300ms", H.last_json(t))
    rc, t = gx(cid, f"curl -sS -o /tmp/blob -w '%{{http_code}} %{{size_download}} %{{local_ip}}:%{{local_port}} %{{remote_ip}}:%{{remote_port}}\\n' http://{pub}:8080/blob.bin; sha256sum /tmp/blob")
    rec("v2", "curl_host", t)
    rc, t = gx(cid, "getent hosts example.com; echo rc=$?")
    rec("v5", "getent", t)
    rc, t = gx(cid, f"/vfwd udp-client --dst {pub}:7010 --sizes 0,1,1431,59000")
    rec("v5", "udp_unconnected", H.last_json(t))
    rc, t = gx(cid, f"/vfwd tcp-stress --dst {pub}:7000 --iterations 200 --max-size 4096")
    rec("v3", "stress200_egress", H.last_json(t, "tcp_stress"))
    rc, t = gx(cid, f"/vfwd tcp-stress --dst {pub}:7001 --iterations 200 --max-size 4096 --banner")
    rec("v3", "stress200_egress_banner", H.last_json(t, "tcp_stress"))
    r = sh(f"timeout 20 ssh -i {H.OUT}/keys/client -p 2222 -o BatchMode=yes -o StrictHostKeyChecking=no "
           f"-o UserKnownHostsFile=/dev/null root@{pub} 'echo SSH_CONNECTION=$SSH_CONNECTION; uname -r'", check=False)
    rec("v4", "ssh_once", r)
    r = sh(f"timeout 30 {V} tcp-client --dst {pub}:2711 --size 4096 --banner", check=False)
    rec("v4", "inbound_banner_once", r)
    r = sh(f"timeout 30 {V} tcp-client --dst {pub}:2710 --size 65536", check=False)
    rec("v4", "inbound_plain_once", r)
    r = sh(f"timeout 120 {V} tcp-stress --dst {pub}:2711 --iterations 200 --banner --max-size 4096", check=False)
    rec("v4", "inbound_stress200_banner", r)
    rc, t = gx(cid, "cat /tmp/owner-stats.json; tail -c 3000 /tmp/owner.log")
    rec("owner", "guest_stats", t)
    rec("owner", "host_stats", H.host_stats())


H.run_increment(probes, OOB)

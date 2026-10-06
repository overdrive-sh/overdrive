#!/usr/bin/env python3
"""increment-b: smoke of every path after increment-a fixes (host listeners bound
VMADDR_CID_ANY; guest uses iproute2 by absolute path; robust module restore)."""
import pathlib, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
OOB = ["--ack", "oob", "--flush", "yes", "--udp", "park"]


def probes():
    cid, pub, V, W = H.CTX["cid"], H.CTX["pub"], H.CTX["vfwd"], H.W
    gx, rec, sh = H.gx, H.record, H.sh
    rc, t = gx(cid, "uname -r; /usr/sbin/ip -br link; /usr/sbin/ip -4 addr; cat /tmp/owner.out")
    rec("d15", "guest_net", t)
    rc, t = gx(cid, f"/vfwd sockname --dst {pub}:7000; /vfwd sockname --bind {W}:0 --dst {pub}:7000")
    rec("d15", "sockname", t)
    rc, t = gx(cid, f"/vfwd tcp-client --dst {pub}:7000 --size 1048576")
    rec("v2", "tcp_1mib", H.last_json(t))
    rc, t = gx(cid, f"/vfwd tcp-client --dst {pub}:7001 --size 4096 --banner")
    rec("v3", "banner_once", H.last_json(t))
    rc, t = gx(cid, f"/vfwd tcp-client --dst {pub}:7002 --size 100 --expect-rst")
    rec("v2", "rst_once", H.last_json(t))
    rc, t = gx(cid, f"curl -sS -o /tmp/blob -w '%{{http_code}} %{{size_download}} %{{local_ip}}:%{{local_port}} %{{remote_ip}}:%{{remote_port}}\\n' http://{pub}:8080/blob.bin; sha256sum /tmp/blob")
    rec("v2", "curl_host", t)
    rc, t = gx(cid, "getent hosts example.com; echo rc=$?")
    rec("v5", "getent", t)
    rc, t = gx(cid, f"/vfwd udp-client --dst {pub}:7010 --sizes 0,1,1431,59000")
    rec("v5", "udp_unconnected", H.last_json(t))
    rc, t = gx(cid, "head -c 8000 /tmp/owner.log; cat /tmp/owner-stats.json")
    rec("owner", "guest_log_head", t)
    rec("owner", "host_stats", H.host_stats())
    r = sh(f"timeout 20 ssh -i {H.OUT}/keys/client -p 2222 -o BatchMode=yes -o StrictHostKeyChecking=no "
           f"-o UserKnownHostsFile=/dev/null root@{pub} 'echo SSH_CONNECTION=$SSH_CONNECTION; uname -r'", check=False)
    rec("v4", "ssh_once", r)
    r = sh(f"timeout 20 {V} tcp-client --dst {pub}:2711 --size 4096 --banner", check=False)
    rec("v4", "inbound_banner_once", r)
    r = sh(f"timeout 20 {V} tcp-client --dst {pub}:2710 --size 65536", check=False)
    rec("v4", "inbound_plain_once", r)


H.run_increment(probes, OOB)

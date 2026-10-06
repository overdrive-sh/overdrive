#!/usr/bin/env python3
"""increment-h: final build (K2 counters non-LRU + deleted on close -- increment-g's single
inbound failure was an LRU eviction of a live parking-cell counter that delayed FIN 10 s).
Re-runs every V-3 stress on this build, the guest-side strace (increment-g's guest strace
never started: no busybox nohup applet), the pre-registered negative controls (LATE-NOFLUSH,
LATE-FLUSH, D18, UDP-NAIVE) and the fail-closed probes."""
import json, os, pathlib, re, signal, subprocess, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
OOB = ["--ack", "oob", "--intake-mode", "park", "--arm", "yes", "--flush", "yes", "--udp", "park"]
LATE_NOFLUSH = ["--ack", "oob", "--intake-mode", "late", "--arm", "no", "--flush", "no", "--udp", "park"]
LATE_FLUSH = ["--ack", "oob", "--intake-mode", "late", "--arm", "no", "--flush", "yes", "--udp", "park"]
D18 = ["--ack", "inband", "--intake-mode", "late", "--arm", "no", "--flush", "yes", "--udp", "park"]
UDP_NAIVE = ["--ack", "oob", "--intake-mode", "park", "--arm", "yes", "--flush", "yes", "--udp", "naive"]
STRACE = "read,write,readv,writev,recvfrom,sendto,recvmsg,sendmsg,recvmmsg,sendmmsg,splice,sendfile,pread64,pwrite64"


def server_lines(name):
    p = H.OUT / f"proc-{H.INC.name}-{name}.log"
    return len(p.read_text().splitlines()) if p.exists() else -1


def probes():
    cid, pub, V, W = H.CTX["cid"], H.CTX["pub"], str(H.CTX["vfwd"]), H.W
    gx, rec, sh, J = H.gx, H.record, H.sh, H.last_json
    SCP = (f"scp -i {H.OUT}/keys/client -P 2222 -o BatchMode=yes -o StrictHostKeyChecking=no "
           f"-o UserKnownHostsFile=/dev/null -o LogLevel=ERROR")
    rec("G-D15", "guest_net", gx(cid, "uname -r; /usr/sbin/ip -br link; /usr/sbin/ip -4 addr")[1])
    rec("G-V2d", "curl_telnet_github_ssh_banner",
        gx(cid, "(sleep 4) | timeout 8 curl -s -m 6 telnet://github.com:22 | head -c 40; echo; echo done", timeout=60)[1])
    # ---------------------------------------------------------------- kernel path proof (both owners)
    gx(cid, f"strace -f -qq -yy -s 0 -e trace={STRACE} -p $(cat /tmp/owner.pid) -o /tmp/strace-owner.txt "
            f">/tmp/strace.err 2>&1 </dev/null & echo $! > /tmp/strace.pid; sleep 1; cat /tmp/strace.pid /tmp/strace.err")
    hst = subprocess.Popen(["strace", "-f", "-qq", "-yy", "-s", "0", "-e", f"trace={STRACE}", "-p", str(H.CTX["host_owner"].pid),
                            "-o", str(H.EV / "strace-host-owner.txt")], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    kp = {"window": "64 MiB TCP egress + getent + UDP 0/1431/59000 + scp 8 MiB inbound + 64 KiB inbound litmus"}
    blob8 = H.OUT / "blob8h.bin"
    blob8.write_bytes(os.urandom(8 * 1024 * 1024))
    kp["tcp_64mib"] = J(gx(cid, f"/vfwd tcp-client --dst {pub}:7000 --size 67108864", timeout=300)[1])
    kp["dns"] = gx(cid, "getent hosts example.com")[1]
    kp["udp"] = J(gx(cid, f"/vfwd udp-client --dst {pub}:7010 --sizes 0,1431,59000")[1])
    kp["scp_up"] = sh(f"timeout 120 {SCP} {blob8} root@{pub}:/tmp/blob8h.bin && echo UP_OK", check=False)
    kp["scp_sha_host"] = H.sha(blob8)
    kp["inbound_64k"] = J(sh(f"timeout 30 {V} tcp-client --dst {pub}:2710 --size 65536", check=False))
    time.sleep(1)
    hst.send_signal(signal.SIGINT)
    hst.wait(timeout=20)
    gx(cid, "kill -INT $(cat /tmp/strace.pid); sleep 1; sha256sum /tmp/blob8h.bin; wc -l /tmp/strace-owner.txt")
    gtxt = H.gsave("/tmp/strace-owner.txt", "strace-guest-owner.txt")
    htxt = (H.EV / "strace-host-owner.txt").read_text(errors="replace")
    for side, txt in [("guest", gtxt), ("host", htxt)]:
        kp[f"{side}_syscall_bytes"] = H.strace_summary(txt)
        socket_ios = [int(m.group(5)) for m in map(H.RE_LINE.match, txt.splitlines()) if m and "socket" in m.group(4)]
        kp[f"{side}_socket_io_count"] = len(socket_ios)
        kp[f"{side}_socket_io_max_bytes"] = max(socket_ios or [0])
        kp[f"{side}_socket_io_sizes"] = sorted(set(socket_ios))
    kp["guest_stats"] = gx(cid, "cat /tmp/owner-stats.json")[1]
    kp["host_stats"] = H.host_stats()
    rec("G-KP", "result", kp)
    # ---------------------------------------------------------------- V-3 stresses on the final build
    st = {}
    st["egress_early_fin_10000"] = J(gx(cid, f"/vfwd tcp-stress --dst {pub}:7000 --iterations 10000 --max-size 4096", timeout=1500)[1], "tcp_stress")
    st["egress_early_fin_10000_par8"] = J(gx(cid, f"/vfwd tcp-stress --dst {pub}:7000 --iterations 10000 --max-size 4096 --parallel 8", timeout=1500)[1], "tcp_stress")
    st["egress_server_first_10000"] = J(gx(cid, f"/vfwd tcp-stress --dst {pub}:7001 --iterations 10000 --max-size 4096 --banner", timeout=1500)[1], "tcp_stress")
    st["egress_no_fin_early_ack_10000"] = J(gx(cid, f"/vfwd tcp-stress --dst {pub}:7004 --iterations 10000 --max-size 4096 --early-ack", timeout=1500)[1], "tcp_stress")
    st["inbound_server_first_10000"] = J(sh(f"timeout 1500 {V} tcp-stress --dst {pub}:2711 --iterations 10000 --banner --max-size 4096", check=False, timeout=1600), "tcp_stress")
    st["inbound_client_writes_immediately_10000"] = J(sh(f"timeout 1500 {V} tcp-stress --dst {pub}:2710 --iterations 10000 --max-size 4096", check=False, timeout=1600), "tcp_stress")
    st["udp_first_datagram_unconnected_10000"] = J(gx(cid, f"/vfwd udp-stress --dst {pub}:7010 --iterations 10000 --size 64", timeout=1500)[1], "udp_stress")
    st["udp_first_datagram_connected_2000"] = J(gx(cid, f"/vfwd udp-stress --dst {pub}:7010 --iterations 2000 --size 64 --connect", timeout=1500)[1], "udp_stress")
    rec("G-V3", "stresses", st)
    time.sleep(2)
    rec("G-V3-5", "guest_ss", gx(cid, "/usr/bin/ss -tna | awk 'NR==1 || $2 != 0'; echo; /usr/bin/ss -tna | wc -l")[1])
    rec("G-V3-5", "host_ss_nonzero_recvq", sh("ss -tna | awk 'NR==1 || $2 != 0'", check=False, quiet=True))
    rec("G-V3-5", "guest_stats", gx(cid, "cat /tmp/owner-stats.json")[1])
    rec("G-V3-5", "host_stats", H.host_stats())
    H.gsave("/tmp/owner.log", "guest-owner-oob.log")
    # ---------------------------------------------------------------- negative controls (K1)
    ctl = {}
    H.set_modes(LATE_NOFLUSH)
    ctl["late_noflush_early_ack_100"] = J(gx(cid, f"/vfwd tcp-stress --dst {pub}:7004 --iterations 100 --max-size 4096 --early-ack --timeout-ms 1000", timeout=600)[1], "tcp_stress")
    ctl["late_noflush_stats"] = gx(cid, "cat /tmp/owner-stats.json")[1]
    H.set_modes(LATE_FLUSH)
    ctl["late_flush_early_ack_1000"] = J(gx(cid, f"/vfwd tcp-stress --dst {pub}:7004 --iterations 1000 --max-size 4096 --early-ack --timeout-ms 1000", timeout=900)[1], "tcp_stress")
    ctl["late_flush_write_fin_immediately_100"] = J(gx(cid, f"/vfwd tcp-stress --dst {pub}:7000 --iterations 100 --max-size 4096 --timeout-ms 1000", timeout=600)[1], "tcp_stress")
    ctl["late_flush_stats"] = gx(cid, "cat /tmp/owner-stats.json")[1]
    H.gsave("/tmp/owner.log", "guest-owner-late-flush.log")
    H.set_modes(D18)
    ctl["d18_egress_client_first_hold50_200"] = J(gx(cid, f"/vfwd tcp-stress --dst {pub}:7000 --iterations 200 --max-size 4096 --hold-ms 50 --timeout-ms 1000", timeout=600)[1], "tcp_stress")
    ctl["d18_egress_server_first_100"] = J(gx(cid, f"/vfwd tcp-stress --dst {pub}:7001 --iterations 100 --max-size 4096 --banner --timeout-ms 1000", timeout=600)[1], "tcp_stress")
    ctl["d18_inbound_server_first_100"] = J(sh(f"timeout 600 {V} tcp-stress --dst {pub}:2711 --iterations 100 --banner --max-size 4096 --timeout-ms 1000", check=False, timeout=700), "tcp_stress")
    ctl["d18_guest_stats"] = gx(cid, "cat /tmp/owner-stats.json; grep -c inband_ack_bad /tmp/owner.log")[1]
    ctl["d18_host_stats"] = H.host_stats()
    H.gsave("/tmp/owner.log", "guest-owner-d18.log")
    H.set_modes(UDP_NAIVE)
    ctl["udp_naive_first_datagram_30"] = J(gx(cid, f"/vfwd udp-stress --dst {pub}:7010 --iterations 30 --size 64 --timeout-ms 1000", timeout=300)[1], "udp_stress")
    ctl["udp_naive_stats"] = gx(cid, "cat /tmp/owner-stats.json; grep udp_associate /tmp/owner.log | head -5")[1]
    rec("G-K1", "controls", ctl)
    # ---------------------------------------------------------------- fail-closed
    H.set_modes(OOB)
    fc = {}
    fc["sanity_before"] = J(gx(cid, f"/vfwd tcp-client --dst {pub}:7000 --size 1000")[1])
    fc["guest_cgroup_bpf_before"] = gx(cid, "bpftool cgroup show /sys/fs/cgroup 2>&1")[1]
    n7000, n8080 = server_lines("tcp7000"), server_lines("http8080")
    fc["fc1_guest_owner_kill"] = gx(cid, f"/vfwd tcp-client --dst {pub}:7003 --size 67108864 > /tmp/fc1.json 2>&1 </dev/null & "
                                         f"sleep 0.15; kill -9 $(cat /tmp/owner.pid); echo killed_at_ms=$(date +%s%3N); wait; cat /tmp/fc1.json; "
                                         f"bpftool cgroup show /sys/fs/cgroup 2>&1; "
                                         f"/vfwd sockname --dst {pub}:7000; /vfwd udp-client --dst {pub}:7010 --sizes 1; "
                                         f"getent hosts example.com; echo getent_rc=$?; "
                                         f"curl -sS -m 3 -o /dev/null -w '%{{http_code}}\\n' http://{pub}:8080/blob.bin; echo curl_rc=$?", timeout=120)[1]
    time.sleep(1)
    fc["fc1_host_server_new_lines"] = {"tcp7000": server_lines("tcp7000") - n7000, "http8080": server_lines("http8080") - n8080}
    H.set_modes(OOB)
    fc["sanity_after_fc1"] = J(gx(cid, f"/vfwd tcp-client --dst {pub}:7000 --size 1000")[1])
    gx(cid, f"/vfwd tcp-client --dst {pub}:7003 --size 67108864 > /tmp/fc2.json 2>&1 </dev/null & echo $! > /tmp/fc2.pid")
    time.sleep(0.15)
    H.CTX["host_owner"].kill()
    fc["fc2_host_owner_killed"] = True
    time.sleep(2)
    fc["fc2_guest_view"] = gx(cid, f"while kill -0 $(cat /tmp/fc2.pid) 2>/dev/null; do sleep 0.2; done; cat /tmp/fc2.json; "
                                   f"/vfwd tcp-client --dst {pub}:7000 --size 100 --timeout-ms 3000; "
                                   f"/vfwd udp-client --dst {pub}:7010 --sizes 1; getent hosts example.com; echo getent_rc=$?; "
                                   f"tail -5 /tmp/owner.log; cat /tmp/owner-stats.json", timeout=120)[1]
    H.set_modes(OOB)
    fc["sanity_after_fc2"] = J(gx(cid, f"/vfwd tcp-client --dst {pub}:7000 --size 1000")[1])
    fc["sanity_after_fc2_udp"] = J(gx(cid, f"/vfwd udp-client --dst {pub}:7010 --sizes 0,59000")[1])
    rec("G-FC", "result", fc)


H.run_increment(probes, OOB)

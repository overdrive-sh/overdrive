#!/usr/bin/env python3
"""increment-g: pre-registered evidence run, OOB mode (the mechanism): D15, V-2, V-3
(10,000-iteration stresses), V-4 (sshd/scp), V-5 (resolvers, UDP sizes, first-datagram
stress) and the kernel-path proof (strace of both owners + BPF counters).
Pre-registration: docs/feature/netns-density-295/spike/guest-vsock-capture-findings.md"""
import json, os, pathlib, re, signal, subprocess, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
OOB = ["--ack", "oob", "--intake-mode", "park", "--arm", "yes", "--flush", "yes", "--udp", "park"]
STRACE = "read,write,readv,writev,recvfrom,sendto,recvmsg,sendmsg,recvmmsg,sendmmsg,splice,sendfile,pread64,pwrite64"


def probes():
    cid, pub, V, W = H.CTX["cid"], H.CTX["pub"], str(H.CTX["vfwd"]), H.W
    gx, rec, sh, J = H.gx, H.record, H.sh, H.last_json
    SSH = (f"ssh -i {H.OUT}/keys/client -p 2222 -o BatchMode=yes -o StrictHostKeyChecking=no "
           f"-o UserKnownHostsFile=/dev/null -o LogLevel=ERROR")
    SCP = (f"scp -i {H.OUT}/keys/client -P 2222 -o BatchMode=yes -o StrictHostKeyChecking=no "
           f"-o UserKnownHostsFile=/dev/null -o LogLevel=ERROR")
    # ---------------------------------------------------------------- D15 identity
    rec("G-D15", "guest_net", gx(cid, "uname -a; /usr/sbin/ip -br link; /usr/sbin/ip -4 addr; /usr/sbin/ip route; "
                                     "/usr/sbin/ip rule; ls /sys/class/net; cat /proc/net/dev")[1])
    rec("G-D15", "sockname", gx(cid, f"/vfwd sockname --dst {pub}:7000; /vfwd sockname --bind {W}:0 --dst {pub}:7000")[1])
    # ---------------------------------------------------------------- V-2
    rec("G-V2a", "tcp_1mib", J(gx(cid, f"/vfwd tcp-client --dst {pub}:7000 --size 1048576")[1]))
    rec("G-V2a", "tcp_64mib", J(gx(cid, f"/vfwd tcp-client --dst {pub}:7000 --size 67108864", timeout=300)[1]))
    rec("G-V2b", "server_rx_withheld_300ms_4mib", J(gx(cid, f"/vfwd tcp-client --dst {pub}:7003 --size 4194304")[1]))
    rec("G-V2b", "client_rx_withheld_300ms_4mib", J(gx(cid, f"/vfwd tcp-client --dst {pub}:7000 --size 4194304 --read-delay 300")[1]))
    rec("G-V2c", "rst", J(gx(cid, f"/vfwd tcp-client --dst {pub}:7002 --size 100 --expect-rst")[1]))
    rec("G-V2d", "curl_host_blob", gx(cid, f"curl -sS -o /tmp/blob -w '%{{http_code}} %{{size_download}} local=%{{local_ip}}:%{{local_port}} remote=%{{remote_ip}}:%{{remote_port}}\\n' http://{pub}:8080/blob.bin; sha256sum /tmp/blob")[1])
    rec("G-V2d", "curl_internet", gx(cid, "curl -sS -m 15 -o /tmp/ex.html -w '%{http_code} %{size_download} local=%{local_ip}:%{local_port} remote=%{remote_ip}:%{remote_port}\\n' http://example.com/; head -c 120 /tmp/ex.html", timeout=60)[1])
    rec("G-V2d", "nc_github_ssh_banner", gx(cid, "timeout 10 nc github.com 22 </dev/null | head -c 40; echo; echo rc=$?", timeout=60)[1])
    # ---------------------------------------------------------------- V-5
    rec("G-V5a", "resolvers", gx(cid, "getent hosts example.com; echo getent_rc=$?; getent ahostsv4 example.com; echo ahosts_rc=$?; "
                                      "dig +time=3 +tries=1 example.com A +short; echo dig_rc=$?; nslookup example.com; echo nslookup_rc=$?", timeout=60)[1])
    rec("G-V5b", "udp_unconnected", J(gx(cid, f"/vfwd udp-client --dst {pub}:7010 --sizes 0,1,1431,4096,4097,59000")[1]))
    rec("G-V5b", "udp_connected", J(gx(cid, f"/vfwd udp-client --dst {pub}:7010 --sizes 59000,0,1,4097 --connect")[1]))
    # ---------------------------------------------------------------- V-4
    rec("G-V4a", "ssh_cmd", sh(f"timeout 30 {SSH} root@{pub} 'echo SSH_CONNECTION=$SSH_CONNECTION; id; /usr/sbin/ip -4 -br addr'", check=False))
    rec("G-V4a", "ssh_pty", sh(f"timeout 30 {SSH} -tt root@{pub} 'tty; echo PTY_OK; exit' </dev/null", check=False))
    blob8 = H.OUT / "blob8.bin"
    blob8.write_bytes(os.urandom(8 * 1024 * 1024))
    rec("G-V4b", "blob8_sha_host", H.sha(blob8))
    rec("G-V4b", "scp_up", sh(f"timeout 120 {SCP} {blob8} root@{pub}:/tmp/blob8.bin && echo UP_OK", check=False))
    rec("G-V4b", "guest_sha", gx(cid, "sha256sum /tmp/blob8.bin")[1])
    down = H.OUT / "blob8.down"
    rec("G-V4b", "scp_down", sh(f"rm -f {down}; timeout 120 {SCP} root@{pub}:/tmp/blob8.bin {down} && sha256sum {down}", check=False))
    rec("G-V4a", "sshd_log", gx(cid, "cat /tmp/sshd.log")[1])
    # ---------------------------------------------------------------- kernel path proof
    gx(cid, f"nohup strace -f -qq -yy -s 0 -e trace={STRACE} -p $(cat /tmp/owner.pid) -o /tmp/strace-owner.txt "
            f">/dev/null 2>&1 </dev/null & echo $! > /tmp/strace.pid; sleep 1; cat /tmp/strace.pid")
    hpid = H.CTX["host_owner"].pid
    hst = subprocess.Popen(["strace", "-f", "-qq", "-yy", "-s", "0", "-e", f"trace={STRACE}", "-p", str(hpid),
                            "-o", str(H.EV / "strace-host-owner.txt")], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    kp = {}
    kp["tcp_64mib"] = J(gx(cid, f"/vfwd tcp-client --dst {pub}:7000 --size 67108864", timeout=300)[1])
    kp["dns"] = gx(cid, "getent hosts example.com")[1]
    kp["udp"] = J(gx(cid, f"/vfwd udp-client --dst {pub}:7010 --sizes 0,1431,59000")[1])
    kp["scp_up"] = sh(f"timeout 120 {SCP} {blob8} root@{pub}:/tmp/blob8b.bin && echo UP_OK", check=False)
    kp["inbound_64k"] = sh(f"timeout 30 {V} tcp-client --dst {pub}:2710 --size 65536", check=False)
    time.sleep(1)
    hst.send_signal(signal.SIGINT)
    hst.wait(timeout=20)
    gx(cid, "kill -INT $(cat /tmp/strace.pid); sleep 1")
    gtxt = H.gsave("/tmp/strace-owner.txt", "strace-guest-owner.txt")
    htxt = (H.EV / "strace-host-owner.txt").read_text(errors="replace")
    kp["guest_syscall_bytes_by_call_and_fd"] = H.strace_summary(gtxt)
    kp["host_syscall_bytes_by_call_and_fd"] = H.strace_summary(htxt)
    kp["guest_max_single_io"] = max([int(m.group(5)) for m in map(H.RE_LINE.match, gtxt.splitlines()) if m] or [0])
    kp["host_max_single_io"] = max([int(m.group(5)) for m in map(H.RE_LINE.match, htxt.splitlines()) if m] or [0])
    kp["guest_stats"] = gx(cid, "cat /tmp/owner-stats.json")[1]
    kp["host_stats"] = H.host_stats()
    rec("G-KP", "result", kp)
    # ---------------------------------------------------------------- V-3 stresses (10,000 each)
    st = {}
    st["egress_early_fin_10000"] = J(gx(cid, f"/vfwd tcp-stress --dst {pub}:7000 --iterations 10000 --max-size 4096", timeout=1500)[1], "tcp_stress")
    st["egress_early_fin_10000_par8"] = J(gx(cid, f"/vfwd tcp-stress --dst {pub}:7000 --iterations 10000 --max-size 4096 --parallel 8", timeout=1500)[1], "tcp_stress")
    st["egress_server_first_10000"] = J(gx(cid, f"/vfwd tcp-stress --dst {pub}:7001 --iterations 10000 --max-size 4096 --banner", timeout=1500)[1], "tcp_stress")
    st["egress_no_fin_early_ack_10000"] = J(gx(cid, f"/vfwd tcp-stress --dst {pub}:7004 --iterations 10000 --max-size 4096 --early-ack", timeout=1500)[1], "tcp_stress")
    st["inbound_server_first_10000"] = J(sh(f"timeout 1500 {V} tcp-stress --dst {pub}:2711 --iterations 10000 --banner --max-size 4096", check=False, timeout=1600), "tcp_stress")
    st["inbound_client_writes_immediately_10000"] = J(sh(f"timeout 1500 {V} tcp-stress --dst {pub}:2710 --iterations 10000 --max-size 4096", check=False, timeout=1600), "tcp_stress")
    st["udp_first_datagram_unconnected_10000"] = J(gx(cid, f"/vfwd udp-stress --dst {pub}:7010 --iterations 10000 --size 64", timeout=1500)[1], "udp_stress")
    st["udp_first_datagram_connected_2000"] = J(gx(cid, f"/vfwd udp-stress --dst {pub}:7010 --iterations 2000 --size 64 --connect", timeout=1500)[1], "udp_stress")
    st["dns_getent_200"] = gx(cid, "ok=0; for i in $(seq 200); do getent hosts example.com >/dev/null && ok=$((ok+1)); done; echo dns_ok=$ok/200", timeout=600)[1]
    rec("G-V3", "stresses", st)
    time.sleep(2)
    rec("G-V3-5", "guest_ss", gx(cid, "/usr/bin/ss -tna; /usr/bin/ss -una")[1])
    rec("G-V3-5", "host_ss", sh(f"ss -tna | awk 'NR==1 || /{pub}:(70|22|27)/ || /127.0.0.1:/' | head -80", check=False, quiet=True))
    rec("G-V3-5", "guest_stats", gx(cid, "cat /tmp/owner-stats.json")[1])
    rec("G-V3-5", "host_stats", H.host_stats())
    H.gsave("/tmp/owner.log", "guest-owner-g.log")


H.run_increment(probes, OOB)

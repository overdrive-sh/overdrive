#!/usr/bin/env python3
"""Probe library for the V-11 / VIP / V-14 spike (netns-density-295). Imported by
increment-*/run.py, which runs under `cargo xtask metal run` as root on the metal host.

Topology (host side, all created and removed by this file):
  * netns `v11p` behind veth `v11h` (10.250.95.1/24, MTU 1500) <-> `v11p0` (10.250.95.2/24).
    The guest's datagrams to 10.250.95.2 leave the host's root namespace through the real
    egress interface `v11h` -- the off-host stand-in (a second machine was not available).
  * The physical NIC (route to 1.1.1.1) carries real Internet DNS and TEST-NET wire probes.
  * Mock VIPs 10.251.0.10 / 10.251.0.11 exist only in the owner's VIP map (ADR-0053 layout).
"""
import json, os, re, signal, socket, struct, subprocess, threading, time
import harness as H

NS = "v11p"
HV, PV = "v11h", "v11p0"
HOST_NS_IP, PEER = "10.250.95.1", "10.250.95.2"
VIP, VIP2 = "10.251.0.10", "10.251.0.11"
TESTNET = "192.0.2.77"
SIZES = [0, 1, 1431, 4096, 4097, 59000]
OOB = ["--ack", "oob", "--intake-mode", "park", "--arm", "yes", "--flush", "yes", "--udp", "park"]
DECLARE = ["--declare", "7300,7301,7302"]
STRACE = "read,write,readv,writev,recvfrom,sendto,recvmsg,sendmsg,recvmmsg,sendmmsg,splice,sendfile,pread64,pwrite64"


def J(text, ev=None):
    return H.last_json(text, ev)


# ------------------------------------------------------------------ fixtures
def phys_dev():
    return H.sh("ip -4 route get 1.1.1.1 | sed -n 's/.* dev \\([^ ]*\\).*/\\1/p'", quiet=True).strip()


def setup_offhost():
    sh = H.sh
    for cmd in [f"ip netns add {NS}", f"ip link add {HV} type veth peer name {PV}", f"ip link set {PV} netns {NS}",
                f"ip addr add {HOST_NS_IP}/24 dev {HV}", f"ip link set {HV} mtu 1500 up",
                f"ip -n {NS} addr add {PEER}/24 dev {PV}", f"ip -n {NS} link set {PV} mtu 1500 up",
                f"ip -n {NS} link set lo up", f"ip -n {NS} route add default via {HOST_NS_IP}"]:
        sh(cmd)
    H.CLEANUPS.append(teardown_offhost)
    V = str(H.CTX["vfwd"])
    H.spawn("ns-udp7210", ["ip", "netns", "exec", NS, V, "udp-server", "--bind", f"{PEER}:7210", "--mark-len", "777"])
    H.spawn("ns-tcp7200", ["ip", "netns", "exec", NS, V, "tcp-server", "--bind", f"{PEER}:7200"])
    H.spawn("ns-tcp7201", ["ip", "netns", "exec", NS, V, "tcp-server", "--bind", f"{PEER}:7201", "--banner"])
    time.sleep(0.5)
    return sh(f"ip -d link show {HV}; ip -n {NS} -br addr; ip -n {NS} route; ethtool -k {HV} | grep -E 'udp|frag|segment'; "
              f"ip netns exec {NS} ss -lunt", check=False)


def teardown_offhost():
    subprocess.run(["ip", "netns", "exec", NS, "pkill", "-f", "vfwd"], capture_output=True)
    for p in H.PROCS:
        if getattr(p, "name", "").startswith("ns-") and p.poll() is None:
            p.kill()
            p.wait()
    subprocess.run(["ip", "link", "del", HV], capture_output=True)
    subprocess.run(["ip", "netns", "del", NS], capture_output=True)


def bpf_stats(on):
    H.sh(f"sysctl -w kernel.bpf_stats_enabled={1 if on else 0}", check=False, quiet=True)


def tcx_prog_stats(dev):
    """run_cnt / run_time_ns of every TCX program on dev (needs bpf_stats_enabled=1)."""
    out = []
    try:
        net = json.loads(H.sh(f"bpftool -j net show dev {dev}", check=False, quiet=True) or "[]")
    except Exception:
        return {"err": "bpftool net"}
    ids = set()
    for blk in net:
        for k in ("tc", "tcx"):
            for e in blk.get(k, []) or []:
                if "prog_id" in e:
                    ids.add(e["prog_id"])
                elif "id" in e:
                    ids.add(e["id"])
    for i in sorted(ids):
        try:
            j = json.loads(H.sh(f"bpftool -j prog show id {i}", check=False, quiet=True))
            out.append({k: j.get(k) for k in ("id", "name", "run_cnt", "run_time_ns", "xlated_prog_len", "jited_prog_len")})
        except Exception:
            pass
    return out


class Capture:
    """tcpdump on `dev`, started before and stopped after a guest send."""

    def __init__(self, dev, flt, tag):
        self.path = H.EV / f"pcap-{tag}.txt"
        self.p = subprocess.Popen(["tcpdump", "-i", dev, "-nn", "-v", "-l", "-c", "40", flt],
                                  stdout=open(self.path, "w"), stderr=subprocess.STDOUT)
        time.sleep(1.0)

    def stop(self):
        time.sleep(0.8)
        if self.p.poll() is None:
            self.p.send_signal(signal.SIGINT)
            try:
                self.p.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.p.kill()
        return self.path.read_text(errors="replace")


def host_udp_unrelated(frame_like):
    """A host process that is NOT the owner sends to the netns echo server from an
    unregistered socket. With frame_like=True its payload is itself a valid frame: a
    tuple-gated unframe must leave it untouched."""
    body = b"hello"
    payload = (b"ZUD1" + struct.pack(">I", len(body)) + body) if frame_like else body
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.settimeout(2)
    s.bind((HOST_NS_IP, 0))
    try:
        s.sendto(payload, (PEER, 7210))
        data, frm = s.recvfrom(65536)
        want = bytes(b ^ 0xA5 for b in payload)
        return {"sent": len(payload), "got": len(data), "intact": data == want, "from": f"{frm[0]}:{frm[1]}"}
    except Exception as e:
        return {"sent": len(payload), "err": repr(e)}
    finally:
        s.close()


# ------------------------------------------------------------------ V-11
def udp_matrix(cid, dst, sizes=SIZES, timeout_ms=2000):
    """Each size on a FRESH socket (first datagram of a new association), unconnected and
    connected, so one failing size cannot poison the others."""
    res = {}
    for connected in (False, True):
        for n in sizes:
            flag = "--connect" if connected else ""
            rc, t = H.gx(cid, f"/vfwd udp-client --dst {dst} --sizes {n} --timeout-ms {timeout_ms} {flag}", quiet=True)
            j = J(t, "udp_client") or {"raw": t[-300:]}
            r = (j.get("results") or [{}])[0]
            res[f"{'conn' if connected else 'unconn'}_{n}"] = {"ok": j.get("ok"), "got": r.get("got"), "from": r.get("from"),
                                                              "err": r.get("err"), "local": j.get("local"), "peer": j.get("peer")}
    return res


def udp_same_assoc(cid, dst, sizes=SIZES, connected=False):
    flag = "--connect" if connected else ""
    rc, t = H.gx(cid, f"/vfwd udp-client --dst {dst} --sizes {','.join(map(str, sizes))} --timeout-ms 2000 {flag}", quiet=True)
    return J(t, "udp_client") or {"raw": t[-500:]}


def wire_probe(cid, dev, dst_ip, tag, sizes=(0, 1431, 4096)):
    out = {}
    for n in sizes:
        cap = Capture(dev, f"udp and dst host {dst_ip}", f"{tag}-{n}")
        rc, t = H.gx(cid, f"/vfwd udp-send --dst {dst_ip}:9 --size {n} --count 2", quiet=True)
        out[str(n)] = {"send": J(t, "udp_send"), "tcpdump": cap.stop()[-3000:]}
    return out


def counters_delta(before, after):
    if not before or not after:
        return None
    b, a = before.get("counters", []), after.get("counters", [])
    names = {9: "helper_fail", 10: "bad_frame", 11: "redirect_fail", 12: "encoded", 13: "tc_decoded",
             14: "encoded_empty", 15: "tc_decoded_empty", 16: "redirect_ok", 20: "tc_err", 23: "zero_len_drop",
             28: "vstrip", 29: "vstrip_empty_drop", 30: "vstrip_empty_fwd", 31: "hybrid_empty_framed",
             32: "frag_first", 33: "frag_shift", 34: "vip_hit", 37: "tc_err_frag", 38: "hybrid_escaped",
             39: "tc_lenient_pass", 22: "sent_calls", 35: "listen_ev", 36: "listen_ev_lost"}
    return {names.get(i, str(i)): a[i] - (b[i] if i < len(b) else 0) for i in range(len(a)) if a[i] != (b[i] if i < len(b) else 0)}


def strace_start(cid, tag):
    H.gx(cid, f"strace -f -qq -yy -s 0 -e trace={STRACE} -p $(cat /tmp/owner.pid) -o /tmp/strace-{tag}.txt "
              f">/tmp/strace.err 2>&1 </dev/null & echo $! > /tmp/strace.pid; sleep 1; cat /tmp/strace.pid /tmp/strace.err", quiet=True)
    hst = subprocess.Popen(["strace", "-f", "-qq", "-yy", "-s", "0", "-e", f"trace={STRACE}", "-p", str(H.CTX["host_owner"].pid),
                            "-o", str(H.EV / f"strace-host-owner-{tag}.txt")], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    return hst


def strace_stop(cid, hst, tag):
    time.sleep(1)
    hst.send_signal(signal.SIGINT)
    hst.wait(timeout=20)
    H.gx(cid, "kill -INT $(cat /tmp/strace.pid); sleep 1", quiet=True)
    gtxt = H.gsave(f"/tmp/strace-{tag}.txt", f"strace-guest-owner-{tag}.txt")
    htxt = (H.EV / f"strace-host-owner-{tag}.txt").read_text(errors="replace")
    kp = {}
    for side, txt in [("guest", gtxt), ("host", htxt)]:
        kp[f"{side}_syscall_bytes"] = H.strace_summary(txt)
        ios = [int(m.group(5)) for m in map(H.RE_LINE.match, txt.splitlines()) if m and "socket" in m.group(4)]
        kp[f"{side}_socket_io_count"] = len(ios)
        kp[f"{side}_socket_io_max_bytes"] = max(ios or [0])
        kp[f"{side}_socket_io_sizes"] = sorted(set(ios))
    return kp


# ------------------------------------------------------------------ V-14
def probe(port, timeout=0.5):
    """The probe runner's TCP mechanic: connect() succeeds -> pass."""
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(timeout)
    try:
        s.connect((H.CTX["pub"], port))
        return "ok"
    except ConnectionRefusedError:
        return "refused"
    except socket.timeout:
        return "timeout"
    except OSError as e:
        return f"err:{e.errno}"
    finally:
        s.close()


def probe_read(port, timeout=2.0):
    """Connect and read the guest application's greeting end to end."""
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(timeout)
    try:
        s.connect((H.CTX["pub"], port))
        data = b""
        while True:
            c = s.recv(64)
            if not c:
                break
            data += c
        return {"connect": "ok", "read": data.decode(errors="replace")}
    except ConnectionRefusedError:
        return {"connect": "refused"}
    except Exception as e:
        return {"connect": "ok?", "err": repr(e), "read_so_far": None}
    finally:
        s.close()


def wait_for(port, want, deadline_s=5.0, interval=0.001):
    """Poll until probe(port) == want ('ok' or 'refused'); return (elapsed_s, attempts, last)."""
    t0 = time.monotonic()
    n = 0
    last = None
    while time.monotonic() - t0 < deadline_s:
        n += 1
        last = probe(port, timeout=0.3)
        if last == want:
            return time.monotonic() - t0, n, last
        time.sleep(interval)
    return None, n, last


def pct(xs, q):
    xs = sorted(x for x in xs if x is not None)
    if not xs:
        return None
    return xs[min(len(xs) - 1, int(round(q * (len(xs) - 1))))]


def tc_cost(dev, n=20000):
    """Per-run cost of the TCX unframe on `dev` for traffic that is NOT registered: a host
    process (not the owner) sends n small datagrams through `dev` (needs bpf_stats_enabled)."""
    def tot(st):
        return sum((x.get("run_cnt") or 0) for x in st), sum((x.get("run_time_ns") or 0) for x in st)
    b = tot(tcx_prog_stats(dev))
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind((HOST_NS_IP, 0))
    t0 = time.monotonic()
    for i in range(n):
        s.sendto(b"x" * 32, (PEER, 7299))
    wall = time.monotonic() - t0
    s.close()
    time.sleep(0.5)
    a = tot(tcx_prog_stats(dev))
    dc, dt = a[0] - b[0], a[1] - b[1]
    return {"sent": n, "run_cnt_delta": dc, "run_time_ns_delta": dt, "ns_per_run": (dt / dc) if dc else None, "send_wall_s": round(wall, 3)}


def framelike(cid, dst):
    return {c: J(H.gx(cid, f"/vfwd udp-framelike --dst {dst} {'--connect' if c == 'conn' else ''}", quiet=True)[1], "udp_framelike")
            for c in ("unconn", "conn")}

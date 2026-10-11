#!/usr/bin/env python3
"""Probe library for V-18 (netns-density-295). Imported by increment-*/run.py, which runs as root on the
native metal host (never via `cargo xtask metal run` sync; see launch.sh).

Topology (all created and removed by this file, every name prefixed v18-):
  netns `v18-p` behind veth `v29-h` (10.250.229.1/24, MTU 1500) <-> `v29-p0` (10.250.229.2/24).
  The guest's datagrams to 10.250.229.2 leave the host root namespace through the egress interface `v29-h`
  and are IP-fragmented to the 1500-byte MTU, exactly as a NIC would see them.
  The peer (vfwd v29-peer) in the namespace logs {len, sha256} of every datagram it RECEIVES.
"""
import hashlib, json, os, random, signal, socket, struct, subprocess, time
import harness as H

NS = "v29-p"
HV, PV = "v29-h", "v29-p0"
HOST_NS_IP, PEER = "10.250.229.1", "10.250.229.2"
PEER_PORT = 7229
OOB = ["--ack", "oob", "--intake-mode", "park", "--arm", "yes", "--flush", "yes", "--udp", "park"]
DECLARE = ["--declare", "7300,7301,7302"]
MODE = OOB + DECLARE
MAGIC = b"OVD1"


def J(text, ev=None):
    return H.last_json(text, ev)


# ------------------------------------------------------------------ fixtures
def setup_offhost():
    for cmd in [f"ip netns add {NS}", f"ip link add {HV} type veth peer name {PV}", f"ip link set {PV} netns {NS}",
                f"ip addr add {HOST_NS_IP}/24 dev {HV}", f"ip link set {HV} mtu 1500 up",
                f"ip -n {NS} addr add {PEER}/24 dev {PV}", f"ip -n {NS} link set {PV} mtu 1500 up",
                f"ip -n {NS} link set lo up",
                f"ip netns exec {NS} sysctl -w net.core.rmem_max=33554432"]:
        H.sh(cmd)
    H.CLEANUPS.append(teardown_offhost)
    V = str(H.CTX["vfwd"])
    start_peer()
    return H.sh(f"ip -d link show {HV}; ip -n {NS} -br addr; ethtool -k {HV} | grep -E 'udp|frag|segment|generic-receive|tcp-seg'; "
                f"ip netns exec {NS} ss -lun", check=False)


def start_peer():
    V = str(H.CTX["vfwd"])
    p = H.spawn(f"ns-peer-{len(H.PROCS)}", ["ip", "netns", "exec", NS, V, "v29-peer", "--bind", f"{PEER}:{PEER_PORT}", "--sha"])
    H.CTX["peer"] = p
    time.sleep(0.5)
    return p


def teardown_offhost():
    # only the processes this run started (a global pkill would hit concurrent spikes)
    for p in H.PROCS:
        if getattr(p, "name", "").startswith("ns-") and p.poll() is None:
            p.kill()
            p.wait()
    subprocess.run(["ip", "link", "del", HV], capture_output=True)
    subprocess.run(["ip", "netns", "del", NS], capture_output=True)


def peer_log_path():
    return H.CTX["peer"].logpath


def peer_offset():
    try:
        return peer_log_path().stat().st_size
    except FileNotFoundError:
        return 0


def peer_rx_since(off):
    time.sleep(0.4)
    rows = []
    with open(peer_log_path(), "rb") as f:
        f.seek(off)
        for line in f.read().decode(errors="replace").splitlines():
            line = line.strip()
            if line.startswith("{"):
                try:
                    j = json.loads(line)
                except Exception:
                    continue
                if j.get("ev") == "rx":
                    rows.append(j)
    return rows


# ------------------------------------------------------------------ guest sender runs + oracle
def sender(cid, tag, kinds, sizes, reps=1, connect=False, seq=False, threads=1, random_n=0, seed=1, timeout_ms=2000, extra=""):
    """Run vfwd v29-send in the guest; keep every JSON line verbatim in evidence; return (sent rows, done row)."""
    cmd = (f"/vfwd v29-send --dst {PEER}:{PEER_PORT} --kinds {','.join(kinds)} --sizes {','.join(map(str, sizes))} "
           f"--reps {reps} --seed {seed} --threads {threads} --timeout-ms {timeout_ms} "
           + (f"--random {random_n} " if random_n else "") + ("--connect " if connect else "") + ("--seq " if seq else "") + extra)
    rows = []
    path = H.EV / f"sender-{tag}.jsonl"
    f = path.open("w")

    def on_line(line, t):
        f.write(line + "\n")
        j = None
        if line.startswith("{"):
            try:
                j = json.loads(line)
            except Exception:
                pass
        if j:
            rows.append(j)

    rc, lines = H.gx_stream(H.CTX["cid"], cmd, on_line, timeout=1800)
    f.close()
    sent = [r for r in rows if r.get("ev") == "v18"]
    done = next((r for r in rows if r.get("ev") == "v18_done"), None)
    return sent, done, rc, [l for l in lines if not l.startswith("{")]


def judge(tag, sent, rx):
    """Byte-exact oracle: every datagram the guest application sent must have been RECEIVED by the peer with
    the same length and SHA-256, and nothing else may have been received."""
    from collections import Counter
    s = Counter((r["len"], r["sha"]) for r in sent)
    g = Counter((r["len"], r["sha"]) for r in rx)
    missing = s - g
    extra = g - s
    by_key = {(r["len"], r["sha"]): r for r in sent}
    miss_rows = [{"kind": by_key[k]["kind"], "len": k[0], "conn": by_key[k]["conn"], "shaped": by_key[k]["shaped"], "n": v} for k, v in missing.items()]
    extra_rows = [{"len": k[0], "sha": k[1][:16], "n": v} for k, v in extra.items()]
    # near-miss diagnosis for an 'extra': same prefix length +8 (frame leaked) or -8 (over-stripped)
    diag = []
    for r in extra_rows:
        for m in miss_rows:
            if r["len"] - m["len"] in (8, -8):
                diag.append({"extra_len": r["len"], "sent_len": m["len"], "delta": r["len"] - m["len"], "kind": m["kind"]})
                break
    ack_fail = [r for r in sent if not r.get("ok")]
    by_kind = {}
    for r in sent:
        d = by_kind.setdefault(r["kind"], {"sent": 0, "ok": 0})
        d["sent"] += 1
        d["ok"] += 1 if r.get("ok") else 0
    res = {"tag": tag, "sent": len(sent), "sender_ack_ok": len(sent) - len(ack_fail), "peer_rx": len(rx),
           "missing_at_peer": sum(missing.values()), "extra_at_peer": sum(extra.values()),
           "shaped_sent": sum(1 for r in sent if r["shaped"]), "max_len": max([r["len"] for r in sent] or [0]),
           "above_mtu_sent": sum(1 for r in sent if r["len"] > 1472), "by_kind": by_kind,
           "missing_examples": miss_rows[:12], "extra_examples": extra_rows[:12], "extra_diag": diag[:12],
           "ack_fail_examples": [{k: r.get(k) for k in ("kind", "len", "conn", "err", "ack_len", "ack_sha_match")} for r in ack_fail[:12]]}
    res["PASS"] = (res["missing_at_peer"] == 0 and res["extra_at_peer"] == 0 and not ack_fail and len(sent) > 0)
    (H.EV / f"judge-{tag}.json").write_text(json.dumps(res, indent=2, sort_keys=True))
    return res


def guest_stats():
    rc, txt = H.gx(H.CTX["cid"], "cat /tmp/owner-stats.json", quiet=True)
    j = J(txt)
    return j


def guest_delta(b, a):
    if not b or not a:
        return None
    names = {1: "udp_capture", 2: "udp_noslot", 3: "peer_rewrite", 4: "recv_rewrite", 8: "noroute", 9: "helper_fail", 10: "bad_frame",
             11: "redirect_fail", 12: "encoded", 17: "strp_frames", 18: "release", 21: "tc_apport", 23: "zero_len_drop", 24: "parked",
             25: "park_empty", 13: "tc_decoded"}
    bc, ac = b.get("counters", []), a.get("counters", [])
    return {names[i]: ac[i] - (bc[i] if i < len(bc) else 0) for i in range(len(ac)) if i in names and ac[i] != (bc[i] if i < len(bc) else 0)}


def run_phase(tag, kinds, sizes, pcap_on_fail=False, **kw):
    """sender + peer-side independent record + judge + host/guest counter deltas."""
    time.sleep(1.2)  # the owners write their stats every 500 ms
    cap = Capture(HV, f"phase-{tag}") if pcap_on_fail else None
    off = peer_offset()
    b = H.host_stats()
    gb = guest_stats()
    sb = peer_ns_snmp()
    sent, done, rc, other = sender(H.CTX["cid"], tag, kinds, sizes, **kw)
    rx = peer_rx_since(off)
    time.sleep(1.2)
    a = H.host_stats()
    ga = guest_stats()
    sa = peer_ns_snmp()
    res = judge(tag, sent, rx)
    if cap:
        path = cap.stop()
        if res["PASS"]:
            path.unlink()
        else:
            res["pcap"] = path.name
    res["done"] = done
    res["rc"] = rc
    res["non_json_output"] = other[-5:]
    res["counters"] = counters_delta(b, a)
    res["guest_counters"] = guest_delta(gb, ga)
    res["peer_ns_snmp_delta"] = snmp_delta(sb, sa)
    res["peer_log_bytes"] = [off, peer_offset()]
    esc = (res["counters"] or {}).get("hybrid_escaped", 0)
    res["escape_counter_equals_shaped_sent"] = (esc == res["shaped_sent"])
    H.log(f"PHASE {tag}: PASS={res['PASS']} sent={res['sent']} ack_ok={res['sender_ack_ok']} peer_rx={res['peer_rx']} "
          f"missing={res['missing_at_peer']} extra={res['extra_at_peer']} shaped={res['shaped_sent']} escaped_ctr={esc} counters={res['counters']} "
          f"guest={res['guest_counters']} peer_snmp={res['peer_ns_snmp_delta']}")
    return res, sent, rx


def counters_delta(before, after):
    if not before or not after:
        return None
    b, a = before.get("counters", []), after.get("counters", [])
    names = {8: "noroute", 9: "helper_fail", 10: "bad_frame", 11: "redirect_fail", 12: "encoded", 13: "tc_decoded",
             14: "encoded_empty", 15: "tc_decoded_empty", 16: "redirect_ok", 20: "tc_err", 23: "zero_len_drop",
             28: "vstrip", 29: "vstrip_empty_drop", 30: "vstrip_empty_fwd", 31: "hybrid_empty_framed",
             32: "frag_first", 33: "frag_shift", 37: "tc_err_frag", 38: "hybrid_escaped",
             39: "tc_lenient_pass", 22: "sent_calls"}
    return {names.get(i, f"c{i}"): a[i] - (b[i] if i < len(b) else 0) for i in range(len(a)) if a[i] != (b[i] if i < len(b) else 0)}


# ------------------------------------------------------------------ wire capture + independent reassembly
class Capture:
    def __init__(self, dev, tag, snap=0):
        self.path = H.EV / f"pcap-{tag}.pcap"
        self.p = subprocess.Popen(["tcpdump", "-i", dev, "-nn", "-s", str(snap), "-U", "-w", str(self.path), f"dst host {PEER}"],
                                  stdout=subprocess.DEVNULL, stderr=open(H.EV / f"pcap-{tag}.err", "w"))
        time.sleep(1.0)

    def stop(self):
        time.sleep(1.0)
        self.p.send_signal(signal.SIGINT)
        try:
            self.p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.p.kill()
        return self.path


def parse_pcap(path):
    """Return the IPv4/UDP datagrams (fragment trains) on the wire, in order of their first-seen fragment, each
    reassembled from the capture alone. A train ends at its last fragment (MF=0) or when a new offset-0 fragment
    of the same (src, dst, id) arrives, so reused IP ids are separate instances."""
    data = open(path, "rb").read()
    if len(data) < 24:
        return []
    magic = struct.unpack("<I", data[:4])[0]
    assert magic in (0xA1B2C3D4, 0xA1B23C4D), hex(magic)
    off = 24
    instances, cur = [], {}
    while off + 16 <= len(data):
        ts_s, ts_f, incl, orig = struct.unpack("<IIII", data[off:off + 16])
        pkt = data[off + 16:off + 16 + incl]
        off += 16 + incl
        if len(pkt) < 34 or pkt[12:14] != b"\x08\x00":
            continue
        ip = pkt[14:]
        ihl = (ip[0] & 0xf) * 4
        if ip[9] != 17:
            continue
        tot = struct.unpack(">H", ip[2:4])[0]
        ident = struct.unpack(">H", ip[4:6])[0]
        fw = struct.unpack(">H", ip[6:8])[0]
        mf = bool(fw & 0x2000)
        fo = (fw & 0x1fff) * 8
        key = (ip[12:16], ip[16:20], ident)
        payload = ip[ihl:tot]
        flen = tot - ihl
        if key in cur and fo == 0 and any(f["off"] == 0 for f in cur[key]):
            del cur[key]
        if key not in cur:
            cur[key] = []
            instances.append((key, cur[key]))
        cur[key].append({"off": fo, "mf": mf, "data": payload, "flen": flen, "trunc": len(payload) < flen, "iptot": tot, "ihl": ihl,
                         "csum_ok": ip_csum(ip[:ihl]) if len(ip) >= ihl else False})
        if not mf:
            del cur[key]
    out = []
    for key, flist in instances:
        fl = flist  # arrival order is the wire order
        srt = sorted(fl, key=lambda x: x["off"])
        contiguous, pos = True, 0
        for f in srt:
            if f["off"] != pos:
                contiguous = False
            pos = f["off"] + f["flen"]
        complete = contiguous and not srt[-1]["mf"] and srt[0]["off"] == 0
        trunc = any(f["trunc"] for f in srt)
        buf = b"".join(f["data"] for f in srt) if (contiguous and not trunc) else b""
        d = {"id": key[2], "frags": [(f["off"], f["flen"], f["mf"]) for f in fl], "truncated": trunc, "complete": complete,
             "ip_csum_all_ok": all(f["csum_ok"] for f in fl), "udp_len": None, "dport": None, "sport": None}
        first = next((f for f in fl if f["off"] == 0), None)
        if first and len(first["data"]) >= 8:
            sp, dp, ul, _ = struct.unpack(">HHHH", first["data"][:8])
            d.update(sport=sp, dport=dp, udp_len=ul)
        if complete and d["udp_len"] is not None and not trunc:
            d["total_len"] = len(buf)
            d["len_consistent"] = (len(buf) == d["udp_len"])
            body = buf[8:]
            d["payload_len"] = len(body)
            d["sha"] = hashlib.sha256(body).hexdigest()
            d["head"] = body[:8].hex()
        out.append(d)
    return out


def ip_csum(h):
    s = 0
    for i in range(0, len(h), 2):
        s += (h[i] << 8) + h[i + 1]
    while s >> 16:
        s = (s & 0xffff) + (s >> 16)
    return s == 0xffff


# ------------------------------------------------------------------ raw crafting (host, NOT the owner)
def raw_sock():
    s = socket.socket(socket.AF_INET, socket.SOCK_RAW, socket.IPPROTO_RAW)
    s.setsockopt(socket.IPPROTO_IP, socket.IP_HDRINCL, 1)
    return s


def ip_hdr(src, dst, ident, off_units, mf, payload_len):
    fw = (0x2000 if mf else 0) | off_units
    h = struct.pack(">BBHHHBBH4s4s", 0x45, 0, 20 + payload_len, ident, fw, 64, 17, 0, socket.inet_aton(src), socket.inet_aton(dst))
    return h


def send_frag(s, src, dst, ident, off_bytes, mf, data):
    assert off_bytes % 8 == 0
    s.sendto(ip_hdr(src, dst, ident, off_bytes // 8, mf, len(data)) + data, (dst, 0))


def udp_hdr(sport, dport, ulen):
    return struct.pack(">HHHH", sport, dport, ulen, 0)


def peer_ns_snmp():
    t = H.sh(f"ip netns exec {NS} cat /proc/net/snmp", quiet=True, check=False)
    out = {}
    rows = [l.split() for l in t.splitlines()]
    for i in range(0, len(rows) - 1, 2):
        if rows[i] and rows[i][0] == rows[i + 1][0] and rows[i][0] in ("Ip:", "Udp:"):
            for k, v in zip(rows[i][1:], rows[i + 1][1:]):
                out[rows[i][0] + k] = int(v)
    keep = ("Ip:ReasmReqds", "Ip:ReasmOKs", "Ip:ReasmFails", "Ip:ReasmTimeout", "Udp:InDatagrams", "Udp:InErrors", "Udp:RcvbufErrors", "Udp:InCsumErrors")
    return {k: out.get(k) for k in keep}


def snmp_delta(b, a):
    return {k: a[k] - b[k] for k in a if a[k] is not None and b.get(k) is not None and a[k] != b[k]}


def match_wire(sent, wire):
    """Match each sent datagram to the datagram reassembled from the wire capture by (payload length, SHA-256)."""
    idx = {}
    for w in wire:
        if w.get("sha"):
            idx.setdefault((w["payload_len"], w["sha"]), []).append(w)
    found, notfound, layouts = 0, [], []
    for r in sent:
        ws = idx.get((r["len"], r["sha"]))
        if ws:
            found += 1
            w = ws[0]
            layouts.append({"kind": r["kind"], "len": r["len"], "shaped": r["shaped"], "frags": w["frags"],
                            "udp_len": w["udp_len"], "len_consistent": w["len_consistent"], "ip_csum_ok": w["ip_csum_all_ok"]})
        else:
            notfound.append({"kind": r["kind"], "len": r["len"], "shaped": r["shaped"]})
    return {"sent": len(sent), "wire_exact": found, "not_on_wire_exact": notfound[:20], "layouts": layouts,
            "wire_datagrams": len(wire), "wire_incomplete": sum(1 for w in wire if not w["complete"]),
            "wire_bad_len": sum(1 for w in wire if w.get("complete") and not w.get("len_consistent")),
            "wire_bad_ip_csum": sum(1 for w in wire if not w["ip_csum_all_ok"])}


def frags_entries(dev=None):
    """Number of entries in the egress program's fragment-tracking map (FRAGS) of THIS run's owner: found through the
    program attached to our veth (bpftool net -> prog -> map_ids -> map named FRAGS)."""
    dev = dev or HV
    try:
        net = json.loads(H.sh(f"bpftool -j net show dev {dev}", check=False, quiet=True) or "[]")
        pids = set()
        for blk in net:
            for k in ("tc", "tcx"):
                for e in blk.get(k, []) or []:
                    if e.get("name") == "eg_decode_frag" or "prog_id" in e:
                        pids.add(e.get("prog_id", e.get("id")))
        for pid in sorted(x for x in pids if x):
            pj = json.loads(H.sh(f"bpftool -j prog show id {pid}", check=False, quiet=True))
            if pj.get("name") != "eg_decode_frag":
                continue
            for mid in pj.get("map_ids", []):
                mj = json.loads(H.sh(f"bpftool -j map show id {mid}", check=False, quiet=True))
                if mj.get("name") == "FRAGS":
                    dump = json.loads(H.sh(f"bpftool -j map dump id {mid}", check=False, quiet=True) or "[]")
                    return {"map_id": mid, "prog_id": pid, "entries": len(dump), "max_entries": mj.get("max_entries")}
        return {"err": "not found", "net": net}
    except Exception as e:
        return {"err": repr(e)}


# ------------------------------------------------------------------ AF_PACKET crafting (bypasses netfilter)
# Increment-b/c finding: IPPROTO_RAW/IP_HDRINCL fragments traverse the host's netfilter LOCAL_OUT defrag
# (nf_defrag_ipv4, loaded on the metal host), which re-assembles crafted fragments with the same (src,dst,id) into
# one datagram and re-fragments it. Crafted fragments therefore must be injected below netfilter: an AF_PACKET
# socket hands the Ethernet frame to dev_queue_xmit (TC egress still runs, netfilter does not).
def link_mac(dev, ns=None):
    pre = f"ip -n {ns} " if ns else "ip "
    t = H.sh(f"{pre}-o link show {dev}", quiet=True)
    import re as _re
    return bytes.fromhex(_re.search(r"link/ether ([0-9a-f:]{17})", t).group(1).replace(":", ""))


class PacketInjector:
    def __init__(self, dev=HV):
        self.s = socket.socket(socket.AF_PACKET, socket.SOCK_RAW)
        self.s.bind((dev, 0))
        self.src_mac = link_mac(dev)
        self.dst_mac = link_mac(PV, NS)

    @staticmethod
    def _csum(h):
        s = 0
        for i in range(0, len(h), 2):
            s += (h[i] << 8) + h[i + 1]
        while s >> 16:
            s = (s & 0xffff) + (s >> 16)
        return (~s) & 0xffff

    def frag(self, src, dst, ident, off_bytes, mf, data):
        assert off_bytes % 8 == 0
        fw = (0x2000 if mf else 0) | (off_bytes // 8)
        h = struct.pack(">BBHHHBBH4s4s", 0x45, 0, 20 + len(data), ident, fw, 64, 17, 0, socket.inet_aton(src), socket.inet_aton(dst))
        h = h[:10] + struct.pack(">H", self._csum(h)) + h[12:]
        self.s.send(self.dst_mac + self.src_mac + b"\x08\x00" + h + data)

    def close(self):
        self.s.close()

#!/usr/bin/env python3
"""V-29 probe library (netns-density-295): guest->host UDP empty-datagram delivery without the egress TC unframe chain.
Imported by increment-*/run.py (root, on the native metal host, under the canonical lease; see launch.py).
Reuses harness.py / probes.py (V-18 fixtures, renamed v29-*): off-host peer behind veth v29-h <-> v29-p0 in netns v29-p.
"""
import hashlib, json, os, pathlib, re, signal, struct, subprocess, time
import harness as H
import probes as P

HV, PEER, PEER_PORT = P.HV, P.PEER, P.PEER_PORT
VERDICT_NOTIFY = ["--unframe", "verdict-notify"]
HOST_NAMES = {8: "noroute", 9: "helper_fail", 10: "bad_frame", 11: "redirect_fail", 12: "encoded", 13: "tc_decoded",
              14: "encoded_empty", 16: "redirect_ok", 20: "tc_err", 22: "sent_calls", 23: "zero_len_drop", 28: "vstrip",
              29: "vstrip_empty_drop", 30: "vstrip_empty_fwd", 41: "verdict_empty_seen", 42: "notice_ok", 43: "notice_lost",
              44: "sent_err", 45: "sent_zero", 46: "sent_err_last_errno", 49: "verdict_pull_fail", 50: "verdict_nolinear",
              51: "verdict_queued"}


def payload(seq, size):
    b = bytearray(size)
    hdr = b"V29D" + struct.pack(">I", seq)
    for i in range(size):
        b[i] = hdr[i] if i < 8 else (seq * 31 + i * 7) & 0xFF
    return bytes(b)


def sha16(b):
    return hashlib.sha256(b).hexdigest()[:16]


# ------------------------------------------------------------------ counters
def host_snapshot():
    for _ in range(3):
        j = H.host_stats()
        if j:
            return j
        time.sleep(0.2)
    return None


def host_delta(b, a):
    if not b or not a:
        return None
    out = {}
    bc, ac = b.get("counters", []), a.get("counters", [])
    for i in range(len(ac)):
        d = ac[i] - (bc[i] if i < len(bc) else 0)
        if d and i in HOST_NAMES:
            out[HOST_NAMES[i]] = d
    for k, v in a.items():
        if k in ("counters", "t_ms", "flows_open") or not isinstance(v, (int, float)):
            continue
        d = v - b.get(k, 0)
        if d:
            out[k] = d
    return out


def settle():
    time.sleep(1.3)  # owners write stats every 500 ms


def drain(off, quiet_s=3.0, max_s=90):
    """Wait until the peer's arrival log stops growing for quiet_s (the throttled backlog has drained)."""
    t0, last, last_t = time.time(), -1, time.time()
    while time.time() - t0 < max_s:
        sz = P.peer_offset()
        if sz != last:
            last, last_t = sz, time.time()
        elif time.time() - last_t >= quiet_s:
            break
        time.sleep(0.25)
    return round(time.time() - t0, 1)


# ------------------------------------------------------------------ pcap (host egress interface v29-h)
class Cap:
    def __init__(self, tag, snap=96):
        self.path = H.EV / f"pcap-{tag}.pcap"
        self.p = subprocess.Popen(["tcpdump", "-i", HV, "-nn", "-s", str(snap), "-U", "-B", "65536", "-w", str(self.path),
                                   f"udp and dst host {PEER} and dst port {PEER_PORT}"], stdout=subprocess.DEVNULL,
                                  stderr=open(H.EV / f"pcap-{tag}.err", "w"))
        time.sleep(1.0)

    def stop(self):
        time.sleep(1.0)
        self.p.send_signal(signal.SIGINT)
        try:
            self.p.wait(timeout=15)
        except subprocess.TimeoutExpired:
            self.p.kill()
        err = (H.EV / self.path.name.replace(".pcap", ".err")).read_text()
        return self.path, err


def wire_tokens(path):
    """Wire order of the UDP datagrams the host emitted to the peer: ('E',) empty (UDP length 8),
    ('D', seq, udp_payload_len), ('X', udp_payload_len, head_hex) for a datagram not starting with V29D.
    Fragments (IP MF/offset != 0) are folded into their first fragment (only the first carries the UDP header)."""
    data = open(path, "rb").read()
    if len(data) < 24:
        return [], {"packets": 0}
    magic = struct.unpack("<I", data[:4])[0]
    nano = magic == 0xA1B23C4D
    off, toks, pk, frag_more, later_frag = 24, [], 0, 0, 0
    while off + 16 <= len(data):
        ts_s, ts_f, incl, orig = struct.unpack("<IIII", data[off:off + 16])
        pkt = data[off + 16:off + 16 + incl]
        off += 16 + incl
        pk += 1
        if len(pkt) < 34 or pkt[12:14] != b"\x08\x00":
            continue
        ip = pkt[14:]
        ihl = (ip[0] & 0xF) * 4
        if ip[9] != 17:
            continue
        fw = struct.unpack(">H", ip[6:8])[0]
        fo = (fw & 0x1FFF) * 8
        if fw & 0x2000:
            frag_more += 1
        if fo != 0:
            later_frag += 1
            continue
        u = ip[ihl:]
        if len(u) < 8:
            continue
        ulen = struct.unpack(">H", u[4:6])[0]
        body = u[8:]
        t = ts_s + ts_f / (1e9 if nano else 1e6)
        if ulen == 8:
            toks.append(("E", t))
        elif body[:8] == b"V29D\xff\xff\xff\xff":
            continue  # warm-up datagram
        elif body[:4] == b"V29D" and len(body) >= 8:
            toks.append(("D", struct.unpack(">I", body[4:8])[0], ulen - 8, t))
        else:
            toks.append(("X", ulen - 8, body[:8].hex(), t))
    return toks, {"packets": pk, "datagrams": len(toks), "ip_more_fragments_flagged": frag_more, "later_fragments": later_frag}


def rx_tokens(rx):
    toks = []
    for r in rx:
        if r["head"].startswith("56323944ffffffff"):
            continue  # warm-up datagram
        if r["len"] == 0:
            toks.append(("E", r["t_ns"] / 1e9))
        elif r["head"].startswith("56323944") and len(r["head"]) >= 16:
            toks.append(("D", int(r["head"][8:16], 16), r["len"], r["t_ns"] / 1e9, r.get("sha", "")))
        else:
            toks.append(("X", r["len"], r["head"], r["t_ns"] / 1e9))
    return toks


# ------------------------------------------------------------------ sender (guest) + analysis
def sender(tag, args, timeout=900):
    cmd = f"/vfwd v29-send --dst {PEER}:{PEER_PORT} {args}"
    rows = []
    f = (H.EV / f"sender-{tag}.jsonl").open("w")

    def on_line(line, t):
        f.write(line + "\n")
        if line.startswith("{"):
            try:
                rows.append(json.loads(line))
            except Exception:
                pass

    rc, lines = H.gx_stream(H.CTX["cid"], cmd, on_line, timeout=timeout)
    f.close()
    done = next((r for r in rows if r.get("ev") == "v29_done"), None)
    sent = [r for r in rows if r.get("ev") == "v29_sent"]
    return done, sent, rc


def analyze_seq(toks, n, size, k, lead=0):
    """Pattern: `lead` empties, then data 0..n-1 with an empty after every k-th data. toks = arrival order."""
    data = [t for t in toks if t[0] == "D"]
    empt = [i for i, t in enumerate(toks) if t[0] == "E"]
    other = [t for t in toks if t[0] == "X"]
    seqs = [t[1] for t in data]
    seen = {}
    for s in seqs:
        seen[s] = seen.get(s, 0) + 1
    dup = sum(v - 1 for v in seen.values() if v > 1)
    missing = [s for s in range(n) if s not in seen]
    # data-data order
    ooo = sum(1 for a, b in zip(seqs, seqs[1:]) if b < a)
    pos = {}
    for i, t in enumerate(toks):
        if t[0] == "D":
            pos.setdefault(t[1], i)
    n_empty_sent = lead + (n // k if k > 0 else 0)
    # k-th (0-based) non-lead empty follows data index p=(j+1)*k-1 and precedes data index p+1
    early = late = 0
    early_dist = []
    late_dist = []
    mapped = 0
    ep = empt[lead:]
    for j, ei in enumerate(ep):
        p = (j + 1) * k - 1
        if p >= n:
            continue
        mapped += 1
        # early: the predecessor data had not arrived before this empty
        pred_after = p not in pos or pos[p] > ei
        if pred_after:
            early += 1
            early_dist.append(sum(1 for s, i in pos.items() if s <= p and i > ei))
        # late: the successor data (if any) arrived before this empty
        if p + 1 < n and p + 1 in pos and pos[p + 1] < ei:
            late += 1
            late_dist.append(sum(1 for s, i in pos.items() if s > p and i < ei))
    return {"data_sent": n, "data_rx": len(data), "data_missing": len(missing), "data_missing_examples": missing[:8], "data_dup": dup,
            "data_out_of_order": ooo, "empty_sent": n_empty_sent, "empty_rx": len(empt), "empty_missing": n_empty_sent - len(empt),
            "other_rx": len(other), "other_examples": other[:4],
            "empty_early": early, "empty_late": late, "empties_mapped": mapped,
            "early_rate": round(early / mapped, 6) if mapped else None, "late_rate": round(late / mapped, 6) if mapped else None,
            "early_dist_max": max(early_dist or [0]), "late_dist_max": max(late_dist or [0]),
            "early_dist_mean": round(sum(early_dist) / len(early_dist), 2) if early_dist else 0}


def sha_check(toks, size):
    bad = 0
    n = 0
    for t in toks:
        if t[0] == "D" and len(t) >= 5 and t[4]:
            n += 1
            if t[2] == size and sha16(payload(t[1], size)) != t[4]:
                bad += 1
    return {"checked": n, "bad_sha": bad}


def phase_seq(tag, n, size, k, gap_us=0, burst=1, lead=0, extra="--warm --linger-ms 1000", pcap=True, expect_loss=False, record=True):
    """One ordered phase: guest sends the pattern, the host owner stack forwards, we compare the wire (tcpdump on v29-h)
    and the peer's arrival log, and the host/guest counters."""
    settle()
    cap = Cap(tag) if pcap else None
    off = P.peer_offset()
    hb, gb = host_snapshot(), P.guest_stats()
    sb = P.peer_ns_snmp()
    done, _, rc = sender(tag, f"--n {n} --size {size} --k {k} --gap-us {gap_us} --burst {burst} --lead-empties {lead} {extra}")
    dsec = drain(off)
    rx = P.peer_rx_since(off)
    settle()
    ha, ga = host_snapshot(), P.guest_stats()
    res = {"tag": tag, "params": {"n": n, "size": size, "k": k, "gap_us": gap_us, "burst": burst, "lead": lead, "extra": extra},
           "sender": done, "rc": rc, "drain_wait_s": dsec}
    ptoks = rx_tokens(rx)
    res["peer"] = analyze_seq(ptoks, n, size, k, lead)
    res["peer"].update(sha_check(ptoks, size))
    if cap:
        path, err = cap.stop()
        wtoks, winfo = wire_tokens(path)
        res["wire"] = analyze_seq(wtoks, n, size, k, lead)
        res["wire"]["capture"] = winfo
        res["wire"]["tcpdump_stderr"] = err.strip().splitlines()[-3:]
        res["wire"]["zero_length_datagrams_on_wire"] = sum(1 for t in wtoks if t[0] == "E")
        if res["wire"]["zero_length_datagrams_on_wire"] > 0 or len(wtoks) < 50000:
            pass
        else:
            pass
    res["host_counters"] = host_delta(hb, ha)
    res["guest_counters"] = P.guest_delta(gb, ga)
    res["peer_ns_snmp_delta"] = P.snmp_delta(sb, P.peer_ns_snmp())
    res["qdisc"] = H.sh(f"tc -s qdisc show dev {HV}", quiet=True, check=False).strip().splitlines()[:4]
    if record:
        H.record("phases", tag, res)
    w = res.get("wire", {})
    p = res["peer"]
    H.log(f"PHASE {tag}: wire(E={w.get('empty_rx')}/{p['empty_sent']} early={w.get('empty_early')} late={w.get('empty_late')} "
          f"dmiss={w.get('data_missing')}) peer(E={p['empty_rx']} early={p['empty_early']} late={p['empty_late']} dmiss={p['data_missing']} "
          f"dup={p['data_dup']} ooo={p['data_out_of_order']} badsha={p['bad_sha']}) host={res['host_counters']}")
    return res


def tbf(rate="20mbit", limit=400000, burst=6000):
    H.sh(f"tc qdisc replace dev {HV} root tbf rate {rate} burst {burst} limit {limit}")


def untbf():
    H.sh(f"tc qdisc del dev {HV} root", check=False)


def analyze_sizes(sent, toks):
    """sent: [{'seq','len'}] in send order; toks: arrival tokens. Data datagram `seq` must arrive once, byte-exact, in ONE
    datagram; anything else (a datagram not starting with V29D, or a second one for the same seq) is a split piece."""
    want = {r["seq"]: r["len"] for r in sent if r["len"] >= 8}
    tiny_sent = sorted(r["len"] for r in sent if 0 < r["len"] < 8)
    n_empty = sum(1 for r in sent if r["len"] == 0)
    got = {}
    for t in toks:
        if t[0] == "D":
            got.setdefault(t[1], []).append(t)
    xs = [t for t in toks if t[0] == "X" and t[1] >= 8]
    tiny_rx = sorted(t[1] for t in toks if t[0] == "X" and t[1] < 8)
    per = {}
    for seq, ln in want.items():
        g = got.get(seq, [])
        ok = len(g) == 1 and g[0][2] == ln and (not g[0][4] or sha16(payload(seq, ln)) == g[0][4])
        per.setdefault(ln, {"sent": 0, "exact_single_datagram": 0, "missing": 0, "wrong_len_or_sha": 0, "dup": 0})
        d = per[ln]
        d["sent"] += 1
        if not g:
            d["missing"] += 1
        elif len(g) > 1:
            d["dup"] += 1
        elif ok:
            d["exact_single_datagram"] += 1
        else:
            d["wrong_len_or_sha"] += 1
    # split pieces: sizes of the datagrams that arrive for a seq and the headerless continuation datagrams in between
    pieces = {}
    cur = None
    for t in toks:
        if t[0] == "D":
            cur = t[1]
            pieces.setdefault(cur, [t[2]])
        elif t[0] == "X" and t[1] >= 8 and cur is not None:
            pieces[cur].append(t[1])
    split = {seq: p for seq, p in pieces.items() if len(p) > 1}
    return {"sent_data": len(want), "sent_empty": n_empty, "rx_total": len(toks), "rx_empty": sum(1 for t in toks if t[0] == "E"),
            "tiny_sent": len(tiny_sent), "tiny_rx": len(tiny_rx), "tiny_equal": tiny_sent == tiny_rx, "rx_headerless_X": len(xs), "per_size": dict(sorted(per.items())),
            "split_seqs": len(split), "split_examples": {str(k): v for k, v in list(sorted(split.items()))[:10]}}


def phase_sizes(tag, sizes, reps=1, gap_us=0, extra="", pcap=False, rec=True):
    settle()
    cap = Cap(tag, snap=96) if pcap else None
    off = P.peer_offset()
    hb, gb = host_snapshot(), P.guest_stats()
    sb = P.peer_ns_snmp()
    done, sent, rc = sender(tag, f"--sizes {','.join(map(str, sizes))} --reps {reps} --gap-us {gap_us} --linger-ms 1000 {extra}")
    dsec = drain(off)
    rx = P.peer_rx_since(off)
    settle()
    ha, ga = host_snapshot(), P.guest_stats()
    toks = rx_tokens(rx)
    res = {"tag": tag, "sender": done, "rc": rc, "drain_wait_s": dsec, "peer": analyze_sizes(sent, toks)}
    if cap:
        path, err = cap.stop()
        wtoks, winfo = wire_tokens(path)
        res["wire"] = {"capture": winfo, "empty_on_wire": sum(1 for t in wtoks if t[0] == "E"),
                       "data_on_wire": sum(1 for t in wtoks if t[0] == "D"), "headerless_on_wire": sum(1 for t in wtoks if t[0] == "X")}
    res["host_counters"] = host_delta(hb, ha)
    res["guest_counters"] = P.guest_delta(gb, ga)
    res["peer_ns_snmp_delta"] = P.snmp_delta(sb, P.peer_ns_snmp())
    if rec:
        H.record("phases", tag, res)
    H.log(f"PHASE {tag}: peer sent_data={res['peer']['sent_data']} sent_empty={res['peer']['sent_empty']} rx_empty={res['peer']['rx_empty']} "
          f"tiny_ok={res['peer']['tiny_equal']} X={res['peer']['rx_headerless_X']} split={res['peer']['split_seqs']} host={res['host_counters']}")
    return res

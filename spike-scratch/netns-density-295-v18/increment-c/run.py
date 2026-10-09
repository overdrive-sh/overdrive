#!/usr/bin/env python3
"""increment-c: the V-18 evidence run (increment-b was the development run; its evidence is kept) (findings: docs/feature/netns-density-295/spike/findings-v18.md).

Pre-registered hypotheses (written before this increment ran; see findings for the table):
  H1  frame-shaped payloads (single, nested, chained; all sizes incl. every 8-byte fragment-boundary +-2) arrive
      byte-exact at an off-host peer under hybrid, connected and unconnected, fresh association and same association.
  H2  payloads that are NOT frame-shaped but look like frames (wrong declared length, near-miss magic, chains with a bad
      first block, declared = first-fragment size) arrive byte-exact (the egress program must not strip them).
  H3  holds for concurrent fragmented datagrams of several associations (interleaved fragment trains).
  H4  holds at other egress MTUs (1280, 1400, 1492, 9000): the shift is one 8-byte unit regardless of MTU.
  H5  the wire carries no frame after the egress program (reassembled datagram == application payload), IP header
      checksums valid, UDP length == reassembled length.
  H6  control: without the egress program on the egress interface only empty and frame-shaped payloads arrive 8 bytes
      long (the ADR-0165 degradation); the oracle sees it.
  H7  the fragment-tracking map holds no stale state: a first fragment whose last fragment never passes the program
      does not affect later datagrams. PREDICTION (from reading eg_decode_frag, before running): FALSIFIED -- the map is a
      plain BPF hash with no expiry; an entry is removed only when the train's MF=0 fragment passes the program, so
      (S1) a later unrelated train with the same (src, dst, IP id) has its later fragments shifted (datagram lost, not
      delivered corrupt: UDP length no longer matches); (S2) 4096 stale entries make every later insert fail, so every
      later frame-kept fragmented datagram loses its tail; (S3) such stale entries arise from an egress queue that
      drops a non-last fragment (ip_do_fragment stops sending the rest).
  Extra oracle: the host `hybrid_escaped` counter delta must equal the number of frame-shaped payloads sent.
"""
import json, os, pathlib, struct, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H
import probes as P

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
EG = ["--unframe", "hybrid", "--egress-if", P.HV]
FRAMEY = ["frame", "frame2", "frame3", "frame_chain"]
NEARMISS = ["decl_len", "decl_m7", "decl_m9", "decl_m16", "decl_zero", "decl_max", "decl_59000", "decl_1464", "decl_1472",
            "decl_1480", "decl_p8", "badmagic", "badmagic_case", "magic_only", "nf_chain"]
FIXED = [8, 9, 15, 16, 17, 24, 25, 64, 100, 1431, 2048, 4096, 4097, 8192, 16384, 32768, 58999, 59000]


def sizes_for(mtu, ks=(0, 1, 2, 3, 10, 38), ds=(-2, -1, 0, 1, 2), fixed=FIXED):
    cap = ((mtu - 20) // 8) * 8  # IP payload bytes of a full non-last fragment
    s = set(fixed)
    for k in ks:
        for d in ds:
            s.add(cap - 16 + k * cap + d)  # kept frame: UDP hdr + frame + L == cap + k*cap
            s.add(cap - 8 + k * cap + d)   # bare: UDP hdr + L == cap + k*cap
    return sorted(x for x in s if 8 <= x <= 59000)


def summarize(res):
    return {k: res[k] for k in ("tag", "PASS", "sent", "sender_ack_ok", "peer_rx", "missing_at_peer", "extra_at_peer", "shaped_sent",
                                "above_mtu_sent", "max_len", "counters", "peer_ns_snmp_delta")}


def probes():
    cid = H.CTX["cid"]
    rec = H.record
    rec("substrate", "guest_uname", H.gx(cid, "uname -a")[1])
    rec("substrate", "sha_selftest_host", H.sh(f"{H.CTX['vfwd']} sha-selftest"))
    rec("substrate", "sha_selftest_guest", H.gx(cid, "/vfwd sha-selftest")[1])
    rec("substrate", "offhost", P.setup_offhost())
    H.set_modes(P.MODE, EG)
    rec("programs", "loaded", H.sh(f"grep -o '\"programs\":.*' {H.CTX['host_owner'].owner_log} | head -c 3000", check=False, quiet=True))
    rec("programs", "tcx_on_veth", H.sh(f"bpftool net show dev {P.HV}", check=False, quiet=True))
    S = sizes_for(1500)
    rec("plan", "sizes_1500", S)
    rec("plan", "frame_kinds", FRAMEY)
    rec("plan", "nearmiss_kinds", NEARMISS)
    results = {}

    def phase(tag, kinds, sizes, **kw):
        kw.setdefault("pcap_on_fail", True)
        res, sent, rx = P.run_phase(tag, kinds, sizes, **kw)
        results[tag] = summarize(res)
        results[tag]["escape_counter_equals_shaped_sent"] = res["escape_counter_equals_shaped_sent"]
        results[tag]["guest_counters"] = res.get("guest_counters")
        if res.get("pcap"):
            results[tag]["pcap"] = res["pcap"]
        rec("phases", tag, results[tag])
        return res, sent, rx

    # ---------------------------------------------------------------- H1 frame-shaped payloads
    phase("h1_frame_unconn", FRAMEY, S, reps=2)
    phase("h1_frame_conn", FRAMEY, S, reps=2, connect=True)
    phase("h1_empty_unconn", ["rand"], [0], reps=3)
    phase("h1_empty_conn", ["rand"], [0], reps=3, connect=True)
    seqs = [8, 16, 100, 1464, 1465, 1472, 1473, 1480, 2944, 2952, 4096, 8192, 16384, 59000]
    phase("h1_seq_unconn", ["frame", "rand", "frame2", "nf_chain", "frame_chain"], seqs, seq=True, reps=2)
    phase("h1_seq_conn", ["frame", "rand", "frame2", "nf_chain", "frame_chain"], seqs, seq=True, reps=2, connect=True)
    # ---------------------------------------------------------------- H2 frame-looking but not frame-shaped, and plain
    NM = [8, 16, 17, 1464, 1465, 1472, 1473, 1480, 2944, 2952, 4096, 4097, 59000]
    phase("h2_nearmiss_unconn", NEARMISS, NM, reps=1)
    phase("h2_nearmiss_conn", NEARMISS, NM, reps=1, connect=True)
    phase("h2_plain", ["rand"], [1, 7] + S, reps=1)
    phase("h2_zero_ones", ["zero", "ones"], NM, reps=1)
    # ---------------------------------------------------------------- H3 concurrent fragmented trains (repeated: one loss seen in development)
    BIG = [x for x in S if x >= 1440]
    KC = ["frame", "frame2", "frame_chain", "rand", "nf_chain", "decl_len", "decl_m9"]
    for i in range(3):
        phase(f"h3_conc_unconn_{i}", KC, BIG, threads=4, random_n=250, seed=7 + i)
        phase(f"h3_conc_conn_{i}", KC, BIG, threads=4, random_n=250, seed=19 + i, connect=True)
    # ---------------------------------------------------------------- H5 wire capture and independent reassembly
    cap = P.Capture(P.HV, "wire")
    allsent = []
    for tag, kinds, sizes in [("w_frame", ["frame"], [8, 16, 1464, 1465, 1472, 1473, 1480, 2936, 2944, 2952, 4096, 59000]),
                              ("w_frame2", ["frame2"], [4096, 59000]), ("w_chain", ["frame_chain"], [4096, 59000]),
                              ("w_rand", ["rand"], [1472, 1473, 4096, 59000]), ("w_decl1472", ["decl_1472"], [4096])]:
        res, sent, rx = phase(tag, kinds, sizes, reps=1, pcap_on_fail=False)
        allsent += sent
    path = cap.stop()
    wire = P.parse_pcap(path)
    mw = P.match_wire(allsent, wire)
    rec("H5_wire", "match", mw)
    rec("H5_wire", "layouts_frame_shaped_above_mtu", [l for l in mw["layouts"] if l["shaped"] and l["len"] > 1472])
    # ---------------------------------------------------------------- H4 other MTUs
    mtu_res = {}
    for mtu in (1280, 1400, 1492, 9000):
        H.sh(f"ip link set {P.HV} mtu {mtu}")
        H.sh(f"ip -n {P.NS} link set {P.PV} mtu {mtu}")
        Sm = sizes_for(mtu, ks=(0, 1, 2, 5), ds=(-2, -1, 0, 1, 2), fixed=[8, 16, 4096, 59000])
        phase(f"h4_mtu{mtu}_frame", ["frame", "frame_chain"], Sm, reps=1, pcap_on_fail=False)
        phase(f"h4_mtu{mtu}_plain", ["rand", "nf_chain", "decl_len"], [x for x in Sm if x in (8, 16) or x % 3 == 0] or Sm[:6], reps=1, pcap_on_fail=False)
        mtu_res[mtu] = Sm
    H.sh(f"ip link set {P.HV} mtu 1500")
    H.sh(f"ip -n {P.NS} link set {P.PV} mtu 1500")
    rec("H4_mtu", "sizes", mtu_res)
    # ---------------------------------------------------------------- H6 control: egress program missing on the egress interface
    H.set_modes(P.MODE, ["--unframe", "hybrid"])
    rec("H6", "tcx_on_veth", H.sh(f"bpftool net show dev {P.HV}", check=False, quiet=True))
    r6, _, _ = P.run_phase("h6_missing_tc", ["frame", "rand", "nf_chain"], [8, 100, 1464, 1472, 4096, 59000], reps=1)
    rec("H6", "judge", r6)
    r6e, _, _ = P.run_phase("h6_missing_tc_empty", ["rand"], [0], reps=3)
    rec("H6", "empty", summarize(r6e))
    # ---------------------------------------------------------------- H7 fragment-tracking map state
    import hashlib, random
    H.set_modes(P.MODE, EG)
    s7 = {}
    H.sh(f"ip netns exec {P.NS} sysctl -w net.ipv4.ipfrag_time=2 net.ipv4.ipfrag_high_thresh=134217728 net.ipv4.ipfrag_low_thresh=100663296")

    def holder():
        off = P.peer_offset()
        H.gx(cid, f"/vfwd v18-send --dst {P.PEER}:{P.PEER_PORT} --kinds rand --sizes 64 --reps 1 --connect --hold-ms 180000 > /tmp/hold.out 2>&1 </dev/null & echo started", quiet=True)
        time.sleep(1.5)
        frm = P.peer_rx_since(off)[-1]["from"]
        return int(frm.split(":")[1]), frm

    sport, frm = holder()
    s7["holder_from"] = frm
    s7["frags_entries_start"] = P.frags_entries()
    rs = P.raw_sock()
    # ---- S1
    cap1 = P.Capture(P.HV, "h7-s1")
    ID, L = 0x6e57, 3000
    stageA = P.udp_hdr(sport, P.PEER_PORT, 16 + L) + P.MAGIC + struct.pack(">I", L) + os.urandom(1464)
    P.send_frag(rs, P.HOST_NS_IP, P.PEER, ID, 0, True, stageA)
    time.sleep(4)  # the peer's partial datagram expires (ipfrag_time=2)
    s7["S1_entries_after_stageA"] = P.frags_entries()
    snm0 = P.peer_ns_snmp()

    def victim(ident, sp, tag):
        body = os.urandom(3000)
        d = P.udp_hdr(sp, P.PEER_PORT, 8 + 3000) + body
        o = P.peer_offset()
        P.send_frag(rs, P.HOST_NS_IP, P.PEER, ident, 0, True, d[:1480])
        P.send_frag(rs, P.HOST_NS_IP, P.PEER, ident, 1480, True, d[1480:2960])
        P.send_frag(rs, P.HOST_NS_IP, P.PEER, ident, 2960, False, d[2960:])
        rows = P.peer_rx_since(o)
        sha = hashlib.sha256(body).hexdigest()
        return {"tag": tag, "id": hex(ident), "sport": sp, "received_byte_exact": any(r["sha"] == sha for r in rows),
                "peer_rx_rows": [{"len": r["len"]} for r in rows], "sha": sha}

    s7["S1_victim_same_id"] = victim(ID, 45555, "unrelated bare datagram, same (src,dst,id) as the stale entry")
    s7["S1_control_other_id"] = victim(ID + 1, 45555, "control: id the program never saw")
    s7["S1_victim_again_same_id"] = victim(ID, 45555, "same id again: the previous train's last fragment consumed the entry")
    s7["S1_peer_snmp_delta"] = P.snmp_delta(snm0, P.peer_ns_snmp())
    s7["S1_entries_after"] = P.frags_entries()
    w1 = P.parse_pcap(cap1.stop())
    s7["S1_wire_trains"] = [{"id": hex(w["id"]), "sport": w["sport"], "udp_len": w["udp_len"], "frags": w["frags"]} for w in w1 if w["id"] in (ID, ID + 1)]
    # ---- S2: fill the map with stale entries (random ids), then real traffic
    cap2 = P.Capture(P.HV, "h7-s2", snap=96)
    ids = random.Random(18).sample(range(1, 65536), 4300)
    s7["S2_entries_before_fill"] = P.frags_entries()
    t0 = time.time()
    for i, ident in enumerate(ids):
        hdr = P.udp_hdr(sport, P.PEER_PORT, 16 + 2000) + P.MAGIC + struct.pack(">I", 2000) + os.urandom(1464)
        P.send_frag(rs, P.HOST_NS_IP, P.PEER, ident, 0, True, hdr)
        if i % 128 == 127:
            time.sleep(0.02)
    s7["S2_fill"] = {"sent": len(ids), "wall_s": round(time.time() - t0, 2)}
    time.sleep(4)
    s7["S2_entries_after_fill"] = P.frags_entries()
    stale = set(ids)
    crafted_n = len(ids)
    r_fr, _, _ = P.run_phase("h7_s2_frame_after_fill", ["frame"], [4096], reps=40)
    s7["S2_frame_after_fill"] = summarize(r_fr)
    r_rd, _, _ = P.run_phase("h7_s2_rand_after_fill", ["rand"], [4096], reps=400)
    s7["S2_rand_after_fill"] = summarize(r_rd)
    s7["S2_entries_after_traffic"] = P.frags_entries()
    wire2 = P.parse_pcap(cap2.stop())
    crafted_idx = [i for i, w in enumerate(wire2) if w.get("sport") == sport and w.get("udp_len") == 2016]
    real = [w for w in wire2[(crafted_idx[-1] + 1 if crafted_idx else 0):] if w.get("sport") not in (None, sport) and len(w["frags"]) > 1]
    def shifted(w):  # a bare (unstripped, first fragment full size) train whose later fragments were moved back
        return len(w["frags"]) > 1 and w["frags"][0][1] == 1480 and any(f[0] % 1480 != 0 for f in w["frags"][1:])
    bare = [w for w in real if w["frags"][0][1] == 1480 or shifted(w)]
    s7["S2_wire"] = {"real_fragmented_trains_after_fill": len(real), "bare_trains": len(bare),
                     "bare_with_id_in_stale_set": sum(1 for w in bare if w["id"] in stale),
                     "bare_shifted_on_wire": sum(1 for w in bare if shifted(w)),
                     "bare_shifted_and_id_in_stale_set": sum(1 for w in bare if shifted(w) and w["id"] in stale),
                     "bare_not_in_set_but_shifted": sum(1 for w in bare if shifted(w) and w["id"] not in stale),
                     "example_shifted_layouts": [w["frags"] for w in bare if shifted(w)][:2]}
    time.sleep(40)
    r_40, _, _ = P.run_phase("h7_s2_frame_after_40s", ["frame"], [4096], reps=20)
    s7["S2_frame_after_40s"] = summarize(r_40)
    s7["S2_entries_after_40s"] = P.frags_entries()
    # owner restart clears the program's maps
    H.gx(cid, "pkill -f 'v18-send' ; true", quiet=True)
    H.set_modes(P.MODE, EG)
    r_rs, _, _ = P.run_phase("h7_s2_after_owner_restart", ["rand", "frame"], [4096], reps=40)
    s7["S2_after_owner_restart"] = summarize(r_rs)
    s7["S2_entries_after_restart"] = P.frags_entries()
    # ---- S3: how stale entries arise without crafting: an egress queue that tail-drops a non-last fragment
    sport3, frm3 = holder()
    s7["S3_entries_before"] = P.frags_entries()
    H.sh(f"tc qdisc add dev {P.HV} root tbf rate 4mbit burst 1600 limit 3200")
    r_q, sent_q, rx_q = P.run_phase("h7_s3_congested_frame", ["frame"], [4096], reps=60, timeout_ms=400)
    H.sh(f"tc qdisc del dev {P.HV} root", check=False)
    s7["S3_congested"] = summarize(r_q)
    s7["S3_tc_stats"] = "see commands.jsonl"
    s7["S3_entries_after_congestion"] = P.frags_entries()
    time.sleep(3)
    r_ok, _, _ = P.run_phase("h7_s3_after_congestion", ["frame", "rand"], [4096], reps=20)
    s7["S3_after_congestion"] = summarize(r_ok)
    s7["S3_entries_after_recovery"] = P.frags_entries()
    rec("H7", "result", s7)
    rs.close()
    H.gx(cid, "pkill -f 'v18-send' ; true", quiet=True)
    rec("summary", "all_phases_pass", {k: v["PASS"] for k, v in results.items()})
    rec("summary", "phase_results", results)
    rec("summary", "escape_counter_equals_shaped_sent", {k: v["escape_counter_equals_shaped_sent"] for k, v in results.items()})


H.run_increment(probes, P.MODE)

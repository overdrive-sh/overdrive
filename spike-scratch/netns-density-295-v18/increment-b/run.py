#!/usr/bin/env python3
"""increment-b: the V-18 evidence run (findings: docs/feature/netns-density-295/spike/findings-v18.md).

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
      does not affect later datagrams (to be tried; no prediction of failure).
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
    out = {}
    rec("substrate", "guest_uname", H.gx(cid, "uname -a")[1])
    rec("substrate", "sha_selftest_host", H.sh(f"{H.CTX['vfwd']} sha-selftest"))
    rec("substrate", "sha_selftest_guest", H.gx(cid, "/vfwd sha-selftest")[1])
    rec("substrate", "offhost", P.setup_offhost())
    H.set_modes(P.MODE, EG)
    rec("programs", "loaded", H.sh(f"grep -o '\"programs\":.*' {H.CTX['host_owner'].owner_log} | head -c 3000", check=False, quiet=True))
    S = sizes_for(1500)
    rec("plan", "sizes_1500", S)
    results = {}

    def phase(tag, kinds, sizes, **kw):
        res, sent, rx = P.run_phase(tag, kinds, sizes, **kw)
        results[tag] = summarize(res)
        rec("phases", tag, results[tag])
        return res, sent, rx

    # ---------------------------------------------------------------- H1 frame-shaped payloads
    phase("h1_frame_unconn", FRAMEY, S, reps=2)
    phase("h1_frame_conn", FRAMEY, S, reps=2, connect=True)
    phase("h1_empty_both", ["rand"], [0], reps=3)
    phase("h1_empty_conn", ["rand"], [0], reps=3, connect=True)
    # same association: frame-shaped and plain, mixed, ascending sizes on one socket
    seqs = [8, 16, 100, 1464, 1465, 1472, 1473, 1480, 2944, 2952, 4096, 8192, 16384, 59000]
    phase("h1_seq_unconn", ["frame", "rand", "frame2", "nf_chain", "frame_chain"], seqs, seq=True, reps=2)
    phase("h1_seq_conn", ["frame", "rand", "frame2", "nf_chain", "frame_chain"], seqs, seq=True, reps=2, connect=True)
    # ---------------------------------------------------------------- H2 not frame-shaped but frame-looking, plain
    NM = [8, 16, 17, 1464, 1465, 1472, 1473, 1480, 2944, 2952, 4096, 4097, 59000]
    phase("h2_nearmiss_unconn", NEARMISS, NM, reps=1)
    phase("h2_nearmiss_conn", NEARMISS, NM, reps=1, connect=True)
    phase("h2_plain", ["rand"], [1, 7] + S, reps=1)
    phase("h2_zero_ones", ["zero", "ones"], NM, reps=1)
    # ---------------------------------------------------------------- H3 concurrent fragmented trains
    BIG = [x for x in S if x >= 1440]
    phase("h3_conc_unconn", ["frame", "frame2", "frame_chain", "rand", "nf_chain", "decl_len", "decl_m9"], BIG, threads=4, random_n=250, seed=7)
    phase("h3_conc_conn", ["frame", "frame2", "frame_chain", "rand", "nf_chain", "decl_len", "decl_m9"], BIG, threads=4, random_n=250, seed=9, connect=True)
    # ---------------------------------------------------------------- H5 wire capture and independent reassembly
    wk = ["frame"]
    ws = [8, 16, 1464, 1465, 1472, 1473, 1480, 2936, 2944, 2952, 4096, 59000]
    cap = P.Capture(P.HV, "wire")
    off = P.peer_offset()
    allsent = []
    for tag, kinds, sizes in [("w_frame", ["frame"], ws), ("w_frame2", ["frame2"], [4096, 59000]), ("w_chain", ["frame_chain"], [4096, 59000]),
                              ("w_rand", ["rand"], [1472, 1473, 4096, 59000]), ("w_decl1472", ["decl_1472"], [4096])]:
        res, sent, rx = P.run_phase(tag, kinds, sizes, reps=1)
        results[tag] = summarize(res)
        allsent += sent
    path = cap.stop()
    wire = P.parse_pcap(path)
    mw = P.match_wire(allsent, wire)
    rec("H5_wire", "match", mw)
    rec("H5_wire", "all_layouts_of_frame_shaped_above_mtu", [l for l in mw["layouts"] if l["shaped"] and l["len"] > 1472])
    # ---------------------------------------------------------------- H4 other MTUs
    mtu_res = {}
    for mtu in (1280, 1400, 1492, 9000):
        H.sh(f"ip link set {P.HV} mtu {mtu}")
        H.sh(f"ip -n {P.NS} link set {P.PV} mtu {mtu}")
        Sm = sizes_for(mtu, ks=(0, 1, 2, 5), ds=(-2, -1, 0, 1, 2), fixed=[8, 16, 4096, 59000])
        r1, _, _ = P.run_phase(f"h4_mtu{mtu}_frame", ["frame", "frame_chain"], Sm, reps=1)
        r2, _, _ = P.run_phase(f"h4_mtu{mtu}_plain", ["rand", "nf_chain", "decl_len"], [x for x in Sm if x in (8, 16) or x % 3 == 0] or Sm[:6], reps=1)
        mtu_res[mtu] = {"frame": summarize(r1), "plain": summarize(r2), "sizes": Sm}
    H.sh(f"ip link set {P.HV} mtu 1500")
    H.sh(f"ip -n {P.NS} link set {P.PV} mtu 1500")
    rec("H4_mtu", "result", mtu_res)
    # ---------------------------------------------------------------- H6 control: egress program missing on the egress interface
    H.set_modes(P.MODE, ["--unframe", "hybrid"])
    rec("H6_programs", "loaded", H.sh(f"grep -o '\"programs\":.*' {H.CTX['host_owner'].owner_log} | head -c 1500", check=False, quiet=True))
    rec("H6_tcx_on_veth", "bpftool", H.sh(f"bpftool net show dev {P.HV}", check=False, quiet=True))
    r6, sent6, rx6 = P.run_phase("h6_missing_tc", ["frame", "rand", "nf_chain"], [8, 100, 1464, 1472, 4096, 59000], reps=1)
    results["h6_missing_tc"] = summarize(r6)
    rec("H6_missing_tc", "judge", r6)
    r6e, _, _ = P.run_phase("h6_missing_tc_empty", ["rand"], [0], reps=3)
    rec("H6_missing_tc", "empty", summarize(r6e))
    # ---------------------------------------------------------------- H7 stale fragment-tracking state
    H.set_modes(P.MODE, EG)
    s7 = {}
    H.sh(f"ip netns exec {P.NS} sysctl -w net.ipv4.ipfrag_time=2 net.ipv4.ipfrag_high_thresh=134217728 net.ipv4.ipfrag_low_thresh=100663296")
    cap7 = P.Capture(P.HV, "h7")
    base_f, sent_bf, _ = P.run_phase("h7_base_frame", ["frame"], [4096], reps=10)
    base_r, sent_br, _ = P.run_phase("h7_base_rand", ["rand"], [4096], reps=10)
    s7["base"] = [summarize(base_f), summarize(base_r)]
    # holder keeps a registered association (tuple) alive
    off = P.peer_offset()
    H.gx(cid, f"/vfwd v18-send --dst {P.PEER}:{P.PEER_PORT} --kinds rand --sizes 64 --reps 1 --connect --hold-ms 120000 > /tmp/hold.out 2>&1 </dev/null & echo started", quiet=True)
    time.sleep(1.5)
    rx = P.peer_rx_since(off)
    frm = rx[-1]["from"]
    sport = int(frm.split(":")[1])
    s7["holder"] = {"peer_saw_from": frm}
    rs = P.raw_sock()
    # ---- S1: one first fragment whose last fragment never passes the program, then an unrelated datagram with the same IP id
    ID = 0x6e57
    L = 3000
    frame = P.MAGIC + struct.pack(">I", L)
    stageA = P.udp_hdr(sport, P.PEER_PORT, 16 + L) + frame + os.urandom(1464)
    P.send_frag(rs, P.HOST_NS_IP, P.PEER, ID, 0, True, stageA)
    time.sleep(4)  # the peer's partial datagram expires (ipfrag_time=2); the program's map entry has no expiry
    snm0 = P.peer_ns_snmp()

    def victim(ident, sp, tag):
        body = os.urandom(3000)
        import hashlib
        d = P.udp_hdr(sp, P.PEER_PORT, 8 + 3000) + body
        o = P.peer_offset()
        P.send_frag(rs, P.HOST_NS_IP, P.PEER, ident, 0, True, d[:1480])
        P.send_frag(rs, P.HOST_NS_IP, P.PEER, ident, 1480, True, d[1480:2960])
        P.send_frag(rs, P.HOST_NS_IP, P.PEER, ident, 2960, False, d[2960:])
        rows = P.peer_rx_since(o)
        sha = hashlib.sha256(body).hexdigest()
        return {"tag": tag, "id": hex(ident), "sport": sp, "sent_len": 3000, "received": any(r["sha"] == sha for r in rows),
                "peer_rx_rows": len(rows)}

    s7["S1_victim_same_id"] = victim(ID, 45555, "victim (stale id)")
    s7["S1_control_other_id"] = victim(ID + 1, 45555, "control (id never seen by the program)")
    s7["S1_victim_again_same_id"] = victim(ID, 45555, "again (entry consumed by the previous last fragment)")
    s7["S1_peer_snmp_delta"] = P.snmp_delta(snm0, P.peer_ns_snmp())
    # ---- S2: stale entries for the IP ids the next real datagrams will use, then real traffic of the other associations
    wire_so_far = P.parse_pcap(cap7.path) if cap7.path.exists() else []
    ids = [w["id"] for w in wire_so_far if w.get("sport") not in (None, 45555, sport) and len(w["frags"]) > 1]
    last = ids[-1] if ids else None
    s7["S2_last_real_ip_id"] = last
    if last is not None:
        n_stale = 4096
        t0 = time.time()
        for i in range(n_stale):
            ident = (last + 1 + i) & 0xffff
            hdr = P.udp_hdr(sport, P.PEER_PORT, 16 + 2000) + P.MAGIC + struct.pack(">I", 2000)
            P.send_frag(rs, P.HOST_NS_IP, P.PEER, ident, 0, True, hdr)
            if i % 256 == 255:
                time.sleep(0.02)
        s7["S2_stuffed"] = {"count": n_stale, "first_id": (last + 1) & 0xffff, "wall_s": round(time.time() - t0, 2)}
        time.sleep(4)  # peer partial datagrams expire
        r_b, _, _ = P.run_phase("h7_after_stuff_rand", ["rand"], [4096], reps=60)
        r_c, _, _ = P.run_phase("h7_after_stuff_frame", ["frame"], [4096], reps=20)
        s7["S2_after_stuff"] = [summarize(r_b), summarize(r_c)]
        s7["S2_after_stuff_rand_examples"] = {"missing": r_b["missing_examples"][:3], "extra": r_b["extra_examples"][:3]}
        time.sleep(40)
        r_d, _, _ = P.run_phase("h7_after_40s_rand", ["rand"], [4096], reps=20)
        s7["S2_after_40s"] = summarize(r_d)
    wire7 = P.parse_pcap(cap7.stop())
    # keep only a compact, relevant slice of the capture analysis
    s7["wire_victims"] = [w for w in wire7 if w.get("sport") == 45555][:12]
    s7["wire_stageA"] = [w for w in wire7 if w.get("id") == ID and w.get("sport") == sport][:3]
    if last is not None:
        stuffed = set(((last + 1 + i) & 0xffff) for i in range(4096))
        crafted_idx = [i for i, w in enumerate(wire7) if w.get("sport") == sport and w.get("udp_len") == 2016]
        real_after = [w for w in wire7[(crafted_idx[-1] + 1 if crafted_idx else 0):]
                      if w.get("sport") not in (None, sport, 45555) and len(w["frags"]) > 1]
        s7["S2_wire_real_fragmented_after_stuff"] = {
            "n": len(real_after),
            "id_in_stuffed_range": sum(1 for w in real_after if w["id"] in stuffed),
            "of_those_incomplete_or_inconsistent_on_wire": sum(1 for w in real_after if w["id"] in stuffed and (not w["complete"] or not w.get("len_consistent"))),
            "not_in_range_but_incomplete": sum(1 for w in real_after if w["id"] not in stuffed and not w["complete"]),
            "first_ids": [w["id"] for w in real_after[:10]], "example_layouts_in_range": [w["frags"] for w in real_after if w["id"] in stuffed][:3],
            "example_layouts_not_in_range": [w["frags"] for w in real_after if w["id"] not in stuffed][:3]}
    rec("H7", "result", s7)
    rs.close()
    H.gx(cid, "pkill -f 'v18-send' ; true", quiet=True)
    # owner restart clears the program's maps
    H.set_modes(P.MODE, EG)
    r_e, _, _ = P.run_phase("h7_after_owner_restart_rand", ["rand", "frame"], [4096], reps=20)
    rec("H7", "after_owner_restart", summarize(r_e))
    rec("summary", "all_phases_pass", {k: v["PASS"] for k, v in results.items()})
    rec("summary", "phase_results", results)


H.run_increment(probes, P.MODE)

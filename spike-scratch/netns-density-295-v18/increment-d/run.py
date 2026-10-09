#!/usr/bin/env python3
"""increment-d: V-18 H7 redone, plus an H3 soak.

Why: increment-b and -c crafted their stale fragments with a raw IP socket (IP_HDRINCL). The metal host's netfilter
LOCAL_OUT defrag (nf_defrag_ipv4) reassembled those crafted fragments with each other (same src, dst, IP id) and
re-fragmented the result, so (b) S1's "victim" was a chimera of stage A's first fragment and the victim's later
fragments and (c) S2's stuffed first fragments were held in the host's defrag queue and never reached the egress
program. Both H7 results are INVALID; only the AF_PACKET injection here bypasses netfilter.

Pre-registered predictions (written before this increment ran, from reading eg_decode_frag in bpf/src/main.rs):
  S1  After a stripped first fragment whose last fragment never passes the program, FRAGS holds 1 entry. An unrelated
      bare fragmented datagram with the same (src, dst, IP id) then has its later fragments moved back by 8 bytes and
      is NOT delivered (peer reassembles it, UDP drops it: length mismatch); a control with another id is delivered; the
      same id again is delivered (the previous train's last fragment removed the entry).
  S2  With FRAGS full (4,096 entries), every later frame-kept fragmented datagram loses its tail: the insert fails, the
      later fragments are not shifted, the peer's reassembly never completes. Bare fragmented datagrams whose IP id
      equals a stale entry are lost, about 4096/65536 = 6% of them. Entries never expire. An owner restart clears them.
  S3  An egress queue that tail-drops a non-last fragment (ip_do_fragment then stops sending) leaves entries behind
      without any crafting.
  H3soak  Concurrent fragmented trains: 0 losses in 12,000 datagrams (increment-b lost 1 of 2,000 and the cause was not
      identified; this tries to reproduce it with a capture running).
"""
import hashlib, json, os, pathlib, random, struct, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H
import probes as P

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
EG = ["--unframe", "hybrid", "--egress-if", P.HV]


def summarize(res):
    return {k: res[k] for k in ("tag", "PASS", "sent", "sender_ack_ok", "peer_rx", "missing_at_peer", "extra_at_peer", "shaped_sent",
                                "counters", "peer_ns_snmp_delta")}


def probes():
    cid = H.CTX["cid"]
    rec = H.record
    rec("substrate", "guest_uname", H.gx(cid, "uname -a")[1])
    rec("substrate", "netfilter_modules", H.sh("lsmod | grep -E 'nf_defrag|nf_conntrack|nf_tables|ip_tables' || true", check=False, quiet=True))
    rec("substrate", "offhost", P.setup_offhost())
    H.set_modes(P.MODE, EG)
    H.sh(f"ip netns exec {P.NS} sysctl -w net.ipv4.ipfrag_time=2 net.ipv4.ipfrag_high_thresh=134217728 net.ipv4.ipfrag_low_thresh=100663296")
    rec("programs", "tcx_on_veth", H.sh(f"bpftool net show dev {P.HV}", check=False, quiet=True))
    s7 = {}

    def holder():
        off = P.peer_offset()
        H.gx(cid, f"/vfwd v18-send --dst {P.PEER}:{P.PEER_PORT} --kinds rand --sizes 64 --reps 1 --connect --hold-ms 170000 > /tmp/hold.out 2>&1 </dev/null & echo started", quiet=True)
        time.sleep(1.5)
        frm = P.peer_rx_since(off)[-1]["from"]
        return int(frm.split(":")[1]), frm

    sport, frm = holder()
    s7["holder_from"] = frm
    s7["entries_start"] = P.frags_entries()
    inj = P.PacketInjector()
    # ---------------------------------------------------------------- S1
    cap1 = P.Capture(P.HV, "d-s1")
    ID, L = 0x6e57, 3000
    stageA = P.udp_hdr(sport, P.PEER_PORT, 16 + L) + P.MAGIC + struct.pack(">I", L) + os.urandom(1464)
    inj.frag(P.HOST_NS_IP, P.PEER, ID, 0, True, stageA)
    time.sleep(0.5)
    s7["S1_entries_after_stageA"] = P.frags_entries()
    time.sleep(3.5)  # the peer's partial datagram expires (ipfrag_time=2); the program's entry has no expiry
    snm0 = P.peer_ns_snmp()

    def victim(ident, sp, tag):
        body = os.urandom(3000)
        d = P.udp_hdr(sp, P.PEER_PORT, 8 + 3000) + body
        o = P.peer_offset()
        inj.frag(P.HOST_NS_IP, P.PEER, ident, 0, True, d[:1480])
        inj.frag(P.HOST_NS_IP, P.PEER, ident, 1480, True, d[1480:2960])
        inj.frag(P.HOST_NS_IP, P.PEER, ident, 2960, False, d[2960:])
        rows = P.peer_rx_since(o)
        return {"tag": tag, "id": hex(ident), "sport": sp, "received_byte_exact": any(r["sha"] == hashlib.sha256(body).hexdigest() for r in rows),
                "peer_rx_rows": [{"len": r["len"]} for r in rows]}

    s7["S1_victim_same_id"] = victim(ID, 45555, "unrelated bare datagram, same (src,dst,id) as the stale entry")
    s7["S1_entries_after_victim"] = P.frags_entries()
    s7["S1_control_other_id"] = victim(ID + 1, 45555, "control: an id the program never saw")
    s7["S1_victim_again_same_id"] = victim(ID, 45555, "same id again: the previous train's last fragment consumed the entry")
    s7["S1_peer_snmp_delta"] = P.snmp_delta(snm0, P.peer_ns_snmp())
    w1 = P.parse_pcap(cap1.stop())
    s7["S1_wire_trains"] = [{"i": i, "id": hex(w["id"]), "sport": w["sport"], "udp_len": w["udp_len"], "frags": w["frags"]} for i, w in enumerate(w1)
                            if w["id"] in (ID, ID + 1)]
    # ---------------------------------------------------------------- S2
    cap2 = P.Capture(P.HV, "d-s2", snap=96)
    ids = random.Random(18).sample(range(1, 65536), 4300)
    t0 = time.time()
    for i, ident in enumerate(ids):
        hdr = P.udp_hdr(sport, P.PEER_PORT, 16 + 2000) + P.MAGIC + struct.pack(">I", 2000) + os.urandom(1464)
        inj.frag(P.HOST_NS_IP, P.PEER, ident, 0, True, hdr)
        if i % 128 == 127:
            time.sleep(0.02)
    s7["S2_fill"] = {"sent": len(ids), "wall_s": round(time.time() - t0, 2)}
    time.sleep(4)
    s7["S2_entries_after_fill"] = P.frags_entries()
    stale = set(ids)
    r_fr, _, _ = P.run_phase("d_s2_frame_after_fill", ["frame"], [4096], reps=40)
    s7["S2_frame_after_fill"] = summarize(r_fr)
    r_rd, _, _ = P.run_phase("d_s2_rand_after_fill", ["rand"], [4096], reps=400)
    s7["S2_rand_after_fill"] = summarize(r_rd)
    s7["S2_entries_after_traffic"] = P.frags_entries()
    wire2 = P.parse_pcap(cap2.stop())
    crafted_idx = [i for i, w in enumerate(wire2) if w.get("sport") == sport and w.get("udp_len") == 2016 + 0]
    # stripped crafted heads have udp_len rewritten to 2008
    crafted_idx = [i for i, w in enumerate(wire2) if w.get("sport") == sport and w.get("udp_len") in (2016, 2008)]
    real = [w for w in wire2[(crafted_idx[-1] + 1 if crafted_idx else 0):] if w.get("sport") not in (None, sport) and len(w["frags"]) > 1]

    def shifted(w):
        return len(w["frags"]) > 1 and w["frags"][0][1] == 1480 and any(f[0] % 1480 != 0 for f in w["frags"][1:])

    bare = [w for w in real if w["frags"][0][1] == 1480]
    s7["S2_wire"] = {"crafted_heads_seen": len(crafted_idx), "crafted_heads_stripped_on_wire": sum(1 for i in crafted_idx if wire2[i]["udp_len"] == 2008),
                     "real_fragmented_trains_after_fill": len(real), "bare_trains": len(bare),
                     "bare_with_id_in_stale_set": sum(1 for w in bare if w["id"] in stale),
                     "bare_shifted_on_wire": sum(1 for w in bare if shifted(w)),
                     "bare_shifted_and_id_in_stale_set": sum(1 for w in bare if shifted(w) and w["id"] in stale),
                     "bare_shifted_but_id_not_in_stale_set": sum(1 for w in bare if shifted(w) and w["id"] not in stale),
                     "example_shifted_layouts": [w["frags"] for w in bare if shifted(w)][:2],
                     "frame_kept_trains_on_wire": [w["frags"] for w in real if w["frags"][0][1] == 1472][:2]}
    time.sleep(40)
    r_40, _, _ = P.run_phase("d_s2_frame_after_40s", ["frame"], [4096], reps=20)
    s7["S2_frame_after_40s"] = summarize(r_40)
    s7["S2_entries_after_40s"] = P.frags_entries()
    H.gx(cid, "pkill -f 'v18-send' ; true", quiet=True)
    H.set_modes(P.MODE, EG)
    r_rs, _, _ = P.run_phase("d_s2_after_owner_restart", ["rand", "frame"], [4096], reps=40)
    s7["S2_after_owner_restart"] = summarize(r_rs)
    s7["S2_entries_after_restart"] = P.frags_entries()
    # ---------------------------------------------------------------- S3
    s7["S3_entries_before"] = P.frags_entries()
    H.sh(f"tc qdisc add dev {P.HV} root tbf rate 4mbit burst 1600 limit 1600")
    r_q, _, _ = P.run_phase("d_s3_congested_frame", ["frame"], [4096], reps=60, timeout_ms=400)
    s7["S3_tc_qdisc_stats"] = H.sh(f"tc -s qdisc show dev {P.HV}", check=False, quiet=True)
    H.sh(f"tc qdisc del dev {P.HV} root", check=False)
    s7["S3_congested"] = summarize(r_q)
    s7["S3_entries_after_congestion"] = P.frags_entries()
    time.sleep(3)
    r_ok, _, _ = P.run_phase("d_s3_after_congestion", ["frame", "rand"], [4096], reps=20)
    s7["S3_after_congestion"] = summarize(r_ok)
    s7["S3_entries_after_recovery"] = P.frags_entries()
    rec("H7", "result", s7)
    inj.close()
    H.gx(cid, "pkill -f 'v18-send' ; true", quiet=True)
    # ---------------------------------------------------------------- H3 soak
    H.set_modes(P.MODE, EG)
    KC = ["frame", "frame2", "frame_chain", "rand", "nf_chain", "decl_len", "decl_m9"]
    BIG = [1440, 1456, 1463, 1464, 1465, 1466, 1470, 1471, 1472, 1473, 1474, 2936, 2944, 2951, 2952, 2953, 4096, 4097, 8192, 16384, 32768, 58999, 59000]
    soak = {}
    for i in range(12):
        conn = bool(i % 2)
        res, _, _ = P.run_phase(f"d_h3soak_{i}", KC, BIG, threads=4, random_n=250, seed=100 + i, connect=conn, pcap_on_fail=True)
        soak[f"{i}_{'conn' if conn else 'unconn'}"] = {k: summarize(res)[k] for k in ("PASS", "sent", "missing_at_peer", "extra_at_peer")} | {"pcap": res.get("pcap")}
    rec("H3soak", "result", soak)
    rec("H3soak", "total_sent", sum(v["sent"] for v in soak.values()))
    rec("H3soak", "total_missing", sum(v["missing_at_peer"] for v in soak.values()))


H.run_increment(probes, P.MODE)

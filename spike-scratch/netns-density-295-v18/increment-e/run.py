#!/usr/bin/env python3
"""increment-e: three follow-ups to increment-d.

Pre-registered predictions (written before this increment ran):
  S1b  Increment-d's "same id again" was not delivered although the entry was consumed. Prediction: the peer's own
       reassembly queue for that (src,dst,id) from the victim train was still alive (ipfrag_time=2 s). Waiting 3 s before
       the second try delivers it.
  S3b  A queue that drops a NON-last fragment leaves stale entries. `netem delay 20ms limit 1`: fragment 1 queued,
       fragment 2 dropped (NET_XMIT_DROP), ip_do_fragment stops, fragment 3 never reaches the egress program. Prediction:
       FRAGS entries > 0 after the congested phase and they stay after the qdisc is removed; repeating adds more.
  Soak Increment-d lost 1 datagram in 7 of 12 concurrent runs (4 threads x 250). Hypothesis A: the loss is in the
       inherited guest/vsock/pairing leg, upstream of the host verdict, and independent of size/fragmentation.
       Prediction: the same loss rate with SMALL datagrams only (<= 1,431 bytes, no IP fragmentation), and every failure
       classified as never_reached_egress (not on the wire, guest sends > host verdict count) or ack_lost_after_delivery
       (peer has it byte-exact, the reply to the guest is lost). Falsification: a failure classified lost_after_egress, or
       a wire train that is incomplete/inconsistent, or small-datagram soak with 0 losses while big-datagram soak loses.
       Serial (1 thread) soak: 0 losses.
"""
import hashlib, json, os, pathlib, struct, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H
import probes as P

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
EG = ["--unframe", "hybrid", "--egress-if", P.HV]
KC = ["frame", "frame2", "frame_chain", "rand", "nf_chain", "decl_len", "decl_m9"]
BIG = [1440, 1456, 1463, 1464, 1465, 1466, 1470, 1471, 1472, 1473, 1474, 2936, 2944, 2951, 2952, 2953, 4096, 4097, 8192, 16384, 32768, 58999, 59000]
SMALL = [8, 16, 64, 100, 500, 1000, 1400, 1431, 1440]  # wire UDP <= 1480 even with a kept frame: no IP fragmentation
kept_pcaps = []


def summarize(res):
    return {k: res[k] for k in ("tag", "PASS", "sent", "sender_ack_ok", "peer_rx", "missing_at_peer", "extra_at_peer", "shaped_sent",
                                "counters", "peer_ns_snmp_delta")}


def classify(res, sent, rx):
    """For every sender row that did not complete: where was it lost?"""
    rxset = {(r["len"], r["sha"]) for r in rx}
    wire = P.parse_pcap(H.EV / res["pcap"]) if res.get("pcap") else None
    wset = {w["sha"] for w in wire if w.get("sha")} if wire is not None else None
    out = []
    for r in sent:
        if r.get("ok"):
            continue
        at_peer = (r["len"], r["sha"]) in rxset
        on_wire = (r["sha"] in wset) if wset is not None else None
        if at_peer:
            c = "ack_lost_after_delivery"
        elif on_wire:
            c = "lost_after_egress"
        elif on_wire is False:
            c = "never_reached_egress"
        else:
            c = "not_at_peer_no_capture"
        out.append({"kind": r["kind"], "len": r["len"], "conn": r["conn"], "shaped": r["shaped"], "class": c})
    res["failures_classified"] = out
    if wire is not None:
        res["wire_bad_trains"] = sum(1 for w in wire if (not w["complete"]) or (w.get("complete") and not w.get("len_consistent", True)))
        res["wire_trains"] = len(wire)
    return out


def soak(tag, n, sizes, **kw):
    rows = {}
    for i in range(n):
        conn = bool(i % 2) if kw.get("threads", 4) > 0 and not kw.pop("force_unconn", False) else False
        res, sent, rx = P.run_phase(f"{tag}_{i}", KC, sizes, connect=conn, pcap_on_fail=True, seed=300 + i, **kw)
        fc = classify(res, sent, rx)
        g = res.get("guest_counters") or {}
        c = res.get("counters") or {}
        rows[f"{i}_{'conn' if conn else 'unconn'}"] = {
            "PASS": res["PASS"], "sent": res["sent"], "ack_ok": res["sender_ack_ok"], "peer_rx": res["peer_rx"],
            "missing_at_peer": res["missing_at_peer"], "extra_at_peer": res["extra_at_peer"],
            "guest_udp_capture": g.get("udp_capture"), "guest_encoded": g.get("encoded"),
            "host_verdict_total": (c.get("vstrip", 0) + c.get("hybrid_escaped", 0) + c.get("hybrid_empty_framed", 0)),
            "host_drop_counters": {k: c.get(k) for k in ("noroute", "helper_fail", "bad_frame", "redirect_fail", "tc_err_frag", "tc_err", "zero_len_drop") if c.get(k)},
            "failures": fc, "wire_bad_trains": res.get("wire_bad_trains"), "pcap": res.get("pcap")}
        if res.get("pcap"):
            if len(kept_pcaps) >= 2:
                (H.EV / res["pcap"]).unlink()
                rows[f"{i}_{'conn' if conn else 'unconn'}"]["pcap"] = res["pcap"] + " (deleted after classification to bound evidence size)"
            else:
                kept_pcaps.append(res["pcap"])
    return rows


def probes():
    cid = H.CTX["cid"]
    rec = H.record
    rec("substrate", "guest_uname", H.gx(cid, "uname -a")[1])
    rec("substrate", "offhost", P.setup_offhost())
    H.set_modes(P.MODE, EG)
    H.sh(f"ip netns exec {P.NS} sysctl -w net.ipv4.ipfrag_time=2 net.ipv4.ipfrag_high_thresh=134217728 net.ipv4.ipfrag_low_thresh=100663296")
    rec("programs", "tcx_on_veth", H.sh(f"bpftool net show dev {P.HV}", check=False, quiet=True))

    def holder():
        off = P.peer_offset()
        H.gx(cid, f"/vfwd v18-send --dst {P.PEER}:{P.PEER_PORT} --kinds rand --sizes 64 --reps 1 --connect --hold-ms 100000 > /tmp/hold.out 2>&1 </dev/null & echo started", quiet=True)
        time.sleep(1.5)
        frm = P.peer_rx_since(off)[-1]["from"]
        return int(frm.split(":")[1]), frm

    # ---------------------------------------------------------------- S1b
    s1 = {}
    sport, frm = holder()
    inj = P.PacketInjector()
    cap1 = P.Capture(P.HV, "e-s1")
    ID, L = 0x6e57, 3000
    inj.frag(P.HOST_NS_IP, P.PEER, ID, 0, True, P.udp_hdr(sport, P.PEER_PORT, 16 + L) + P.MAGIC + struct.pack(">I", L) + os.urandom(1464))
    time.sleep(0.5)
    s1["entries_after_stageA"] = P.frags_entries()
    time.sleep(3.5)

    def victim(ident, sp, tag):
        body = os.urandom(3000)
        d = P.udp_hdr(sp, P.PEER_PORT, 8 + 3000) + body
        o = P.peer_offset()
        inj.frag(P.HOST_NS_IP, P.PEER, ident, 0, True, d[:1480])
        inj.frag(P.HOST_NS_IP, P.PEER, ident, 1480, True, d[1480:2960])
        inj.frag(P.HOST_NS_IP, P.PEER, ident, 2960, False, d[2960:])
        rows = P.peer_rx_since(o)
        return {"tag": tag, "id": hex(ident), "received_byte_exact": any(r["sha"] == hashlib.sha256(body).hexdigest() for r in rows)}

    snm = P.peer_ns_snmp()
    s1["victim_same_id"] = victim(ID, 45555, "unrelated bare datagram, same (src,dst,id) as the stale entry")
    s1["entries_after_victim"] = P.frags_entries()
    s1["control_other_id"] = victim(ID + 1, 45555, "control: an id the program never saw")
    time.sleep(3)
    s1["same_id_again_after_3s"] = victim(ID, 45555, "same id again after the peer's queue expired; entry was consumed")
    s1["peer_snmp_delta"] = P.snmp_delta(snm, P.peer_ns_snmp())
    w1 = P.parse_pcap(cap1.stop())
    s1["wire_trains"] = [{"id": hex(w["id"]), "sport": w["sport"], "udp_len": w["udp_len"], "frags": w["frags"]} for w in w1 if w["id"] in (ID, ID + 1)]
    inj.close()
    H.gx(cid, "pkill -f 'v18-send' ; true", quiet=True)
    rec("S1b", "result", s1)
    # ---------------------------------------------------------------- S3b
    s3 = {"entries_before": P.frags_entries()}
    H.sh(f"tc qdisc add dev {P.HV} root netem delay 20ms limit 1")
    runs = []
    for k in range(3):
        r, _, _ = P.run_phase(f"e_s3_congested_{k}", ["frame"], [4096], reps=40, timeout_ms=400)
        runs.append({"phase": summarize(r), "entries_after": P.frags_entries()})
    s3["congested_runs"] = runs
    s3["qdisc_stats"] = H.sh(f"tc -s qdisc show dev {P.HV}", check=False, quiet=True)
    H.sh(f"tc qdisc del dev {P.HV} root", check=False)
    time.sleep(3)
    s3["entries_after_qdisc_removed"] = P.frags_entries()
    r, _, _ = P.run_phase("e_s3_after_congestion", ["frame", "rand"], [4096], reps=100)
    s3["after_congestion_traffic"] = summarize(r)
    s3["entries_after_traffic"] = P.frags_entries()
    rec("S3b", "result", s3)
    # ---------------------------------------------------------------- soaks (fresh owner state)
    H.set_modes(P.MODE, EG)
    rec("soak_small", "result", soak("e_small", 12, SMALL, threads=4, random_n=250))
    rec("soak_big", "result", soak("e_big", 12, BIG, threads=4, random_n=250))
    rec("soak_serial", "result", soak("e_serial", 6, BIG, threads=1, random_n=1000, force_unconn=True))


H.run_increment(probes, P.MODE)

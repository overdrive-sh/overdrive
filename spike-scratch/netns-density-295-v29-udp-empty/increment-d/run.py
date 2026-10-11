#!/usr/bin/env python3
"""increment-d: the V-29 evidence run (increment-b was the first evidence run; its harness flaws are listed in the findings: throttled phases were read before the backlog drained, the application socket closed before the backlog or the paused ring drained, tiny sizes were misclassified as split pieces; its evidence is kept). Findings: docs/feature/netns-density-295/spike/findings-v29-udp-empty.md

Pre-registered hypotheses (written before this increment ran; development run increment-a showed the empty-datagram mechanism
works and that bursts closer than ~50 us break the guest->host leg even with no empties at all, so pacing is >= 100 us unless a
phase says otherwise):
  H1  verdict-notify (strip every frame in the host verdict; empty -> SK_DROP + Notice; owner send(h, NULL, 0)):
      every empty datagram is emitted exactly once as a real zero-length UDP datagram (tcpdump on v29-h); no empty is duplicated;
      fexit skb_send_sock never returns <= 0 (no EPIPE / TX disable); data sent after an empty still arrives byte-exact.
  H2  control: verdict-naive (redirect the zero-length skb) disables the target psock TX: later data on the association is lost and
      skb_send_sock returns 0 (this proves the detector would see an interaction).
  H3  an ungated owner-sent empty is reordered against the redirected datagrams around it, at a high rate (the redirect waits for
      a workqueue, the owner wakes from the ring buffer); PREDICTION: > 10 % early at 100-1000 us pacing, worse under a throttled
      destination. With the gate (owner waits until the flow's earlier redirected skbs were sent: SENTN >= Notice.queued) the
      rate is 0 in both regimes.
  H4  ring buffer full: verdict_empty_seen == notice_ok + notice_lost exactly; NLOST (per flow) == notice_lost; the owner's
      notice_lost_seen == notice_lost; peer receives notice_ok empties; with compensation the peer receives all of them.
  H5  paged skbs: PREDICTION from __skb_send_sock: with bpf_skb_pull_data(len) in the verdict every datagram leaves as one
      datagram for every size up to 59000; without it (control) a datagram larger than one fragment of the vhost rx skb (32 KiB)
      leaves as several.
"""
import json, pathlib, sys, time
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import harness as H
import probes as P
import p29 as Q

H.INC = pathlib.Path(__file__).resolve().parent
H.EV = H.INC / "evidence"
PAUSE = "/tmp/v29-work/notice.pause"
N = 4000


def modes(*extra):
    H.set_modes(P.MODE, Q.VERDICT_NOTIFY + list(extra))


def probes():
    cid = H.CTX["cid"]
    rec = H.record
    rec("substrate", "host_uname_r", H.sh("uname -r", quiet=True).strip())
    rec("substrate", "guest_uname_r", H.gx(cid, "uname -r")[1].strip())
    rec("substrate", "offhost", P.setup_offhost())
    Q.untbf()
    modes()
    rec("programs", "loaded", H.sh(f"grep -o '\"programs\":.*' {H.CTX['host_owner'].owner_log} | head -c 2500", check=False, quiet=True))
    rec("programs", "tcx_attachments_during_run", H.sh("bpftool net show 2>&1", check=False, quiet=True))
    rec("programs", "tc_filters_v29_h_and_lo", H.sh(f"tc filter show dev {Q.HV} egress; tc filter show dev lo egress", check=False, quiet=True))

    # ---------------------------------------------------------------- H1 emission, exactly once, no psock interaction
    Q.phase_sizes("h1_empties_only_gap200", [0], reps=3000, gap_us=200, pcap=True)
    Q.phase_sizes("h1_mixed_sizes", [0, 1, 0, 0, 8, 0, 100, 0, 1431, 0, 1472, 0, 1473, 0, 4096, 0, 0, 59000, 0, 7], reps=40, gap_us=2000, pcap=True)
    Q.phase_seq("h1_after_empties_k1_gap200", N, 1400, 1, gap_us=200)

    # ---------------------------------------------------------------- H2 control: naive zero-length redirect
    H.set_modes(P.MODE, ["--unframe", "verdict-naive"])
    Q.phase_sizes("h2_naive_control", [100, 0, 100, 100, 100, 1400, 100], reps=1, gap_us=20000, pcap=True)
    Q.phase_sizes("h2_naive_control_second_assoc", [100, 100], reps=1, gap_us=20000, pcap=True)

    # ---------------------------------------------------------------- H3 reorder, no load
    for gated in ("no", "yes", "spin"):
        modes("--empty-gate", gated)
        for gap in (1000, 500, 200, 100):
            Q.phase_seq(f"h3_noload_gate{gated}_k1_gap{gap}", N, 1400, 1, gap_us=gap)
        Q.phase_seq(f"h3_noload_gate{gated}_k1_gap200_rep2", N, 1400, 1, gap_us=200)
        Q.phase_seq(f"h3_noload_gate{gated}_k8_gap200", N, 1400, 8, gap_us=200)
        Q.phase_seq(f"h3_noload_gate{gated}_k1_small64_gap200", N, 64, 1, gap_us=200)
    modes()
    Q.phase_seq("h3_noload_datanly_control_gap100", N, 1400, 0, gap_us=100)
    # leg-limited regime (informational; the data-only control shows the loss is independent of empties)
    Q.phase_seq("h3_leglimit_gate_no_k1_gap0", 2000, 1400, 1, gap_us=0)
    Q.phase_seq("h3_leglimit_datanly_gap0", 2000, 1400, 0, gap_us=0)

    # ---------------------------------------------------------------- H3 reorder, throttled / backlogged destination
    # tbf 5 Mbit: the destination drains ~440 datagrams/s; the guest offers 2000/s. The application socket lingers until the
    # backlog has drained (host teardown on close purges the psock backlog), and the phase waits for the peer log to go quiet.
    Q.tbf("5mbit", limit=4000000, burst=3000)
    rec("throttle", "tbf", H.sh(f"tc qdisc show dev {Q.HV}", quiet=True))
    LING = "--warm --linger-ms 14000"
    for gated in ("no", "yes", "spin"):
        modes("--empty-gate", gated)
        Q.phase_seq(f"h3_throttle5m_gate{gated}_k1_gap500", 3000, 1400, 1, gap_us=500, extra=LING)
    modes("--empty-gate", "no", "--empty-retry", "no")
    Q.phase_seq("h3_throttle5m_gateno_noretry_k1_gap500", 3000, 1400, 1, gap_us=500, extra=LING)
    modes()
    Q.phase_seq("h3_throttle5m_datanly_control_gap500", 3000, 1400, 0, gap_us=500, extra=LING)
    modes("--empty-gate", "spin", "--h-sndbuf", "16384")
    Q.phase_seq("h3_throttle5m_gatespin_sndbuf16k_k1_gap500", 3000, 1400, 1, gap_us=500, extra=LING)
    Q.untbf()

    # ---------------------------------------------------------------- H4 ring buffer full
    import os, threading
    for tag, extra, reps in [("h4_ring4k_nocomp", ["--notice-bytes", "4096"], 1000), ("h4_ring4k_comp", ["--notice-bytes", "4096", "--notice-compensate", "yes"], 1000),
                             ("h4_ring64k_paused_fits", [], 1000)]:
        modes("--notice-pause-file", PAUSE, *extra)
        open(PAUSE, "w").write("pause")
        off = P.peer_offset()
        hb = Q.host_snapshot()
        box = {}
        th = threading.Thread(target=lambda: box.update(r=Q.sender(tag, f"--sizes 0 --reps {reps} --gap-us 100 --warm --linger-ms 9000")))
        th.start()
        time.sleep(4.0)
        mid = Q.host_snapshot()
        paused_peer_rx = sum(1 for r in P.peer_rx_since(off) if r["len"] == 0)
        os.unlink(PAUSE)
        th.join()
        Q.drain(off)
        rx = P.peer_rx_since(off)
        Q.settle()
        ha = Q.host_snapshot()
        d = Q.host_delta(hb, ha) or {}
        peer_empty = sum(1 for r in rx if r["len"] == 0)
        res = {"sender": box["r"][0], "sent_empty": reps, "peer_empty_rx_while_paused": paused_peer_rx, "peer_empty_rx_final": peer_empty,
               "host_counters_total": d, "host_counters_while_paused": Q.host_delta(hb, mid)}
        res["invariants"] = {"seen_eq_ok_plus_lost": d.get("verdict_empty_seen", 0) == d.get("notice_ok", 0) + d.get("notice_lost", 0),
                             "owner_lost_seen_eq_kernel_lost": d.get("notice_lost_seen", 0) == d.get("notice_lost", 0),
                             "peer_rx_eq_ok_plus_compensated": peer_empty == d.get("notice_ok", 0) + d.get("empty_compensated", 0),
                             "all_empties_on_wire_or_counted_lost": peer_empty + (d.get("notice_lost", 0) - d.get("empty_compensated", 0)) == reps}
        rec("h4", tag, res)
        H.log(f"H4 {tag}: sent={reps} peer_rx={peer_empty} invariants={res['invariants']} {d}")
    modes()

    # ---------------------------------------------------------------- H5 paged skbs / datagram size
    SZ = [1024, 2048, 4096, 8192, 16384, 24576, 32000, 32768, 32769, 40000, 49152, 59000, 65507]
    modes()
    Q.phase_sizes("h5_sizes_linearise", SZ, reps=5, gap_us=30000, pcap=True)
    modes("--no-linearise", "yes")
    Q.phase_sizes("h5_sizes_NO_linearise_control", SZ, reps=5, gap_us=30000, pcap=True)
    modes()


H.run_increment(probes, P.MODE)

#!/usr/bin/env python3
"""Offline table from evidence/*/<label>-summary.json (one row per cell)."""
import glob, json, sys
rows = []
for f in sorted(glob.glob("evidence/vm-matrix-*/*-summary.json")):
    r = json.load(open(f)); h, g, c = r["host"], r["guest"], r["cell"]
    if "outcome" not in h: continue
    o, hc, gc, q = h["outcome"], h.get("bpf_counters", {}), g.get("bpf_counters", {}), h["seqrecv"]
    sent = g.get("app_sent_ok", 0)
    once, dup = o["delivered_once"], o["duplicated_seqs"]
    o_ = g.get("skb_send_sock_outcomes_on_vsock_leg") or {}
    rows.append((r["label"], f.split("/")[1][-16:], c.get("variant"), c.get("size"), c.get("n"), c.get("hostbuf", 0), c.get("pause_ms", 0), c.get("nodelay", 0),
                 sent, once, dup, o["duplicate_extra_copies"], o["lost"],
                 hc.get("rx_short", 0) + q["short"], hc.get("rx_long", 0) + q["long"], hc.get("rx_lt8", 0) + hc.get("rx_badmagic", 0) + q["lt8"] + q["badmagic"],
                 o_.get("errno11", 0), o_.get("ok_partial", 0), hc.get("rx_pkt_seq_noeom", 0), gc.get("strp_bad_frame", 0), g.get("sender_stalled")))
hdr = ("label", "run", "var", "size", "n", "hostbuf", "pause", "nodelay", "sent", "once", "dupseq", "dupcopies", "lost", "short", "long", "lt8/badmagic", "EAGAIN", "partialOK", "noEOMpkts", "guestStrpBad", "stalled")
if len(sys.argv) > 1: rows = [r for r in rows if sys.argv[1] in r[0]]
w = [max(len(str(x)) for x in col) for col in zip(hdr, *rows)]
for r in [hdr] + rows: print("  ".join(str(x).ljust(wi) for x, wi in zip(r, w)))

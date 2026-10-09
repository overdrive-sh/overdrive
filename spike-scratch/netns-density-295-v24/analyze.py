#!/usr/bin/env python3
"""Offline analysis of a V-24 increment's raw captures (evidence/cases/*.jsonl, case-summaries.json).
Matches each guest flow report to its in-guest application event by (c_port, nearest accepted time),
because ephemeral C ports are reused across sequential trials (SO_LINGER{1,0} leaves no TIME_WAIT)."""
import collections, errno as E, json, pathlib, sys

inc = pathlib.Path(sys.argv[1])
ev = inc / "evidence"
summ = json.loads((ev / "case-summaries.json").read_text())


def pct(xs, p):
    xs = sorted(xs)
    return round(xs[min(len(xs) - 1, int(len(xs) * p))], 3) if xs else None


def en(e):
    return E.errorcode.get(e, str(e)) if e else "0"


rows = []
for f in sorted((ev / "cases").glob("*.jsonl")):
    name = f.stem
    evs = [json.loads(l) for l in f.read_text().splitlines() if l.strip()]
    flows = [e for e in evs if e["k"] == "flow" and "error" not in e]
    errs = [e for e in evs if e["k"] == "flow" and "error" in e]
    apps = collections.defaultdict(list)
    for e in evs:
        if e["k"] == "app":
            apps[e["c_port"]].append(e)
    used = set()
    ok = 0
    first = collections.Counter()
    bytes_c = 0
    lat = []
    unmatched = 0
    for fl in flows:
        cands = [(abs(a["accepted_t"] - fl["t_c"]), i, a) for i, a in enumerate(apps.get(fl["c_port"], [])) if (fl["c_port"], i) not in used]
        if not cands:
            unmatched += 1
            continue
        _, i, a = min(cands, key=lambda c: c[0])
        used.add((fl["c_port"], i))
        first[a["first"]] += 1
        bytes_c += a["bytes"]
        lat.append((a["first_t"] - fl["t_accept"]) / 1000.0)
        if fl["decision"].startswith("teardown") and a["first"] in ("reset", "eof") and a["bytes"] == 0:
            ok += 1
    req16 = sum(1 for fl in flows if fl["req"]["n"] == 16)
    rdhup_at_accept = sum(1 for fl in flows if fl["pre"]["rev"] & 0x2000)
    eof_next = sum(1 for fl in flows if fl["obs"]["rd_rc"] == 0)
    epipe = sum(1 for fl in flows if fl["obs"]["wr_err"] == 32)
    ctl = sum(1 for fl in flows if fl["decision"] == "teardown_ctl")
    vg = sum(1 for fl in flows if fl["decision"] == "teardown_vg")
    s = summ[name]
    L = s["leftover_1s_after"]
    rows.append(dict(case=name, n=len(flows) + len(errs), worker_errors=len(errs), teardown=ctl + vg, via_vg=vg, via_ctl=ctl,
                     matched_app=len(flows) - unmatched, teardown_ok=ok, app_first=dict(first), bytes_on_C=bytes_c,
                     request_16B_readable=req16, rdhup_before_request_read=rdhup_at_accept, read_after_request_EOF=eof_next,
                     write_EPIPE=epipe, leftover_vsock=L["flow_vsock_total"], leftover_tcp=L["tcp_app_conns"],
                     accept_to_app_ms_p50_p99_max=[pct(lat, .5), pct(lat, .99), pct(lat, 1.0)],
                     host_paired_discarded=s["host_paired_discarded_no_reply"], guest_ctl_discarded=s["guest_counters"]["ctl_discarded"],
                     guest_ctl_unexpected=s["guest_counters"]["ctl_unexpected"],
                     close_to_vg_event_ms=s["latency_host_close_to_vg_event_ms(approx, clock offset ~ms)"]))
out = ev / "analysis.json"
out.write_text(json.dumps(rows, indent=1))
hdr = ["case", "n", "worker_errors", "teardown", "via_vg", "via_ctl", "teardown_ok", "app_first", "bytes_on_C", "request_16B_readable",
       "rdhup_before_request_read", "read_after_request_EOF", "write_EPIPE", "leftover_vsock", "leftover_tcp", "accept_to_app_ms_p50_p99_max"]
for r in rows:
    print(" | ".join(f"{k}={r[k]}" for k in hdr))

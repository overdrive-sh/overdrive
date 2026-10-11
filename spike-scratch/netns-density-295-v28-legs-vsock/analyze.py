#!/usr/bin/env python3
"""Compact view of evidence/<stage>-*/results.json. Usage: analyze.py <evidence-dir> [filter]"""
import json, sys, pathlib

d = pathlib.Path(sys.argv[1]); flt = sys.argv[2] if len(sys.argv) > 2 else ""
r = json.loads((d / "results.json").read_text())
print("substrate:", r.get("substrate"))
for k, v in r.items():
    if k == "substrate" or flt not in k: continue
    if not isinstance(v, dict): print(k, v); continue
    c = v.get("client") or {}
    if c:
        cs = (f"up_sent={c.get('up_sent')} up_err={c.get('up_err')} down={c.get('down_recv')}/{c.get('down_expected')} "
              f"down_end={c.get('down_end')} ok={c.get('down_ok')}{' HANG' if c.get('HANG') else ''}")
        if c.get("killed_at_to_client_done_s") is not None: cs += f" kill->done={c['killed_at_to_client_done_s']}s"
        if c.get("setup_err"): cs += f" SETUP_ERR={c['setup_err']}"
        if c.get("wall_s") is not None: cs += f" wall={c['wall_s']}s"
    else:
        cs = json.dumps({a: b for a, b in v.items() if a not in ("t_total_us", "t_first_us")})[:300]
    apps = v.get("app") or []
    ap = "; ".join(f"up={a.get('up_bytes')} {a.get('up_end')} ok={a.get('up_ok')} sent={a.get('down_sent')} err={a.get('down_err')} {a.get('end_action')} peer={a.get('peer')}" for a in apps) or "-"
    rl = v.get("relay") or []
    pe = ",".join(f"{e['dir']}:{e['end']}" for e in rl if e.get("ev") == "pump_end")
    cl = ",".join(f"{e['dir']}:z{e['zero_send_errno']}" for e in rl if e.get("ev") == "vsock_eof_classify")
    ga = v.get("gagent") or []
    tpc = ",".join(f"{e['dir']}:{e['first']}" for e in (rl + ga) if e.get("ev") == "tp_classify")
    gp = ",".join(f"{e['dir']}:{e['end']}" for e in ga if e.get("ev") == "pump_end") + " classify=" + ",".join(f"{e['dir']}:z{e['zero_send_errno']}" for e in ga if e.get("ev") == "vsock_eof_classify")
    print(f"{k}\n   client: {cs}\n   app:    {ap}\n   relay:  {pe} {('classify=' + cl) if cl else ''}\n   tp:     {tpc or '-'}\n   gagent: {gp or '-'}")

#!/usr/bin/env python3
"""Per-cell table for the V-1 sweep: for each (cell, direction) the vsock-source -> TCP-target direction (d=1) of the role that owns the
non-reading application: G -> guest role, H -> host role. Usage: analyze.py evidence/vm-sweep-*/"""
import glob, json, sys, statistics as st
d = sys.argv[1]
rows = []
def tag(label):
    return label
for f in sorted(glob.glob(d + "/*-summary.json"), key=lambda p: (json.load(open(p))["label"])):
    s = json.load(open(f)); label = s["label"]
    role = "guest_settles" if label.startswith("G") or label.startswith("C") else "host_settles"
    if label.startswith("C"):
        sets = s["guest_settles"] + s["host_settles"]
    else:
        sets = s[role] or []
    d1 = [r for r in sets if r[1] == 1]
    # completed-before-settle rows have sample 0 and done=True; stuck rows are sample 1 and 2
    n_done = sum(1 for r in d1 if r[2] == 0)
    s1 = [r for r in d1 if r[2] == 1]; s2 = [r for r in d1 if r[2] == 2]
    n_trials = s["trials"]["total"]
    # a trial is: complete (sample 0) | incomplete-at-settle (sample 1 exists)
    inc = len(s1)
    def stat(rs, i):
        v = [r[i] for r in rs if r[i] is not None]
        return (min(v), int(st.median(v)), max(v)) if v else None
    # sent == goal at sample 1 / 2 ?
    full1 = sum(1 for r in s1 if r[4] is not None and r[5] >= r[4])
    full2 = sum(1 for r in s2 if r[4] is not None and r[5] >= r[4])
    plateau = sum(1 for a in s1 for b in s2 if a[0] == b[0] and a[5] == b[5])
    app = s["cli_app"] if label.startswith("G") else s["srv_app"]
    rcvbuf = sorted(set(x[3] if label.startswith("G") else x[1] for x in app if (x[3] if label.startswith("G") else x[1]) not in (None, 0)))
    unread = [x[4] if label.startswith("G") else x[2] for x in app]
    unread = [u for u in unread if u is not None]
    goal = stat(d1, 4)
    rows.append((label, n_trials, len(d1), n_done, inc, full1, full2, plateau, stat(s1, 5), stat(s1, 6), goal, stat(s1, 8), stat(s1, 9), rcvbuf, (min(unread), int(st.median(unread)), max(unread)) if unread else None))
print("%-14s %6s %6s %6s %6s %6s %6s %6s | sent@settle(min,med,max) | fwd(med) | goal(min,med,max) | tgt SO_SNDBUF | tgt outq | app SO_RCVBUF | app unread" % ("cell", "trials", "d1rows", "done", "stuck", "s1full", "s2full", "plat"))
for r in rows:
    print("%-14s %6d %6d %6d %6d %6d %6d %6d | %s | %s | %s | %s | %s | %s | %s" % (r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7], r[8], r[9][1] if r[9] else None, r[10], r[11][1] if r[11] else None, r[12][1] if r[12] else None, r[13], r[14]))

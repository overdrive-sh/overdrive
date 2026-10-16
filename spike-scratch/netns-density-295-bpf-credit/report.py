#!/usr/bin/env python3
"""Condense evidence/*/result.json into one table per run."""
import json, sys, glob
keys = ["outcomes","send_outcomes","app_ok","peak_queued_MiB","unreclaim_rise_MiB(max-start)","avail_drop_MiB(start-min)","sender_max_send_ms","sender_sent_at_longest_block_start_MiB","resume_latency_ms(unblock-resume)","reader_MiBps","sender_MiBps","sys_busy_ms","owner_cpu_ms","owner_trial_secs","hard_drops","passed","oom_lines","lost_bytes_per_trial","first_bad","lat_us_(recv-send)"]
for d in sys.argv[1:]:
    r = json.load(open(d + "/result.json"))
    print("=====", d.split("/")[-1])
    for n, s in r["scenarios"].items():
        print("--", n)
        if "exception" in s: print("   EXC", s["exception"]); continue
        for k in keys:
            if k in s and s[k] not in (None, [], {}): print(f"   {k}: {json.dumps(s[k])}")

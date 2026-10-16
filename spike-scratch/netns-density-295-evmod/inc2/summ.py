#!/usr/bin/env python3
"""Compact per-scenario summary of vmrun logs. Usage: summ.py out/x.txt [keys...]"""
import json, re, sys
txt = open(sys.argv[1], errors='replace').read()
KEYS = ['reader_end','writer_outcomes_all','sender_outcomes','abort_propagation_ms(cut: target-app cut -> source writer end)','abort_propagation_ms(rst: source RST -> target reader end)','unload','per_trial_series_after_close','leak_check','module_err','pairs_aborted','pairs_completed','live_end','oom_or_bug_lines','ktls']
for m in re.finditer(r'SUMMARY (\{.*\})\n', txt):
    d = json.loads(m.group(1))
    print('==', d['scenario'], 'wall', d['wall_s'], d['outcomes'], 'stream_ok', d.get('main_stream_ok'), 'eof', d.get('eof_seen'), 'extra', d.get('extra_after_eof'))
    for k in KEYS:
        if k in d and d[k] not in (None, {}, []): print('  ', k, json.dumps(d[k])[:700])

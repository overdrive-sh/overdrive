#!/usr/bin/env python3
"""Print the per-scenario SUMMARY lines of vmrun.py logs as a compact table. Usage: report_vm.py out/vm-A.txt ..."""
import json, re, sys
KEYS = ['main_stream_ok','main_hash_eq','eof_seen','extra_after_eof','back_ok','sender_back_ok','outcomes','peak_pipe_MiB(src_inq+tgt_outq+peer_inq)',
        'guest_unreclaim_rise_MiB(max-start)','guest_avail_drop_MiB(start-min)','guest_unreclaim_after_close_minus_start_MiB','sender_max_send_ms','writer_max_send_ms',
        'sender_sent_at_longest_block_start_MiB','writer_sent_at_longest_block_start_MiB','resume_latency_ms(stall_end->sender_unblocked)',
        'host_unreclaim_rise_MiB(max-start)','host_peak_vsock_inq_KiB','module_err','pairs_aborted','pairs_completed','live_end','short_consume','fault_hits','main_stream_first_bad',
        'reader_MiBps','sender_MiBps','cpu_core_s_per_GiB','tasks_start','tasks_max','oom_or_bug_lines','trial_end']
for path in sys.argv[1:]:
    txt = open(path, errors='replace').read()
    for m in re.finditer(r'SUMMARY (\{.*\})\n', txt):
        d = json.loads(m.group(1))
        print('==', d['scenario'], 'probe=' + d.get('probe', ''), 'wall=', d.get('wall_s'))
        for k in KEYS:
            if k in d and d[k] not in (None, {}, [], 'None'):
                print('   ', k, '=', json.dumps(d[k]))
        if d.get('unload_output'):
            print('    unload_output =', d['unload_output'][-900:].replace('\n', ' | '))

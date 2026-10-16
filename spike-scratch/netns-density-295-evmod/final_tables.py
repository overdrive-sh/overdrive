#!/usr/bin/env python3
import json, re, sys
def S(path):
    out = {}
    for m in re.finditer(r'SUMMARY (\{.*\})\n', open(path, errors='replace').read()):
        d = json.loads(m.group(1)); out[d['scenario']] = d
    return out
def r(x, k='p50'):
    return '-' if not x else x[k]
def rng(x):
    return '-' if not x else f"{x['min']:g}/{x['p50']:g}/{x['max']:g}"
logs = {'A': S('out/vm-A.txt'), 'T': S('out/vm-T.txt'), 'G': S('out/vm-G.txt')}
ctl = S('out/vm-ctl.txt')
print("### (a) bounded memory: non-reading target, byte-verified")
print("| scenario | dir | trials | stream ok | pipeline MiB (min/p50/max) | guest SUnreclaim rise MiB (max) | host SUnreclaim rise MiB (max) | sender/writer longest block ms | bytes accepted before block MiB | resume ms (p50) |")
print("|---|---|---:|---:|---|---:|---:|---|---|---:|")
for name in ['stall', 'long', 'slow', 'cyc', 'bidir', 'burst']:
    for d in 'ATG':
        s = logs[d].get(f'ev-{name}-{d}')
        if not s: continue
        blk = s.get('sender_max_send_ms') or s.get('writer_max_send_ms')
        acc = s.get('sender_sent_at_longest_block_start_MiB') or s.get('writer_sent_at_longest_block_start_MiB')
        print(f"| ev-{name}-{d} | {d} | {s['trial_end']} | {s['main_stream_ok']} | {rng(s['peak_pipe_MiB(src_inq+tgt_outq+peer_inq)'])} | {r(s['guest_unreclaim_rise_MiB(max-start)'],'max')} | {r(s.get('host_unreclaim_rise_MiB(max-start)'),'max')} | {rng(blk)} | {rng(acc)} | {r(s.get('resume_latency_ms(stall_end->sender_unblocked)'))} |")
for k, s in ctl.items():
    if k.startswith('ctl'):
        print(f"| {k} (sockmap control) | {s['dir']} | {s['trial_end']} | n/a | peak_queued {rng(s['peak_queued_MiB'])} (abort cap 160) | {r(s['unreclaim_rise_MiB(max-start)'],'max')} | - | - | - | - |")

print("\n### (b) correctness (module build 9eebe7fe; every row is a fresh pair per trial)")
print("| scenario | trials | main stream byte-exact | verified GiB | FIN seen at target | extra bytes after FIN | reverse stream ok | first_bad (fault runs) | module err | short_consume |")
print("|---|---:|---:|---:|---:|---:|---:|---|---:|---:|")
for name in ['stall', 'long', 'slow', 'cyc', 'bidir', 'eof', 'tiny', 'burst', 'fault1', 'fault2', 'fault3']:
    for d in 'ATG':
        s = logs[d].get(f'ev-{name}-{d}')
        if not s: continue
        back = s['back_ok'] if s['back_ok'] != '0/0' else s['sender_back_ok']
        fb = s.get('main_stream_first_bad') or ''
        print(f"| ev-{name}-{d} | {s['trial_end']} | {s['main_stream_ok']} | {s['bytes_verified_MiB']/1024:.2f} | {s['eof_seen']} | {s['extra_after_eof']} | {back} | {fb} | {s['module_err']} | {s['short_consume']} |")
print("\n### single-flow throughput (fast reader, count only)")
for d in 'ATG':
    s = logs[d].get(f'ev-tp-{d}')
    print(d, 'reader MiB/s', rng(s['reader_MiBps']), 'trial secs', rng(s['trial_secs']), 'guest busy ms', rng(s['sys_busy_ms']))
for k in ['tp-sockmap-A', 'tp-pump-A', 'tp-sockmap-T', 'tp-pump-T']:
    s = ctl[k]; print(k, 'reader MiB/s', rng(s['reader_MiBps']), 'cpu core-s/GiB', rng(s['cpu_core_s_per_GiB']))

#!/usr/bin/env python3
"""Read retained native receipts into an explicit measured summary; no native operations."""
import gzip,json,pathlib
SCRATCH=pathlib.Path(__file__).parent
def read(path):return path.read_text() if path.exists() else gzip.decompress(path.with_name(path.name+'.gz').read_bytes()).decode()
results=[]
for inc in sorted(SCRATCH.glob('increment-*')):
    path=inc/'evidence/events.jsonl'
    if not path.exists() and not path.with_name(path.name+'.gz').exists():continue
    events=[json.loads(line) for line in read(path).splitlines()]
    row={'attempt':inc.name,'source_identity':next((e for e in events if e['event']=='identity'),None),'admission':next((e for e in events if e['event']=='admission-budget'),None),'stages':[e for e in events if e['event']=='stage'],'transport_summary':next((e for e in events if e['event']=='transport-summary'),None),'final_full_pool_recheck':next((e for e in events if e['event']=='final-full-pool-recheck'),None),'cleanup':next((e for e in events if e['event']=='cleanup'),None),'host_oom_counter':next((e for e in events if e['event']=='host-oom-counter'),None),'final':next((e for e in events if e['event']=='final'),None),'actual_vmm_spawn_events':sum(e['event']=='vmm-spawn' for e in events),'actual_guest_ready_and_host_http_events':sum(e['event']=='guest-ready' for e in events),'failure':next((e for e in events if e['event']=='probe-failure'),None)}
    resources=inc/'evidence/live-process-resources.json'
    if resources.exists() or resources.with_name(resources.name+'.gz').exists():
        proc=json.loads(read(resources));row['two_vm_resources']={}
        for role,value in proc['vmms'].items():
            def field(content,name):return int(next(s.split()[1] for s in content.splitlines() if s.startswith(name+':')))
            row['two_vm_resources'][role]={'rss_kib':field(value['status'],'VmRSS'),'pss_kib':field(value['smaps_rollup'],'Pss') if 'smaps_rollup' in value else None,'fds':value['fd_count'],'threads':field(value['status'],'Threads')}
    results.append(row)
(SCRATCH/'measurement-summary.json').write_text(json.dumps(results,indent=2,sort_keys=True))
print(json.dumps({'attempts_summarized':len(results),'successful_final_attempts':[r['attempt'] for r in results if r['final'] and r['final']['passed']]}))

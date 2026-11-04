#!/usr/bin/env python3
"""Reads BCJSON lines (stdout of run.sh run) and prints p50/p90/max tables. Usage: analyze.py log"""
import sys, json, statistics, collections
rows=[json.loads(l.split('BCJSON ',1)[1]) for l in open(sys.argv[1]) if 'BCJSON ' in l]
rows=[r for r in rows if not r['warm']]
def pct(v,p):
    v=sorted(v); k=(len(v)-1)*p/100; f=int(k); c=min(f+1,len(v)-1)
    return v[f]+(v[c]-v[f])*(k-f)
def st(v): return (pct(v,50),pct(v,90),max(v),min(v),statistics.mean(v))
by=collections.defaultdict(list)
for r in rows: by[r['variant']].append(r)
def spans(r):
    d={}
    for l,s,du in r['bc']:
        d.setdefault(l,[]).append((s,du))
    return d
def derive(r):
    d=spans(r); o={}
    o['host_vmm_start_to_READY']=r['ready_ms']
    o['host_vmm_start_to_beacon_connect']=r['conn_ms']
    if not r['bc']: return o
    s=lambda l: d[l][0][1]/1e6
    o['prep:total']=s('prep:total')
    o['prep:Ebpf::load(guest obj)']=s('prep:ebpf_load_guest_obj')
    o['prep:Ebpf::load(candidate obj)']=s('prep:ebpf_load_candidate_obj')
    # remainder of prep:total = mount bpffs + staging + active storage + init maps
    o['prep:other(mount,staging,maps)']=o['prep:total']-o['prep:Ebpf::load(guest obj)']-o['prep:Ebpf::load(candidate obj)']
    loads=[k for k in d if k.startswith('load:') and not k.startswith('load:sub:')]
    atts=[k for k in d if k.startswith('attach:') and not k.startswith('attach:sub:')]
    for k in loads: o[k.replace('load:','verifier+load:')]=s(k)
    for k in atts: o[k]=s(k)
    o['SUM program loads (verifier)']=sum(s(k) for k in loads)
    o['SUM program attaches']=sum(s(k) for k in atts)
    o['load:sub:btf_from_sys_fs (inside candidate load)']=s('load:sub:btf_from_sys_fs')
    o['attach:sub:cgroup_file (first attach mounts cgroup2)']=d['attach:sub:cgroup_file'][0][1]/1e6
    o['PRE-READY BPF TOTAL (prep+loads+attaches)']=o['prep:total']+o['SUM program loads (verifier)']+o['SUM program attaches']
    o['transport_total (configure_guest_transport)']=s('transport_total')
    o['phase:network_total']=s('phase:network_total')
    o['phase:modules']=s('phase:modules'); o['phase:root']=s('phase:root'); o['phase:connect_beacon']=s('phase:connect_beacon')
    o['phase:owner']=s('phase:owner'); o['phase:ready_send']=s('phase:ready_send')
    t0=d['init_start'][0][0]
    o['init_start_to_ready_sent']=(d['ready_sent'][0][0]-t0)/1e6
    o['guest_kernel_boot_to_init_start (CLOCK_MONOTONIC)']=t0/1e6
    return o
D={v:[derive(r) for r in rs] for v,rs in by.items()}
labels=collections.OrderedDict()
for v in D:
    for o in D[v]:
        for k in o: labels.setdefault(k,1)
out=[]
def P(s=''): out.append(s); print(s)
P(f"samples per variant (measured, warmup excluded): "+", ".join(f"{v}={len(D[v])}" for v in D))
P()
for v in ['R','A','A2','B']:
    if v not in D: continue
    P(f"### variant {v}: p50 / p90 / max / min / mean (ms)")
    for k in labels:
        vals=[o[k] for o in D[v] if k in o]
        if len(vals)<len(D[v]): continue
        p50,p90,mx,mn,me=st(vals)
        P(f"{k:58s} {p50:9.3f} {p90:9.3f} {mx:9.3f} {mn:9.3f} {me:9.3f}")
    P()
def delta(a,b):
    P(f"### delta {b} - {a}  (p50 of per-metric distributions, and paired by round: median / p90 / max of per-round difference) (ms)")
    for k in labels:
        if not all(k in o for o in D[a]+D[b]): continue
        va=[o[k] for o in D[a]]; vb=[o[k] for o in D[b]]
        pa=pct(va,50); pb=pct(vb,50)
        diffs=[y-x for x,y in zip(va,vb)]  # rounds are aligned (interleaved)
        P(f"{k:58s} p50Δ={pb-pa:8.3f} p90Δ={pct(vb,90)-pct(va,90):8.3f}  paired: med={pct(diffs,50):8.3f} p90={pct(diffs,90):8.3f} max={max(diffs):8.3f}")
    P()
delta('A','B'); delta('A2','B'); delta('A','A2')
if 'R' in D and 'A' in D:
    P("### instrumentation control: host VMM-start->READY, real unmodified init R vs instrumented A")
    ra=[o['host_vmm_start_to_READY'] for o in D['R']]; aa=[o['host_vmm_start_to_READY'] for o in D['A']]
    P(f"R p50={pct(ra,50):.3f} p90={pct(ra,90):.3f} max={max(ra):.3f};  A p50={pct(aa,50):.3f} p90={pct(aa,90):.3f} max={max(aa):.3f};  A-R p50Δ={pct(aa,50)-pct(ra,50):.3f}")
open(sys.argv[1].rsplit('.',1)[0]+'-summary.txt','w').write("\n".join(out)+"\n")

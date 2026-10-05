#!/usr/bin/env python3
"""Combine only explicitly compatible completed native datasets; no pilot rows."""
import csv,json,pathlib,collections,statistics
BASE=pathlib.Path(__file__).resolve().parent;OUT=BASE/'comparison';OUT.mkdir(exist_ok=True)
def load(inc,name):
 p=BASE/inc/'derived'/name
 if not p.exists():return []
 return list(csv.DictReader(p.open()))
rows=[r for r in load('increment-c','samples.csv')if r['kind']=='tcp-rr'and r['variant']=='kernel']+[r for r in load('increment-e','samples.csv')if r['kind']=='tcp-rr'and r['variant']=='aya']+[r for r in load('increment-h','samples.csv')if r['protocol']=='udp']+[r for r in load('increment-k','samples.csv')if r['protocol']=='udp']
# The repaired common path is h; earlier UDP paths remain original evidence only.
resources=load('increment-c','resources.csv')+load('increment-e','resources.csv')+load('increment-h','resources.csv')+load('increment-k','resources.csv');idle=load('increment-h','idle-cpu.csv')+load('increment-k','idle-cpu.csv')
def write(name,data):
 if not data:return
 fields=list(dict.fromkeys(k for r in data for k in r))
 with(OUT/name).open('w')as f:w=csv.DictWriter(f,fieldnames=fields);w.writeheader();w.writerows(data)
write('samples.csv',rows);write('resources.csv',resources);write('idle-cpu.csv',idle)
metrics=['application_GiB_per_s','delivered_datagrams_per_s','vm_mean_busy_cores','vm_core_seconds_per_delivered_GiB','vm_cpu_us_per_delivered_datagram','p95_us','p99_us','loss_host_guest','loss_guest_host','participating_peers','maximum_inflight_datagrams','maximum_inflight_peers','actual_offered_per_s','actual_sent_per_s','actor_user_cpu_s','actor_system_cpu_s']
groups=collections.defaultdict(list)
for r in rows:groups[tuple(r[k]for k in ['kind','variant','population','eligible_active','payload','target_per_s'])].append(r)
summary=[]
for key,rs in sorted(groups.items()):
 r=dict(zip(['kind','variant','population','eligible_active','payload','target_per_s'],key));r['measured_windows']=len(rs);r['private_vm_cohorts']=len(set((x['source_increment'],x['run'])for x in rs))
 for metric in metrics:
  xs=[float(x[metric])for x in rs if x.get(metric)not in [None,'']]
  for stat,fn in [('median',statistics.median),('min',min),('max',max)]:r[metric+'_'+stat]=fn(xs)if xs else None
 summary.append(r)
write('summary.csv',summary)
idle_summary=[]
for key in sorted(set((r['variant'],r['protocol'],r['population'])for r in idle)):
 rs=[r for r in idle if (r['variant'],r['protocol'],r['population'])==key];vals=[float(r['vm_mean_busy_cores'])for r in rs];idle_summary.append({'variant':key[0],'protocol':key[1],'population':key[2],'intervals':len(vals),'busy_cores_median':statistics.median(vals),'busy_cores_min':min(vals),'busy_cores_max':max(vals)})
write('idle-summary.csv',idle_summary)
(OUT/'source-selection.json').write_text(json.dumps({'tcp_module':'increment-c completed 00-kernel-tcp, validated afterward by e corrected preflight; byte-identical window path','tcp_aya':'increment-e completed 01-aya-tcp','udp_comparison':'compatible increment-h/increment-k common corrected path; failed pressure windows excluded','idle_cpu':'increment-h/increment-k exact stat/sleep/stat intervals only','excluded':'a pilot; b sync failure; c failed Aya gate/planned UDP; all c/e original idle CPU; d/g compile failures; e UDP paths with failed epoch accounting; f prepared but not executed','physical_vs_private_cpu':'private busy-CPU primary; QEMU/physical/cgroup observations secondary, separate scopes'},indent=2))
print(json.dumps({'valid_rows':len(rows),'groups':len(summary),'idle_intervals':len(idle),'output':str(OUT)}))
import matplotlib;matplotlib.use('Agg');import matplotlib.pyplot as plt
colors={'kernel':'#af4e38','aya':'#2169a6'}
for kind,size in [('tcp-rr',64),('tcp-rr',65536),('udp-open',64),('udp-open',1431),('udp-open',59000)]:
 fig,ax=plt.subplots(1,3,figsize=(14,4))
 for variant in ['kernel','aya']:
  selected=sorted([r for r in summary if r['kind']==kind and r['variant']==variant and int(r['payload'])==size and int(r['target_per_s'])==0 and r['eligible_active']==r['population']],key=lambda r:int(r['population']))
  if not selected:continue
  x=[int(r['population'])for r in selected]
  for a,metric in zip(ax,['application_GiB_per_s','vm_mean_busy_cores','p99_us']):
   y=[r[metric+'_median']for r in selected];a.plot(x,y,'o-',label=variant,color=colors[variant]);a.fill_between(x,[r[metric+'_min']for r in selected],[r[metric+'_max']for r in selected],alpha=.12,color=colors[variant])
 for a,title in zip(ax,['Delivered application GiB/s','Whole-private-VM busy cores','Observed roundtrip p99 (µs)']):a.set_xscale('log',base=2);a.set_xlabel('Actual held peers (whole pool eligible)');a.set_title(title);a.grid(alpha=.2);a.legend()
 fig.suptitle(f'{kind}, {size} bytes: median and measured-window range; eligibility is not concurrent activity');fig.tight_layout();fig.savefig(OUT/f'{kind}-{size}-population.svg');plt.close(fig)
for population in [1,64,1024,4096,16384]:
 for payload in [64,59000]:
  selected=[r for r in summary if r['kind']=='udp-open'and int(r['population'])==population and int(r['payload'])==payload and int(r['target_per_s'])>0 and r['eligible_active']==r['population']]
  if not selected:continue
  fig,ax=plt.subplots(1,3,figsize=(13,4))
  for variant in ['kernel','aya']:
   rs=sorted([r for r in selected if r['variant']==variant],key=lambda r:r['actual_sent_per_s_median']);x=[r['actual_sent_per_s_median']for r in rs]
   for a,m in zip(ax,['p95_us','p99_us','vm_mean_busy_cores']):a.plot(x,[r[m+'_median']for r in rs],'o-',label=variant,color=colors[variant]);a.set_xscale('log');a.grid(alpha=.2);a.legend()
  for a,title in zip(ax,['Observed p95 (µs)','Observed p99 (µs)','Whole-private-VM busy cores']):a.set_xlabel('Actual ordinary UDP sends/s');a.set_title(title)
  fig.suptitle(f'UDP {payload} bytes, {population} held/eligible peers; delivered subset latency');fig.tight_layout();fig.savefig(OUT/f'udp-{payload}-load-{population}.svg');plt.close(fig)
fig,ax=plt.subplots(1,3,figsize=(14,4))
for protocol in ['tcp','udp']:
 for variant in ['kernel','aya']:
  # A single common h resource cohort per protocol/population supplies the final
  # memory curve; before h completion retain c/e TCP evidence as preliminary.
  rs=load('increment-h','resources.csv')+load('increment-k','resources.csv');rs=[r for r in rs if r['tag']=='interval-start'and r['protocol']==protocol and r['variant']==variant]
  if not rs and protocol=='tcp':rs=[r for r in load('increment-c'if variant=='kernel'else'increment-e','resources.csv')if r['tag']=='interval-start'and r['protocol']==protocol and r['variant']==variant]
  unique={int(r['population']):r for r in rs};rs=[unique[n]for n in sorted(unique)];x=[int(r['population'])for r in rs]
  if not rs:continue
  for a,m in zip(ax,['sunreclaim_delta_loaded_kib','kernel_stack_delta_loaded_kib','pss_delta_loaded_kib']):a.plot(x,[float(r[m])/1024 for r in rs],'o-',label=variant+' '+protocol)
for a,title in zip(ax,['Kernel SUnreclaim delta (MiB)','KernelStack delta (MiB)','Process PSS delta (MiB)']):a.set_xscale('log',base=2);a.set_xlabel('Actual held transport peers');a.set_title(title);a.grid(alpha=.2);a.legend()
fig.suptitle('Separate native memory components relative to loaded baseline; no invented combined total');fig.tight_layout();fig.savefig(OUT/'memory-population.svg');plt.close(fig)
if idle_summary:
 fig,ax=plt.subplots(figsize=(7,4))
 for protocol in ['tcp','udp']:
  for variant in ['kernel','aya']:
   rs=sorted([r for r in idle_summary if r['protocol']==protocol and r['variant']==variant],key=lambda r:int(r['population']));ax.plot([int(r['population'])for r in rs],[r['busy_cores_median']for r in rs],'o-',label=variant+' '+protocol)
 ax.set_xscale('log',base=2);ax.set_xlabel('Actual held transport peers');ax.set_ylabel('Whole-VM busy cores during exact idle sleep');ax.grid(alpha=.2);ax.legend();fig.tight_layout();fig.savefig(OUT/'idle-cpu-population.svg');plt.close(fig)

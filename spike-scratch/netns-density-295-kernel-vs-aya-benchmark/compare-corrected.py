#!/usr/bin/env python3
"""Compare only the matched corrected UDP cohort; frozen original results stay separate."""
import csv,json,pathlib,collections,statistics
BASE=pathlib.Path(__file__).resolve().parent;INC=BASE/'increment-r';OUT=BASE/'comparison/corrected-udp';OUT.mkdir(parents=True,exist_ok=True)
def load(name):return list(csv.DictReader((INC/'derived'/name).open()))
def write(name,rows):
 if not rows:return
 with(OUT/name).open('w')as f:
  w=csv.DictWriter(f,fieldnames=list(dict.fromkeys(k for r in rows for k in r)));w.writeheader();w.writerows(rows)
rows=load('samples.csv');resources=load('resources.csv');idle=load('idle-cpu.csv');write('memory-stages.csv',load('memory-stages.csv'));write('posttraffic-settle.csv',load('posttraffic-settle.csv'));write('cohort-accounting.csv',load('cohort-accounting.csv'));write('samples.csv',rows);write('resources.csv',resources);write('idle-cpu.csv',idle)
metrics=['application_GiB_per_s','delivered_datagrams_per_s','vm_mean_busy_cores','vm_core_seconds_per_delivered_GiB','vm_cpu_us_per_delivered_datagram','p50_us','p95_us','p99_us','loss_host_guest','loss_guest_host','participating_peers','maximum_inflight_datagrams','maximum_inflight_peers','actual_offered_per_s','actual_sent_per_s','sent_per_total_measured_s','offered_per_total_measured_s','qemu_cpu_seconds_approx','actor_user_cpu_s','actor_system_cpu_s']
keys=['candidate','kind','variant','population','eligible_active','payload','target_per_s'];groups=collections.defaultdict(list)
for r in rows:groups[tuple(r[k]for k in keys)].append(r)
summary=[]
for key,rs in sorted(groups.items()):
 r=dict(zip(keys,key));r['measured_windows']=len(rs);r['private_vm_cohorts']=len(set(x['run']for x in rs));r['initial_all_peer_burst']=rs[0]['initial_all_peer_burst']
 for metric in metrics:
  xs=[float(x[metric])for x in rs if x.get(metric)not in [None,'']]
  for name,fn in [('median',statistics.median),('min',min),('max',max)]:r[metric+'_'+name]=fn(xs)if xs else None
 summary.append(r)
write('summary.csv',summary)
resource_summary=[]
for key in sorted(set((r['candidate'],r['variant'],r['population'])for r in resources if r['tag']=='interval-start')):
 rs=[r for r in resources if(r['candidate'],r['variant'],r['population'])==key and r['tag']=='interval-start'];r=dict(zip(['candidate','variant','population'],key));r['cohorts']=len(rs)
 for metric in ['sunreclaim_delta_loaded_kib','kernel_stack_delta_loaded_kib','page_tables_delta_loaded_kib','pss_delta_loaded_kib','rss_kib','fds','self_tasks','all_tasks','memavailable_delta_loaded_kib']:
  xs=[float(x[metric])for x in rs]
  for name,fn in [('median',statistics.median),('min',min),('max',max)]:r[metric+'_'+name]=fn(xs)
 resource_summary.append(r)
write('resource-summary.csv',resource_summary)
idle_summary=[]
for key in sorted(set((r['candidate'],r['variant'],r['population'])for r in idle)):
 rs=[r for r in idle if(r['candidate'],r['variant'],r['population'])==key];xs=[float(r['vm_mean_busy_cores'])for r in rs];r=dict(zip(['candidate','variant','population'],key));r.update({'intervals':len(xs),'cohorts':len(set(x['run']for x in rs)),'busy_cores_median':statistics.median(xs),'busy_cores_min':min(xs),'busy_cores_max':max(xs)});idle_summary.append(r)
write('idle-summary.csv',idle_summary)
setup_summary=[]
coverage=json.loads((INC/'derived/coverage.json').read_text())
for variant in ['kernel','aya']:
 for population in [1,64,1024,4096,16384]:
  cells=[v for v in coverage if v['event']=='setup'and v['population']==population and v['run'].split('-')[1]==variant]
  if not cells:continue
  r={'variant':variant,'population':population,'cohorts':len(cells),'new_owners':cells[0]['new_owners']}
  for metric,get in [('setup_s',lambda v:v['duration_s']),('new_endpoints_per_s',lambda v:v['new_owners']/v['duration_s'])]:
   xs=[get(v)for v in cells]
   for name,fn in [('median',statistics.median),('min',min),('max',max)]:r[metric+'_'+name]=fn(xs)
  setup_summary.append(r)
write('setup-summary.csv',setup_summary)
retirement=json.loads((INC/'derived/retirement.json').read_text());(OUT/'retirement.json').write_text(json.dumps(retirement,indent=2))

(OUT/'source-selection.json').write_text(json.dumps({'scope':'Matched UDP only: increment-r module original versus separately pinned Aya directional STREAM/SEQPACKET candidate','original_candidate':'Original increment-h/k data and failures retained in comparison/ and original raw evidence; not merged into corrected rows','matrix':json.loads((INC/'run-matrix.json').read_text()),'timer_scope':'Whole-private-VM busy CPU/stat boundaries around common ready/start/finish. perf leading30ms includes broader software task-clock scope. Physical QEMU CPU interpolation approximate0.5s sample; separate from private kernel scope.','activity':'Eligible-all large populations include one request per peer before global endpoint-service barrier. Maximum pending peers proves actual simultaneous application requests awaiting reply, not all kernel queues being simultaneously full.','offered_rate_scope':'Raw offering_duration_s is the declared pacing interval. All-peer burst guarantees one request per actual peer and may extend it under generator saturation; actual offered/s over complete measured traffic+delivery duration is separately derived and used for load plots.','generator':'Common epoll_wait0 busy polling consumes CPU at paced loads; efficiency includes generators and control. Unrestricted rates are measured complete-pipeline results, not isolated technology peak.'},indent=2))
import matplotlib;matplotlib.use('Agg');import matplotlib.pyplot as plt
colors={'kernel':'#af4e38','aya':'#2169a6'};labels={'kernel':'C module original','aya':'Aya corrected duplex'}
for size in [0,64,1431,59000]:
 fig,ax=plt.subplots(1,3,figsize=(14,4))
 for variant in ['kernel','aya']:
  rs=sorted([r for r in summary if r['kind']=='udp-open'and r['variant']==variant and int(r['payload'])==size and r['target_per_s']=='0'and r['eligible_active']==r['population']],key=lambda r:int(r['population']));x=[int(r['population'])for r in rs]
  for a,m in zip(ax,['delivered_datagrams_per_s','vm_mean_busy_cores','p99_us']):
   valid=[r for r in rs if r[m+'_median']is not None];a.plot([int(r['population'])for r in valid],[r[m+'_median']for r in valid],'o-',label=labels[variant],color=colors[variant]);a.fill_between([int(r['population'])for r in valid],[r[m+'_min']for r in valid],[r[m+'_max']for r in valid],alpha=.12,color=colors[variant])
 for a,title in zip(ax,['Delivered datagrams/s (sum of directions)','Whole-private-VM busy cores','Delivered roundtrip p99 (µs)']):a.set_xscale('log',base=2);a.set_xlim(.8,20000);a.set_xlabel('Actual held and participating peers');a.set_title(title);a.grid(alpha=.2);a.legend()
 if size==0:ax[2].text(.1,.5,'Empty UDP open loop: count/loss only',transform=ax[2].transAxes)
 fig.suptitle(f'Corrected matched UDP {size}B: AB/BA/AB median and range; large all-peer burst retained');fig.tight_layout();fig.savefig(OUT/f'udp-{size}-population.svg');plt.close(fig)
for population in [1,64,1024,4096,16384]:
 for payload in [64,1431,59000]:
  rs=[r for r in summary if r['kind']=='udp-open'and int(r['population'])==population and int(r['payload'])==payload and int(r['target_per_s'])>0 and r['eligible_active']==r['population']]
  if not rs:continue
  fig,ax=plt.subplots(1,3,figsize=(14,4))
  for variant in ['kernel','aya']:
   cells=sorted([r for r in rs if r['variant']==variant],key=lambda r:r['sent_per_total_measured_s_median']);x=[r['sent_per_total_measured_s_median']for r in cells]
   for a,m in zip(ax,['p95_us','p99_us','vm_mean_busy_cores']):a.plot(x,[r[m+'_median']for r in cells],'o-',label=labels[variant],color=colors[variant]);a.set_xscale('log');a.grid(alpha=.2);a.legend()
  for a,title in zip(ax,['Delivered p95 (µs)','Delivered p99 (µs)','Whole-private-VM busy cores']):a.set_xlabel('App sends / measured traffic + delivery seconds');a.set_title(title)
  fig.suptitle(f'Corrected UDP {payload}B / {population} actual peers: delivered subset latency; measured loss separate');fig.tight_layout();fig.savefig(OUT/f'udp-{payload}-load-{population}.svg');plt.close(fig)
fig,ax=plt.subplots(1,3,figsize=(14,4))
for variant in ['kernel','aya']:
 rs=sorted([r for r in resource_summary if r['variant']==variant],key=lambda r:int(r['population']));x=[int(r['population'])for r in rs]
 for a,m in zip(ax,['sunreclaim_delta_loaded_kib','kernel_stack_delta_loaded_kib','pss_delta_loaded_kib']):a.plot(x,[r[m+'_median']/1024 for r in rs],'o-',label=labels[variant],color=colors[variant])
for a,title in zip(ax,['Kernel SUnreclaim delta (MiB)','KernelStack delta (MiB)','Process PSS delta (MiB)']):a.set_xscale('log',base=2);a.set_xlabel('Actual held transport peers');a.set_title(title);a.grid(alpha=.2);a.legend()
fig.suptitle('Corrected matched UDP: separate memory components; no invented total');fig.tight_layout();fig.savefig(OUT/'memory-population.svg');plt.close(fig)
print(json.dumps({'rows':len(rows),'groups':len(summary),'resources':len(resources),'idle':len(idle),'output':str(OUT)}))

fig,ax=plt.subplots(1,4,figsize=(18,4))
for a,payload in zip(ax,[0,64,1431,59000]):
 for variant in ['kernel','aya']:
  rs=[r for r in summary if r['kind']=='udp-open'and r['variant']==variant and int(r['payload'])==payload]
  metric='delivered_datagrams_per_s'if payload==0 else'application_GiB_per_s'
  a.scatter([r['vm_mean_busy_cores_median']for r in rs],[r[metric+'_median']for r in rs],label=labels[variant],color=colors[variant],alpha=.7,s=25)
 a.set_xlabel('Whole-private-VM busy cores');a.set_ylabel('Delivered datagrams/s'if payload==0 else'Delivered application GiB/s');a.set_title(f'{payload}B, measured populations/load/activity');a.grid(alpha=.2);a.legend()
fig.suptitle('Actual goodput vs CPU: complete measured windows, including endpoint/generator work');fig.tight_layout();fig.savefig(OUT/'goodput-cpu.svg');plt.close(fig)

#!/usr/bin/env python3
"""Derive comparison tables and standalone plots solely from captured native data."""
import argparse,csv,json,pathlib,re,statistics,collections,gzip
p=argparse.ArgumentParser();p.add_argument('increment');args=p.parse_args();base=pathlib.Path(__file__).resolve().parent;inc=base/args.increment;out=inc/'derived';out.mkdir(exist_ok=True)
import importlib.util
spec=importlib.util.spec_from_file_location('native_events',base/'native-events.py');native=importlib.util.module_from_spec(spec);spec.loader.exec_module(native)
uart_repairs=[];uart_invalid=[]
def events(path):
 rows,repairs,invalid=native.read_events(path)
 uart_repairs.extend({'run':path.parent.name,**r}for r in repairs);uart_invalid.extend({'run':path.parent.name,**r}for r in invalid)
 return rows
def cpu(s):
 x=list(map(int,s.splitlines()[0].split()[1:]));return sum(x[i]for i in [0,1,2,5,6]),x[7]
def mem(s):
 return {k:int(v.split()[0]) for line in s.splitlines()if ':'in line for k,v in [line.split(':',1)]if v.split()and v.split()[0].isdigit()}
def qcpu(row):
 fields=row['qemu_stat'].rsplit(')',1)[1].split();return (int(fields[11])+int(fields[12]))/row['clock_ticks']
def interpolate(samples,t):
 for a,b in zip(samples,samples[1:]):
  if a['physical_elapsed_s']<=t<=b['physical_elapsed_s']:
   f=(t-a['physical_elapsed_s'])/(b['physical_elapsed_s']-a['physical_elapsed_s']);return qcpu(a)+f*(qcpu(b)-qcpu(a))
 return None
rows=[];resources=[];retirement=[];audits=[];profiles=[];coverage=[];excluded=[];idle=[];excluded_idle=[];settling=[];cohort=[];memory_stages=[]
for directory in sorted((inc/'evidence').glob('[0-9][0-9]-*')):
 serial=directory/'native-serial.log'
 if not serial.exists()and not serial.with_name(serial.name+'.gz').exists():continue
 parsed=list(events(serial));begin=next((v for _,v in parsed if v.get('event')=='begin'),None)
 if not begin:continue
 hz=begin['clock_ticks'];variant=begin['variant'];protocol=begin['protocol'];candidate=begin.get('corrected_candidate',variant+'-'+protocol+'-original');loaded=next((v['snapshot']for _,v in parsed if v.get('event')=='loaded'),begin['baseline']);lm=mem(loaded['meminfo']);lp=mem(loaded['self_smaps_rollup'])
 samplepath=directory/'qemu-resources.jsonl';sampletext=samplepath.read_text()if samplepath.exists()else gzip.decompress(samplepath.with_name(samplepath.name+'.gz').read_bytes()).decode();samples=[json.loads(l)for l in sampletext.splitlines()];observed={}
 observer=directory/'event-observer.jsonl'
 if observer.exists()or observer.with_name(observer.name+'.gz').exists():
  observertext=observer.read_text()if observer.exists()else gzip.decompress(observer.with_name(observer.name+'.gz').read_bytes()).decode()
  for line in observertext.splitlines():
   r=json.loads(line)
   if r.get('id')and r['event']in ['window_begin','window_end']:observed[(r['id'],r['event'])]=r['physical_elapsed_s']
 for line,v in parsed:
  ev=v['event']
  stage_snap=v.get('snapshot') if ev in ['loaded','traffic_resources_before_settle','traffic_resources','owned_retirement','transport_drain_live_kernel'] else v.get('after') if ev=='idle_resources' else v.get('baseline') if ev=='begin' else None
  if stage_snap:
   m=mem(stage_snap['meminfo']);ps=mem(stage_snap['self_smaps_rollup']);st=mem(stage_snap['self_status']);memory_stages.append({'source_increment':args.increment,'run':directory.name,'candidate':candidate,'variant':variant,'stage':ev,'population':v.get('population',v.get('owners')),'slab_kib':m.get('Slab'),'sunreclaim_kib':m.get('SUnreclaim'),'sunreclaim_delta_loaded_kib':m.get('SUnreclaim',0)-lm.get('SUnreclaim',0),'kernel_stack_kib':m.get('KernelStack'),'page_tables_kib':m.get('PageTables'),'vmalloc_used_kib':m.get('VmallocUsed'),'memavailable_kib':m.get('MemAvailable'),'pss_kib':ps.get('Pss'),'pss_delta_loaded_kib':ps.get('Pss',0)-lp.get('Pss',0),'rss_kib':st.get('VmRSS'),'fds':stage_snap['self_fds'],'all_tasks':stage_snap['all_tasks']})
  if ev=='idle':excluded_idle.append({'run':directory.name,'serial_line':line,'population':v['population'],'declared_sleep_s':v['duration_s'],'actual_snapshot_span_s':float(v['after']['uptime'].split()[0])-float(v['before']['uptime'].split()[0]),'reason':'ending inventory scan inside CPU snapshot interval; retain memory/audit evidence only'})
  if ev=='posttraffic_settle':
   cb,sb=cpu(v['cpu_before']);ca,sa=cpu(v['cpu_after']);settling.append({'source_increment':args.increment,'run':directory.name,'candidate':candidate,'variant':variant,'population':v['population'],'duration_s':v['duration_s'],'validated_late_endpoint_messages':v['validated_late_endpoint_messages'],'vm_busy_cpu_s':(ca-cb)/hz,'vm_mean_busy_cores':(ca-cb)/hz/v['duration_s'],'vm_steal_s':(sa-sb)/hz,'control_and_late_work_scope':True,'errors':json.dumps(v['errors'])})
  if ev=='idle_cpu':
   cb,sb=cpu(v['cpu_before']);ca,sa=cpu(v['cpu_after']);idle.append({'candidate':candidate,'source_increment':args.increment,'run':directory.name,'serial_line':line,'variant':variant,'protocol':protocol,'population':v['population'],'repeat':v['repeat'],'duration_s':v['duration_s'],'vm_busy_cpu_s':(ca-cb)/hz,'vm_mean_busy_cores':(ca-cb)/hz/v['duration_s'],'vm_steal_s':(sa-sb)/hz,'inventory_outside_interval':v['inventory_outside_interval']})
  if ev=='udp_window_fixture_failure':
   excluded.append({'run':directory.name,'serial_line':line,'id':v['id'],'reason':'native actor assertion failure: outcome/correctness violation; timing excluded','failures':v.get('failures'),'offered':v.get('offered'),'sent':v.get('sent'),'guest_delivered':v.get('guest_delivered'),'app_delivered':v.get('app_delivered')});continue
  if ev=='udp_open_window'and v.get('valid_timing_comparison')is False:
   excluded.append({'run':directory.name,'serial_line':line,'id':v['id'],'reason':'explicit native timing validity failure','late_previous_window_during_interval':v.get('late_previous_window_during_interval'),'failures':v.get('failures')});continue
  if ev=='udp_open_window'and args.increment=='increment-c':
   excluded.append({'run':directory.name,'serial_line':line,'id':v['id'],'reason':'post-send latency origin and per-window epoll/descriptor registration inside CPU interval','offered':v['offered'],'sent':v['sent'],'guest_delivered':v['guest_delivered'],'app_delivered':v['app_delivered'],'host_to_guest_loss':v['host_to_guest_loss'],'guest_to_host_loss':v['guest_to_host_loss']});continue
  if ev in ['window','udp_open_window']and v['repeat']<3:
   cb,sb=cpu(v['cpu_before']);ca,sa=cpu(v['cpu_after']);busy=(ca-cb)/hz;steal=(sa-sb)/hz
   isopen=ev=='udp_open_window';wall=v['total_delivery_duration_s']if isopen else v['duration_s'];size=v['application_size']if isopen else v['application_bytes_each_direction'];count=v['app_delivered']if isopen else v['delivered_roundtrips'];datagrams=(v['guest_delivered']+v['app_delivered'])if isopen else count*2
   delivered_bytes=(v['guest_delivered']+v['app_delivered'])*size if isopen else v['application_bytes_delivered'];gib=delivered_bytes/2**30
   qb=interpolate(samples,observed.get((v['id'],'window_begin'),-1));qe=interpolate(samples,observed.get((v['id'],'window_end'),-1))
   row={'candidate':candidate,'initial_all_peer_burst':v.get('initial_all_peer_burst',False),'all_peer_burst_service_barrier':v.get('all_peer_burst_service_barrier',False),'source_increment':args.increment,'run':directory.name,'serial_line':line,'kind':'udp-open'if isopen else protocol+'-rr','variant':variant,'protocol':protocol,'population':v['population'],'eligible_active':v['eligible_active']if isopen else v['active'],'participating_peers':v.get('participating_peers'),'maximum_inflight_datagrams':v.get('maximum_inflight_datagrams'),'payload':size,'target_per_s':v['offered_target_per_s']if isopen else v['offered_target_roundtrips_per_s'],'actual_offered_per_s':v['offered']/v['offering_duration_s']if isopen else count/wall,'declared_offering_seconds':v.get('offering_duration_s'),'offered_per_total_measured_s':v['offered']/wall if isopen else count/wall,'sent_per_total_measured_s':v['sent']/wall if isopen else count/wall,'actual_sent_per_s':v['sent']/v['offering_duration_s']if isopen else count/wall,'maximum_inflight_peers':v.get('maximum_inflight_peers'),'actor_user_cpu_s':v.get('actor_user_cpu_s'),'actor_system_cpu_s':v.get('actor_system_cpu_s'),'latency_sample_count':v.get('latency_sample_count',len(v.get('latency_ns',[]))),'repeat':v['repeat'],'duration_s':wall,'delivered_roundtrips':count,'application_bytes_delivered':delivered_bytes,'application_GiB_per_s':gib/wall,'delivered_datagrams_per_s':datagrams/wall,'vm_busy_core_seconds':busy,'vm_mean_busy_cores':busy/wall,'vm_steal_seconds':steal,'vm_core_seconds_per_delivered_GiB':busy/gib if gib else None,'vm_cpu_us_per_delivered_datagram':busy*1e6/datagrams if datagrams else None,'p50_us':v.get('p50_ns')/1000 if v.get('p50_ns')is not None else None,'p95_us':v.get('p95_ns')/1000 if v.get('p95_ns')is not None else None,'p99_us':v.get('p99_ns')/1000 if v.get('p99_ns')is not None else None,'offered':v.get('offered',count),'sent':v.get('sent',count),'guest_delivered':v.get('guest_delivered',count),'app_delivered':count,'loss_host_guest':v.get('host_to_guest_loss',0),'loss_guest_host':v.get('guest_to_host_loss',0),'qemu_cpu_seconds_approx':qe-qb if qb is not None and qe is not None else None,'corrupt':v['corrupt'],'boundary_violations':v['boundary_violations']};rows.append(row)
  if ev=='audit':
   snap=v['snapshot'];m=mem(snap['meminfo']);rss=mem(snap['self_status']);pss=mem(snap['self_smaps_rollup']);resources.append({'source_increment':args.increment,'run':directory.name,'candidate':candidate,'variant':variant,'protocol':protocol,'population':v['actual_vhost_devices'],'tag':v['tag'],'slab_kib':m.get('Slab'),'sunreclaim_kib':m.get('SUnreclaim'),'kernel_stack_kib':m.get('KernelStack'),'kernel_stack_delta_loaded_kib':m.get('KernelStack',0)-lm.get('KernelStack',0),'page_tables_kib':m.get('PageTables'),'page_tables_delta_loaded_kib':m.get('PageTables',0)-lm.get('PageTables',0),'vmallocused_kib':m.get('VmallocUsed'),'slab_delta_loaded_kib':m.get('Slab',0)-lm.get('Slab',0),'sunreclaim_delta_loaded_kib':m.get('SUnreclaim',0)-lm.get('SUnreclaim',0),'memavailable_delta_loaded_kib':lm.get('MemAvailable',0)-m.get('MemAvailable',0),'rss_kib':rss.get('VmRSS'),'pss_kib':pss.get('Pss'),'pss_delta_loaded_kib':pss.get('Pss',0)-lp.get('Pss',0),'fds':snap['self_fds'],'self_tasks':snap['self_tasks'],'all_tasks':snap['all_tasks']});audits.append({'run':directory.name,'serial_line':line,'population':v['actual_vhost_devices'],'tag':v['tag'],'independent_memory_contexts':v['independent_memory_contexts'],'cid_first':v['cid_first'],'cid_last':v['cid_last'],'backend':v['backend'],'udp_bindings':v.get('udp_bindings')})
  if ev=='transport_drain_live_kernel':
   start=begin['baseline'];end=v['snapshot'];cb,sb=cpu(start['proc_stat']);ca,sa=cpu(end['proc_stat']);elapsed=float(end['uptime'].split()[0])-float(start['uptime'].split()[0]);cohort.append({'source_increment':args.increment,'run':directory.name,'candidate':candidate,'variant':variant,'scope':'whole private harness cohort including loading/control/actors/kernel/idle/retirement/serialization; module initialization before baseline excluded','cohort_elapsed_s':elapsed,'vm_busy_cpu_s':(ca-cb)/hz,'vm_mean_busy_cores':(ca-cb)/hz/elapsed,'vm_steal_s':(sa-sb)/hz,'live_zero':v['confirmed_zero']})
  if ev in ['setup','idle','owned_retirement','transport_drain_live_kernel','correctness_gate_passed','preflight_tcp_backpressure_halfclose','preflight_owned_mapping_fail_closed']:
   if ev in ['owned_retirement','transport_drain_live_kernel']:retirement.append({'run':directory.name,'serial_line':line,**{k:x for k,x in v.items()if k not in ['snapshot','backend','modules']}})
   if ev in ['setup','correctness_gate_passed']:coverage.append({'run':directory.name,'serial_line':line,**v})
  if ev=='profile':profiles.append(v)
def write_csv(name,data):
 if not data:return
 with(out/name).open('w')as f:w=csv.DictWriter(f,fieldnames=list(data[0]));w.writeheader();w.writerows(data)
(out/'uart-deinterleave-receipt.json').write_text(json.dumps({'original_bytes_changed':False,'recovered_events':uart_repairs,'excluded_unrecoverable_events':uart_invalid},indent=2))
write_csv('memory-stages.csv',memory_stages);write_csv('posttraffic-settle.csv',settling);write_csv('cohort-accounting.csv',cohort);
write_csv('samples.csv',rows);write_csv('resources.csv',resources);write_csv('idle-cpu.csv',idle);(out/'excluded-idle-cpu.json').write_text(json.dumps(excluded_idle,indent=2))
(out/'retirement.json').write_text(json.dumps(retirement,indent=2));(out/'live-audits.json').write_text(json.dumps(audits,indent=2));(out/'coverage.json').write_text(json.dumps(coverage,indent=2));(out/'excluded-open-windows.json').write_text(json.dumps(excluded,indent=2));(out/'profiles.json').write_text(json.dumps(profiles,indent=2))
groups=collections.defaultdict(list)
for r in rows:groups[tuple(r[k]for k in ['kind','variant','population','eligible_active','payload','target_per_s'])].append(r)
summary=[]
for key,values in sorted(groups.items()):
 r=dict(zip(['kind','variant','population','eligible_active','payload','target_per_s'],key));r['repeats']=len(values)
 for field in ['application_GiB_per_s','delivered_datagrams_per_s','vm_mean_busy_cores','vm_core_seconds_per_delivered_GiB','vm_cpu_us_per_delivered_datagram','p95_us','p99_us','loss_host_guest','loss_guest_host','participating_peers','maximum_inflight_datagrams','actual_offered_per_s','actual_sent_per_s','maximum_inflight_peers','actor_user_cpu_s','actor_system_cpu_s']:
  xs=[x[field]for x in values if x[field]is not None];r[field+'_median']=statistics.median(xs)if xs else None;r[field+'_min']=min(xs)if xs else None;r[field+'_max']=max(xs)if xs else None
 summary.append(r)
write_csv('summary.csv',summary)
(out/'derivation.json').write_text(json.dumps({'increment':args.increment,'source':'original evidence/*/native-serial.log and qemu-resources.jsonl','measured_rows':len(rows),'groups':len(groups),'repeat_filter':'repeat < 3, excludes warmups/profile','cpu_primary':'whole-VM non-idle user+nice+system+irq+softirq ticks / CLK_TCK','qemu_scope':'interpolated 0.5-second physical-process CPU samples between lightweight window markers; approximate','latency':'actual endpoint request/reply nanoseconds; zero open-loop is count-only'},indent=2))
print(json.dumps({'rows':len(rows),'groups':len(groups),'output':str(out)}))
try:
 import matplotlib;matplotlib.use('Agg');import matplotlib.pyplot as plt
except ImportError:raise SystemExit('Tables generated; install matplotlib to generate standalone plots.')
colors={'kernel':'#ab4637','aya':'#2366ac'}
for kind,size in [('tcp-rr',65536),('udp-open',64),('udp-open',59000)]:
 fig,ax=plt.subplots(1,3,figsize=(14,4))
 for variant in ['kernel','aya']:
  selected=[r for r in summary if r['kind']==kind and r['variant']==variant and r['payload']==size and r['target_per_s']==0 and r['eligible_active']==r['population']]
  selected.sort(key=lambda r:r['population']);x=[r['population']for r in selected]
  ax[0].plot(x,[r['application_GiB_per_s_median']for r in selected],'o-',color=colors[variant],label=variant)
  ax[1].plot(x,[r['vm_mean_busy_cores_median']for r in selected],'o-',color=colors[variant],label=variant)
  ax[2].plot(x,[r['p99_us_median']for r in selected],'o-',color=colors[variant],label=variant)
 for a,title in zip(ax,['Application GiB/s','Whole-VM busy CPU cores','Observed roundtrip p99 (µs)']):a.set_xscale('log',base=2);a.set_xlabel('Actual held transport population');a.set_title(title);a.grid(alpha=.2);a.legend()
 fig.suptitle(f'{kind}, {size} application bytes, unrestricted common endpoint workload');fig.tight_layout();fig.savefig(out/f'{kind}-{size}-population.svg');plt.close(fig)
for population in [1,64,16384]:
 selected=[r for r in summary if r['kind']=='udp-open'and r['population']==population and r['payload']==64 and r['target_per_s']>0 and r['eligible_active']==population]
 if not selected:continue
 fig,ax=plt.subplots(1,2,figsize=(10,4))
 for variant in ['kernel','aya']:
  xs=sorted([r for r in selected if r['variant']==variant],key=lambda r:r['target_per_s']);x=[r['actual_sent_per_s_median']for r in xs]
  for a,field in zip(ax,['p95_us','p99_us']):a.plot(x,[r[field+'_median']for r in xs],'o-',label=variant,color=colors[variant]);a.set_xscale('log');a.set_xlabel('Actual ordinary UDP sends/sec');a.set_ylabel(field);a.grid(alpha=.2);a.legend()
 fig.suptitle(f'UDP 64-byte observed tail latency vs target load, population {population}');fig.tight_layout();fig.savefig(out/f'udp-tail-load-population-{population}.svg');plt.close(fig)
fig,ax=plt.subplots(1,2,figsize=(10,4))
for protocol in ['tcp','udp']:
 for variant in ['kernel','aya']:
  rs=sorted([r for r in resources if r['protocol']==protocol and r['variant']==variant and r['tag']=='interval-start'],key=lambda r:r['population']);x=[r['population']for r in rs];label=variant+' '+protocol
  ax[0].plot(x,[r['sunreclaim_delta_loaded_kib']/1024 for r in rs],'o-',label=label);ax[1].plot(x,[r['pss_delta_loaded_kib']/1024 for r in rs],'o-',label=label)
for a,title in zip(ax,['Kernel SUnreclaim delta from loaded (MiB)','Process PSS delta from loaded (MiB)']):a.set_xscale('log',base=2);a.set_xlabel('Actual held transport population');a.set_title(title);a.legend();a.grid(alpha=.2)
fig.tight_layout();fig.savefig(out/'memory-population.svg');plt.close(fig)

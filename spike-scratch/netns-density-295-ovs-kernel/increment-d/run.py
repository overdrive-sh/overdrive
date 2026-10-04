#!/usr/bin/env python3
"""No daemon: stock OVS kernel module, isolated namespace, direct UAPI flows."""
import gzip,hashlib,json,os,pathlib,resource,signal,subprocess,time,traceback,sys,struct
P=pathlib.Path;INC=P(__file__).resolve().parent;SCRATCH=INC.parent;OUT=SCRATCH/'out'/INC.name;OUT.mkdir(parents=True,exist_ok=False);START=time.perf_counter();LOG=(OUT/'events.jsonl').open('x');NS='ok295-'+str(os.getpid());PROC=None;MON=None;BEFORE=None;OK=False
def emit(event,**kw):
 d={'event':event,'at_s':time.perf_counter()-START,**kw};line=json.dumps(d,sort_keys=True);print(line,flush=True);LOG.write(line+'\n');LOG.flush()
def cmd(a,check=True,timeout=120,env=None):
 r=subprocess.run(a,capture_output=True,text=True,timeout=timeout,env=env)
 if check and r.returncode:raise RuntimeError(json.dumps({'argv':a,'rc':r.returncode,'stdout':r.stdout,'stderr':r.stderr}))
 return r
def raw(n,a):
 r=cmd(a,False);(OUT/n).write_text(json.dumps({'argv':a,'rc':r.returncode,'stdout':r.stdout,'stderr':r.stderr},indent=2));return r.stdout
def gz(n,data): (OUT/n).write_bytes(gzip.compress(json.dumps(data,sort_keys=True).encode(),mtime=0))
def sha(p):return hashlib.sha256(P(p).read_bytes()).hexdigest()
def snap(prefix):
 d={}
 for name,a in [('links',['ip','-j','-d','link']),('addresses',['ip','-j','address']),('routes',['ip','-j','route','show','table','all']),('rules',['ip','-j','rule']),('neighbors',['ip','-j','-s','neigh']),('nft',['nft','-j','list','ruleset']),('bpfmaps',['bpftool','-j','map','show']),('bpflinks',['bpftool','-j','link','show']),('netns',['ip','netns','list'])]:
  s=raw(prefix+'-'+name+'.json',a)
  try:d[name]=json.loads(s)
  except ValueError:d[name]=s
 d['sysctls']={p:P(p).read_text()for p in ['/proc/sys/net/ipv4/ip_forward','/proc/sys/net/ipv4/conf/all/rp_filter','/proc/sys/net/ipv4/neigh/default/gc_thresh1','/proc/sys/net/ipv4/neigh/default/gc_thresh2','/proc/sys/net/ipv4/neigh/default/gc_thresh3']if P(p).exists()};raw(prefix+'-modules.txt',['cat','/proc/modules']);raw(prefix+'-processes.txt',['ps','-e','-o','pid,ppid,comm,args']);return d
def norm(v,key=''):
 if isinstance(v,dict):return {k:norm(x,k)for k,x in sorted(v.items())if k not in {'valid_life_time','preferred_life_time','expires','used','confirmed','updated','stats','stats64','cache','packets','bytes','lastuse','state','probes','refcnt','gc_timer'}}
 if isinstance(v,list):return sorted([norm(x,key)for x in v],key=lambda x:json.dumps(x,sort_keys=True))
 return v
def na(a,**kw):return cmd(['ip','netns','exec',NS]+a,**kw)
def audit(n,label='stage'):
 available=int(next(x.split()[1]for x in P('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')));assert available>8*1024*1024,'bounded 8GiB reserve exhausted'
 ids=json.loads((OUT/f'identities-{n}.json').read_text());root=P('/proc')/str(PROC.pid);fdinfos={str(i['owner_fd']):(root/'fdinfo'/str(i['owner_fd'])).read_text()for i in ids};links=json.loads(na(['ip','-j','-d','link']).stdout)
 neighbors=json.loads(na(['ip','-j','neigh','show','dev','ok295dp']).stdout);assert len(neighbors)==16384 and all(x['state']==['PERMANENT']for x in neighbors);gz(f'{label}-{n}-neighbors.json.gz',neighbors)
 kernel=json.loads(na(['python3',str(INC/'audit.py'),str(DP)]).stdout);gz(f'{label}-{n}-kernel-dump.json.gz',kernel);gz(f'{label}-{n}-links.json.gz',links);gz(f'{label}-{n}-tap-fdinfo.json.gz',fdinfos)
 byname={l['ifname']:l for l in links};byport={x['port']:x for x in kernel['ports']};assert len(ids)==n and len({x['ifindex']for x in ids})==n and len(fdinfos)==n
 for i in ids:
  assert byname[i['name']]['ifindex']==i['ifindex'] and byname[i['name']]['linkinfo']['info_kind']=='tun' and byname[i['name']]['linkinfo']['info_data']['type']=='tap'
  assert byport[i['kernel_vport']]['name']==i['name'] and byport[i['kernel_vport']]['type']==1 and byport[i['kernel_vport']]['upcall_pids']==[0] and byport[i['kernel_vport']]['ifindex']==i['ifindex']
  assert 'iff:\t'+i['name']+'\n' in fdinfos[str(i['owner_fd'])]
 assert len(kernel['ports'])==n+1 and len(kernel['flows'])==2*n and len(links)==n+2
 assert all(p['upcall_pids']==[0]for p in kernel['ports'])
 res={'pid':PROC.pid,'fd_count':len(list((root/'fd').iterdir())),'threads':len(list((root/'task').iterdir())),'status':(root/'status').read_text(),'smaps_rollup':(root/'smaps_rollup').read_text(),'limits':(root/'limits').read_text(),'executable_sha256':sha(root/'exe'),'meminfo':P('/proc/meminfo').read_text(),'slabinfo':P('/proc/slabinfo').read_text()};gz(f'{label}-{n}-resources.json.gz',res)
 emit('independent-audit',held_taps=n,kernel_netdev_vports=n,local_internal_vports=1,exact_owner_fdinfo=n,distinct_ifindices=n,owned_permanent_neighbors=len(neighbors),output_only_flows=len(kernel['flows']),all_upcall_pids_zero=True,dp_stats=kernel['stats'],process_fds=res['fd_count'],process_threads=res['threads'],validated=True)
OOM=int(next(x.split()[1]for x in P('/proc/vmstat').read_text().splitlines()if x.startswith('oom_kill ')))
try:
 lease=P('/run/lock/overdrive-metal-shared.owner').read_text();assert 'scenario=spike-ovs-kernel-'+INC.name in lease
 emit('identity',pid=os.getpid(),uname=list(os.uname()),lease_owner=lease,namespace=NS,requested_taps=16384,guest_boots=0,userspace_forwarding_daemons=0,control='stock OVS Generic Netlink UAPI direct configuration')
 BEFORE=snap('before');MON=subprocess.Popen(['ip','-ts','monitor','neigh'],stdout=(OUT/'foreign-neighbor-monitor.txt').open('x'),stderr=subprocess.STDOUT)
 module=cmd(['modinfo','-n','openvswitch']).stdout.strip();(OUT/'module-file-sha256.json').write_text(json.dumps({'path':module,'sha256':sha(module),'bytes':P(module).stat().st_size}));raw('kernel-module-attestation.txt',['modinfo','openvswitch']);raw('host-before-slabinfo.txt',['cat','/proc/slabinfo']);raw('installed-tools.txt',['sh','-c','command -v ovs-dpctl; command -v ovs-vswitchd; true']);raw('host-before-meminfo.txt',['cat','/proc/meminfo']);raw('host-before-vmstat.txt',['cat','/proc/vmstat']);raw('rustc-version.txt',['rustc','-Vv'])
 env=os.environ.copy();env['CARGO_TARGET_DIR']=str(SCRATCH/'target');env.pop('RUSTFLAGS',None);r=cmd(['cargo','build','--release','--manifest-path',str(INC/'Cargo.toml')],timeout=600,env=env);(OUT/'build.txt').write_text(r.stdout+r.stderr);(OUT/'Cargo.lock.executed').write_bytes((INC/'Cargo.lock').read_bytes());binary=SCRATCH/'target/release/ovs-kernel-capacity-probe';emit('built',binary_sha256=sha(binary),rust_source_sha256=sha(INC/'src/main.rs'))
 cmd(['modprobe','openvswitch']);before_ovs=json.loads(cmd(['python3',str(INC/'audit.py'),'0']).stdout);gz('foreign-ovs-before.json.gz',before_ovs)
 cmd(['ip','netns','add',NS]);na(['sysctl','-w','net.ipv6.conf.all.disable_ipv6=1','net.ipv6.conf.default.disable_ipv6=1']);na(['ip','link','set','lo','up'])
 soft,hard=resource.getrlimit(resource.RLIMIT_NOFILE)
 def limits():resource.setrlimit(resource.RLIMIT_NOFILE,(hard,hard))
 PROC=subprocess.Popen(['ip','netns','exec',NS,str(binary),str(OUT)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=(OUT/'harness.stderr').open('x'),text=True,preexec_fn=limits)
 with (OUT/'harness.stdout').open('x')as log:
  for line in PROC.stdout:
   log.write(line);log.flush();print(line,end='',flush=True);d=json.loads(line)
   if d['event']=='dp_ready':
    DP=d['dp_ifindex'];PACKET_FAMILY=d['families']['packet'];na(['ip','link','set','ok295dp','address','02:aa:29:50:00:01']);na(['ip','addr','add','198.18.0.1/16','dev','ok295dp']);na(['ip','link','set','ok295dp','up']);batch=OUT/'owned-neighbors.batch'
    batch.write_text('\n'.join('neigh add 198.18.%d.%d lladdr 02:00:c6:12:%02x:%02x dev ok295dp nud permanent'%(((i+2)>>8)&255,(i+2)&255,((i+2)>>8)&255,(i+2)&255)for i in range(16384))+'\n');na(['ip','-batch',str(batch)]);emit('local-gateway-configured',permanent_owned_neighbors=16384,namespace_only=True,global_gc_tuning=False)
   if d['event']=='stage_held':audit(d['actual_tap_netdevices'])
   if d['event']=='paired_tcp_held':audit(d['held_taps'],'tcp-paired')
   if d['event']=='cleaned':OK=d['released_taps']==16384
   if d['event']in {'dp_ready','stage_held','paired_tcp_held','removed_flow_drop'}:PROC.stdin.write('continue\n');PROC.stdin.flush()
 rc=PROC.wait(timeout=30);wire=OUT/'netlink-wire.jsonl';sent=0;received=0;executes=0;packet_messages=0;counts={}
 for line in wire.open():
  w=json.loads(line)
  if w['direction']=='sent':
   sent+=1;counts[str(w['family'])+':'+str(w['command'])]=counts.get(str(w['family'])+':'+str(w['command']),0)+1
   executes+=int(w['family']==PACKET_FAMILY and w['command']==3)
  else:
   received+=1;rawbytes=bytes.fromhex(w['datagram_hex']);off=0
   while off+16<=len(rawbytes):
    length,family=struct.unpack_from('IH',rawbytes,off);packet_messages+=int(family==PACKET_FAMILY);off+=(length+3)&~3
 assert executes==0 and packet_messages==0;emit('netlink-wire-audit',actual_sent_datagrams=sent,actual_received_datagrams=received,requests_by_family_command=counts,packet_family=PACKET_FAMILY,packet_execute_requests=executes,packet_family_received_messages=packet_messages,capture_range='before first family resolution through acknowledged datapath deletion')
 emit('harness-exit',rc=rc,full_population_validated=OK);OK=OK and rc==0
except BaseException as e:emit('failure',error=str(e),trace=traceback.format_exc())
finally:
 if PROC and PROC.poll()is None:
  PROC.terminate()
  try:PROC.wait(timeout=10)
  except subprocess.TimeoutExpired:PROC.kill();PROC.wait(timeout=10)
 if NS in cmd(['ip','netns','list'],False).stdout:
  remaining=na(['ip','-j','link'],check=False).stdout;(OUT/'owned-links-before-namespace-delete.json').write_text(remaining);cmd(['ip','netns','delete',NS])
 if MON:MON.terminate();MON.wait(timeout=10)
 after_ovs=cmd(['python3',str(INC/'audit.py'),'0'],False)
 if after_ovs.returncode==0:gz('foreign-ovs-after.json.gz',json.loads(after_ovs.stdout))
 AFTER=snap('after');a=norm(BEFORE)if BEFORE else {};b=norm(AFTER);different=[k for k in a if a[k]!=b[k]];gz('foreign-configuration-complement.json.gz',{'matched':not different,'different':different,'before':a,'after':b,'neighbor_note':'State/cache age excluded, but every neighbor IP/device/MAC identity retained; raw state and live events are preserved.'})
 oom=int(next(x.split()[1]for x in P('/proc/vmstat').read_text().splitlines()if x.startswith('oom_kill ')))-OOM
 ovs_equal='before_ovs'in globals()and after_ovs.returncode==0 and before_ovs==json.loads(after_ovs.stdout)
 emit('cleanup',namespace=NS,namespace_absent=NS not in cmd(['ip','netns','list'],False).stdout,owned_pid=PROC.pid if PROC else None,owned_process_absent=PROC is None or PROC.poll()is not None,foreign_configuration_matched=not different,different_keys=different,foreign_ovs_matched=ovs_equal,new_oom_kills=oom,stock_module_kept_loaded=True,global_module_unload=False)
 
 for p in list(OUT.glob('*traffic*.jsonl'))+list(OUT.glob('netlink-wire.jsonl')):
  (p.with_suffix(p.suffix+'.gz')).write_bytes(gzip.compress(p.read_bytes(),mtime=0));p.unlink()
 raw('host-after-meminfo.txt',['cat','/proc/meminfo']);raw('host-after-vmstat.txt',['cat','/proc/vmstat']);raw('host-after-slabinfo.txt',['cat','/proc/slabinfo']);OK=OK and not different and ovs_equal and oom==0;emit('final',passed=OK,wall_s=time.perf_counter()-START);LOG.close()
sys.exit(0 if OK else 1)

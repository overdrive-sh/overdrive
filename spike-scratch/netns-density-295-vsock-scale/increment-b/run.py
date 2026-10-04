#!/usr/bin/env python3
"""Native stock CH device capacity; canonical launcher holds exclusive lease."""
import gzip, hashlib, json, os, pathlib, resource, signal, subprocess, time, traceback, sys
INC=pathlib.Path(__file__).resolve().parent;SCRATCH=INC.parent;OUT=SCRATCH/'out'/INC.name;OUT.mkdir(parents=True,exist_ok=False)
START=time.perf_counter();LOG=(OUT/'events.jsonl').open('x');PROC=None;BEFORE=None;OK=False
UDSDIR=pathlib.Path('/run')/('vs295-scale-'+str(os.getpid()))
def emit(event,**v):
 d={'event':event,'at_s':time.perf_counter()-START,**v};s=json.dumps(d,sort_keys=True);print(s,flush=True);LOG.write(s+'\n');LOG.flush()
def command(args,check=True,timeout=90,env=None):
 p=subprocess.run(args,capture_output=True,text=True,timeout=timeout,env=env)
 if check and p.returncode:raise RuntimeError(json.dumps({'argv':args,'rc':p.returncode,'stdout':p.stdout,'stderr':p.stderr}))
 return p
def raw(name,args):
 p=command(args,check=False);(OUT/name).write_text(json.dumps({'argv':args,'rc':p.returncode,'stdout':p.stdout,'stderr':p.stderr},indent=2));return p.stdout
def sha(p):return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()
def memavailable():return int(next(s.split()[1] for s in pathlib.Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:')))
def oom():return int(next(s.split()[1] for s in pathlib.Path('/proc/vmstat').read_text().splitlines() if s.startswith('oom_kill ')))
import snapshot
snapshot.raw=raw
snapshot.OUT=OUT
def audit(n):
 os.kill(PROC.pid,signal.SIGSTOP)
 try:
  root=pathlib.Path('/proc')/str(PROC.pid)
  until=time.perf_counter()+5
  while not all('State:\tT' in (t/'status').read_text() for t in (root/'task').iterdir()):
   if time.perf_counter()>until:raise RuntimeError('owned process did not freeze')
   time.sleep(.01)
  identities=json.loads((OUT/('identities-'+str(n)+'.json')).read_text());assert len(identities)==n
  fdtargets={int(f.name):os.readlink(f) for f in (root/'fd').iterdir()};tasks=[int(t.name) for t in (root/'task').iterdir()]
  unix=(root/'net/unix').read_text();(OUT/('stage-'+str(n)+'-unix.txt.gz')).write_bytes(gzip.compress(unix.encode()))
  sockets={}
  for line in unix.splitlines()[1:]:
   cells=line.split()
   if len(cells)==8 and cells[7].startswith(str(UDSDIR)+'/') and cells[3]=='00010000':sockets[cells[7]]=int(cells[6])
  receipts=[];epolls=set();exits=set();connections=set();addresses=set();contexts=set()
  for p in identities:
   cid=p['cid_from_device_config'];assert cid==p['cid_confirmed_by_rx_packet'];ef=p['backend_epoll_fd'];assert fdtargets[ef]=='anon_inode:[eventpoll]';info=(root/'fdinfo'/str(ef)).read_text()
   watched=[int(x.split()[1]) for x in info.splitlines() if x.startswith('tfd:')];assert len(watched)==2
   listen=[f for f in watched if fdtargets[f]=='socket:['+str(sockets[p['backend_path']])+']'];assert len(listen)==1
   assert fdtargets[p['exit_fd']]=='anon_inode:[eventfd]';ei=(root/'fdinfo'/str(p['exit_fd'])).read_text();eventid=next(s.split()[1] for s in ei.splitlines() if s.startswith('eventfd-id:'))
   assert fdtargets[p['host_connection_fd']].startswith('socket:[');assert p['device_status']==15
   epolls.add(ef);exits.add(eventid);connections.add(fdtargets[p['host_connection_fd']]);addresses.add(p['device_address']);contexts.add(p['guest_memory_context_address'])
   receipts.append({'cid':cid,'backend_path':p['backend_path'],'kernel_listening_socket_inode':sockets[p['backend_path']],'kernel_listener_fd':listen[0],'backend_epoll_fd':ef,'backend_epoll_fdinfo':info,'exit_eventfd_id':eventid,'host_connection_fd':p['host_connection_fd'],'host_connection_kernel_target':fdtargets[p['host_connection_fd']],'outer_worker_epoll_found':False})
  # Each upstream worker's nested backend epoll must also be registered in an outer epoll.
  outer={};all_epolls=[]
  for f,t in fdtargets.items():
   if t=='anon_inode:[eventpoll]' and f not in epolls:
    info=(root/'fdinfo'/str(f)).read_text();watch=[int(x.split()[1]) for x in info.splitlines() if x.startswith('tfd:')];all_epolls.append({'fd':f,'fdinfo':info})
    for x in watch:
     if x in epolls:assert x not in outer;outer[x]=f
  for r in receipts:r['outer_worker_epoll_found']=r['backend_epoll_fd'] in outer;r['outer_worker_epoll_fd']=outer.get(r['backend_epoll_fd'])
  assert all(r['outer_worker_epoll_found'] for r in receipts)
  assert len(sockets)==n and len(epolls)==n and len(exits)==n and len(connections)==n and len(addresses)==n and len(contexts)==n and len(tasks)==n+1
  (OUT/('stage-'+str(n)+'-kernel-owner-audit.json.gz')).write_bytes(gzip.compress(json.dumps({'owners':receipts,'outer_worker_epolls':all_epolls},sort_keys=True).encode()))
  status=(root/'status').read_text();smaps=(root/'smaps_rollup').read_text();maps=(root/'maps').read_text();(OUT/('stage-'+str(n)+'-maps.txt.gz')).write_bytes(gzip.compress(maps.encode()))
  (OUT/('stage-'+str(n)+'-resources.json')).write_text(json.dumps({'pid':PROC.pid,'status':status,'smaps_rollup':smaps,'fd_count':len(fdtargets),'tasks':len(tasks),'mapping_count':len(maps.splitlines()),'limits':(root/'limits').read_text(),'cgroup':(root/'cgroup').read_text(),'host_memavailable_kib':memavailable(),'socket_count':len(sockets),'epoll_count':len(epolls),'thread_owner_count':len(outer),'exit_eventfd_identity_count':len(exits),'device_address_count':len(addresses),'memory_context_address_count':len(contexts),'process_cmdline':(root/'cmdline').read_bytes().replace(b'\0',b' ').decode(),'executable_sha256':sha(root/'exe')},indent=2))
  emit('independent-frozen-owner-audit',held_stock_activated_devices=n,kernel_unique_listening_socket_inodes=len(sockets),distinct_stock_muxer_epoll_fds=len(epolls),distinct_worker_outer_epoll_fds=len(outer),distinct_exit_eventfd_ids=len(exits),distinct_peer_connections=len(connections),distinct_device_addresses=len(addresses),distinct_memory_contexts=len(contexts),native_process_tasks=len(tasks),native_process_fds=len(fdtargets),host_memavailable_kib=memavailable(),all_checks_passed=True)
 finally:os.kill(PROC.pid,signal.SIGCONT)
OOM=oom()
try:
 emit('identity',pid=os.getpid(),uname=list(os.uname()),lease_owner=pathlib.Path('/run/lock/overdrive-metal-shared.owner').read_text(),runner_sha256=sha(__file__),rust_source_sha256=sha(INC/'src/main.rs'),upstream_git_rev='9ed824d6d08df3e96f7d5f50795d9449ac99f431',requested_count=16384,user_approved_boundary='stock activated transport devices and independent muxers; no guest cohort',actual_guest_boots=0)
 raw('rustc-version.txt',['rustc','-Vv']);raw('host-limits.txt',['cat','/proc/self/limits']);raw('host-kernel.txt',['uname','-a']);raw('host-meminfo-before.txt',['cat','/proc/meminfo']);raw('host-vmstat-before.txt',['cat','/proc/vmstat']);raw('host-fd-limits.txt',['sysctl','fs.file-max','fs.nr_open','kernel.threads-max','kernel.pid_max','vm.max_map_count']);BEFORE=snapshot.snap('before')
 if memavailable()<16*1024*1024:raise RuntimeError('host MemAvailable below bounded 16GiB admission requirement')
 env=os.environ.copy();env['CARGO_TARGET_DIR']=str(SCRATCH/'target');env.pop('RUSTFLAGS',None)
 args=['cargo','build','--manifest-path',str(INC/'Cargo.toml'),'--release'];emit('build-start',argv=args)
 build=command(args,timeout=900,env=env);(OUT/'build.txt').write_text(build.stdout+build.stderr);(OUT/'Cargo.lock.executed').write_bytes((INC/'Cargo.lock').read_bytes());binary=SCRATCH/'target/release/vsock-device-capacity-probe';emit('build-complete',rc=build.returncode,binary_sha256=sha(binary),binary_bytes=binary.stat().st_size)
 soft,hard=resource.getrlimit(resource.RLIMIT_NOFILE)
 def before_exec():resource.setrlimit(resource.RLIMIT_NOFILE,(hard,hard))
 stages=[4,1024,4096,8192,16384];cmd=[str(binary),str(UDSDIR),str(OUT),','.join(map(str,stages))];emit('capacity-start',argv=cmd,original_nofile_soft=soft,inherited_nofile_hard=hard,owned_process_nofile_soft=hard,global_limits_changed=False,admission_reserve_kib=8*1024*1024)
 PROC=subprocess.Popen(cmd,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=(OUT/'harness.stderr').open('xb'),text=True,preexec_fn=before_exec)
 with (OUT/'harness.stdout').open('x') as log:
  for line in PROC.stdout:
   log.write(line);log.flush();print(line,end='',flush=True)
   try:d=json.loads(line)
   except ValueError:continue
   if d.get('event')=='stage_held':
    n=d['actual_stock_activated_devices'];audit(n)
    if memavailable()<8*1024*1024:PROC.stdin.write('stop\n')
    else:PROC.stdin.write('continue\n')
    PROC.stdin.flush()
   if d.get('event')=='cleaned':OK=d['released_devices']==16384 and d['final_fds']==d['initial_fds'] and d['final_tasks']==d['initial_tasks']
 rc=PROC.wait(timeout=30);emit('capacity-exit',rc=rc,full_16384_device_capacity_validated=OK);OK=OK and rc==0
except BaseException as e:emit('probe-failure',error=str(e),trace=traceback.format_exc())
finally:
 if PROC and PROC.poll() is None:
  os.kill(PROC.pid,signal.SIGCONT);PROC.terminate()
  try:PROC.wait(timeout=10)
  except subprocess.TimeoutExpired:PROC.kill();PROC.wait(timeout=10)
 remaining=[]
 if UDSDIR.exists():
  remaining=[str(p) for p in UDSDIR.iterdir()]
  for p in UDSDIR.iterdir():
   if p.is_socket():p.unlink()
  if not list(UDSDIR.iterdir()):UDSDIR.rmdir()
 if BEFORE is not None:
  AFTER=snapshot.snap('after');a=snapshot.normalize(BEFORE);b=snapshot.normalize(AFTER);different=[k for k in a if a[k]!=b[k]];(OUT/'configuration-complement.json.gz').write_bytes(gzip.compress(json.dumps({'matched':not different,'different_keys':different,'before':a,'after':b}).encode()));OK=OK and not different
 else:different=[]
 emit('cleanup',owned_pid=PROC.pid if PROC else None,owned_pid_alive=PROC is not None and PROC.poll() is None,owned_socket_directory=str(UDSDIR),owned_socket_directory_exists=UDSDIR.exists(),exact_owned_residual_sockets_removed_after_process_exit=remaining,administrative_configuration_matched=not different,different_keys=different,new_host_oom_kills=oom()-OOM,foreign_configuration_mutated=False,global_limits_changed=False,actual_guest_boots=0);OK=OK and not UDSDIR.exists() and oom()==OOM
 raw('host-meminfo-after.txt',['cat','/proc/meminfo']);raw('host-vmstat-after.txt',['cat','/proc/vmstat']);emit('final',passed=OK,wall_s=time.perf_counter()-START);LOG.close()
sys.exit(0 if OK else 1)

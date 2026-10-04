#!/usr/bin/env python3
"""Native PROBE orchestrator only: package exact Rust helper, boot real VMs, drive transport."""
import hashlib,json,os,pathlib,resource,shutil,socket,subprocess,time,traceback
from quota import Quota
INC=pathlib.Path(__file__).resolve().parent;SCRATCH=INC.parent
OUT=SCRATCH/'out'/INC.name;OUT.mkdir(parents=True,exist_ok=False)
START=time.perf_counter();LOG=(OUT/'events.jsonl').open('x');PROCS=[];FILES=[]
QUOTA=Quota(OUT,'v295-quota-'+str(os.getpid()))
def emit(event,**fields):
    data={'event':event,'at_s':time.perf_counter()-START,**fields};s=json.dumps(data,sort_keys=True);print(s,flush=True);LOG.write(s+'\n');LOG.flush()
def command(args,check=True,timeout=90,env=None):
    p=subprocess.run(args,capture_output=True,text=True,timeout=timeout,env=env)
    if check and p.returncode:raise RuntimeError(json.dumps({'argv':args,'rc':p.returncode,'stdout':p.stdout,'stderr':p.stderr}))
    return p
def raw(name,args):
    p=command(args,check=False);(OUT/name).write_text(json.dumps({'argv':args,'rc':p.returncode,'stdout':p.stdout,'stderr':p.stderr},indent=2));return p.stdout
def sha(path):return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()
def snap(prefix):
    result={}
    for key,args in [('links',['ip','-j','-d','link']),('addrs',['ip','-j','address']),('routes',['ip','-j','route','show','table','all']),('rules',['ip','-j','rule']),('nft',['nft','-j','list','ruleset']),('netns',['ip','netns','list']),('bpfmaps',['bpftool','-j','map','show']),('bpflinks',['bpftool','-j','link','show'])]:
        content=raw(prefix+'-'+key+'.json',args)
        try:result[key]=json.loads(content)
        except ValueError:result[key]=content
    for key,path in [('modules','/proc/modules'),('ip-forward','/proc/sys/net/ipv4/ip_forward'),('rp-filter','/proc/sys/net/ipv4/conf/all/rp_filter')]:
        result[key]=pathlib.Path(path).read_text();(OUT/(prefix+'-'+key+'.txt')).write_text(result[key])
    raw(prefix+'-neighbors.json',['ip','-j','-s','neigh','show']);raw(prefix+'-processes.txt',['ps','-e','-o','pid,ppid,comm,args']);raw(prefix+'-bpfpins.txt',['find','/sys/fs/bpf/overdrive','-maxdepth','4','-printf','%P %y %i\n'])
    return result
def normalize(data):
    ignored={'valid_life_time','preferred_life_time','expires','lastuse','used','updated','stats64','gc_timer','stats','cache','packets','bytes'}
    def walk(value):
        if isinstance(value,dict):return {k:walk(v) for k,v in sorted(value.items()) if k not in ignored}
        if isinstance(value,list):return sorted([walk(v) for v in value],key=lambda v:json.dumps(v,sort_keys=True))
        return value
    out=walk(data);out['modules']=sorted(' '.join(s.split()[:2]+s.split()[4:]) for s in data['modules'].splitlines());return out
def proc_info(proc):
    if proc.poll() is not None:return {'pid':proc.pid,'exit':proc.returncode}
    root=pathlib.Path('/proc')/str(proc.pid)
    fdlist=[]
    for p in (root/'fd').iterdir():
        try:fdlist.append({'fd':int(p.name),'target':os.readlink(p)})
        except FileNotFoundError:pass
    return {'pid':proc.pid,'executable_sha256':sha(root/'exe'),'status':(root/'status').read_text(),'smaps_rollup':(root/'smaps_rollup').read_text(),'stat':(root/'stat').read_text(),'schedstat':(root/'schedstat').read_text(),'fd_count':len(fdlist),'fds':fdlist,'cgroup':(root/'cgroup').read_text(),'cmdline':(root/'cmdline').read_bytes().replace(b'\0',b' ').decode()}
def spawn(args,logname):
    log=(OUT/logname).open('wb');FILES.append(log)
    if args[0]=='cloud-hypervisor':
        role=logname.split('-',1)[1].split('.',1)[0];group=QUOTA.child(role);proc=subprocess.Popen(args,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT,preexec_fn=lambda:QUOTA.before_exec(group));emit('quota-before-vmm-exec',role=role,pid=proc.pid,path=str(group),cpu_max=(group/'cpu.max').read_text(),host_cpu_quota_logical_cpus=.125,guest_vcpu_topology=1)
    else:proc=subprocess.Popen(args,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT)
    PROCS.append(proc);return proc
def wait_text(path,needle,proc,seconds):
    until=time.perf_counter()+seconds
    while time.perf_counter()<until:
        text=path.read_text(errors='replace') if path.exists() else ''
        if needle in text:return text
        if proc.poll() is not None:raise RuntimeError('process exited before '+needle+' '+text[-3000:])
        time.sleep(.05)
    raise RuntimeError('deadline waiting for '+needle+' '+text[-3000:])
def host_request(base,port,payload):
    begin=time.perf_counter();s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.settimeout(3)
    try:
        s.connect(base);s.sendall(('CONNECT '+str(port)+'\n').encode());ack=b''
        while not ack.endswith(b'\n'):
            b=s.recv(1)
            if not b:break
            ack+=b
        if not ack.startswith(b'OK '):return {'passed':False,'ack':ack.decode(errors='replace'),'elapsed_s':time.perf_counter()-begin}
        s.sendall(payload);reply=b''
        while True:
            b=s.recv(4096)
            if not b:break
            reply+=b
        return {'passed':True,'ack':ack.decode(),'reply':reply.decode(errors='replace'),'reply_hex':reply.hex(),'elapsed_s':time.perf_counter()-begin}
    except OSError as e:return {'passed':False,'errno':e.errno,'error':str(e),'elapsed_s':time.perf_counter()-begin}
    finally:s.close()
# Bounded native population. Guest RAM is pinned after successful RAM-floor probes.
RAM_MIB=96
COUNT=64
BATCH=8
RESERVE_KIB=20*1024*1024
OVERHEAD_KIB=4*1024*1024
BEFORE=None;OK=False;BASES=[];BOOTS=[];UDSDIR=pathlib.Path('/run')/('v295-'+str(os.getpid()))
def memavailable():
    return int(next(s.split()[1] for s in pathlib.Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:')))
def stage(stage_count):
    alive=[proc.pid for role,cid,base,proc,console,begin,image in BOOTS if proc.poll() is None]
    if len(alive)!=stage_count:raise RuntimeError('VMM stage-count/liveness mismatch')
    info={role:proc_info(proc) for role,cid,base,proc,console,begin,image in BOOTS};host_info=proc_info(HOST)
    (OUT/('stage-'+str(stage_count)+'-quota-readbacks.json')).write_text(json.dumps([QUOTA.readback(role,proc.pid) for role,cid,base,proc,console,begin,image in BOOTS],indent=2))
    def status_kib(p,key):return int(next(s.split()[1] for s in p['status'].splitlines() if s.startswith(key+':')))
    total_rss=sum(status_kib(p,'VmRSS') for p in info.values());total_fds=sum(p['fd_count'] for p in info.values());total_threads=sum(status_kib(p,'Threads') for p in info.values())
    def pss(p):return int(next(s.split()[1] for s in p['smaps_rollup'].splitlines() if s.startswith('Pss:')))
    total_pss=sum(pss(p) for p in info.values())
    (OUT/('stage-'+str(stage_count)+'-process-resources.json')).write_text(json.dumps({'vmms':info,'host_backend':host_info},indent=2));raw('stage-'+str(stage_count)+'-host-meminfo.txt',['cat','/proc/meminfo']);raw('stage-'+str(stage_count)+'-links.json',['ip','-j','-d','link'])
    emit('stage',actual_live_communicating_vm_count=stage_count,ready_guest_identities=len(READY),successful_guest_echo_count=len(READY),successful_host_http_count=len(READY),unique_cids=len({cid for role,cid,base,proc,console,begin,image in BOOTS}),unique_vm_pids=len(set(alive)),configured_guest_memory_total_mib=stage_count*RAM_MIB,total_vmm_rss_kib=total_rss,total_vmm_pss_kib=total_pss,total_vmm_fds=total_fds,total_vmm_threads=total_threads,host_backend_rss_kib=status_kib(host_info,'VmRSS'),host_backend_pss_kib=pss(host_info),host_backend_fds=host_info['fd_count'],host_backend_threads=status_kib(host_info,'Threads'),host_memavailable_kib=memavailable(),reserve_kib=RESERVE_KIB,guest_kernel='7.0.0-29-generic',host_taps_created=0,host_bridge_ports_created=0)
READY=[]
OOM_BEFORE=None
def oom_count():return int(next(s.split()[1] for s in pathlib.Path('/proc/vmstat').read_text().splitlines() if s.startswith('oom_kill ')))
try:
    emit('user-decision',approval='lets lower both so it fits within the machine',scope='lower experiment resources/count only; original 16384 DESIGN/E18 contract remains pending')
    emit('user-cpu-correction',host_cpu_quota_logical_cpus=.125,quota_us=12500,period_us=100000,guest_vcpu_topology=1,aggregate_parent_quota_logical_cpus=8,hardware_logical_cpus=16,scope='all VMM threads; separate shared platform receiver is measured outside per-VMM budget')
    emit('identity',pid=os.getpid(),parent_executable_sha256=sha('/proc/self/exe'),parent_executable=os.readlink('/proc/self/exe'),source_sha256=sha(INC/'src/main.rs'),runner_sha256=sha(__file__),uname=list(os.uname()),lease_owner=pathlib.Path('/run/lock/overdrive-metal-shared.owner').read_text(),rlimit_nofile=resource.getrlimit(resource.RLIMIT_NOFILE))
    available=memavailable();guest_budget=COUNT*RAM_MIB*1024
    disk=shutil.disk_usage(OUT);rootfs=os.environ['OVERDRIVE_METAL_ROOTFS'];kernel=os.environ['OVERDRIVE_METAL_KERNEL'];image_bytes=pathlib.Path(rootfs).stat().st_size
    emit('admission-budget',revised_target_vm_count=COUNT,guest_memory_mib=RAM_MIB,vcpu_per_vm=1,boot_batch_size=BATCH,host_memavailable_kib=available,configured_guest_memory_kib=guest_budget,overhead_allowance_kib=OVERHEAD_KIB,host_reserved_kib=RESERVE_KIB,host_cpu_quota_logical_cpus=.125,aggregate_cpu_budget_logical_cpus=COUNT*.125,guest_vcpu_topology=1,required_available_kib=guest_budget+OVERHEAD_KIB+RESERVE_KIB,disk_free_bytes=disk.free,image_copy_budget_bytes=COUNT*image_bytes,disk_reserve_bytes=8*1024**3,memory_overcommit_relied_on=False)
    if available<guest_budget+OVERHEAD_KIB+RESERVE_KIB:raise RuntimeError('declared memory budget/headroom does not fit')
    if disk.free<COUNT*image_bytes+8*1024**3:raise RuntimeError('declared image-copy/disk reserve does not fit')
    raw('ch-version.txt',['cloud-hypervisor','--version']);raw('rustc-version.txt',['rustc','-Vv']);emit('fixture-identity',cloud_hypervisor_sha256=sha(shutil.which('cloud-hypervisor')),kernel_sha256=sha(kernel),rootfs_sha256=sha(rootfs),rootfs_bytes=image_bytes)
    OOM_BEFORE=oom_count();raw('host-vmstat-before.txt',['cat','/proc/vmstat']);BEFORE=snap('before');QUOTA.prepare();emit('owned-cgroup-parent',path=str(QUOTA.parent),cpu_max=(QUOTA.parent/'cpu.max').read_text());raw('host-meminfo-before.txt',['cat','/proc/meminfo'])
    env=os.environ.copy();env['SPIKE_SOURCE_SHA']=sha(INC/'src/main.rs');env['CARGO_TARGET_DIR']=str(SCRATCH/'target');env['RUSTFLAGS']='-C target-feature=+crt-static -C linker=cc'
    begin=time.perf_counter();build=command(['cargo','build','--manifest-path',str(INC/'Cargo.toml'),'--release','--target','x86_64-unknown-linux-musl'],timeout=240,env=env);(OUT/'build.txt').write_text(build.stdout+build.stderr);(OUT/'Cargo.lock.executed').write_bytes((INC/'Cargo.lock').read_bytes());binary=SCRATCH/'target/x86_64-unknown-linux-musl/release/shared-memory-vsock-probe-01';BINARY_SHA=sha(binary);emit('build',elapsed_s=time.perf_counter()-begin,binary_sha256=BINARY_SHA,binary_bytes=binary.stat().st_size)
    UDSDIR.mkdir(mode=0o700,exist_ok=False);BASES=[str(UDSDIR/('v'+str(i).zfill(3))) for i in range(COUNT)]
    HOST=spawn([str(binary),'host',*BASES],'host-backend.log');wait_text(OUT/'host-backend.log','HOST_ALL_RECEIVERS_READY',HOST,5);emit('host-ready',pid=HOST.pid,prearmed_probe_receiver_count=COUNT,before_any_vmm_spawn=True)
    caplog=(OUT/'capture.stderr').open('wb');FILES.append(caplog);CAP=subprocess.Popen(['tcpdump','-i','any','-U','-n','-w',str(OUT/'all-interfaces.pcap')],stdout=subprocess.DEVNULL,stderr=caplog);PROCS.append(CAP);wait_text(OUT/'capture.stderr','listening on',CAP,3);emit('capture-armed',pid=CAP.pid,before_any_vmm_spawn=True,limitation='AF_PACKET does not observe virtio-vsock payloads; no production zero-leakage proof')
    for offset in range(0,COUNT,BATCH):
        if memavailable()<RESERVE_KIB+(COUNT-offset)*RAM_MIB*1024+OVERHEAD_KIB:raise RuntimeError('remaining configured-memory/headroom budget no longer fits')
        cohort=[]
        for index in range(offset,min(offset+BATCH,COUNT)):
            role='v'+str(index).zfill(3);cid=41000+index;base=BASES[index];image=OUT/('guest-'+role+'.ext4');command(['cp','--reflink=auto',rootfs,str(image)]);command(['debugfs','-w','-R','rm /init',str(image)]);command(['debugfs','-w','-R','write '+str(binary)+' /init',str(image)]);command(['debugfs','-w','-R','set_inode_field /init mode 0100755',str(image)]);emit('staged-guest',role=role,cid=cid,image_sha256=sha(image),init_sha256=BINARY_SHA)
            console=OUT/('guest-'+role+'.console');args=['cloud-hypervisor','--cpus','boot=1','--memory','size='+str(RAM_MIB)+'M','--kernel',kernel,'--disk','path='+str(image)+',image_type=raw','--cmdline','console=ttyS0 panic=1 root=/dev/vda rw init=/init spike_role='+role+' spike_cid='+str(cid),'--serial','file='+str(console),'--console','off','--api-socket',str(UDSDIR/('api-'+role)),'--vsock','cid='+str(cid)+',socket='+base];begin=time.perf_counter();proc=spawn(args,'vmm-'+role+'.stderr');entry=(role,cid,base,proc,console,begin,image);BOOTS.append(entry);cohort.append(entry);emit('vmm-spawn',role=role,cid=cid,pid=proc.pid,host_net_devices_requested=0,net_option_present=False)
        for role,cid,base,proc,console,begin,image in cohort:
            text=wait_text(console,'GUEST_PROBE_READY role='+role+' transport_result=true\n',proc,90)
            if ('GUEST_EXECUTABLE_SHA256 role='+role+' sha256='+BINARY_SHA) not in text:raise RuntimeError('loaded guest binary mismatch '+role)
            checks={'cid_matched':'matched=true' in text,'echo_passed':'transport_result=true' in text,'three_denials':text.count('denied=true')==3}
            if not all(checks.values()):
                (OUT/('returned-console-on-failure-'+role+'.txt')).write_text(text);emit('guest-check-failure',role=role,checks=checks,returned_console_bytes=len(text));raise RuntimeError('guest transport/identity/denials failed '+role)
            response=host_request(base,5001,('GET /spike-'+role+' HTTP/1.1\r\nHost: ordinary-app\r\nConnection: close\r\n\r\n').encode());response['passed']=response['passed'] and ('ordinary-http-'+role) in response.get('reply','');emit('host-to-guest-http',role=role,cid=cid,**response)
            denied=host_request(base,5009,b'forbidden\n');emit('host-unregistered-guest-port',role=role,cid=cid,denied=not denied['passed'],result=denied)
            if not response['passed'] or denied['passed']:raise RuntimeError('host HTTP/denied-port failed '+role)
            READY.append({'role':role,'cid':cid,'pid':proc.pid,'guest_loaded_sha256':BINARY_SHA,'host_http_passed':True,'guest_echo_passed':True,'guest_negative_count':3,'host_negative_count':1});emit('guest-ready',role=role,cid=cid,pid=proc.pid,elapsed_s=time.perf_counter()-begin,loaded_sha256=BINARY_SHA)
        if len(BOOTS) in [16,32,64,128]:stage(len(BOOTS))
    # Recheck the entire retained population at the final cardinality, not merely each boot batch.
    recheck_begin=time.perf_counter();rechecks=[]
    for role,cid,base,proc,console,begin,image in BOOTS:
        if proc.poll() is not None:raise RuntimeError('VMM exited before final full-pool recheck '+role)
        result=host_request(base,5001,('GET /final-'+role+' HTTP/1.1\r\nHost: ordinary-app\r\nConnection: close\r\n\r\n').encode())
        passed=result['passed'] and ('ordinary-http-'+role) in result.get('reply','');rechecks.append({'role':role,'cid':cid,'pid':proc.pid,'passed':passed,'elapsed_s':result['elapsed_s'],'reply':result.get('reply')})
        if not passed:raise RuntimeError('retained final-cardinality HTTP recheck failed '+role)
    (OUT/'final-live-http-recheck.json').write_text(json.dumps(rechecks,indent=2));emit('final-full-pool-recheck',actual_live_vm_count=len(BOOTS),successful_live_http_identities=sum(r['passed'] for r in rechecks),unique_cids=len({r['cid'] for r in rechecks}),elapsed_s=time.perf_counter()-recheck_begin)
    (OUT/'communicating-identities.json').write_text(json.dumps(READY,indent=2));emit('transport-summary',actual_linux_microvms=len(BOOTS),actual_ready_communicating_identities=len(READY),configured_memory_mib_per_vm=RAM_MIB,configured_vcpu_per_vm=1,host_cpu_quota_logical_cpus=.125,aggregate_parent_cpu_max='800000 100000',actual_host_taps_created=0,actual_host_bridge_ports_created=0,all_passed=len(READY)==COUNT,full_16384_target_validated=False,scope='bounded revised target under explicit resource/count approval; no production network-policy or observer acceptance');OK=len(READY)==COUNT
except BaseException as error:
    emit('probe-failure',error=str(error),trace=traceback.format_exc())
finally:
    (OUT/'cleanup-ledger.json').write_text(json.dumps({'owned_pids':[p.pid for p in PROCS],'owned_uds_directory':str(UDSDIR),'owned_cgroup_parent':str(QUOTA.parent),'owned_vm_paths':[str(image) for role,cid,base,proc,console,begin,image in BOOTS]},indent=2))
    for proc in reversed(PROCS):
        if proc.poll() is None:
            proc.send_signal(2 if proc.args[0]=='tcpdump' else 15)
            try:proc.wait(timeout=5)
            except subprocess.TimeoutExpired:proc.kill();proc.wait(timeout=3)
        emit('owned-process-stopped',pid=proc.pid,returncode=proc.returncode)
    for file in FILES:file.close()
    QUOTA.cleanup();emit('owned-cgroup-cleanup',path=str(QUOTA.parent),remaining=QUOTA.parent.exists())
    for role,cid,base,proc,console,begin,image in BOOTS:
        for path in [pathlib.Path(base),pathlib.Path(base+'_5000'),UDSDIR/('api-'+role),UDSDIR/('api-'+role+'.lock'),image]:
            if path.exists() or path.is_socket():path.unlink()
    # Prearmed receivers whose VMM was never spawned still belong to this exact registry.
    for base in BASES:
        path=pathlib.Path(base+'_5000')
        if path.exists() or path.is_socket():path.unlink()
    if UDSDIR.exists():
        leftovers=[p.name for p in UDSDIR.iterdir()]
        if leftovers:emit('cleanup-unexpected-owned-directory-contents',entries=leftovers);OK=False
        else:UDSDIR.rmdir()
    if (OUT/'all-interfaces.pcap').exists():raw('capture-decode.txt',['tcpdump','-n','-e','-r',str(OUT/'all-interfaces.pcap')])
    if BEFORE is not None:
        AFTER=snap('after');a=normalize(BEFORE);b=normalize(AFTER);keys=sorted(k for k in a if a[k]!=b[k]);(OUT/'configuration-complement.json').write_text(json.dumps({'matched':not keys,'different_keys':keys,'before':a,'after':b,'dynamic_state_limitation':'Administrative configuration only; no foreign neighbor/timer/counter/page-cache/process equality asserted.'},indent=2));emit('cleanup',administrative_configuration_matched=not keys,different_keys=keys,owned_processes_alive=[p.pid for p in PROCS if p.poll() is None],owned_uds_directory_exists=UDSDIR.exists(),owned_images_remaining=[str(image) for role,cid,base,proc,console,begin,image in BOOTS if image.exists()],host_memavailable_kib=memavailable(),sysctls_tuned=False,foreign_modules_unloaded=False);OK=OK and not keys and not UDSDIR.exists()
    raw('host-vmstat-after.txt',['cat','/proc/vmstat']);oom_after=oom_count();emit('host-oom-counter',before=OOM_BEFORE,after=oom_after,no_new_host_oom_kills=OOM_BEFORE==oom_after);OK=OK and OOM_BEFORE==oom_after
    emit('final',passed=OK,actual_ready_communicating_count=len(READY),wall_s=time.perf_counter()-START);LOG.close()
raise SystemExit(0 if OK else 1)

#!/usr/bin/env python3
"""Native PROBE orchestrator only: package exact Rust helper, boot real VMs, drive transport."""
import hashlib,json,os,pathlib,resource,shutil,socket,subprocess,time,traceback
INC=pathlib.Path(__file__).resolve().parent;SCRATCH=INC.parent
OUT=SCRATCH/'out'/INC.name;OUT.mkdir(parents=True,exist_ok=False)
START=time.perf_counter();LOG=(OUT/'events.jsonl').open('x');PROCS=[];FILES=[]
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
    return {'pid':proc.pid,'executable_sha256':sha(root/'exe'),'status':(root/'status').read_text(),'smaps_rollup':(root/'smaps_rollup').read_text(),'fd_count':len(fdlist),'fds':fdlist,'cgroup':(root/'cgroup').read_text(),'cmdline':(root/'cmdline').read_bytes().replace(b'\0',b' ').decode()}
def spawn(args,logname):
    log=(OUT/logname).open('wb');FILES.append(log);proc=subprocess.Popen(args,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT);PROCS.append(proc);return proc
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
BEFORE=None;OK=False;BASES=[];UDSDIR=pathlib.Path("/run")/("v295-"+str(os.getpid()))
try:
    emit('user-decision',approval='lets lower both so it fits within the machine',scope='lower probe resources and VM count only; original 16384 DESIGN/E18 contract remains pending')
    emit('identity',pid=os.getpid(),parent_executable_sha256=sha('/proc/self/exe'),parent_executable=os.readlink('/proc/self/exe'),source_sha256=sha(INC/'src/main.rs'),runner_sha256=sha(__file__),uname=list(os.uname()),lease_owner=pathlib.Path('/run/lock/overdrive-metal-shared.owner').read_text(),rlimit_nofile=resource.getrlimit(resource.RLIMIT_NOFILE))
    raw('ch-version.txt',['cloud-hypervisor','--version']);raw('rustc-version.txt',['rustc','-Vv']);raw('host-kernel-config.txt',['sh','-c','cat /boot/config-"$(uname -r)" | sed -n "/VSOCK/p; /VSOCKETS/p; /VHOST/p"'])
    kernel=os.environ['OVERDRIVE_METAL_KERNEL'];rootfs=os.environ['OVERDRIVE_METAL_ROOTFS']
    emit('fixture-identity',cloud_hypervisor_sha256=sha(shutil.which('cloud-hypervisor')),kernel_path=kernel,kernel_sha256=sha(kernel),rootfs_path=rootfs,rootfs_sha256=sha(rootfs),rootfs_bytes=pathlib.Path(rootfs).stat().st_size)
    raw('guest-rootfs-modules.txt',['debugfs','-R','ls -l /lib/modules',rootfs]);raw('guest-rootfs-busybox.txt',['debugfs','-R','stat /bin/busybox',rootfs]);raw('disk-free-before.txt',['df','-h',str(OUT)]);raw('host-meminfo-before.txt',['cat','/proc/meminfo'])
    # Previous attempt left only CH-created API lock files, after exact children exited.
    previous=pathlib.Path('/run/v295-792743')
    if previous.exists():
        entries=sorted(p.name for p in previous.iterdir());emit('prior-owned-cleanup-readback',directory=str(previous),entries=entries)
        if entries!=['api-a.lock','api-b.lock']:raise RuntimeError('unexpected prior owned directory contents')
        for name in entries:(previous/name).unlink()
        previous.rmdir();emit('prior-owned-cleanup-complete',directory=str(previous))
    raw('guest-rootfs-module-inventory.txt',['debugfs','-R','ls -l /modules',rootfs])
    BEFORE=snap('before')
    env=os.environ.copy();env['SPIKE_SOURCE_SHA']=sha(INC/'src/main.rs');env['CARGO_TARGET_DIR']=str(SCRATCH/'target');env['RUSTFLAGS']='-C target-feature=+crt-static -C linker=cc'
    build_begin=time.perf_counter();result=command(['cargo','build','--manifest-path',str(INC/'Cargo.toml'),'--release','--target','x86_64-unknown-linux-musl'],timeout=240,env=env)
    (OUT/'build.txt').write_text(result.stdout+result.stderr);(OUT/'Cargo.lock.executed').write_bytes((INC/'Cargo.lock').read_bytes());binary=SCRATCH/'target/x86_64-unknown-linux-musl/release/shared-memory-vsock-probe-01'
    emit('build',elapsed_s=time.perf_counter()-build_begin,binary_sha256=sha(binary),binary_bytes=binary.stat().st_size);raw('binary-file.txt',['file',str(binary)])
    # UDS paths must fit sockaddr_un; runtime files remain in task-owned ignored out.
    UDSDIR.mkdir(mode=0o700,exist_ok=False)
    for role in ['a','b']:BASES.append(str(UDSDIR/('v'+role)))
    if any(len(base)+6>=108 for base in BASES):raise RuntimeError('UDS length exceeds Linux sockaddr_un bound')
    host=spawn([str(binary),'host',*BASES],'host-backend.log');wait_text(OUT/'host-backend.log','HOST_ALL_RECEIVERS_READY',host,3)
    emit('host-ready',pid=host.pid,before_any_vmm_spawn=True,receiver_paths=[b+'_5000' for b in BASES])
    # Read-only AF_PACKET observer is armed first. It does not capture AF_VSOCK/virtqueue bytes.
    caplog=(OUT/'capture.stderr').open('wb');FILES.append(caplog)
    cap=subprocess.Popen(['tcpdump','-i','any','-U','-n','-w',str(OUT/'all-interfaces.pcap')],stdout=subprocess.DEVNULL,stderr=caplog);PROCS.append(cap)
    wait_text(OUT/'capture.stderr','listening on',cap,3);emit('capture-armed',pid=cap.pid,before_any_vmm_spawn=True,limitation='AF_PACKET cannot witness virtio-vsock payloads; Rust backend logs are sampled successful reads, not loss-accounted observers')
    boots=[]
    for role,cid,base in [('a',39501,BASES[0]),('b',39502,BASES[1])]:
        image=OUT/('guest-'+role+'.ext4');command(['cp','--reflink=auto',rootfs,str(image)])
        command(['debugfs','-w','-R','rm /init',str(image)]);command(['debugfs','-w','-R','write '+str(binary)+' /init',str(image)]);command(['debugfs','-w','-R','set_inode_field /init mode 0100755',str(image)])
        emit('staged-guest',role=role,cid=cid,image_sha256=sha(image),init_sha256=sha(binary),image_bytes=image.stat().st_size)
        console=OUT/('guest-'+role+'.console');args=['cloud-hypervisor','--cpus','boot=1','--memory','size=128M','--kernel',kernel,'--disk','path='+str(image)+',image_type=raw','--cmdline','console=ttyS0 panic=1 root=/dev/vda rw init=/init spike_role='+role+' spike_cid='+str(cid),'--serial','file='+str(console),'--console','off','--api-socket',str(UDSDIR/('api-'+role)),'--vsock','cid='+str(cid)+',socket='+base]
        begin=time.perf_counter();proc=spawn(args,'vmm-'+role+'.stderr');boots.append((role,cid,base,proc,console,begin));emit('vmm-spawn',role=role,pid=proc.pid,cid=cid,host_net_devices_requested=0,net_option_present=False)
    for role,cid,base,proc,console,begin in boots:
        text=wait_text(console,'GUEST_PROBE_READY',proc,35)
        if ('GUEST_EXECUTABLE_SHA256 role='+role+' sha256='+sha(binary)) not in text:raise RuntimeError('loaded guest binary hash mismatch')
        emit('guest-ready',role=role,cid=cid,elapsed_s=time.perf_counter()-begin,lines=[s for s in text.splitlines() if s.startswith('GUEST_') or 'PROBE_ERROR' in s])
    results=[]
    for role,cid,base,proc,console,begin in boots:
        payload=('GET /spike-'+role+' HTTP/1.1\r\nHost: ordinary-app\r\nConnection: close\r\n\r\n').encode();result=host_request(base,5001,payload);result['passed']=result['passed'] and ('ordinary-http-'+role) in result.get('reply','');results.append(result['passed']);emit('host-to-guest-http',role=role,cid=cid,**result)
        denied=host_request(base,5009,b'forbidden\n');emit('host-unregistered-guest-port',role=role,cid=cid,denied=not denied['passed'],result=denied)
        text=console.read_text(errors='replace');results.append('transport_result=true' in text and text.count('denied=true')==3)
    emit('uds-permissions',directory_mode=oct(UDSDIR.stat().st_mode&0o777),directory_uid=UDSDIR.stat().st_uid,paths=[{'name':p.name,'mode':oct(p.stat().st_mode&0o777),'uid':p.stat().st_uid} for p in UDSDIR.iterdir()])
    live={'host_backend':proc_info(host),'vmms':{role:proc_info(proc) for role,cid,base,proc,console,begin in boots},'runner':proc_info(type('P',(),{'pid':os.getpid(),'poll':lambda self:None})())};(OUT/'live-process-resources.json').write_text(json.dumps(live,indent=2));raw('host-meminfo-live.txt',['cat','/proc/meminfo']);raw('live-links.json',['ip','-j','-d','link']);raw('live-unix-sockets.txt',['ss','-xap']);raw('live-vsock-sockets.txt',['ss','--vsock','-ap'])
    emit('transport-summary',actual_linux_microvms=2,configured_memory_mib_per_vm=128,configured_vcpu_per_vm=1,actual_host_taps_created=0,actual_host_bridge_ports_created=0,unique_guest_cids=[39501,39502],all_passed=all(results),scope='Rust guest AF_VSOCK -> CH userspace virtio-vsock -> owned Unix backend; ordinary TCP applications use explicit loopback wrapper')
    OK=all(results)
except BaseException as e:
    emit('probe-failure',error=str(e),trace=traceback.format_exc())
finally:
    for proc in reversed(PROCS):
        if proc.poll() is None:
            proc.send_signal(2 if proc.args[0]=='tcpdump' else 15)
            try:proc.wait(timeout=5)
            except subprocess.TimeoutExpired:proc.kill();proc.wait(timeout=3)
        emit('owned-process-stopped',pid=proc.pid,returncode=proc.returncode)
    for file in FILES:file.close()
    # Remove only exact registered paths after their owned processes are dead.
    for name in ['va','vb','va_5000','vb_5000','api-a','api-b','api-a.lock','api-b.lock','guest-a.ext4','guest-b.ext4']:
        p=(OUT if name.endswith('.ext4') else UDSDIR)/name
        if p.exists() or p.is_socket():p.unlink();emit('owned-path-removed',path=str(p))
    if UDSDIR.exists():UDSDIR.rmdir();emit('owned-uds-directory-removed',path=str(UDSDIR))
    if (OUT/'all-interfaces.pcap').exists():raw('capture-decode.txt',['tcpdump','-n','-e','-r',str(OUT/'all-interfaces.pcap')])
    if BEFORE is not None:
        AFTER=snap('after');a=normalize(BEFORE);b=normalize(AFTER);keys=sorted(k for k in a if a[k]!=b[k]);(OUT/'configuration-complement.json').write_text(json.dumps({'matched':not keys,'different_keys':keys,'dynamic_state_limitation':'Administrative configuration compared; neighbor state/timers, counters and page cache/RSS fluctuate. No foreign dynamic cache equality asserted.','before':a,'after':b},indent=2));emit('cleanup',administrative_configuration_matched=not keys,different_keys=keys,owned_processes_alive=[p.pid for p in PROCS if p.poll() is None],owned_uds_remaining=[p.name for p in OUT.iterdir() if p.is_socket()],new_host_netdevices_requested=0,sysctls_tuned=False,foreign_modules_unloaded=False);OK=OK and not keys
    emit('final',passed=OK,wall_s=time.perf_counter()-START);LOG.close()
raise SystemExit(0 if OK else 1)

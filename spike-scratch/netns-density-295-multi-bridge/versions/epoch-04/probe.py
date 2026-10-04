#!/usr/bin/env python3
"""PROBE ONLY: actual TAP queue fanout through stock Linux leaf/root bridges."""
import ctypes, fcntl, hashlib, json, os, pathlib, resource, select, shutil
import socket, struct, subprocess, sys, time, traceback, threading

START = time.perf_counter()
RUN = pathlib.Path(__file__).parent / ('run-' + str(os.getpid()))
RUN.mkdir()
LOG = open(RUN / 'events.jsonl', 'x')
FDS = []
OWNED = []
PROCS = []
TAG = 'm295' + format(os.getpid() & 65535, '04x')
ROOT = TAG + 'r'
LEAVES = []
MODE = sys.argv[1] if len(sys.argv)>1 else 'primitive'
HOST_NS = os.open('/proc/self/ns/net', os.O_RDONLY)
libc = ctypes.CDLL(None, use_errno=True)

def emit(event, **data):
    item = dict(event=event, at_s=time.perf_counter()-START, **data)
    line = json.dumps(item, sort_keys=True)
    print(line, flush=True)
    LOG.write(line+'\n'); LOG.flush()

def run(args, check=True, input=None, timeout=30):
    p = subprocess.run(args, input=input, text=True, capture_output=True, timeout=timeout)
    if check and p.returncode:
        raise RuntimeError(json.dumps(dict(argv=args, rc=p.returncode, stdout=p.stdout, stderr=p.stderr)))
    return p

def raw(name, args):
    p = run(args, check=False)
    (RUN / name).write_text(json.dumps(dict(argv=args, rc=p.returncode, stdout=p.stdout, stderr=p.stderr)))
    return p.stdout

def snap(prefix):
    data = {}
    for key,args in [('links',['ip','-j','-d','link']),('addrs',['ip','-j','address']),
                     ('routes',['ip','-j','route','show','table','all']),('rules',['ip','-j','rule']),
                     ('nft',['nft','-j','list','ruleset']),('netns',['ip','netns','list']),
                     ('bpfmaps',['bpftool','-j','map','show']),('bpflinks',['bpftool','-j','link','show'])]:
        out = raw(prefix+'-'+key+'.json',args)
        try: data[key] = json.loads(out)
        except ValueError: data[key] = out
    for key,path in [('modules','/proc/modules'),('sysctl-ip-forward','/proc/sys/net/ipv4/ip_forward'),
                     ('sysctl-rp-filter','/proc/sys/net/ipv4/conf/all/rp_filter')]:
        out = pathlib.Path(path).read_text(); (RUN/(prefix+'-'+key+'.txt')).write_text(out); data[key]=out
    raw(prefix+'-processes.txt',['ps','-e','-o','pid,ppid,comm,args'])
    raw(prefix+'-bpfpins.txt',['find','/sys/fs/bpf/overdrive','-maxdepth','4','-printf','%P %y %i\n'])
    return data

def normalize(data):
    # Preserve raw volatile values. Compare administrative configuration, not timers/counters.
    ignored={'valid_life_time','preferred_life_time','expires','lastuse','used','updated','stats64','gc_timer',
             'stats','cache','packets','bytes'}
    def walk(x):
        if isinstance(x,dict): return {k:walk(v) for k,v in sorted(x.items()) if k not in ignored}
        if isinstance(x,list): return sorted([walk(v) for v in x], key=lambda v:json.dumps(v,sort_keys=True))
        return x
    # /proc/modules reference counts can change during activity; module names/size/address are retained.
    result=walk(data)
    result['modules']=[' '.join(line.split()[:2]+line.split()[4:]) for line in data['modules'].splitlines()]
    return result

def memory():
    data={}
    for name,path in [('meminfo','/proc/meminfo'),('status','/proc/self/status')]:
        vals={}
        for line in pathlib.Path(path).read_text().splitlines():
            if ':' in line:
                key,value=line.split(':',1)
                if key in ('MemAvailable','MemFree','Slab','SReclaimable','SUnreclaim','VmRSS','VmHWM','VmSize','Threads'):
                    vals[key]=value.strip()
        data[name]=vals
    data['fd_count']=len(list(pathlib.Path('/proc/self/fd').iterdir()))
    data['file_nr']=pathlib.Path('/proc/sys/fs/file-nr').read_text().strip()
    return data

def checkpoint(count, create_s):
    links=json.loads(raw('stage-'+str(count)+'-links.json',['ip','-j','-d','link']))
    ports=json.loads(raw('stage-'+str(count)+'-bridge-ports.json',['bridge','-j','-d','link']))
    fdb=json.loads(raw('stage-'+str(count)+'-fdb.json',['bridge','-j','fdb']))
    master_counts={b:sum(l.get('master')==b for l in links) for b in [ROOT]+LEAVES}
    tap_links=[l for l in links if l['ifname'].startswith(TAG+'t')]
    owned_iff=[]
    for fd in FDS:
        text=pathlib.Path('/proc/self/fdinfo/'+str(fd)).read_text()
        owned_iff += [line.split()[1] for line in text.splitlines() if line.startswith('iff:')]
    if len(tap_links)!=count or len(owned_iff)!=count or len(set(owned_iff))!=count:
        raise RuntimeError('TAP count/queue custody readback mismatch')
    if any(n>1023 for n in master_counts.values()): raise RuntimeError('stock bridge port budget exceeded')
    emit('stage',taps=count, held_queue_fds=len(FDS), kernel_tap_links=len(tap_links),
         kernel_bridge_port_count=len(ports), bridges=len(LEAVES)+1, master_counts=master_counts,
         fdb_entries=len(fdb), create_interval_s=create_s, memory=memory(), namespace=os.readlink('/proc/self/ns/net'))

def checksum(data):
    if len(data)%2: data+=b'\x00'
    value=sum(struct.unpack('!'+str(len(data)//2)+'H',data))
    value=(value&65535)+(value>>16); value=(value&65535)+(value>>16)
    return (~value)&65535

def ipbytes(ip): return socket.inet_aton(ip)
def macbytes(mac): return bytes.fromhex(mac.replace(':',''))
def arp(srcmac,srcip,target,reply=False,dstmac=b'\xff'*6):
    return dstmac+srcmac+b'\x08\x06'+struct.pack('!HHBBH',1,0x800,6,4,2 if reply else 1)+srcmac+ipbytes(srcip)+(dstmac if reply else b'\x00'*6)+ipbytes(target)
def ipv4(srcmac,dstmac,srcip,dstip,proto,payload):
    hdr=struct.pack('!BBHHHBBH4s4s',0x45,0,20+len(payload),295,0,64,proto,0,ipbytes(srcip),ipbytes(dstip))
    hdr=hdr[:10]+struct.pack('!H',checksum(hdr))+hdr[12:]
    return dstmac+srcmac+b'\x08\x00'+hdr+payload

def ready(fd,seconds):
    observer=select.poll(); observer.register(fd,select.POLLIN)
    return bool(observer.poll(max(0,int(seconds*1000))))
def drain(fd):
    while ready(fd,0): os.read(fd,65536)
def receive(fd,predicate,seconds=2):
    until=time.perf_counter()+seconds
    observed=[]
    while time.perf_counter()<until:
        if ready(fd,max(0,until-time.perf_counter())):
            frame=os.read(fd,65536); observed.append(frame.hex())
            if predicate(frame): return frame,observed
    return None,observed

def packets():
    a,b=FDS[0],FDS[1022]
    aip,bip='100.95.0.2','100.95.4.0'
    am,bm=macbytes('02:00:64:5f:00:02'),macbytes('02:00:64:5f:04:00')
    gm=macbytes('02:01:00:00:00:01')
    results=[]
    def attempt(name,source,destination,frame,predicate):
        drain(source); drain(destination); start=time.perf_counter(); os.write(source,frame)
        got,seen=receive(destination,predicate)
        result=dict(name=name,passed=got is not None,elapsed_s=time.perf_counter()-start,
                    sent_hex=frame.hex(),received_hex=got.hex() if got else None,observed_frames=len(seen))
        results.append(result); emit('packet',**result)
        (RUN/(name+'-frames.json')).write_text(json.dumps(seen))
    attempt('cross-leaf-arp-request',a,b,arp(am,aip,bip),lambda f:f[:6]==b'\xff'*6 and f[12:14]==b'\x08\x06' and f[28:32]==ipbytes(aip))
    attempt('cross-leaf-arp-reply',b,a,arp(bm,bip,aip,True,am),lambda f:f[:12]==am+bm and f[20:22]==b'\x00\x02')
    udp=struct.pack('!HHHH',19000,19001,8+16,0)+b'm295-peer-proof!'
    # Correct UDP length for exact payload.
    udp=struct.pack('!HHHH',19000,19001,8+len(b'm295-peer-proof!'),0)+b'm295-peer-proof!'
    attempt('cross-leaf-unicast-ipv4',a,b,ipv4(am,bm,aip,bip,17,udp),lambda f:f[:12]==bm+am and b'm295-peer-proof!' in f)
    attempt('cross-leaf-unicast-ipv4-reverse',b,a,ipv4(bm,am,bip,aip,17,udp),lambda f:f[:12]==am+bm and b'm295-peer-proof!' in f)
    for fd,ip,mac,which in [(a,aip,am,'a'),(b,bip,bm,'b')]:
        attempt('gateway-arp-'+which,fd,fd,arp(mac,ip,'100.95.0.1'),lambda f:f[:12]==mac+gm and f[20:22]==b'\x00\x02' and f[28:32]==ipbytes('100.95.0.1'))
        payload=struct.pack('!BBHHH',8,0,0,295,1)+b'm295-gateway'
        payload=payload[:2]+struct.pack('!H',checksum(payload))+payload[4:]
        attempt('gateway-icmp-'+which,fd,fd,ipv4(mac,gm,ip,'100.95.0.1',1,payload),lambda f:f[:12]==mac+gm and f[12:14]==b'\x08\x00' and f[23]==1 and f[34]==0 and b'm295-gateway' in f)
    # An off-subnet Service IP is represented by a scratch host-local address only;
    # this proves gateway forwarding/local-delivery, not Overdrive service resolution/TPROXY.
    run(['ip','addr','add','10.98.0.1/32','dev','lo'])
    service=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); service.bind(('10.98.0.1',18951)); service.settimeout(2)
    for fd,ip,mac,which in [(a,aip,am,'a'),(b,bip,bm,'b')]:
        drain(fd); msg=('m295-service-'+which).encode()
        frame=ipv4(mac,gm,ip,'10.98.0.1',17,struct.pack('!HHHH',18952,18951,8+len(msg),0)+msg)
        start=time.perf_counter(); os.write(fd,frame)
        try:
            got,peer=service.recvfrom(4096); service.sendto(b'reply-'+got,peer)
            reply,seen=receive(fd,lambda f:f[:6]==mac and b'reply-'+msg in f)
            emit('service',guest=which,passed=got==msg and reply is not None,peer=peer,
                 original_destination='10.98.0.1:18951',elapsed_s=time.perf_counter()-start,
                 received_hex=reply.hex() if reply else None)
        except socket.timeout: emit('service',guest=which,passed=False,error='timeout')
    service.close()
    emit('traffic-summary',packet_results=results,scope='primitive TAP queues; no real VMs or production TCX/TPROXY/kTLS')

def real_vms():
    guest_source=pathlib.Path(__file__).parent/'guest.rs'; guest_binary=RUN/'guest-init'
    emit('guest-source',path=str(guest_source),sha256=hashlib.sha256(guest_source.read_bytes()).hexdigest())
    raw('guest-rustc-version.txt',['rustc','-Vv'])
    build=time.perf_counter()
    result=run(['rustc','--edition=2024','--target','x86_64-unknown-linux-musl','-C','linker=cc','-C','target-feature=+crt-static','-O',str(guest_source),'-o',str(guest_binary)],timeout=120)
    (RUN/'guest-build.txt').write_text(result.stdout+result.stderr)
    emit('guest-build',elapsed_s=time.perf_counter()-build,sha256=hashlib.sha256(guest_binary.read_bytes()).hexdigest(),bytes=guest_binary.stat().st_size)
    raw('guest-binary-file.txt',['file',str(guest_binary)])
    run(['ip','addr','add','10.98.0.1/32','dev','lo'])
    listeners=[];threads=[];stop=threading.Event()
    def echo_server(sock,label):
        while not stop.is_set():
            try: conn,peer=sock.accept()
            except socket.timeout: continue
            except OSError: break
            with conn:
                conn.settimeout(5); data=conn.recv(4096);conn.sendall(data)
                emit('host-tcp-echo',label=label,original_destination=conn.getsockname(),peer=peer,bytes=len(data),payload=data.decode())
    for address,port,label in [('100.95.0.1',18950,'gateway'),('10.98.0.1',18951,'off-subnet-service')]:
        sock=socket.socket(socket.AF_INET,socket.SOCK_STREAM);sock.bind((address,port));sock.listen(4);sock.settimeout(.2);listeners.append(sock)
        thread=threading.Thread(target=echo_server,args=(sock,label));thread.start();threads.append(thread)
    dns=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);dns.bind(('100.95.0.1',53));dns.settimeout(.2);listeners.append(dns)
    def dns_server():
        while not stop.is_set():
            try:query,peer=dns.recvfrom(512)
            except socket.timeout:continue
            except OSError:break
            target='100.95.4.0' if peer[0]=='100.95.0.2' else '100.95.0.2'
            answer=query[:2]+bytes.fromhex('81800001000100000000')+query[12:]+bytes.fromhex('c00c000100010000001e0004')+ipbytes(target)
            dns.sendto(answer,peer);emit('host-dns-fixture',peer=peer,answer=target)
    thread=threading.Thread(target=dns_server);thread.start();threads.append(thread)
    caperr=open(RUN/'capture.stderr','wb')
    capture=subprocess.Popen(['tcpdump','-i','any','-U','-n','-w',str(RUN/'all-interfaces.pcap')],stdout=subprocess.DEVNULL,stderr=caperr)
    PROCS.append(capture);time.sleep(.3)
    emit('capture-armed',pid=capture.pid,before_any_source_vmm_spawn=True,scope='all interfaces in one isolated node domain')
    boots=[]
    try:
        for role,index,ip,mac in [('a',0,'100.95.0.2','02:00:64:5f:00:02'),('b',1022,'100.95.4.0','02:00:64:5f:04:00')]:
            image=RUN/('guest-'+role+'.ext4')
            run(['cp','--reflink=auto',os.environ['OVERDRIVE_METAL_ROOTFS'],str(image)])
            run(['debugfs','-w','-R','rm /init',str(image)])
            run(['debugfs','-w','-R','write '+str(guest_binary)+' /init',str(image)])
            run(['debugfs','-w','-R','set_inode_field /init mode 0100755',str(image)])
            err=open(RUN/('guest-'+role+'.stderr'),'wb'); console=RUN/('guest-'+role+'.console')
            fd=FDS[index]
            args=['cloud-hypervisor','--cpus','boot=1','--memory','size=256M','--kernel',os.environ['OVERDRIVE_METAL_KERNEL'],
                  '--disk','path='+str(image)+',image_type=raw','--cmdline','console=ttyS0 panic=1 root=/dev/vda rw init=/init spike_role='+role,
                  '--serial','file='+str(console),'--console','off','--api-socket',str(RUN/('guest-'+role+'.api')),
                  '--net','fd=['+str(fd)+'],mac='+mac+',offload_tso=off,offload_ufo=off,offload_csum=off']
            begin=time.perf_counter();proc=subprocess.Popen(args,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=err,pass_fds=(fd,))
            PROCS.append(proc);boots.append((role,proc,console,begin,fd,err))
            emit('vmm-spawn',role=role,pid=proc.pid,guest_ip=ip,prefix=16,gateway='100.95.0.1',guest_mac=mac,
                 leaf=LEAVES[index//1022],tap_index=index,inherited_queue_fd=fd,scope='scratch VMM fixture, not production launcher confinement')
        until=time.perf_counter()+45
        while time.perf_counter()<until:
            texts=[console.read_text(errors='replace') if console.exists() else '' for _,_,console,_,_,_ in boots]
            if all('REAL_VM_ALL_TRAFFIC_PASS' in text for text in texts):break
            if any(proc.poll() is not None for _,proc,_,_,_,_ in boots):break
            time.sleep(.1)
        for role,proc,console,begin,fd,err in boots:
            text=console.read_text(errors='replace') if console.exists() else ''
            status={line.split(':',1)[0]:line.split(':',1)[1].strip() for line in pathlib.Path('/proc/'+str(proc.pid)+'/status').read_text().splitlines() if line.startswith(('VmRSS:','Threads:','Uid:','Gid:'))} if proc.poll() is None else {}
            q=[{'fd':int(path.name),'iff':line.split()[1]} for path in pathlib.Path('/proc/'+str(proc.pid)+'/fdinfo').glob('*') for line in path.read_text().splitlines() if line.startswith('iff:')] if proc.poll() is None else []
            emit('real-vm-result',role=role,pid=proc.pid,passed='REAL_VM_ALL_TRAFFIC_PASS' in text,elapsed_s=time.perf_counter()-begin,
                 console_lines=[line for line in text.splitlines() if 'REAL_VM_' in line or 'panicked' in line],status=status,queue_holders=q,
                 executable_sha256=hashlib.sha256(pathlib.Path('/proc/'+str(proc.pid)+'/exe').read_bytes()).hexdigest() if proc.poll() is None else None)
        emit('real-vm-summary',actual_vms=len(boots),kernel_taps=len(FDS),passed=all('REAL_VM_ALL_TRAFFIC_PASS' in console.read_text(errors='replace') for _,_,console,_,_,_ in boots),
             security='plain scratch TCP fixture; production TCX/TPROXY/kTLS/zero-frame/owner lifecycle not proven')
    finally:
        for role,proc,console,begin,fd,err in boots:
            if proc.poll() is None:
                proc.terminate()
                try:proc.wait(timeout=5)
                except subprocess.TimeoutExpired:proc.kill();proc.wait()
            err.close()
        stop.set()
        for sock in listeners:sock.close()
        for thread in threads:thread.join(timeout=3)
        capture.send_signal(2);capture.wait(timeout=5);caperr.close()
        emit('capture-finished',statistics=(RUN/'capture.stderr').read_text())
        raw('capture-decode.txt',['tcpdump','-n','-e','-r',str(RUN/'all-interfaces.pcap')])

def neighbor_pressure():
    # Each actual retained TAP emits one distinct, current-/16 guest ARP request.
    # This is NIC/neighbor admission, not a 1100-VM population.
    raw('neighbor-pressure-before-tables.json',['ip','-j','-s','ntable','show'])
    outcomes=[];gm=macbytes('02:01:00:00:00:01');begin=time.perf_counter();consecutive=0
    for index,fd in enumerate(FDS):
        ip=socket.inet_ntoa(struct.pack('!I',int.from_bytes(ipbytes('100.95.0.2'),'big')+index))
        mac=b'\x02\x00'+ipbytes(ip)
        drain(fd);start=time.perf_counter();os.write(fd,arp(mac,ip,'100.95.0.1'))
        got,seen=receive(fd,lambda f:f[:12]==mac+gm and f[12:14]==b'\x08\x06' and f[20:22]==b'\x00\x02',seconds=.25)
        outcomes.append(dict(index=index,ip=ip,passed=got is not None,elapsed_s=time.perf_counter()-start,observed_frames=len(seen)))
        consecutive=0 if got else consecutive+1
        count=index+1
        if count in [100,300,500,900,1023,1024,1025,1100] or got is None:
            neighbors=json.loads(raw('pressure-'+str(count)+'-neighbors.json',['ip','-j','neigh']))
            raw('pressure-'+str(count)+'-tables.json',['ip','-j','-s','ntable','show'])
            emit('neighbor-pressure-stage',distinct_sources=count,reply_passes=sum(x['passed'] for x in outcomes),
                 neighbor_entries=len(neighbors),by_device={b:sum(x['dev']==b for x in neighbors) for b in [ROOT]+LEAVES},
                 elapsed_s=time.perf_counter()-begin,memory=memory(),last=outcomes[-1])
        if consecutive>=3:break
    (RUN/'neighbor-pressure-outcomes.json').write_text(json.dumps(outcomes,indent=2))
    raw('neighbor-pressure-after-tables.json',['ip','-j','-s','ntable','show'])
    emit('neighbor-pressure-result',actual_taps=len(FDS),distinct_arp_sources=len(outcomes),reply_passes=sum(x['passed'] for x in outcomes),
         failures=[x for x in outcomes if not x['passed']],elapsed_s=time.perf_counter()-begin,vm_count=0)

before=None
switched=False
error=None
try:
    emit('identity',source_path=str(pathlib.Path(__file__).resolve()),source_sha256=hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
         parent_pid=os.getpid(),parent_executable=os.readlink('/proc/'+str(os.getpid())+'/exe'),
         parent_executable_sha256=hashlib.sha256(pathlib.Path('/proc/'+str(os.getpid())+'/exe').read_bytes()).hexdigest(),run_dir=str(RUN))
    for name,args in [('uname',['uname','-a']),('cpu',['lscpu','-J']),('virt',['systemd-detect-virt']),
                      ('ch-version',['cloud-hypervisor','--version']),('ip-version',['ip','-V']),('bridge-version',['bridge','-V']),
                      ('nft-version',['nft','--version']),('module-tun',['modinfo','tun']),('module-bridge',['modinfo','bridge']),
                      ('kernel-package',['dpkg-query','-W','linux-image-'+os.uname().release]),
                      ('tool-availability',['sh','-c','command -v cloud-hypervisor; command -v debugfs; command -v tcpdump; command -v busybox'])]:
        raw('preflight-'+name+'.txt',args)
    kernel=os.environ.get('OVERDRIVE_METAL_KERNEL'); rootfs=os.environ.get('OVERDRIVE_METAL_ROOTFS')
    for name,path in [('ch',shutil.which('cloud-hypervisor')),('guest-kernel',kernel),('guest-rootfs',rootfs),('kernel-config','/boot/config-'+os.uname().release)]:
        if path and pathlib.Path(path).is_file():
            emit('artifact',name=name,path=path,bytes=pathlib.Path(path).stat().st_size,sha256=hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest())
    if rootfs: raw('preflight-rootfs-bin.txt',['debugfs','-R','ls -l /bin',rootfs])
    raw('preflight-memory.txt',['cat','/proc/meminfo'])
    raw('preflight-slab.txt',['cat','/proc/slabinfo'])
    raw('preflight-kernel-config.txt',['cat','/boot/config-'+os.uname().release])
    before=snap('host-before')
    emit('preflight',resources=memory(),rlimit_nofile=resource.getrlimit(resource.RLIMIT_NOFILE),lease_owner=pathlib.Path('/run/lock/overdrive-metal-shared.owner').read_text())
    oldlimit=resource.getrlimit(resource.RLIMIT_NOFILE)
    resource.setrlimit(resource.RLIMIT_NOFILE,(min(16384,oldlimit[1]),oldlimit[1]))
    emit('process-rlimit',before=oldlimit,after=resource.getrlimit(resource.RLIMIT_NOFILE))
    if libc.unshare(0x40000000): raise OSError(ctypes.get_errno(),'unshare node-level network domain')
    switched=True
    emit('namespace',original_fd=HOST_NS,namespace=os.readlink('/proc/self/ns/net'),role='one isolated node domain, no per-VM namespaces')
    run(['ip','link','set','lo','up'])
    run(['ip','link','add',ROOT,'type','bridge'])
    OWNED.append(ROOT)
    run(['ip','link','set',ROOT,'address','02:01:00:00:00:01'])
    run(['ip','addr','add','100.95.0.1/16','dev',ROOT])
    run(['ip','link','set',ROOT,'up'])
    baseline=memory(); emit('topology-baseline',memory=baseline)
    last=0
    for goal in ([1024] if MODE=='vm' else [1100] if MODE=='pressure' else [2,1022,1023,1024,1025,2044,4095,4096,4097]):
        start=time.perf_counter(); batch=[]
        for index in range(last,goal):
            group=index//1022
            if group==len(LEAVES):
                leaf=TAG+'b'+str(group); up=TAG+'u'+str(group); rp=TAG+'p'+str(group)
                run(['ip','link','add',leaf,'type','bridge']); OWNED.append(leaf); LEAVES.append(leaf)
                run(['ip','link','set',leaf,'address','02:01:00:00:01:'+format(group+1,'02x')])
                run(['ip','link','set',leaf,'up'])
                run(['ip','link','add',up,'type','veth','peer','name',rp]); OWNED.append(up)
                run(['ip','link','set',up,'master',leaf]); run(['ip','link','set',rp,'master',ROOT])
                run(['ip','link','set',up,'up']); run(['ip','link','set',rp,'up'])
            name=TAG+'t'+format(index,'04x')
            fd=os.open('/dev/net/tun',os.O_RDWR|os.O_CLOEXEC|os.O_NONBLOCK)
            FDS.append(fd)
            fcntl.ioctl(fd,0x400454ca,struct.pack('16sH',name.encode(),0x5002 if MODE=='vm' else 0x1002))
            fcntl.ioctl(fd,0x400454cc,0)
            fcntl.ioctl(fd,0x400454d0,0)
            batch+=['link set '+name+' master '+LEAVES[group],'link set '+name+' up']
        run(['ip','-batch','-'],input='\n'.join(batch)+'\n',timeout=120)
        checkpoint(goal,time.perf_counter()-start)
        last=goal
    real_vms() if MODE=='vm' else neighbor_pressure() if MODE=='pressure' else packets()
    raw('probe-final-routes.json',['ip','-j','route','show','table','all'])
    raw('probe-final-neighbors.json',['ip','-j','neigh'])
    raw('probe-final-fdb.json',['bridge','-j','fdb'])
    raw('probe-final-nstat.txt',['nstat','-az'])
    raw('probe-final-arp-thresholds.txt',['sh','-c','for x in /proc/sys/net/ipv4/neigh/default/gc_thresh*; do test ! -e "$x" || cat "$x"; done'])
    emit('attachment-verdict',verdict='WORKS',taps=len(FDS),vm_count=2 if MODE=='vm' else 0,bridges=len(LEAVES)+1)
except BaseException as exc:
    error=str(exc); emit('error',error=error,traceback=traceback.format_exc())
finally:
    clean_start=time.perf_counter(); cleanup_errors=[]
    for proc in PROCS:
        if proc.poll() is None:
            proc.terminate()
            try: proc.wait(timeout=5)
            except subprocess.TimeoutExpired: proc.kill(); proc.wait()
    for fd in reversed(FDS):
        try: os.close(fd)
        except OSError as exc: cleanup_errors.append(str(exc))
    if switched:
        for name in reversed(OWNED):
            p=run(['ip','link','del',name],check=False)
            if p.returncode: cleanup_errors.append(p.stderr.strip())
        links=json.loads(raw('probe-cleanup-links.json',['ip','-j','-d','link']))
        emit('namespace-cleanup',owned_remaining=[l['ifname'] for l in links if l['ifname'].startswith(TAG)],elapsed_s=time.perf_counter()-clean_start,errors=cleanup_errors)
        if libc.setns(HOST_NS,0x40000000): raise OSError(ctypes.get_errno(),'restore host namespace')
    if before is not None:
        after=snap('host-after'); bn=normalize(before); an=normalize(after)
        (RUN/'host-before-normalized.json').write_text(json.dumps(bn,sort_keys=True,indent=2))
        (RUN/'host-after-normalized.json').write_text(json.dumps(an,sort_keys=True,indent=2))
        emit('foreign-complement',equal=bn==an,changed_sections=[k for k in bn if bn[k]!=an[k]],normalization='raw values retained; omit lifetime/counter/timer fields and module use counts only')
    os.close(HOST_NS)
    emit('complete',error=error,total_s=time.perf_counter()-START,code_retained=True,run_dir=str(RUN),cleanup_errors=cleanup_errors)
    LOG.close()
sys.exit(1 if error else 0)

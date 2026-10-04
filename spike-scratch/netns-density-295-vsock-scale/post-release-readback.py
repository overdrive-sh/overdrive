"""Read-only exact owned cleanup, native hardware/limit and upstream-source attestation."""
import pathlib,hashlib,json,subprocess,os,shutil
P=pathlib.Path
def sha(p):return hashlib.sha256(P(p).read_bytes()).hexdigest()
pid_ids=[809942,810294,810746,811085,811404,811758,814497]
paths=['/run/lock/overdrive-metal-shared.owner','/run/vs295-scale-810294','/run/vs295-scale-811758','/home/ubuntu/overdrive/spike-scratch/netns-density-295-vsock-scale/increment-a/__pycache__']
receipt={'uname':list(os.uname()),'exact_owned_pids_exist':{str(p):P('/proc/'+str(p)).exists() for p in pid_ids},'exact_owned_paths_exist':{p:P(p).exists() for p in paths},'meminfo':P('/proc/meminfo').read_text(),'cpu_online':P('/sys/devices/system/cpu/online').read_text(),'limits':P('/proc/self/limits').read_text(),'disk':dict(zip(['total','used','free'],shutil.disk_usage('/home/ubuntu/overdrive'))),'cgroup_readbacks':{}}
for path in ['/sys/fs/cgroup','/sys/fs/cgroup/user.slice','/sys/fs/cgroup/user.slice/user-1000.slice','/sys/fs/cgroup/user.slice/user-1000.slice/session-29353.scope']:
 d={'exists':P(path).exists()}
 if d['exists']:
  for name in ['pids.max','pids.current','pids.events','memory.max','memory.current','memory.events','cpu.max','cpu.stat','cgroup.controllers','cgroup.subtree_control','cpuset.cpus.effective']:
   if (P(path)/name).exists():d[name]=(P(path)/name).read_text()
 receipt['cgroup_readbacks'][path]=d
receipt['kernel_limits']={p:P(p).read_text() for p in ['/proc/sys/fs/file-max','/proc/sys/fs/file-nr','/proc/sys/fs/nr_open','/proc/sys/fs/epoll/max_user_watches','/proc/sys/kernel/threads-max','/proc/sys/kernel/pid_max','/proc/sys/vm/max_map_count','/proc/sys/net/core/somaxconn']}
sources=['virtio-devices/src/vsock/device.rs','virtio-devices/src/vsock/unix/muxer.rs','virtio-devices/src/vsock/unix/mod.rs','virtio-devices/src/device.rs','virtio-devices/src/thread_helper.rs','vmm/src/device_manager.rs','Cargo.toml']
receipt['upstream_checkouts']=[]
for p in P('/home/ubuntu/.cargo/git/checkouts').glob('cloud-hypervisor-*/9ed824d*'):
 d={'path':str(p),'source_sha256':{name:sha(p/name) for name in sources},'git_head':subprocess.check_output(['git','-C',str(p),'rev-parse','HEAD'],text=True).strip(),'git_status':subprocess.check_output(['git','-C',str(p),'status','--porcelain'],text=True)};receipt['upstream_checkouts'].append(d)
receipt['native_executed_sources_sha256']={str(p):sha(p) for p in P('/home/ubuntu/overdrive/spike-scratch/netns-density-295-vsock-scale/increment-c').rglob('*') if p.is_file()}
print(json.dumps(receipt,sort_keys=True))

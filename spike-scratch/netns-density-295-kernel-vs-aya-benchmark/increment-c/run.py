#!/usr/bin/env python3
"""Build frozen variants and run a common benchmark only in private KVM kernels."""
import argparse,fcntl,hashlib,json,os,pathlib,re,shutil,subprocess,threading,time
INC=pathlib.Path(__file__).resolve().parent;BASE=INC.parent
parser=argparse.ArgumentParser();parser.add_argument('--max',type=int,default=64);parser.add_argument('--order',default='kernel-tcp,aya-tcp,aya-udp,kernel-udp');args=parser.parse_args()
EVIDENCE=INC/'evidence';EVIDENCE.mkdir(exist_ok=True);OUT=BASE/'out'/INC.name;OUT.mkdir(parents=True,exist_ok=False)
lease=open('/tmp/netns-density-295-kernel-vs-aya-nested-qemu.lock','a+');fcntl.flock(lease,fcntl.LOCK_EX|fcntl.LOCK_NB)
def capture(cmd,**kw):
 start=time.perf_counter();p=subprocess.run(cmd,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,**kw)
 with(EVIDENCE/'commands.jsonl').open('a')as f:f.write(json.dumps({'argv':list(map(str,cmd)),'exit':p.returncode,'wall_s':time.perf_counter()-start,'output':p.stdout})+'\n')
 print(p.stdout,flush=True)
 if p.returncode:raise RuntimeError(f'{cmd} exited {p.returncode}')
 return p.stdout
physical={'modules':pathlib.Path('/proc/modules').read_text(),'links':sorted(p.name for p in pathlib.Path('/sys/class/net').iterdir()),'cpu_stat':pathlib.Path('/proc/stat').read_text(),'meminfo':pathlib.Path('/proc/meminfo').read_text()}
(EVIDENCE/'physical-before.json').write_text(json.dumps(physical,indent=2));kernel=capture(['uname','-r']).strip();assert kernel=='7.0.0-29-generic'
manifest={str(p.relative_to(INC)):hashlib.sha256(p.read_bytes()).hexdigest()for p in INC.rglob('*')if p.is_file()and'evidence'not in p.parts}
(EVIDENCE/'executed-source-manifest.json').write_text(json.dumps({'kernel':kernel,'hashes':manifest},indent=2))
env=os.environ.copy();env['CARGO_TARGET_DIR']=str(OUT/'target');env['CARGO_HOME']=str(BASE.parent/'netns-density-295-aya-vsock-proxy/out/cargo-home')
capture(['cargo','build','--release','--locked','--manifest-path',str(INC/'Cargo.toml')],env=env)
for protocol in ['tcp','udp']:
 e=env.copy();e['CARGO_TARGET_DIR']=str(OUT/(protocol+'-target'));e['RUSTFLAGS']='-C linker=bpf-linker'
 capture(['cargo','+nightly','build','--release','--locked','--target','bpfel-unknown-none','-Z','build-std=core','--manifest-path',str(INC/(protocol+'-bpf/Cargo.toml'))],env=e)
module=OUT/'module';shutil.copytree(INC/'module',module);capture(['make','-C',f'/lib/modules/{kernel}/build',f'M={module}','modules','-j2'])
for cmd in [['rustc','--version'],['cargo','+nightly','--version'],['bpf-linker','--version'],['lscpu']]:capture(cmd)
for name in ['Cargo.lock','tcp-bpf/Cargo.lock','udp-bpf/Cargo.lock']:
 shutil.copy2(INC/name,EVIDENCE/(name.replace('/','-')+'.executed'))
root=OUT/'rootfs';root.mkdir()
for name in ['proc','sys','dev','tmp','bin','lib','sbin']:(root/name).mkdir()
shutil.copy2('/bin/busybox',root/'bin/busybox')
for name in ['sh','mount','insmod','rmmod','cat','echo','uname','dmesg','poweroff','sha256sum','ls','sleep','ip','kill','date']:(root/'bin'/name).symlink_to('busybox')
binary=OUT/'target/release/aya-vsock-probe';shutil.copy2(binary,root/'probe');shutil.copy2(module/'shmproxy.ko',root/'shmproxy.ko')
for protocol in ['tcp','udp']:shutil.copy2(OUT/(protocol+'-target/bpfel-unknown-none/release/aya-vsock-bpf'),root/(protocol+'.o'))
perf_candidates=list(pathlib.Path('/usr/lib/linux-tools').glob('*/perf'))+list(pathlib.Path('/usr/lib/linux-tools').glob('perf'))
perf=next((p for p in perf_candidates if p.is_file()and p.read_bytes()[:4]==b'\x7fELF'),None)
if not perf:
 p=pathlib.Path('/usr/bin/perf');perf=p if p.is_file()and p.read_bytes()[:4]==b'\x7fELF'else None
if perf:shutil.copy2(perf,root/'perf')
(EVIDENCE/'perf-availability.json').write_text(json.dumps({'actual_elf':str(perf)if perf else None,'unsupported_if_missing':True}))
for p in [binary]+([perf]if perf else[]):
 for dependency in re.findall(r'(/[^\s()]+)',capture(['ldd',str(p)])):
  dest=root/dependency.lstrip('/');dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(dependency,dest)
insmods=[];pins={}
for line in capture(['modprobe','--show-depends','vhost_vsock']).splitlines():
 if not line.startswith('insmod '):continue
 src=pathlib.Path(line.split()[1]);dest=root/src.name.removesuffix('.zst');data=subprocess.check_output(['zstd','-dc',str(src)])if src.suffix=='.zst'else src.read_bytes();dest.write_bytes(data);insmods.append('insmod /'+dest.name+' || poweroff -f');pins[str(src)]=hashlib.sha256(src.read_bytes()).hexdigest()
with(OUT/'kernel').open('wb')as f:subprocess.run(['sudo','-n','cat',f'/boot/vmlinuz-{kernel}'],stdout=f,check=True)
shutil.copy2(OUT/'kernel',OUT/'Image');stock=(OUT/'Image').read_bytes();assert b'zimg'not in stock[:4096],'unexpected EFI zboot image'
config=pathlib.Path('/boot/config-'+kernel).read_text();(EVIDENCE/'kernel-config.selected.txt').write_text('\n'.join(l for l in config.splitlines()if any(x in l for x in ['BPF','VSOCK','VHOST','PERF','HZ=','KALLSYMS'])))
(EVIDENCE/'kernel-btf.sha256').write_text(hashlib.sha256(pathlib.Path('/sys/kernel/btf/vmlinux').read_bytes()).hexdigest()+'\n')
pins.update({p.name:hashlib.sha256(p.read_bytes()).hexdigest()for p in [OUT/'Image',binary,root/'shmproxy.ko',root/'tcp.o',root/'udp.o']+([root/'perf']if perf else[])})
(EVIDENCE/'executed-artifact-pins.json').write_text(json.dumps(pins,indent=2))
all_pass=True
for index,cell in enumerate(args.order.split(',')):
 variant,protocol=cell.split('-');runid=f'{index:02d}-{variant}-{protocol}';ev=EVIDENCE/runid;ev.mkdir();vo=OUT/runid;vo.mkdir()
 available=int(next(l.split()[1]for l in pathlib.Path('/proc/meminfo').read_text().splitlines()if l.startswith('MemAvailable:')));assert available>(16+8)*1024*1024
 (ev/'host-admission-budget.json').write_text(json.dumps({'host_memavailable_kib':available,'private_memory_mib':16384,'online_vcpus':4,'possible_vcpus':64,'host_reserve_gib':8,'no_physical_sysctl_changes':True,'physical_loadavg':pathlib.Path('/proc/loadavg').read_text(),'cpu_frequency_khz':{str(p):p.read_text().strip() for p in pathlib.Path('/sys/devices/system/cpu').glob('cpu*/cpufreq/scaling_cur_freq')}}))
 init='''#!/bin/sh
mount -t proc proc /proc
mount -t sysfs sys /sys
mount -t devtmpfs dev /dev
ip link set lo up
uname -a
cat /proc/sys/kernel/pid_max /proc/sys/kernel/threads-max
cat /sys/devices/system/cpu/online /sys/devices/system/cpu/possible
'''+ '\n'.join(insmods)+'\n'+'echo BENCH_PRE_VARIANT_MEMINFO\ncat /proc/meminfo\necho BENCH_PRE_VARIANT_SLABINFO\ncat /proc/slabinfo\necho BENCH_PRE_VARIANT_END\n'+('insmod /shmproxy.ko || poweroff -f\n'if variant=='kernel'else'')+'''echo BENCH_PERF_PMU_PROBE
/perf stat -a -e cycles,instructions -- sleep 0.1 2>&1
echo BENCH_PERF_SOFTWARE_PROBE
/perf stat -a -e cpu-clock,task-clock,context-switches,cpu-migrations -- sleep 0.1 2>&1
'''+f'/probe {variant} {protocol} {args.max}\n'+'''rc=$?
echo BENCH_HARNESS_EXIT=$rc
cat /proc/modules
'''+('rmmod shmproxy\necho BENCH_MODULE_UNLOAD_EXIT=$?\n'if variant=='kernel'else'')+'''echo BENCH_LIVE_MODULES_AFTER_UNLOAD
cat /proc/modules
cat /sys/class/net/lo/ifindex
dmesg
echo BENCH_COMPLETE_EXIT=$rc
poweroff -f
'''
 (root/'init').write_text(init);(root/'init').chmod(0o755);(ev/'init.executed').write_text(init)
 with(vo/'initramfs').open('wb')as archive:
  p=subprocess.Popen(['find','.','-print0'],cwd=root,stdout=subprocess.PIPE);subprocess.run(['cpio','--null','-o','--format=newc','--owner=0:0'],cwd=root,stdin=p.stdout,stdout=archive,check=True);assert p.wait()==0
 (ev/'initramfs.sha256').write_text(hashlib.sha256((vo/'initramfs').read_bytes()).hexdigest()+'\n')
 q=['qemu-system-x86_64','-machine','q35','-cpu','host','-accel','kvm','-m','16384','-smp','4,maxcpus=64','-nodefaults','-nographic','-monitor','none','-serial','stdio','-no-reboot','-kernel',str(OUT/'Image'),'-initrd',str(vo/'initramfs'),'-append','console=ttyS0 rdinit=/init panic=-1']
 (ev/'qemu-command.json').write_text(json.dumps(q,indent=2));started=time.perf_counter();p=subprocess.Popen(q,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,bufsize=1);stop=threading.Event()
 def observe():
  with(ev/'qemu-resources.jsonl').open('x')as f:
   while not stop.wait(0.5):
    try:
     v={'physical_elapsed_s':time.perf_counter()-started,'clock_ticks':os.sysconf('SC_CLK_TCK'),'qemu_pid':p.pid,'qemu_stat':pathlib.Path(f'/proc/{p.pid}/stat').read_text(),'qemu_status':pathlib.Path(f'/proc/{p.pid}/status').read_text(),'qemu_fds':len(list(pathlib.Path(f'/proc/{p.pid}/fd').iterdir())),'physical_stat':pathlib.Path('/proc/stat').read_text(),'physical_loadavg':pathlib.Path('/proc/loadavg').read_text(),'qemu_cgroup':pathlib.Path(f'/proc/{p.pid}/cgroup').read_text()};f.write(json.dumps(v)+'\n');f.flush()
    except FileNotFoundError:break
 observer=threading.Thread(target=observe);observer.start();timer=threading.Timer(3600,p.terminate);timer.start();serial=[]
 with(ev/'native-serial.log').open('x')as f:
  for line in p.stdout:
   f.write(line);f.flush();serial.append(line)
   # Every traffic window carries raw evidence to file, not a 50MiB console dump.
   if not line.startswith('{'):print(line,end='',flush=True)
   else:
    try:
     v=json.loads(line)
     with (ev/'event-observer.jsonl').open('a')as obs:obs.write(json.dumps({'physical_elapsed_s':time.perf_counter()-started,'event':v.get('event'),'id':v.get('id'),'population':v.get('population')})+'\n')
     print(json.dumps({k:v[k]for k in ['event','id','population','duration_s','delivered_roundtrips','confirmed_zero','usage']if k in v}),flush=True)
    except json.JSONDecodeError:print(line,end='',flush=True)
 rc=p.wait();timer.cancel();stop.set();observer.join();good=rc==0 and any('BENCH_COMPLETE_EXIT=0'in x for x in serial)
 (ev/'native-result.json').write_text(json.dumps({'qemu_exit':rc,'passed':good,'elapsed_s':time.perf_counter()-started,'qemu_pid_absent':not pathlib.Path(f'/proc/{p.pid}').exists(),'variant':variant,'protocol':protocol,'max_population':args.max,'private_kernel_only':True},indent=2));all_pass&=good
 if not good:break
physical_after={'modules':pathlib.Path('/proc/modules').read_text(),'links':sorted(p.name for p in pathlib.Path('/sys/class/net').iterdir())};(EVIDENCE/'physical-after.json').write_text(json.dumps(physical_after,indent=2));assert physical_after['modules']==physical['modules']and physical_after['links']==physical['links']
(EVIDENCE/'result.json').write_text(json.dumps({'all_pass':all_pass,'order':args.order,'maximum':args.max,'physical_modules_links_preserved':True},indent=2));fcntl.flock(lease,fcntl.LOCK_UN);lease.close();raise SystemExit(0 if all_pass else 1)

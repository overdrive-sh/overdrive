#!/usr/bin/env python3
"""Build against installed stock headers, then execute only in private nested QEMU."""
import fcntl, hashlib, json, os, pathlib, re, shutil, subprocess, sys, threading, time
INC = pathlib.Path(__file__).resolve().parent
SCRATCH = INC.parent
OUT = SCRATCH / 'out' / INC.name
EVIDENCE = INC / 'evidence'
OUT.mkdir(parents=True, exist_ok=False)
EVIDENCE.mkdir(exist_ok=True)
lease = open('/tmp/netns-density-295-ayavsproxy-nested-qemu.lock', 'a+')
fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
def run(args, **kw):
    started=time.perf_counter()
    p=subprocess.run(args, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, **kw)
    with (EVIDENCE/'commands.jsonl').open('a') as out:
        out.write(json.dumps({'argv':list(map(str,args)), 'exit':p.returncode,'wall_s':time.perf_counter()-started,'output':p.stdout})+'\n')
    print(p.stdout,flush=True)
    if p.returncode: raise RuntimeError(f'{args}: exit {p.returncode}')
    return p.stdout
kernel=run(['uname','-r']).strip()
sources={str(p.relative_to(INC)):hashlib.sha256(p.read_bytes()).hexdigest() for p in INC.rglob('*') if p.is_file() and 'evidence' not in p.parts}
(EVIDENCE/'executed-source-manifest.json').write_text(json.dumps({'kernel':kernel,'source_hashes':sources},indent=2))
env=os.environ.copy();env['CARGO_TARGET_DIR']=str(OUT/'target');env['CARGO_HOME']=str(SCRATCH/'out'/'cargo-home')
run(['cargo','build','--release','--manifest-path',str(INC/'Cargo.toml')],env=env)
bpf_env=env.copy();bpf_env['RUSTFLAGS']='-C linker=bpf-linker';bpf_env['CARGO_TARGET_DIR']=str(OUT/'bpf-target')
run(['cargo','+nightly','build','--release','--target','bpfel-unknown-none','-Z','build-std=core','--manifest-path',str(INC/'bpf/Cargo.toml')],env=bpf_env)
run(['rustc','--version']);run(['cargo','+nightly','--version']);run(['bpf-linker','--version'])
config=pathlib.Path('/boot/config-'+kernel).read_text();(EVIDENCE/'kernel-config.selected.txt').write_text('\n'.join(l for l in config.splitlines() if any(k in l for k in ['BPF','VSOCK','VHOST','NET_SOCK_MSG','STREAM_PARSER'])))
(EVIDENCE/'kernel-btf.sha256').write_text(hashlib.sha256(pathlib.Path('/sys/kernel/btf/vmlinux').read_bytes()).hexdigest()+'\n')
shutil.copy2(INC/'Cargo.lock',EVIDENCE/'Cargo.lock.executed');shutil.copy2(INC/'bpf/Cargo.lock',EVIDENCE/'bpf.Cargo.lock.executed')
root=OUT/'rootfs';root.mkdir()
for d in ['proc','sys','dev','tmp','bin','lib','sbin']:(root/d).mkdir(exist_ok=True)
shutil.copy2('/bin/busybox',root/'bin/busybox')
for name in ['sh','mount','insmod','rmmod','cat','echo','uname','dmesg','poweroff','sha256sum','ls','sleep','ip']:(root/'bin'/name).symlink_to('busybox')
binary=OUT/'target/release/aya-vsock-probe';shutil.copy2(binary,root/'probe');shutil.copy2(OUT/'bpf-target/bpfel-unknown-none/release/aya-vsock-bpf',root/'program.o')
shutil.copy2('/usr/bin/strace',root/'strace')
ldd=run(['ldd',str(binary)])+run(['ldd','/usr/bin/strace'])
for path in re.findall(r'(/[^\s()]+)',ldd):
    p=root/path.lstrip('/');p.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(path,p)
deps=run(['modprobe','--show-depends','vhost_vsock'])
insmods=[];pins={}
for line in deps.splitlines():
    if not line.startswith('insmod '): continue
    src=pathlib.Path(line.split()[1]);dest=root/src.name.removesuffix('.zst');data=subprocess.check_output(['zstd','-dc',str(src)]) if src.suffix=='.zst' else src.read_bytes();dest.write_bytes(data)
    insmods.append('insmod /'+dest.name+' || poweroff -f');pins[str(src)] = hashlib.sha256(src.read_bytes()).hexdigest()
with (OUT/'kernel').open('wb') as f: subprocess.run(['sudo','-n','cat',f'/boot/vmlinuz-{kernel}'],stdout=f,check=True)
# Ubuntu EFI zboot wraps the stock Image in a zstd payload. Decompress only,
# without modifying or recompiling the kernel. Header ABI: zboot-header.S.
stock=(OUT/'kernel').read_bytes()
zimg=stock.find(b'zimg')
if zimg >= 4:
    header=zimg-4
    offset=int.from_bytes(stock[header+8:header+12],'little')
    length=int.from_bytes(stock[header+12:header+16],'little')
    payload=stock[header+offset:header+offset+length]
    assert payload[:4] == bytes.fromhex('28b52ffd')
    image=subprocess.check_output(['zstd','-dc'],input=payload)
    assert image[56:60] == b'ARM\x64'
    (OUT/'Image').write_bytes(image)
    (EVIDENCE/'stock-image-extraction.json').write_text(json.dumps({'zboot_header_offset':header,'payload_offset':offset,'compressed_length':length,'stock_vmlinuz_sha256':hashlib.sha256(stock).hexdigest(),'executed_Image_sha256':hashlib.sha256(image).hexdigest(),'decompression_only_no_patch_or_rebuild':True},indent=2))
else:
    shutil.copy2(OUT/'kernel',OUT/'Image')
pins.update({'stock_kernel_image':hashlib.sha256((OUT/'kernel').read_bytes()).hexdigest(),'bpf_elf':hashlib.sha256((root/'program.o').read_bytes()).hexdigest(),'probe_binary':hashlib.sha256(binary.read_bytes()).hexdigest(),'busybox':hashlib.sha256((root/'bin/busybox').read_bytes()).hexdigest()})
(EVIDENCE/'executed-artifact-pins.json').write_text(json.dumps(pins,indent=2,sort_keys=True))
init='''#!/bin/sh
mount -t proc proc /proc
mount -t sysfs sys /sys
mount -t devtmpfs dev /dev
ip link set lo up
echo AYAPROXY_HOST_ROLE_KERNEL
uname -a
echo AYAPROXY_STOCK_DEFAULT_LIMITS
cat /proc/sys/kernel/pid_max /proc/sys/kernel/threads-max
cat /sys/devices/system/cpu/online /sys/devices/system/cpu/possible
echo AYAPROXY_INITIAL_MODULES
cat /proc/modules
'''+ '\n'.join(insmods)+'''
echo AYAPROXY_EXECUTED_HASHES
sha256sum /probe /program.o
echo AYAPROXY_MODULES_LOADED
cat /proc/modules
echo AYAPROXY_LINKS
ls /sys/class/net
/probe
rc=$?
echo AYAPROXY_HARNESS_EXIT=$rc
cat /proc/modules
echo AYAPROXY_FINAL_LINKS
ls /sys/class/net
dmesg
echo AYAPROXY_COMPLETE_EXIT=$rc
poweroff -f
'''
(root/'init').write_text(init);(root/'init').chmod(0o755);(EVIDENCE/'init.executed').write_text(init)
with (OUT/'initramfs').open('wb') as archive:
    p=subprocess.Popen(['find','.','-print0'],cwd=root,stdout=subprocess.PIPE)
    subprocess.run(['cpio','--null','-o','--format=newc','--owner=0:0'],cwd=root,stdin=p.stdout,stdout=archive,check=True)
    assert p.wait()==0
available=int(next(l.split()[1] for l in pathlib.Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:')))
assert available > (16+8)*1024*1024, 'private 16GiB VM plus 8GiB host reserve unavailable'
(EVIDENCE/'host-admission-budget.json').write_text(json.dumps({'host_memavailable_kib':available,'private_vm_memory_kib':16*1024*1024,'host_reserve_kib':8*1024*1024,'cpu_boot':4,'cpu_possible':64,'kernel_default_pid_budget_uses_possible_cpus':True,'no_sysctl_tuning':True},indent=2))
args=['qemu-system-x86_64','-machine','q35','-cpu','host','-accel','kvm','-m','16384','-smp','4,maxcpus=64','-nodefaults','-nographic','-monitor','none','-serial','stdio','-no-reboot','-kernel',str(OUT/'Image'),'-initrd',str(OUT/'initramfs'),'-append','console=ttyS0 rdinit=/init panic=-1']
(EVIDENCE/'qemu-command.json').write_text(json.dumps(args,indent=2))
started=time.perf_counter();proc=subprocess.Popen(args,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,bufsize=1)
samples=[];stop=threading.Event()
def sample():
    while not stop.wait(1):
        try:samples.append({'elapsed_s':time.perf_counter()-started,'pid':proc.pid,'status':pathlib.Path(f'/proc/{proc.pid}/status').read_text(),'stat':pathlib.Path(f'/proc/{proc.pid}/stat').read_text(),'fds':len(list(pathlib.Path(f'/proc/{proc.pid}/fd').iterdir()))})
        except FileNotFoundError:break
observer=threading.Thread(target=sample);observer.start()
timer=threading.Timer(600,proc.terminate);timer.start()
serial=[]
with (EVIDENCE/'native-serial.log').open('x') as capture:
    for line in proc.stdout:capture.write(line);capture.flush();serial.append(line);print(line,end='',flush=True)
rc=proc.wait();timer.cancel();stop.set();observer.join()
(EVIDENCE/'qemu-resources.json').write_text(json.dumps(samples,indent=2))
good=rc==0 and any('AYAPROXY_COMPLETE_EXIT=0' in l for l in serial)
(EVIDENCE/'native-result.json').write_text(json.dumps({'qemu_exit':rc,'host_role_pass':good,'wall_s':time.perf_counter()-started,'qemu_pid_absent':not pathlib.Path(f'/proc/{proc.pid}').exists(),'bpf_loaded_only_inside_qemu':True, 'custom_kernel_module_loaded':False,'lease_release_after_result':True},indent=2))
fcntl.flock(lease,fcntl.LOCK_UN);lease.close()
sys.exit(0 if good else 1)

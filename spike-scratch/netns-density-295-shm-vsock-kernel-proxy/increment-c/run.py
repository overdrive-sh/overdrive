#!/usr/bin/env python3
"""Build against installed stock headers, then execute only in private nested QEMU."""
import fcntl, hashlib, json, os, pathlib, re, shutil, subprocess, sys, threading, time
INC = pathlib.Path(__file__).resolve().parent
SCRATCH = INC.parent
OUT = SCRATCH / 'out' / INC.name
EVIDENCE = INC / 'evidence'
OUT.mkdir(parents=True, exist_ok=False)
EVIDENCE.mkdir(exist_ok=True)
lease = open('/tmp/netns-density-295-shmproxy-nested-qemu.lock', 'a+')
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
module=OUT/'module';shutil.copytree(INC/'module',module)
run(['make','-C',f'/lib/modules/{kernel}/build',f'M={module}','modules','-j2'])
env=os.environ.copy();env['CARGO_TARGET_DIR']=str(OUT/'target');env['CARGO_HOME']=str(OUT/'cargo-home')
run(['cargo','build','--release','--manifest-path',str(INC/'Cargo.toml')],env=env)
shutil.copy2(INC/'Cargo.lock',EVIDENCE/'Cargo.lock.executed')
root=OUT/'rootfs';root.mkdir()
for d in ['proc','sys','dev','tmp','bin','lib','sbin']:(root/d).mkdir(exist_ok=True)
shutil.copy2('/bin/busybox',root/'bin/busybox')
for name in ['sh','mount','insmod','rmmod','cat','echo','uname','dmesg','poweroff','sha256sum','ls','sleep']:(root/'bin'/name).symlink_to('busybox')
binary=OUT/'target/release/kernel-vsock-proxy-probe';shutil.copy2(binary,root/'probe');shutil.copy2(module/'shmproxy.ko',root/'shmproxy.ko')
ldd=run(['ldd',str(binary)])
for path in re.findall(r'(/[^\s()]+)',ldd):
    p=root/path.lstrip('/');p.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(path,p)
deps=run(['modprobe','--show-depends','vhost_vsock'])
insmods=[];pins={}
for line in deps.splitlines():
    if not line.startswith('insmod '): continue
    src=pathlib.Path(line.split()[1]);dest=root/src.name.removesuffix('.zst');data=subprocess.check_output(['zstd','-dc',str(src)]) if src.suffix=='.zst' else src.read_bytes();dest.write_bytes(data)
    insmods.append('insmod /'+dest.name+' || poweroff -f');pins[str(src)] = hashlib.sha256(src.read_bytes()).hexdigest()
with (OUT/'kernel').open('wb') as f: subprocess.run(['sudo','-n','cat',f'/boot/vmlinuz-{kernel}'],stdout=f,check=True)
pins.update({'stock_kernel_image':hashlib.sha256((OUT/'kernel').read_bytes()).hexdigest(),'module':hashlib.sha256((root/'shmproxy.ko').read_bytes()).hexdigest(),'probe_binary':hashlib.sha256(binary.read_bytes()).hexdigest(),'busybox':hashlib.sha256((root/'bin/busybox').read_bytes()).hexdigest()})
(EVIDENCE/'executed-artifact-pins.json').write_text(json.dumps(pins,indent=2,sort_keys=True))
init='''#!/bin/sh
mount -t proc proc /proc
mount -t sysfs sys /sys
mount -t devtmpfs dev /dev
echo SHMPROXY_HOST_ROLE_KERNEL
uname -a
echo SHMPROXY_INITIAL_MODULES
cat /proc/modules
'''+ '\n'.join(insmods)+'''
insmod /shmproxy.ko || poweroff -f
echo SHMPROXY_EXECUTED_HASHES
sha256sum /probe /shmproxy.ko
echo SHMPROXY_MODULES_LOADED
cat /proc/modules
echo SHMPROXY_LINKS
ls /sys/class/net
/probe
rc=$?
echo SHMPROXY_HARNESS_EXIT=$rc
cat /proc/shmproxy
rmmod shmproxy
unload=$?
echo SHMPROXY_MODULE_UNLOAD_EXIT=$unload
cat /proc/modules
echo SHMPROXY_FINAL_LINKS
ls /sys/class/net
dmesg
echo SHMPROXY_COMPLETE_EXIT=$rc
poweroff -f
'''
(root/'init').write_text(init);(root/'init').chmod(0o755);(EVIDENCE/'init.executed').write_text(init)
with (OUT/'initramfs').open('wb') as archive:
    p=subprocess.Popen(['find','.','-print0'],cwd=root,stdout=subprocess.PIPE)
    subprocess.run(['cpio','--null','-o','--format=newc','--owner=0:0'],cwd=root,stdin=p.stdout,stdout=archive,check=True)
    assert p.wait()==0
args=['qemu-system-aarch64','-machine','virt','-cpu','max','-accel','tcg','-m','1024','-smp','2','-nodefaults','-nographic','-monitor','none','-serial','stdio','-no-reboot','-kernel',str(OUT/'kernel'),'-initrd',str(OUT/'initramfs'),'-append','console=ttyAMA0 rdinit=/init panic=-1']
(EVIDENCE/'qemu-command.json').write_text(json.dumps(args,indent=2))
started=time.perf_counter();proc=subprocess.Popen(args,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,bufsize=1)
samples=[];stop=threading.Event()
def sample():
    while not stop.wait(1):
        try:samples.append({'elapsed_s':time.perf_counter()-started,'pid':proc.pid,'status':pathlib.Path(f'/proc/{proc.pid}/status').read_text(),'stat':pathlib.Path(f'/proc/{proc.pid}/stat').read_text(),'fds':len(list(pathlib.Path(f'/proc/{proc.pid}/fd').iterdir()))})
        except FileNotFoundError:break
observer=threading.Thread(target=sample);observer.start()
timer=threading.Timer(300,proc.terminate);timer.start()
serial=[]
with (EVIDENCE/'native-serial.log').open('x') as capture:
    for line in proc.stdout:capture.write(line);capture.flush();serial.append(line);print(line,end='',flush=True)
rc=proc.wait();timer.cancel();stop.set();observer.join()
(EVIDENCE/'qemu-resources.json').write_text(json.dumps(samples,indent=2))
good=rc==0 and any('SHMPROXY_COMPLETE_EXIT=0' in l for l in serial) and any('SHMPROXY_MODULE_UNLOAD_EXIT=0' in l for l in serial)
(EVIDENCE/'native-result.json').write_text(json.dumps({'qemu_exit':rc,'host_role_pass':good,'wall_s':time.perf_counter()-started,'qemu_pid_absent':not pathlib.Path(f'/proc/{proc.pid}').exists(),'module_loaded_only_inside_qemu':True,'lease_release_after_result':True},indent=2))
fcntl.flock(lease,fcntl.LOCK_UN);lease.close()
sys.exit(0 if good else 1)

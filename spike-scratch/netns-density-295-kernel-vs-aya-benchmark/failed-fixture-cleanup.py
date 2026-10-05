#!/usr/bin/env python3
"""Stop the single verified-owned failed private VM via the runner timeout signal."""
import hashlib,json,os,pathlib,signal,time
needle='/spike-scratch/netns-density-295-kernel-vs-aya-benchmark/out/increment-e/03-aya-udp/initramfs'
found=[]
for entry in pathlib.Path('/proc').iterdir():
 if not entry.name.isdigit():continue
 try:
  cmd=(entry/'cmdline').read_bytes().split(b'\0');argv=[x.decode()for x in cmd if x]
  if argv and argv[0].endswith('qemu-system-x86_64')and'-initrd'in argv and argv[argv.index('-initrd')+1].endswith(needle):
   stat=(entry/'stat').read_text();start=stat.rsplit(')',1)[1].split()[19];found.append((int(entry.name),argv,start))
 except(FileNotFoundError,PermissionError,UnicodeDecodeError):pass
assert len(found)==1,f'expected exactly one owned failed VM, found {len(found)}'
pid,argv,start=found[0]
verify=pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()[19];assert verify==start
receipt={'timestamp_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'target_scope':'only verified-owned failed private QEMU/KVM host-role VM','pid':pid,'process_start_ticks':start,'argv':argv,'signal':'SIGTERM','reason':'Aya UDP benchmark actor panicked on a late previous-window nonce; controller finish barrier cannot return','authorization':'explicit orchestrator instruction, equivalent to predeclared fixture timeout; benchmark agent/objective continue','no_physical_module_or_network_mutation':True,'live_private_kernel_reference_zero_proven':False}
os.kill(pid,signal.SIGTERM)
end=time.monotonic()+15
while pathlib.Path(f'/proc/{pid}').exists()and time.monotonic()<end:time.sleep(.1)
receipt['qemu_pid_absent']=not pathlib.Path(f'/proc/{pid}').exists();print(json.dumps(receipt));assert receipt['qemu_pid_absent']

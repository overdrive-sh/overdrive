import pathlib,json,subprocess,os
base=pathlib.Path('overdrive/spike-scratch/netns-density-295-shm-vsock-kernel-proxy')
owned=[]
for p in pathlib.Path('/proc').iterdir():
 if p.name.isdigit():
  try:
   cmd=(p/'cmdline').read_bytes().replace(bytes([0]),b' ').decode(errors='replace')
   if 'qemu-system' in cmd and str(base)+'/out/' in cmd:owned.append({'pid':int(p.name),'argv':cmd})
  except (OSError,PermissionError):pass
module='shmproxy ' in pathlib.Path('/proc/modules').read_text()
results={'owned_qemu_processes':owned,'physical_host_shmproxy_module_present':module,'canonical_lease_owner_metadata_present':pathlib.Path('/run/lock/overdrive-metal-shared.owner').exists(),'clock_ticks_per_s':os.sysconf('SC_CLK_TCK'),'physical_host_kernel':subprocess.check_output(['uname','-r'],text=True).strip()}
print(json.dumps(results))

"""Read-only attestation after all canonical leases are released."""
import pathlib,subprocess,json,hashlib,os,sys
P=pathlib.Path;owned=json.loads(sys.argv[1]);base=P('/home/ubuntu/overdrive/spike-scratch/netns-density-295-ovs-kernel')
def command(a):
 r=subprocess.run(a,capture_output=True,text=True);return {'argv':a,'rc':r.returncode,'stdout':r.stdout,'stderr':r.stderr}
paths=['/run/lock/overdrive-metal-shared.owner']+['/run/netns/'+n for n in owned['namespaces']]
modules=command(['modinfo','openvswitch']);module=command(['modinfo','-n','openvswitch'])['stdout'].strip()
receipt={'uname':list(os.uname()),'known_owned_pids_exist':{str(p):P('/proc/'+str(p)).exists()for p in owned['pids']},'known_owned_paths_exist':{p:P(p).exists()for p in paths},'netns':command(['ip','netns','list']),'foreign_ovs':command(['python3',str(base/'increment-d/audit.py'),'0']),'iproute2_version':command(['ip','-V']),'module_info':modules,'module_sha256':hashlib.sha256(P(module).read_bytes()).hexdigest(),'loaded_module_srcversion':P('/sys/module/openvswitch/srcversion').read_text(),'loaded_module_taint':P('/sys/module/openvswitch/taint').read_text(),'cpu_online':P('/sys/devices/system/cpu/online').read_text(),'meminfo':P('/proc/meminfo').read_text(),'limits':P('/proc/self/limits').read_text(),'packages':command(['dpkg-query','-W','linux-image-7.0.0-29-generic','linux-modules-7.0.0-29-generic','linux-modules-extra-7.0.0-29-generic','iproute2']), 'kernel_limits':{p:P(p).read_text()for p in ['/proc/sys/fs/file-max','/proc/sys/fs/nr_open','/proc/sys/net/ipv4/neigh/default/gc_thresh3']if P(p).exists()},'native_executed_sources_sha256':{str(p.relative_to(base/'increment-d')):hashlib.sha256(p.read_bytes()).hexdigest()for p in (base/'increment-d').rglob('*')if p.is_file()and'evidence'not in p.parts}}
processes=[]
for p in P('/proc').iterdir():
 if p.name.isdigit():
  try:
   name=(p/'comm').read_text().strip()
   if name in ['ovs-vswitchd','ovsdb-server','ovs-dpctl']:processes.append({'pid':int(p.name),'comm':name})
  except (FileNotFoundError,PermissionError,ProcessLookupError):pass
receipt['ovs_userspace_processes']=processes
assert not any(receipt['known_owned_pids_exist'].values()) and not any(receipt['known_owned_paths_exist'].values())
assert receipt['foreign_ovs']['rc']==0 and json.loads(receipt['foreign_ovs']['stdout'])['datapaths']==[]
print(json.dumps(receipt,sort_keys=True))

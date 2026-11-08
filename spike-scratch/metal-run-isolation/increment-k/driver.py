import json,os,subprocess
from pathlib import Path
repo=Path.cwd();out=repo/'docs/feature/metal-run-isolation/spike';base=repo/'spike-scratch/metal-run-isolation/increment-k'
for name in list(os.environ):
    if name.startswith('CARGO_TARGET_') and name.endswith('_RUNNER'):os.environ.pop(name)
os.environ['CARGO_TARGET_DIR']=str(repo/'target')

def run(name,argv,expected=(0,),timeout=1200):
 print('DESIGN-COMMAND',name,json.dumps(argv),flush=True)
 with (out/('k-'+name+'.stdout')).open('w') as stdout,(out/('k-'+name+'.stderr')).open('w') as stderr:
  result=subprocess.run(argv,stdout=stdout,stderr=stderr,timeout=timeout)
 print('DESIGN-RESULT',name,'status='+str(result.returncode),flush=True)
 if result.returncode not in expected:print((out/('k-'+name+'.stderr')).read_text()[-16000:],flush=True)
 return result.returncode

# Independent cleanup construction work continues even if a contrast fails.
cleanup=run('timeout-cleanup',['python3','spike-scratch/metal-run-isolation/increment-h/driver.py'],expected=(0,),timeout=600)
original=run('native-original-baseline',['cargo','nextest','run','-p','overdrive-host','--test','integration','--features','integration-tests,kvm-tests','-E','test(integration::vsock_vmm_device::the_fork_boots_and_passes_data_under_the_production_launch_identity)','--no-capture'],expected=(0,100),timeout=180)
diag=run('native-observer-copy',['cargo','nextest','run','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target'),'-E','test(the_fork_boots_and_passes_data_under_the_production_launch_identity)','--no-capture'],expected=(0,100),timeout=600)
print('DESIGN-CONTRAST',json.dumps({'cleanup_construction_status':cleanup,'unmodified_native_initial_namespace_status':original,'diagnostic_only_copy_initial_namespace_status':diag}),flush=True)

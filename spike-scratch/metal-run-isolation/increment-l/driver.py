import json,os,subprocess
from pathlib import Path
repo=Path.cwd();out=repo/'docs/feature/metal-run-isolation/spike';base=repo/'spike-scratch/metal-run-isolation/increment-l'
os.environ['OVERDRIVE_METAL_DESIGN_REPO']=str(repo);os.environ['CARGO_TARGET_DIR']=str(repo/'target')
for name in list(os.environ):
 if name.startswith('CARGO_TARGET_') and name.endswith('_RUNNER'):os.environ.pop(name)
for name,command in [('timeout-cleanup',['python3','spike-scratch/metal-run-isolation/increment-h/driver.py']),('native-diagnostic',['cargo','nextest','run','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target'),'-E','test(the_fork_boots_and_passes_data_under_the_production_launch_identity)','--no-capture'])]:
 print('DESIGN-COMMAND',name,json.dumps(command),flush=True)
 with (out/('l-'+name+'.stdout')).open('w') as stdout,(out/('l-'+name+'.stderr')).open('w') as stderr:
  result=subprocess.run(command,stdout=stdout,stderr=stderr,timeout=600)
 print('DESIGN-RESULT',name,'status='+str(result.returncode),flush=True)
 print((out/('l-'+name+'.stdout')).read_text()[-4000:]+(out/('l-'+name+'.stderr')).read_text()[-16000:],flush=True)

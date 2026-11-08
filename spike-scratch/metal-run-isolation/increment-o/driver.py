import json,os,subprocess
from pathlib import Path
repo=Path.cwd();base=repo/'spike-scratch/metal-run-isolation/increment-g';out=repo/'docs/feature/metal-run-isolation/spike'
os.environ['OVERDRIVE_METAL_DESIGN_REPO']=str(repo);os.environ['CARGO_TARGET_DIR']=str(repo/'target')
os.environ['PROPTEST_CASES']='7';os.environ['PROPTEST_RNG_SEED']='123456';os.environ['RUST_BACKTRACE']='1'
for key in list(os.environ):
 if key.startswith('CARGO_TARGET_') and key.endswith('_RUNNER'):os.environ.pop(key)
config=base/'env-nextest.toml'
config.write_text('experimental = ["wrapper-scripts"]\n[scripts.wrapper.metal-design]\ncommand = '+json.dumps(['python3',str(base/'local_netns_exec.py'),str(repo/'target/debug/metal-isolation-local-vsock-spike')])+'\n[[profile.default.scripts]]\nfilter = "all()"\nrun-wrapper = "metal-design"\n[profile.default]\nslow-timeout = { period = "60s", terminate-after = 3 }\n')
commands=[('prepare-current-dispatcher',['cargo','build','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target')]),('guest-environment',['cargo','nextest','run','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target'),'--config-file',str(config),'-E','test(selected_guest_environment_is_preserved)','--no-capture']),('timeout-cleanup',['python3','spike-scratch/metal-run-isolation/increment-h/driver.py'])]
statuses={}
for name,command in commands:
 print('DESIGN-COMMAND',name,json.dumps(command),flush=True)
 with (out/('o-'+name+'.stdout')).open('w') as stdout,(out/('o-'+name+'.stderr')).open('w') as stderr:
  result=subprocess.run(command,stdout=stdout,stderr=stderr,timeout=600)
 statuses[name]=result.returncode
 print('DESIGN-RESULT',name,'status='+str(result.returncode),flush=True)
 print('RAW-BEGIN '+name+'\n'+(out/('o-'+name+'.stdout')).read_text()+(out/('o-'+name+'.stderr')).read_text()+'\nRAW-END '+name,flush=True)
 if name=='timeout-cleanup':
  for label in ['without-cleanup','with-cleanup']:
   for stream in ['stdout','stderr']:
    p=out/('h-'+label+'.'+stream)
    if p.exists(): print('CLEANUP-RAW '+label+' '+stream+'\n'+p.read_text(),flush=True)
assert all(status==0 for status in statuses.values()),statuses
print('DESIGN-ENV-AND-CLEANUP-PROOF',json.dumps({'kernel':os.uname().release,'statuses':statuses,'guest_property_and_attempt_environment_preserved':True,'actual_nextest_timeout_cleanup_contrast_held':True},sort_keys=True),flush=True)

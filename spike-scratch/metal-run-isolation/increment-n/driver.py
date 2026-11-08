import json,os,re,shutil,stat,subprocess
from pathlib import Path
repo=Path.cwd();out=repo/'docs/feature/metal-run-isolation/spike';base=repo/'spike-scratch/metal-run-isolation/increment-n'
os.environ['OVERDRIVE_METAL_DESIGN_REPO']=str(repo);os.environ['CARGO_TARGET_DIR']=str(repo/'target')
for key in list(os.environ):
 if key.startswith('CARGO_TARGET_') and key.endswith('_RUNNER'):os.environ.pop(key)
run_id=re.search(r'overdrive-metal-run-([0-9a-f]{24})\.service',Path('/proc/self/cgroup').read_text())[1]
parent=Path('/srv/vm/metal');made_parent=False
if not parent.exists():
 parent.mkdir(mode=0o710);os.chown(parent,0,4200);os.chmod(parent,0o710);made_parent=True
metadata=parent.stat();print('NATIVE-BASE',metadata.st_uid,metadata.st_gid,oct(stat.S_IMODE(metadata.st_mode)),flush=True);assert metadata.st_uid==0 and metadata.st_gid==4200 and stat.S_IMODE(metadata.st_mode)==0o710
root=parent/run_id;root.mkdir(mode=0o710);os.chown(root,0,4200);os.chmod(root,0o710)
os.environ['METAL_DESIGN_RUN_ROOT']=str(root)
argv=['cargo','nextest','run','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target'),'-E','test(the_fork_boots_and_passes_data_under_the_production_launch_identity)','--no-capture']
results=[]
try:
 for label,command in [('initial',argv),('local',['python3','spike-scratch/metal-run-isolation/increment-g/local_netns_exec.py',*argv])]:
  print('DESIGN-COMMAND',label,json.dumps(command),flush=True)
  with (out/('n-native-'+label+'.stdout')).open('w') as stdout,(out/('n-native-'+label+'.stderr')).open('w') as stderr:
   result=subprocess.run(command,stdout=stdout,stderr=stderr,timeout=600)
  text=(out/('n-native-'+label+'.stdout')).read_text()+(out/('n-native-'+label+'.stderr')).read_text()
  print('DESIGN-RESULT',label,'status='+str(result.returncode),flush=True)
  print('NATIVE-RAW-BEGIN '+label+'\n'+text+'\nNATIVE-RAW-END '+label,flush=True)
  assert result.returncode==0,(label,text[-12000:])
  matches=re.findall(r'api_path_bytes=(\d+) guest_bytes=(\d+)',text)
  assert matches and all(int(size)<108 and int(memory)==128*1024*1024 for size,memory in matches)
  results.append({'context':label,'status':result.returncode,'api_path_bytes':int(matches[-1][0]),'guest_bytes':int(matches[-1][1]),'original_identity_and_seccomp_assertions_unchanged':True})
 os.environ['PROPTEST_CASES']='7';os.environ['PROPTEST_RNG_SEED']='123456';os.environ['RUST_BACKTRACE']='1'
 config=repo/'spike-scratch/metal-run-isolation/increment-g/env-nextest.toml'
 config.write_text('experimental = ["wrapper-scripts"]\n[scripts.wrapper.metal-design]\ncommand = '+json.dumps(['python3',str(repo/'spike-scratch/metal-run-isolation/increment-g/local_netns_exec.py'),str(repo/'target/debug/metal-isolation-local-vsock-spike')])+'\n[[profile.default.scripts]]\nfilter = "all()"\nrun-wrapper = "metal-design"\n[profile.default]\nslow-timeout = { period = "60s", terminate-after = 3 }\n')
 envcmd=['cargo','nextest','run','--manifest-path',str(repo/'spike-scratch/metal-run-isolation/increment-g/Cargo.toml'),'--target-dir',str(repo/'target'),'--config-file',str(config),'-E','test(selected_guest_environment_is_preserved)','--no-capture']
 with (out/'n-guest-env.stdout').open('w') as stdout,(out/'n-guest-env.stderr').open('w') as stderr:
  environment_result=subprocess.run(envcmd,stdout=stdout,stderr=stderr,timeout=600)
 print('ENV-RAW-BEGIN\n'+(out/'n-guest-env.stdout').read_text()+(out/'n-guest-env.stderr').read_text()+'\nENV-RAW-END',flush=True)
 assert environment_result.returncode==0,'selected guest must see the original property/attempt environment'
 record={'environment_preserved':True,'kernel':os.uname().release,'run_id':run_id,'staging_root':str(root),'only_fixture_input_fix':'run parent group 4200, mode 0710 before production VMM create','cases':results}
 (out/'n-native-result.json').write_text(json.dumps(record,indent=2)+'\n')
 print('DESIGN-NATIVE-PARENT-CONTRAST',json.dumps(record,sort_keys=True),flush=True)
finally:
 shutil.rmtree(root)
 if made_parent:
  try:parent.rmdir()
  except OSError:pass

# Re-run the completed cancellation contrast with full raw output in durable stdout.
cleanup=subprocess.run(['python3','spike-scratch/metal-run-isolation/increment-h/driver.py'],timeout=600)
assert cleanup.returncode==0
for label in ['without-cleanup','with-cleanup']:
 print('CLEANUP-RAW-BEGIN '+label,flush=True)
 print((out/('h-'+label+'.stdout')).read_text()+(out/('h-'+label+'.stderr')).read_text(),flush=True)
 print('CLEANUP-RAW-END '+label,flush=True)

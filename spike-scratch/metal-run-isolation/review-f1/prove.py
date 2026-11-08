"""One paired current test and one paired current DST entrypoint; no source or schedule seam."""
import json,os,subprocess,tempfile,time
from pathlib import Path
repo=Path.cwd();out=repo/'docs/feature/metal-run-isolation/design/f1-evidence';out.mkdir(parents=True,exist_ok=True)
os.environ['CARGO_TARGET_DIR']=str(repo/'target')
for key in list(os.environ):
 if key.startswith('CARGO_TARGET_') and key.endswith('_RUNNER'):os.environ.pop(key)
meta_cmd=['cargo','nextest','list','-p','overdrive-sim','--test','acceptance','--features','integration-tests','--message-format','json']
with (out/'list.json').open('w') as stdout,(out/'list.stderr').open('w') as stderr:
 result=subprocess.run(meta_cmd,stdout=stdout,stderr=stderr,timeout=300)
assert result.returncode==0,(out/'list.stderr').read_text()[-12000:]
meta=json.loads((out/'list.json').read_text());suite=meta['rust-suites']['overdrive-sim::acceptance'];binary=suite['binary-path']
identity='acceptance::gateway_relay_exchange::seeded_peer_events_never_move_an_octet_past_a_message_boundary_and_leave_no_residue'
assert identity in suite['testcases']

def pair(label,commands,envs=None):
 processes=[]
 for index,command in enumerate(commands):
  stdout=(out/f'{label}-{index}.stdout').open('w');stderr=(out/f'{label}-{index}.stderr').open('w')
  process=subprocess.Popen(command,stdout=stdout,stderr=stderr,env=None if envs is None else envs[index])
  processes.append((process,stdout,stderr))
 rows=[]
 for index,(process,stdout,stderr) in enumerate(processes):
  status=process.wait(timeout=120);stdout.close();stderr.close()
  text=(out/f'{label}-{index}.stdout').read_text()+(out/f'{label}-{index}.stderr').read_text()
  rows.append({'status':status,'addr_in_use':('AddrInUse' in text or 'Address already in use' in text),'pid':process.pid})
  print(f'RAW {label} {index}\n{text}',flush=True)
 observed=any(row['addr_in_use'] and row['status']!=0 for row in rows)
 print('PAIR-RESULT',label,json.dumps(rows),flush=True)
 return {'label':label,'rows':rows,'collision_reproduced':observed}

initial_ns=os.readlink('/proc/self/ns/net')
tests=pair('actual-test',[[binary,'--exact',identity,'--nocapture','--test-threads=1']]*2)
with (out/'dst-build.stdout').open('w') as stdout,(out/'dst-build.stderr').open('w') as stderr:
 result=subprocess.run(['cargo','build','-p','overdrive-sim','--bin','dst','--features','integration-tests'],stdout=stdout,stderr=stderr,timeout=300)
assert result.returncode==0,(out/'dst-build.stderr').read_text()[-12000:]
with tempfile.TemporaryDirectory(prefix='metal-f1-dst-') as directory:
 envs=[]
 for index in range(2):
  env=dict(os.environ);env['CARGO_TARGET_DIR']=str(Path(directory)/str(index));envs.append(env)
 dst=pair('actual-dst',[[str(repo/'target/debug/dst'),'--seed','0','--only','gateway-application-generation-lifecycle-is-safe']]*2,envs)
record={'kernel':os.uname().release,'namespace':initial_ns,'same_initial_namespace':True,'test_entry':identity,'test':tests,'dst':dst,'scheduling_seam_used':False,'production_source_modified':False}
(out/'result.json').write_text(json.dumps(record,indent=2)+'\n');print('F1-REACHABILITY',json.dumps(record,sort_keys=True),flush=True)

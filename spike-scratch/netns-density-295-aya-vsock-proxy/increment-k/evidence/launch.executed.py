#!/usr/bin/env python3
"""Canonical exclusive metal runner; only target addresses are redacted."""
import hashlib,io,json,os,pathlib,subprocess,sys,tarfile,time
ROOT=pathlib.Path(__file__).resolve().parents[2];SCRATCH=pathlib.Path(__file__).parent
attempt=sys.argv[1];assert pathlib.Path(attempt).name==attempt and attempt.startswith('increment-')
inc=SCRATCH/attempt;evidence=inc/'evidence';evidence.mkdir(exist_ok=True)
(evidence/'launch.executed.py').write_bytes(pathlib.Path(__file__).read_bytes())
(evidence/'rsync-filter.executed.sh').write_bytes((SCRATCH/'rsync-owned-filter.sh').read_bytes())
env=os.environ.copy()
for entry in (ROOT/'.env').read_text().splitlines():
 if '=' in entry and entry.startswith('OVERDRIVE_METAL_'):
  key,value=entry.split('=',1);env.setdefault(key,value.strip().strip('\"').strip("'"))
target=env['OVERDRIVE_METAL_TARGET'];address=target.split('@')[-1]
env['RSYNC_BIN']=str(SCRATCH/'rsync-owned-filter.sh')
env['OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS']='1';env['OVERDRIVE_METAL_SCENARIO']='spike-aya-vsock-proxy-'+attempt
manifest={'head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),'command':'cargo xtask metal run -- python3 spike-scratch/netns-density-295-aya-vsock-proxy/'+attempt+'/run.py','sources':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in inc.rglob('*') if p.is_file() and 'evidence' not in p.parts}}
(evidence/'source-manifest.json').write_text(json.dumps(manifest,indent=2,sort_keys=True));start=time.perf_counter()
with (evidence/'native-launch.log').open('x') as output:
 proc=subprocess.Popen(['cargo','xtask','metal','run','--','python3',str((inc/'run.py').relative_to(ROOT))],cwd=ROOT,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
 for line in proc.stdout:
  line=line.replace(target,'<redacted canonical metal target>').replace(address,'<redacted metal address>');output.write(line);output.flush();print(line,end='',flush=True)
 rc=proc.wait();output.write(json.dumps({'launcher_rc':rc,'wall_s':time.perf_counter()-start})+'\n')
result=subprocess.run(['ssh','-o','BatchMode=yes',target,'sudo -n tar -C overdrive/spike-scratch/netns-density-295-aya-vsock-proxy/'+attempt+' -czf - evidence'],capture_output=True)
if result.returncode==0:
 private=SCRATCH/'out'/(attempt+'-private.tar.gz');private.parent.mkdir(exist_ok=True);private.write_bytes(result.stdout)
 with tarfile.open(fileobj=io.BytesIO(result.stdout),mode='r:gz') as archive:
  for entry in archive:
   if not entry.isfile():continue
   rel=pathlib.PurePosixPath(entry.name).relative_to('evidence');assert '..' not in rel.parts
   if rel.name in ('native-launch.log','source-manifest.json','launch.executed.py'):continue
   content=archive.extractfile(entry).read()
   if rel.name.endswith('.gz'):
    import gzip
    content=gzip.compress(gzip.decompress(content).replace(address.encode(),b'<redacted metal address>'),mtime=0)
   else:content=content.replace(address.encode(),b'<redacted metal address>')
   p=evidence/pathlib.Path(str(rel));p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(content)
 print(json.dumps({'retrieved':True,'private_raw_archive_sha256':hashlib.sha256(result.stdout).hexdigest(),'launcher_rc':rc,'wall_s':time.perf_counter()-start}),flush=True)
else:print(json.dumps({'retrieved':False,'rc':result.returncode,'stderr':result.stderr.decode().replace(address,'<address>')}))
sys.exit(rc)

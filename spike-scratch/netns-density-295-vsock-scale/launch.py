#!/usr/bin/env python3
"""Exclusive canonical metal runner, target-redacted retained evidence."""
import hashlib, io, json, os, pathlib, subprocess, sys, tarfile, time
ROOT=pathlib.Path(__file__).resolve().parents[2]
SCRATCH=pathlib.Path(__file__).parent
attempt=sys.argv[1]
assert pathlib.Path(attempt).name==attempt and attempt.startswith('increment-')
inc=SCRATCH/attempt
evidence=inc/'evidence';evidence.mkdir(exist_ok=True)
env=os.environ.copy()
for entry in (ROOT/'.env').read_text().splitlines():
    if '=' in entry and not entry.lstrip().startswith('#'):
        key,value=entry.split('=',1)
        if key.startswith('OVERDRIVE_METAL_'):env.setdefault(key,value.strip().strip('\"').strip("'"))
target=env['OVERDRIVE_METAL_TARGET'];address=target.split('@')[-1]
env['OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS']='1';env['OVERDRIVE_METAL_SCENARIO']='spike-vsock-'+attempt
manifest={'head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),'command':'cargo xtask metal run -- python3 spike-scratch/netns-density-295-vsock-scale/'+attempt+'/run.py','sources':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in inc.rglob('*') if p.is_file() and 'evidence' not in p.parts}}
(evidence/'source-manifest.json').write_text(json.dumps(manifest,indent=2,sort_keys=True))
start=time.perf_counter();log=evidence/'native-launch.log'
with log.open('x') as output:
    proc=subprocess.Popen(['cargo','xtask','metal','run','--','python3',str((inc/'run.py').relative_to(ROOT))],cwd=ROOT,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
    for line in proc.stdout:
        line=line.replace(target,'<redacted canonical metal target>').replace(address,'<redacted metal address>')
        output.write(line);output.flush();print(line,end='',flush=True)
    rc=proc.wait();output.write(json.dumps({'launcher_rc':rc,'wall_s':time.perf_counter()-start})+'\n')
print(json.dumps({'launcher_rc':rc,'wall_s':time.perf_counter()-start}),flush=True)
# Retrieval is read-only after canonical runner releases its lease. Private exact bytes
# are retained under ignored out; shared text redacts only configured target address.
remote='sudo -n tar -C overdrive/spike-scratch/netns-density-295-vsock-scale/out -czf - '+attempt
result=subprocess.run(['ssh','-o','BatchMode=yes',target,remote],capture_output=True)
if result.returncode==0:
    private=SCRATCH/'out'/(attempt+'-private.tar.gz');private.parent.mkdir(exist_ok=True);private.write_bytes(result.stdout)
    with tarfile.open(fileobj=io.BytesIO(result.stdout),mode='r:gz') as archive:
        for entry in archive:
            if not entry.isfile():continue
            rel=pathlib.PurePosixPath(entry.name).relative_to(attempt)
            if '..' in rel.parts:raise RuntimeError('unsafe archive')
            data=archive.extractfile(entry).read()
            if rel.suffix not in ('.pcap','.gz'):data=data.replace(address.encode(),b'<redacted metal address>')
            dest=evidence/str(rel);dest.parent.mkdir(parents=True,exist_ok=True)
            if dest.exists():raise RuntimeError('evidence overwrite refused: '+str(dest))
            dest.write_bytes(data)
    (evidence/'retrieval.json').write_text(json.dumps({'private_archive_sha256':hashlib.sha256(result.stdout).hexdigest(),'private_archive_bytes':len(result.stdout),'redaction':'configured metal address in shared text only; exact private archive retained under ignored out'},indent=2))
else:print('evidence retrieval error '+result.stderr.decode().replace(address,'<redacted metal address>'))
sys.exit(rc)

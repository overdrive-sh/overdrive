#!/usr/bin/env python3
"""Task-owned canonical metal launcher; target is redacted from shared receipts."""
import base64, hashlib, io, json, os, pathlib, subprocess, sys, tarfile, time
ROOT=pathlib.Path(__file__).resolve().parents[2]
SCRATCH=pathlib.Path(__file__).parent
env=os.environ.copy()
for line in (ROOT/'.env').read_text().splitlines():
    if '=' in line and not line.lstrip().startswith('#'):
        k,v=line.split('=',1)
        if k.startswith('OVERDRIVE_METAL_'): env.setdefault(k,v.strip().strip('\"').strip("'"))
target=env['OVERDRIVE_METAL_TARGET']
target_address=target.split('@')[-1]
env['OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS']='1'
attempt=os.environ.get('SPIKE_MULTI_BRIDGE_ATTEMPT','02')
env['OVERDRIVE_METAL_SCENARIO']='spike-multi-bridge-primitive-'+attempt
changed=subprocess.check_output(['git','diff','--name-only','HEAD'],cwd=ROOT,text=True).splitlines()
untracked=subprocess.check_output(['git','ls-files','--others','--exclude-standard'],cwd=ROOT,text=True).splitlines()
manifest={'head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
          'files':{f:hashlib.sha256((ROOT/f).read_bytes()).hexdigest() for f in changed+untracked if (ROOT/f).is_file()},
          'probe_sha256':hashlib.sha256((SCRATCH/'probe.py').read_bytes()).hexdigest(),
          'command':'cargo xtask metal run -- python3 spike-scratch/netns-density-295-multi-bridge/probe.py <profile>',
          'target':'<redacted canonical metal target>'}
(ROOT/('.context/spike-multi-bridge-source-manifest-'+attempt+'.json')).write_text(json.dumps(manifest,indent=2,sort_keys=True))
mode=os.environ.get('SPIKE_MULTI_BRIDGE_MODE','primitive')
log=ROOT/('.context/spike-multi-bridge-native-'+attempt+'.log')
start=time.perf_counter()
with log.open('x') as output:
    proc=subprocess.Popen(['cargo','xtask','metal','run','--','python3','spike-scratch/netns-density-295-multi-bridge/probe.py',mode],cwd=ROOT,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
    for line in proc.stdout:
        line=line.replace(target,'<redacted canonical metal target>').replace(target_address,'<redacted metal address>')
        output.write(line);output.flush()
        if not line.lstrip().startswith('Running `'): print(line,end='',flush=True)
    rc=proc.wait()
    output.write(json.dumps({'launcher_rc':rc,'wall_s':time.perf_counter()-start})+'\n')
print(json.dumps({'launcher_rc':rc,'wall_s':time.perf_counter()-start,'receipt':str(log)}),flush=True)
# Read-only evidence retrieval after the canonical launcher has released its lease.
cmd='sudo -n tar -C overdrive/spike-scratch -czf - netns-density-295-multi-bridge'
data=subprocess.run(['ssh','-o','BatchMode=yes',target,cmd],capture_output=True)
if data.returncode==0:
    private=SCRATCH/'out'/('evidence-private-native-'+attempt+'.tar.gz'); private.parent.mkdir(parents=True,exist_ok=True); private.write_bytes(data.stdout)
    sanitized=io.BytesIO()
    with tarfile.open(fileobj=io.BytesIO(data.stdout),mode='r:gz') as incoming,tarfile.open(fileobj=sanitized,mode='w:gz') as outgoing:
        for member in incoming:
            content=incoming.extractfile(member).read() if member.isfile() else None
            if content is not None:
                content=content.replace(target_address.encode(),b'<redacted metal address>');member.size=len(content)
            outgoing.addfile(member,io.BytesIO(content) if content is not None else None)
    archive=ROOT/('.context/spike-multi-bridge-native-'+attempt+'.tar.gz');archive.write_bytes(sanitized.getvalue())
    print(json.dumps({'archive':str(archive),'bytes':len(sanitized.getvalue()),'sha256':hashlib.sha256(sanitized.getvalue()).hexdigest(),'private_raw_sha256':hashlib.sha256(data.stdout).hexdigest(),'redaction':'exact metal target address only; original retained in task-owned scratch'}),flush=True)
else: print('read-only evidence retrieval failed: '+data.stderr.decode().replace(target,'<redacted canonical metal target>'),flush=True)
sys.exit(rc)

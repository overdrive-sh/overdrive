import datetime, hashlib, json, os, pathlib, subprocess, sys
root=pathlib.Path('/Users/marcus/conductor/workspaces/helios/ouagadougou')
peer=pathlib.Path('/tmp/metal-run-isolation-peer.UKMMtE6B')
base=pathlib.Path('/tmp/metal-feature-closeout-20261109T104319Z')
environment=os.environ.copy()
for raw in (root/'.env').read_text().splitlines():
    line=raw.strip()
    if line.startswith('export '): line=line[7:].strip()
    if not line or line.startswith('#') or '=' not in line: continue
    key,value=line.split('=',1); key=key.strip(); value=value.strip()
    if key.startswith('OVERDRIVE_METAL_'):
        if len(value)>=2 and value[0]==value[-1] and value[0] in ('"',"'"): value=value[1:-1]
        environment.setdefault(key,value)
environment.setdefault('OVERDRIVE_METAL_KERNEL','/srv/vm/overdrive-testing/kernel')
environment.setdefault('OVERDRIVE_METAL_ROOTFS','/srv/vm/overdrive-testing/rootfs.ext4')
def now(): return datetime.datetime.now(datetime.timezone.utc).isoformat()
def source(checkout):
    files=['infra/metal/lease-holder.sh','infra/metal/bootstrap.sh','infra/metal/test-wrapper.sh','infra/metal/tests/worktree_journey.sh','infra/metal/tests/admission_journey.sh','infra/metal/tests/placement_journey.sh','infra/metal/tests/observe_host.py','crates/overdrive-testing/src/guest_lane.rs','crates/overdrive-testing/bin/fwd_guest_suite.rs']
    return {'canonical_checkout':str(checkout.resolve()),'head':subprocess.check_output(['git','-C',str(checkout),'rev-parse','HEAD'],text=True).strip(),'status':subprocess.check_output(['git','-C',str(checkout),'status','--porcelain=v1'],text=True),'source_sha256':{p:hashlib.sha256((checkout/p).read_bytes()).hexdigest() for p in files}}
def capture(label, command, cwd=root, output_arg=None, input_bytes=None):
    out=base/label
    if output_arg:
        assert not out.exists()
        log=base/(label+'.foreground.log'); receipt=base/(label+'.invocation.json')
    else:
        out.mkdir()
        log=out/'foreground.log'; receipt=out/'foreground-invocation.json'
    data={'command':command,'cwd':str(cwd),'started_at':now(),'sources':{'root':source(root),'peer':source(peer)},'configured_inputs':{k:environment[k] for k in ['OVERDRIVE_METAL_TARGET','OVERDRIVE_METAL_KERNEL','OVERDRIVE_METAL_ROOTFS']}}
    if input_bytes is not None: data['stdin_sha256']=hashlib.sha256(input_bytes).hexdigest()
    receipt.write_text(json.dumps(data,indent=2)+'\n')
    print('CAPTURE '+label+' '+str(out),flush=True)
    with log.open('wb') as stream:
        process=subprocess.Popen(command,cwd=cwd,env=environment,stdin=subprocess.PIPE if input_bytes is not None else None,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
        if input_bytes is not None:
            process.stdin.write(input_bytes); process.stdin.close()
        for line in process.stdout:
            stream.write(line); stream.flush(); sys.stdout.buffer.write(line); sys.stdout.buffer.flush()
        status=process.wait()
    data.update(finished_at=now(),exit_code=status,sources_after={'root':source(root),'peer':source(peer)})
    receipt.write_text(json.dumps(data,indent=2)+'\n')
    if output_arg and out.is_dir():
        log.rename(out/'foreground.log'); receipt.rename(out/'foreground-invocation.json')
    print('RESULT '+label+' '+str(status),flush=True)
    return status
metadata=b"""import json,os,platform,subprocess\nfrom pathlib import Path\ncommands={'uname':['uname','-a'],'bash':['bash','--version'],'systemd':['systemctl','--version'],'flock':['flock','--version'],'ip':['ip','-Version'],'cargo':[str(Path.home()/'.cargo/bin/cargo'),'--version'],'nextest':[str(Path.home()/'.cargo/bin/cargo'),'nextest','--version']}\nresult={'kernel':dict(platform.uname()._asdict()),'boot_id':Path('/proc/sys/kernel/random/boot_id').read_text(),'tools':{}}\nfor name,argv in commands.items():\n try:\n  p=subprocess.run(argv,capture_output=True,text=True);result['tools'][name]={'command':argv,'status':p.returncode,'stdout':p.stdout,'stderr':p.stderr}\n except OSError as error:result['tools'][name]={'command':argv,'query_error':str(error)}\nprint(json.dumps(result,indent=2))\n"""
status=capture('substrate',['ssh',environment['OVERDRIVE_METAL_TARGET'],'python3','-'],input_bytes=metadata)
if status: sys.exit(status)
for label,cwd in [('prepare-root',root),('prepare-peer',peer)]:
    status=capture(label,['cargo','xtask','metal','run','--','cargo','xtask','guest-kit-prepare'],cwd)
    if status: sys.exit(status)
for label,command in [
    ('worktree',['bash',str(root/'infra/metal/tests/worktree_journey.sh'),str(root),str(peer),str(base/'worktree')]),
    ('admission',['bash',str(root/'infra/metal/tests/admission_journey.sh'),str(root),str(base/'admission')]),
    ('placement',['bash',str(base/'placement-with-baseline-exclusion.sh'),str(root),str(base/'placement')])]:
    status=capture(label,command,output_arg=True)
    if status: sys.exit(status)
assert (root/'infra/metal/tests/placement_journey.sh').read_bytes()==(base/'placement-original.sh').read_bytes()
print('QUALIFIED_FEATURE_CLOSEOUT workflows=worktree,admission,placement all_retained_assertions_passed=true excluded_only=guest_flow::real_kernel::a_burst_of_host_aborts_resets_every_application_connection',flush=True)

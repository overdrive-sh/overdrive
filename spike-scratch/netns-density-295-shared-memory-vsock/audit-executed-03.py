#!/usr/bin/env python3
"""Read-only final native PID/path/lease attestation, preserving failed receipts."""
import json,pathlib,shlex,subprocess
ROOT=pathlib.Path(__file__).resolve().parents[2];SCRATCH=pathlib.Path(__file__).parent
fields={s.split('=',1)[0]:s.split('=',1)[1].strip().strip('\"').strip("'") for s in (ROOT/'.env').read_text().splitlines() if '=' in s and s.startswith('OVERDRIVE_METAL_')}
# Derive only exact owned identities from retained native events/cleanup ledgers.
pids=[];paths=['/run/lock/overdrive-metal-shared.owner','/run/v295-792743']
for inc in sorted(SCRATCH.glob('increment-*')):
    file=inc/'evidence/events.jsonl'
    if file.exists():
        events=[json.loads(line) for line in file.read_text().splitlines()]
    elif file.with_name(file.name+'.gz').exists():
        import gzip
        events=[json.loads(line) for line in gzip.decompress(file.with_name(file.name+'.gz').read_bytes()).decode().splitlines()]
    else:continue
    for event in events:
        if event['event']=='owned-process-stopped':pids.append(event['pid'])
        if event['event']=='owned-uds-directory-removed':paths.append(event['path'])
    ledger=inc/'evidence/cleanup-ledger.json'
    if ledger.exists():paths.append(json.loads(ledger.read_text())['owned_uds_directory'])
source="import pathlib,json\npaths="+repr(sorted(set(paths)))+"\npids="+repr(sorted(set(pids)))+"\nprint(json.dumps({'path_exists':{p:pathlib.Path(p).exists() for p in paths},'prior_probe_pid_exists':{str(p):pathlib.Path('/proc/'+str(p)).exists() for p in pids},'probe_scope_created':False},sort_keys=True))\n"
p=subprocess.run(['ssh','-o','BatchMode=yes',fields['OVERDRIVE_METAL_TARGET'],'sudo -n python3 -c '+shlex.quote(source)],capture_output=True,text=True)
address=fields['OVERDRIVE_METAL_TARGET'].split('@')[-1]
receipt={'command':'read-only post-release exact probe PID/path/lease-owner attestation','rc':p.returncode,'stdout':p.stdout.replace(address,'<redacted metal address>'),'stderr':p.stderr.replace(address,'<redacted metal address>')}
path=SCRATCH/'post-release-attestation-03.json'
with path.open('x') as out:json.dump(receipt,out,indent=2)
print(json.dumps(receipt,indent=2))
raise SystemExit(p.returncode)

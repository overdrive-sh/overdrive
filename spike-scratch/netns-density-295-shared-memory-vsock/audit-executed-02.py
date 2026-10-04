#!/usr/bin/env python3
"""Read-only final native PID/path/lease attestation, preserving failed receipts."""
import json,pathlib,shlex,subprocess
ROOT=pathlib.Path(__file__).resolve().parents[2];SCRATCH=pathlib.Path(__file__).parent
fields={s.split('=',1)[0]:s.split('=',1)[1].strip().strip('\"').strip("'") for s in (ROOT/'.env').read_text().splitlines() if '=' in s and s.startswith('OVERDRIVE_METAL_')}
source='''import pathlib,json
paths=['/run/v295-792743','/run/v295-794479','/run/v295-795377','/run/v295-796422','/run/lock/overdrive-metal-shared.owner']
pids=[792808,792811,792816,792828,794543,794546,794551,794563,795567,795570,795575,795587,796488,796491,796496,796508]
print(json.dumps({'path_exists':{p:pathlib.Path(p).exists() for p in paths},'prior_probe_pid_exists':{str(p):pathlib.Path('/proc/'+str(p)).exists() for p in pids},'probe_scope_created':False},sort_keys=True))
'''
p=subprocess.run(['ssh','-o','BatchMode=yes',fields['OVERDRIVE_METAL_TARGET'],'sudo -n python3 -c '+shlex.quote(source)],capture_output=True,text=True)
address=fields['OVERDRIVE_METAL_TARGET'].split('@')[-1]
receipt={'command':'read-only post-release exact probe PID/path/lease-owner attestation','rc':p.returncode,'stdout':p.stdout.replace(address,'<redacted metal address>'),'stderr':p.stderr.replace(address,'<redacted metal address>')}
path=SCRATCH/'post-release-attestation-02.json'
with path.open('x') as out:json.dump(receipt,out,indent=2)
print(json.dumps(receipt,indent=2))
raise SystemExit(p.returncode)

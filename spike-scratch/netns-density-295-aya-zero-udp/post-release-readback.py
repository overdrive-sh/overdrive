#!/usr/bin/env python3
"""Read-only completion audit, after the canonical launcher released its lease."""
import json,os,pathlib,shlex,subprocess
ROOT=pathlib.Path(__file__).resolve().parents[2]
SCRATCH=pathlib.Path(__file__).parent
env=os.environ.copy()
for entry in (ROOT/'.env').read_text().splitlines():
 if '=' in entry and entry.startswith('OVERDRIVE_METAL_'):
  key,value=entry.split('=',1);env.setdefault(key,value.strip().strip('\"').strip("'"))
remote='''import pathlib,json,os
pids=[]
for p in pathlib.Path('/proc').iterdir():
 if not p.name.isdigit():continue
 try:
  words=(p/'cmdline').read_bytes().split(b'\\0')
  if words and words[0].rsplit(b'/',1)[-1].startswith(b'qemu-system-') and any(b'netns-density-295-aya-zero-udp' in w for w in words):pids.append(int(p.name))
 except (FileNotFoundError,PermissionError,ProcessLookupError):pass
print(json.dumps({'kernel':pathlib.Path('/proc/sys/kernel/osrelease').read_text().strip(),'owned_qemu_pids':pids,'canonical_owner_exists':pathlib.Path('/run/lock/overdrive-metal-shared.owner').exists(),'physical_links':sorted(p.name for p in pathlib.Path('/sys/class/net').iterdir()),'physical_modules':pathlib.Path('/proc/modules').read_text(),'clock_ticks':os.sysconf('SC_CLK_TCK'),'checked_after_final_increment':'increment-f'}))
'''
result=subprocess.run(['ssh','-o','BatchMode=yes',env['OVERDRIVE_METAL_TARGET'],'sudo -n python3 -c '+shlex.quote(remote)],capture_output=True,text=True,check=True)
value=json.loads(result.stdout)
assert not value['owned_qemu_pids'] and not value['canonical_owner_exists']
before=json.loads((SCRATCH/'increment-f/evidence/physical-before.json').read_text())
value['physical_modules_equal_before']=value['physical_modules']==before['modules']
value['physical_links_equal_before']=value['physical_links']==before['links']
assert value['physical_modules_equal_before'] and value['physical_links_equal_before']
(SCRATCH/'post-release-readback.json').write_text(json.dumps(value,indent=2,sort_keys=True))
print(json.dumps({k:v for k,v in value.items() if k!='physical_modules'}))

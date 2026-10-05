#!/usr/bin/env python3
"""Read-only final owned-resource and physical-inventory audit."""
import pathlib,os,subprocess,json,hashlib,time
BASE=pathlib.Path(__file__).resolve().parent;ROOT=BASE.parents[1];env=os.environ.copy()
for line in(ROOT/'.env').read_text().splitlines():
 if line.startswith('OVERDRIVE_METAL_')and'='in line:
  k,v=line.split('=',1);env.setdefault(k,v.strip().strip('"').strip("'"))
script='''import pathlib,json,time
owned=[]
for p in pathlib.Path('/proc').iterdir():
 if not p.name.isdigit():continue
 try:
  argv=[x.decode()for x in(p/'cmdline').read_bytes().split(b'\\0')if x]
  if argv and argv[0].endswith('qemu-system-x86_64')and any('kernel-vs-aya-benchmark' in x for x in argv):owned.append({'pid':int(p.name),'argv':argv})
 except(FileNotFoundError,PermissionError,UnicodeDecodeError):pass
print(json.dumps({'timestamp_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'owned_qemu':owned,'canonical_owner_exists':pathlib.Path('/run/lock/overdrive-metal-shared.owner').exists(),'modules':pathlib.Path('/proc/modules').read_text(),'links':sorted(p.name for p in pathlib.Path('/sys/class/net').iterdir())}))
'''
r=subprocess.run(['ssh','-o','BatchMode=yes',env['OVERDRIVE_METAL_TARGET'],'sudo -n python3 -'],input=script,text=True,capture_output=True,check=True);v=json.loads(r.stdout);before=json.loads((BASE/'increment-l/evidence/physical-before.json').read_text());v['physical_modules_equal_before']=v['modules']==before['modules'];v['physical_links_equal_before']=v['links']==before['links'];(BASE/'post-release-audit.json').write_text(json.dumps(v,indent=2));assert not v['owned_qemu']and not v['canonical_owner_exists'];assert v['physical_modules_equal_before']and v['physical_links_equal_before'];print(json.dumps({k:x for k,x in v.items()if k!='modules'}))

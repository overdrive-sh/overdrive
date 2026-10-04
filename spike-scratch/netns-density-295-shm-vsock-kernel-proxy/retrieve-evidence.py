#!/usr/bin/env python3
"""Retrieve immutable native evidence only; private binaries remain ignored."""
import hashlib,io,json,os,pathlib,subprocess,sys,tarfile,time
base=pathlib.Path(__file__).resolve().parent;root=base.parents[1]
attempt=sys.argv[1];assert pathlib.Path(attempt).name==attempt and attempt.startswith('increment-')
env=os.environ.copy()
for line in (root/'.env').read_text().splitlines():
 if line.startswith('OVERDRIVE_METAL_') and '=' in line:
  key,value=line.split('=',1);env.setdefault(key,value.strip().strip('\"').strip("'"))
target=env['OVERDRIVE_METAL_TARGET'];address=target.split('@')[-1]
command=f'sudo -n tar -C overdrive/spike-scratch/netns-density-295-shm-vsock-kernel-proxy/{attempt} -czf - evidence'
p=subprocess.run(['ssh','-o','BatchMode=yes',target,command],capture_output=True)
assert p.returncode==0,p.stderr.decode().replace(address,'<address>')
private=base/'out'/f'{attempt}-evidence-private.tar.gz';private.parent.mkdir(exist_ok=True);private.write_bytes(p.stdout)
with tarfile.open(fileobj=io.BytesIO(p.stdout),mode='r:gz') as archive:
 for entry in archive:
  if not entry.isfile():continue
  rel=pathlib.PurePosixPath(entry.name).relative_to('evidence');assert '..' not in rel.parts
  content=archive.extractfile(entry).read().replace(address.encode(),b'<canonical metal address>')
  path=base/attempt/'evidence'/str(rel)
  if path.exists():assert path.read_bytes()==content, f'immutable receipt differs: {rel}'
  else:path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(content)
receipt={'timestamp_epoch_s':time.time(),'attempt':attempt,'source':'canonical metal synchronized source increment/evidence','remote_tar_exit':p.returncode,'private_archive_sha256':hashlib.sha256(p.stdout).hexdigest(),'retrieved_evidence_files':[str(p.relative_to(base/attempt/'evidence')) for p in (base/attempt/'evidence').rglob('*') if p.is_file()]}
(base/attempt/'evidence/retrieval.json').write_text(json.dumps(receipt,indent=2));print(json.dumps(receipt))

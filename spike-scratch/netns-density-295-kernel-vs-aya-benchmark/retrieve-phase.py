#!/usr/bin/env python3
"""Read a completed private-VM phase without waiting for later phases or syncing."""
import hashlib,io,json,os,pathlib,subprocess,sys,tarfile,time
BASE=pathlib.Path(__file__).resolve().parent;ROOT=BASE.parents[1];attempt,phase=sys.argv[1:3]
assert attempt.startswith('increment-')and pathlib.Path(attempt).name==attempt
assert len(phase)<40 and all(c.isalnum()or c=='-'for c in phase)
env=os.environ.copy()
for line in(ROOT/'.env').read_text().splitlines():
 if line.startswith('OVERDRIVE_METAL_')and'='in line:
  k,v=line.split('=',1);env.setdefault(k,v.strip().strip('"').strip("'"))
target=env['OVERDRIVE_METAL_TARGET'];address=target.split('@')[-1];remote='overdrive/spike-scratch/netns-density-295-kernel-vs-aya-benchmark/'+attempt+'/evidence/'+phase
result=subprocess.run(['ssh','-o','BatchMode=yes',target,'sudo -n test -f '+remote+'/native-result.json && sudo -n tar -C '+remote+' -czf - .'],capture_output=True)
if result.returncode:print(json.dumps({'phase_complete':False,'phase':phase,'exit':result.returncode}));raise SystemExit(result.returncode)
private=BASE/'out'/(attempt+'-'+phase+'-phase.tar.gz');private.parent.mkdir(exist_ok=True);private.write_bytes(result.stdout)
evidence=BASE/attempt/'evidence'/phase;evidence.mkdir(exist_ok=True)
with tarfile.open(fileobj=io.BytesIO(result.stdout),mode='r:gz')as archive:
 for entry in archive:
  if not entry.isfile():continue
  relative=pathlib.PurePosixPath(entry.name);assert'..'not in relative.parts
  dest=evidence/str(relative);dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(archive.extractfile(entry).read().replace(address.encode(),b'<redacted metal address>'))
receipt={'retrieved_completed_phase':phase,'original_archive_sha256':hashlib.sha256(result.stdout).hexdigest(),'timestamp_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'native_result':json.loads((evidence/'native-result.json').read_text())};(evidence/'phase-retrieval-receipt.json').write_text(json.dumps(receipt,indent=2));print(json.dumps(receipt))

#!/usr/bin/env python3
"""Losslessly archive original native logs; retain byte hashes and local views."""
import gzip,hashlib,json,pathlib
BASE=pathlib.Path(__file__).resolve().parent;records=[]
for inc in sorted(BASE.glob('increment-*')):
 for p in (inc/'evidence').glob('**/*'):
  if not p.is_file()or p.name not in ['native-serial.log','qemu-resources.jsonl','event-observer.jsonl']:continue
  # A running phase is never archived as if complete.
  partial=(inc/'evidence/partial-retrieval-receipt.json').exists()
  if not(p.parent/'native-result.json').exists()and not partial:continue
  raw=p.read_bytes();dest=p.with_name(p.name+'.gz');dest.write_bytes(gzip.compress(raw,mtime=0));assert gzip.decompress(dest.read_bytes())==raw
  records.append({'scope':'partial environment-interrupted original'if partial and not(p.parent/'native-result.json').exists()else'completed native phase','original':str(p.relative_to(BASE)),'archive':str(dest.relative_to(BASE)),'original_sha256':hashlib.sha256(raw).hexdigest(),'archive_sha256':hashlib.sha256(dest.read_bytes()).hexdigest(),'original_bytes':len(raw),'archive_bytes':dest.stat().st_size})
(BASE/'compressed-evidence-index.json').write_text(json.dumps(records,indent=2));print(json.dumps({'complete_native_files_archived':len(records),'original_bytes':sum(x['original_bytes']for x in records),'archive_bytes':sum(x['archive_bytes']for x in records)}))

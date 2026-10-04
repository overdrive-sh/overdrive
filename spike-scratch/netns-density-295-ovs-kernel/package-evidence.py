"""Lossless receipt compression, retaining hashes of the original bytes."""
import pathlib,gzip,hashlib,json
ROOT=pathlib.Path(__file__).resolve().parent;records=[]
for evidence in ROOT.glob('increment-*/evidence'):
 for p in evidence.iterdir():
  if p.is_file() and p.suffix!='.gz' and p.stat().st_size>128*1024:
   b=p.read_bytes();compressed=gzip.compress(b,mtime=0);assert gzip.decompress(compressed)==b
   q=p.with_name(p.name+'.gz');q.write_bytes(compressed)
   records.append({'original_path':str(p.relative_to(ROOT)),'retained_path':str(q.relative_to(ROOT)),'original_bytes':len(b),'original_sha256':hashlib.sha256(b).hexdigest(),'gzip_bytes':len(compressed),'gzip_sha256':hashlib.sha256(compressed).hexdigest()});p.unlink()
(ROOT/'compression-manifest.json').write_text(json.dumps(records,indent=2,sort_keys=True))
print(json.dumps({'losslessly_compressed_receipts':len(records),'original_bytes':sum(x['original_bytes']for x in records),'retained_gzip_bytes':sum(x['gzip_bytes']for x in records)}))

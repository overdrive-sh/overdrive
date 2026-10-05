"""Read retained UART JSON; explicitly recover exact interleaved printk bytes.

Only known stock warning / module drain console messages are removable, only
when a strict JSON event is recovered. Original bytes are never changed.
"""
import gzip,hashlib,json,re,pathlib
PRINTK=re.compile(r'\[\s*\d+\.\d+\] (?:shmproxy: drained accepted=\d+ closed=\d+|clocksource: Long readout interval, skipping watchdog check: cs_nsec: \d+ wd_nsec: \d+)\n')
def read_events(path):
 path=pathlib.Path(path);data=path.read_text()if path.exists()else gzip.decompress(path.with_name(path.name+'.gz').read_bytes()).decode();lines=data.splitlines(keepends=True);events=[];repairs=[];invalid=[];i=0
 while i<len(lines):
  line=lines[i]
  if not line.startswith('{'):i+=1;continue
  try:events.append((i+1,json.loads(line)));i+=1;continue
  except json.JSONDecodeError:pass
  raw=line;j=i+1;recovered=None
  # An event may continue after printk's inserted newline, never across a new JSON event.
  while j<len(lines)and not lines[j].startswith('{')and j-i<8:
   raw+=lines[j];j+=1
   cleaned,n=PRINTK.subn('',raw)
   if not n:continue
   try:recovered=json.loads(cleaned)
   except json.JSONDecodeError:continue
   if not isinstance(recovered,dict)or'event'not in recovered:continue
   events.append((i+1,recovered));repairs.append({'first_line':i+1,'last_line':j,'event':recovered['event'],'id':recovered.get('id'),'removed_known_printk':[m.group(0)for m in PRINTK.finditer(raw)],'raw_span_sha256':hashlib.sha256(raw.encode()).hexdigest(),'recovered_event_sha256':hashlib.sha256(cleaned.encode()).hexdigest(),'recovery':'remove exact matched printk console insertion including inserted newline; retain every JSON character and strict-parse; no numeric value edits'});break
  if recovered is None:invalid.append({'line':i+1,'reason':'unrecoverable malformed UART JSON; excluded','line_sha256':hashlib.sha256(line.encode()).hexdigest()});i+=1
  else:i=j
 return events,repairs,invalid

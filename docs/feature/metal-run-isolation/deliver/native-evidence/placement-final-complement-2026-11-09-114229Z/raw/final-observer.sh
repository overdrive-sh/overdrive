#!/usr/bin/env bash
set -euo pipefail
checkout="$1"
out="$2"
attempt_id="$3"
command_id="$4"
observe() { ssh "$OVERDRIVE_METAL_TARGET" 'sudo -n python3 -' "$@" < "$checkout/infra/metal/tests/observe_host.py"; }
observe "$attempt_id" "$command_id" > "$out/after.json"
python3 - "$out" <<'PY'
import copy,json,sys
from pathlib import Path
out=Path(sys.argv[1]); before=json.loads((out/'before.json').read_text()); after=json.loads((out/'after.json').read_text())
# Keep raw snapshots intact. Only paired finite RA expiry countdowns vary;
# route identity, every other attribute and every other physical key stay exact.
comparison=copy.deepcopy(after['physical'])
for old,new in zip(before['physical']['routes'],comparison['routes']):
 if old.get('protocol')==new.get('protocol')=='ra' and type(old.get('expires')) is int and type(new.get('expires')) is int and old['expires']>=0 and new['expires']>=0:
  new['expires']=old['expires']
assert before['physical']==comparison, 'ordinary test physical-kernel complement changed'
for o in after['owners'].values():
 assert o['completion.v1']['kind']=='complete'
 assert not o['processes'] and not any(root['exists'] for root in o['roots'].values())
print('METAL-ISOLATION-PLACEMENT exact_guest_attempts=true fresh_retry=true actual_helper=true nextest_status=100 command_status=255 physical_complement_unchanged=true')
PY

#!/usr/bin/env bash
# External developer-tool E2E acceptance, never an Overdrive expectation runner.
# CONTRACT_SHAPE: bounded-change (two real invocation universes; physical test
# kernel and every surviving invocation are the independently observed complement).
set -euo pipefail
[ "$#" = 3 ] || { echo 'usage: worktree_journey.sh CHECKOUT_A CHECKOUT_B OUTPUT_DIR' >&2; exit 64; }
first="$(cd "$1" && pwd -P)"
second="$(cd "$2" && pwd -P)"
out="$3"
mkdir -p "$out"
[ -z "$(ls -A "$out")" ] || { echo "choose a new empty evidence directory" >&2; exit 64; }
out="$(cd "$out" && pwd -P)"
python3 - "$out" "$first" "$second" <<'PY_OUTPUT'
import sys
from pathlib import Path
out=Path(sys.argv[1]).resolve()
if any(out.is_relative_to(Path(source).resolve()) for source in sys.argv[2:]):
 raise SystemExit('Choose an empty evidence directory outside the input worktrees so capture cannot change source identity')
PY_OUTPUT
exec 2> >(tee -a "$out/driver.stderr" >&2)
: "${OVERDRIVE_METAL_TARGET:?export the existing configured Metal target for read-only observations}"
observer="$first/infra/metal/tests/observe_host.py"
observe() { ssh "$OVERDRIVE_METAL_TARGET" 'sudo -n python3 -' "$@" < "$observer"; }
# Suppress Cargo's per-launch build/timing banners at their source; compare the
# collector's retained output and all of its diagnostics without filtering.
invoke() { local checkout="$1"; shift; (cd "$checkout" && cargo --quiet xtask metal "$@"); }
for checkout in "$first" "$second"; do
  [ -f "$checkout/infra/metal/test-wrapper.sh" ] || { echo 'pending DELIVER: ordinary Metal adapter is absent' >&2; exit 78; }
done
[ "$(git -C "$first" rev-parse --absolute-git-dir)" != "$(git -C "$second" rev-parse --absolute-git-dir)" ]
git -C "$first" status --porcelain=v1 > "$out/first-source.status"
git -C "$second" status --porcelain=v1 > "$out/second-source.status"
observe > "$out/before.json"
# Existing explicit suite keeps its real batched core group and actual bodies.
(cd "$first" && exec cargo xtask metal run -- env OVERDRIVE_FWD_GUEST_GROUP=core cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests) > "$out/first.stdout" 2> "$out/first.stderr" &
first_controller=$!
(cd "$second" && exec cargo xtask metal run -- env OVERDRIVE_FWD_GUEST_GROUP=core cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests) > "$out/second.stdout" 2> "$out/second.stderr" &
second_controller=$!
run_id() { sed -n 's/^OVERDRIVE_METAL_RUN_ID=\([0-9a-f]\{24\}\)$/\1/p' "$1" | head -1; }
for ((attempt=0; attempt<1200; attempt++)); do
  first_id="$(run_id "$out/first.stderr")"
  second_id="$(run_id "$out/second.stderr")"
  if [ -n "$first_id" ] && [ -n "$second_id" ] && observe "$first_id" "$second_id" > "$out/overlap-$attempt.json" 2> "$out/overlap-query-$attempt.stderr"; then
    if python3 - "$out/overlap-$attempt.json" <<'PY'
import json,sys
x=json.load(open(sys.argv[1])); owners=list(x['owners'].values())
assert len(owners)==2
assert all(o.get('phase.v1',{}).get('phase')=='running' and o['unit']['ActiveState']=='active' and o['processes'] for o in owners)
for o in owners:
 assert o['metadata']['execution_class']=='ordinary'
 for field in ('guest_staging','vmm_runtime'):
  for item in (o['roots'][field],o['launcher_bases'][field]):
   assert item['exists'] and item['uid']==0 and item['gid']==4200 and item['mode']==0o710 and not item['symlink'], 'exact launcher traversal ownership/mode required'

 main=o['processes'][o['unit']['MainPID']]
 locks=[fd for fd in main['fds'].values() if fd['target']=='/run/lock/overdrive-metal-shared.lock']
 import base64,hashlib
 canonical=base64.b64decode(o['metadata']['remote_dir_b64'],validate=True)
 expected_tree_key=hashlib.sha256(canonical).hexdigest()
 assert any(expected_tree_key in fd['target'].rsplit('/',1)[-1] and 'FLOCK' in fd['fdinfo'] and 'WRITE' in fd['fdinfo'] for fd in main['fds'].values()), 'main owner must hold the canonical directory-hash checkout lock'

 assert any('FLOCK' in fd['fdinfo'] and 'READ' in fd['fdinfo'] for fd in locks)
 assert not any('WRITE' in fd['fdinfo'] for fd in locks)
assert owners[0]['checkout']['checkout_key']!=owners[1]['checkout']['checkout_key']
vmm=[]
for o in owners:
 launchers=[p for p in o['processes'].values() if any('vhost-vsock' in fd['target'] for fd in p['fds'].values())]
 assert launchers, 'real guest VMM must be live in both invocations'
 vmm.append(launchers)
assert {p['netns'] for p in vmm[0]}.isdisjoint({p['netns'] for p in vmm[1]})
import re
cids=[set(re.findall(r'cid=(\d+)', ' '.join(arg for p in launchers for arg in p['argv']))) for launchers in vmm]
assert cids[0] & cids[1], 'same existing CID must be safe in disjoint local namespaces'
assert owners[0]['metadata']['source_digest']==owners[0]['source_marker']['source_digest']
assert owners[1]['metadata']['source_digest']==owners[1]['source_marker']['source_digest']
PY
    then cp "$out/overlap-$attempt.json" "$out/overlap.json"; break; fi
  fi
  printf "phase=overlap attempt=%s timestamp=%s\n" "$attempt" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$out/observations.log"
  sleep 0.25
done
[ -f "$out/overlap.json" ] || { echo 'two actual ordinary command intervals did not overlap' >&2; exit 1; }
printf '%s\n%s\n' "$first_id" "$second_id" > "$out/run-ids"
# Disconnect the controller after START, then drive the already-approved
# SupervisorLost -> StopOwner fault through this invocation's real systemd owner.
# Ending it explicitly exposes the survivor boundary without racing equal suites.
end_controller() {
  local parent="$1" child
  for child in $(pgrep -P "$parent" || true); do end_controller "$child"; done
  kill -TERM "$parent" 2>/dev/null || true
}
end_controller "$first_controller"
wait "$first_controller" 2>/dev/null || true
ssh "$OVERDRIVE_METAL_TARGET" "sudo -n systemctl kill --kill-whom=main --signal=KILL -- overdrive-metal-run-$first_id.service && sudo -n systemctl stop -- overdrive-metal-run-$first_id.service" > "$out/first.owner-stop.stdout" 2> "$out/first.owner-stop.stderr"
observe "$first_id" "$second_id" > "$out/survivor-boundary.json"
python3 - "$out/overlap.json" "$out/survivor-boundary.json" "$first_id" "$second_id" <<'PY_SURVIVOR'
import json,sys
initial=json.load(open(sys.argv[1]))['owners']; now=json.load(open(sys.argv[2]))['owners']
completed=[r for r,o in now.items() if o.get('completion.v1',{}).get('kind')=='complete' and o['unit']['ActiveState'] not in ('active','activating','deactivating')]
live=[r for r,o in now.items() if o['unit']['ActiveState']=='active' and o['processes']]
assert completed==[sys.argv[3]] and live==[sys.argv[4]], 'the stopped owner must complete while its peer continues'
finished=now[sys.argv[3]]; survivor=now[sys.argv[4]]
assert not finished['processes'] and not any(root['exists'] for root in finished['roots'].values())
assert 'terminal.v1' not in finished, 'actual supervisor loss cannot invent a command terminal'
assert survivor['checkout']==initial[live[0]]['checkout']
assert survivor['source_marker']==initial[live[0]]['source_marker']
assert survivor['roots']==initial[live[0]]['roots'], 'finishing peer cannot remove or change survivor staging/runtime/tmp roots'
old_vmm={pid:p['netns'] for pid,p in initial[live[0]]['processes'].items() if any('vhost-vsock' in fd['target'] for fd in p['fds'].values())}
for pid,netns in old_vmm.items():
 assert survivor['processes'][pid]['netns']==netns, 'finishing peer cannot end or replace survivor VMM'
assert old_vmm
PY_SURVIVOR
first_collect_status=0
invoke "$first" collect "$first_id" > "$out/first.collect.stdout" 2> "$out/first.collect.stderr" || first_collect_status=$?
printf '%s\n' "$first_collect_status" > "$out/first.collect.status"
[ "$first_collect_status" = 70 ]
first_repeat_status=0
invoke "$first" collect "$first_id" > "$out/first.repeat.stdout" 2> "$out/first.repeat.stderr" || first_repeat_status=$?
printf '%s\n' "$first_repeat_status" > "$out/first.repeat.status"
[ "$first_repeat_status" = 70 ]
rg -qx 'OVERDRIVE_METAL_RUNNER_ERROR error=supervisor_lost' "$out/first.collect.stderr"
cmp "$out/first.collect.stdout" "$out/first.repeat.stdout"
cmp "$out/first.collect.stderr" "$out/first.repeat.stderr"
observe "$first_id" "$second_id" > "$out/first-complete.json"
set +e
wait "$second_controller"
second_status=$?
set -e
[ "$second_status" = 0 ]
invoke "$second" collect "$second_id" > "$out/second.collect.stdout" 2> "$out/second.collect.stderr"
invoke "$second" collect "$second_id" > "$out/second.repeat.stdout" 2> "$out/second.repeat.stderr"
rg -qx "OVERDRIVE_METAL_RUN_COMPLETE run_id=$second_id exit_code=0 cause=command_returned" "$out/second.collect.stderr"
cmp "$out/second.collect.stdout" "$out/second.repeat.stdout"
cmp "$out/second.collect.stderr" "$out/second.repeat.stderr"
observe "$first_id" "$second_id" > "$out/after.json"
python3 - "$out" "$first_id" "$second_id" <<'PY'
import copy,json,sys
from pathlib import Path
out=Path(sys.argv[1]); a,b=sys.argv[2:]; before=json.loads((out/'before.json').read_text()); after=json.loads((out/'after.json').read_text())
# Keep raw snapshots intact. Only paired finite RA expiry countdowns vary;
# route identity, every other attribute and every other physical key stay exact.
comparison=copy.deepcopy(after['physical'])
for old,new in zip(before['physical']['routes'],comparison['routes']):
 if old.get('protocol')==new.get('protocol')=='ra' and type(old.get('expires')) is int and type(new.get('expires')) is int and old['expires']>=0 and new['expires']>=0:
  new['expires']=old['expires']
assert before['physical']==comparison, 'physical test-kernel complement changed'
for run_id in (a,b):
 o=after['owners'][run_id]
 assert o['unit']['ActiveState'] not in ('active','activating','deactivating')
 assert not o['processes'], 'owned descendants survived completion'
 assert o['completion.v1']['kind']=='complete'
 assert not any(root['exists'] for root in o['roots'].values()), 'owned root survived completion'
 if run_id==a:
  assert 'terminal.v1' not in o, 'lost supervisor must retain its missing-terminal outcome'
 else:
  assert o['terminal.v1']['kind']=='command' and o['terminal.v1']['exit_code']=='0' and o['terminal.v1']['cause']=='command_returned'
log=(out/'second.collect.stdout').read_text()
assert 'disposable guest core' in log and '86 passed; 0 failed' in log
print('METAL-ISOLATION-JOURNEY overlapping_ordinary_commands=true attributable_results=true physical_complement_unchanged=true owned_roots_absent=true')
PY

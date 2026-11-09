#!/usr/bin/env bash
# Real systemd owner and physical maintenance admission, through developer ports.
# CONTRACT_SHAPE: bounded-change (public guest-timeout invocation plus three real
# E4 owners on the shared fixture barrier; records/process/root complements).
set -euo pipefail
[ "$#" = 2 ] || { echo 'usage: admission_journey.sh CHECKOUT OUTPUT_DIR' >&2; exit 64; }
checkout="$(cd "$1" && pwd -P)"
mkdir -p "$2"
out="$(cd "$2" && pwd -P)"
[ -z "$(ls -A "$out")" ] || { echo "choose a new empty evidence directory" >&2; exit 64; }
python3 - "$out" "$checkout" <<'PY_OUTPUT'
import sys
from pathlib import Path
out=Path(sys.argv[1]).resolve()
if any(out.is_relative_to(Path(source).resolve()) for source in sys.argv[2:]):
 raise SystemExit('Choose an empty evidence directory outside the input worktrees so capture cannot change source identity')
PY_OUTPUT
exec 2> >(tee -a "$out/driver.stderr" >&2)
: "${OVERDRIVE_METAL_TARGET:?export existing configured Metal target}"
observe() { ssh "$OVERDRIVE_METAL_TARGET" 'sudo -n python3 -' "$@" < "$checkout/infra/metal/tests/observe_host.py"; }
run_id() { sed -n 's/^OVERDRIVE_METAL_RUN_ID=\([0-9a-f]\{24\}\)$/\1/p' "$1" | head -1; }
host_boot_id="$(ssh "$OVERDRIVE_METAL_TARGET" 'cat /proc/sys/kernel/random/boot_id')"
(cd "$checkout" && exec cargo xtask metal run -- env "RUST_LOG=metal_host_boot_id=$host_boot_id" cargo nextest run --manifest-path infra/metal/tests/fixtures/attempt/Cargo.toml --target-dir target/metal-run-isolation-fixture --run-ignored only -E 'test(selected_attempt_waits_for_its_timeout)' --no-capture) > "$out/ordinary.stdout" 2> "$out/ordinary.stderr" &
ordinary_controller=$!
for ((attempt=0; attempt<1200; attempt++)); do
  ordinary_id="$(run_id "$out/ordinary.stderr")"
  if [ -n "$ordinary_id" ] && observe "$ordinary_id" > "$out/ordinary-start-$attempt.json" 2> "$out/ordinary-start-query-$attempt.stderr"; then
    if python3 - "$out/ordinary-start-$attempt.json" "$ordinary_id" <<'PY_BODY_STARTED'
import base64,json,sys
owner=json.load(open(sys.argv[1]))['owners'][sys.argv[2]]
assert owner['unit']['ActiveState']=='active' and owner['phase.v1']['phase']=='running'
assert any(b'METAL-ISOLATION-TIMEOUT body_started=true' in base64.b64decode(process['guest_console']['raw_b64'],validate=True).splitlines() for process in owner['processes'].values() if 'guest_console' in process), 'the actual selected body has not been observed in its owned guest console'
PY_BODY_STARTED
    then cp "$out/ordinary-start-$attempt.json" "$out/ordinary-running.json"; break; fi
  fi
  if ! kill -0 "$ordinary_controller" 2>/dev/null; then break; fi
  sleep 0.1
done
[ -f "$out/ordinary-running.json" ] || { echo 'actual guest body-start was not observed; inspect owned-console snapshots and query errors' >&2; exit 1; }
remote_source="$(python3 - "$out/ordinary-running.json" <<'PY'
import base64,json,sys
owner=next(iter(json.load(open(sys.argv[1]))['owners'].values()))
assert owner['unit']['ActiveState']=='active'
assert owner['phase.v1']['phase']=='running'
print(base64.b64decode(owner['metadata']['remote_dir_b64'],validate=True).decode())
PY
)"
# The public default-barrier guest timeout is independent evidence. The E4
# stop window below uses the already-authored NativeUnit callback fixture;
# it is not credited as an unchanged public-bootstrap prior command.
set +e
wait "$ordinary_controller"; ordinary_status=$?
set -e
printf '%s\n' "$ordinary_status" > "$out/ordinary.status"
[ "$ordinary_status" = 100 ]
rg -q 'TIMEOUT .*selected_attempt_waits_for_its_timeout' "$out/ordinary.stdout"
observe "$ordinary_id" > "$out/ordinary-complete.json"
: > "$out/controlled.stdout"
ssh "$OVERDRIVE_METAL_TARGET" 'sudo -n python3 -' "$ordinary_id" < "$checkout/infra/metal/tests/fixtures/native-unit-controller.py" > "$out/controlled.stdout" 2> "$out/controlled.stderr" &
controlled_controller=$!
for ((attempt=0; attempt<1200; attempt++)); do
  if python3 - "$out/controlled.stdout" > "$out/fixture-input.json" 2> "$out/fixture-input-query-$attempt.stderr" <<'PY_FIXTURE_INPUT'
import hashlib,json,re,sys
from pathlib import Path
line=Path(sys.argv[1]).read_text().splitlines()[0]
context=json.loads(line)['fixture_input'];actor=context['actor']
assert re.fullmatch(r'/tmp/metal-admission-owner-[A-Za-z0-9_-]+',actor)
assert context['run_id']==hashlib.sha256(actor.encode()).hexdigest()[:24]
assert all(context[key]==actor+'/'+name for key,name in (('barrier','maintenance.lock'),('barrier_owner','maintenance.owner'),('tree','tree.lock'),('tree_owner','tree.owner')))
print(json.dumps(context))
PY_FIXTURE_INPUT
  then break; fi
  if ! kill -0 "$controlled_controller" 2>/dev/null; then break; fi
  sleep 0.01
done
read -r prior_id fixture_actor fixture_barrier fixture_owner fixture_tree fixture_tree_owner < <(python3 - "$out/fixture-input.json" <<'PY_FIXTURE_VALUES'
import json,sys
x=json.load(open(sys.argv[1]));print(*(x[key] for key in ('run_id','actor','barrier','barrier_owner','tree','tree_owner')))
PY_FIXTURE_VALUES
)
fixture_input() { ssh "$OVERDRIVE_METAL_TARGET" sudo -n bash "$fixture_actor/native-unit" input "$fixture_actor" "$1"; }
release_fixture() {
  local original="$?"
  trap - EXIT
  fixture_input command.exit > "$out/fixture-release-command.stdout" 2> "$out/fixture-release-command.stderr" || true
  fixture_input stop > "$out/fixture-release-stop.stdout" 2> "$out/fixture-release-stop.stderr" || true
  exit "$original"
}
trap release_fixture EXIT
for ((attempt=0; attempt<1200; attempt++)); do
  if observe "$prior_id" > "$out/controlled-running-$attempt.json" 2> "$out/controlled-query-$attempt.stderr"; then
    if python3 - "$out/controlled-running-$attempt.json" "$prior_id" "$fixture_barrier" "$fixture_actor" <<'PY_CONTROLLED_RUNNING'
import json,sys
owner=json.load(open(sys.argv[1]))['owners'][sys.argv[2]]
assert owner['metadata']['execution_class']=='ordinary' and owner['phase.v1']['phase']=='running' and owner['unit']['ActiveState']=='active'
main=owner['processes'][owner['unit']['MainPID']]
assert any(fd['target']==sys.argv[3] and 'FLOCK' in fd['fdinfo'] and 'READ' in fd['fdinfo'] for fd in main['fds'].values())
assert any(sys.argv[4]+'/native-unit' in process['argv'] and 'command' in process['argv'] for process in owner['processes'].values())
PY_CONTROLLED_RUNNING
    then cp "$out/controlled-running-$attempt.json" "$out/controlled-running.json"; break; fi
  fi
  sleep 0.01
done
[ -f "$out/controlled-running.json" ]
maintenance_body='set -eu
active=$(systemctl show "overdrive-metal-run-$1.service" --property=ActiveState --value)
case "$active" in active|activating|deactivating) echo premature-maintenance >&2; exit 91;; esac
test ! -e "/srv/vm/metal/$1"
test ! -L "/srv/vm/metal/$1"
test ! -e "/run/ovd-gk/$1"
test ! -L "/run/ovd-gk/$1"
test ! -e "$2/target/metal-run-tmp/$1"
test ! -L "$2/target/metal-run-tmp/$1"
printf "METAL-MAINTENANCE-START predecessor_stopped=true predecessor_roots_absent=true\n"
sleep 3'
(cd "$checkout" && exec env OVERDRIVE_METAL_LOCK_PATH="$fixture_barrier" OVERDRIVE_METAL_OWNER_PATH="$fixture_owner" OVERDRIVE_METAL_TREE_LOCK_PATH="$fixture_tree" OVERDRIVE_METAL_TREE_OWNER_PATH="$fixture_tree_owner" cargo xtask metal run --exclusive -- bash -c "$maintenance_body" metal-admission "$prior_id" "$remote_source") > "$out/maintenance.stdout" 2> "$out/maintenance.stderr" &
maintenance_controller=$!
for ((attempt=0; attempt<1200; attempt++)); do
  maintenance_id="$(run_id "$out/maintenance.stderr")"
  if [ -n "$maintenance_id" ] && observe "$prior_id" "$maintenance_id" > "$out/waiting-$attempt.json" 2> "$out/maintenance-query-$attempt.stderr"; then
    if python3 - "$out/waiting-$attempt.json" "$maintenance_id" <<'PY'
import json,sys
x=json.load(open(sys.argv[1])); m=x['owners'][sys.argv[2]]
assert m['phase.v1']['phase']=='waiting_for_lease'
assert m['metadata']['execution_class']=='exclusive'
PY
    then cp "$out/waiting-$attempt.json" "$out/maintenance-waited.json"; break; fi
  fi
  printf "phase=admission attempt=%s timestamp=%s\n" "$attempt" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$out/observations.log"
  sleep 0.05
done
[ -f "$out/maintenance-waited.json" ]
fixture_input command.exit > "$out/fixture-command-exit.stdout" 2> "$out/fixture-command-exit.stderr"
for ((attempt=0; attempt<1200; attempt++)); do
  if observe "$prior_id" "$maintenance_id" > "$out/stopping-$attempt.json" 2> "$out/stopping-query-$attempt.stderr"; then
    if python3 - "$out/stopping-$attempt.json" "$prior_id" "$maintenance_id" "$fixture_barrier" <<'PY_EXCLUSIVE_HELD'
import json,sys
owners=json.load(open(sys.argv[1]))['owners']; ordinary,maintenance=(owners[run] for run in sys.argv[2:-1])
assert ordinary['metadata']['execution_class']=='ordinary' and ordinary['phase.v1']['phase']=='running'
assert ordinary['unit']['ActiveState']=='deactivating', 'the admitted predecessor must still be in its real stop/cleanup path'
assert ordinary['unit_after']['ActiveState']=='deactivating' and ordinary['unit_after']['MainPID']=='0'
assert ordinary['terminal.v1']['kind']=='command' and ordinary['terminal.v1']['exit_code']=='0'
assert maintenance['metadata']['execution_class']=='exclusive'
assert maintenance['unit']['ActiveState']=='active' and maintenance['phase.v1']['phase']=='waiting_for_lease'
assert maintenance['unit_after']['ActiveState']=='active' and maintenance['unit_after']['MainPID']==maintenance['unit']['MainPID'] and maintenance['phase_after']['phase']=='waiting_for_lease'
main=maintenance['processes'][maintenance['unit']['MainPID']]
assert any(fd['target']==sys.argv[-1] and 'FLOCK' in fd['fdinfo'] and 'WRITE' in fd['fdinfo'] for fd in main['fds'].values()), 'the actual exclusive owner must hold the shared fixture barrier'
PY_EXCLUSIVE_HELD
    then cp "$out/stopping-$attempt.json" "$out/maintenance-stopping-predecessor.json"; break; fi
  fi
  if ! kill -0 "$maintenance_controller" 2>/dev/null; then break; fi
  printf "phase=admission attempt=%s timestamp=%s\n" "$attempt" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$out/observations.log"
  sleep 0.05
done
[ -f "$out/maintenance-stopping-predecessor.json" ]
# Register behind the real EX descriptor while the previously admitted unit is
# still stopping. Its waiting record must not become another admitted predecessor.
(cd "$checkout" && exec env OVERDRIVE_METAL_LOCK_PATH="$fixture_barrier" OVERDRIVE_METAL_OWNER_PATH="$fixture_owner" OVERDRIVE_METAL_TREE_LOCK_PATH="$fixture_tree" OVERDRIVE_METAL_TREE_OWNER_PATH="$fixture_tree_owner" cargo xtask metal run -- cargo check -p xtask) > "$out/waiter.stdout" 2> "$out/waiter.stderr" &
waiter_controller=$!
for ((attempt=0; attempt<100; attempt++)); do
  waiter_id="$(run_id "$out/waiter.stderr")"
  if [ -n "$waiter_id" ] && observe "$prior_id" "$maintenance_id" "$waiter_id" > "$out/waiter-$attempt.json" 2> "$out/waiter-query-$attempt.stderr"; then
    if python3 - "$out/waiter-$attempt.json" "$prior_id" "$maintenance_id" "$waiter_id" "$fixture_barrier" <<'PY'
import json,sys
owners=json.load(open(sys.argv[1]))['owners']; ordinary,m,w=(owners[run] for run in sys.argv[2:-1])
assert ordinary['metadata']['execution_class']=='ordinary' and ordinary['phase.v1']['phase']=='running'
assert ordinary['unit']['ActiveState']=='deactivating', 'the admitted predecessor must still be stopping alongside the unadmitted waiter'
assert ordinary['unit_after']['ActiveState']=='deactivating' and ordinary['unit_after']['MainPID']=='0', 'the stop window must bracket the whole three-owner read'
assert ordinary['terminal.v1']['kind']=='command' and ordinary['terminal.v1']['exit_code']=='0'
assert m['metadata']['execution_class']=='exclusive'
assert m['unit']['ActiveState']=='active' and m['phase.v1']['phase']=='waiting_for_lease'
assert m['unit_after']['ActiveState']=='active' and m['unit_after']['MainPID']==m['unit']['MainPID'] and m['phase_after']['phase']=='waiting_for_lease'
main=m['processes'][m['unit']['MainPID']]
assert any(fd['target']==sys.argv[-1] and 'FLOCK' in fd['fdinfo'] and 'WRITE' in fd['fdinfo'] for fd in main['fds'].values()), 'the exclusive owner must still hold admission'
assert w['metadata']['execution_class']=='ordinary'
assert w['unit']['ActiveState']=='active' and w['phase.v1']['phase']=='waiting_for_lease'
assert w['unit_after']['ActiveState']=='active' and w['unit_after']['MainPID']==w['unit']['MainPID'] and w['phase_after']['phase']=='waiting_for_lease'
assert not any(fd['target']==sys.argv[-1] and 'FLOCK' in fd['fdinfo'] for process in w['processes'].values() for fd in process['fds'].values()), 'a registered but unadmitted waiter cannot own a shared barrier descriptor'
PY
    then cp "$out/waiter-$attempt.json" "$out/unadmitted-waiter.json"; break; fi
  fi
  sleep 0.01
done
[ -f "$out/unadmitted-waiter.json" ]
fixture_input stop > "$out/fixture-stop.stdout" 2> "$out/fixture-stop.stderr"
# The predecessor may now finish stopping. EX must pass its admitted-unit wait
# while the registered ordinary waiter remains unadmitted behind the same lock.
for ((attempt=0; attempt<1200; attempt++)); do
  if observe "$prior_id" "$maintenance_id" "$waiter_id" > "$out/progress-$attempt.json" 2> "$out/progress-query-$attempt.stderr"; then
    if python3 - "$out/progress-$attempt.json" "$prior_id" "$maintenance_id" "$waiter_id" "$fixture_barrier" <<'PY_EXCLUSIVE_PROGRESS'
import json,sys
owners=json.load(open(sys.argv[1]))['owners']; ordinary,m,w=(owners[run] for run in sys.argv[2:-1])
assert ordinary['unit']['ActiveState'] not in ('active','activating','deactivating')
assert ordinary['unit_after']['ActiveState'] not in ('active','activating','deactivating')
assert ordinary['completion.v1']['kind']=='complete' and not ordinary['processes'] and not any(root['exists'] for root in ordinary['roots'].values())
assert m['metadata']['execution_class']=='exclusive'
assert m['unit']['ActiveState']=='active' and m['phase.v1']['phase'] in ('preparing','running')
assert m['unit_after']['ActiveState']=='active' and m['unit_after']['MainPID']==m['unit']['MainPID'] and m['phase_after']['phase'] in ('preparing','running')
main=m['processes'][m['unit']['MainPID']]
assert any(fd['target']==sys.argv[-1] and 'FLOCK' in fd['fdinfo'] and 'WRITE' in fd['fdinfo'] for fd in main['fds'].values())
assert w['metadata']['execution_class']=='ordinary' and w['unit']['ActiveState']=='active' and w['phase.v1']['phase']=='waiting_for_lease'
assert w['unit_after']['ActiveState']=='active' and w['unit_after']['MainPID']==w['unit']['MainPID'] and w['phase_after']['phase']=='waiting_for_lease'
assert not any(fd['target']==sys.argv[-1] and 'FLOCK' in fd['fdinfo'] for process in w['processes'].values() for fd in process['fds'].values())
PY_EXCLUSIVE_PROGRESS
    then cp "$out/progress-$attempt.json" "$out/maintenance-passed-unadmitted-waiter.json"; break; fi
  fi
  if ! kill -0 "$maintenance_controller" 2>/dev/null; then break; fi
  sleep 0.05
done
[ -f "$out/maintenance-passed-unadmitted-waiter.json" ]
set +e
wait "$controlled_controller"; prior_status=$?
wait "$maintenance_controller"; maintenance_status=$?
wait "$waiter_controller"; waiter_status=$?
set -e
printf '%s\n' "$prior_status" > "$out/controlled.status"
printf '%s\n' "$maintenance_status" > "$out/maintenance.status"
printf '%s\n' "$waiter_status" > "$out/waiter.status"
[ "$ordinary_status" = 100 ] && [ "$prior_status" = 0 ] && [ "$maintenance_status" = 0 ] && [ "$waiter_status" = 0 ]
rg -q 'TIMEOUT .*selected_attempt_waits_for_its_timeout' "$out/ordinary.stdout"
rg -q 'METAL-MAINTENANCE-START predecessor_stopped=true predecessor_roots_absent=true' "$out/maintenance.stdout"
controlled_collect_status=0
(cd "$checkout" && cargo xtask metal collect "$prior_id") > "$out/controlled.collect.stdout" 2> "$out/controlled.collect.stderr" || controlled_collect_status=$?
printf '%s\n' "$controlled_collect_status" > "$out/controlled.collect.status"
[ "$controlled_collect_status" = 0 ]
rg -qx "OVERDRIVE_METAL_RUN_COMPLETE run_id=$prior_id exit_code=0 cause=command_returned" "$out/controlled.collect.stderr"
observe "$ordinary_id" "$prior_id" "$maintenance_id" "$waiter_id" > "$out/completed.json"
python3 - "$out/completed.json" "$ordinary_id" <<'PY'
import json,sys
for run_id,owner in json.load(open(sys.argv[1]))['owners'].items():
 assert owner['completion.v1']['kind']=='complete'
 assert not owner['processes'] and not any(root['exists'] for root in owner['roots'].values())
 assert owner['unit_after']['ActiveState'] not in ('active','activating','deactivating')
 assert owner['terminal.v1']['kind']=='command' and owner['terminal.v1']['exit_code']==('100' if run_id==sys.argv[2] else '0') and owner['terminal.v1']['cause']=='command_returned'
print('METAL-ISOLATION-ADMISSION public_guest_timeout=true e4_boundary=controlled-owner-fixture maintenance_waited=true stopped_owner_before_start=true unadmitted_waiter_did_not_block=true')
PY
trap - EXIT
ssh "$OVERDRIVE_METAL_TARGET" sudo -n rm -rf -- "$fixture_actor" > "$out/fixture-cleanup.stdout" 2> "$out/fixture-cleanup.stderr"

#!/usr/bin/env bash
# Real systemd owner and physical maintenance admission, through developer ports.
# CONTRACT_SHAPE: bounded-change (three invocation records/locks/process trees and
# their exact roots; maintenance body is read-only; other owners are complements).
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
(cd "$checkout" && exec cargo xtask metal run --exclusive -- bash -c "$maintenance_body" metal-admission "$ordinary_id" "$remote_source") > "$out/maintenance.stdout" 2> "$out/maintenance.stderr" &
maintenance_controller=$!
for ((attempt=0; attempt<1200; attempt++)); do
  maintenance_id="$(run_id "$out/maintenance.stderr")"
  if [ -n "$maintenance_id" ] && observe "$ordinary_id" "$maintenance_id" > "$out/waiting-$attempt.json" 2> "$out/maintenance-query-$attempt.stderr"; then
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
for ((attempt=0; attempt<1200; attempt++)); do
  if rg -q 'METAL-MAINTENANCE-START predecessor_stopped=true predecessor_roots_absent=true' "$out/maintenance.stdout"; then break; fi
  printf "phase=admission attempt=%s timestamp=%s\n" "$attempt" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$out/observations.log"
  sleep 0.05
done
rg -q 'METAL-MAINTENANCE-START predecessor_stopped=true predecessor_roots_absent=true' "$out/maintenance.stdout"
# Register an ordinary request while the real exclusive body owns admission.
# Its waiting record must not turn it into a predecessor that blocks that owner.
(cd "$checkout" && exec cargo xtask metal run -- cargo check -p xtask) > "$out/waiter.stdout" 2> "$out/waiter.stderr" &
waiter_controller=$!
for ((attempt=0; attempt<100; attempt++)); do
  waiter_id="$(run_id "$out/waiter.stderr")"
  if [ -n "$waiter_id" ] && observe "$maintenance_id" "$waiter_id" > "$out/waiter-$attempt.json" 2> "$out/waiter-query-$attempt.stderr"; then
    if python3 - "$out/waiter-$attempt.json" "$maintenance_id" "$waiter_id" <<'PY'
import json,sys
x=json.load(open(sys.argv[1]))['owners']; m,w=x[sys.argv[2]],x[sys.argv[3]]
assert m['unit']['ActiveState']=='active' and m['phase.v1']['phase']=='running'
assert w['phase.v1']['phase']=='waiting_for_lease'
PY
    then cp "$out/waiter-$attempt.json" "$out/unadmitted-waiter.json"; break; fi
  fi
  sleep 0.01
done
[ -f "$out/unadmitted-waiter.json" ]
set +e
wait "$ordinary_controller"; ordinary_status=$?
wait "$maintenance_controller"; maintenance_status=$?
wait "$waiter_controller"; waiter_status=$?
set -e
[ "$ordinary_status" = 100 ] && [ "$maintenance_status" = 0 ] && [ "$waiter_status" = 0 ]
observe "$ordinary_id" "$maintenance_id" "$waiter_id" > "$out/completed.json"
python3 - "$out/completed.json" <<'PY'
import json,sys
for owner in json.load(open(sys.argv[1]))['owners'].values():
 assert owner['completion.v1']['kind']=='complete'
 assert not owner['processes'] and not any(root['exists'] for root in owner['roots'].values())
print('METAL-ISOLATION-ADMISSION maintenance_waited=true stopped_owner_before_start=true unadmitted_waiter_did_not_block=true')
PY

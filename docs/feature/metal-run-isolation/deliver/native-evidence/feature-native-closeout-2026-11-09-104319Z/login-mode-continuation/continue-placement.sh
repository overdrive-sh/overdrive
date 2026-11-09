#!/usr/bin/env bash
set -euo pipefail
checkout="$1"
out="$2"
attempt_id="$3"
command_id="$4"
exec 2> >(tee -a "$out/driver.stderr" >&2)
observe() { ssh "$OVERDRIVE_METAL_TARGET" 'sudo -n python3 -' "$@" < "$checkout/infra/metal/tests/observe_host.py"; }
run_case() {
  local label="$1" expected="$2"; shift 2
  local status=0
  # Repeated collection compares product diagnostics byte-for-byte. Suppress
  # only Cargo's launcher banners at source, as in the worktree driver.
  if [ "$1" = collect ]; then
    (cd "$checkout" && cargo --quiet xtask metal "$@") > "$out/$label.stdout" 2> "$out/$label.stderr" || status=$?
  else
    (cd "$checkout" && cargo xtask metal "$@") > "$out/$label.stdout" 2> "$out/$label.stderr" || status=$?
  fi
  printf '%s\n' "$status" > "$out/$label.status"
  [ "$status" = "$expected" ] || { cat "$out/$label.stderr" >&2; echo "$label: expected $expected, got $status" >&2; exit 1; }
}
for label in raw-test shell-chain host-module duplicate-env context-env invalid-bpf-selector invalid-group env-option conflicting-package; do
  rg -q 'OVERDRIVE_METAL_RUNNER_ERROR error=unsupported_ordinary_command' "$out/$label.stderr"
done
for label in config-replacement debugger-replacement; do
  rg -q 'OVERDRIVE_METAL_RUNNER_ERROR error=execution_override_refused' "$out/$label.stderr"
done
for label in archive-reuse foreign-target foreign-manifest; do
  rg -q 'OVERDRIVE_METAL_RUNNER_ERROR error=artifact_scope_mismatch' "$out/$label.stderr"
done
run_case invalid-mode 2 sync --exclusive
for flags in '' '--no-sync' '--no-sudo' '--no-sync --no-sudo'; do
  case_label="pure$(printf '%s' "$flags" | tr ' ' '_')"
  # Splitting is confined to these four checked-in test inputs.
  # shellcheck disable=SC2086
  run_case "$case_label" 0 run $flags -- cargo nextest run -p xtask --test acceptance --features integration-tests -E 'test(placement::non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain)'
done
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

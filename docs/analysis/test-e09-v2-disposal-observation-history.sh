#!/usr/bin/env bash
# Host-safe recording regression; these port responses do not model VM timing.
# <!-- DES-ENFORCEMENT : exempt -->
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DIAG_DIR="$(mktemp -d "${TMPDIR:-/tmp}/e09-observation-history.XXXXXX")"
trap 'rm -rf -- "$DIAG_DIR"' EXIT
SVM_E09_V2_OUTPUT_ROOT="$DIAG_DIR/root"
export -n SVM_E09_V2_OUTPUT_ROOT
# Bash resolves this repository-relative path; combined lint supplies the source.
# shellcheck disable=SC1091
source "$ROOT/examples/service-kind-vm-workloads-v2/run-example.sh" check-source
mkdir -p "$CASE_ROOT/1"
printf 'startup probe[0] tcp 0.0.0.0:18999 last=fail\n' >"$CASE_ROOT/1/failure-describe.out"
printf '0\n' >"$DIAG_DIR/stops"
printf '0\n' >"$DIAG_DIR/queries"

# Native 20:53Z capture: c010 stop succeeds; first sampled Terminated at
# +85.200s. Availability rounds upward to logical second 86; this is a polled
# observation window, not a manufactured internal-transition timestamp.
if [[ "${1:-}" == --replay-native-disposal-window ]]; then
  cat >"$DIAG_DIR/native-stop" <<'NATIVE_CAPTURE'
Stopped workload 'service-e09-v2-c010-f'.
Endpoint: https://127.0.0.1:7644/
NATIVE_CAPTURE
  cat >"$DIAG_DIR/native-before" <<'NATIVE_CAPTURE'
Service 'service-e09-v2-c010-f' (kind: Service)
Spec digest: 9259c41c5576e3ff1eabe974ac1292d6e2109aad11e57a236f1205374c25a0c9
Replicas (desired/running): 1/0
Alloc                    State        Restarts   Since               
alloc-service-e09-v2-c010-f-0 Failed       0          (c=84,w=local)      
    reason: driver started
Memory:        134217728
VIP:           10.96.0.12
Listeners:
  18081/tcp
Probes:
  startup probe[0] tcp 0.0.0.0:18999 last=fail (connect failed: NetworkUnreachable: Network is unreachable (os error 101)) last_observed_at=1788900988111
NATIVE_CAPTURE
  cat >"$DIAG_DIR/native-running" <<'NATIVE_CAPTURE'
Service 'service-e09-v2-c010-f' (kind: Service)
Spec digest: 9259c41c5576e3ff1eabe974ac1292d6e2109aad11e57a236f1205374c25a0c9
Replicas (desired/running): 1/1
Alloc                    State        Restarts   Since               
alloc-service-e09-v2-c010-f-0 Running      1          (c=86,w=local)      
    reason: driver started
    last terminated: Failed at (c=85,w=local) — driver internal error: primary rejection: driver vm rejected start: bind beacon listener: Address already in use (os error 98); cgroup remove: Device or resource busy (os error 16)
    last terminated ran since: 1788900973.875881219
    last terminated detail: primary rejection: driver vm rejected start: bind beacon listener: Address already in use (os error 98); cgroup remove: Device or resource busy (os error 16)
Memory:        134217728
Addresses:
  alloc-service-e09-v2-c010-f-0: 10.99.128.18
VIP:           10.96.0.12
Listeners:
  18081/tcp
Issued certificates:
  serial:        fce68f70be2725f7e5cc7a4cbe038f6b
    spiffe_id:     spiffe://overdrive.local/workload/service-e09-v2-c010-f/alloc/alloc-service-e09-v2-c010-f-0
    issuer_serial: d203e6fd13fb44a91450c4c48ec97222
    not_after:     1788904535.286534092
Probes:
  startup probe[0] tcp 0.0.0.0:18999 last=fail (connection refused) last_observed_at=1788901053550
NATIVE_CAPTURE
  cat >"$DIAG_DIR/native-terminal" <<'NATIVE_CAPTURE'
Service 'service-e09-v2-c010-f' (kind: Service)
Spec digest: 9259c41c5576e3ff1eabe974ac1292d6e2109aad11e57a236f1205374c25a0c9
Replicas (desired/running): 1/0
Alloc                    State        Restarts   Since               
alloc-service-e09-v2-c010-f-0 Terminated   1          (c=99,w=local)      
    reason: stopped
    last terminated: Failed at (c=85,w=local) — driver internal error: primary rejection: driver vm rejected start: bind beacon listener: Address already in use (os error 98); cgroup remove: Device or resource busy (os error 16)
    last terminated ran since: 1788900973.875881219
    last terminated detail: primary rejection: driver vm rejected start: bind beacon listener: Address already in use (os error 98); cgroup remove: Device or resource busy (os error 16)
Memory:        134217728
VIP:           10.96.0.12
Listeners:
  18081/tcp
Probes:
  startup probe[0] tcp 0.0.0.0:18999 last=fail (connection refused) last_observed_at=1788901137714
NATIVE_CAPTURE

  # CONTRACT_SHAPE: bounded-change. Same observed stop/describe responses and
  # existing helper entry points; logical polling avoids real VM/process work.
  # The runtime probe adapter is empty only for this shell-policy control.
  replay_window() (
    local helper="$1" label="$2" rc=0 observed
    local case_dir="$DIAG_DIR/$label"
    mkdir -p "$case_dir"
    cp "$DIAG_DIR/native-before" "$case_dir/failure-describe.out"
    local logical_start=$SECONDS
    # Sourced helpers invoke these command adapters dynamically.
    # shellcheck disable=SC2329
    assert_serve_identity() { :; }
    # shellcheck disable=SC2329
    workload_runtime_resources_remain() { return 1; }
    # Each replay owns its logical clock within this subshell.
    # shellcheck disable=SC2329,SC2030
    sleep() { SECONDS=$((SECONDS + 1)); }
    # shellcheck disable=SC2329
    bounded() {
      if [[ "$*" == *'job stop'* ]]; then
        cat "$DIAG_DIR/native-stop"
      elif [[ "$((SECONDS - logical_start))" -lt 86 ]]; then
        cat "$DIAG_DIR/native-running"
      else
        cat "$DIAG_DIR/native-terminal"
      fi
    }
    if [[ "$helper" == stop_workload ]]; then
      stop_workload service-e09-v2-c010-f "$case_dir" stop.out final.out || rc=$?
    else
      dispose_failed_service service-e09-v2-c010-f "$case_dir" || rc=$?
    fi
    observed=rejected
    [[ "$rc" -ne 0 ]] || observed=accepted
    printf '%s: final_sample_available_after_seconds=86 expected=accepted observed=%s\n' "$label" "$observed"
    [[ "$observed" == accepted ]]
  )
  failed=0
  replay_window stop_workload ordinary_stop_control || failed=1
  replay_window dispose_failed_service disposal_window_regression || failed=1
  exit "$failed"
fi

# CONTRACT_SHAPE: bounded-change. Recording must preserve main-attempt command
# and failed/successful polling responses byte-for-byte after real EXIT cleanup
# retries disposal. Adapter responses isolate recording, not a product defect.
run_worker() (
  # Sourced recording and cleanup helpers invoke these adapters dynamically.
  # shellcheck disable=SC2329
  now_ns() { python3 -c 'import time; print(time.time_ns())'; }
  # shellcheck disable=SC2329
  assert_serve_identity() { :; }
  # This worker owns an independent logical clock, not the replay's clock.
  # shellcheck disable=SC2329,SC2031
  sleep() { SECONDS=$((SECONDS + 31)); }
  # shellcheck disable=SC2329
  bounded() {
    local n
    if [[ "$*" == *'job stop'* ]]; then
      n=$(( $(<"$DIAG_DIR/stops") + 1 ))
      printf '%s\n' "$n" >"$DIAG_DIR/stops"
      printf 'stop-%s-stdout\n' "$n"
      printf 'stop-%s-stderr\n' "$n" >&2
      return 0
    fi
    n=$(( $(<"$DIAG_DIR/queries") + 1 ))
    printf '%s\n' "$n" >"$DIAG_DIR/queries"
    if [[ "$n" -eq 1 ]]; then
      printf 'query-1-failed\n' >&2
      return 7
    fi
    printf 'Service sample\nAlloc State Restarts Since\n'
    if [[ "$(<"$DIAG_DIR/stops")" -eq 1 ]]; then
      printf 'alloc-sample-0 Running 1 sample\n'
    else
      printf 'alloc-sample-0 Terminated 1 sample\n    reason: stopped\n'
    fi
  }
  WORKER_DIR="$CASE_ROOT/1"
  # These globals are consumed by the sourced worker_cleanup helper.
  # shellcheck disable=SC2034
  WORKER_TIMING="$WORKER_DIR/timing.tsv"
  # shellcheck disable=SC2034
  WORKER_STAGE=failure-stop
  # shellcheck disable=SC2034
  WORKER_CLIENT_DEPLOYED=0
  # shellcheck disable=SC2034
  WORKER_HEALTHY_DEPLOYED=0
  # shellcheck disable=SC2034
  WORKER_FAILURE_DEPLOYED=1
  # shellcheck disable=SC2034
  WORKER_FAILURE_SERVICE=sample
  # The sourced cleanup helper invokes this result adapter dynamically.
  # shellcheck disable=SC2329
  write_worker_result() { printf '%s\n' "$1" >"$DIAG_DIR/worker-result"; }
  if dispose_failed_service sample "$WORKER_DIR"; then
    printf 'unexpected successful main disposal\n' >&2
    exit 2
  fi
  if [[ -d "$WORKER_DIR/observations" ]]; then
    cp -R "$WORKER_DIR/observations" "$DIAG_DIR/main-records"
  fi
  worker_cleanup 1
)
rc=0
run_worker || rc=$?
[[ "$rc" -eq 1 && "$(<"$DIAG_DIR/stops")" -eq 2 ]] || {
  printf 'FAIL: did not exercise failed main disposal and cleanup retry\n'; exit 1;
}
history="$CASE_ROOT/1/observations"
[[ -d "$history" && -d "$DIAG_DIR/main-records" ]] || {
  printf 'FAIL: first disposal command/query records do not survive cleanup retry\n'; exit 1;
}
while IFS= read -r original; do
  relative="${original#"$DIAG_DIR/main-records/"}"
  cmp "$original" "$history/$relative" || { printf 'FAIL: original record changed\n'; exit 1; }
done < <(find "$DIAG_DIR/main-records" -type f)
print_case_transcript 1 >"$DIAG_DIR/all-records"
for value in stop-1-stdout stop-1-stderr stop-2-stdout stop-2-stderr query-1-failed \
  'context=main' 'context=exit-cleanup' 'exit=7' 'exit=0' 'exit=1' \
  started_epoch_ns= finished_epoch_ns= elapsed_ms= 'sampled_state=Running' 'sampled_state=Terminated'; do
  grep -Fq "$value" "$DIAG_DIR/all-records" || { printf 'FAIL: missing preserved %s\n' "$value"; exit 1; }
done
printf 'PASS: main command, query failure, Running sample and timings survive cleanup Terminated sample unchanged\n'

#!/usr/bin/env bash
# Host-safe behavioral diagnostic of the checked-in worker's phase ordering.
# <!-- DES-ENFORCEMENT : exempt -->
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DIAG_DIR="$(mktemp -d "${TMPDIR:-/tmp}/e09-submit-order.XXXXXX")"
trap 'rm -rf -- "$DIAG_DIR"' EXIT
source "$ROOT/examples/service-kind-vm-workloads-v2/run-example.sh" check-source
mkdir -p "$DIAG_DIR/worker" "$DIAG_DIR/cohort"

# CONTRACT_SHAPE: bounded-change. Observe the real run_trial prefix through
# existing external adapters, with a healthy successful first phase. No product
# process, lifecycle row, invented public API, or wall-clock race participates.
# The private adapter exits at failure submission, before any failure behavior
# is synthesized. The EXIT adapter preserves that diagnostic exit status.
run_prefix() (
  own_cleanup_done=0
  gated_after_cleanup=0
  assert_serve_identity() { :; }
  now_ns() { printf '1\n'; }
  record_timing() { :; }
  run_service_deploy() {
    if [[ "$1" == */healthy-service.toml ]]; then
      printf 'Service is stable witness: startup probe[0] (tcp 0.0.0.0:18081)\n' >"$2"
      return 0
    fi
    printf 'failure-submit: own_cleanup=%s cohort_gate_after_cleanup=%s\n' \
      "$own_cleanup_done" "$gated_after_cleanup" >"$DIAG_DIR/boundary"
    exit 42
  }
  wait_for_service_healthy() { printf 'healthy describe\n' >"$3"; }
  first_service_alloc_state() { cat >/dev/null; printf 'Running\n'; }
  target_for_probe() { printf '0.0.0.0:%s\n' "$2"; }
  all_service_alloc_ids() { cat >/dev/null; printf 'alloc-healthy\n'; }
  capture_case_resources() { :; }
  wait_for_gate() {
    if [[ "$own_cleanup_done" -eq 1 ]]; then gated_after_cleanup=1; fi
  }
  run_client_deploy() { :; }
  wait_for_job_success() { :; }
  stop_workload() { printf 'terminal describe\n' >"$2/$4"; }
  assert_case_resources_released() { own_cleanup_done=1; }
  # The sourced production EXIT trap invokes this adapter indirectly.
  # shellcheck disable=SC2329
  worker_cleanup() { exit "$1"; }
  run_trial 1 "$DIAG_DIR/worker" "$DIAG_DIR/cohort"
)

rc=0
run_prefix || rc=$?
[[ "$rc" -eq 42 && -f "$DIAG_DIR/boundary" ]] || {
  printf 'diagnostic failed to reach the actual failure-submit boundary (rc=%s)\n' "$rc" >&2
  exit 2
}
cat "$DIAG_DIR/boundary"
if ! grep -Fq 'own_cleanup=1 cohort_gate_after_cleanup=1' "$DIAG_DIR/boundary"; then
  printf 'FAIL: failure submit is reachable before a post-cleanup cohort gate\n'
  exit 1
fi
printf 'PASS: worker awaits a post-cleanup cohort gate before failure submit\n'

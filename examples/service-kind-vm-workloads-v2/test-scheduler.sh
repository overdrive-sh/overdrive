#!/usr/bin/env bash
# Host-safe orchestration tests, not native product evidence. Exercise the
# example's existing shell boundaries with private files and surrogate workers.
# No product binary, KVM, cargo, SSH, or shared host resources are used.
# Shell globals below are consumed by the sourced production functions.
# shellcheck disable=SC2034
set -euo pipefail

TEST_SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
TEST_EXAMPLE="$(dirname "$TEST_SCRIPT")/run-example.sh"

fail() { echo "E09 v2 scheduler test: $*" >&2; exit 1; }

load_example() {
  TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/svm-e09-v2-scheduler.XXXXXX")"
  trap 'rm -rf -- "$TEST_ROOT"' EXIT
  # Keep the temporary override shell-local: the real, read-only preparer
  # checks its normal safe path while the sourced example writes only here.
  SVM_E09_V2_OUTPUT_ROOT="$TEST_ROOT/output"
  export -n SVM_E09_V2_OUTPUT_ROOT
  # shellcheck source=/dev/null
  source "$TEST_EXAMPLE" check-source
  mkdir -p "$MEASURE_ROOT"
}

# CONTRACT_SHAPE: bounded-change. Capacity selection may write only its
# capacity record; ten workers must not be silently reduced by the old cap.
test_capacity() {
  load_example
  nproc() { printf '16\n'; }
  awk() {
    if [[ "${2:-}" == /proc/meminfo ]]; then
      printf '67001489817\n'
    else
      command awk "$@"
    fi
  }
  SVM_E09_V2_CONCURRENCY=10 configure_capacity
  [[ "$CONCURRENCY" -eq 10 ]] || fail "requested 10 workers, selected $CONCURRENCY"
  grep -Fq 'configured_concurrency=10' "$MEASURE_ROOT/capacity" \
    || fail 'capacity evidence omits the actual ten-worker selection'
  unset SVM_E09_V2_CONCURRENCY
  configure_capacity
  [[ "$CONCURRENCY" -eq 10 ]] || fail "default is not ten workers: $CONCURRENCY"
}

# CONTRACT_SHAPE: bounded-change. Malformed existing concurrency overrides
# fail explicitly and do not create a successful capacity record.
test_invalid_capacity() {
  load_example
  nproc() { printf '16\n'; }
  awk() {
    if [[ "${2:-}" == /proc/meminfo ]]; then printf '67001489817\n';
    else command awk "$@"; fi
  }
  local invalid
  for invalid in 0 -1 ten; do
    if (SVM_E09_V2_CONCURRENCY="$invalid" configure_capacity) >"$TEST_ROOT/error" 2>&1; then
      fail "malformed concurrency $invalid was accepted"
    fi
    grep -Fq 'SVM_E09_V2_CONCURRENCY must be a positive integer' "$TEST_ROOT/error" \
      || fail "malformed concurrency $invalid lost its explicit error"
    [[ ! -e "$MEASURE_ROOT/capacity" ]] || fail 'invalid input created capacity evidence'
  done
}

# CONTRACT_SHAPE: bounded-change. Materialization emits exactly 20 stable
# healthy/failure identities and four checked-in-derived specs per pair.
test_manifest() {
  load_example
  materialize_specs
  local count
  count="$(awk 'END { print NR - 1 }' "$INPUT_MANIFEST")"
  [[ "$count" -eq 40 ]] || fail "expected 40 healthy/failure inputs, observed $count"
  [[ "$(find "$CASE_ROOT" -name '*.toml' | wc -l | tr -d ' ')" -eq 80 ]] \
    || fail 'expected exactly four materialized specs per pair'
  local trial phase service client service_spec client_spec expected_trial=1 expected_phase=healthy
  while IFS=$'\t' read -r trial phase service client service_spec client_spec; do
    [[ "$trial" == trial ]] && continue
    [[ "$trial" -eq "$expected_trial" && "$phase" == "$expected_phase" ]] \
      || fail "input sequence changed at pair $trial/$phase"
    local suffix=h
    [[ "$phase" == healthy ]] || suffix=f
    local expected_id
    printf -v expected_id 'service-e09-v2-c%03d-%s' "$trial" "$suffix"
    [[ "$service" == "$expected_id" && "$client" == "$expected_id-client" ]] \
      || fail 'materialized identity is not deterministic'
    [[ -s "$service_spec" && -s "$client_spec" ]] || fail 'materialized spec missing'
    if [[ "$phase" == healthy ]]; then
      expected_phase=failure
    else
      expected_phase=healthy
      expected_trial=$((expected_trial + 1))
    fi
  done <"$INPUT_MANIFEST"
  [[ "$expected_trial" -eq 21 && "$expected_phase" == healthy ]] \
    || fail 'manifest did not retain all twenty complete pairs'
}

# CONTRACT_SHAPE: bounded-change. Completed, failed, cancelled, and untouched
# inputs remain distinguishable in the same deterministic twenty-row ledger.
test_partial_ledger() {
  load_example
  mkdir -p "$CASE_ROOT"
  LEDGER="$MEASURE_ROOT/ledger.tsv"
  SERVE_PID=123
  SERVE_START_TICKS=456
  local completed trial
  for completed in 0 1 3; do
    for ((trial = 1; trial <= completed; trial++)); do
      WORKER_TRIAL="$trial"
      WORKER_DIR="$CASE_ROOT/$trial"
      mkdir -p "$WORKER_DIR"
      WORKER_STAGE=healthy-peer
      printf -v WORKER_HEALTHY_SERVICE 'service-e09-v2-c%03d-h' "$trial"
      printf -v WORKER_FAILURE_SERVICE 'service-e09-v2-c%03d-f' "$trial"
      WORKER_CANCELLED=0
      case "$trial" in
        1) write_worker_result 0 ;;
        2) write_worker_result 9 ;;
        3) WORKER_CANCELLED=1; write_worker_result 130 ;;
      esac
    done
    aggregate_ledger
    awk -F'\t' -v completed="$completed" '
      NR == 1 { if (NF != 20) exit 1; next }
      {
        trial = NR - 1
        outcome = trial > completed ? "not-run-cancelled" : (trial == 1 ? "pass" : (trial == 2 ? "failed" : "not-run-cancelled"))
        stage = trial > completed ? "not-started" : "healthy-peer"
        if (NF != 20 || $1 != trial || $2 != outcome || $3 != stage ||
            $4 != "123" || $5 != "456" ||
            $6 != sprintf("service-e09-v2-c%03d-h", trial) ||
            $13 != sprintf("service-e09-v2-c%03d-f", trial)) exit 1
      }
      END { if (NR != 21) exit 1 }
    ' "$LEDGER" || fail "partial ledger lost, reordered, or fabricated input outcomes ($completed results)"
    cp "$LEDGER" "$TEST_ROOT/previous-ledger"
    aggregate_ledger
    cmp "$TEST_ROOT/previous-ledger" "$LEDGER" \
      || fail 'repeated aggregation changed partial evidence'
  done
}

# CONTRACT_SHAPE: bounded-change. Two ten-worker cohorts use the production
# barrier/join/ledger orchestration. Worker bodies are deliberately surrogate
# processes, so this proves neither guest health nor kernel cleanup.
test_cohorts() {
  load_example
  mkdir -p "$CASE_ROOT"
  LEDGER="$MEASURE_ROOT/ledger.tsv"
  BASELINE_DIR="$MEASURE_ROOT/baseline"
  sleep 30 &
  SERVE_PID=$!
  SERVE_START_TICKS="$(ps -p "$SERVE_PID" -o lstart=)"
  trap 'kill "$SERVE_PID" 2>/dev/null || true; wait "$SERVE_PID" 2>/dev/null || true; rm -rf -- "$TEST_ROOT"' EXIT
  # Platform/process and resource adapters for the surrogate scheduler only.
  assert_serve_identity() {
    kill -0 "$SERVE_PID" && [[ "$(ps -p "$SERVE_PID" -o lstart=)" == "$SERVE_START_TICKS" ]]
  }
  worker_is_alive() { kill -0 "$1" 2>/dev/null; }
  find() {
    local args=() arg
    while [[ "$#" -gt 0 ]]; do
      arg="$1"; shift
      if [[ "$arg" == -printf ]]; then
        [[ "$1" == '%f\n' ]] || fail 'unexpected fixture find format'
        shift
        args+=(-print)
      else
        args+=("$arg")
      fi
    done
    command find "${args[@]}"
  }
  snapshot_all() {
    mkdir -p "$1"
    local name
    for name in hypervisors scopes run-dirs network bpf nft loops mounts clones; do
      : >"$1/$name"
    done
    printf 'store_files=0\tstore_bytes=0\n' >"$1/store"
  }
  active_resource_count() {
    printf 'vm=%s\tcgroup=%s\n' "${#WORKER_PIDS[@]}" "${#WORKER_PIDS[@]}"
  }
  run_trial() {
    WORKER_TRIAL="$1"; WORKER_DIR="$2"; WORKER_COHORT="$3"
    WORKER_STAGE=scheduled
    WORKER_CANCELLED=0
    printf -v WORKER_HEALTHY_SERVICE 'service-e09-v2-c%03d-h' "$1"
    printf -v WORKER_FAILURE_SERVICE 'service-e09-v2-c%03d-f' "$1"
    printf 'cgroup\tsurrogate-%s\nrun-dir\tsurrogate-%s\n' "$1" "$1" \
      >"$WORKER_DIR/healthy-resources-active"
    touch "$WORKER_COHORT/$1.healthy-active"
    wait_for_gate "$WORKER_COHORT/healthy-release" || return 1
    [[ "$(command find "$WORKER_COHORT" -name '*.healthy-active' | wc -l | tr -d ' ')" -eq 10 ]] \
      || return 1
    touch "$WORKER_COHORT/$1.failure-active"
    wait_for_gate "$WORKER_COHORT/failure-release" || return 1
    [[ "$(command find "$WORKER_COHORT" -name '*.failure-active' | wc -l | tr -d ' ')" -eq 10 ]] \
      || return 1
    # Pin one inverted completion order per cohort without requiring all
    # scheduler interleavings to match a wall-clock sleep race.
    if [[ "$((WORKER_TRIAL % 10))" -eq 1 ]]; then
      while [[ ! -e "$WORKER_COHORT/last-completed" ]]; do sleep 0.02; done
    fi
    WORKER_STAGE=complete
    write_worker_result 0
    printf '%s\n' "$1" >>"$TEST_ROOT/completion-order"
    [[ "$((WORKER_TRIAL % 10))" -ne 0 ]] || touch "$WORKER_COHORT/last-completed"
  }
  snapshot_all "$BASELINE_DIR"
  local trial cohort first last
  for trial in $(seq 1 20); do mkdir -p "$CASE_ROOT/$trial"; done
  for cohort in 1 2; do
    first=$(( (cohort - 1) * 10 + 1 )); last=$((cohort * 10))
    run_cohort "$cohort" "$first" "$last" || fail "surrogate cohort $cohort failed"
    [[ "$(wc -l <"$TEST_ROOT/completion-order" | tr -d ' ')" -eq "$last" ]] \
      || fail 'a worker was discarded before the next cohort'
  done
  aggregate_ledger
  assert_single_control_plane_in_ledger || fail 'control-plane surrogate identity changed'
  awk -F'\t' 'NR > 1 { if ($1 != NR - 1 || $2 != "pass") exit 1 }
    END { if (NR != 21) exit 1 }' "$LEDGER" || fail 'cohort results lost deterministic order'
  awk '$1 == 10 {ten=NR} $1 == 1 {one=NR}
    $1 == 20 {twenty=NR} $1 == 11 {eleven=NR}
    END {exit !(ten < one && twenty < eleven)}' "$TEST_ROOT/completion-order" \
    || fail 'surrogate proof did not exercise out-of-order completion'
  [[ "$(wc -l <"$MEASURE_ROOT/concurrency.tsv" | tr -d ' ')" -eq 4 ]] \
    || fail 'missing healthy/failure observations from both cohorts'
  [[ "$(grep -c 'workers=10' "$MEASURE_ROOT/concurrency.tsv")" -eq 4 ]] \
    || fail 'cohort evidence did not observe ten simultaneous workers'
}

case "${1:-all}" in
  capacity) test_capacity ;;
  invalid-capacity) test_invalid_capacity ;;
  manifest) test_manifest ;;
  partial-ledger) test_partial_ledger ;;
  cohorts) test_cohorts ;;
  all)
    bash "$TEST_SCRIPT" capacity
    bash "$TEST_SCRIPT" invalid-capacity
    bash "$TEST_SCRIPT" manifest
    bash "$TEST_SCRIPT" partial-ledger
    timeout --signal=TERM --kill-after=1s 15s bash "$TEST_SCRIPT" cohorts
    echo 'E09 v2 HOST-SAFE scheduler tests PASS (native health/throughput unverified)'
    ;;
  *) fail "unknown test: $1" ;;
esac

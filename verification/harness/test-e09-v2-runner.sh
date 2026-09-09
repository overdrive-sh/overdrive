#!/usr/bin/env bash
# Host-safe E09-v2 expectation-runner tests. The command/transport boundary
# supplies transcript inputs or a private process fixture, never a fake product.
# This file is NOT called by an expectation and emits no native evidence.
set -euo pipefail

TEST_SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
TEST_SCRIPT="${E09_TEST_SCRIPT:-$TEST_SCRIPT}"
TEST_REPO="$(cd "$(dirname "$TEST_SCRIPT")/../.." && pwd)"

fail() { echo "E09 v2 runner test: $*" >&2; exit 1; }

emit_ledger() {
  local count="$1" trial outcome pid
  echo '--- E09 v2 ledger begin ---'
  printf 'trial\toutcome\tstage\tcontrol_plane_pid\tcontrol_plane_start_ticks\thealthy_service_id\thealthy_deploy_rc\thealthy_terminal_state\thealthy_target\thealthy_peer\thealthy_allocations\thealthy_cleanup\tfailure_service_id\tfailure_deploy_rc\tfailure_terminal_state\tfailure_reason\tfailure_target\tfailure_peer\tfailure_allocations\tfailure_cleanup\n'
  for trial in $(seq 1 "$count"); do
    outcome=pass; pid=123
    [[ "$E09_TEST_MODE" != failed-row || "$trial" -ne 2 ]] || outcome=failed
    [[ "$E09_TEST_MODE" != split-identity || "$trial" -ne 2 ]] || pid=124
    printf '%s\t%s\tcomplete\t%s\t456\thealthy-%s\t0\tRunning\tguest:18081\texact-reply\th-%s\tzero-runtime\tfailure-%s\t1\tFailed\tStartupProbeFailed\tguest:18999\tno-reply\tf-%s\tzero-runtime\n' \
      "$trial" "$outcome" "$pid" "$trial" "$trial" "$trial" "$trial"
  done
  echo '--- E09 v2 ledger end ---'
}

fixture_command() {
  case "$1" in
    cargo)
      shift
      [[ "$1 $2 $3 $4" == 'xtask metal run --' ]] || fail 'unexpected cargo command'
      shift 4
      printf '%s\n' "$@" >"$E09_TEST_ROOT/remote-command"
      export E09_TEST_REMOTE=1
      # The SSH boundary executes its requested shell command locally. Replace
      # only the external example command, not timeout/runner orchestration.
      [[ "$1" == bash && "$2" == -lc ]] || fail 'unexpected remote shell boundary'
      local remote_command="$3"
      remote_command="${remote_command//examples\/service-kind-vm-workloads-v2\/run-example.sh/$TEST_SCRIPT __remote}"
      # SSH's remote owner does not share the local client's process group.
      # A local timeout must therefore leave this fixture alive (and fail the
      # test); only the command's remote-side timeout can terminate it.
      python3 -c 'import os, sys; os.setsid(); os.execvp("bash", ["bash", "-c", sys.argv[1]])' \
        "$remote_command" &
      wait "$!"
      ;;
    timeout)
      shift
      printf '%s\n' "$@" >>"$E09_TEST_ROOT/timeout-${E09_TEST_REMOTE:-0}"
      local args=() duration_seen=0 arg
      while [[ "$#" -gt 0 ]]; do
        arg="$1"; shift
        case "$arg" in
          --kill-after=60s) args+=(--kill-after=0.3s) ;;
          1200s) args+=(0.5s); duration_seen=1; break ;;
          360s) args+=(0.2s); duration_seen=1; break ;;
          -*) args+=("$arg") ;;
          *) args+=("$arg"); duration_seen=1; break ;;
        esac
      done
      [[ "$duration_seen" -eq 1 ]] || fail 'timeout duration missing'
      exec "$E09_TEST_REAL_TIMEOUT" "${args[@]}" "$@"
      ;;
  esac
}

remote_fixture() {
  [[ "$1" == run ]] || fail 'remote example operation is not run'
  printf '%s\n' "$2" "${SVM_E09_V2_CONCURRENCY:-unset}" \
    >"$E09_TEST_ROOT/example-config"
  case "$E09_TEST_MODE" in
    timeout)
      echo "$$" >>"$E09_TEST_ROOT/pids"
      bash "$TEST_SCRIPT" __descendant &
      local child=$!
      echo "$child" >>"$E09_TEST_ROOT/pids"
      echo 'partial setup/trial transcript retained'
      emit_ledger 1
      trap 'echo "remote owner observed TERM"; wait "$child"' TERM
      wait "$child"
      ;;
    short) emit_ledger 19 ;;
    long) emit_ledger 21 ;;
    historical) emit_ledger 100 ;;
    *) emit_ledger 20 ;;
  esac
  echo 'E09 v2 PASS: 20/20 truthful pairs through one unchanged control-plane PID; no retries or discarded pairs'
}

live_process() {
  local state
  state="$(ps -p "$1" -o stat= 2>/dev/null)" || return 1
  [[ -n "$state" && "$state" != Z* ]]
}

cleanup_case() {
  local rc=$? pid
  trap - EXIT HUP INT TERM
  if [[ -f "$E09_TEST_ROOT/pids" ]]; then
    while read -r pid; do
      if live_process "$pid"; then kill -KILL "$pid" 2>/dev/null || true; fi
    done <"$E09_TEST_ROOT/pids"
  fi
  rm -rf -- "$E09_TEST_ROOT"
  exit "$rc"
}

# CONTRACT_SHAPE: bounded-change. Runner accepts exactly twenty successful
# transcript rows from one identity; malformed sample/identity outcomes fail.
test_transcript_case() {
  local mode="$1" expected_rc="$2"
  E09_TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/svm-e09-v2-runner.XXXXXX")"
  export E09_TEST_ROOT E09_TEST_MODE="$mode"
  export E09_TEST_SCRIPT="$TEST_SCRIPT"
  export E09_TEST_REAL_TIMEOUT
  E09_TEST_REAL_TIMEOUT="$(command -v timeout)"
  trap cleanup_case EXIT HUP INT TERM
  mkdir -p "$E09_TEST_ROOT/bin" "$E09_TEST_ROOT/evidence"
  ln -s "$TEST_SCRIPT" "$E09_TEST_ROOT/bin/cargo"
  ln -s "$TEST_SCRIPT" "$E09_TEST_ROOT/bin/timeout"
  local runner
  runner="$TEST_REPO/verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/runner.sh"
  # The legacy location allows an honest behavioral RED before its active
  # successor exists; historical evidence is never read or rewritten.
  [[ -f "$runner" ]] || runner="$TEST_REPO/verification/expectations/E09-v2-vm-service-tcp-truthfulness-100/runner.sh"
  local rc=0
  PATH="$E09_TEST_ROOT/bin:$PATH" REPO_ROOT="$TEST_REPO" \
    EVIDENCE_DIR="$E09_TEST_ROOT/evidence" bash "$runner" \
    >"$E09_TEST_ROOT/runner.out" 2>&1 || rc=$?
  if [[ "$expected_rc" == zero ]]; then
    [[ "$rc" -eq 0 ]] || { cat "$E09_TEST_ROOT/runner.out" >&2; fail "$mode returned $rc"; }
  else
    [[ "$rc" -ne 0 ]] || fail "$mode accepted an invalid sample"
  fi
  if [[ "$mode" != timeout ]]; then
    grep -Fxq 'exit: 0' "$E09_TEST_ROOT/evidence/product-run.meta" \
      || fail "$mode failed at transport/setup instead of the transcript validation boundary"
  fi
  if [[ "$mode" == valid ]]; then
    [[ "$(<"$E09_TEST_ROOT/example-config")" == $'tcp-truthfulness-20\n10' ]] \
      || fail 'actual remote invocation did not carry the twenty-pair selector and ten-worker configuration'
  fi
  if [[ "$mode" == timeout ]]; then
    grep -Fq 'partial setup/trial transcript retained' "$E09_TEST_ROOT/evidence/product-run.out" \
      || fail 'partial transcript was lost on timeout'
    grep -Fq 'remote owner observed TERM' "$E09_TEST_ROOT/evidence/product-run.out" \
      || fail 'deadline did not signal the remote owner'
    grep -Fxq 1200s "$E09_TEST_ROOT/timeout-1" || fail 'remote budget was not 1200s'
    grep -Fxq -- --kill-after=60s "$E09_TEST_ROOT/timeout-1" \
      || fail 'cleanup grace was not 60s'
    local pid
    while read -r pid; do
      ! live_process "$pid" || fail "remote owner/descendant $pid survived the deadline"
    done <"$E09_TEST_ROOT/pids"
  fi
}

# CONTRACT_SHAPE: bounded-change. The deadline must terminate a remote owner
# and its TERM-resistant descendant, retaining partial output with nonzero exit.
test_timeout() { test_transcript_case timeout nonzero; }

case "$(basename "$0")" in
  cargo|timeout) fixture_command "$(basename "$0")" "$@"; exit ;;
esac
case "${1:-all}" in
  __remote) shift; remote_fixture "$@" ;;
  __descendant) trap '' TERM; while :; do sleep 0.05; done ;;
  timeout) test_timeout ;;
  valid) test_transcript_case valid zero ;;
  short|long|historical|failed-row|split-identity) test_transcript_case "$1" nonzero ;;
  all)
    for mode in valid short long historical failed-row split-identity timeout; do
      timeout --signal=TERM --kill-after=1s 8s bash "$TEST_SCRIPT" "$mode"
    done
    echo 'E09 v2 HOST-SAFE runner tests PASS (native product outcomes unverified)'
    ;;
  *) fail "unknown test mode: $1" ;;
esac

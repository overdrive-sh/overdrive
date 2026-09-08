#!/usr/bin/env bash
# Host-safe scheduler proof for E09-v2.  This deliberately uses synthetic
# sleep workers; it is not native product evidence and never invokes cargo or
# an overdrive crate.  The native runner is responsible for exercising the
# same barriers around real deploy/describe/stop operations.
set -euo pipefail

TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/svm-e09-v2-scheduler.XXXXXX")"
CP_PID=""
CP_START_ID=""

fail() {
  echo "E09 v2 scheduler test: $*" >&2
  exit 1
}

proc_start_id() {
  ps -p "$1" -o lstart= | sed 's/[[:space:]][[:space:]]*/ /g; s/^ //; s/ $//'
}

cleanup() {
  local rc=$?
  trap - EXIT HUP INT TERM
  if [[ -n "$CP_PID" ]] && kill -0 "$CP_PID" 2>/dev/null; then
    kill -TERM "$CP_PID" 2>/dev/null || true
    wait "$CP_PID" 2>/dev/null || true
  fi
  rm -rf -- "$TEST_ROOT"
  exit "$rc"
}

trap cleanup EXIT HUP INT TERM

sleep 60 >"$TEST_ROOT/control-plane.out" 2>&1 &
CP_PID=$!
for _ in $(seq 1 20); do
  kill -0 "$CP_PID" 2>/dev/null && break
  sleep 0.05
done
kill -0 "$CP_PID" 2>/dev/null || fail 'synthetic control plane did not start'
CP_START_ID="$(proc_start_id "$CP_PID")"
[[ -n "$CP_START_ID" ]] || fail 'synthetic control plane has no start identity'

worker() {
  local trial="$1"
  local delay="$2"
  touch "$TEST_ROOT/$trial.active"
  local deadline=$((SECONDS + 10))
  while [[ ! -e "$TEST_ROOT/release" ]]; do
    [[ "$SECONDS" -lt "$deadline" ]] || return 1
    sleep 0.02
  done
  sleep "$delay"
  printf '%s\tpass\t%s\t%s\n' "$trial" "$CP_PID" "$CP_START_ID" \
    >"$TEST_ROOT/$trial.result"
  printf '%s\n' "$trial" >>"$TEST_ROOT/completion-order"
}

worker 1 0.25 &
PID_ONE=$!
worker 2 0.05 &
PID_TWO=$!

deadline=$((SECONDS + 10))
while [[ "$SECONDS" -lt "$deadline" ]]; do
  [[ "$(find "$TEST_ROOT" -maxdepth 1 -type f -name '*.active' | wc -l | tr -d ' ')" -eq 2 ]] \
    && break
  sleep 0.02
done
[[ "$(find "$TEST_ROOT" -maxdepth 1 -type f -name '*.active' | wc -l | tr -d ' ')" -eq 2 ]] \
  || fail 'bounded concurrency barrier did not observe both workers'
[[ "$(proc_start_id "$CP_PID")" == "$CP_START_ID" ]] \
  || fail 'control-plane start identity changed before release'

touch "$TEST_ROOT/release"
wait "$PID_ONE"
wait "$PID_TWO"
[[ "$(proc_start_id "$CP_PID")" == "$CP_START_ID" ]] \
  || fail 'control-plane start identity changed after workers completed'

printf 'trial\toutcome\tcontrol_plane_pid\tcontrol_plane_start_id\n' \
  >"$TEST_ROOT/ledger.tsv"
for trial in 1 2; do
  [[ -s "$TEST_ROOT/$trial.result" ]] || fail "worker $trial result is missing"
  cat "$TEST_ROOT/$trial.result" >>"$TEST_ROOT/ledger.tsv"
done
[[ "$(awk -F'\t' 'NR > 1 {print $1}' "$TEST_ROOT/ledger.tsv" | tr '\n' ' ')" == '1 2 ' ]] \
  || fail 'ledger was not emitted in deterministic trial order'
[[ "$(awk 'NR == 1 {print $1} NR == 2 {print $1}' "$TEST_ROOT/completion-order")" == $'2\n1' ]] \
  || fail 'synthetic workers did not complete out of order as required by the proof'
[[ "$(awk -F'\t' 'NR > 1 && $2 == "pass" {count++} END {print count + 0}' "$TEST_ROOT/ledger.tsv")" -eq 2 ]] \
  || fail 'ledger did not retain both pass rows'

echo 'E09 v2 HOST-SAFE scheduler test PASS: two bounded workers overlapped, completed out of order, and emitted a deterministic two-row ledger through one unchanged control-plane identity'

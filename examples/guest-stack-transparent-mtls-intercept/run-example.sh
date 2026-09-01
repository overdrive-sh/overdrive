#!/usr/bin/env bash
# Run the sole E07 product journey inside one `cargo xtask metal run --`
# lease. This script is operator runnable and prints only the public product
# observations consumed by the black-box expectation.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly EXAMPLE_DIR="$REPO_ROOT/examples/guest-stack-transparent-mtls-intercept"
readonly PREPARE="$EXAMPLE_DIR/prepare.sh"
readonly SESSION_LIFECYCLE="$EXAMPLE_DIR/session-lifecycle.sh"
readonly SESSION_WRAPPER="$EXAMPLE_DIR/session-wrapper.sh"
readonly OUTPUT_ROOT="/srv/vm/overdrive-testing/gti-e07"
readonly DATA_DIR="$OUTPUT_ROOT/data"
readonly CONFIG_DIR="$OUTPUT_ROOT/config"
readonly CREDS_DIR="$OUTPUT_ROOT/credentials"
readonly BIN="$REPO_ROOT/target/debug/overdrive"
readonly BIND="127.0.0.1:7643"
readonly CALLER_ID="gti-e07-caller"
readonly CALLEE_ID="gti-e07-callee"
readonly KEK_DESCRIPTION="overdrive:ca:kek:overdrive-ca-root"
readonly SESSION_READY_FILE="$OUTPUT_ROOT/session-wrapper.ready"
readonly SESSION_ACK_FILE="$OUTPUT_ROOT/session-wrapper.ack"
readonly SERVE_PID_FILE="$OUTPUT_ROOT/serve.pid"
readonly LEASE_OWNER="${OVERDRIVE_METAL_OWNER_PATH:-/run/lock/overdrive-metal-shared.owner}"

SERVE_PID=""
SERVE_PID_START=""
SERVE_PGID=""
SESSION_WRAPPER_PID=""
SESSION_WRAPPER_START=""
SESSION_WRAPPER_PGID=""
SESSION_GROUP_OWNED=0
SESSION_WRAPPER_REAPED=0
SESSION_LAUNCH_TOKEN=""
LAUNCH_INTERRUPTED=0
PREPARE_ATTEMPTED=0
PREPARE_TOKEN=""
CALLER_DEPLOYED=0
CALLEE_DEPLOYED=0
CLEANUP_FAILED=0

# shellcheck source=SCRIPTDIR/session-lifecycle.sh
source "$SESSION_LIFECYCLE"

die() {
  echo "gti-e07 run: $*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command is unavailable: $1"
}

bounded() {
  local duration="$1"
  shift
  timeout --foreground --signal=TERM --kill-after=5s "$duration" "$@"
}

stop_exact_workload() {
  local id="$1"
  local output="$OUTPUT_ROOT/${id}-stop.log"
  echo "=== built overdrive job stop $id ==="
  if bounded 20s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" job stop "$id" >"$output" 2>&1; then
    cat "$output"
    return 0
  fi
  cat "$output" >&2
  return 1
}

serve_pid_is_exact() {
  local identity state pgid start
  [[ -n "$SERVE_PID" && -n "$SERVE_PID_START" && -e "/proc/$SERVE_PID/exe" ]] \
    || return 1
  identity="$(e07_session_process_identity "$SERVE_PID")" || return 1
  read -r state pgid start <<<"$identity"
  [[ "$start" == "$SERVE_PID_START" && "$pgid" == "$SERVE_PGID" ]] || return 1
  [[ "$(readlink -f "/proc/$SERVE_PID/exe")" == "$(readlink -f "$BIN")" ]]
}

serve_pid_belongs_to_owned_group() {
  local identity state pgid start
  [[ "$SESSION_GROUP_OWNED" -eq 1 && -n "$SERVE_PID_START" ]] || return 1
  identity="$(e07_session_process_identity "$SERVE_PID")" || return 1
  read -r state pgid start <<<"$identity"
  [[ "$start" == "$SERVE_PID_START" \
    && "$pgid" == "$SERVE_PGID" \
    && "$pgid" == "$SESSION_WRAPPER_PGID" ]]
}

terminate_serve() {
  [[ -n "$SERVE_PID" || -n "$SESSION_WRAPPER_PID" ]] || return 0
  local failed=0
  if [[ -n "$SERVE_PID" ]] && e07_session_process_identity "$SERVE_PID" >/dev/null; then
    if serve_pid_is_exact; then
      : # Exact product identity is proven; the owned group handles signalling.
    elif serve_pid_belongs_to_owned_group; then
      : # Published but pre-exec: the owned group is the safe termination unit.
    else
      echo "gti-e07 run: recorded serve PID no longer belongs to the owned launch group; refusing to signal it" >&2
      failed=1
    fi
  fi
  if ! e07_session_terminate_owned_unit; then
    echo "gti-e07 run: fresh-session launch unit did not exit within its TERM/KILL bounds" >&2
    failed=1
  fi
  SERVE_PID=""
  SERVE_PID_START=""
  SERVE_PGID=""
  SESSION_WRAPPER_PID=""
  SESSION_WRAPPER_START=""
  SESSION_WRAPPER_PGID=""
  SESSION_GROUP_OWNED=0
  # shellcheck disable=SC2034 # consumed by sourced lifecycle library
  SESSION_WRAPPER_REAPED=0
  return "$failed"
}

cleanup() {
  local incoming_rc=$?
  trap - EXIT HUP INT TERM

  if [[ "$CALLER_DEPLOYED" -eq 1 ]] && ! stop_exact_workload "$CALLER_ID"; then
    echo "gti-e07 run: public stop failed for $CALLER_ID" >&2
    CLEANUP_FAILED=1
  fi
  # The current public stop endpoint is exposed by the `job stop` command but
  # accepts the canonical WorkloadId for this Service as well.
  if [[ "$CALLEE_DEPLOYED" -eq 1 ]] && ! stop_exact_workload "$CALLEE_ID"; then
    echo "gti-e07 run: public stop failed for $CALLEE_ID" >&2
    CLEANUP_FAILED=1
  fi
  if ! terminate_serve; then
    CLEANUP_FAILED=1
  fi

  if [[ "$PREPARE_ATTEMPTED" -eq 1 \
    && -f "$OUTPUT_ROOT/.gti-e07-owned" \
    && "$(<"$OUTPUT_ROOT/.gti-e07-owned")" == "gti-e07-owned-v1:$PREPARE_TOKEN" ]]; then
    bounded 45s env GTI_E07_OWNERSHIP_TOKEN="$PREPARE_TOKEN" \
      "$PREPARE" cleanup || CLEANUP_FAILED=1
  fi

  if [[ "$PREPARE_ATTEMPTED" -eq 1 && -e "$OUTPUT_ROOT" ]]; then
    echo "gti-e07 run: preparation output remains or is not proven owned by this invocation: $OUTPUT_ROOT" >&2
    CLEANUP_FAILED=1
  fi

  if [[ "$incoming_rc" -eq 0 && "$CLEANUP_FAILED" -ne 0 ]]; then
    exit 1
  fi
  exit "$incoming_rc"
}

require_native_metal() {
  [[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]] \
    || die "runtime is restricted to native x86_64 Linux metal"
  [[ "$(id -u)" -eq 0 ]] || die "run through cargo xtask metal run -- (root context)"
  require_command systemd-detect-virt
  local virt rc
  set +e
  virt="$(systemd-detect-virt 2>/dev/null)"
  rc=$?
  set -e
  [[ "$rc" -eq 1 && "$virt" == "none" ]] \
    || die "virtualized or unknown substrate refused (${virt:-unknown}, status=$rc)"
  [[ -c /dev/kvm ]] || die "/dev/kvm is not an accessible character device"
  grep -qE '^flags.*\b(vmx|svm)\b' /proc/cpuinfo \
    || die "CPU exposes neither VMX nor SVM"
  ! grep -qw hypervisor /proc/cpuinfo || die "CPU hypervisor flag is present"
  [[ -r "$LEASE_OWNER" ]] || die "canonical metal lease metadata is absent"
  grep -qx 'action=run' "$LEASE_OWNER" \
    || die "canonical metal lease is not held for a Run action"
}

wait_for_serve() {
  local config_file="$CONFIG_DIR/.overdrive/config"
  local attempts=60
  while [[ "$attempts" -gt 0 ]]; do
    if [[ -f "$config_file" ]] && serve_pid_is_exact; then
      return 0
    fi
    serve_pid_belongs_to_owned_group || return 1
    sleep 0.5
    attempts=$((attempts - 1))
  done
  return 1
}

read_serve_identity() {
  local token pid pgid start extra=""
  read -r token pid pgid start extra <"$SERVE_PID_FILE" || return 1
  [[ -z "$extra" \
    && "$token" == "$SESSION_LAUNCH_TOKEN" \
    && "$pid" =~ ^[0-9]+$ \
    && "$pgid" == "$SESSION_WRAPPER_PGID" \
    && "$start" =~ ^[0-9]+$ ]] || return 1
  SERVE_PID="$pid"
  SERVE_PGID="$pgid"
  SERVE_PID_START="$start"
  serve_pid_belongs_to_owned_group
}

launch_fresh_session_serve() {
  local launched_pid capture_ticks=10
  LAUNCH_INTERRUPTED=0
  # Defer exit during the two-command `$!` handoff. Bash delivers the trap, it
  # records intent, and launch proceeds just far enough to make the direct
  # child addressable before the normal exit-130 trap is restored.
  trap 'LAUNCH_INTERRUPTED=1' HUP INT TERM
  setsid "$SESSION_WRAPPER" "$SESSION_LAUNCH_TOKEN" "$SESSION_READY_FILE" \
    "$SESSION_ACK_FILE" "$SERVE_PID_FILE" "$KEK_DESCRIPTION" "$CONFIG_DIR" \
    "$CREDS_DIR" "$BIN" "$BIND" "$DATA_DIR" \
    >"$OUTPUT_ROOT/serve.log" 2>&1 &
  launched_pid=$!
  SESSION_WRAPPER_PID="$launched_pid"
  while [[ -z "$SESSION_WRAPPER_START" && "$capture_ticks" -gt 0 ]]; do
    e07_session_capture_wrapper "$launched_pid" && break
    kill -0 "$launched_pid" 2>/dev/null || break
    sleep 0.01
    capture_ticks=$((capture_ticks - 1))
  done
  trap 'exit 130' HUP INT TERM
  [[ "$LAUNCH_INTERRUPTED" -eq 0 ]] || return 130
  [[ -n "$SESSION_WRAPPER_START" ]] \
    || die "fresh-session wrapper exited before ownership could be recorded"

  e07_session_wait_for_private_group 50 \
    || die "fresh-session wrapper did not establish its private process group within five seconds"
  e07_session_acknowledge_ownership \
    || die "fresh-session wrapper ownership could not be acknowledged"
  e07_session_wait_for_ready 50 \
    || die "fresh-session wrapper did not publish readiness within five seconds"

  local pid_attempts=50
  while [[ ! -s "$SERVE_PID_FILE" && "$pid_attempts" -gt 0 ]]; do
    e07_session_unit_is_live \
      || die "fresh-session wrapper exited before recording the serve PID"
    sleep 0.1
    pid_attempts=$((pid_attempts - 1))
  done
  [[ -s "$SERVE_PID_FILE" ]] \
    || die "fresh-session serve did not publish its PID within five seconds"
  read_serve_identity \
    || die "fresh-session serve published an invalid or foreign PID identity"
}

first_service_alloc_state() {
  awk '
    /^Alloc[[:space:]]+State[[:space:]]+/ { in_table=1; next }
    in_table && /^[-[:space:]]+$/ { next }
    in_table && $1 ~ /^alloc-/ { print $2; exit }
  '
}

first_job_attempt_state() {
  awk '
    /^Attempt[[:space:]]+State[[:space:]]+/ { in_table=1; next }
    in_table && /^[-[:space:]]+$/ { next }
    in_table && $1 ~ /^[0-9]+$/ { print $2; exit }
  '
}

check_render_parsers() {
  local service_render job_render
  service_render="Service 'gti-e07-callee' (kind: Service)
Replicas (desired/running): 1/1
Alloc                    State        Restarts   Since
alloc-gti-e07-callee-0   Running      0          (c=1,w=local)"
  job_render="Job 'gti-e07-caller' (kind: Job)
Verdict: Succeeded

Attempt  State        Exit   Started              Duration
1        Terminated   0      (c=2,w=local)        1s"
  [[ "$(first_service_alloc_state <<<"$service_render")" == "Running" ]] \
    || die "Service describe parser does not recognize Alloc/State=Running"
  [[ "$(first_job_attempt_state <<<"$job_render")" == "Terminated" ]] \
    || die "Job describe parser does not recognize Attempt/State=Terminated"
  [[ -z "$(first_job_attempt_state <<<"$service_render")" ]] \
    || die "Job describe parser accepted a Service table"
  [[ -z "$(first_service_alloc_state <<<"$job_render")" ]] \
    || die "Service describe parser accepted a Job table"
}

wait_for_service_running() {
  local out="$1"
  local deadline=$((SECONDS + 90))
  local state=""
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
      "$BIN" workload describe "$CALLEE_ID" >"$out" 2>&1 || true
    state="$(first_service_alloc_state <"$out")"
    if [[ "$state" == "Running" ]] \
      && grep -Fxq 'Replicas (desired/running): 1/1' "$out"; then
      return 0
    fi
    [[ "$state" == "Failed" || "$state" == "Terminated" ]] && return 1
    sleep 0.5
  done
  echo "gti-e07 run: $CALLEE_ID did not reach Alloc/State=Running with replicas 1/1 (last=${state:-none})" >&2
  return 1
}

wait_for_job_succeeded() {
  local id="$1"
  local timeout_seconds="$2"
  local out="$3"
  local deadline=$((SECONDS + timeout_seconds))
  local state=""
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
      "$BIN" workload describe "$id" >"$out" 2>&1 || true
    state="$(first_job_attempt_state <"$out")"
    if [[ "$state" == "Terminated" ]] && grep -Fq 'Verdict: Succeeded' "$out"; then
      return 0
    fi
    [[ "$state" == "Failed" || "$state" == "Stopped" ]] && return 1
    sleep 0.5
  done
  echo "gti-e07 run: $id did not reach Terminated with Verdict: Succeeded (last=${state:-none})" >&2
  return 1
}

run() {
  require_native_metal
  local command
  for command in awk cargo file grep mv rustc setsid systemd-detect-virt timeout; do
    require_command "$command"
  done
  "$PREPARE" check-source
  check_render_parsers
  trap cleanup EXIT
  trap 'exit 130' HUP INT TERM

  bounded 600s cargo build -p overdrive-cli --bin overdrive
  [[ -x "$BIN" ]] || die "default-feature product binary was not built: $BIN"
  require_command keyctl
  require_command readlink
  [[ -r /proc/sys/kernel/random/uuid ]] \
    || die "kernel UUID source is unavailable for per-run fixture ownership"
  [[ ! -e "$OUTPUT_ROOT" ]] \
    || die "$OUTPUT_ROOT already exists; refusing to overwrite or clean pre-existing state"
  IFS= read -r PREPARE_TOKEN </proc/sys/kernel/random/uuid
  [[ "$PREPARE_TOKEN" =~ ^[A-Fa-f0-9-]+$ ]] \
    || die "kernel UUID source returned an invalid ownership token"
  PREPARE_ATTEMPTED=1
  SESSION_LAUNCH_TOKEN="$PREPARE_TOKEN"
  bounded 240s env GTI_E07_OWNERSHIP_TOKEN="$PREPARE_TOKEN" "$PREPARE" prepare
  bounded 45s env GTI_E07_OWNERSHIP_TOKEN="$PREPARE_TOKEN" "$PREPARE" check

  launch_fresh_session_serve
  wait_for_serve || die "serve did not become ready within 30 seconds"

  CALLEE_DEPLOYED=1
  bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" deploy --detach "$EXAMPLE_DIR/callee.toml"
  wait_for_service_running "$OUTPUT_ROOT/callee-describe.log" \
    || die "callee did not reach public Alloc/State=Running"
  echo "=== built overdrive workload describe $CALLEE_ID ==="
  cat "$OUTPUT_ROOT/callee-describe.log"

  CALLER_DEPLOYED=1
  bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" deploy --detach "$EXAMPLE_DIR/caller.toml"
  wait_for_job_succeeded "$CALLER_ID" 120 "$OUTPUT_ROOT/caller-describe.log" \
    || die "caller did not reach its successful terminal state"
  echo "=== built overdrive workload describe $CALLER_ID ==="
  cat "$OUTPUT_ROOT/caller-describe.log"
  grep -Fq 'Verdict: Succeeded' "$OUTPUT_ROOT/caller-describe.log" \
    || die "caller terminal result was not successful"

  echo "E07 PASS: one VM Job called one Exec Service and received the exact reply"
}

case "${1:-run}" in
  check-source)
    "$PREPARE" check-source
    check_render_parsers
    ;;
  run) run ;;
  *) die "usage: run-example.sh [check-source|run]" ;;
esac

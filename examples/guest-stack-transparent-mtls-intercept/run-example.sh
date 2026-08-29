#!/usr/bin/env bash
# Run the sole E07 product journey inside one `cargo xtask metal run --`
# lease. This script is operator runnable; the E07 expectation runner remains
# fail-closed until DELIVER regenerates and implements its evidence capture.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly EXAMPLE_DIR="$REPO_ROOT/examples/guest-stack-transparent-mtls-intercept"
readonly PREPARE="$EXAMPLE_DIR/prepare.sh"
readonly OUTPUT_ROOT="/srv/vm/overdrive-testing/gti-e07"
readonly DATA_DIR="$OUTPUT_ROOT/data"
readonly CONFIG_DIR="$OUTPUT_ROOT/config"
readonly CREDS_DIR="$OUTPUT_ROOT/credentials"
readonly BIN="$REPO_ROOT/target/debug/overdrive"
readonly BIND="127.0.0.1:7643"
readonly CALLER_ID="gti-e07-caller"
readonly CALLEE_ID="gti-e07-callee"
readonly KEK_DESCRIPTION="overdrive:ca:kek:overdrive-ca-root"
readonly SERVE_PID_FILE="$OUTPUT_ROOT/serve.pid"
readonly LEASE_OWNER="${OVERDRIVE_METAL_OWNER_PATH:-/run/lock/overdrive-metal-shared.owner}"

SERVE_PID=""
SESSION_WRAPPER_PID=""
PREPARE_ATTEMPTED=0
PREPARE_TOKEN=""
CALLER_DEPLOYED=0
CALLEE_DEPLOYED=0
CLEANUP_FAILED=0

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
  bounded 20s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" job stop "$id" >"$OUTPUT_ROOT/${id}-stop.log" 2>&1
}

serve_pid_is_exact() {
  [[ -n "$SERVE_PID" && -e "/proc/$SERVE_PID/exe" ]] || return 1
  [[ "$(readlink -f "/proc/$SERVE_PID/exe")" == "$(readlink -f "$BIN")" ]]
}

serve_pid_is_live() {
  kill -0 "$SERVE_PID" 2>/dev/null || return 1
  [[ -r "/proc/$SERVE_PID/stat" ]] || return 1
  [[ "$(awk '{print $3}' "/proc/$SERVE_PID/stat")" != "Z" ]]
}

terminate_serve() {
  [[ -n "$SERVE_PID" || -n "$SESSION_WRAPPER_PID" ]] || return 0
  if serve_pid_is_live; then
    if ! serve_pid_is_exact; then
      echo "gti-e07 run: refusing to signal PID $SERVE_PID because it is not the started overdrive binary" >&2
      CLEANUP_FAILED=1
      return 1
    fi
    kill -TERM "$SERVE_PID" 2>/dev/null || CLEANUP_FAILED=1
    local remaining=50
    while serve_pid_is_live && [[ "$remaining" -gt 0 ]]; do
      sleep 0.1
      remaining=$((remaining - 1))
    done
    if serve_pid_is_live; then
      serve_pid_is_exact \
        && kill -KILL "$SERVE_PID" 2>/dev/null \
        || CLEANUP_FAILED=1
    fi
  fi
  if [[ -n "$SESSION_WRAPPER_PID" ]]; then
    wait "$SESSION_WRAPPER_PID" 2>/dev/null || true
  fi
  SERVE_PID=""
  SESSION_WRAPPER_PID=""
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
    kill -0 "$SERVE_PID" 2>/dev/null || return 1
    sleep 0.5
    attempts=$((attempts - 1))
  done
  return 1
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

wait_for_job_state() {
  local id="$1"
  local wanted="$2"
  local timeout_seconds="$3"
  local out="$4"
  local deadline=$((SECONDS + timeout_seconds))
  local state=""
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
      "$BIN" workload describe "$id" >"$out" 2>&1 || true
    state="$(first_job_attempt_state <"$out")"
    [[ "$state" == "$wanted" ]] && return 0
    [[ "$state" == "Failed" || "$state" == "Stopped" ]] && return 1
    sleep 0.5
  done
  echo "gti-e07 run: $id did not reach $wanted (last=${state:-none})" >&2
  return 1
}

run() {
  require_native_metal
  local command
  for command in awk cargo file grep rustc systemd-detect-virt timeout; do
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
  bounded 240s env GTI_E07_OWNERSHIP_TOKEN="$PREPARE_TOKEN" "$PREPARE" prepare
  bounded 45s env GTI_E07_OWNERSHIP_TOKEN="$PREPARE_TOKEN" "$PREPARE" check

  # shellcheck disable=SC2016 # positional parameters expand in the child shell
  keyctl session - bash -c '
    set -euo pipefail
    description="$1"
    pid_file="$2"
    config_dir="$3"
    creds_dir="$4"
    bin="$5"
    bind="$6"
    data_dir="$7"
    keyctl describe @s >/dev/null \
      || { echo "gti-e07 run: fresh session keyring is not accessible" >&2; exit 70; }
    set +e
    keyctl search @s user "$description" >/dev/null 2>&1
    search_rc=$?
    set -e
    case "$search_rc" in
      0)
        echo "gti-e07 run: fresh session unexpectedly contains $description" >&2
        exit 70
        ;;
      1) ;;
      *)
        echo "gti-e07 run: could not verify absence of $description (keyctl exit $search_rc)" >&2
        exit 70
        ;;
    esac
    printf "%s\n" "$$" >"$pid_file"
    exec env OVERDRIVE_CONFIG_DIR="$config_dir" CREDENTIALS_DIRECTORY="$creds_dir" \
      "$bin" serve --bind "$bind" --data-dir "$data_dir"
  ' gti-e07-session "$KEK_DESCRIPTION" "$SERVE_PID_FILE" "$CONFIG_DIR" \
    "$CREDS_DIR" "$BIN" "$BIND" "$DATA_DIR" \
    >"$OUTPUT_ROOT/serve.log" 2>&1 &
  SESSION_WRAPPER_PID=$!
  local pid_attempts=50
  while [[ ! -s "$SERVE_PID_FILE" && "$pid_attempts" -gt 0 ]]; do
    kill -0 "$SESSION_WRAPPER_PID" 2>/dev/null \
      || die "fresh-session serve wrapper exited before recording the serve PID"
    sleep 0.1
    pid_attempts=$((pid_attempts - 1))
  done
  [[ -s "$SERVE_PID_FILE" ]] || die "fresh-session serve did not record its PID"
  IFS= read -r SERVE_PID <"$SERVE_PID_FILE"
  [[ "$SERVE_PID" =~ ^[0-9]+$ ]] || die "fresh-session serve recorded an invalid PID"
  wait_for_serve || die "serve did not become ready within 30 seconds"

  CALLEE_DEPLOYED=1
  bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" deploy --detach "$EXAMPLE_DIR/callee.toml"
  wait_for_service_running "$OUTPUT_ROOT/callee-describe.log" \
    || die "callee did not reach public Alloc/State=Running"

  CALLER_DEPLOYED=1
  bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" deploy --detach "$EXAMPLE_DIR/caller.toml"
  wait_for_job_state "$CALLER_ID" Terminated 120 "$OUTPUT_ROOT/caller-describe.log" \
    || die "caller did not reach its successful terminal state"
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

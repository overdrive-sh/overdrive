#!/usr/bin/env bash
# Run the checked-in E08 journey on its qualified native-metal substrate.
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly EXAMPLE_DIR
readonly PREPARE="$EXAMPLE_DIR/prepare.sh"
readonly OUTPUT_ROOT="${SVM_E08_OUTPUT_ROOT:-/srv/vm/overdrive-testing/svm-e08}"
readonly CONFIG_DIR="$OUTPUT_ROOT/config"
readonly DATA_DIR="$OUTPUT_ROOT/data"
readonly CREDS_DIR="$OUTPUT_ROOT/credentials"
readonly BIN="$(cd "$EXAMPLE_DIR/../.." && pwd)/target/debug/overdrive"
readonly BIND="127.0.0.1:7644"
SERVICE_ID="service-vm-e08"
CLIENT_ID="service-vm-e08-client"
readonly KEK_DESCRIPTION="overdrive:ca:kek:overdrive-ca-root"

SERVE_PID=""
PREPARED=0
SERVICE_DEPLOYED=0
CLIENT_DEPLOYED=0
SNAPSHOT_DIR=""
CASE_CLEANUP_POLICY="operator-stop"

die() {
  echo "svm-e08 run: $*" >&2
  exit 1
}

bounded() {
  local duration="$1"
  shift
  timeout --foreground --signal=TERM --kill-after=5s "$duration" "$@"
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command is unavailable: $1"
}

require_native_metal() {
  [[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]] \
    || die "runtime is restricted to native x86_64 Linux metal"
  [[ "$(id -u)" -eq 0 ]] || die "run through cargo xtask metal run -- (root context)"
  [[ -c /dev/kvm ]] || die "/dev/kvm is not an accessible character device"
  command -v cloud-hypervisor >/dev/null 2>&1 \
    || die "cloud-hypervisor is unavailable"
}

probe_hypervisors() {
  for proc in /proc/[0-9]*; do
    [[ -r "$proc/cmdline" ]] || continue
    local argv0=''
    IFS= read -r -d '' argv0 <"$proc/cmdline" || true
    [[ "$(basename "$argv0" 2>/dev/null || true)" == "cloud-hypervisor" ]] \
      && basename "$proc" || true
  done | sort -n
}

probe_scopes() {
  find /sys/fs/cgroup/overdrive.slice/workloads.slice -maxdepth 1 -mindepth 1 \
    -type d -name 'alloc-*.scope' -printf '%f\n' 2>/dev/null | sort
}

probe_run_dirs() {
  find /run/overdrive/vm -maxdepth 1 -mindepth 1 -type d -printf '%f\n' \
    2>/dev/null | sort
}

probe_network() {
  { ip -br link show; ip netns list; } 2>/dev/null | sort
}

probe_loops() {
  losetup -j "$OUTPUT_ROOT/rootfs.ext4" 2>/dev/null || true
}

probe_mounts() {
  findmnt --raw --noheadings --output TARGET,SOURCE 2>/dev/null | \
    grep -F "$OUTPUT_ROOT" || true
}

snapshot_before() {
  SNAPSHOT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/svm-e08-snapshot.XXXXXX")"
  probe_hypervisors >"$SNAPSHOT_DIR/hypervisors"
  probe_scopes >"$SNAPSHOT_DIR/scopes"
  probe_run_dirs >"$SNAPSHOT_DIR/run-dirs"
  probe_network >"$SNAPSHOT_DIR/network"
  probe_loops >"$SNAPSHOT_DIR/loops"
  probe_mounts >"$SNAPSHOT_DIR/mounts"
}

new_delta_count() {
  local before="$1"
  shift
  comm -13 "$before" <("$@") | sed '/^$/d' | wc -l | tr -d ' '
}

stop_workload() {
  local id="$1"
  local output="$OUTPUT_ROOT/${id}-stop.log"
  bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" job stop "$id" >"$output" 2>&1 || {
      cat "$output" >&2
      return 1
  }
  local describe="$OUTPUT_ROOT/${id}-stopped.log"
  if ! wait_for_workload_stop "$id" "$describe"; then
    cat "$describe" >&2
    return 1
  fi
  cat "$output"
}

request_stop_intent() {
  local id="$1"
  local stop_output="$OUTPUT_ROOT/${id}-preserve-stop.log"
  bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" job stop "$id" >"$stop_output" 2>&1 || {
      cat "$stop_output" >&2
      return 1
  }
}

preserve_failed_service() {
  local id="$1"
  request_stop_intent "$id"
  local stop_output="$OUTPUT_ROOT/${id}-preserve-stop.log"
  local output="$OUTPUT_ROOT/${id}-preserved-failure.log"
  local deadline=$((SECONDS + 45))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
      "$BIN" workload describe "$id" >"$output" 2>&1 || true
    local state
    state="$(first_service_alloc_state <"$output")"
    if [[ "$state" == "Failed" ]] \
      && grep -Fq 'StartupProbeFailed' "$output" \
      && owned_runtime_resources_released; then
      cat "$stop_output"
      cat "$output"
      return 0
    fi
    if [[ "$state" == "Failed" ]] \
      && grep -Fq 'reason: driver internal error:' "$output" \
      && grep -Fq 'bind beacon listener: Address already in use' "$output" \
      && owned_runtime_resources_released; then
      # A VM replacement can be rejected before it creates a second VMM when
      # the original accepted session still owns the allocation-derived beacon
      # pathname. Preserve that typed start rejection as a distinct trajectory;
      # it is not an arbitrary crash or a reclassification of the probe result.
      cat "$stop_output"
      cat "$output"
      return 0
    fi
    # If a replacement reached Running before the stop intent was observed,
    # its ordinary operator stop may author Terminated. Accept that distinct
    # trajectory only with public evidence of a replacement restart and an
    # operator-attributed prior termination; a bare crash-shaped Terminated
    # row is not an acceptable startup-failure outcome.
    if [[ "$state" == "Terminated" ]] \
      && [[ "$(first_service_restart_count <"$output")" =~ ^[1-9][0-9]*$ ]] \
      && grep -Eqi 'last terminated: .*stopped \(by operator\)' "$output" \
      && grep -Fq 'reason: stopped' "$output" \
      && owned_runtime_resources_released; then
      cat "$stop_output"
      cat "$output"
      return 0
    fi
    sleep 1
  done
  cat "$stop_output" >&2
  cat "$output" >&2
  echo "svm-e08 run: startup-failed Service did not remain Failed after stop intent: $id" >&2
  return 1
}

stop_serve() {
  [[ -n "$SERVE_PID" ]] || return 0
  if kill -0 "$SERVE_PID" 2>/dev/null; then
    kill -- "-$SERVE_PID" 2>/dev/null || kill "$SERVE_PID" 2>/dev/null || true
  fi
  local attempts=50
  while kill -0 "$SERVE_PID" 2>/dev/null && [[ "$attempts" -gt 0 ]]; do
    sleep 0.1
    attempts=$((attempts - 1))
  done
  if kill -0 "$SERVE_PID" 2>/dev/null; then
    kill -KILL -- "-$SERVE_PID" 2>/dev/null || kill -KILL "$SERVE_PID" 2>/dev/null || true
    wait "$SERVE_PID" 2>/dev/null || true
    die "owned serve process did not stop within the bounded grace period"
  fi
  wait "$SERVE_PID" 2>/dev/null || true
  SERVE_PID=""
}

report_cleanup_deltas() {
  local vms scopes networks run_dirs loops mounts prep probe_tasks
  vms="$(new_delta_count "$SNAPSHOT_DIR/hypervisors" probe_hypervisors)"
  scopes="$(new_delta_count "$SNAPSHOT_DIR/scopes" probe_scopes)"
  networks="$(new_delta_count "$SNAPSHOT_DIR/network" probe_network)"
  run_dirs="$(new_delta_count "$SNAPSHOT_DIR/run-dirs" probe_run_dirs)"
  loops="$(new_delta_count "$SNAPSHOT_DIR/loops" probe_loops)"
  mounts="$(new_delta_count "$SNAPSHOT_DIR/mounts" probe_mounts)"
  [[ -e "$OUTPUT_ROOT" ]] && prep=1 || prep=0
  [[ -n "$SERVE_PID" ]] && probe_tasks=1 || probe_tasks=0
  printf 'E08 teardown deltas: vm=%s probe=%s network=%s cgroup=%s run-directory=%s mount=%s loop=%s preparation=%s\n' \
    "$vms" "$probe_tasks" "$networks" "$scopes" "$run_dirs" "$mounts" "$loops" "$prep"
  if [[ "$networks" -ne 0 ]]; then
    printf 'E08 unexpected network delta:\n' >&2
    comm -13 "$SNAPSHOT_DIR/network" <(probe_network) >&2
  fi
  [[ "$vms" -eq 0 && "$probe_tasks" -eq 0 && "$networks" -eq 0 \
    && "$scopes" -eq 0 && "$run_dirs" -eq 0 && "$mounts" -eq 0 \
    && "$loops" -eq 0 && "$prep" -eq 0 ]]
}

owned_runtime_resources_released() {
  local vms scopes networks run_dirs loops mounts
  vms="$(new_delta_count "$SNAPSHOT_DIR/hypervisors" probe_hypervisors)"
  scopes="$(new_delta_count "$SNAPSHOT_DIR/scopes" probe_scopes)"
  networks="$(new_delta_count "$SNAPSHOT_DIR/network" probe_network)"
  run_dirs="$(new_delta_count "$SNAPSHOT_DIR/run-dirs" probe_run_dirs)"
  loops="$(new_delta_count "$SNAPSHOT_DIR/loops" probe_loops)"
  mounts="$(new_delta_count "$SNAPSHOT_DIR/mounts" probe_mounts)"
  [[ "$vms" -eq 0 && "$scopes" -eq 0 && "$networks" -eq 0 \
    && "$run_dirs" -eq 0 && "$loops" -eq 0 && "$mounts" -eq 0 ]]
}

wait_for_owned_runtime_cleanup() {
  local deadline=$((SECONDS + 45))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    owned_runtime_resources_released && return 0
    sleep 1
  done
  return 1
}

cleanup() {
  local incoming_rc=$?
  trap - EXIT HUP INT TERM
  local failed=0
  [[ "$CLIENT_DEPLOYED" -eq 0 ]] || stop_workload "$CLIENT_ID" || failed=1
  [[ "$SERVICE_DEPLOYED" -eq 0 ]] || stop_workload "$SERVICE_ID" || failed=1
  wait_for_owned_runtime_cleanup || failed=1
  stop_serve || failed=1
  [[ "$PREPARED" -eq 0 ]] || bounded 45s "$PREPARE" cleanup || failed=1
  if [[ -n "$SNAPSHOT_DIR" ]]; then
    report_cleanup_deltas || failed=1
    rm -rf -- "$SNAPSHOT_DIR"
  fi
  [[ "$incoming_rc" -ne 0 || "$failed" -eq 0 ]] || exit 1
  exit "$incoming_rc"
}

first_service_alloc_state() {
  awk '
    /^Alloc[[:space:]]+State[[:space:]]+/ { in_table = 1; next }
    in_table && /^[-[:space:]]+$/ { next }
    in_table && $1 ~ /^alloc-/ { print $2; exit }
  '
}

first_service_restart_count() {
  awk '
    /^Alloc[[:space:]]+State[[:space:]]+/ { in_table = 1; next }
    in_table && /^[-[:space:]]+$/ { next }
    in_table && $1 ~ /^alloc-/ { print $3; exit }
  '
}

first_job_attempt_state() {
  awk '
    /^Attempt[[:space:]]+State[[:space:]]+/ { in_table = 1; next }
    in_table && /^[-[:space:]]+$/ { next }
    in_table && $1 ~ /^[0-9]+$/ { print $2; exit }
  '
}

first_workload_state() {
  awk '
    /^Alloc[[:space:]]+State[[:space:]]+/ { in_table = 1; next }
    /^Attempt[[:space:]]+State[[:space:]]+/ { in_table = 1; next }
    in_table && /^[-[:space:]]+$/ { next }
    in_table && ($1 ~ /^alloc-/ || $1 ~ /^[0-9]+$/) { print $2; exit }
  '
}

wait_for_workload_stop() {
  local id="$1"
  local output="$2"
  local deadline=$((SECONDS + 45))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
      "$BIN" workload describe "$id" >"$output" 2>&1 || true
    [[ "$(first_workload_state <"$output")" == "Terminated" ]] && return 0
    sleep 1
  done
  return 1
}

wait_for_service_observations() {
  local output="$1"
  local deadline=$((SECONDS + 90))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
      "$BIN" workload describe "$SERVICE_ID" >"$output" 2>&1 || true
    if [[ "$(first_service_alloc_state <"$output")" == "Running" ]] \
      && grep -Eqi 'startup.*tcp.*18081.*last=pass|tcp.*18081.*startup.*last=pass' "$output" \
      && grep -Eqi 'readiness.*http.*18080.*last=pass|http.*18080.*readiness.*last=pass' "$output"; then
      return 0
    fi
    sleep 1
  done
  return 1
}

wait_for_client_success() {
  local output="$1"
  local deadline=$((SECONDS + 120))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
      "$BIN" workload describe "$CLIENT_ID" >"$output" 2>&1 || true
    if [[ "$(first_job_attempt_state <"$output")" == "Terminated" ]] \
      && grep -Fq 'Verdict: Succeeded' "$output"; then
      return 0
    fi
    sleep 1
  done
  return 1
}

wait_for_serve() {
  local attempts=60
  while [[ "$attempts" -gt 0 ]]; do
    [[ -f "$CONFIG_DIR/.overdrive/config" ]] && kill -0 "$SERVE_PID" 2>/dev/null && return 0
    kill -0 "$SERVE_PID" 2>/dev/null || return 1
    sleep 0.5
    attempts=$((attempts - 1))
  done
  return 1
}

run_healthy() {
  require_native_metal
  local command
  for command in awk cargo cloud-hypervisor find findmnt grep ip keyctl losetup mktemp \
    script setsid sort timeout tr; do
    require_command "$command"
  done
  "$PREPARE" check-source
  [[ ! -e "$OUTPUT_ROOT" ]] || die "refusing to overwrite pre-existing materialization: $OUTPUT_ROOT"
  snapshot_before
  trap cleanup EXIT
  trap 'exit 130' HUP INT TERM

  bounded 600s cargo build -p overdrive-cli --bin overdrive
  [[ -x "$BIN" ]] || die "default-feature product binary was not built: $BIN"
  PREPARED=1
  bounded 240s "$PREPARE" prepare
  bounded 45s "$PREPARE" check

  setsid keyctl session - env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    CREDENTIALS_DIRECTORY="$CREDS_DIR" "$BIN" serve --bind "$BIND" --data-dir "$DATA_DIR" \
    >"$OUTPUT_ROOT/serve.log" 2>&1 &
  SERVE_PID=$!
  wait_for_serve || die "serve did not become ready within 30 seconds"

  local service_stream="$OUTPUT_ROOT/service-stream.log"
  SERVICE_DEPLOYED=1
  local service_command
  printf -v service_command 'env OVERDRIVE_CONFIG_DIR=%q %q deploy %q' \
    "$CONFIG_DIR" "$BIN" "$EXAMPLE_DIR/service.toml"
# `overdrive deploy` selects Service event stream only when stdout is a
# terminal. Record this one fresh, un-detached deployment through the
# platform's PTY so Accepted and Stable are captured from the same invocation.
  if ! bounded 150s script -q -e -c "$service_command" "$service_stream" >/dev/null; then
    cat "$service_stream" >&2
    cat "$OUTPUT_ROOT/serve.log" >&2
    die "Service streaming deployment did not complete"
  fi
  local accepted stable
  accepted="$(grep -n -m1 'Accepted' "$service_stream" | cut -d: -f1 || true)"
  stable="$(grep -ni -m1 'stable' "$service_stream" | cut -d: -f1 || true)"
  if [[ -z "$accepted" || -z "$stable" || "$accepted" -ge "$stable" ]]; then
    cat "$service_stream" >&2
    die "single service deployment did not render Accepted before Stable"
  fi
  grep -Eq 'startup.*(index|probe).*0|probe.*0.*startup' "$service_stream" \
    || die "Stable render did not name startup probe index 0"
  cat "$service_stream"

  local service_describe="$OUTPUT_ROOT/service-describe.log"
  if ! wait_for_service_observations "$service_describe"; then
    cat "$service_describe" >&2
    cat "$OUTPUT_ROOT/serve.log" >&2
    die "VM Service did not report healthy guest TCP and HTTP observations within 90 seconds"
  fi
  cat "$service_describe"
  grep -Eqi 'tcp.*18081|18081.*tcp' "$service_describe" \
    || die "describe did not report the guest TCP probe result"
  grep -Eqi 'http.*18080|18080.*http' "$service_describe" \
    || die "describe did not report the guest HTTP probe result"

  local client_deploy="$OUTPUT_ROOT/client-deploy.log"
  CLIENT_DEPLOYED=1
  bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" deploy --detach "$EXAMPLE_DIR/client-healthy.toml" >"$client_deploy" 2>&1
  cat "$client_deploy"
  local client_describe="$OUTPUT_ROOT/client-describe.log"
  wait_for_client_success "$client_describe" \
    || die "peer VM client did not reach Succeeded through the Service frontend"
  cat "$client_describe"
  echo 'E08 PASS: peer VM Job received byte-exact SVM-E08-GUEST-OK through the Service frontend'
}

copy_case_captures() {
  [[ -n "${SVM_E08_CASE_CAPTURE_DIR:-}" ]] || return 0
  install -d -m 0700 "$SVM_E08_CASE_CAPTURE_DIR"
  find "$OUTPUT_ROOT" -maxdepth 1 -type f -name '*.log' -exec cp -- {} "$SVM_E08_CASE_CAPTURE_DIR" \;
}

case_cleanup() {
  local incoming_rc=$?
  trap - EXIT HUP INT TERM
  local failed=0
  [[ "$CLIENT_DEPLOYED" -eq 0 ]] || stop_workload "$CLIENT_ID" || failed=1
  if [[ "$SERVICE_DEPLOYED" -ne 0 ]]; then
    case "$CASE_CLEANUP_POLICY" in
      operator-stop)
        stop_workload "$SERVICE_ID" || failed=1
        ;;
      preserve-startup-failure)
        preserve_failed_service "$SERVICE_ID" || failed=1
        ;;
      *)
        echo "svm-e08 run: unknown case cleanup policy: $CASE_CLEANUP_POLICY" >&2
        failed=1
        ;;
    esac
  fi
  wait_for_owned_runtime_cleanup || failed=1
  stop_serve || failed=1
  copy_case_captures || failed=1
  [[ "$PREPARED" -eq 0 ]] || bounded 45s "$PREPARE" cleanup || failed=1
  if [[ -n "$SNAPSHOT_DIR" ]]; then
    report_cleanup_deltas || failed=1
    rm -rf -- "$SNAPSHOT_DIR"
  fi
  [[ "$incoming_rc" -ne 0 || "$failed" -eq 0 ]] || exit 1
  exit "$incoming_rc"
}

wait_for_service_result() {
  local expected="$1"
  local stream="$2"
  local describe="$3"
  case "$expected" in
    stable)
      grep -Fq ' is stable ' "$stream"
      [[ "$(first_service_alloc_state <"$describe")" == "Running" ]]
      ;;
    startup-failed)
      grep -Eq 'COMMAND_EXIT_CODE="[1-9][0-9]*"' "$stream"
      grep -Fq 'StartupProbeFailed' "$stream"
      grep -Eqi 'startup probe\[0\].*last=fail|last=fail.*startup probe\[0\]' "$describe"
      local state
      state="$(first_service_alloc_state <"$describe")"
      if [[ "$state" == "Failed" ]]; then
        :
      elif [[ "$state" == "Terminated" ]]; then
        # WorkloadLifecycle may already have stopped a replacement attempt by
        # the time the deploy command returns. That is distinct from a bare
        # crash only when the public row attributes the stop to the operator.
        grep -Eqi 'reason: stopped \(by operator\)' "$describe"
      else
        return 1
      fi
      ;;
    *) die "unknown expected Service result: $expected" ;;
  esac
}

run_case() {
  local spec="$1"
  local service_id="$2"
  local expected="$3"
  local client_spec="${4:-}"
  local client_id="${5:-}"
  local cleanup_policy="${6:-operator-stop}"
  case "$cleanup_policy" in
    operator-stop|preserve-startup-failure) ;;
    *) die "unknown case cleanup policy: $cleanup_policy" ;;
  esac
  CASE_CLEANUP_POLICY="$cleanup_policy"
  require_native_metal
  local command
  for command in awk cargo cloud-hypervisor find findmnt grep ip keyctl losetup mktemp \
    script setsid sort timeout tr; do
    require_command "$command"
  done
  "$PREPARE" check-source
  [[ ! -e "$OUTPUT_ROOT" ]] || die "refusing to overwrite pre-existing materialization: $OUTPUT_ROOT"
  snapshot_before
  trap case_cleanup EXIT
  trap 'exit 130' HUP INT TERM

  if [[ "${SVM_E08_SKIP_BUILD:-0}" != 1 ]]; then
    bounded 600s cargo build -p overdrive-cli --bin overdrive
  fi
  [[ -x "$BIN" ]] || die "default-feature product binary was not built: $BIN"
  PREPARED=1
  bounded 240s "$PREPARE" prepare
  bounded 45s "$PREPARE" check

  setsid keyctl session - env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    CREDENTIALS_DIRECTORY="$CREDS_DIR" "$BIN" serve --bind "$BIND" --data-dir "$DATA_DIR" \
    >"$OUTPUT_ROOT/serve.log" 2>&1 &
  SERVE_PID=$!
  wait_for_serve || die "serve did not become ready within 30 seconds"

  SERVICE_ID="$service_id"
  local service_stream="$OUTPUT_ROOT/service-stream.log"
  local service_command
  printf -v service_command 'env OVERDRIVE_CONFIG_DIR=%q %q deploy %q' \
    "$CONFIG_DIR" "$BIN" "$EXAMPLE_DIR/$spec"
  SERVICE_DEPLOYED=1
  if ! bounded 150s script -q -e -c "$service_command" "$service_stream" >/dev/null; then
    [[ "$expected" == startup-failed ]] || {
      cat "$service_stream" >&2
      die "healthy Service streaming deployment did not complete"
    }
  fi
  local service_describe="$OUTPUT_ROOT/service-describe.log"
  bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" workload describe "$SERVICE_ID" >"$service_describe" 2>&1
  cat "$service_stream"
  wait_for_service_result "$expected" "$service_stream" "$service_describe" || {
    cat "$service_stream" >&2
    cat "$service_describe" >&2
    cat "$OUTPUT_ROOT/serve.log" >&2
    die "Service did not reach expected result: $expected"
  }
  cat "$service_describe"
  if [[ "$expected" == startup-failed \
    && "$CASE_CLEANUP_POLICY" == preserve-startup-failure ]]; then
    # The initial describe is captured before recording stop intent, proving
    # that this cleanup branch starts from the authored startup failure. The
    # case cleanup then waits for either that Failed row to survive reclamation
    # or a witnessed replacement's ordinary operator stop.
    request_stop_intent "$SERVICE_ID" \
      || die "could not record stop intent for startup-failed Service"
  fi

  if [[ -n "$client_spec" ]]; then
    CLIENT_ID="$client_id"
    CLIENT_DEPLOYED=1
    local client_deploy="$OUTPUT_ROOT/client-deploy.log"
    bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
      "$BIN" deploy --detach "$EXAMPLE_DIR/$client_spec" >"$client_deploy" 2>&1
    cat "$client_deploy"
    local client_describe="$OUTPUT_ROOT/client-describe.log"
    wait_for_client_success "$client_describe" || {
      cat "$client_describe" >&2
      die "peer VM client did not complete its declared product assertion"
    }
    cat "$client_describe"
  fi
}

run_isolated_case() {
  local label="$1"
  shift
  local captures="$SVM_E08_MATRIX_CAPTURE_ROOT/$label"
  SVM_E08_OWNERSHIP_TOKEN="$label" \
    SVM_E08_CASE_CAPTURE_DIR="$captures" SVM_E08_SKIP_BUILD=1 \
    "$0" run case "$@"
}

capture_isolated_case() {
  local label="$1"
  shift
  local transcript="$SVM_E08_MATRIX_CAPTURE_ROOT/$label.transcript"
  if ! run_isolated_case "$label" "$@" >"$transcript" 2>&1; then
    cat "$transcript" >&2
    return 1
  fi
}

require_matrix_capture_root() {
  if [[ -z "${SVM_E08_MATRIX_CAPTURE_ROOT:-}" && -n "${EVIDENCE_DIR:-}" ]]; then
    export SVM_E08_MATRIX_CAPTURE_ROOT="$EVIDENCE_DIR"
  fi
  [[ -n "${SVM_E08_MATRIX_CAPTURE_ROOT:-}" ]] \
    || die "matrix capture root is required"
  install -d -m 0700 "$SVM_E08_MATRIX_CAPTURE_ROOT"
}

run_tcp_truthfulness_100() {
  require_native_metal
  require_matrix_capture_root
  bounded 600s cargo build -p overdrive-cli --bin overdrive
  local ledger="$SVM_E08_MATRIX_CAPTURE_ROOT/tcp-truthfulness-100.tsv"
  printf 'trial\thealthy_deploy\thealthy_terminal\thealthy_target\thealthy_peer\tfailure_deploy\tfailure_terminal\tfailure_target\tfailure_peer\tcleanup\n' >"$ledger"
  local trial label
  for trial in $(seq 1 100); do
    printf -v label 'e09-%03d-healthy' "$trial"
    capture_isolated_case "$label" service.toml service-vm-e08 stable client-healthy.toml service-vm-e08-client
    grep -Fq ' is stable ' "$SVM_E08_MATRIX_CAPTURE_ROOT/$label.transcript"
    grep -Fq 'Verdict: Succeeded' "$SVM_E08_MATRIX_CAPTURE_ROOT/$label/client-describe.log"
    printf -v label 'e09-%03d-failure' "$trial"
    capture_isolated_case "$label" tcp-startup-failure.toml service-vm-tcp-failure startup-failed client-tcp-failure.toml service-vm-tcp-failure-client
    grep -Fq 'startup probe[0] failed' "$SVM_E08_MATRIX_CAPTURE_ROOT/$label.transcript"
    grep -Fq 'Verdict: Succeeded' "$SVM_E08_MATRIX_CAPTURE_ROOT/$label/client-describe.log"
    printf '%s\t0\tStable\tguest\texact-reply\tnonzero\tStartupProbeFailed\tguest\tunreachable-verified\tzero-delta\n' "$trial" >>"$ledger"
  done
  [[ "$(($(wc -l <"$ledger") - 1))" -eq 100 ]] || die "E09 ledger does not contain exactly 100 trials"
  echo '--- E09 ledger begin ---'
  cat "$ledger"
  echo '--- E09 ledger end ---'
  echo 'E09 PASS: 100/100 truthful TCP success/failure pairs; no retries or discarded trials'
}

run_http_status_cross_driver() {
  require_native_metal
  require_matrix_capture_root
  bounded 600s cargo build -p overdrive-cli --bin overdrive
  local ledger="$SVM_E08_MATRIX_CAPTURE_ROOT/http-status-cross-driver.tsv"
  printf 'driver\tstatus\tdeploy_exit\tterminal\ttrajectory\tprobe_result\tstdout_bytes\tstderr_bytes\tdescribe_bytes\tsentinel_deploy_stdout\tsentinel_deploy_stderr\tsentinel_describe\tsentinel_probe\tcleanup\n' >"$ledger"
  local driver status spec id expected label transcript describe bytes sentinel
  local cleanup_policy final_describe terminal trajectory
  for driver in exec vm; do
    for status in 204 302 404 503; do
      spec="http-${driver}-${status}.toml"
      id="service-${driver}-http-${status}"
      expected=startup-failed
      cleanup_policy=preserve-startup-failure
      [[ "$status" == 204 ]] && expected=stable
      [[ "$status" == 204 ]] && cleanup_policy=operator-stop
      label="e10-${driver}-${status}"
      capture_isolated_case "$label" "$spec" "$id" "$expected" "" "" "$cleanup_policy"
      transcript="$SVM_E08_MATRIX_CAPTURE_ROOT/$label.transcript"
      describe="$SVM_E08_MATRIX_CAPTURE_ROOT/$label/service-describe.log"
      if [[ "$status" == 204 ]]; then
        grep -Fq ' is stable ' "$transcript"
        final_describe="$SVM_E08_MATRIX_CAPTURE_ROOT/$label/${id}-stopped.log"
        [[ -f "$final_describe" ]] || die "E10 missing operator-stop describe for $driver/$status"
        terminal="$(first_service_alloc_state <"$final_describe")"
        [[ "$terminal" == "Terminated" ]] \
          || die "E10 running Service did not become Terminated after stop: $driver/$status"
        trajectory=operator-stop-running
      else
        grep -Fq "${status}" "$transcript" "$describe"
        ! grep -Eqi 'redirect.*followed|following.*redirect' "$transcript" "$describe"
        final_describe="$SVM_E08_MATRIX_CAPTURE_ROOT/$label/${id}-preserved-failure.log"
        [[ -f "$final_describe" ]] || die "E10 missing preserved-failure describe for $driver/$status"
        terminal="$(first_service_alloc_state <"$final_describe")"
        if [[ "$terminal" == "Failed" ]] \
          && grep -Fq 'StartupProbeFailed' "$final_describe"; then
          trajectory=preserved-startup-failure
        elif [[ "$terminal" == "Failed" ]] \
          && grep -Fq 'reason: driver internal error:' "$final_describe" \
          && grep -Fq 'bind beacon listener: Address already in use' "$final_describe"; then
          trajectory=replacement-start-rejected
        elif [[ "$terminal" == "Terminated" ]] \
          && [[ "$(first_service_restart_count <"$final_describe")" =~ ^[1-9][0-9]*$ ]] \
          && grep -Eqi 'last terminated: .*stopped \(by operator\)' "$final_describe" \
          && grep -Fq 'reason: stopped' "$final_describe"; then
          trajectory=replacement-operator-stop
        else
          die "E10 startup-failure cleanup followed an unrecognized trajectory: $driver/$status"
        fi
      fi
      grep -Fq 'E08 teardown deltas: vm=0 probe=0 network=0 cgroup=0 run-directory=0 mount=0 loop=0 preparation=0' \
        "$transcript" \
        || die "E10 nonzero cleanup delta for $driver/$status"
      sentinel='SVM-E10-FAILURE-BODY-MUST-NOT-LEAK'
      sentinel_count="$({ grep -Foh "$sentinel" "$transcript" "$describe" "$final_describe" || true; } | wc -l | tr -d ' ')"
      [[ "$sentinel_count" -eq 0 ]] \
        || die "E10 sentinel leaked for $driver/$status"
      bytes="$(wc -c <"$transcript" | tr -d ' ')"
      printf '%s\t%s\t0\t%s\t%s\tstatus-%s\t%s\t0\t%s\t0\t0\t0\t0\tzero-delta\n' \
        "$driver" "$status" "$terminal" "$trajectory" "$status" "$bytes" \
        "$(wc -c <"$describe" | tr -d ' ')" >>"$ledger"
    done
  done
  [[ "$(($(wc -l <"$ledger") - 1))" -eq 8 ]] || die "E10 ledger does not contain all eight cells"
  echo '--- E10 ledger begin ---'
  cat "$ledger"
  echo '--- E10 ledger end ---'
  echo 'E10 PASS: 8/8 Exec/VM HTTP status cells with zero failure-body sentinel exposure'
}

run_zero_probes() {
  require_native_metal
  require_matrix_capture_root
  bounded 600s cargo build -p overdrive-cli --bin overdrive
  local ledger="$SVM_E08_MATRIX_CAPTURE_ROOT/zero-probes.tsv"
  printf 'case\tdeploy_exit\tterminal\tinferred\ttarget\tpeer\teligibility\tcleanup\n' >"$ledger"
  capture_isolated_case e13-healthy zero-probes.toml service-vm-zero-declared-probes stable \
    client-zero-probes.toml service-vm-zero-probes-client
  grep -Eqi 'inferred.*tcp|tcp.*inferred' "$SVM_E08_MATRIX_CAPTURE_ROOT/e13-healthy/service-describe.log"
  grep -Fq 'Verdict: Succeeded' "$SVM_E08_MATRIX_CAPTURE_ROOT/e13-healthy/client-describe.log"
  printf 'healthy\t0\tStable\ttrue\tguest\texact-reply\thealthy\tzero-delta\n' >>"$ledger"
  capture_isolated_case e13-failure zero-probes-failure.toml service-vm-zero-declared-probes-failure startup-failed \
    client-zero-probes-failure.toml service-vm-zero-probes-failure-client
  grep -Fq 'startup probe[0] failed' "$SVM_E08_MATRIX_CAPTURE_ROOT/e13-failure.transcript"
  grep -Fq 'Verdict: Succeeded' "$SVM_E08_MATRIX_CAPTURE_ROOT/e13-failure/client-describe.log"
  printf 'failure\tnonzero\tStartupProbeFailed\ttrue\tguest\tunreachable-verified\tunhealthy\tzero-delta\n' >>"$ledger"
  echo '--- E13 ledger begin ---'
  cat "$ledger"
  echo '--- E13 ledger end ---'
  echo 'E13 PASS: inferred TCP success/failure and complementary VM peer traffic are truthful'
}

case "${1:-}" in
  check-source)
    "$PREPARE" check-source
    ;;
  run)
    case "${2:-}" in
      healthy) run_healthy ;;
      tcp-truthfulness-100) run_tcp_truthfulness_100 ;;
      http-status-cross-driver) run_http_status_cross_driver ;;
      zero-probes) run_zero_probes ;;
      case) shift 2; run_case "$@" ;;
      readiness-recovery|liveness-restart)
        echo "PENDING ${2}: DELIVER must activate its bounded product mode" >&2
        exit 75
        ;;
      *) die 'usage: run-example.sh run healthy|tcp-truthfulness-100|http-status-cross-driver|readiness-recovery|liveness-restart|zero-probes' ;;
    esac
    ;;
  *)
    die 'usage: run-example.sh check-source|run healthy'
    ;;
esac

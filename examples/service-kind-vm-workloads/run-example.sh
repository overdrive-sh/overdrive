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
BIN="$(cd "$EXAMPLE_DIR/../.." && pwd)/target/debug/overdrive"
readonly BIN
readonly BIND="127.0.0.1:7644"
SERVICE_ID="service-vm-e08"
CLIENT_ID="service-vm-e08-client"

SERVE_PID=""
PREPARED=0
SERVICE_DEPLOYED=0
CLIENT_DEPLOYED=0
SNAPSHOT_DIR=""
CASE_CLEANUP_POLICY="operator-stop"
CASE_LABEL="E08"

# Readiness transition evidence is populated by wait_for_service_readiness and
# consumed only by the bounded E11 journey. Keep these values as shell state so
# the product runner can record the exact observation row and detection time
# without introducing a second control-plane API.
READINESS_OBSERVED_AT_MS=""
READINESS_DETECTED_AT_MS=""
READINESS_LATENCY_MS=""
CLIENT_STARTED_AT_MS=""
CLIENT_ELAPSED_MS=""

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

probe_overdrive_network_names() {
  probe_network | awk '
    $1 ~ /^ovd-(hv|wl|tp|ns)-/ {
      split($1, parts, "@")
      print parts[1]
    }
  ' | sort -u
}

probe_hypervisors_for_alloc() {
  local alloc_id="$1"
  local proc argv0
  for proc in /proc/[0-9]*; do
    [[ -r "$proc/cmdline" ]] || continue
    argv0=''
    IFS= read -r -d '' argv0 <"$proc/cmdline" || true
    [[ "$(basename "$argv0" 2>/dev/null || true)" == "cloud-hypervisor" ]] \
      || continue
    grep -aFq "/run/overdrive/vm/$alloc_id/" "$proc/cmdline" \
      && basename "$proc"
  done | sort -n
}

probe_loops() {
  losetup -j "$OUTPUT_ROOT/rootfs.ext4" 2>/dev/null || true
}

probe_mounts() {
  findmnt --raw --noheadings --output TARGET,SOURCE 2>/dev/null | \
    grep -F "$OUTPUT_ROOT" || true
}

capture_resource_snapshot() {
  echo '[hypervisors]'
  probe_hypervisors
  echo '[cgroup-scopes]'
  probe_scopes
  echo '[vm-run-directories]'
  probe_run_dirs
  echo '[network]'
  probe_network
  echo '[loop-devices]'
  probe_loops
  echo '[mounts]'
  probe_mounts
  printf '[materialization]\n%s\n' "$([[ -e "$OUTPUT_ROOT" ]] && printf present || printf absent)"
}

capture_e10_resource_snapshot() {
  local id="$1"
  capture_resource_snapshot
  echo '[allocation-hypervisors]'
  probe_hypervisors_for_alloc "alloc-$id-0"
}

snapshot_before() {
  SNAPSHOT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/svm-e08-snapshot.XXXXXX")"
  probe_hypervisors >"$SNAPSHOT_DIR/hypervisors"
  probe_scopes >"$SNAPSHOT_DIR/scopes"
  probe_run_dirs >"$SNAPSHOT_DIR/run-dirs"
  probe_network >"$SNAPSHOT_DIR/network"
  probe_overdrive_network_names >"$SNAPSHOT_DIR/network-names"
  probe_loops >"$SNAPSHOT_DIR/loops"
  probe_mounts >"$SNAPSHOT_DIR/mounts"
}

capture_e10_current_owned_resources() {
  local id="$1"
  local alloc_id="alloc-$id-0"
  printf 'allocation=%s\n' "$alloc_id"
  grep -Fxq "$alloc_id.scope" <(probe_scopes) \
    && printf 'cgroup=%s.scope\n' "$alloc_id" || true
  grep -Fxq "$alloc_id" <(probe_run_dirs) \
    && printf 'run-directory=%s\n' "$alloc_id" || true
  probe_hypervisors_for_alloc "$alloc_id" \
    | while IFS= read -r pid; do
        [[ -z "$pid" ]] || printf 'hypervisor=%s\n' "$pid"
      done
  comm -13 "$SNAPSHOT_DIR/network-names" <(probe_overdrive_network_names) \
    | while IFS= read -r name; do
        [[ -z "$name" ]] || printf 'network=%s\n' "$name"
      done
}

e10_named_runtime_resources_absent() {
  local id="$1"
  local observed="$2"
  local alloc_id="alloc-$id-0"
  local kind name

  # These allocation-bearing resources have stable external names. Their
  # absence is required even when cleanup won the race before the pre-stop
  # resource observation and therefore no positive delta was captured.
  ! grep -Fxq "$alloc_id.scope" <(probe_scopes) || return 1
  ! grep -Fxq "$alloc_id" <(probe_run_dirs) || return 1
  [[ -z "$(probe_hypervisors_for_alloc "$alloc_id")" ]] || return 1

  # Network names are learned from the current-session snapshot because the
  # slot is not part of the public allocation row. A name observed there must
  # disappear; an already-clean snapshot does not have to invent a delta.
  while IFS='=' read -r kind name; do
    [[ "$kind" == "network" ]] || continue
    ! grep -Fxq "$name" <(probe_overdrive_network_names) || return 1
  done <"$observed"
}

wait_for_e10_named_runtime_cleanup() {
  local id="$1"
  local observed="$2"
  local deadline=$((SECONDS + 45))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    e10_named_runtime_resources_absent "$id" "$observed" && return 0
  done
  return 1
}

new_delta_count() {
  local before="$1"
  shift
  comm -13 "$before" <("$@") | sed '/^$/d' | wc -l | tr -d ' '
}

stop_workload() {
  local id="$1"
  local output="$OUTPUT_ROOT/${id}-stop.log"
  local stdout="$OUTPUT_ROOT/stop-stdout.log"
  local stderr="$OUTPUT_ROOT/stop-stderr.log"
  local exit_file="$OUTPUT_ROOT/stop-exit.log"
  printf 'env OVERDRIVE_CONFIG_DIR=%q %q job stop %q\n' \
    "$CONFIG_DIR" "$BIN" "$id" >"$OUTPUT_ROOT/stop-command.log"
  local stop_rc
  set +e
  bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" job stop "$id" >"$stdout" 2>"$stderr"
  stop_rc=$?
  set -e
  printf '%s\n' "$stop_rc" >"$exit_file"
  { cat "$stdout"; cat "$stderr"; } >"$output"
  if [[ "$stop_rc" -ne 0 ]]; then
      cat "$output" >&2
      return 1
  fi
  local describe="$OUTPUT_ROOT/${id}-stopped.log"
  if ! wait_for_workload_stop "$id" "$describe"; then
    cat "$describe" >&2
    return 1
  fi
  if [[ -n "${SVM_E10_CAPTURE_TOKEN:-}" ]]; then
    local settled="$OUTPUT_ROOT/${id}-stopped-settled.log"
    local observed_resources="$OUTPUT_ROOT/current-session-owned-resources.log"
    wait_for_e10_named_runtime_cleanup "$id" "$observed_resources" || return 1
    capture_e10_resource_snapshot "$id" \
      >"$OUTPUT_ROOT/post-runtime-cleanup-resources.log"
    bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
      "$BIN" workload describe "$id" >"$settled" 2>&1 || return 1
    [[ "$(first_service_alloc_state <"$settled")" == "Terminated" ]] \
      && [[ "$(first_service_alloc_id <"$settled")" \
        == "$(first_service_alloc_id <"$describe")" ]] \
      && [[ "$(first_service_restart_count <"$settled")" \
        == "$(first_service_restart_count <"$describe")" ]] \
      && grep -Fq '    reason: stopped' "$settled" \
      || return 1
    cp -- "$describe" "$OUTPUT_ROOT/after-stop-describe.log"
    cp -- "$settled" "$OUTPUT_ROOT/post-cleanup-describe.log"
    {
      printf 'after_stop_allocation=%s\n' \
        "$(first_service_alloc_id <"$describe")"
      printf 'after_stop_restart_count=%s\n' \
        "$(first_service_restart_count <"$describe")"
      printf 'after_cleanup_allocation=%s\n' \
        "$(first_service_alloc_id <"$settled")"
      printf 'after_cleanup_restart_count=%s\n' \
        "$(first_service_restart_count <"$settled")"
    } >"$OUTPUT_ROOT/stale-session-refusal.log"
  fi
  cat "$output"
}

request_stop_intent() {
  local id="$1"
  local stop_output="$OUTPUT_ROOT/${id}-preserve-stop.log"
  local stdout="$OUTPUT_ROOT/stop-stdout.log"
  local stderr="$OUTPUT_ROOT/stop-stderr.log"
  local exit_file="$OUTPUT_ROOT/stop-exit.log"
  printf 'env OVERDRIVE_CONFIG_DIR=%q %q job stop %q\n' \
    "$CONFIG_DIR" "$BIN" "$id" >"$OUTPUT_ROOT/stop-command.log"
  local stop_rc
  set +e
  bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" job stop "$id" >"$stdout" 2>"$stderr"
  stop_rc=$?
  set -e
  printf '%s\n' "$stop_rc" >"$exit_file"
  { cat "$stdout"; cat "$stderr"; } >"$stop_output"
  if [[ "$stop_rc" -ne 0 ]]; then
      cat "$stop_output" >&2
      return 1
  fi
}

stop_recovered_service_after_startup_failure() {
  local id="$1"
  local before="$OUTPUT_ROOT/service-describe.log"
  local before_state before_alloc before_restarts before_history before_probe
  before_state="$(first_service_alloc_state <"$before")"
  before_alloc="$(first_service_alloc_id <"$before")"
  before_restarts="$(first_service_restart_count <"$before")"
  before_history="$(first_service_last_terminated_line <"$before")"
  before_probe="$(first_service_startup_probe_line <"$before")"
  [[ -n "$before_alloc" && "$before_restarts" =~ ^[0-9]+$ ]] || return 1
  [[ -n "$before_probe" ]] || return 1
  [[ "$before_state" == "Running" ]] || return 1
  [[ "$before_restarts" =~ ^[1-9][0-9]*$ ]] || return 1
  has_prior_failed_snapshot <"$before" || return 1

  request_stop_intent "$id"
  local stop_output="$OUTPUT_ROOT/${id}-preserve-stop.log"
  local output="$OUTPUT_ROOT/${id}-preserved-failure.log"
  local settled="$OUTPUT_ROOT/${id}-preserved-failure-settled.log"
  local observations="$OUTPUT_ROOT/${id}-preserved-failure-observations.log"
  local deadline=$((SECONDS + 45))
  : >"$observations"
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
      "$BIN" workload describe "$id" >"$output" 2>&1 || true
    local state
    state="$(first_service_alloc_state <"$output")"
    {
      printf 'observed_utc=%s state=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${state:-unrecognized}"
      cat "$output"
    } >>"$observations"

    local outcome_ok=1
    [[ "$state" == "Terminated" ]] \
      && [[ "$(first_service_alloc_id <"$output")" == "$before_alloc" ]] \
      && [[ "$(first_service_restart_count <"$output")" == "$before_restarts" ]] \
      && has_prior_failed_snapshot <"$output" \
      && [[ "$(first_service_last_terminated_line <"$output")" == "$before_history" ]] \
      && e10_failed_probe_is_preserved \
        "$before_probe" "$(first_service_startup_probe_line <"$output")" \
      && grep -Fq '    reason: stopped' "$output" \
      || outcome_ok=0

    local observed_resources="$OUTPUT_ROOT/current-session-owned-resources.log"
    if [[ "$outcome_ok" -eq 1 ]] \
      && e10_named_runtime_resources_absent "$id" "$observed_resources"; then
      # Observe once more after the owned runtime is gone. A stale accepted-
      # session watcher must not replace the recovered incarnation's operator
      # termination or the retained failure snapshot after cleanup settles.
      capture_e10_resource_snapshot "$id" \
        >"$OUTPUT_ROOT/post-runtime-cleanup-resources.log"
      bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
        "$BIN" workload describe "$id" >"$settled" 2>&1 || true
      {
        printf 'settled_utc=%s state=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
          "$(first_service_alloc_state <"$settled")"
        cat "$settled"
      } >>"$observations"
      if [[ "$(first_service_alloc_state <"$settled")" == "$state" ]] \
        && [[ "$(first_service_alloc_id <"$settled")" == "$before_alloc" ]] \
        && [[ "$(first_service_restart_count <"$settled")" == "$before_restarts" ]] \
        && e10_failed_probe_is_preserved \
          "$(first_service_startup_probe_line <"$output")" \
          "$(first_service_startup_probe_line <"$settled")" \
        && has_prior_failed_snapshot <"$settled" \
        && [[ "$(first_service_last_terminated_line <"$settled")" == "$before_history" ]] \
        && grep -Fq '    reason: stopped' "$settled"; then
        cp -- "$output" "$OUTPUT_ROOT/after-stop-describe.log"
        cp -- "$settled" "$OUTPUT_ROOT/post-cleanup-describe.log"
        cp -- "$observations" "$OUTPUT_ROOT/stop-observations.log"
        {
          printf 'before_stop_allocation=%s\n' "$before_alloc"
          printf 'before_stop_restart_count=%s\n' "$before_restarts"
          printf 'before_stop_prior_failure=%s\n' "$before_history"
          printf 'before_stop_probe=%s\n' "$before_probe"
          printf 'after_stop_probe=%s\n' \
            "$(first_service_startup_probe_line <"$output")"
          printf 'after_cleanup_allocation=%s\n' \
            "$(first_service_alloc_id <"$settled")"
          printf 'after_cleanup_restart_count=%s\n' \
            "$(first_service_restart_count <"$settled")"
          printf 'after_cleanup_prior_failure=%s\n' \
            "$(first_service_last_terminated_line <"$settled")"
          printf 'after_cleanup_probe=%s\n' \
            "$(first_service_startup_probe_line <"$settled")"
        } >"$OUTPUT_ROOT/stale-session-refusal.log"
        cat "$stop_output"
        cat "$output"
        cat "$settled"
        return 0
      fi
      cat "$stop_output" >&2
      cat "$output" >&2
      cat "$settled" >&2
      echo "svm-e08 run: state changed after owned runtime cleanup: $id" >&2
      return 1
    fi
  done
  cat "$stop_output" >&2
  cat "$output" >&2
  echo "svm-e08 run: recovered startup-failure incarnation did not stop with preserved history: $id" >&2
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
  local label="$CASE_LABEL"
  vms="$(new_delta_count "$SNAPSHOT_DIR/hypervisors" probe_hypervisors)"
  scopes="$(new_delta_count "$SNAPSHOT_DIR/scopes" probe_scopes)"
  networks="$(new_delta_count "$SNAPSHOT_DIR/network" probe_network)"
  run_dirs="$(new_delta_count "$SNAPSHOT_DIR/run-dirs" probe_run_dirs)"
  loops="$(new_delta_count "$SNAPSHOT_DIR/loops" probe_loops)"
  mounts="$(new_delta_count "$SNAPSHOT_DIR/mounts" probe_mounts)"
  [[ -e "$OUTPUT_ROOT" ]] && prep=1 || prep=0
  [[ -n "$SERVE_PID" ]] && probe_tasks=1 || probe_tasks=0
  printf '%s teardown deltas: vm=%s probe=%s network=%s cgroup=%s run-directory=%s mount=%s loop=%s preparation=%s\n' \
    "$label" "$vms" "$probe_tasks" "$networks" "$scopes" "$run_dirs" "$mounts" "$loops" "$prep"
  if [[ "$networks" -ne 0 ]]; then
    printf '%s unexpected network delta:\n' "$label" >&2
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
  # shellcheck disable=SC2320 # EXIT trap supplies the status being preserved.
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

has_prior_failed_snapshot() {
  awk '
    /^Alloc[[:space:]]+State[[:space:]]+/ { in_table = 1; next }
    in_table && $1 ~ /^alloc-/ {
      if (seen) exit
      seen = 1
    }
    seen && /^Memory:/ { exit }
    seen && /last terminated:[^[:cntrl:]]*Failed/ { found = 1 }
    END { exit(found ? 0 : 1) }
  '
}

first_service_last_terminated_line() {
  awk '/^    last terminated: / { print; exit }'
}

first_service_startup_probe_line() {
  awk '/^  startup probe\[0\] / { print; exit }'
}

# E10 preserves the failure evidence carried by a ProbeResultRow, not the
# byte rendering of its observation time. The probe role/index, mechanic,
# method, target, failed status, and failure reason must be identical. The
# LWW observation timestamp may stay equal or advance when the same failed
# result is observed again, but it must never regress.
e10_failed_probe_is_preserved() {
  local before_line="$1"
  local after_line="$2"
  local pattern='^([[:space:]]*startup[[:space:]]+probe\[0\][[:space:]]+http[[:space:]]+GET[[:space:]]+[^[:space:]]+[[:space:]]+last=fail[[:space:]]+\(.*\))[[:space:]]+last_observed_at=([0-9]+)$'
  local before_semantics before_observed_at after_semantics after_observed_at

  [[ "$before_line" =~ $pattern ]] || return 1
  before_semantics="${BASH_REMATCH[1]}"
  before_observed_at="${BASH_REMATCH[2]}"
  [[ "$after_line" =~ $pattern ]] || return 1
  after_semantics="${BASH_REMATCH[1]}"
  after_observed_at="${BASH_REMATCH[2]}"

  [[ "$after_semantics" == "$before_semantics" ]] \
    && [[ "$after_observed_at" -ge "$before_observed_at" ]]
}

first_service_terminal() {
  awk '
    /^    terminal: / {
      sub(/^    terminal: /, "")
      print
      exit
    }
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

now_ms() {
  local value
  # Coreutils accepts `%N` for nanoseconds but does not consistently honour
  # a field width for it. Strip the six sub-millisecond digits explicitly so
  # this remains epoch milliseconds on the native Linux host.
  value="$(date +%s%N)"
  [[ "$value" =~ ^[0-9]+$ ]] || die "date did not return epoch milliseconds: $value"
  local milliseconds_end=$(( ${#value} - 6 ))
  (( milliseconds_end > 0 )) || die "date returned too-short epoch timestamp: $value"
  printf '%s' "${value:0:milliseconds_end}"
}

readiness_observed_at_ms() {
  local output="$1"
  sed -n -E 's/.*readiness probe\[0\].*last_observed_at=([0-9]+).*/\1/p' "$output" | tail -n 1
}

wait_for_service_readiness() {
  local expected="$1"
  local output="$2"
  local deadline=$((SECONDS + 45))
  local pattern
  case "$expected" in
    pass) pattern='readiness.*last=pass' ;;
    fail) pattern='readiness.*last=fail' ;;
    *) die "unknown readiness state: $expected" ;;
  esac

  READINESS_OBSERVED_AT_MS=""
  READINESS_DETECTED_AT_MS=""
  READINESS_LATENCY_MS=""
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
      "$BIN" workload describe "$SERVICE_ID" >"$output" 2>&1 || true
    if [[ "$(first_service_alloc_state <"$output")" == "Running" ]] \
      && grep -Eqi "$pattern" "$output" \
      && [[ "$(first_service_terminal <"$output")" == "Stable" ]]; then
      READINESS_OBSERVED_AT_MS="$(readiness_observed_at_ms "$output")"
      [[ "$READINESS_OBSERVED_AT_MS" =~ ^[0-9]+$ ]] \
        || die "readiness describe omitted last_observed_at for $expected"
      READINESS_DETECTED_AT_MS="$(now_ms)"
      READINESS_LATENCY_MS=$((READINESS_DETECTED_AT_MS - READINESS_OBSERVED_AT_MS))
      (( READINESS_LATENCY_MS >= 0 )) \
        || die "readiness observation timestamp is ahead of the local clock"
      (( READINESS_LATENCY_MS <= 2000 )) \
        || die "readiness $expected observation exceeded the two-second bound: ${READINESS_LATENCY_MS}ms"
      return 0
    fi
    sleep 0.2
  done
  return 1
}

assert_service_steady() {
  local describe="$1"
  [[ "$(first_service_alloc_state <"$describe")" == "Running" ]] \
    || die "readiness phase changed the VM Service allocation out of Running"
  [[ "$(first_service_restart_count <"$describe")" == "0" ]] \
    || die "readiness phase restarted the VM Service allocation"
}

first_service_alloc_id() {
  awk '
    /^Alloc[[:space:]]+State[[:space:]]+/ { in_table = 1; next }
    in_table && /^[-[:space:]]+$/ { next }
    in_table && $1 ~ /^alloc-/ { print $1; exit }
  '
}

first_service_alloc_started_at() {
  awk '
    /^Alloc[[:space:]]+State[[:space:]]+/ { in_table = 1; next }
    in_table && /^[-[:space:]]+$/ { next }
    in_table && $1 ~ /^alloc-/ { print $4; exit }
  '
}

first_service_startup_observed_at() {
  sed -n -E 's/.*startup probe\[0\].*last_observed_at=([0-9]+).*/\1/p' | tail -n 1
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

run_readiness_recovery() {
  require_native_metal
  local command
  for command in awk cargo cloud-hypervisor date find findmnt grep ip keyctl losetup mktemp \
    script setsid sort tail timeout tr; do
    require_command "$command"
  done
  "$PREPARE" check-source
  [[ ! -e "$OUTPUT_ROOT" ]] || die "refusing to overwrite pre-existing materialization: $OUTPUT_ROOT"
  snapshot_before
  CASE_LABEL="E11"
  SERVICE_ID="service-vm-readiness-recovery"
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

  local service_stream="$OUTPUT_ROOT/e11-service-stream.log"
  local service_command
  printf -v service_command 'env OVERDRIVE_CONFIG_DIR=%q %q deploy %q' \
    "$CONFIG_DIR" "$BIN" "$EXAMPLE_DIR/readiness-recovery.toml"
  SERVICE_DEPLOYED=1
  if ! bounded 150s script -q -e -c "$service_command" "$service_stream" >/dev/null; then
    cat "$service_stream" >&2
    cat "$OUTPUT_ROOT/serve.log" >&2
    die "E11 Service streaming deployment did not complete"
  fi
  local accepted stable
  accepted="$(grep -n -m1 'Accepted' "$service_stream" | cut -d: -f1 || true)"
  stable="$(grep -ni -m1 'stable' "$service_stream" | cut -d: -f1 || true)"
  if [[ -z "$accepted" || -z "$stable" || "$accepted" -ge "$stable" ]]; then
    cat "$service_stream" >&2
    die "E11 single service deployment did not render Accepted before Stable"
  fi
  grep -Eq 'startup.*(index|probe).*0|probe.*0.*startup' "$service_stream" \
    || die "E11 Stable render did not name startup probe index 0"
  grep -Fq ' is stable ' "$service_stream" \
    || die "E11 Service did not render the stable lifecycle state"
  cat "$service_stream"

  local service_describe="$OUTPUT_ROOT/e11-before-describe.log"
  wait_for_service_observations "$service_describe" \
    || { cat "$service_describe" >&2; cat "$OUTPUT_ROOT/serve.log" >&2; \
         die "E11 VM Service did not report initial startup/readiness Pass"; }
  wait_for_service_readiness pass "$service_describe" \
    || { cat "$service_describe" >&2; die "E11 initial readiness Pass was not observed"; }
  assert_service_steady "$service_describe"
  cat "$service_describe"

  local before_client_deploy="$OUTPUT_ROOT/e11-before-client-deploy.log"
  local before_client_describe="$OUTPUT_ROOT/e11-before-client-describe.log"
  CLIENT_ID="service-vm-readiness-client-before"
  CLIENT_DEPLOYED=1
  CLIENT_STARTED_AT_MS="$(now_ms)"
  bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" deploy --detach "$EXAMPLE_DIR/client-readiness-before.toml" \
    >"$before_client_deploy" 2>&1
  cat "$before_client_deploy"
  wait_for_client_success "$before_client_describe" \
    || { cat "$before_client_describe" >&2; die "E11 before client did not receive the exact guest reply"; }
  CLIENT_ELAPSED_MS=$(( $(now_ms) - CLIENT_STARTED_AT_MS ))
  grep -Fq 'Verdict: Succeeded' "$before_client_describe" \
    || die "E11 before client did not have a successful public verdict"
  cat "$before_client_describe"
  stop_workload "$CLIENT_ID"
  CLIENT_DEPLOYED=0

  local before_observed="$READINESS_OBSERVED_AT_MS"
  local before_detected="$READINESS_DETECTED_AT_MS"
  local before_latency="$READINESS_LATENCY_MS"
  local before_terminal
  before_terminal="$(first_service_terminal <"$service_describe")"
  [[ "$before_terminal" == "Stable" ]] \
    || die "E11 initial readiness response omitted the current Stable terminal claim"
  local before_client_start="$CLIENT_STARTED_AT_MS"
  local before_client_elapsed="$CLIENT_ELAPSED_MS"

  local during_describe="$OUTPUT_ROOT/e11-during-describe.log"
  wait_for_service_readiness fail "$during_describe" \
    || { cat "$during_describe" >&2; die "E11 readiness did not withdraw the backend"; }
  assert_service_steady "$during_describe"
  cat "$during_describe"
  local during_observed="$READINESS_OBSERVED_AT_MS"
  local during_detected="$READINESS_DETECTED_AT_MS"
  local during_latency="$READINESS_LATENCY_MS"
  local during_terminal
  during_terminal="$(first_service_terminal <"$during_describe")"
  [[ "$during_terminal" == "Stable" ]] \
    || die "E11 failed-readiness response omitted the current Stable terminal claim"

  local during_client_deploy="$OUTPUT_ROOT/e11-during-client-deploy.log"
  local during_client_describe="$OUTPUT_ROOT/e11-during-client-describe.log"
  CLIENT_ID="service-vm-readiness-client-during"
  CLIENT_DEPLOYED=1
  CLIENT_STARTED_AT_MS="$(now_ms)"
  bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" deploy --detach "$EXAMPLE_DIR/client-readiness-during.toml" \
    >"$during_client_deploy" 2>&1
  cat "$during_client_deploy"
  wait_for_client_success "$during_client_describe" \
    || { cat "$during_client_describe" >&2; die "E11 negative client observed the failed backend or did not terminate successfully"; }
  CLIENT_ELAPSED_MS=$(( $(now_ms) - CLIENT_STARTED_AT_MS ))
  grep -Fq 'Verdict: Succeeded' "$during_client_describe" \
    || die "E11 negative client did not have the successful no-reply verdict"
  cat "$during_client_describe"
  local during_client_start="$CLIENT_STARTED_AT_MS"
  local during_client_elapsed="$CLIENT_ELAPSED_MS"
  stop_workload "$CLIENT_ID"
  CLIENT_DEPLOYED=0

  local after_describe="$OUTPUT_ROOT/e11-after-describe.log"
  wait_for_service_readiness pass "$after_describe" \
    || { cat "$after_describe" >&2; die "E11 readiness did not restore the backend"; }
  assert_service_steady "$after_describe"
  cat "$after_describe"
  local after_observed="$READINESS_OBSERVED_AT_MS"
  local after_detected="$READINESS_DETECTED_AT_MS"
  local after_latency="$READINESS_LATENCY_MS"
  local after_terminal
  after_terminal="$(first_service_terminal <"$after_describe")"
  [[ "$after_terminal" == "Stable" ]] \
    || die "E11 restored-readiness response omitted the current Stable terminal claim"

  local after_client_deploy="$OUTPUT_ROOT/e11-after-client-deploy.log"
  local after_client_describe="$OUTPUT_ROOT/e11-after-client-describe.log"
  CLIENT_ID="service-vm-readiness-client-after"
  CLIENT_DEPLOYED=1
  CLIENT_STARTED_AT_MS="$(now_ms)"
  bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" deploy --detach "$EXAMPLE_DIR/client-readiness-after.toml" \
    >"$after_client_deploy" 2>&1
  cat "$after_client_deploy"
  wait_for_client_success "$after_client_describe" \
    || { cat "$after_client_describe" >&2; die "E11 after client did not receive the exact guest reply"; }
  CLIENT_ELAPSED_MS=$(( $(now_ms) - CLIENT_STARTED_AT_MS ))
  grep -Fq 'Verdict: Succeeded' "$after_client_describe" \
    || die "E11 after client did not have a successful public verdict"
  cat "$after_client_describe"
  stop_workload "$CLIENT_ID"
  CLIENT_DEPLOYED=0

  local after_client_start="$CLIENT_STARTED_AT_MS"
  local after_client_elapsed="$CLIENT_ELAPSED_MS"
  local before_restarts during_restarts after_restarts
  before_restarts="$(first_service_restart_count <"$service_describe")"
  during_restarts="$(first_service_restart_count <"$during_describe")"
  after_restarts="$(first_service_restart_count <"$after_describe")"
  [[ "$before_restarts" == "0" && "$during_restarts" == "0" && "$after_restarts" == "0" ]] \
    || die "E11 readiness journey observed an allocation restart"

  echo '--- E11 ledger begin ---'
  printf 'phase\treadiness\tterminal\tobserved_at_ms\tdetected_at_ms\ttransition_latency_ms\tclient_started_at_ms\tclient_elapsed_ms\tlifecycle\trestarts\tpeer_result\n'
  printf 'before\tpass\t%s\t%s\t%s\t%s\t%s\t%s\tRunning\t%s\texact-reply\n' \
    "$before_terminal" "$before_observed" "$before_detected" "$before_latency" "$before_client_start" \
    "$before_client_elapsed" "$before_restarts"
  printf 'during\tfail\t%s\t%s\t%s\t%s\t%s\t%s\tRunning\t%s\tunreachable-no-exact-reply\n' \
    "$during_terminal" "$during_observed" "$during_detected" "$during_latency" "$during_client_start" \
    "$during_client_elapsed" "$during_restarts"
  printf 'after\tpass\t%s\t%s\t%s\t%s\t%s\t%s\tRunning\t%s\texact-reply\n' \
    "$after_terminal" "$after_observed" "$after_detected" "$after_latency" "$after_client_start" \
    "$after_client_elapsed" "$after_restarts"
  echo '--- E11 ledger end ---'
  echo 'E11 PASS: 2/2 readiness transitions within two seconds; exact peer replies before/after and no failed-window reply'

  # Run the existing bounded teardown before returning so the E11 PASS line
  # and its zero-delta result are adjacent, durable evidence in the runner's
  # transcript. cleanup() removes its EXIT trap and exits with the teardown
  # result; failure therefore cannot be mistaken for a passing journey.
  cleanup
}

read_liveness_failure_threshold() {
  awk '
    /^\[\[health_check\.liveness\]\]/ { in_liveness = 1; next }
    in_liveness && /^\[\[/ { exit }
    in_liveness && $1 == "failure_threshold" { print $3; exit }
  ' "$EXAMPLE_DIR/liveness-restart.toml"
}

LIVENESS_FAILURE_COUNT=0

wait_for_liveness_restart() {
  local terminal_describe="$1"
  local replacement_describe="$2"
  local failure_threshold="$3"
  local baseline_startup_observed_at="$4"
  local baseline_started_at="$5"
  local deadline=$((SECONDS + 90))
  local liveness_failure_count=0
  local last_liveness_failure_observed_at=""
  local terminal_seen=0
  : >"$terminal_describe"
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
      "$BIN" workload describe "$SERVICE_ID" >"$replacement_describe" 2>&1 || true
    local liveness_failure_observed_at
    liveness_failure_observed_at="$(sed -n -E \
      's/.*liveness probe\[0\].*last=fail \(HTTP 503\).*last_observed_at=([0-9]+).*/\1/p' \
      "$replacement_describe" | tail -n 1)"
    if [[ "$liveness_failure_observed_at" =~ ^[0-9]+$ ]] \
      && [[ "$liveness_failure_observed_at" != "$last_liveness_failure_observed_at" ]]; then
      liveness_failure_count=$((liveness_failure_count + 1))
      last_liveness_failure_observed_at="$liveness_failure_observed_at"
    fi
    local allocation_state restart_count startup_observed_at
    allocation_state="$(first_service_alloc_state <"$replacement_describe")"
    restart_count="$(first_service_restart_count <"$replacement_describe")"
    startup_observed_at="$(first_service_startup_observed_at <"$replacement_describe")"
    if [[ "$terminal_seen" -eq 0 ]] \
      && [[ "$liveness_failure_count" -ge "$failure_threshold" ]] \
      && [[ "$allocation_state" == "Terminated" || "$allocation_state" == "Failed" ]] \
      && [[ "$restart_count" == "0" ]]; then
      cp -- "$replacement_describe" "$terminal_describe"
      terminal_seen=1
    fi
    if [[ "$terminal_seen" -eq 1 ]] \
      && [[ "$allocation_state" == "Running" ]] \
      && [[ "$restart_count" =~ ^[1-9][0-9]*$ ]] \
      && grep -Fq 'last terminated:' \
        "$replacement_describe" \
      && grep -Eqi 'startup probe\[0\].*last=pass|last=pass.*startup probe\[0\]' \
        "$replacement_describe" \
      && grep -Eqi 'readiness probe\[0\].*last=pass|last=pass.*readiness probe\[0\]' \
        "$replacement_describe" \
      && [[ "$startup_observed_at" =~ ^[0-9]+$ ]] \
      && [[ "$startup_observed_at" != "$baseline_startup_observed_at" ]] \
      && [[ "$(first_service_alloc_started_at <"$replacement_describe")" \
        != "$baseline_started_at" ]]; then
      LIVENESS_FAILURE_COUNT="$liveness_failure_count"
      return 0
    fi
    sleep 0.2
  done
  LIVENESS_FAILURE_COUNT="$liveness_failure_count"
  return 1
}

run_liveness_restart() {
  require_native_metal
  local command
  for command in awk cargo cloud-hypervisor cp find findmnt grep ip keyctl losetup mktemp \
    script setsid sleep sort timeout tr; do
    require_command "$command"
  done
  "$PREPARE" check-source
  [[ ! -e "$OUTPUT_ROOT" ]] || die "refusing to overwrite pre-existing materialization: $OUTPUT_ROOT"
  snapshot_before
  CASE_LABEL="E12"
  SERVICE_ID="service-vm-liveness-restart"
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

  local service_stream="$OUTPUT_ROOT/e12-service-stream.log"
  local service_command
  printf -v service_command 'env OVERDRIVE_CONFIG_DIR=%q %q deploy %q' \
    "$CONFIG_DIR" "$BIN" "$EXAMPLE_DIR/liveness-restart.toml"
  SERVICE_DEPLOYED=1
  if ! bounded 150s script -q -e -c "$service_command" "$service_stream" >/dev/null; then
    cat "$service_stream" >&2
    cat "$OUTPUT_ROOT/serve.log" >&2
    die "E12 Service streaming deployment did not complete"
  fi
  local accepted stable
  accepted="$(grep -n -m1 'Accepted' "$service_stream" | cut -d: -f1 || true)"
  stable="$(grep -ni -m1 'stable' "$service_stream" | cut -d: -f1 || true)"
  if [[ -z "$accepted" || -z "$stable" || "$accepted" -ge "$stable" ]]; then
    cat "$service_stream" >&2
    die "E12 single service deployment did not render Accepted before Stable"
  fi
  grep -Eq 'startup.*(index|probe).*0|probe.*0.*startup' "$service_stream" \
    || die "E12 Stable render did not name startup probe index 0"
  grep -Fq ' is stable ' "$service_stream" \
    || die "E12 Service did not render the stable lifecycle state"
  cat "$service_stream"

  local before_describe="$OUTPUT_ROOT/e12-before-describe.log"
  wait_for_service_observations "$before_describe" \
    || { cat "$before_describe" >&2; cat "$OUTPUT_ROOT/serve.log" >&2; \
         die "E12 VM Service did not report initial startup/readiness Pass"; }
  grep -Eqi 'liveness probe\[0\].*last=pass|last=pass.*liveness probe\[0\]' \
    "$before_describe" \
    || die "E12 baseline did not report a passing liveness probe"
  [[ "$(first_service_alloc_state <"$before_describe")" == "Running" ]] \
    || die "E12 baseline Service was not Running"
  [[ "$(first_service_restart_count <"$before_describe")" == "0" ]] \
    || die "E12 baseline unexpectedly restarted before liveness failure"
  local original_alloc
  original_alloc="$(first_service_alloc_id <"$before_describe")"
  [[ "$original_alloc" =~ ^alloc- ]] || die "E12 baseline omitted its allocation identity"
  local baseline_startup_observed_at baseline_started_at failure_threshold
  baseline_startup_observed_at="$(first_service_startup_observed_at <"$before_describe")"
  [[ "$baseline_startup_observed_at" =~ ^[0-9]+$ ]] \
    || die "E12 baseline omitted startup observation timestamp"
  baseline_started_at="$(first_service_alloc_started_at <"$before_describe")"
  [[ -n "$baseline_started_at" ]] || die "E12 baseline omitted allocation start timestamp"
  failure_threshold="$(read_liveness_failure_threshold)"
  [[ "$failure_threshold" =~ ^[1-9][0-9]*$ ]] \
    || die "E12 liveness fixture omitted a positive failure threshold"
  cat "$before_describe"

  local terminal_describe="$OUTPUT_ROOT/e12-terminal-describe.log"
  local replacement_describe="$OUTPUT_ROOT/e12-after-describe.log"
  wait_for_liveness_restart "$terminal_describe" "$replacement_describe" \
    "$failure_threshold" "$baseline_startup_observed_at" "$baseline_started_at" \
    || { cat "$terminal_describe" >&2; cat "$replacement_describe" >&2; \
         cat "$OUTPUT_ROOT/serve.log" >&2; \
         die "E12 liveness failure did not produce an ordinary replacement"; }
  grep -Fq 'HTTP 503' "$terminal_describe" \
    || die "E12 liveness terminal observation omitted the failed response status"
  grep -Eqi 'liveness probe\[0\].*last=fail|last=fail.*liveness probe\[0\]' \
    "$terminal_describe" \
    || die "E12 terminal capture omitted liveness probe failure"
  grep -Eqi 'liveness probe\[0\].*last=(pass|fail)' "$replacement_describe" \
    || die "E12 replacement omitted its liveness observation"
  local replacement_alloc
  replacement_alloc="$(first_service_alloc_id <"$replacement_describe")"
  [[ "$replacement_alloc" == "$original_alloc" ]] \
    || die "E12 restart changed the allocation identity unexpectedly"
  grep -Fq 'last terminated:' "$replacement_describe" \
    || die "E12 replacement did not retain the prior terminal observation"
  ! grep -Eqi 'last terminated:.*readiness|readiness probe\[0\].*last=fail' \
    "$terminal_describe" "$replacement_describe" \
    || die "E12 readiness was incorrectly attributed as the restart owner"
  cat "$terminal_describe"
  cat "$replacement_describe"

  local restart_count
  restart_count="$(first_service_restart_count <"$replacement_describe")"
  local terminal_state terminal_started_at replacement_started_at replacement_startup_observed_at
  terminal_state="$(first_service_alloc_state <"$terminal_describe")"
  [[ "$terminal_state" == "Terminated" || "$terminal_state" == "Failed" ]] \
    || die "E12 terminal capture did not report a terminal allocation state"
  terminal_started_at="$(first_service_alloc_started_at <"$terminal_describe")"
  replacement_started_at="$(first_service_alloc_started_at <"$replacement_describe")"
  replacement_startup_observed_at="$(first_service_startup_observed_at <"$replacement_describe")"
  [[ -n "$terminal_started_at" && -n "$replacement_started_at" ]] \
    || die "E12 terminal/replacement capture omitted allocation start timestamps"
  [[ "$replacement_started_at" != "$baseline_started_at" ]] \
    || die "E12 replacement retained the pre-restart allocation start timestamp"
  [[ "$replacement_startup_observed_at" =~ ^[0-9]+$ ]] \
    || die "E12 replacement omitted startup observation timestamp"
  [[ "$replacement_startup_observed_at" != "$baseline_startup_observed_at" ]] \
    || die "E12 replacement retained the pre-restart startup observation"
  local after_probe_status after_response
  if grep -Eqi 'liveness probe\[0\].*last=fail|last=fail.*liveness probe\[0\]' \
    "$replacement_describe"; then
    after_probe_status='fail'
    after_response='HTTP 503'
  elif grep -Eqi 'liveness probe\[0\].*last=pass|last=pass.*liveness probe\[0\]' \
    "$replacement_describe"; then
    after_probe_status='pass'
    after_response='HTTP 204'
  else
    die "E12 replacement liveness observation was not pass or fail"
  fi
  local terminal_reason
  if [[ "$LIVENESS_FAILURE_COUNT" -ge "$failure_threshold" ]] \
    && [[ "$(first_service_alloc_state <"$terminal_describe")" == "Terminated" \
      || "$(first_service_alloc_state <"$terminal_describe")" == "Failed" ]] \
    && grep -Eqi 'liveness probe\[0\].*last=fail|last=fail.*liveness probe\[0\]' \
      "$terminal_describe"; then
    terminal_reason='liveness-probe'
  else
    die "E12 terminal attribution lacked the observed liveness threshold transition"
  fi
  echo '--- E12 ledger begin ---'
  printf 'phase\tallocation_id\tstate\trestarts\tprobe_role\tprobe_status\tterminal_reason\tresponse\tstartup_observed_at\tstarted_at\tthreshold_failures\n'
  printf 'before\t%s\t%s\t%s\tliveness\tpass\tnone\tHTTP 204\t%s\t%s\t0\n' \
    "$original_alloc" "$(first_service_alloc_state <"$before_describe")" \
    "$(first_service_restart_count <"$before_describe")" \
    "$baseline_startup_observed_at" "$baseline_started_at"
  printf 'terminal\t%s\t%s\t%s\tliveness\tfail\t%s\tHTTP 503\t%s\t%s\t%s\n' \
    "$original_alloc" "$(first_service_alloc_state <"$terminal_describe")" \
    "$(first_service_restart_count <"$terminal_describe")" "$terminal_reason" \
    "$(first_service_startup_observed_at <"$terminal_describe")" \
    "$terminal_started_at" "$LIVENESS_FAILURE_COUNT"
  printf 'after\t%s\t%s\t%s\tliveness\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$replacement_alloc" "$(first_service_alloc_state <"$replacement_describe")" \
    "$restart_count" "$after_probe_status" "$terminal_reason" "$after_response" \
    "$replacement_startup_observed_at" "$replacement_started_at" "$LIVENESS_FAILURE_COUNT"
  echo '--- E12 ledger end ---'
  echo 'E12 PASS: liveness stop, ordinary same-ID replacement, no readiness restart, no dead revival'

  cleanup
}

copy_case_captures() {
  [[ -n "${SVM_E08_CASE_CAPTURE_DIR:-}" ]] || return 0
  local mode=0700
  [[ -z "${SVM_E10_CAPTURE_TOKEN:-}" ]] || mode=0755
  install -d -m "$mode" "$SVM_E08_CASE_CAPTURE_DIR"
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
        stop_recovered_service_after_startup_failure "$SERVICE_ID" || failed=1
        ;;
      *)
        echo "svm-e08 run: unknown case cleanup policy: $CASE_CLEANUP_POLICY" >&2
        failed=1
        ;;
    esac
  fi
  if [[ -n "${SVM_E10_CAPTURE_TOKEN:-}" ]]; then
    wait_for_e10_named_runtime_cleanup "$SERVICE_ID" \
      "$OUTPUT_ROOT/current-session-owned-resources.log" || failed=1
  else
    wait_for_owned_runtime_cleanup || failed=1
  fi
  stop_serve || failed=1
  copy_case_captures || failed=1
  [[ "$PREPARED" -eq 0 ]] || bounded 45s "$PREPARE" cleanup || failed=1
  if [[ -n "$SNAPSHOT_DIR" ]]; then
    if [[ -n "${SVM_E08_CASE_CAPTURE_DIR:-}" ]]; then
      if [[ -n "${SVM_E10_CAPTURE_TOKEN:-}" ]]; then
        capture_e10_resource_snapshot "$SERVICE_ID" \
          >"$SVM_E08_CASE_CAPTURE_DIR/cleanup-resources-after.log"
      else
        capture_resource_snapshot >"$SVM_E08_CASE_CAPTURE_DIR/cleanup-resources-after.log"
      fi
      if [[ -n "${SVM_E10_CAPTURE_TOKEN:-}" ]]; then
        # Keep the whole-host delta as diagnostic context, but E10's oracle is
        # the absolute absence of this cell's named resources. Unrelated host
        # activity must not manufacture a cleanup failure for this allocation.
        report_cleanup_deltas \
          >"$SVM_E08_CASE_CAPTURE_DIR/cleanup-resource-delta.log" 2>&1 || true
        if e10_named_runtime_resources_absent "$SERVICE_ID" \
          "$SVM_E08_CASE_CAPTURE_DIR/current-session-owned-resources.log" \
          && [[ -z "$SERVE_PID" ]] \
          && [[ -z "$(probe_loops)" ]] && [[ -z "$(probe_mounts)" ]] \
          && grep -Fxq 'absent' \
            "$SVM_E08_CASE_CAPTURE_DIR/cleanup-resources-after.log"; then
          local observed_count
          observed_count="$(awk -F= '$1 != "allocation" { count++ } END { print count + 0 }' \
            "$SVM_E08_CASE_CAPTURE_DIR/current-session-owned-resources.log")"
          printf 'E10 named cleanup: observed-before-stop=%s post-stop=absent serve=stopped preparation=absent\n' \
            "$observed_count" \
            >"$SVM_E08_CASE_CAPTURE_DIR/cleanup-resource-complement.log"
          cat "$SVM_E08_CASE_CAPTURE_DIR/cleanup-resource-complement.log"
        else
          echo 'E10 named cleanup: residual owned resource' \
            >"$SVM_E08_CASE_CAPTURE_DIR/cleanup-resource-complement.log"
          cat "$SVM_E08_CASE_CAPTURE_DIR/cleanup-resource-complement.log" >&2
          failed=1
        fi
      elif ! report_cleanup_deltas \
        >"$SVM_E08_CASE_CAPTURE_DIR/cleanup-resource-delta.log" 2>&1; then
        cat "$SVM_E08_CASE_CAPTURE_DIR/cleanup-resource-delta.log" >&2
        failed=1
      else
        cat "$SVM_E08_CASE_CAPTURE_DIR/cleanup-resource-delta.log"
      fi
    else
      report_cleanup_deltas || failed=1
    fi
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
      if [[ "$CASE_CLEANUP_POLICY" == "preserve-startup-failure" ]]; then
        wait_for_e10_recovered_running "$SERVICE_ID" "$describe"
      else
        [[ "$(first_service_alloc_state <"$describe")" == "Failed" ]]
      fi
      ;;
    *) die "unknown expected Service result: $expected" ;;
  esac
}

wait_for_e10_recovered_running() {
  local id="$1"
  local output="$2"
  local observations="$OUTPUT_ROOT/${id}-recovery-observations.log"
  local alloc_id
  alloc_id="$(first_service_alloc_id <"$output")"
  [[ -n "$alloc_id" ]] || return 1
  local deadline=$((SECONDS + 45))
  : >"$observations"
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    local state restarts
    state="$(first_service_alloc_state <"$output")"
    restarts="$(first_service_restart_count <"$output")"
    {
      printf 'observed_utc=%s state=%s restarts=%s\n' \
        "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${state:-unrecognized}" "${restarts:-unrecognized}"
      cat "$output"
    } >>"$observations"
    if [[ "$state" == "Running" ]] \
      && [[ "$(first_service_alloc_id <"$output")" == "$alloc_id" ]] \
      && [[ "$restarts" =~ ^[1-9][0-9]*$ ]] \
      && has_prior_failed_snapshot <"$output" \
      && grep -Eqi 'startup probe\[0\].*last=fail|last=fail.*startup probe\[0\]' "$output"; then
      cp -- "$observations" "$OUTPUT_ROOT/recovery-observations.log"
      return 0
    fi
    bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
      "$BIN" workload describe "$id" >"$output" 2>&1 || true
  done
  cat "$observations" >&2
  echo "svm-e08 run: startup-failed Service did not settle on its recovered Running incarnation: $id" >&2
  return 1
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
  if [[ -n "${SVM_E08_CASE_CAPTURE_DIR:-}" ]]; then
    local capture_mode=0700
    [[ -z "${SVM_E10_CAPTURE_TOKEN:-}" ]] || capture_mode=0755
    install -d -m "$capture_mode" "$SVM_E08_CASE_CAPTURE_DIR"
    capture_resource_snapshot >"$SVM_E08_CASE_CAPTURE_DIR/cleanup-resources-before.log"
    printf '%s\n' "${SVM_E10_CAPTURE_TOKEN:-not-an-e10-capture}" \
      >"$SVM_E08_CASE_CAPTURE_DIR/capture-token.log"
  fi
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
  local service_stdout="$OUTPUT_ROOT/service-deploy-stdout.log"
  local service_stderr="$OUTPUT_ROOT/service-deploy-stderr.log"
  local service_exit="$OUTPUT_ROOT/service-deploy-exit.log"
  local service_command
  printf -v service_command 'env OVERDRIVE_CONFIG_DIR=%q %q deploy %q' \
    "$CONFIG_DIR" "$BIN" "$EXAMPLE_DIR/$spec"
  printf '%s\n' "$service_command" >"$OUTPUT_ROOT/service-deploy-command.log"
  SERVICE_DEPLOYED=1
  local service_deploy_rc
  set +e
  bounded 150s script -q -e -c "$service_command" "$service_stream" \
    >"$service_stdout" 2>"$service_stderr"
  service_deploy_rc=$?
  set -e
  printf '%s\n' "$service_deploy_rc" >"$service_exit"
  if [[ "$service_deploy_rc" -ne 0 ]]; then
    if [[ "$expected" != startup-failed ]]; then
      cat "$service_stream" >&2
      die "healthy Service streaming deployment did not complete"
    fi
  elif [[ "$expected" == startup-failed ]]; then
    cat "$service_stream" >&2
    die "startup-failed Service streaming deployment exited zero"
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
  cp -- "$service_describe" "$OUTPUT_ROOT/before-stop-describe.log"
  if [[ -n "${SVM_E10_CAPTURE_TOKEN:-}" ]]; then
    capture_e10_resource_snapshot "$SERVICE_ID" \
      >"$OUTPUT_ROOT/current-session-resources.log"
  else
    capture_resource_snapshot >"$OUTPUT_ROOT/current-session-resources.log"
  fi
  if [[ -n "${SVM_E10_CAPTURE_TOKEN:-}" ]]; then
    capture_e10_current_owned_resources "$SERVICE_ID" \
      >"$OUTPUT_ROOT/current-session-owned-resources.log"
  fi
  {
    printf 'allocation_id=%s\n' "$(first_service_alloc_id <"$service_describe")"
    printf 'current_state=%s\n' "$(first_service_alloc_state <"$service_describe")"
    printf 'restart_count=%s\n' "$(first_service_restart_count <"$service_describe")"
    printf 'current_row=%s\n' \
      "$(awk '$1 ~ /^alloc-/ { print $1, $2, $3, $4; exit }' "$service_describe")"
    printf 'prior_failure=%s\n' \
      "$(first_service_last_terminated_line <"$service_describe")"
    printf 'startup_probe=%s\n' \
      "$(first_service_startup_probe_line <"$service_describe")"
  } >"$OUTPUT_ROOT/current-session-observation.log"
  cat "$service_describe"
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
    install -d -m 0755 "$SVM_E08_MATRIX_CAPTURE_ROOT/$label"
    cp -- "$transcript" "$SVM_E08_MATRIX_CAPTURE_ROOT/$label/case-transcript.log"
    cat "$transcript" >&2
    return 1
  fi
  cp -- "$transcript" "$SVM_E08_MATRIX_CAPTURE_ROOT/$label/case-transcript.log"
}

require_matrix_capture_root() {
  if [[ -z "${SVM_E08_MATRIX_CAPTURE_ROOT:-}" && -n "${EVIDENCE_DIR:-}" ]]; then
    export SVM_E08_MATRIX_CAPTURE_ROOT="$EVIDENCE_DIR"
  fi
  [[ -n "${SVM_E08_MATRIX_CAPTURE_ROOT:-}" ]] \
    || die "matrix capture root is required"
  local mode=0700
  [[ -z "${SVM_E10_CAPTURE_TOKEN:-}" ]] || mode=0755
  install -d -m "$mode" "$SVM_E08_MATRIX_CAPTURE_ROOT"
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
  printf 'driver\tstatus\tdeploy_exit\tallocation_id\tbefore_stop\trestart_before\tfailure_surface\tafter_stop\trestart_after\tsettled_state\ttrajectory\tprobe_result\tstdout_bytes\tstderr_bytes\tdescribe_bytes\tsentinel_deploy_stdout\tsentinel_deploy_stderr\tsentinel_describe\tsentinel_probe\tcleanup\n' >"$ledger"
  local driver status spec id expected label transcript describe sentinel
  local cleanup_policy final_describe settled_describe terminal trajectory
  local alloc_id before_state before_restarts after_restarts settled_state failure_surface
  local deploy_stdout deploy_stderr deploy_exit deploy_stdout_sentinel deploy_stderr_sentinel
  local describe_sentinel probe_sentinel probe_line
  for driver in exec vm; do
    for status in 204 302 404 503; do
      spec="http-${driver}-${status}.toml"
      id="service-${driver}-http-${status}"
      expected=startup-failed
      cleanup_policy=preserve-startup-failure
      [[ "$status" == 204 ]] && expected=stable
      [[ "$status" == 204 ]] && cleanup_policy=operator-stop
      label="e10-${driver}-${status}"
      settled_describe=''
      capture_isolated_case "$label" "$spec" "$id" "$expected" "" "" "$cleanup_policy"
      transcript="$SVM_E08_MATRIX_CAPTURE_ROOT/$label.transcript"
      describe="$SVM_E08_MATRIX_CAPTURE_ROOT/$label/service-describe.log"
      deploy_stdout="$SVM_E08_MATRIX_CAPTURE_ROOT/$label/service-deploy-stdout.log"
      deploy_stderr="$SVM_E08_MATRIX_CAPTURE_ROOT/$label/service-deploy-stderr.log"
      deploy_exit="$(tr -d '[:space:]' \
        <"$SVM_E08_MATRIX_CAPTURE_ROOT/$label/service-deploy-exit.log")"
      alloc_id="$(first_service_alloc_id <"$describe")"
      before_state="$(first_service_alloc_state <"$describe")"
      before_restarts="$(first_service_restart_count <"$describe")"
      if [[ "$status" == 204 ]]; then
        [[ "$deploy_exit" == 0 ]] \
          || die "E10 healthy deploy did not exit zero: $driver/$status"
        grep -Fq ' is stable ' "$transcript"
        [[ "$before_state" == "Running" && "$before_restarts" == "0" ]] \
          || die "E10 healthy Service was not a first-incarnation Running allocation: $driver/$status"
        final_describe="$SVM_E08_MATRIX_CAPTURE_ROOT/$label/${id}-stopped.log"
        [[ -f "$final_describe" ]] || die "E10 missing operator-stop describe for $driver/$status"
        terminal="$(first_service_alloc_state <"$final_describe")"
        after_restarts="$(first_service_restart_count <"$final_describe")"
        [[ "$terminal" == "Terminated" ]] \
          || die "E10 running Service did not become Terminated after stop: $driver/$status"
        [[ "$(first_service_alloc_id <"$final_describe")" == "$alloc_id" \
          && "$after_restarts" == "$before_restarts" ]] \
          || die "E10 healthy Service stop changed allocation identity or restart count: $driver/$status"
        settled_state="$terminal"
        failure_surface=none
        trajectory=operator-stop-running
      else
        [[ "$deploy_exit" == 1 ]] \
          || die "E10 startup-failed deploy did not exit one: $driver/$status"
        grep -Fq "HTTP ${status}" "$transcript" \
          || die "E10 deploy stream omitted HTTP status $status: $driver/$status"
        grep -Fq "HTTP ${status}" "$describe" \
          || die "E10 describe omitted HTTP status $status: $driver/$status"
        if [[ "$status" == 302 ]]; then
          grep -Fq 'HTTP 302 (redirect not followed)' "$transcript" \
            || die "E10 deploy stream did not classify the redirect without following it: $driver/$status"
          grep -Fq 'HTTP 302 (redirect not followed)' "$describe" \
            || die "E10 describe did not retain the redirect classification: $driver/$status"
        fi
        final_describe="$SVM_E08_MATRIX_CAPTURE_ROOT/$label/${id}-preserved-failure.log"
        settled_describe="$SVM_E08_MATRIX_CAPTURE_ROOT/$label/${id}-preserved-failure-settled.log"
        [[ -f "$final_describe" ]] || die "E10 missing preserved-failure describe for $driver/$status"
        [[ -f "$settled_describe" ]] || die "E10 missing post-cleanup settled describe for $driver/$status"
        terminal="$(first_service_alloc_state <"$final_describe")"
        after_restarts="$(first_service_restart_count <"$final_describe")"
        settled_state="$(first_service_alloc_state <"$settled_describe")"
        if [[ "$before_state" == "Running" ]] \
          && [[ "$before_restarts" =~ ^[1-9][0-9]*$ ]] \
          && has_prior_failed_snapshot <"$describe" \
          && [[ "$terminal" == "Terminated" && "$settled_state" == "Terminated" ]] \
          && [[ "$after_restarts" == "$before_restarts" ]] \
          && [[ "$(first_service_alloc_id <"$final_describe")" == "$alloc_id" ]] \
          && [[ "$(first_service_alloc_id <"$settled_describe")" == "$alloc_id" ]] \
          && [[ "$(first_service_last_terminated_line <"$final_describe")" \
            == "$(first_service_last_terminated_line <"$describe")" ]] \
          && [[ "$(first_service_last_terminated_line <"$settled_describe")" \
            == "$(first_service_last_terminated_line <"$describe")" ]] \
          && grep -Fq '    reason: stopped' "$final_describe" \
          && grep -Fq '    reason: stopped' "$settled_describe"; then
          failure_surface=deploy-exit-1+prior-failed+probe
          trajectory=recovered-running-operator-stop
        else
          die "E10 startup-failure cleanup followed an unrecognized trajectory: $driver/$status"
        fi
      fi
      grep -Eq '^E10 named cleanup: observed-before-stop=[0-9]+ post-stop=absent serve=stopped preparation=absent$' \
        "$SVM_E08_MATRIX_CAPTURE_ROOT/$label/cleanup-resource-complement.log" \
        || die "E10 named cleanup complement failed for $driver/$status"
      sentinel='SVM-E10-FAILURE-BODY-MUST-NOT-LEAK'
      deploy_stdout_sentinel="$({ grep -Foh "$sentinel" "$deploy_stdout" || true; } \
        | wc -l | tr -d ' ')"
      deploy_stderr_sentinel="$({ grep -Foh "$sentinel" "$deploy_stderr" || true; } \
        | wc -l | tr -d ' ')"
      describe_sentinel="$({ grep -Foh "$sentinel" "$describe" "$final_describe" \
        "$SVM_E08_MATRIX_CAPTURE_ROOT/$label/post-cleanup-describe.log" || true; } \
        | wc -l | tr -d ' ')"
      probe_line="$(first_service_startup_probe_line <"$describe")"
      probe_sentinel="$({ grep -Foh "$sentinel" <<<"$probe_line" || true; } \
        | wc -l | tr -d ' ')"
      [[ "$deploy_stdout_sentinel" -eq 0 && "$deploy_stderr_sentinel" -eq 0 \
        && "$describe_sentinel" -eq 0 && "$probe_sentinel" -eq 0 ]] \
        || die "E10 sentinel leaked for $driver/$status"
      printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\tstatus-%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\tnamed-absent\n' \
        "$driver" "$status" "$deploy_exit" "$alloc_id" "$before_state" "$before_restarts" \
        "$failure_surface" "$terminal" "$after_restarts" "$settled_state" \
        "$trajectory" "$status" \
        "$(wc -c <"$deploy_stdout" | tr -d ' ')" \
        "$(wc -c <"$deploy_stderr" | tr -d ' ')" \
        "$(wc -c <"$describe" | tr -d ' ')" \
        "$deploy_stdout_sentinel" "$deploy_stderr_sentinel" \
        "$describe_sentinel" "$probe_sentinel" >>"$ledger"
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
      readiness-recovery) run_readiness_recovery ;;
      case) shift 2; run_case "$@" ;;
      liveness-restart)
        run_liveness_restart
        ;;
      *) die 'usage: run-example.sh run healthy|tcp-truthfulness-100|http-status-cross-driver|readiness-recovery|liveness-restart|zero-probes' ;;
    esac
    ;;
  *)
    die 'usage: run-example.sh check-source|run healthy'
    ;;
esac

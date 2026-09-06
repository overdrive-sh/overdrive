#!/usr/bin/env bash
# Run the checked-in E08 journey on its qualified native-metal substrate.
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly EXAMPLE_DIR
readonly PREPARE="$EXAMPLE_DIR/prepare.sh"
readonly OUTPUT_ROOT="/srv/vm/overdrive-testing/svm-e08"
readonly CONFIG_DIR="$OUTPUT_ROOT/config"
readonly DATA_DIR="$OUTPUT_ROOT/data"
readonly CREDS_DIR="$OUTPUT_ROOT/credentials"
readonly BIN="$(cd "$EXAMPLE_DIR/../.." && pwd)/target/debug/overdrive"
readonly BIND="127.0.0.1:7644"
readonly SERVICE_ID="service-vm-e08"
readonly CLIENT_ID="service-vm-e08-client"
readonly KEK_DESCRIPTION="overdrive:ca:kek:overdrive-ca-root"

SERVE_PID=""
PREPARED=0
SERVICE_DEPLOYED=0
CLIENT_DEPLOYED=0
SNAPSHOT_DIR=""

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

  local service_deploy="$OUTPUT_ROOT/service-deploy.log"
  SERVICE_DEPLOYED=1
  # First submit through the public detached lane to retain the operator's
  # Accepted acknowledgement. The following PTY-bound resubmission observes
  # the same idempotent Service through its public Stable terminal stream.
  if ! bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" deploy --detach "$EXAMPLE_DIR/service.toml" >"$service_deploy" 2>&1; then
    cat "$service_deploy" >&2
    die "Service detached deployment was not accepted"
  fi
  local accepted
  accepted="$(grep -n -m1 'Accepted' "$service_deploy" | cut -d: -f1 || true)"
  if [[ -z "$accepted" ]]; then
    cat "$service_deploy" >&2
    die "service deploy did not render Accepted"
  fi

  local service_stream="$OUTPUT_ROOT/service-stream.log"
  local service_command
  printf -v service_command 'env OVERDRIVE_CONFIG_DIR=%q %q deploy %q' \
    "$CONFIG_DIR" "$BIN" "$EXAMPLE_DIR/service.toml"
# `overdrive deploy` selects Service event stream only when stdout is a
# terminal. Run this idempotent resubmission through the platform's PTY
# recorder so the journey captures the same Service's Stable terminal instead
# of silently switching to the detached JSON-ack lane because the harness
# redirects its output to a file.
  if ! bounded 150s script -q -e -c "$service_command" "$service_stream" >/dev/null; then
    cat "$service_stream" >&2
    cat "$OUTPUT_ROOT/serve.log" >&2
    die "Service streaming deployment did not complete"
  fi
  cat "$service_stream" >>"$service_deploy"

  local service_describe="$OUTPUT_ROOT/service-describe.log"
  if ! wait_for_service_observations "$service_describe"; then
    cat "$service_describe" >&2
    cat "$OUTPUT_ROOT/serve.log" >&2
    die "VM Service did not report healthy guest TCP and HTTP observations within 90 seconds"
  fi
  {
    printf '\n# post-acceptance Service state\n'
    cat "$service_describe"
  } >>"$service_deploy"
  local stable
  stable="$(grep -ni -m1 'stable' "$service_deploy" | cut -d: -f1 || true)"
  if [[ -z "$stable" || "$accepted" -ge "$stable" ]]; then
    cat "$service_deploy" >&2
    die "service journey did not render Accepted before Stable"
  fi
  grep -Eq 'startup.*(index|probe).*0|probe.*0.*startup' "$service_deploy" \
    || die "Stable render did not name startup probe index 0"
  cat "$service_deploy"
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

case "${1:-}" in
  check-source)
    "$PREPARE" check-source
    ;;
  run)
    case "${2:-}" in
      healthy) run_healthy ;;
      tcp-truthfulness-100|http-status-cross-driver|readiness-recovery|liveness-restart|zero-probes)
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

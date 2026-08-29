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
readonly LEASE_OWNER="${OVERDRIVE_METAL_OWNER_PATH:-/run/lock/overdrive-metal-shared.owner}"

SERVE_PID=""
PREPARED=0
CALLER_DEPLOYED=0
CALLEE_DEPLOYED=0
CLEANUP_FAILED=0
SNAP_DIR=""

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
    "$BIN" job stop "$id" >/dev/null 2>&1 || true
}

terminate_serve() {
  [[ -n "$SERVE_PID" ]] || return 0
  if kill -0 "$SERVE_PID" 2>/dev/null; then
    kill -TERM "$SERVE_PID" 2>/dev/null || true
    local remaining=50
    while kill -0 "$SERVE_PID" 2>/dev/null && [[ "$remaining" -gt 0 ]]; do
      sleep 0.1
      remaining=$((remaining - 1))
    done
    if kill -0 "$SERVE_PID" 2>/dev/null; then
      kill -KILL "$SERVE_PID" 2>/dev/null || true
    fi
  fi
  wait "$SERVE_PID" 2>/dev/null || true
  SERVE_PID=""
}

probe_ovd_links() {
  ip -o link show | awk -F': ' '{print $2}' | sed 's/@.*//' | grep '^ovd-' | sort -u || true
}

probe_ovd_netns() {
  ip netns list | awk '{print $1}' | grep '^ovd-ns-' | sort -u || true
}

probe_example_pids() {
  local proc cmdline
  for proc in /proc/[0-9]*; do
    [[ -r "$proc/cmdline" ]] || continue
    cmdline="$(tr '\0' ' ' <"$proc/cmdline" 2>/dev/null || true)"
    case "$cmdline" in
      *cloud-hypervisor*"${CALLER_ID}"*|*"$OUTPUT_ROOT/bin/e07-callee"*)
        basename "$proc"
        ;;
    esac
  done | sort -n
}

new_only() {
  local before_file="$1"
  shift
  comm -13 "$before_file" <("$@")
}

snapshot_shared_state() {
  SNAP_DIR="$(mktemp -d /tmp/gti-e07-state.XXXXXX)"
  probe_ovd_links >"$SNAP_DIR/links"
  probe_ovd_netns >"$SNAP_DIR/netns"
  probe_example_pids >"$SNAP_DIR/pids"
}

cleanup_shared_delta() {
  [[ -n "$SNAP_DIR" && -d "$SNAP_DIR" ]] || return 0
  local item

  while IFS= read -r item; do
    [[ -n "$item" ]] || continue
    bounded 10s kill -TERM "$item" 2>/dev/null || true
    bounded 10s kill -KILL "$item" 2>/dev/null || true
  done < <(new_only "$SNAP_DIR/pids" probe_example_pids)

  while IFS= read -r item; do
    case "$item" in
      ovd-ns-*) bounded 10s ip netns delete "$item" 2>/dev/null || CLEANUP_FAILED=1 ;;
      *) CLEANUP_FAILED=1 ;;
    esac
  done < <(new_only "$SNAP_DIR/netns" probe_ovd_netns)

  while IFS= read -r item; do
    case "$item" in
      ovd-*) bounded 10s ip link delete "$item" 2>/dev/null || CLEANUP_FAILED=1 ;;
      *) CLEANUP_FAILED=1 ;;
    esac
  done < <(new_only "$SNAP_DIR/links" probe_ovd_links)
}

cleanup_runner_owned_paths() {
  local path
  if [[ -d /run/overdrive/vm ]]; then
    while IFS= read -r -d '' path; do
      case "$path" in
        /run/overdrive/vm/alloc-gti-e07-caller-*)
          bounded 10s rm -rf -- "$path" || CLEANUP_FAILED=1
          ;;
        *) CLEANUP_FAILED=1 ;;
      esac
    done < <(find /run/overdrive/vm -mindepth 1 -maxdepth 1 \
      -name "alloc-${CALLER_ID}-*" -print0)
  fi

  if [[ -d /sys/fs/cgroup/overdrive.slice/workloads.slice ]]; then
    while IFS= read -r -d '' path; do
      case "$path" in
        /sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-gti-e07-caller-*|\
        /sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-gti-e07-callee-*)
          echo 1 >"$path/cgroup.kill" 2>/dev/null || true
          bounded 10s rmdir "$path" 2>/dev/null || CLEANUP_FAILED=1
          ;;
        *) CLEANUP_FAILED=1 ;;
      esac
    done < <(find /sys/fs/cgroup/overdrive.slice/workloads.slice \
      -mindepth 1 -maxdepth 1 \
      \( -name "alloc-${CALLER_ID}-*" -o -name "alloc-${CALLEE_ID}-*" \) \
      -print0)
  fi
}

cleanup() {
  local incoming_rc=$?
  trap - EXIT HUP INT TERM

  [[ "$CALLER_DEPLOYED" -eq 1 ]] && stop_exact_workload "$CALLER_ID"
  # The current public stop endpoint is exposed by the `job stop` command but
  # accepts the canonical WorkloadId for this Service as well.
  [[ "$CALLEE_DEPLOYED" -eq 1 ]] && stop_exact_workload "$CALLEE_ID"
  terminate_serve
  cleanup_shared_delta
  cleanup_runner_owned_paths

  # E07 launches no tcpdump/AF_PACKET observer at all. The capture-process
  # cleanup set is deliberately empty; D7 capture belongs to Rust tests.
  if [[ "$PREPARED" -eq 1 ]]; then
    bounded 45s "$PREPARE" cleanup || CLEANUP_FAILED=1
  fi

  if [[ -d /run/overdrive/vm ]] \
    && find /run/overdrive/vm -mindepth 1 -maxdepth 1 -name "alloc-${CALLER_ID}-*" -print -quit \
      | grep -q .; then
    echo "gti-e07 run: runner-owned VM run-directory residue remains" >&2
    CLEANUP_FAILED=1
  fi
  if [[ -d /sys/fs/cgroup/overdrive.slice/workloads.slice ]] \
    && find /sys/fs/cgroup/overdrive.slice/workloads.slice -mindepth 1 -maxdepth 1 \
      \( -name "alloc-${CALLER_ID}-*" -o -name "alloc-${CALLEE_ID}-*" \) \
      -print -quit | grep -q .; then
    echo "gti-e07 run: runner-owned allocation cgroup residue remains" >&2
    CLEANUP_FAILED=1
  fi
  if [[ -n "$SNAP_DIR" && -d "$SNAP_DIR" ]]; then
    if [[ -n "$(new_only "$SNAP_DIR/pids" probe_example_pids)" \
       || -n "$(new_only "$SNAP_DIR/netns" probe_ovd_netns)" \
       || -n "$(new_only "$SNAP_DIR/links" probe_ovd_links)" ]]; then
      echo "gti-e07 run: runner-owned shared-host delta remains" >&2
      CLEANUP_FAILED=1
    fi
    rm -rf -- "$SNAP_DIR"
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
    [[ -f "$config_file" ]] && return 0
    kill -0 "$SERVE_PID" 2>/dev/null || return 1
    sleep 0.5
    attempts=$((attempts - 1))
  done
  return 1
}

first_attempt_state() {
  awk '
    /^Attempt[[:space:]]+State[[:space:]]+/ { in_table=1; next }
    in_table && /^[-[:space:]]+$/ { next }
    in_table && $1 ~ /^[0-9]+$/ { print $2; exit }
  ' "$1"
}

wait_for_state() {
  local id="$1"
  local wanted="$2"
  local timeout_seconds="$3"
  local out="$4"
  local deadline=$((SECONDS + timeout_seconds))
  local state=""
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
      "$BIN" workload describe "$id" >"$out" 2>&1 || true
    state="$(first_attempt_state "$out")"
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
  for command in awk cargo comm file find grep ip mktemp rustc sed sort systemd-detect-virt timeout tr; do
    require_command "$command"
  done
  "$PREPARE" check-source
  trap cleanup EXIT
  trap 'exit 130' HUP INT TERM
  snapshot_shared_state

  bounded 600s cargo build -p overdrive-cli --bin overdrive
  [[ -x "$BIN" ]] || die "default-feature product binary was not built: $BIN"
  bounded 45s "$PREPARE" cleanup
  bounded 240s "$PREPARE" prepare
  PREPARED=1
  bounded 45s "$PREPARE" check

  OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" CREDENTIALS_DIRECTORY="$CREDS_DIR" \
    "$BIN" serve --bind "$BIND" --data-dir "$DATA_DIR" \
    >"$OUTPUT_ROOT/serve.log" 2>&1 &
  SERVE_PID=$!
  wait_for_serve || die "serve did not become ready within 30 seconds"

  bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" deploy --detach "$EXAMPLE_DIR/callee.toml"
  CALLEE_DEPLOYED=1
  wait_for_state "$CALLEE_ID" Stable 90 "$OUTPUT_ROOT/callee-describe.log" \
    || die "callee did not remain available in Stable"

  bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" deploy --detach "$EXAMPLE_DIR/caller.toml"
  CALLER_DEPLOYED=1
  wait_for_state "$CALLER_ID" Terminated 120 "$OUTPUT_ROOT/caller-describe.log" \
    || die "caller did not reach its successful terminal state"
  grep -Fq 'Verdict: Succeeded' "$OUTPUT_ROOT/caller-describe.log" \
    || die "caller terminal result was not successful"

  echo "E07 PASS: one VM Job called one Exec Service and received the exact reply"
}

case "${1:-run}" in
  check-source) "$PREPARE" check-source ;;
  run) run ;;
  *) die "usage: run-example.sh [check-source|run]" ;;
esac

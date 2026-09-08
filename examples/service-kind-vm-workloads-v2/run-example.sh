#!/usr/bin/env bash
# E09 v2: cycle 20 healthy/failure Service pairs through ONE live serve.
#
# This is an operator-runnable product example. It invokes only the public
# deploy, workload describe, and job stop commands; the only process it starts
# itself is the one control-plane serve process. Guest programs and source
# specifications are reused from the checked-in E08 bundle and are copied into
# unique, marker-owned case specs before the suite starts.
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly EXAMPLE_DIR
SOURCE_DIR="$(cd "$EXAMPLE_DIR/../service-kind-vm-workloads" && pwd)"
readonly SOURCE_DIR
readonly PREPARE="$EXAMPLE_DIR/prepare.sh"
REPO_ROOT="$(cd "$EXAMPLE_DIR/../.." && pwd)"
readonly REPO_ROOT
readonly BIN="$REPO_ROOT/target/debug/overdrive"
readonly OUTPUT_ROOT="${SVM_E09_V2_OUTPUT_ROOT:-/srv/vm/overdrive-testing/svm-e08-v2}"
readonly CONFIG_DIR="$OUTPUT_ROOT/config"
readonly DATA_DIR="$OUTPUT_ROOT/data"
readonly CREDS_DIR="$OUTPUT_ROOT/credentials"
readonly CASE_ROOT="$OUTPUT_ROOT/cases"
readonly MEASURE_ROOT="$OUTPUT_ROOT/measurements"
readonly BIND="127.0.0.1:7644"
readonly RUN_ROOT="/run/overdrive/vm"
readonly CGROUP_ROOT="/sys/fs/cgroup/overdrive.slice/workloads.slice"
readonly E09_V2_PAIR_COUNT=20
readonly E09_V2_EXPECTED_CONCURRENCY=10

SERVE_PID=""
SERVE_START_TICKS=""
SERVE_EXE=""
PREPARED=0
SUITE_STARTED=0
SUITE_FAILED=0
SUITE_CANCELLED=0
REPORT_EMITTED=0
CLEANUP_FAILED=0
HOST_CPUS=0
HOST_MEM_BYTES=0
CONCURRENCY=0
BASELINE_DIR=""
TIMING_LOG=""
INPUT_MANIFEST=""
LEDGER=""

WORKER_PIDS=()
WORKER_DIRS=()

die() {
  echo "svm-e09-v2 run: $*" >&2
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

now_ns() {
  date +%s%N
}

duration_ms() {
  local started="$1"
  local finished="$2"
  echo $(( (finished - started) / 1000000 ))
}

record_timing() {
  local log="$1"
  local stage="$2"
  local started="$3"
  local finished="$4"
  printf '%s\t%s\n' "$stage" "$(duration_ms "$started" "$finished")" \
    >>"$log"
}

proc_start_ticks() {
  local pid="$1"
  [[ -r "/proc/$pid/stat" ]] || return 1
  awk '{print $22}' "/proc/$pid/stat"
}

assert_serve_identity() {
  [[ -n "$SERVE_PID" ]] || die "the suite control plane has no recorded PID"
  kill -0 "$SERVE_PID" 2>/dev/null \
    || die "the suite control plane stopped unexpectedly (pid $SERVE_PID)"
  local current_ticks
  current_ticks="$(proc_start_ticks "$SERVE_PID")" \
    || die "cannot read control-plane start identity for pid $SERVE_PID"
  [[ "$current_ticks" == "$SERVE_START_TICKS" ]] \
    || die "control-plane PID was reused: pid=$SERVE_PID start=$current_ticks expected=$SERVE_START_TICKS"
  local current_exe
  current_exe="$(readlink -f "/proc/$SERVE_PID/exe")" \
    || die "cannot read control-plane executable identity"
  [[ "$current_exe" == "$SERVE_EXE" ]] \
    || die "control-plane executable changed: $current_exe (expected $SERVE_EXE)"
}

probe_hypervisors() {
  local proc argv0
  for proc in /proc/[0-9]*; do
    [[ -r "$proc/cmdline" ]] || continue
    argv0=''
    IFS= read -r -d '' argv0 <"$proc/cmdline" || true
    [[ "$(basename "$argv0" 2>/dev/null || true)" == "cloud-hypervisor" ]] \
      && printf '%s\n' "${proc##*/}" || true
  done | LC_ALL=C sort -n
}

probe_scopes() {
  find "$CGROUP_ROOT" -maxdepth 1 -mindepth 1 -type d -name 'alloc-*.scope' \
    -printf '%f\n' 2>/dev/null | LC_ALL=C sort || true
}

probe_run_dirs() {
  find "$RUN_ROOT" -maxdepth 1 -mindepth 1 -type d -printf '%f\n' \
    2>/dev/null | LC_ALL=C sort || true
}

probe_network() {
  {
    # Record stable resource names, not addresses/flags that can change while
    # a sibling allocation is alive.  The cohort snapshots are therefore
    # suitable for an exact owned-runtime diff.
    ip -o link show 2>/dev/null | awk -F': ' \
      '{name = $2; sub(/@.*/, "", name); if (name ~ /^ovd-/) print "link:" name}' || true
    ip netns list 2>/dev/null | awk '$1 ~ /^ovd-ns-/ {print "netns:" $1}' || true
    while IFS= read -r netns; do
      [[ -n "$netns" ]] || continue
      ip -n "$netns" -o link show 2>/dev/null | awk -F': ' -v netns="$netns" \
        '{name = $2; sub(/@.*/, "", name); if (name ~ /^ovd-/) print "netns-link:" netns ":" name}' \
        || true
    done < <(ip netns list 2>/dev/null | awk '$1 ~ /^ovd-ns-/ {print $1}')
    find /etc/netns -maxdepth 1 -mindepth 1 -type d -name 'ovd-ns-*' \
      -printf 'netns-config:%f\n' 2>/dev/null || true
  } | LC_ALL=C sort
}

probe_bpf() {
  # Limit the kernel-program inventory to the product-owned interfaces.  A
  # sibling host's unrelated BPF attachment must not become this suite's
  # cleanup failure.
  bpftool net show 2>/dev/null | grep -E 'ovd-' | LC_ALL=C sort || true
}

probe_nft() {
  # Rule counters are mutable while a workload is live; normalize them before
  # taking the post-cohort identity diff.  Table/chain/rule identities and
  # marks remain visible for leak detection.
  nft -nn list ruleset 2>/dev/null \
    | sed -E 's/counter packets [0-9]+ bytes [0-9]+/counter packets <count> bytes <count>/g' \
    | LC_ALL=C sort || true
}

probe_loops() {
  # Restrict loop evidence to this marker-owned materialization; allocations
  # may attach their own clone rather than the prepared base rootfs.
  losetup -a 2>/dev/null | grep -F "$OUTPUT_ROOT" | LC_ALL=C sort || true
}

probe_mounts() {
  findmnt --raw --noheadings --output TARGET,SOURCE,FSTYPE,OPTIONS 2>/dev/null \
    | grep -F "$OUTPUT_ROOT" \
    | LC_ALL=C sort || true
}

probe_clone_state() {
  {
    find "$DATA_DIR/vm/clone-staging" -maxdepth 1 -mindepth 1 -printf 'staging:%f\n' \
      2>/dev/null || true
    find "$DATA_DIR/vm/clone-index" -maxdepth 1 -mindepth 1 -printf 'index:%f\n' \
      2>/dev/null || true
  } | LC_ALL=C sort
}

probe_store_metrics() {
  local files bytes
  files="$(find "$DATA_DIR" -xdev -type f -printf '%p\n' 2>/dev/null | wc -l | tr -d ' ')"
  bytes="$(du -sx --bytes "$DATA_DIR" 2>/dev/null | awk '{print $1}')"
  printf 'store_files=%s\tstore_bytes=%s\n' "${files:-0}" "${bytes:-0}"
}

snapshot_all() {
  local destination="$1"
  install -d -m 0700 "$destination"
  probe_hypervisors >"$destination/hypervisors"
  probe_scopes >"$destination/scopes"
  probe_run_dirs >"$destination/run-dirs"
  probe_network >"$destination/network"
  probe_bpf >"$destination/bpf"
  probe_nft >"$destination/nft"
  probe_loops >"$destination/loops"
  probe_mounts >"$destination/mounts"
  probe_clone_state >"$destination/clones"
  probe_store_metrics >"$destination/store"
}

assert_no_new_runtime_resources() {
  local baseline="$1"
  local current="$2"
  local name failed=0
  for name in hypervisors scopes run-dirs network bpf nft loops mounts clones; do
    comm -13 "$baseline/$name" "$current/$name" >"$current/$name.new"
    if [[ -s "$current/$name.new" ]]; then
      echo "svm-e09-v2 run: unexpected new $name after owned cleanup:" >&2
      cat "$current/$name.new" >&2
      failed=1
    fi
  done
  return "$failed"
}

active_resource_count() {
  local vms scopes
  vms="$(probe_hypervisors | sed '/^$/d' | wc -l | tr -d ' ')"
  scopes="$(probe_scopes | sed '/^$/d' | wc -l | tr -d ' ')"
  printf 'vm=%s\tcgroup=%s\n' "${vms:-0}" "${scopes:-0}"
}

owned_active_case_count() {
  local cohort_dir="$1"
  local marker trial resource_file cgroup=0 run_dir=0
  for marker in "$cohort_dir"/*.healthy-active; do
    [[ -e "$marker" ]] || continue
    trial="${marker##*/}"
    trial="${trial%.healthy-active}"
    resource_file="$CASE_ROOT/$trial/healthy-resources-active"
    [[ -f "$resource_file" ]] || continue
    while IFS= read -r resource; do
      case "$resource" in
        cgroup$'\t'*) cgroup=$((cgroup + 1)) ;;
        run-dir$'\t'*) run_dir=$((run_dir + 1)) ;;
      esac
    done <"$resource_file"
  done
  printf 'cgroups=%s\trun_dirs=%s\n' "$cgroup" "$run_dir"
}

assert_owned_active_cases() {
  local cohort_dir="$1"
  local expected="$2"
  local marker trial resource_file cgroups run_dirs
  for marker in "$cohort_dir"/*.healthy-active; do
    [[ -e "$marker" ]] || continue
    trial="${marker##*/}"
    trial="${trial%.healthy-active}"
    resource_file="$CASE_ROOT/$trial/healthy-resources-active"
    [[ -s "$resource_file" ]] || {
      echo "svm-e09-v2 run: active case has no scoped resource evidence: $resource_file" >&2
      return 1
    }
    grep -Fq $'cgroup\t' "$resource_file" || return 1
    grep -Fq $'run-dir\t' "$resource_file" || return 1
  done
  local counts
  counts="$(owned_active_case_count "$cohort_dir")"
  cgroups="$(awk -F'[=\t]' '{print $2}' <<<"$counts")"
  run_dirs="$(awk -F'[=\t]' '{print $4}' <<<"$counts")"
  [[ "${cgroups:-0}" -eq "$expected" && "${run_dirs:-0}" -eq "$expected" ]] || {
    echo "svm-e09-v2 run: expected $expected owned active cgroups/run dirs, observed cgroups=${cgroups:-0} run_dirs=${run_dirs:-0}" >&2
    return 1
  }
}

record_store_growth() {
  local cohort="$1"
  local before="$2"
  local after="$3"
  local before_files before_bytes after_files after_bytes
  read -r before_files before_bytes < <(awk -F'[=\t]' '{print $2, $4}' "$before/store")
  read -r after_files after_bytes < <(awk -F'[=\t]' '{print $2, $4}' "$after/store")
  printf 'cohort=%s\tstore_files_before=%s\tstore_files_after=%s\tstore_files_delta=%s\tstore_bytes_before=%s\tstore_bytes_after=%s\tstore_bytes_delta=%s\n' \
    "$cohort" "${before_files:-0}" "${after_files:-0}" \
    "$(( ${after_files:-0} - ${before_files:-0} ))" \
    "${before_bytes:-0}" "${after_bytes:-0}" \
    "$(( ${after_bytes:-0} - ${before_bytes:-0} ))" \
    >>"$MEASURE_ROOT/store-growth.tsv"
}

capture_case_resources() {
  local alloc_file="$1"
  local output="$2"
  : >"$output"
  local alloc
  while IFS= read -r alloc; do
    [[ -n "$alloc" ]] || continue
    [[ -d "$CGROUP_ROOT/$alloc.scope" ]] \
      && printf 'cgroup\t%s\n' "$CGROUP_ROOT/$alloc.scope" >>"$output"
    [[ -d "$RUN_ROOT/$alloc" ]] \
      && printf 'run-dir\t%s\n' "$RUN_ROOT/$alloc" >>"$output"
    [[ -e "$DATA_DIR/vm/clone-staging/.overdrive-vm-rootfs-$alloc.img" ]] \
      && printf 'rootfs-clone\t%s\n' \
        "$DATA_DIR/vm/clone-staging/.overdrive-vm-rootfs-$alloc.img" >>"$output"
    [[ -e "$DATA_DIR/vm/clone-index/.overdrive-vm-rootfs-$alloc.img" ]] \
      && printf 'clone-index\t%s\n' \
        "$DATA_DIR/vm/clone-index/.overdrive-vm-rootfs-$alloc.img" >>"$output"
  done <"$alloc_file"
}

assert_case_resources_released() {
  local alloc_file="$1"
  local output="$2"
  capture_case_resources "$alloc_file" "$output"
  [[ ! -s "$output" ]] || {
    echo "svm-e09-v2 run: owned allocation resources remain:" >&2
    cat "$output" >&2
    return 1
  }
}

workload_runtime_resources_remain() {
  local id="$1"
  local allocation_prefix="alloc-$id-"
  if probe_scopes | grep -F "$allocation_prefix" >/dev/null; then
    return 0
  fi
  if probe_run_dirs | grep -F "$allocation_prefix" >/dev/null; then
    return 0
  fi
  if probe_clone_state | grep -F "$allocation_prefix" >/dev/null; then
    return 0
  fi
  return 1
}

wait_for_workload_runtime_cleanup() {
  local id="$1"
  local deadline=$((SECONDS + 60))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    assert_serve_identity
    if ! workload_runtime_resources_remain "$id"; then
      return 0
    fi
    sleep 1
  done
  return 1
}

first_service_alloc_state() {
  awk '
    /^Alloc[[:space:]]+State[[:space:]]+/ { in_table = 1; next }
    in_table && /^[-[:space:]]+$/ { next }
    in_table && $1 ~ /^alloc-/ { print $2; exit }
  '
}

first_service_alloc_id() {
  awk '
    /^Alloc[[:space:]]+State[[:space:]]+/ { in_table = 1; next }
    in_table && /^[-[:space:]]+$/ { next }
    in_table && $1 ~ /^alloc-/ { print $1; exit }
  '
}

first_service_restart_count() {
  awk '
    /^Alloc[[:space:]]+State[[:space:]]+/ { in_table = 1; next }
    in_table && /^[-[:space:]]+$/ { next }
    in_table && $1 ~ /^alloc-/ { print $3; exit }
  '
}

all_service_alloc_ids() {
  awk '
    /^Alloc[[:space:]]+State[[:space:]]+/ { in_table = 1; next }
    in_table && /^[-[:space:]]+$/ { next }
    in_table && $1 ~ /^alloc-/ { print $1 }
  '
}

first_job_attempt_state() {
  awk '
    /^Attempt[[:space:]]+State[[:space:]]+/ { in_table = 1; next }
    in_table && /^[-[:space:]]+$/ { next }
    in_table && $1 ~ /^[0-9]+$/ { print $2; exit }
  '
}

service_probe_line() {
  local role="$1"
  local port="$2"
  grep -Eio "${role}[^[:cntrl:]]*${port}[^[:cntrl:]]*" "$3" | head -n 1 \
    | tr '\r\t' '  ' || true
}

target_for_probe() {
  local role="$1"
  local port="$2"
  local describe="$3"
  local line
  line="$(service_probe_line "$role" "$port" "$describe")"
  [[ -n "$line" ]] || return 1
  printf '%s\n' "$line"
}

query_describe() {
  local id="$1"
  local output="$2"
  local rc
  set +e
  bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" workload describe "$id" >"$output" 2>&1
  rc=$?
  set -e
  printf '%s\n' "$rc" >"$output.rc"
  return "$rc"
}

wait_for_serve() {
  local attempts=60
  while [[ "$attempts" -gt 0 ]]; do
    [[ -f "$CONFIG_DIR/.overdrive/config" ]] \
      && kill -0 "$SERVE_PID" 2>/dev/null && return 0
    kill -0 "$SERVE_PID" 2>/dev/null || return 1
    sleep 0.5
    attempts=$((attempts - 1))
  done
  return 1
}

start_serve() {
  local started finished
  started="$(now_ns)"
  setsid keyctl session - env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    CREDENTIALS_DIRECTORY="$CREDS_DIR" "$BIN" serve --bind "$BIND" --data-dir "$DATA_DIR" \
    >"$OUTPUT_ROOT/serve.log" 2>&1 &
  SERVE_PID=$!
  SERVE_START_TICKS=""
  local attempts=60
  while [[ "$attempts" -gt 0 && -z "$SERVE_START_TICKS" ]]; do
    if kill -0 "$SERVE_PID" 2>/dev/null; then
      SERVE_START_TICKS="$(proc_start_ticks "$SERVE_PID" || true)"
    else
      return 1
    fi
    [[ -n "$SERVE_START_TICKS" ]] || sleep 0.1
    attempts=$((attempts - 1))
  done
  [[ -n "$SERVE_START_TICKS" ]] || return 1
  # setsid/keyctl perform a short exec chain before the product process is
  # resident.  Record the intended product executable rather than the
  # transient helper (`env`/`keyctl`) visible during that chain; the PID and
  # Linux start ticks remain the stable identity checked on every poll.
  SERVE_EXE="$(readlink -f "$BIN")"
  wait_for_serve || return 1
  finished="$(now_ns)"
  record_timing "$TIMING_LOG" serve-start "$started" "$finished"
  printf 'control_plane_pid=%s\tstart_ticks=%s\texe=%s\n' \
    "$SERVE_PID" "$SERVE_START_TICKS" "$SERVE_EXE" \
    >"$MEASURE_ROOT/control-plane-identity"
}

stop_serve() {
  [[ -n "$SERVE_PID" ]] || return 0
  if kill -0 "$SERVE_PID" 2>/dev/null; then
    # This is the process group created by this suite's setsid invocation.
    kill -- "-$SERVE_PID" 2>/dev/null || kill "$SERVE_PID" 2>/dev/null || true
  fi
  local attempts=100
  while worker_is_alive "$SERVE_PID" && [[ "$attempts" -gt 0 ]]; do
    sleep 0.1
    attempts=$((attempts - 1))
  done
  if worker_is_alive "$SERVE_PID"; then
    kill -KILL -- "-$SERVE_PID" 2>/dev/null || kill -KILL "$SERVE_PID" 2>/dev/null || true
    wait "$SERVE_PID" 2>/dev/null || true
    echo "svm-e09-v2 run: owned serve process exceeded bounded shutdown" >&2
    return 1
  fi
  wait "$SERVE_PID" 2>/dev/null || true
  SERVE_PID=""
  return 0
}

generate_spec() {
  local source="$1"
  local destination="$2"
  local service_id="$3"
  local client_id="$4"
  sed \
    -e "s|/srv/vm/overdrive-testing/svm-e08|$OUTPUT_ROOT|g" \
    -e "s|service-vm-e08-client|$client_id|g" \
    -e "s|service-vm-tcp-failure-client|$client_id|g" \
    -e "s|service-vm-e08|$service_id|g" \
    -e "s|service-vm-tcp-failure|$service_id|g" \
    "$source" >"$destination"
  grep -Fq "${service_id}" "$destination" \
    || die "generated spec does not carry its unique workload identity: $destination"
  grep -Fq "$OUTPUT_ROOT/kernel" "$destination" \
    || die "generated spec does not point at the v2 kernel: $destination"
  grep -Fq "$OUTPUT_ROOT/rootfs.ext4" "$destination" \
    || die "generated spec does not point at the v2 rootfs: $destination"
}

materialize_specs() {
  local trial healthy_service healthy_client failure_service failure_client
  local healthy_service_spec healthy_client_spec failure_service_spec failure_client_spec
  install -d -m 0700 "$CASE_ROOT"
  INPUT_MANIFEST="$CASE_ROOT/input-manifest.tsv"
  printf 'trial\tphase\tservice_id\tclient_id\tservice_spec\tclient_spec\n' \
    >"$INPUT_MANIFEST"
  for trial in $(seq 1 "$E09_V2_PAIR_COUNT"); do
    printf -v healthy_service 'service-e09-v2-c%03d-h' "$trial"
    printf -v healthy_client '%s-client' "$healthy_service"
    printf -v failure_service 'service-e09-v2-c%03d-f' "$trial"
    printf -v failure_client '%s-client' "$failure_service"
    install -d -m 0700 "$CASE_ROOT/$trial"
    healthy_service_spec="$CASE_ROOT/$trial/healthy-service.toml"
    healthy_client_spec="$CASE_ROOT/$trial/healthy-client.toml"
    failure_service_spec="$CASE_ROOT/$trial/failure-service.toml"
    failure_client_spec="$CASE_ROOT/$trial/failure-client.toml"
    generate_spec "$SOURCE_DIR/service.toml" "$healthy_service_spec" \
      "$healthy_service" "$healthy_client"
    generate_spec "$SOURCE_DIR/client-healthy.toml" "$healthy_client_spec" \
      "$healthy_service" "$healthy_client"
    generate_spec "$SOURCE_DIR/tcp-startup-failure.toml" "$failure_service_spec" \
      "$failure_service" "$failure_client"
    generate_spec "$SOURCE_DIR/client-tcp-failure.toml" "$failure_client_spec" \
      "$failure_service" "$failure_client"
    printf '%s\thealthy\t%s\t%s\t%s\t%s\n' "$trial" "$healthy_service" \
      "$healthy_client" "$healthy_service_spec" "$healthy_client_spec" >>"$INPUT_MANIFEST"
    printf '%s\tfailure\t%s\t%s\t%s\t%s\n' "$trial" "$failure_service" \
      "$failure_client" "$failure_service_spec" "$failure_client_spec" >>"$INPUT_MANIFEST"
  done
  [[ "$(($(wc -l <"$INPUT_MANIFEST") - 1))" -eq $((E09_V2_PAIR_COUNT * 2)) ]] \
    || die "input manifest does not contain exactly $((E09_V2_PAIR_COUNT * 2)) Service/peer inputs"
}

configure_capacity() {
  HOST_CPUS="$(nproc)"
  HOST_MEM_BYTES="$(awk '/^MemTotal:/ {print $2 * 1024; exit}' /proc/meminfo)"
  [[ "$HOST_CPUS" =~ ^[1-9][0-9]*$ ]] \
    || die "nproc returned an invalid host CPU count: $HOST_CPUS"
  [[ "$HOST_MEM_BYTES" =~ ^[1-9][0-9]*$ ]] \
    || die "MemTotal is unavailable or invalid: $HOST_MEM_BYTES"
  local requested_concurrency="${SVM_E09_V2_CONCURRENCY:-$E09_V2_EXPECTED_CONCURRENCY}"
  [[ "$requested_concurrency" =~ ^[1-9][0-9]*$ ]] \
    || die "SVM_E09_V2_CONCURRENCY must be a positive integer"
  [[ "$requested_concurrency" -eq "$E09_V2_EXPECTED_CONCURRENCY" ]] \
    || die "E09 v2 requires exactly $E09_V2_EXPECTED_CONCURRENCY concurrent workers; refusing requested=$requested_concurrency"
  CONCURRENCY="$E09_V2_EXPECTED_CONCURRENCY"
  printf 'host_cpus=%s\thost_mem_bytes=%s\tconfigured_concurrency=%s\n' \
    "$HOST_CPUS" "$HOST_MEM_BYTES" "$CONCURRENCY" \
    >"$MEASURE_ROOT/capacity"
}

run_service_deploy() {
  local spec="$1"
  local output="$2"
  local command
  printf -v command 'env OVERDRIVE_CONFIG_DIR=%q %q deploy %q' \
    "$CONFIG_DIR" "$BIN" "$spec"
  set +e
  bounded 180s script -q -e -c "$command" "$output" >/dev/null
  local rc=$?
  set -e
  printf '%s\n' "$rc" >"$output.rc"
  return "$rc"
}

run_client_deploy() {
  local spec="$1"
  local output="$2"
  set +e
  bounded 45s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" deploy --detach "$spec" >"$output" 2>&1
  local rc=$?
  set -e
  printf '%s\n' "$rc" >"$output.rc"
  return "$rc"
}

wait_for_service_healthy() {
  local id="$1"
  local stream="$2"
  local output="$3"
  local query_errors="$4"
  local deadline=$((SECONDS + 180))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    assert_serve_identity
    if ! query_describe "$id" "$output"; then
      printf '%s\n' "$(<"$output.rc")" >>"$query_errors"
    fi
    if [[ "$(first_service_alloc_state <"$output")" == "Running" ]] \
      && grep -Eqi 'startup[^[:cntrl:]]*probe\[0\][^[:cntrl:]]*last=pass|startup[^[:cntrl:]]*last=pass' "$output" \
      && grep -Eqi 'readiness[^[:cntrl:]]*probe\[0\][^[:cntrl:]]*last=pass|readiness[^[:cntrl:]]*last=pass' "$output" \
      && grep -Eqi '18081' "$output" \
      && grep -Fq ' is stable ' "$stream"; then
      return 0
    fi
    sleep 1
  done
  return 1
}

has_startup_probe_failure() {
  local output="$1"
  # The public renderer currently prints the typed StartupProbeFailed event as
  # "startup probe[0] failed".  Accept the enum spelling too because this
  # expectation is intentionally coupled to the public stream, not to a
  # particular renderer wording.  The caller still requires the failed guest
  # TCP observation and rejects a generic deploy error.
  grep -Eqi 'StartupProbeFailed|startup probe\[0\] failed' "$output"
}

wait_for_service_failure() {
  local id="$1"
  local stream="$2"
  local output="$3"
  local query_errors="$4"
  local deadline=$((SECONDS + 180))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    assert_serve_identity
    if ! query_describe "$id" "$output"; then
      printf '%s\n' "$(<"$output.rc")" >>"$query_errors"
    fi
    local state
    state="$(first_service_alloc_state <"$output")"
    if [[ "$state" == "Failed" ]] \
      && has_startup_probe_failure "$stream" \
      && grep -Eqi 'startup[^[:cntrl:]]*(18999|probe\[0\])[^[:cntrl:]]*last=fail|startup[^[:cntrl:]]*last=fail[^[:cntrl:]]*(18999|probe\[0\])' "$output"; then
      if grep -Fq ' is stable ' "$stream"; then
        return 1
      fi
      return 0
    fi
    # The existing Service lifecycle can publish StartupProbeFailed while a
    # replacement allocation is Running: the failed allocation's beacon path
    # is still occupied, so the replacement start is rejected with the typed
    # bind error.  Accept this trajectory only with all of that public
    # evidence; an arbitrary Running/crashed result is not a failure oracle.
    if [[ "$state" == "Running" ]] \
      && [[ "$(first_service_restart_count <"$output")" =~ ^[1-9][0-9]*$ ]] \
      && grep -Eqi 'last terminated:[^[:cntrl:]]*Failed[^[:cntrl:]]*bind beacon listener:[^[:cntrl:]]*Address already in use' "$output" \
      && has_startup_probe_failure "$stream" \
      && grep -Eqi 'startup[^[:cntrl:]]*(18999|probe\[0\])[^[:cntrl:]]*last=fail|startup[^[:cntrl:]]*last=fail[^[:cntrl:]]*(18999|probe\[0\])' "$output"; then
      if grep -Fq ' is stable ' "$stream"; then
        return 1
      fi
      return 0
    fi
    sleep 1
  done
  return 1
}

wait_for_job_success() {
  local id="$1"
  local output="$2"
  local deadline=$((SECONDS + 120))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    assert_serve_identity
    query_describe "$id" "$output" || true
    local state
    state="$(first_job_attempt_state <"$output")"
    if [[ "$state" == "Terminated" ]] \
      && grep -Fq 'Verdict: Succeeded' "$output"; then
      return 0
    fi
    [[ "$state" == "Failed" || "$state" == "Stopped" ]] && return 1
    sleep 1
  done
  return 1
}

stop_workload() {
  local id="$1"
  local case_dir="$2"
  local output="$case_dir/$3"
  set +e
  bounded 45s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" job stop "$id" >"$output" 2>&1
  local stop_rc=$?
  set -e
  printf '%s\n' "$stop_rc" >"$output.rc"
  [[ "$stop_rc" -eq 0 ]] || return "$stop_rc"
  local describe="$case_dir/$4"
  local deadline=$((SECONDS + 60))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    assert_serve_identity
    query_describe "$id" "$describe" || true
    local service_state job_state
    service_state="$(first_service_alloc_state <"$describe")"
    job_state="$(first_job_attempt_state <"$describe")"
    if [[ "$service_state" == "Terminated" || "$service_state" == "Failed" \
      || "$service_state" == "Stopped" ]] \
      || [[ "$job_state" == "Terminated" || "$job_state" == "Failed" \
      || "$job_state" == "Stopped" ]]; then
      wait_for_workload_runtime_cleanup "$id" || return 1
      return 0
    fi
    sleep 1
  done
  return 1
}

dispose_failed_service() {
  local id="$1"
  local case_dir="$2"
  set +e
  bounded 45s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    "$BIN" job stop "$id" >"$case_dir/failure-stop.out" 2>&1
  local stop_rc=$?
  set -e
  printf '%s\n' "$stop_rc" >"$case_dir/failure-stop.out.rc"
  [[ "$stop_rc" -eq 0 ]] || return "$stop_rc"
  local describe="$case_dir/failure-final-describe.out"
  local deadline=$((SECONDS + 60))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    assert_serve_identity
    query_describe "$id" "$describe" || true
    local state
    state="$(first_service_alloc_state <"$describe")"
    if [[ "$state" == "Failed" ]] \
      && has_startup_probe_failure "$describe" \
      && grep -Eqi 'startup[^[:cntrl:]]*(18999|probe\[0\])[^[:cntrl:]]*last=fail|startup[^[:cntrl:]]*last=fail[^[:cntrl:]]*(18999|probe\[0\])' "$describe"; then
      return 0
    fi
    if [[ "$state" == "Terminated" ]] \
      && grep -Fq 'reason: stopped' "$describe" \
      && grep -Eqi 'startup[^[:cntrl:]]*(18999|probe\[0\])[^[:cntrl:]]*last=fail' \
        "$case_dir/failure-describe.out"; then
      return 0
    fi
    sleep 1
  done
  return 1
}

wait_for_gate() {
  local gate="$1"
  local deadline=$((SECONDS + 300))
  while [[ ! -e "$gate" ]]; do
    assert_serve_identity
    [[ "$SECONDS" -lt "$deadline" ]] || return 1
    sleep 0.2
  done
}

wait_for_markers() {
  local directory="$1"
  local marker="$2"
  local expected="$3"
  local deadline=$((SECONDS + 300))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    assert_serve_identity
    local count
    count="$(find "$directory" -maxdepth 1 -type f -name "*.$marker" -printf '%f\n' \
      2>/dev/null | wc -l | tr -d ' ')"
    [[ "$count" -eq "$expected" ]] && return 0
    sleep 0.2
  done
  return 1
}

worker_is_alive() {
  local pid="$1"
  kill -0 "$pid" 2>/dev/null || return 1
  [[ -r "/proc/$pid/stat" ]] || return 1
  [[ "$(awk '{print $3}' "/proc/$pid/stat")" != Z ]]
}

write_worker_result() {
  local incoming_rc="$1"
  local result="$WORKER_DIR/result.tsv"
  local outcome=pass
  [[ "$incoming_rc" -eq 0 ]] || outcome=failed
  [[ "${WORKER_CANCELLED:-0}" -eq 0 ]] || outcome=not-run-cancelled
  local healthy_allocs failure_allocs
  if [[ -f "$WORKER_DIR/healthy-allocs" ]]; then
    healthy_allocs="$(tr '\n' ',' <"$WORKER_DIR/healthy-allocs" | sed 's/,$//')"
  else
    healthy_allocs=''
  fi
  if [[ -f "$WORKER_DIR/failure-allocs" ]]; then
    failure_allocs="$(tr '\n' ',' <"$WORKER_DIR/failure-allocs" | sed 's/,$//')"
  else
    failure_allocs=''
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$WORKER_TRIAL" "$outcome" "$WORKER_STAGE" "$SERVE_PID" "$SERVE_START_TICKS" \
    "$WORKER_HEALTHY_SERVICE" "${WORKER_HEALTHY_DEPLOY_RC:-}" \
    "${WORKER_HEALTHY_STATE:-}" "${WORKER_HEALTHY_TARGET:-}" \
    "${WORKER_HEALTHY_PEER:-}" "$healthy_allocs" \
    "${WORKER_HEALTHY_CLEANUP:-}" "$WORKER_FAILURE_SERVICE" \
    "${WORKER_FAILURE_DEPLOY_RC:-}" "${WORKER_FAILURE_STATE:-}" \
    "${WORKER_FAILURE_REASON:-}" "${WORKER_FAILURE_TARGET:-}" \
    "${WORKER_FAILURE_PEER:-}" "$failure_allocs" \
    "${WORKER_FAILURE_CLEANUP:-}" >"$result"
}

worker_cleanup() {
  local incoming_rc="$1"
  trap - EXIT HUP INT TERM
  local failed=0 started finished
  if [[ "${WORKER_CLIENT_DEPLOYED:-0}" -eq 1 ]]; then
    stop_workload "$WORKER_CLIENT_ID" "$WORKER_DIR" client-stop.out client-stop-describe.out \
      || failed=1
  fi
  if [[ "${WORKER_HEALTHY_DEPLOYED:-0}" -eq 1 ]]; then
    started="$(now_ns)"
    stop_workload "$WORKER_HEALTHY_SERVICE" "$WORKER_DIR" healthy-stop.out healthy-stop-describe.out \
      || failed=1
    finished="$(now_ns)"
    record_timing "$WORKER_TIMING" healthy-stop "$started" "$finished"
  fi
  if [[ "${WORKER_FAILURE_DEPLOYED:-0}" -eq 1 ]]; then
    started="$(now_ns)"
    dispose_failed_service "$WORKER_FAILURE_SERVICE" "$WORKER_DIR" || failed=1
    finished="$(now_ns)"
    record_timing "$WORKER_TIMING" failure-stop-and-reclamation "$started" "$finished"
  fi
  if [[ "$failed" -ne 0 && "$incoming_rc" -eq 0 ]]; then
    incoming_rc=1
  fi
  write_worker_result "$incoming_rc"
  exit "$incoming_rc"
}

run_trial() {
  WORKER_TRIAL="$1"
  WORKER_DIR="$2"
  WORKER_COHORT="$3"
  WORKER_TIMING="$WORKER_DIR/timing.tsv"
  printf 'stage\tduration_ms\n' >"$WORKER_TIMING"
  WORKER_STAGE=scheduled
  WORKER_HEALTHY_DEPLOYED=0
  WORKER_CLIENT_DEPLOYED=0
  WORKER_FAILURE_DEPLOYED=0
  printf -v WORKER_HEALTHY_SERVICE 'service-e09-v2-c%03d-h' "$WORKER_TRIAL"
  printf -v WORKER_CLIENT_ID '%s-client' "$WORKER_HEALTHY_SERVICE"
  printf -v WORKER_FAILURE_SERVICE 'service-e09-v2-c%03d-f' "$WORKER_TRIAL"
  printf -v WORKER_FAILURE_CLIENT '%s-client' "$WORKER_FAILURE_SERVICE"
  WORKER_CANCELLED=0
  local healthy_service_spec="$CASE_ROOT/$WORKER_TRIAL/healthy-service.toml"
  local healthy_client_spec="$CASE_ROOT/$WORKER_TRIAL/healthy-client.toml"
  local failure_service_spec="$CASE_ROOT/$WORKER_TRIAL/failure-service.toml"
  local failure_client_spec="$CASE_ROOT/$WORKER_TRIAL/failure-client.toml"
  trap 'worker_cleanup $?' EXIT
  trap 'WORKER_CANCELLED=1; exit 130' HUP INT TERM

  assert_serve_identity
  WORKER_STAGE=healthy-deploy
  local started finished rc
  started="$(now_ns)"
  WORKER_HEALTHY_DEPLOYED=1
  if run_service_deploy "$healthy_service_spec" "$WORKER_DIR/healthy-deploy.out"; then
    rc=0
  else
    rc=$?
  fi
  finished="$(now_ns)"
  WORKER_HEALTHY_DEPLOY_RC="$rc"
  record_timing "$WORKER_TIMING" healthy-deploy "$started" "$finished"
  [[ "$rc" -eq 0 ]] || die "healthy Service deploy failed for trial $WORKER_TRIAL (rc=$rc)"
  grep -Fq ' is stable ' "$WORKER_DIR/healthy-deploy.out" \
    || die "healthy Service did not render Stable for trial $WORKER_TRIAL"
  grep -Fq 'witness: startup probe[0] (tcp 0.0.0.0:18081)' \
    "$WORKER_DIR/healthy-deploy.out" \
    || die "healthy Stable event did not name its guest TCP startup witness"
  assert_serve_identity

  WORKER_STAGE=healthy-probe
  started="$(now_ns)"
  wait_for_service_healthy "$WORKER_HEALTHY_SERVICE" \
    "$WORKER_DIR/healthy-deploy.out" "$WORKER_DIR/healthy-describe.out" \
    "$WORKER_DIR/healthy-query-errors" \
    || die "healthy Service did not expose passing guest TCP/readiness probes"
  finished="$(now_ns)"
  record_timing "$WORKER_TIMING" healthy-probe "$started" "$finished"
  WORKER_HEALTHY_STATE="$(first_service_alloc_state <"$WORKER_DIR/healthy-describe.out")"
  WORKER_HEALTHY_TARGET="$(target_for_probe startup 18081 "$WORKER_DIR/healthy-describe.out")" \
    || die "healthy describe did not show the guest TCP target"
  all_service_alloc_ids <"$WORKER_DIR/healthy-describe.out" >"$WORKER_DIR/healthy-allocs"
  [[ -s "$WORKER_DIR/healthy-allocs" ]] || die "healthy describe carried no allocation identity"
  capture_case_resources "$WORKER_DIR/healthy-allocs" "$WORKER_DIR/healthy-resources-active"
  touch "$WORKER_COHORT/$WORKER_TRIAL.healthy-active"
  wait_for_gate "$WORKER_COHORT/healthy-release" \
    || die "healthy overlap barrier did not release trial $WORKER_TRIAL"

  WORKER_STAGE=healthy-client-deploy
  started="$(now_ns)"
  WORKER_CLIENT_DEPLOYED=1
  run_client_deploy "$healthy_client_spec" "$WORKER_DIR/healthy-client-deploy.out" \
    || die "healthy peer VM Job deploy failed for trial $WORKER_TRIAL"
  assert_serve_identity
  finished="$(now_ns)"
  record_timing "$WORKER_TIMING" healthy-client-deploy "$started" "$finished"
  WORKER_STAGE=healthy-client-probe
  started="$(now_ns)"
  wait_for_job_success "$WORKER_CLIENT_ID" "$WORKER_DIR/healthy-client-describe.out" \
    || die "healthy peer VM Job did not receive the exact guest reply"
  finished="$(now_ns)"
  record_timing "$WORKER_TIMING" healthy-client-probe "$started" "$finished"
  WORKER_HEALTHY_PEER=Succeeded
  stop_workload "$WORKER_CLIENT_ID" "$WORKER_DIR" healthy-client-stop.out healthy-client-stop-describe.out \
    || die "healthy peer VM Job did not stop through the public API"
  WORKER_CLIENT_DEPLOYED=0
  WORKER_STAGE=healthy-stop
  started="$(now_ns)"
  stop_workload "$WORKER_HEALTHY_SERVICE" "$WORKER_DIR" healthy-stop.out healthy-stop-describe.out \
    || die "healthy Service did not stop through the public API"
  finished="$(now_ns)"
  record_timing "$WORKER_TIMING" healthy-stop "$started" "$finished"
  WORKER_HEALTHY_DEPLOYED=0
  WORKER_HEALTHY_STATE="$(first_service_alloc_state <"$WORKER_DIR/healthy-stop-describe.out")"
  assert_case_resources_released "$WORKER_DIR/healthy-allocs" "$WORKER_DIR/healthy-resources-after" \
    || die "healthy allocation resources were not reclaimed"
  WORKER_HEALTHY_CLEANUP=zero-runtime

  assert_serve_identity
  WORKER_STAGE=failure-deploy
  started="$(now_ns)"
  WORKER_FAILURE_DEPLOYED=1
  if run_service_deploy "$failure_service_spec" "$WORKER_DIR/failure-deploy.out"; then
    rc=0
  else
    rc=$?
  fi
  finished="$(now_ns)"
  WORKER_FAILURE_DEPLOY_RC="$rc"
  record_timing "$WORKER_TIMING" failure-deploy "$started" "$finished"
  [[ "$rc" -ne 0 ]] || die "failed Service unexpectedly returned deploy success"
  assert_serve_identity
  WORKER_STAGE=failure-probe
  started="$(now_ns)"
  wait_for_service_failure "$WORKER_FAILURE_SERVICE" \
    "$WORKER_DIR/failure-deploy.out" "$WORKER_DIR/failure-describe.out" \
    "$WORKER_DIR/failure-query-errors" \
    || die "failed Service did not produce nonzero StartupProbeFailed without Stable"
  finished="$(now_ns)"
  record_timing "$WORKER_TIMING" failure-probe "$started" "$finished"
  WORKER_FAILURE_STATE="$(first_service_alloc_state <"$WORKER_DIR/failure-describe.out")"
  WORKER_FAILURE_REASON=StartupProbeFailed
  WORKER_FAILURE_TARGET="$(target_for_probe startup 18999 "$WORKER_DIR/failure-describe.out")" \
    || die "failed describe did not show the guest TCP target 18999"
  all_service_alloc_ids <"$WORKER_DIR/failure-describe.out" >"$WORKER_DIR/failure-allocs"
  [[ -s "$WORKER_DIR/failure-allocs" ]] || die "failure describe carried no allocation identity"
  capture_case_resources "$WORKER_DIR/failure-allocs" "$WORKER_DIR/failure-resources-active"
  touch "$WORKER_COHORT/$WORKER_TRIAL.failure-active"
  wait_for_gate "$WORKER_COHORT/failure-release" \
    || die "failure overlap barrier did not release trial $WORKER_TRIAL"

  WORKER_STAGE=failure-client-deploy
  started="$(now_ns)"
  WORKER_CLIENT_ID="$WORKER_FAILURE_CLIENT"
  WORKER_CLIENT_DEPLOYED=1
  run_client_deploy "$failure_client_spec" "$WORKER_DIR/failure-client-deploy.out" \
    || die "negative peer VM Job deploy failed for trial $WORKER_TRIAL"
  assert_serve_identity
  finished="$(now_ns)"
  record_timing "$WORKER_TIMING" failure-client-deploy "$started" "$finished"
  WORKER_STAGE=failure-client-probe
  started="$(now_ns)"
  wait_for_job_success "$WORKER_CLIENT_ID" "$WORKER_DIR/failure-client-describe.out" \
    || die "negative peer VM Job reached a failed result instead of proving unreachable"
  finished="$(now_ns)"
  record_timing "$WORKER_TIMING" failure-client-probe "$started" "$finished"
  WORKER_FAILURE_PEER=Succeeded
  stop_workload "$WORKER_CLIENT_ID" "$WORKER_DIR" failure-client-stop.out failure-client-stop-describe.out \
    || die "negative peer VM Job did not stop through the public API"
  WORKER_CLIENT_DEPLOYED=0
  WORKER_STAGE=failure-stop
  started="$(now_ns)"
  dispose_failed_service "$WORKER_FAILURE_SERVICE" "$WORKER_DIR" \
    || die "failed Service disposal lost its StartupProbeFailed evidence"
  finished="$(now_ns)"
  record_timing "$WORKER_TIMING" failure-stop-and-reclamation "$started" "$finished"
  WORKER_FAILURE_DEPLOYED=0
  WORKER_FAILURE_STATE="$(first_service_alloc_state <"$WORKER_DIR/failure-final-describe.out")"
  assert_case_resources_released "$WORKER_DIR/failure-allocs" "$WORKER_DIR/failure-resources-after" \
    || die "failure allocation resources were not reclaimed"
  WORKER_FAILURE_CLEANUP=zero-runtime
  WORKER_STAGE=complete
}

terminate_workers() {
  local pid
  for pid in "${WORKER_PIDS[@]}"; do
    kill -TERM "$pid" 2>/dev/null || true
  done
}

wait_worker_pids() {
  local wait_seconds="${1:-180}"
  local deadline=$((SECONDS + wait_seconds))
  local pid still_alive=0
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    still_alive=0
    for pid in "${WORKER_PIDS[@]}"; do
      if worker_is_alive "$pid"; then
        still_alive=1
      fi
    done
    [[ "$still_alive" -eq 0 ]] && break
    sleep 0.2
  done
  for pid in "${WORKER_PIDS[@]}"; do
    if worker_is_alive "$pid"; then
      kill -KILL "$pid" 2>/dev/null || true
    fi
  done
  local failed=0 rc
  for pid in "${WORKER_PIDS[@]}"; do
    set +e
    wait "$pid"
    rc=$?
    set -e
    [[ "$rc" -eq 0 ]] || failed=1
  done
  return "$failed"
}

abort_cohort_workers() {
  touch "$1/healthy-release" "$1/failure-release"
  terminate_workers
  wait_worker_pids || true
}

run_cohort() {
  local cohort_no="$1"
  local first_trial="$2"
  local last_trial="$3"
  local cohort_dir="$CASE_ROOT/cohort-$cohort_no"
  local count=$((last_trial - first_trial + 1))
  install -d -m 0700 "$cohort_dir"
  # The before/after pair is taken with every worker stopped.  It is a
  # cohort-local trend sample for intended durable records; the runtime
  # resource assertion below remains against the suite baseline.
  snapshot_all "$cohort_dir/before-snapshot"
  WORKER_PIDS=()
  WORKER_DIRS=()
  local trial case_dir
  for trial in $(seq "$first_trial" "$last_trial"); do
    case_dir="$CASE_ROOT/$trial"
    WORKER_DIRS+=("$case_dir")
    run_trial "$trial" "$case_dir" "$cohort_dir" >"$case_dir/transcript.out" 2>&1 &
    WORKER_PIDS+=("$!")
  done

  if ! wait_for_markers "$cohort_dir" healthy-active "$count"; then
    abort_cohort_workers "$cohort_dir"
    return 1
  fi
  assert_owned_active_cases "$cohort_dir" "$count" || {
    abort_cohort_workers "$cohort_dir"
    return 1
  }
  assert_serve_identity
  snapshot_all "$cohort_dir/healthy-active-snapshot"
  active_resource_count >"$cohort_dir/healthy-active-count"
  local active_vms owned_counts owned_cgroups owned_run_dirs
  active_vms="$(awk -F'[=\t]' '{print $2}' "$cohort_dir/healthy-active-count")"
  owned_counts="$(owned_active_case_count "$cohort_dir")"
  owned_cgroups="$(awk -F'[=\t]' '{print $2}' <<<"$owned_counts")"
  owned_run_dirs="$(awk -F'[=\t]' '{print $4}' <<<"$owned_counts")"
  printf 'cohort=%s\tphase=healthy\tworkers=%s\tactive_vms_host=%s\towned_cgroups=%s\towned_run_dirs=%s\n' \
    "$cohort_no" "$count" "${active_vms:-0}" "${owned_cgroups:-0}" \
    "${owned_run_dirs:-0}" >>"$MEASURE_ROOT/concurrency.tsv"
  touch "$cohort_dir/healthy-release"

  if ! wait_for_markers "$cohort_dir" failure-active "$count"; then
    abort_cohort_workers "$cohort_dir"
    return 1
  fi
  assert_serve_identity
  snapshot_all "$cohort_dir/failure-active-snapshot"
  active_resource_count >"$cohort_dir/failure-active-count"
  active_vms="$(awk -F'[=\t]' '{print $2}' "$cohort_dir/failure-active-count")"
  printf 'cohort=%s\tphase=failure\tworkers=%s\tactive_vms_host=%s\n' \
    "$cohort_no" "$count" "${active_vms:-0}" >>"$MEASURE_ROOT/concurrency.tsv"
  touch "$cohort_dir/failure-release"
  wait_worker_pids || return 1

  # All workers have stopped their peer Jobs and Services before this snapshot;
  # only then is a cohort-level diff meaningful. Intended records remain in the
  # control-plane store and are measured separately from runtime resources.
  snapshot_all "$cohort_dir/after-cleanup-snapshot"
  assert_no_new_runtime_resources "$BASELINE_DIR" "$cohort_dir/after-cleanup-snapshot" \
    || return 1
  record_store_growth "$cohort_no" "$cohort_dir/before-snapshot" \
    "$cohort_dir/after-cleanup-snapshot"
  return 0
}

ledger_header() {
  printf 'trial\toutcome\tstage\tcontrol_plane_pid\tcontrol_plane_start_ticks\thealthy_service_id\thealthy_deploy_rc\thealthy_terminal_state\thealthy_target\thealthy_peer\thealthy_allocations\thealthy_cleanup\tfailure_service_id\tfailure_deploy_rc\tfailure_terminal_state\tfailure_reason\tfailure_target\tfailure_peer\tfailure_allocations\tfailure_cleanup\n'
}

aggregate_ledger() {
  ledger_header >"$LEDGER"
  local trial result
  for trial in $(seq 1 "$E09_V2_PAIR_COUNT"); do
    result="$CASE_ROOT/$trial/result.tsv"
    if [[ -s "$result" ]]; then
      cat "$result" >>"$LEDGER"
    else
      local healthy_service failure_service
      printf -v healthy_service 'service-e09-v2-c%03d-h' "$trial"
      printf -v failure_service 'service-e09-v2-c%03d-f' "$trial"
      printf '%s\tnot-run-cancelled\tnot-started\t%s\t%s\t%s\tn/a\tn/a\tn/a\tn/a\tn/a\tn/a\t%s\tn/a\tn/a\tn/a\tn/a\tn/a\tn/a\tn/a\n' \
        "$trial" "${SERVE_PID:-}" "${SERVE_START_TICKS:-}" "$healthy_service" "$failure_service" \
        >>"$LEDGER"
    fi
  done
  [[ "$(($(wc -l <"$LEDGER") - 1))" -eq "$E09_V2_PAIR_COUNT" ]] \
    || die "result ledger does not contain exactly $E09_V2_PAIR_COUNT input pairs"
}

print_case_transcript() {
  local trial="$1"
  local directory="$CASE_ROOT/$trial"
  echo "--- case $trial begin ---"
  if [[ -f "$directory/transcript.out" ]]; then
    cat "$directory/transcript.out"
  fi
  local file
  for file in healthy-deploy.out healthy-describe.out healthy-client-deploy.out \
    healthy-client-describe.out healthy-client-stop.out healthy-client-stop-describe.out \
    healthy-stop.out \
    healthy-stop-describe.out failure-deploy.out failure-describe.out \
    failure-client-deploy.out failure-client-describe.out failure-client-stop.out \
    failure-client-stop-describe.out \
    failure-stop.out failure-final-describe.out healthy-query-errors \
    failure-query-errors healthy-resources-active healthy-resources-after \
    failure-resources-active failure-resources-after; do
    if [[ -f "$directory/$file" ]]; then
      echo "[$file]"
      cat "$directory/$file"
    fi
  done
  echo "--- case $trial end ---"
}

print_reports() {
  [[ "$REPORT_EMITTED" -eq 0 ]] || return 0
  [[ -n "$INPUT_MANIFEST" && -f "$INPUT_MANIFEST" ]] || return 0
  REPORT_EMITTED=1
  aggregate_ledger
  echo "E09 v2 metadata: one control-plane process, pid=${SERVE_PID:-stopped}, start_ticks=${SERVE_START_TICKS:-unknown}"
  echo "E09 v2 capacity: cpus=$HOST_CPUS memory_bytes=$HOST_MEM_BYTES configured_concurrency=$CONCURRENCY"
  echo '--- E09 v2 input manifest begin ---'
  cat "$INPUT_MANIFEST"
  echo '--- E09 v2 input manifest end ---'
  echo '--- E09 v2 ledger begin ---'
  cat "$LEDGER"
  echo '--- E09 v2 ledger end ---'
  echo '--- E09 v2 timing begin ---'
  cat "$TIMING_LOG"
  find "$CASE_ROOT" -mindepth 2 -maxdepth 2 -type f -name timing.tsv -print0 2>/dev/null \
    | sort -z \
    | while IFS= read -r -d '' file; do cat "$file"; done
  echo '--- E09 v2 timing end ---'
  echo '--- E09 v2 store growth begin ---'
  cat "$MEASURE_ROOT/store-growth.tsv"
  echo '--- E09 v2 store growth end ---'
  echo '--- E09 v2 concurrency begin ---'
  cat "$MEASURE_ROOT/concurrency.tsv"
  echo '--- E09 v2 concurrency end ---'
  echo '--- E09 v2 serve log begin ---'
  if [[ -f "$OUTPUT_ROOT/serve.log" ]]; then
    cat "$OUTPUT_ROOT/serve.log"
  fi
  echo '--- E09 v2 serve log end ---'
  local trial
  echo '--- E09 v2 case transcripts begin ---'
  for trial in $(seq 1 "$E09_V2_PAIR_COUNT"); do print_case_transcript "$trial"; done
  echo '--- E09 v2 case transcripts end ---'
  if [[ "$SUITE_FAILED" -eq 0 && "$SUITE_CANCELLED" -eq 0 ]]; then
    echo "E09 v2 PASS: $E09_V2_PAIR_COUNT/$E09_V2_PAIR_COUNT truthful functional pairs at concurrency $E09_V2_EXPECTED_CONCURRENCY through one unchanged control-plane PID; no retries, replacements, or discarded pairs"
  else
    echo "E09 v2 FAIL: suite_failed=$SUITE_FAILED suite_cancelled=$SUITE_CANCELLED; ledger preserves pass/failed/not-run outcomes"
  fi
}

assert_single_control_plane_in_ledger() {
  local identities="$MEASURE_ROOT/control-plane-ledger-identities"
  awk -F'\t' 'NR > 1 && $2 == "pass" { print $4 "\t" $5 }' "$LEDGER" \
    | LC_ALL=C sort -u >"$identities"
  [[ "$(wc -l <"$identities" | tr -d ' ')" -eq 1 ]] || {
    echo "svm-e09-v2 run: successful rows do not share one control-plane PID/start identity" >&2
    cat "$identities" >&2
    return 1
  }
  local identity
  identity="$(<"$identities")"
  [[ "$identity" == "$SERVE_PID"$'\t'"$SERVE_START_TICKS" ]] || {
    echo "svm-e09-v2 run: ledger identity differs from the live control plane: $identity" >&2
    return 1
  }
}

suite_cleanup() {
  local incoming_rc=$?
  trap - EXIT HUP INT TERM
  [[ "$incoming_rc" -eq 0 || "$SUITE_CANCELLED" -ne 0 ]] || SUITE_FAILED=1
  if [[ "$SUITE_STARTED" -eq 1 ]]; then
    local pid
    for pid in "${WORKER_PIDS[@]}"; do
      kill -TERM "$pid" 2>/dev/null || true
    done
    if [[ "$SUITE_CANCELLED" -ne 0 ]]; then
      # A deadline-triggered cleanup must leave enough of the 60-second grace
      # window for the partial report and bounded serve/preparer shutdown.
      wait_worker_pids 15 || true
    else
      wait_worker_pids || true
    fi
    if [[ -n "$BASELINE_DIR" && -d "$BASELINE_DIR" ]]; then
      local before_shutdown="$MEASURE_ROOT/final-before-shutdown"
      snapshot_all "$before_shutdown"
      assert_no_new_runtime_resources "$BASELINE_DIR" "$before_shutdown" || CLEANUP_FAILED=1
      printf 'final_runtime_resources_before_shutdown:\n' >>"$MEASURE_ROOT/final-cleanup"
      active_resource_count >>"$MEASURE_ROOT/final-cleanup"
    fi
    print_reports
    stop_serve || CLEANUP_FAILED=1
    if [[ -n "$BASELINE_DIR" && -d "$BASELINE_DIR" ]]; then
      local after_shutdown="$MEASURE_ROOT/final-after-shutdown"
      snapshot_all "$after_shutdown"
      assert_no_new_runtime_resources "$BASELINE_DIR" "$after_shutdown" || CLEANUP_FAILED=1
      printf 'final_runtime_resources_after_shutdown:\n' >>"$MEASURE_ROOT/final-cleanup"
      active_resource_count >>"$MEASURE_ROOT/final-cleanup"
    fi
    echo '--- E09 v2 final cleanup begin ---'
    cat "$MEASURE_ROOT/final-cleanup"
    echo 'final_after_shutdown_runtime_snapshot:'
    if [[ -n "$BASELINE_DIR" && -d "$BASELINE_DIR" ]]; then
      for name in hypervisors scopes run-dirs network bpf nft loops mounts clones store; do
        echo "[$name]"
        cat "$MEASURE_ROOT/final-after-shutdown/$name"
      done
    fi
    echo '--- E09 v2 final cleanup end ---'
    if [[ "$PREPARED" -eq 1 ]]; then
      if [[ "$SUITE_CANCELLED" -ne 0 ]]; then
        bounded 15s "$PREPARE" cleanup || CLEANUP_FAILED=1
      else
        bounded 60s "$PREPARE" cleanup || CLEANUP_FAILED=1
      fi
    fi
  fi
  if [[ "$CLEANUP_FAILED" -ne 0 && "$incoming_rc" -eq 0 ]]; then
    incoming_rc=1
  fi
  exit "$incoming_rc"
}

run_suite() {
  require_native_metal
  local command
  for command in awk basename bpftool cargo cat cloud-hypervisor comm date du find \
    findmnt grep install ip keyctl losetup nft nproc readlink script sed setsid \
    sleep sort stat timeout tr wc; do
    require_command "$command"
  done
  cd "$REPO_ROOT"
  "$PREPARE" check-source
  [[ ! -e "$OUTPUT_ROOT" ]] || die "refusing to overwrite pre-existing v2 output: $OUTPUT_ROOT"
  [[ "$OUTPUT_ROOT" =~ ^/srv/vm/overdrive-testing/[A-Za-z0-9._/-]+$ ]] \
    || die "SVM_E09_V2_OUTPUT_ROOT must contain only safe path characters below /srv/vm/overdrive-testing"
  SUITE_STARTED=1
  trap suite_cleanup EXIT
  trap 'SUITE_CANCELLED=1; SUITE_FAILED=1; exit 130' HUP INT TERM

  local started finished build_started build_finished prepare_started prepare_finished
  build_started="$(now_ns)"
  bounded 600s cargo build -p overdrive-cli --bin overdrive
  build_finished="$(now_ns)"
  [[ -x "$BIN" ]] || die "default-feature product binary was not built: $BIN"

  prepare_started="$(now_ns)"
  SVM_E09_V2_OWNERSHIP_TOKEN="e09-v2-suite-$$" bounded 300s "$PREPARE" prepare
  prepare_finished="$(now_ns)"
  PREPARED=1

  # The preparer is the sole creator of the marker-owned output root.  Create
  # suite measurements and generated case specs only after preparation has
  # committed, so prepare's refuse-to-overwrite guard remains meaningful.
  TIMING_LOG="$MEASURE_ROOT/timing.tsv"
  LEDGER="$MEASURE_ROOT/e09-v2-ledger.tsv"
  install -d -m 0700 "$MEASURE_ROOT"
  printf 'stage\tduration_ms\n' >"$TIMING_LOG"
  : >"$MEASURE_ROOT/concurrency.tsv"
  printf 'cohort\tstore_files_before\tstore_files_after\tstore_files_delta\tstore_bytes_before\tstore_bytes_after\tstore_bytes_delta\n' \
    >"$MEASURE_ROOT/store-growth.tsv"
  : >"$MEASURE_ROOT/final-cleanup"
  record_timing "$TIMING_LOG" build "$build_started" "$build_finished"
  record_timing "$TIMING_LOG" prepare "$prepare_started" "$prepare_finished"
  configure_capacity
  materialize_specs

  start_serve || die "serve did not become ready within the bounded startup window"
  BASELINE_DIR="$MEASURE_ROOT/suite-baseline"
  snapshot_all "$BASELINE_DIR"
  printf 'initial_control_plane_pid=%s\tstart_ticks=%s\n' "$SERVE_PID" "$SERVE_START_TICKS" \
    >>"$MEASURE_ROOT/control-plane-identity"

  local cohort_no=1 first_trial=1 last_trial rc
  while [[ "$first_trial" -le "$E09_V2_PAIR_COUNT" ]]; do
    last_trial=$((first_trial + CONCURRENCY - 1))
    [[ "$last_trial" -le "$E09_V2_PAIR_COUNT" ]] || last_trial="$E09_V2_PAIR_COUNT"
    if run_cohort "$cohort_no" "$first_trial" "$last_trial"; then
      :
    else
      rc=$?
      SUITE_FAILED=1
      echo "svm-e09-v2 run: cohort $cohort_no failed with rc=$rc; remaining inputs are recorded not-run" >&2
      break
    fi
    first_trial=$((last_trial + 1))
    cohort_no=$((cohort_no + 1))
  done
  [[ "$SUITE_FAILED" -eq 0 ]] || return 1
  aggregate_ledger
  assert_single_control_plane_in_ledger || return 1
  local pass_count
  pass_count="$(awk -F'\t' 'NR > 1 && $2 == "pass" { count++ } END { print count + 0 }' "$LEDGER")"
  [[ "$pass_count" -eq "$E09_V2_PAIR_COUNT" ]] \
    || die "E09 v2 requires exactly $E09_V2_PAIR_COUNT pass rows; observed $pass_count"
}

case "${1:-}" in
  check-source)
    "$PREPARE" check-source
    ;;
  run)
    case "${2:-}" in
      tcp-truthfulness-20) run_suite ;;
      *) die 'usage: run-example.sh run tcp-truthfulness-20' ;;
    esac
    ;;
  *) die 'usage: run-example.sh check-source|run tcp-truthfulness-20' ;;
esac

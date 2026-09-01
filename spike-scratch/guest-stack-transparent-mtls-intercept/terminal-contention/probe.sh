#!/usr/bin/env bash
set -u
set -o pipefail

WORKSPACE=/Users/marcus/conductor/workspaces/helios/krakow-v3
SCRATCH="$WORKSPACE/spike-scratch/guest-stack-transparent-mtls-intercept/terminal-contention"
TRANSCRIPT="$SCRATCH/transcript.log"
RUNS="$SCRATCH/runs"
QUESTION="With the repository's real default-feature Overdrive production binary running on its configured bare-metal target, when a real guest VM Job's exit contends with the public stop path, do the existing durable ObservationStore terminal state and normal cleanup converge correctly without additional outbox, replay, hydration, or recovery machinery?"
IDENTITY="/root/spike_terminal_contention_v2 — NW-SPIKE Phase 1 PROBE"

utc_now() {
  date -u +'%Y-%m-%dT%H:%M:%S.%NZ'
}

shell_quote() {
  printf '%q ' "$@"
}

init_transcript() {
  mkdir -p "$RUNS"
  : > "$TRANSCRIPT"
  {
    printf 'OVERDRIVE TERMINAL-CONTENTION PROBE TRANSCRIPT\n'
    printf 'workspace: %s\n' "$WORKSPACE"
    printf 'git_commit: %s\n' "$(git -C "$WORKSPACE" rev-parse HEAD 2>&1)"
    printf 'dirty_state_snapshot_begin\n'
    git -C "$WORKSPACE" status --short --untracked-files=all 2>&1
    printf 'dirty_state_snapshot_end\n'
    printf 'agent_probe_identity: %s\n' "$IDENTITY"
    printf 'start_time_utc: %s\n' "$(utc_now)"
    printf 'declared_question: %s\n' "$QUESTION"
    printf 'budget_seconds: 3600\n'
    printf 'transcript_format: each substantive command records UTC start/end, exact command, exit code, stdout, stderr\n'
  } >> "$TRANSCRIPT"
}

next_run_id() {
  local latest
  latest=$(find "$RUNS" -maxdepth 1 -type f -name '*.meta' -print 2>/dev/null | wc -l | tr -d ' ')
  printf '%04d' "$((latest + 1))"
}

run_logged() {
  local step=$1
  shift
  local command=$1
  local run_id started ended exit_code stdout_file stderr_file
  run_id=$(next_run_id)
  started=$(utc_now)
  stdout_file="$RUNS/${run_id}.stdout"
  stderr_file="$RUNS/${run_id}.stderr"
  {
    printf '\nCOMMAND_BEGIN id=%s\n' "$run_id"
    printf 'step: %s\n' "$step"
    printf 'started_utc: %s\n' "$started"
    printf 'exact_shell_command: %s\n' "$command"
  } >> "$TRANSCRIPT"

  set +e
  (cd "$WORKSPACE" && bash -lc "$command") >"$stdout_file" 2>"$stderr_file"
  exit_code=$?
  set -e
  ended=$(utc_now)

  {
    printf 'stdout_begin\n'
    sed -n '1,$p' "$stdout_file"
    printf 'stdout_end\n'
    printf 'stderr_begin\n'
    sed -n '1,$p' "$stderr_file"
    printf 'stderr_end\n'
    printf 'exit_code: %s\n' "$exit_code"
    printf 'ended_utc: %s\n' "$ended"
    printf 'COMMAND_END id=%s\n' "$run_id"
  } >> "$TRANSCRIPT"
  if [[ -s "$stdout_file" ]]; then
    sed -n '1,$p' "$stdout_file"
  fi
  if [[ -s "$stderr_file" ]]; then
    sed -n '1,$p' "$stderr_file" >&2
  fi
  {
    printf 'step=%s\n' "$step"
    printf 'started_utc=%s\n' "$started"
    printf 'ended_utc=%s\n' "$ended"
    printf 'exit_code=%s\n' "$exit_code"
    printf 'command=%s\n' "$command"
  } > "$RUNS/${run_id}.meta"
  return "$exit_code"
}

note() {
  local kind=$1
  shift
  {
    printf '\nNOTE_BEGIN\n'
    printf 'timestamp_utc: %s\n' "$(utc_now)"
    printf 'kind: %s\n' "$kind"
    printf 'text: %s\n' "$*"
    printf 'NOTE_END\n'
  } >> "$TRANSCRIPT"
}

# Phase-1 native-metal probe. This entry point is invoked only through
# `cargo xtask metal run --`, which supplies the canonical shared lease, root
# context, repository sync, and native-host preflight. Runtime files below are
# the checked-in example's marker-owned product materialization, not probe
# evidence; all durable probe evidence is printed and captured by run_logged.
remote_probe() {
  set -euo pipefail

  declare -g repo_root example_dir prepare session_lifecycle session_wrapper output_root
  declare -g creds_dir bin bind caller_id callee_id caller_alloc callee_alloc kek_description
  repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
  example_dir="$repo_root/examples/guest-stack-transparent-mtls-intercept"
  prepare="$example_dir/prepare.sh"
  session_lifecycle="$example_dir/session-lifecycle.sh"
  session_wrapper="$example_dir/session-wrapper.sh"
  output_root=/srv/vm/overdrive-testing/gti-e07
  creds_dir="$output_root/credentials"
  bin="$repo_root/target/debug/overdrive"
  bind=127.0.0.1:7643
  caller_id=gti-e07-caller
  callee_id=gti-e07-callee
  caller_alloc=alloc-gti-e07-caller-0
  callee_alloc=alloc-gti-e07-callee-0
  kek_description=overdrive:ca:kek:overdrive-ca-root

  declare -g prepare_token="" prepare_attempted=0
  declare -g config_dir="" data_dir="" run_root="" serve_log=""
  declare -g session_ready_file="" session_ack_file="" serve_pid_file=""
  declare -g serve_pid="" serve_pid_start="" serve_pgid=""
  declare -g SESSION_WRAPPER_PID="" SESSION_WRAPPER_START="" SESSION_WRAPPER_PGID=""
  declare -g SESSION_GROUP_OWNED=0 SESSION_WRAPPER_REAPED=0 SESSION_LAUNCH_TOKEN=""
  declare -g SESSION_READY_FILE="" SESSION_ACK_FILE=""
  declare -g caller_deployed=0 callee_deployed=0
  declare -g caller_pid="" caller_start="" caller_exe="" caller_scope=""
  declare -g callee_pid="" callee_start="" callee_exe="" callee_scope=""
  declare -g caller_stopped=0 callee_stopped=0 stream_wrapper_pid=""

  # shellcheck source=/dev/null
  source "$session_lifecycle"

  stamp() {
    printf 'PROBE timestamp_utc=%s %s\n' "$(date -u +'%Y-%m-%dT%H:%M:%S.%NZ')" "$*"
  }

  fail() {
    stamp "FAIL $*"
    return 1
  }

  bounded() {
    local duration=$1
    shift
    timeout --foreground --signal=TERM --kill-after=5s "$duration" "$@"
  }

  proc_identity() {
    local pid=$1 line rest
    [[ "$pid" =~ ^[0-9]+$ && -r "/proc/$pid/stat" ]] || return 1
    IFS= read -r line <"/proc/$pid/stat" || return 1
    rest="${line##*) }"
    # shellcheck disable=SC2086
    set -- $rest
    [[ "$#" -ge 20 ]] || return 1
    printf '%s %s %s\n' "${1:0:1}" "$3" "${20}"
  }

  pid_in_exact_scope() {
    local pid=$1 scope=$2
    awk -F: -v scope="$scope" '$3 == scope { found=1 } END { exit !found }' \
      "/proc/$pid/cgroup"
  }

  owned_pid_matches() {
    local pid=$1 expected_start=$2 expected_exe=$3 expected_scope=$4
    local identity state pgid start
    identity="$(proc_identity "$pid")" || return 1
    read -r state pgid start <<<"$identity"
    [[ "$state" != Z && "$start" == "$expected_start" ]] || return 1
    [[ "$(readlink -f "/proc/$pid/exe")" == "$expected_exe" ]] || return 1
    pid_in_exact_scope "$pid" "$expected_scope"
  }

  resolve_exact_pid() {
    local scope=$1 kind=$2 expected_exe=${3:-}
    local cgroup_file="/sys/fs/cgroup${scope}/cgroup.procs"
    local -a pids=()
    local pid identity state pgid start exe cmdline
    [[ -r "$cgroup_file" ]] || fail "missing exact cgroup.procs $cgroup_file"
    mapfile -t pids <"$cgroup_file"
    [[ "${#pids[@]}" -eq 1 ]] \
      || fail "exact cgroup $scope contains ${#pids[@]} processes, expected one: ${pids[*]:-none}"
    pid=${pids[0]}
    identity="$(proc_identity "$pid")" || fail "cannot read identity for exact cgroup pid=$pid"
    read -r state pgid start <<<"$identity"
    exe="$(readlink -f "/proc/$pid/exe")" || fail "cannot resolve exe for pid=$pid"
    pid_in_exact_scope "$pid" "$scope" || fail "pid=$pid is not in exact scope=$scope"
    if [[ "$kind" == exec ]]; then
      [[ "$exe" == "$(readlink -f "$expected_exe")" ]] \
        || fail "exec pid=$pid exe=$exe expected=$(readlink -f "$expected_exe")"
    else
      [[ "$(basename "$exe")" == cloud-hypervisor ]] \
        || fail "VM cgroup pid=$pid has non-Cloud-Hypervisor exe=$exe"
      cmdline="$(tr '\0' ' ' <"/proc/$pid/cmdline")"
      [[ "$cmdline" == *"$caller_alloc"* ]] \
        || fail "VM pid=$pid cmdline does not bind exact allocation: $cmdline"
    fi
    printf '%s %s %s %s %s\n' "$pid" "$start" "$exe" "$state" "$pgid"
  }

  signal_exact_owned_pid() {
    local signal=$1 pid=$2 start=$3 exe=$4 scope=$5 label=$6
    owned_pid_matches "$pid" "$start" "$exe" "$scope" \
      || fail "refusing SIG$signal: $label ownership proof no longer matches pid=$pid"
    stamp "SIGNAL label=$label signal=SIG$signal pid=$pid start=$start exe=$exe scope=$scope"
    kill -"$signal" "$pid"
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

  public_describe() {
    local id=$1 out=$2
    bounded 10s env OVERDRIVE_CONFIG_DIR="$config_dir" \
      "$bin" workload describe "$id" >"$out" 2>&1
  }

  wait_service_running() {
    local out=$1 state="" deadline=$((SECONDS + 90))
    while (( SECONDS < deadline )); do
      public_describe "$callee_id" "$out" || true
      state="$(first_service_alloc_state <"$out")"
      [[ "$state" == Running ]] && grep -Fxq 'Replicas (desired/running): 1/1' "$out" && return 0
      [[ "$state" == Failed || "$state" == Terminated ]] && break
      sleep 0.25
    done
    fail "callee did not reach public Running state; last=${state:-none}"
  }

  wait_job_running() {
    local out=$1 state="" deadline=$((SECONDS + 90))
    while (( SECONDS < deadline )); do
      public_describe "$caller_id" "$out" || true
      state="$(first_job_attempt_state <"$out")"
      [[ "$state" == Running ]] && return 0
      [[ "$state" == Failed || "$state" == Terminated || "$state" == Stopped ]] && break
      sleep 0.1
    done
    fail "caller did not reach public Running state; last=${state:-none}"
  }

  wait_job_terminal() {
    local out=$1 state="" deadline=$((SECONDS + 45))
    while (( SECONDS < deadline )); do
      public_describe "$caller_id" "$out" || true
      state="$(first_job_attempt_state <"$out")"
      case "$state" in Stopped|Terminated|Failed) printf '%s\n' "$state"; return 0 ;; esac
      sleep 0.1
    done
    fail "caller did not reach public terminal state; last=${state:-none}"
  }

  wait_scope_absent() {
    local scope=$1 deadline=$((SECONDS + 30))
    while (( SECONDS < deadline )); do
      [[ ! -e "/sys/fs/cgroup${scope}" ]] && return 0
      sleep 0.1
    done
    fail "allocation cgroup remains after public terminal cleanup: $scope"
  }

  wait_allocation_network_absent() {
    local deadline=$((SECONDS + 30))
    while (( SECONDS < deadline )); do
      if ! ip netns list | grep -q '^ovd-ns-' \
        && ! ip -o link show | grep -Eq 'ovd-(hv|wl)-|(^|[[:space:]])tap[[:alnum:]_-]*[[:space:]@:]'; then
        return 0
      fi
      sleep 0.1
    done
    fail "allocation-scoped netns/veth/tap artifacts remain after public terminal cleanup"
  }

  serve_pid_is_exact() {
    local identity state pgid start
    [[ -n "$serve_pid" && -n "$serve_pid_start" && -e "/proc/$serve_pid/exe" ]] || return 1
    identity="$(e07_session_process_identity "$serve_pid")" || return 1
    read -r state pgid start <<<"$identity"
    [[ "$start" == "$serve_pid_start" && "$pgid" == "$serve_pgid" ]] || return 1
    [[ "$(readlink -f "/proc/$serve_pid/exe")" == "$(readlink -f "$bin")" ]]
  }

  serve_pid_belongs_to_owned_group() {
    local identity state pgid start
    [[ "$SESSION_GROUP_OWNED" -eq 1 && -n "$serve_pid_start" ]] || return 1
    identity="$(e07_session_process_identity "$serve_pid")" || return 1
    read -r state pgid start <<<"$identity"
    [[ "$start" == "$serve_pid_start" && "$pgid" == "$serve_pgid" \
      && "$pgid" == "$SESSION_WRAPPER_PGID" ]]
  }

  read_serve_identity() {
    local token pid pgid start extra=""
    read -r token pid pgid start extra <"$serve_pid_file" || return 1
    [[ -z "$extra" && "$token" == "$SESSION_LAUNCH_TOKEN" \
      && "$pid" =~ ^[0-9]+$ && "$pgid" == "$SESSION_WRAPPER_PGID" \
      && "$start" =~ ^[0-9]+$ ]] || return 1
    serve_pid=$pid
    serve_pgid=$pgid
    serve_pid_start=$start
    serve_pid_belongs_to_owned_group
  }

  launch_serve() {
    local launched_pid capture_ticks=10 pid_attempts=50
    SESSION_WRAPPER_PID=""
    SESSION_WRAPPER_START=""
    SESSION_WRAPPER_PGID=""
    SESSION_GROUP_OWNED=0
    # shellcheck disable=SC2034 # consumed by sourced lifecycle helpers
    SESSION_WRAPPER_REAPED=0
    setsid "$session_wrapper" "$SESSION_LAUNCH_TOKEN" "$session_ready_file" \
      "$session_ack_file" "$serve_pid_file" "$kek_description" "$config_dir" \
      "$creds_dir" "$bin" "$bind" "$data_dir" >"$serve_log" 2>&1 &
    launched_pid=$!
    SESSION_WRAPPER_PID=$launched_pid
    while [[ -z "$SESSION_WRAPPER_START" && "$capture_ticks" -gt 0 ]]; do
      e07_session_capture_wrapper "$launched_pid" && break
      kill -0 "$launched_pid" 2>/dev/null || break
      sleep 0.01
      capture_ticks=$((capture_ticks - 1))
    done
    [[ -n "$SESSION_WRAPPER_START" ]] || fail "session wrapper exited before ownership capture"
    e07_session_wait_for_private_group 50 || fail "session wrapper did not establish private group"
    e07_session_acknowledge_ownership || fail "session wrapper acknowledgement failed"
    e07_session_wait_for_ready 50 || fail "session wrapper readiness failed"
    while [[ ! -s "$serve_pid_file" && "$pid_attempts" -gt 0 ]]; do
      e07_session_unit_is_live || fail "session unit exited before serve PID publication"
      sleep 0.1
      pid_attempts=$((pid_attempts - 1))
    done
    [[ -s "$serve_pid_file" ]] || fail "serve PID was not published"
    read_serve_identity || fail "serve published an invalid/foreign identity"
    local deadline=$((SECONDS + 30))
    while (( SECONDS < deadline )); do
      [[ -f "$config_dir/.overdrive/config" ]] && serve_pid_is_exact && return 0
      sleep 0.25
    done
    fail "serve did not become ready"
  }

  terminate_serve() {
    [[ -n "$SESSION_WRAPPER_PID" ]] || return 0
    if [[ -n "$serve_pid" && -e "/proc/$serve_pid" ]]; then
      serve_pid_is_exact || serve_pid_belongs_to_owned_group \
        || fail "refusing to terminate foreign serve PID=$serve_pid"
    fi
    e07_session_terminate_owned_unit || fail "owned serve session did not terminate"
    stamp "CLEANUP owned_serve_session=absent wrapper_pid=${SESSION_WRAPPER_PID:-none} serve_pid=${serve_pid:-none}"
    SESSION_WRAPPER_PID=""
    serve_pid=""
  }

  public_stop() {
    local id=$1 out=$2 rc=0
    set +e
    bounded 20s env OVERDRIVE_CONFIG_DIR="$config_dir" "$bin" job stop "$id" >"$out" 2>&1
    rc=$?
    set -e
    stamp "PUBLIC_STOP id=$id exit_code=$rc"
    sed 's/^/PUBLIC_STOP_OUTPUT /' "$out"
    return "$rc"
  }

  resume_owned_stops() {
    if [[ "$caller_stopped" -eq 1 ]] && owned_pid_matches "$caller_pid" "$caller_start" "$caller_exe" "$caller_scope"; then
      signal_exact_owned_pid CONT "$caller_pid" "$caller_start" "$caller_exe" "$caller_scope" caller-vmm
    fi
    caller_stopped=0
    if [[ "$callee_stopped" -eq 1 ]] && owned_pid_matches "$callee_pid" "$callee_start" "$callee_exe" "$callee_scope"; then
      signal_exact_owned_pid CONT "$callee_pid" "$callee_start" "$callee_exe" "$callee_scope" callee-exec
    fi
    callee_stopped=0
  }

  cleanup_iteration() {
    local rc=0
    resume_owned_stops || rc=1
    if [[ "$caller_deployed" -eq 1 ]]; then public_stop "$caller_id" "$run_root/caller-cleanup-stop.log" || rc=1; fi
    if [[ "$callee_deployed" -eq 1 ]]; then public_stop "$callee_id" "$run_root/callee-stop.log" || rc=1; fi
    [[ -z "$caller_scope" ]] || wait_scope_absent "$caller_scope" || rc=1
    [[ -z "$callee_scope" ]] || wait_scope_absent "$callee_scope" || rc=1
    terminate_serve || rc=1
    caller_deployed=0
    callee_deployed=0
    return "$rc"
  }

  final_cleanup() {
    local incoming_rc=$? rc=0
    trap - EXIT HUP INT TERM
    cleanup_iteration || rc=1
    if [[ "$prepare_attempted" -eq 1 && -f "$output_root/.gti-e07-owned" \
      && "$(<"$output_root/.gti-e07-owned")" == "gti-e07-owned-v1:$prepare_token" ]]; then
      bounded 45s env GTI_E07_OWNERSHIP_TOKEN="$prepare_token" "$prepare" cleanup || rc=1
    fi
    if [[ -e "$output_root" ]]; then
      stamp "CLEANUP marker_owned_output=REMAINS path=$output_root"
      rc=1
    else
      stamp "CLEANUP marker_owned_output=absent path=$output_root"
    fi
    stamp "FINAL_CLEANUP incoming_exit=$incoming_rc cleanup_exit=$rc"
    if [[ "$incoming_rc" -ne 0 ]]; then exit "$incoming_rc"; fi
    exit "$rc"
  }

  host_snapshot() {
    local label=$1
    stamp "HOST_SNAPSHOT_BEGIN label=$label"
    printf '%s\n' '--- ip netns list ---'
    ip netns list || true
    printf '%s\n' '--- allocation-ish links ---'
    ip -o link show | grep -E 'ovd-|tap|veth' || true
    printf '%s\n' '--- nft table ip overdrive-mtls ---'
    nft -a list table ip overdrive-mtls 2>&1 || true
    printf '%s\n' '--- exact workload scopes ---'
    for scope in "/sys/fs/cgroup${caller_scope:-/nonexistent}" "/sys/fs/cgroup${callee_scope:-/nonexistent}"; do
      if [[ -e "$scope" ]]; then printf 'PRESENT %s\n' "$scope"; else printf 'ABSENT %s\n' "$scope"; fi
    done
    stamp "HOST_SNAPSHOT_END label=$label"
  }

  require_remote_preconditions() {
    [[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 && "$(id -u)" -eq 0 ]] \
      || fail "native x86_64 Linux root context is required"
    [[ -c /dev/kvm ]] || fail "/dev/kvm unavailable"
    [[ -r /run/lock/overdrive-metal-shared.owner ]] || fail "canonical lease metadata absent"
    grep -qx action=run /run/lock/overdrive-metal-shared.owner \
      || fail "canonical metal Run lease is not held"
    local command
    for command in awk cargo file grep ip keyctl mapfile nft nsenter readlink rustc script setsid ss timeout; do
      command -v "$command" >/dev/null 2>&1 || fail "required command unavailable: $command"
    done
  }

  run_iteration() {
    local iteration=$1 delay=$2
    local callee_describe caller_running caller_terminal caller_stop stream_file queue_file
    local resolved state terminal_state stream_rc=0 deadline connection_count
    stamp "ITERATION_BEGIN number=$iteration post_stop_delay_seconds=$delay"
    run_root="$output_root/probe-run-$iteration"
    data_dir="$run_root/data"
    config_dir="$run_root/config"
    serve_log="$run_root/serve.log"
    session_ready_file="$run_root/session-wrapper.ready"
    session_ack_file="$run_root/session-wrapper.ack"
    serve_pid_file="$run_root/serve.pid"
    # shellcheck disable=SC2034 # consumed by sourced lifecycle helpers
    SESSION_READY_FILE="$session_ready_file"
    # shellcheck disable=SC2034 # consumed by sourced lifecycle helpers
    SESSION_ACK_FILE="$session_ack_file"
    mkdir -p "$data_dir" "$config_dir"
    chmod 0711 "$run_root" "$data_dir"
    chmod 0700 "$config_dir"
    SESSION_LAUNCH_TOKEN="$prepare_token-iteration-$iteration"
    caller_scope="/overdrive.slice/workloads.slice/$caller_alloc.scope"
    callee_scope="/overdrive.slice/workloads.slice/$callee_alloc.scope"
    caller_deployed=0; callee_deployed=0; caller_stopped=0; callee_stopped=0
    caller_pid=""; callee_pid=""; stream_wrapper_pid=""
    launch_serve

    callee_describe="$run_root/callee-describe.log"
    caller_running="$run_root/caller-running.log"
    caller_terminal="$run_root/caller-terminal.log"
    caller_stop="$run_root/caller-stop.log"
    stream_file="$run_root/caller-public-stream.log"
    queue_file="$run_root/queued-request.ss"

    bounded 30s env OVERDRIVE_CONFIG_DIR="$config_dir" "$bin" deploy --detach "$example_dir/callee.toml"
    callee_deployed=1
    wait_service_running "$callee_describe"
    stamp "PUBLIC_DESCRIBE callee_running"
    sed 's/^/CALLEE_DESCRIBE /' "$callee_describe"

    resolved="$(resolve_exact_pid "$callee_scope" exec "$output_root/bin/e07-callee")"
    read -r callee_pid callee_start callee_exe state _ <<<"$resolved"
    stamp "OWNERSHIP label=callee-exec pid=$callee_pid start=$callee_start state=$state exe=$callee_exe scope=$callee_scope"
    signal_exact_owned_pid STOP "$callee_pid" "$callee_start" "$callee_exe" "$callee_scope" callee-exec
    callee_stopped=1

    # util-linux script supplies a real PTY, so this is the public deploy
    # streaming path rather than detach/poll masquerading as streaming.
    set +e
    bounded 150s script -qefc \
      "env OVERDRIVE_CONFIG_DIR=$config_dir $bin deploy $example_dir/caller.toml" \
      "$stream_file" >/dev/null 2>&1 &
    stream_wrapper_pid=$!
    set -e
    caller_deployed=1
    wait_job_running "$caller_running"
    stamp "PUBLIC_DESCRIBE caller_running"
    sed 's/^/CALLER_RUNNING /' "$caller_running"

    resolved="$(resolve_exact_pid "$caller_scope" vm)"
    read -r caller_pid caller_start caller_exe state _ <<<"$resolved"
    stamp "OWNERSHIP label=caller-vmm pid=$caller_pid start=$caller_start state=$state exe=$caller_exe scope=$caller_scope"

    deadline=$((SECONDS + 5))
    connection_count=0
    while (( SECONDS < deadline )); do
      nsenter -t "$callee_pid" -n ss -Htn sport = :18951 >"$queue_file" 2>&1 || true
      connection_count="$(awk '$1 == "ESTAB" && ($2 + 0) > 0 { n++ } END { print n+0 }' "$queue_file")"
      [[ "$connection_count" -eq 1 ]] && break
      sleep 0.02
    done
    [[ "$connection_count" -eq 1 ]] \
      || fail "did not observe exactly one established connection with queued request bytes"
    stamp "FAULT_PRECONDITION exact_connections_with_recvq=1 callee_pid=$callee_pid"
    sed 's/^/QUEUED_REQUEST_SS /' "$queue_file"

    signal_exact_owned_pid STOP "$caller_pid" "$caller_start" "$caller_exe" "$caller_scope" caller-vmm
    caller_stopped=1
    signal_exact_owned_pid CONT "$callee_pid" "$callee_start" "$callee_exe" "$callee_scope" callee-exec
    callee_stopped=0
    sleep 0.15
    nsenter -t "$callee_pid" -n ss -Htn sport = :18951 2>&1 \
      | sed 's/^/POST_RESPONSE_SS /' || true

    public_stop "$caller_id" "$caller_stop"
    stamp "CONTENTION_WINDOW public_stop_returned=1 caller_vmm_state=$(proc_identity "$caller_pid" | awk '{print $1}') delay_before_resume=$delay"
    sleep "$delay"
    if owned_pid_matches "$caller_pid" "$caller_start" "$caller_exe" "$caller_scope"; then
      sed -n '/^State:/p;/^SigPnd:/p;/^ShdPnd:/p;/^SigBlk:/p' "/proc/$caller_pid/status" \
        | sed 's/^/FROZEN_VMM_STATUS /'
      signal_exact_owned_pid CONT "$caller_pid" "$caller_start" "$caller_exe" "$caller_scope" caller-vmm
    else
      fail "caller VMM disappeared while SIGSTOP ownership was held"
    fi
    caller_stopped=0

    set +e
    wait "$stream_wrapper_pid"
    stream_rc=$?
    set -e
    stamp "PUBLIC_STREAM_EXIT iteration=$iteration exit_code=$stream_rc"
    sed 's/^/PUBLIC_STREAM /' "$stream_file"
    terminal_state="$(wait_job_terminal "$caller_terminal")"
    stamp "PUBLIC_DESCRIBE caller_terminal state=$terminal_state"
    sed 's/^/CALLER_TERMINAL /' "$caller_terminal"

    public_stop "$callee_id" "$run_root/callee-stop.log"
    wait_scope_absent "$caller_scope"
    wait_scope_absent "$callee_scope"
    wait_allocation_network_absent
    stamp "CLEANUP exact_allocation_scopes=absent allocation_netns_veth_tap=absent iteration=$iteration"
    host_snapshot "iteration-$iteration-before-serve-exit"
    terminate_serve
    stamp "SERVE_LOG_BEGIN iteration=$iteration"
    sed 's/^/SERVE_LOG /' "$serve_log"
    stamp "SERVE_LOG_END iteration=$iteration"
    caller_deployed=0
    callee_deployed=0
    [[ "$terminal_state" == Terminated && "$stream_rc" -eq 130 \
      && "$(grep -Fc 'reason: stopped' "$caller_terminal")" -eq 1 \
      && "$(grep -Fc "Job '$caller_id' stopped by operator." "$stream_file")" -eq 1 ]] \
      || fail "public stream/current terminal projections did not converge to operator-stopped outcome"
    stamp "ITERATION_RESULT number=$iteration durable_alloc_state=Terminated durable_reason=stopped public_stream=operator-stopped stream_exit=$stream_rc scopes_absent=yes allocation_network_absent=yes"
    stamp "ITERATION_END number=$iteration"
  }

  require_remote_preconditions
  stamp "REMOTE_PROBE_BEGIN commit=$(git -C "$repo_root" rev-parse HEAD)"
  stamp "LEASE_METADATA_BEGIN"
  sed 's/^/LEASE /' /run/lock/overdrive-metal-shared.owner
  stamp "LEASE_METADATA_END"
  "$prepare" check-source
  bounded 600s cargo build --manifest-path "$repo_root/Cargo.toml" -p overdrive-cli --bin overdrive
  [[ -x "$bin" ]] || fail "default-feature overdrive product binary absent after build"
  stamp "PRODUCT_BINARY path=$bin sha256=$(sha256sum "$bin" | awk '{print $1}')"
  [[ ! -e "$output_root" ]] || fail "$output_root pre-exists; refusing overwrite/cleanup"
  IFS= read -r prepare_token </proc/sys/kernel/random/uuid
  [[ "$prepare_token" =~ ^[A-Fa-f0-9-]+$ ]] || fail "invalid ownership token"
  prepare_attempted=1
  trap final_cleanup EXIT HUP INT TERM
  bounded 240s env GTI_E07_OWNERSHIP_TOKEN="$prepare_token" "$prepare" prepare
  bounded 45s env GTI_E07_OWNERSHIP_TOKEN="$prepare_token" "$prepare" check
  caller_scope="/overdrive.slice/workloads.slice/$caller_alloc.scope"
  callee_scope="/overdrive.slice/workloads.slice/$callee_alloc.scope"
  host_snapshot baseline-after-prepare
  run_iteration 1 2.25
  run_iteration 2 2.75
  run_iteration 3 3.25
  host_snapshot final-before-marker-cleanup
  stamp "REMOTE_PROBE_RESULT repetitions=3 all_durable_alloc_terminal_reason_stopped=yes all_public_stream_operator_stopped=yes all_exact_scopes_absent=yes all_allocation_network_absent=yes"
  stamp "REMOTE_PROBE_END"
}

# Recover only the marker/token-bound partial materialization left by this
# probe's pre-serve acknowledgement failure. This deliberately refuses any
# other shape and signals only the start-time/PGID/cmdline-proven private
# session wrapper recorded by the checked-in lifecycle protocol.
remote_recover_prior() {
  set -euo pipefail
  local repo_root example_dir prepare output_root marker marker_value token
  local run_root ready ack serve_pid_file ready_token pid pgid start extra=""
  local line rest state observed_pgid observed_start cmdline deadline
  repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
  example_dir="$repo_root/examples/guest-stack-transparent-mtls-intercept"
  prepare="$example_dir/prepare.sh"
  output_root=/srv/vm/overdrive-testing/gti-e07
  marker="$output_root/.gti-e07-owned"
  run_root="$output_root/probe-run-1"
  ready="$run_root/session-wrapper.ready"
  ack="$run_root/session-wrapper.ack"
  serve_pid_file="$run_root/serve.pid"
  printf 'RECOVERY timestamp_utc=%s begin\n' "$(date -u +'%Y-%m-%dT%H:%M:%S.%NZ')"
  [[ -e "$output_root" ]] || { printf 'RECOVERY nothing_to_do\n'; return 0; }
  [[ -f "$marker" ]] || { printf 'RECOVERY refuse missing_marker=%s\n' "$marker" >&2; return 1; }
  marker_value="$(<"$marker")"
  [[ "$marker_value" =~ ^gti-e07-owned-v1:([A-Fa-f0-9-]+)$ ]] \
    || { printf 'RECOVERY refuse unknown_marker=%s\n' "$marker_value" >&2; return 1; }
  token=${BASH_REMATCH[1]}
  [[ -d "$run_root" && ! -e "$ack" && ! -e "$serve_pid_file" ]] \
    || { printf 'RECOVERY refuse unexpected_partial_shape run=%s ready=%s ack=%s serve_pid=%s\n' \
      "$([[ -d "$run_root" ]] && echo yes || echo no)" \
      "$([[ -s "$ready" ]] && echo yes || echo no)" \
      "$([[ -e "$ack" ]] && echo yes || echo no)" \
      "$([[ -e "$serve_pid_file" ]] && echo yes || echo no)" >&2; return 1; }
  if [[ ! -s "$ready" ]]; then
    local process_path process_cmd live_wrapper=0
    [[ -f "$run_root/serve.log" \
      && "$(<"$run_root/serve.log")" == 'gti-e07 session wrapper: ownership acknowledgement timed out' ]] \
      || { printf 'RECOVERY refuse no_ready_without_exact_timeout_log\n' >&2; return 1; }
    for process_path in /proc/[0-9]*; do
      [[ -r "$process_path/cmdline" ]] || continue
      process_cmd="$(tr '\0' ' ' <"$process_path/cmdline")"
      if [[ "$process_cmd" == *"$example_dir/session-wrapper.sh"* \
        && "$process_cmd" == *"$run_root"* ]]; then
        printf 'RECOVERY refuse live_unrecorded_wrapper pid=%s cmdline=%s\n' \
          "${process_path##*/}" "$process_cmd" >&2
        live_wrapper=1
      fi
    done
    [[ "$live_wrapper" -eq 0 ]] || return 1
    printf 'RECOVERY exact_timeout_log=yes live_session_wrapper=absent\n'
    timeout --foreground --signal=TERM --kill-after=5s 45s \
      env GTI_E07_OWNERSHIP_TOKEN="$token" "$prepare" cleanup
    [[ ! -e "$output_root" ]] \
      || { printf 'RECOVERY cleanup_failed path=%s\n' "$output_root" >&2; return 1; }
    printf 'RECOVERY timestamp_utc=%s marker_owned_output=absent end\n' \
      "$(date -u +'%Y-%m-%dT%H:%M:%S.%NZ')"
    return 0
  fi
  read -r ready_token pid pgid start extra <"$ready"
  [[ -z "$extra" && "$ready_token" == "$token-iteration-1" \
    && "$pid" =~ ^[0-9]+$ && "$pgid" == "$pid" && "$start" =~ ^[0-9]+$ ]] \
    || { printf 'RECOVERY refuse invalid_ready_record\n' >&2; return 1; }
  if [[ -r "/proc/$pid/stat" ]]; then
    IFS= read -r line <"/proc/$pid/stat"
    rest="${line##*) }"
    # shellcheck disable=SC2086
    set -- $rest
    state=${1:0:1}; observed_pgid=$3; observed_start=${20}
    cmdline="$(tr '\0' ' ' <"/proc/$pid/cmdline")"
    [[ "$state" != Z && "$observed_pgid" == "$pgid" && "$observed_start" == "$start" \
      && "$cmdline" == *"$example_dir/session-wrapper.sh"* \
      && "$cmdline" == *"$ready"* && "$cmdline" == *"$ack"* ]] \
      || { printf 'RECOVERY refuse live_identity_mismatch pid=%s\n' "$pid" >&2; return 1; }
    printf 'RECOVERY signal=SIGTERM exact_private_pgid=%s pid=%s start=%s cmdline=%s\n' \
      "$pgid" "$pid" "$start" "$cmdline"
    kill -TERM -- "-$pgid"
    deadline=$((SECONDS + 5))
    while (( SECONDS < deadline )) && kill -0 -- "-$pgid" 2>/dev/null; do sleep 0.1; done
    if kill -0 -- "-$pgid" 2>/dev/null; then
      printf 'RECOVERY signal=SIGKILL exact_private_pgid=%s pid=%s start=%s\n' "$pgid" "$pid" "$start"
      kill -KILL -- "-$pgid"
    fi
  else
    printf 'RECOVERY recorded_wrapper_already_absent pid=%s start=%s\n' "$pid" "$start"
  fi
  timeout --foreground --signal=TERM --kill-after=5s 45s \
    env GTI_E07_OWNERSHIP_TOKEN="$token" "$prepare" cleanup
  [[ ! -e "$output_root" ]] || { printf 'RECOVERY cleanup_failed path=%s\n' "$output_root" >&2; return 1; }
  printf 'RECOVERY timestamp_utc=%s marker_owned_output=absent end\n' \
    "$(date -u +'%Y-%m-%dT%H:%M:%S.%NZ')"
}

case "${1:-}" in
  init)
    init_transcript
    ;;
  run)
    shift
    if [[ $# -ne 2 ]]; then
      printf 'usage: %s run <step> <shell-command>\n' "$0" >&2
      exit 64
    fi
    run_logged "$1" "$2"
    ;;
  inspect)
    shift
    if [[ $# -ne 1 ]]; then
      printf 'usage: %s inspect <read-only-shell-command>\n' "$0" >&2
      exit 64
    fi
    # Repository/source inspection is deliberately not retained as probe
    # evidence. The transcript is reserved for executed product/fault results.
    (cd "$WORKSPACE" && bash -lc "$1")
    ;;
  note)
    shift
    if [[ $# -lt 2 ]]; then
      printf 'usage: %s note <kind> <text...>\n' "$0" >&2
      exit 64
    fi
    note "$@"
    ;;
  remote)
    remote_probe
    ;;
  remote-recover-prior)
    remote_recover_prior
    ;;
  *)
    printf 'usage: %s {init|run <step> <shell-command>|inspect <read-only-shell-command>|note <kind> <text...>|remote|remote-recover-prior}\n' "$0" >&2
    exit 64
    ;;
esac

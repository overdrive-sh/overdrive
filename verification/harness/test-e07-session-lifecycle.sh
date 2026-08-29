#!/usr/bin/env bash
# Host-safe fault injection for the E07 wrapper/serve lifecycle. Fixtures use
# only private temporary files and private process groups; no keyring, product
# binary, KVM, network, or shared machine state is touched.
set -euo pipefail

SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
readonly SCRIPT_PATH
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly LIFECYCLE="$REPO_ROOT/examples/guest-stack-transparent-mtls-intercept/session-lifecycle.sh"

fail() {
  echo "E07 session lifecycle test: $*" >&2
  exit 1
}

process_group_and_start() {
  SESSION_READY_FILE=""
  SESSION_LAUNCH_TOKEN=""
  # shellcheck source=SCRIPTDIR/../../examples/guest-stack-transparent-mtls-intercept/session-lifecycle.sh
  source "$LIFECYCLE"
  local identity state pgid start
  identity="$(e07_session_process_identity "$$")"
  read -r state pgid start <<<"$identity"
  printf '%s %s\n' "$pgid" "$start"
}

publish_record() {
  local path="$1"
  local token="$2"
  local pgid="$3"
  local start="$4"
  printf '%s %s %s %s\n' "$token" "$$" "$pgid" "$start" >"$path.tmp.$$"
  mv -- "$path.tmp.$$" "$path"
}

fixture() {
  local mode="$1"
  local token="$2"
  local ready_file="$3"
  local ack_file="$4"
  local pid_file="$5"
  local marker="$6"
  local pgid start
  read -r pgid start < <(process_group_and_start)
  [[ "$pgid" == "$$" ]] || fail "fixture did not enter a private process group"
  : >"$marker.entered"

  if [[ "$mode" != "sentinel" ]]; then
    local ack_token ack_pid ack_pgid ack_start ack_extra=""
    while [[ ! -s "$ack_file" ]]; do :; done
    read -r ack_token ack_pid ack_pgid ack_start ack_extra <"$ack_file"
    [[ -z "$ack_extra" && "$ack_token" == "$token" && "$ack_pid" == "$$" \
      && "$ack_pgid" == "$pgid" && "$ack_start" == "$start" ]] \
      || fail "$mode: invalid parent acknowledgement"
    publish_record "$ready_file" "$token" "$pgid" "$start"
  fi

  case "$mode" in
    signal-before-group|signal-before-ready|signal-before-pid|pid-handshake-timeout|sentinel)
      ;;
    signal-after-pid-before-exec)
      publish_record "$pid_file" "$token" "$pgid" "$start"
      ;;
    normal-term)
      trap 'printf "TERM\n" >"$marker"; exit 0' TERM
      ;;
    term-to-kill)
      trap 'printf "TERM\n" >"$marker"' TERM
      ;;
    *) fail "unknown fixture mode: $mode" ;;
  esac
  : >"$marker.phase"

  # A builtin-only loop gives the pre-publication fixture no descendant. That
  # mirrors session-wrapper.sh, which publishes ownership before spawning or
  # execing keyctl, and makes direct-child termination an honest fault test.
  while :; do :; done
}

send_test_signal() {
  local signal_seen=0
  trap 'signal_seen=1' TERM
  kill -TERM "$$"
  trap - TERM
  [[ "$signal_seen" -eq 1 ]] || fail "injected TERM was not observed"
}

run_case() {
  local mode="$1"
  local case_root token ready_file ack_file pid_file marker
  case_root="$(mktemp -d "${TMPDIR:-/tmp}/e07-session-${mode}.XXXXXX")"
  token="token-$mode-$$"
  ready_file="$case_root/wrapper.ready"
  ack_file="$case_root/wrapper.ack"
  pid_file="$case_root/serve.pid"
  marker="$case_root/signal.marker"

  SESSION_WRAPPER_PID=""
  SESSION_WRAPPER_START=""
  SESSION_WRAPPER_PGID=""
  SESSION_GROUP_OWNED=0
  SESSION_WRAPPER_REAPED=0
  SESSION_READY_FILE="$ready_file"
  SESSION_ACK_FILE="$ack_file"
  SESSION_LAUNCH_TOKEN="$token"
  E07_SESSION_TERM_TICKS=3
  E07_SESSION_KILL_TICKS=20
  E07_SESSION_POLL_INTERVAL=0.02
  # shellcheck source=SCRIPTDIR/../../examples/guest-stack-transparent-mtls-intercept/session-lifecycle.sh
  source "$LIFECYCLE"

  local -a session_launcher
  if command -v setsid >/dev/null 2>&1; then
    session_launcher=(setsid)
  else
    command -v python3 >/dev/null 2>&1 || fail "setsid and python3 are unavailable"
    session_launcher=(python3 -c \
      'import os, sys; os.setsid(); os.execv(sys.argv[1], sys.argv[1:])')
  fi

  local wrapper_pid sentinel_pid wrapper_pgid=""
  if [[ "$mode" == "signal-before-group" ]]; then
    command -v python3 >/dev/null 2>&1 || fail "python3 is unavailable"
    python3 -c \
      'import os, sys, time; time.sleep(1); os.setsid(); os.execv(sys.argv[1], sys.argv[1:])' \
      "$SCRIPT_PATH" __fixture "$mode" "$token" "$ready_file" \
      "$ack_file" "$pid_file" "$marker" &
  else
    "${session_launcher[@]}" "$SCRIPT_PATH" __fixture "$mode" "$token" "$ready_file" \
      "$ack_file" "$pid_file" "$marker" &
  fi
  wrapper_pid=$!
  SESSION_WRAPPER_PID="$wrapper_pid"
  e07_session_capture_wrapper "$wrapper_pid" \
    || fail "$mode: could not capture the direct wrapper child"

  "${session_launcher[@]}" "$SCRIPT_PATH" __fixture sentinel sentinel-token \
    "$case_root/sentinel.ready" "$case_root/sentinel.ack" "$case_root/sentinel.pid" \
    "$case_root/sentinel.marker" &
  sentinel_pid=$!

  cleanup_case() {
    trap - EXIT HUP INT TERM
    if [[ -n "$SESSION_WRAPPER_PID" ]]; then
      e07_session_terminate_owned_unit >/dev/null 2>&1 || true
    fi
    kill -KILL "$sentinel_pid" 2>/dev/null || true
    wait "$sentinel_pid" 2>/dev/null || true
    rm -rf -- "$case_root"
  }
  trap cleanup_case EXIT HUP INT TERM

  if [[ "$mode" != "signal-before-group" ]]; then
    local entered_ticks=20
    while [[ ! -e "$marker.entered" && "$entered_ticks" -gt 0 ]]; do
      e07_session_unit_is_live || fail "$mode: wrapper exited before fixture entry"
      sleep 0.02
      entered_ticks=$((entered_ticks - 1))
    done
    [[ -e "$marker.entered" ]] || fail "$mode: fixture entry timed out"
  fi

  case "$mode" in
    signal-before-group)
      send_test_signal
      [[ "$SESSION_GROUP_OWNED" -eq 0 ]] \
        || fail "$mode: group was unexpectedly adopted"
      [[ ! -e "$ready_file" ]] || fail "$mode: fixture published readiness"
      ;;
    signal-before-ready)
      send_test_signal
      [[ ! -e "$ready_file" ]] || fail "$mode: fixture published readiness"
      ;;
    signal-before-pid)
      e07_session_wait_for_private_group 20 || fail "$mode: group adoption failed"
      e07_session_acknowledge_ownership || fail "$mode: acknowledgement failed"
      e07_session_wait_for_ready 20 || fail "$mode: ready handshake failed"
      send_test_signal
      [[ ! -e "$pid_file" ]] || fail "$mode: fixture published serve PID"
      ;;
    pid-handshake-timeout)
      e07_session_wait_for_private_group 20 || fail "$mode: group adoption failed"
      e07_session_acknowledge_ownership || fail "$mode: acknowledgement failed"
      e07_session_wait_for_ready 20 || fail "$mode: ready handshake failed"
      local phase_ticks=20
      while [[ ! -e "$marker.phase" && "$phase_ticks" -gt 0 ]]; do
        sleep 0.02
        phase_ticks=$((phase_ticks - 1))
      done
      [[ -e "$marker.phase" ]] || fail "$mode: PID-timeout phase was not entered"
      local ticks=2
      while [[ ! -s "$pid_file" && "$ticks" -gt 0 ]]; do
        e07_session_unit_is_live || fail "$mode: wrapper exited during handshake"
        sleep 0.02
        ticks=$((ticks - 1))
      done
      [[ ! -s "$pid_file" ]] || fail "$mode: expected PID handshake timeout"
      e07_session_unit_is_live || fail "$mode: wrapper not live at timeout"
      ;;
    signal-after-pid-before-exec)
      e07_session_wait_for_private_group 20 || fail "$mode: group adoption failed"
      e07_session_acknowledge_ownership || fail "$mode: acknowledgement failed"
      e07_session_wait_for_ready 20 || fail "$mode: ready handshake failed"
      local pid_ticks=20
      while [[ ! -s "$pid_file" && "$pid_ticks" -gt 0 ]]; do
        sleep 0.02
        pid_ticks=$((pid_ticks - 1))
      done
      [[ -s "$pid_file" ]] || fail "$mode: serve PID was not published"
      local published_token published_pid published_pgid published_start
      read -r published_token published_pid published_pgid published_start <"$pid_file"
      [[ "$published_token" == "$token" && "$published_pid" == "$wrapper_pid" \
        && "$published_pgid" == "$SESSION_WRAPPER_PGID" \
        && "$published_start" == "$SESSION_WRAPPER_START" ]] \
        || fail "$mode: published pre-exec identity is invalid"
      send_test_signal
      ;;
    normal-term|term-to-kill)
      e07_session_wait_for_private_group 20 || fail "$mode: group adoption failed"
      e07_session_acknowledge_ownership || fail "$mode: acknowledgement failed"
      e07_session_wait_for_ready 20 || fail "$mode: ready handshake failed"
      local phase_ticks=20
      while [[ ! -e "$marker.phase" && "$phase_ticks" -gt 0 ]]; do
        sleep 0.02
        phase_ticks=$((phase_ticks - 1))
      done
      [[ -e "$marker.phase" ]] || fail "$mode: signal handler was not armed"
      ;;
  esac

  if [[ "$SESSION_GROUP_OWNED" -eq 1 ]]; then
    wrapper_pgid="$SESSION_WRAPPER_PGID"
  fi
  e07_session_terminate_owned_unit \
    || fail "$mode: owned launch unit did not terminate within bounds"
  [[ "$SESSION_WRAPPER_REAPED" -eq 1 ]] \
    || fail "$mode: direct wrapper child was not reaped"
  ! e07_session_process_entry_exists "$wrapper_pid" \
    || fail "$mode: wrapper process remains after termination"
  if [[ -n "$wrapper_pgid" ]]; then
    ! kill -0 -- "-$wrapper_pgid" 2>/dev/null \
      || fail "$mode: private process group remains live"
  fi
  kill -0 "$sentinel_pid" 2>/dev/null \
    || fail "$mode: unrelated sentinel was signalled"

  case "$mode" in
    normal-term|term-to-kill)
      [[ "$(<"$marker")" == "TERM" ]] \
        || fail "$mode: TERM handler evidence is absent"
      ;;
  esac

  SESSION_WRAPPER_PID=""
  cleanup_case
  trap - EXIT HUP INT TERM
}

case "${1:-}" in
  __fixture)
    shift
    fixture "$@"
    ;;
  __case)
    run_case "$2"
    ;;
  "")
    command -v timeout >/dev/null 2>&1 || fail "timeout is unavailable"
    for mode in signal-before-group signal-before-ready signal-before-pid pid-handshake-timeout \
      signal-after-pid-before-exec normal-term term-to-kill; do
      timeout --foreground --signal=TERM --kill-after=1s 5s \
        "$SCRIPT_PATH" __case "$mode" \
        || fail "$mode exceeded its host-safe bound or failed"
    done
    echo "E07 session lifecycle fault tests passed"
    ;;
  *) fail "usage: $0" ;;
esac

#!/usr/bin/env bash
# Process-lifecycle helpers for the E07 fresh-keyring wrapper. The caller owns
# one direct child started with `setsid`; this library records its Linux start
# time and only signals that exact child or its proven private process group.

: "${E07_SESSION_TERM_TICKS:=50}"
: "${E07_SESSION_KILL_TICKS:=50}"
: "${E07_SESSION_POLL_INTERVAL:=0.1}"

SESSION_WRAPPER_PID="${SESSION_WRAPPER_PID:-}"
SESSION_WRAPPER_START="${SESSION_WRAPPER_START:-}"
SESSION_WRAPPER_PGID="${SESSION_WRAPPER_PGID:-}"
SESSION_GROUP_OWNED="${SESSION_GROUP_OWNED:-0}"
SESSION_WRAPPER_REAPED="${SESSION_WRAPPER_REAPED:-0}"
: "${SESSION_READY_FILE:=}"
: "${SESSION_ACK_FILE:=}"
SESSION_LAUNCH_TOKEN="${SESSION_LAUNCH_TOKEN:-}"

e07_session_process_identity() {
  local pid="$1"
  local line rest
  [[ "$pid" =~ ^[0-9]+$ ]] || return 1
  if [[ -r "/proc/$pid/stat" ]]; then
    IFS= read -r line <"/proc/$pid/stat" || return 1
    rest="${line##*) }"
    # The suffix begins at proc(5) field 3: state, ppid, pgrp, ... starttime.
    # shellcheck disable=SC2086 # intentional procfs field splitting
    set -- $rest
    [[ "$#" -ge 20 ]] || return 1
    printf '%s %s %s\n' "${1:0:1}" "$3" "${20}"
    return
  fi

  # Darwin has no procfs. This fallback exists only so the lifecycle fault
  # test can exercise real signals/process groups on the development host;
  # the native-metal product path is Linux-only and always uses proc starttime.
  [[ "$(uname -s)" == "Darwin" ]] || return 1
  local observed_pid state pgid weekday month day clock year extra="" start
  line="$(ps -o pid=,state=,pgid=,lstart= -p "$pid" 2>/dev/null)" || return 1
  read -r observed_pid state pgid weekday month day clock year extra <<<"$line"
  [[ -z "$extra" && "$observed_pid" == "$pid" && "$pgid" =~ ^[0-9]+$ \
    && -n "$year" ]] || return 1
  start="$(printf '%s' "$weekday $month $day $clock $year" | cksum)"
  start="${start%% *}"
  printf '%s %s %s\n' "${state:0:1}" "$pgid" "$start"
}

e07_session_process_entry_exists() {
  local pid="$1"
  if [[ -d /proc ]]; then
    [[ -e "/proc/$pid" ]]
  else
    ps -p "$pid" >/dev/null 2>&1
  fi
}

e07_session_capture_wrapper() {
  local pid="$1"
  local identity state pgid start
  SESSION_WRAPPER_PID="$pid"
  SESSION_WRAPPER_REAPED=0
  identity="$(e07_session_process_identity "$pid")" || return 1
  read -r state pgid start <<<"$identity"
  [[ "$state" != "Z" ]] || return 1
  SESSION_WRAPPER_START="$start"
  return 0
}

e07_session_wrapper_matches() {
  local identity state pgid start
  [[ -n "$SESSION_WRAPPER_PID" && -n "$SESSION_WRAPPER_START" ]] || return 1
  identity="$(e07_session_process_identity "$SESSION_WRAPPER_PID")" || return 1
  read -r state pgid start <<<"$identity"
  [[ "$start" == "$SESSION_WRAPPER_START" ]]
}

e07_session_adopt_ready_group() {
  local token pid pgid start extra=""
  [[ -n "$SESSION_READY_FILE" && -s "$SESSION_READY_FILE" ]] || return 1
  read -r token pid pgid start extra <"$SESSION_READY_FILE" || return 1
  [[ -z "$extra" \
    && "$token" == "$SESSION_LAUNCH_TOKEN" \
    && "$pid" == "$SESSION_WRAPPER_PID" \
    && "$pgid" == "$pid" \
    && "$start" =~ ^[0-9]+$ ]] || return 1
  if [[ -n "$SESSION_WRAPPER_START" ]]; then
    [[ "$start" == "$SESSION_WRAPPER_START" ]] || return 1
  else
    SESSION_WRAPPER_START="$start"
  fi
  SESSION_WRAPPER_PGID="$pgid"
  SESSION_GROUP_OWNED=1
}

e07_session_adopt_live_group() {
  local identity state pgid start
  [[ -n "$SESSION_WRAPPER_START" ]] || return 1
  identity="$(e07_session_process_identity "$SESSION_WRAPPER_PID")" || return 1
  read -r state pgid start <<<"$identity"
  [[ "$start" == "$SESSION_WRAPPER_START" && "$pgid" == "$SESSION_WRAPPER_PID" ]] \
    || return 1
  SESSION_WRAPPER_PGID="$pgid"
  SESSION_GROUP_OWNED=1
}

e07_session_reap_wrapper_if_exited() {
  local identity state pgid start
  [[ -n "$SESSION_WRAPPER_PID" && "$SESSION_WRAPPER_REAPED" -eq 0 ]] || return 0
  if identity="$(e07_session_process_identity "$SESSION_WRAPPER_PID")"; then
    read -r state pgid start <<<"$identity"
    if [[ -n "$SESSION_WRAPPER_START" && "$start" != "$SESSION_WRAPPER_START" ]]; then
      # The original direct child is gone and its PID was reused. Shell wait
      # addresses the original child record, never the unrelated new process.
      wait "$SESSION_WRAPPER_PID" 2>/dev/null || true
      SESSION_WRAPPER_REAPED=1
      return 0
    fi
    [[ "$state" == "Z" ]] || return 1
  elif e07_session_process_entry_exists "$SESSION_WRAPPER_PID"; then
    # An unreadable or malformed live proc entry is not exit proof. Refuse the
    # otherwise potentially blocking wait and let the bounded caller fail.
    return 1
  fi
  # Process-table absence or Z proves the direct child has exited, so wait
  # cannot block. This is the only location that reaps the wrapper.
  wait "$SESSION_WRAPPER_PID" 2>/dev/null || true
  SESSION_WRAPPER_REAPED=1
  return 0
}

e07_session_group_is_live() {
  [[ "$SESSION_GROUP_OWNED" -eq 1 && -n "$SESSION_WRAPPER_PGID" ]] || return 1
  kill -0 -- "-$SESSION_WRAPPER_PGID" 2>/dev/null
}

e07_session_unit_is_live() {
  e07_session_reap_wrapper_if_exited || true
  if [[ "$SESSION_GROUP_OWNED" -eq 1 ]]; then
    e07_session_group_is_live
    return
  fi
  e07_session_wrapper_matches || return 1
  local identity state pgid start
  identity="$(e07_session_process_identity "$SESSION_WRAPPER_PID")" || return 1
  read -r state pgid start <<<"$identity"
  [[ "$state" != "Z" ]]
}

e07_session_signal_owned_unit() {
  local signal="$1"
  if [[ "$SESSION_GROUP_OWNED" -eq 1 ]]; then
    e07_session_group_is_live || return 0
    kill -"$signal" -- "-$SESSION_WRAPPER_PGID" 2>/dev/null
    return
  fi
  e07_session_wrapper_matches || return 0
  kill -"$signal" "$SESSION_WRAPPER_PID" 2>/dev/null
}

e07_session_poll_exit() {
  local ticks="$1"
  while [[ "$ticks" -gt 0 ]]; do
    e07_session_unit_is_live || return 0
    sleep "$E07_SESSION_POLL_INTERVAL"
    ticks=$((ticks - 1))
  done
  ! e07_session_unit_is_live
}

e07_session_terminate_owned_unit() {
  [[ -n "$SESSION_WRAPPER_PID" ]] || return 0

  # Cleanup may run while the parent is between wrapper launch and group
  # adoption. Prefer the token-bound record, then the exact live setsid leader.
  # Before either proof exists the gated wrapper has no descendant, so its
  # start-time-verified direct PID remains the complete termination unit.
  if [[ "$SESSION_GROUP_OWNED" -ne 1 ]]; then
    if [[ -s "$SESSION_READY_FILE" ]]; then
      e07_session_adopt_ready_group || e07_session_adopt_live_group || true
    else
      e07_session_adopt_live_group || true
    fi
  fi

  if ! e07_session_unit_is_live; then
    e07_session_reap_wrapper_if_exited || return 1
    return 0
  fi

  e07_session_signal_owned_unit TERM || true
  if ! e07_session_poll_exit "$E07_SESSION_TERM_TICKS"; then
    e07_session_signal_owned_unit KILL || true
    e07_session_poll_exit "$E07_SESSION_KILL_TICKS" || return 1
  fi
  e07_session_reap_wrapper_if_exited || return 1
  ! e07_session_unit_is_live
}

e07_session_wait_for_ready() {
  local ticks="$1"
  while [[ "$ticks" -gt 0 ]]; do
    if [[ -s "$SESSION_READY_FILE" ]]; then
      e07_session_adopt_ready_group
      return
    fi
    e07_session_unit_is_live || return 1
    sleep "$E07_SESSION_POLL_INTERVAL"
    ticks=$((ticks - 1))
  done
  return 1
}

e07_session_wait_for_private_group() {
  local ticks="$1"
  while [[ "$ticks" -gt 0 ]]; do
    e07_session_adopt_live_group && return 0
    e07_session_unit_is_live || return 1
    sleep "$E07_SESSION_POLL_INTERVAL"
    ticks=$((ticks - 1))
  done
  return 1
}

e07_session_acknowledge_ownership() {
  [[ "$SESSION_GROUP_OWNED" -eq 1 && -n "$SESSION_ACK_FILE" ]] || return 1
  printf '%s %s %s %s\n' "$SESSION_LAUNCH_TOKEN" "$SESSION_WRAPPER_PID" \
    "$SESSION_WRAPPER_PGID" "$SESSION_WRAPPER_START" \
    >"$SESSION_ACK_FILE.tmp.$$" || return 1
  mv -- "$SESSION_ACK_FILE.tmp.$$" "$SESSION_ACK_FILE"
}

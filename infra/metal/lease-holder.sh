#!/usr/bin/env bash
# Canonical remote lease holder for every supported bare-metal writer.
#
# The ordinary positional mode at the bottom of this file preserves the
# bootstrap/sync/shell lease protocol.  `--durable` is the private service
# entrypoint used by `bootstrap.sh --run`; `--collect` is its read-only record
# reader.  Keeping both paths in this file makes the canonical flock and
# token-matched owner cleanup one implementation.
set -euo pipefail

RUNS_ROOT_DEFAULT="/var/lib/overdrive-metal/runs"

utc_now() {
  date -u +%Y-%m-%dT%H:%M:%SZ
}

valid_run_id() {
  [[ "$1" =~ ^[0-9a-f]{24}$ ]]
}

base64_encode() {
  printf '%s' "$1" | base64 | tr -d '\n'
}

is_rfc3339_utc() {
  [[ "$1" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]]
}

# Flush the temporary file before rename and the containing directory after
# rename.  GNU coreutils provides `sync -d`; the filesystem-wide fallback is
# retained for distributions whose `sync` lacks that option.
sync_path() {
  local path="$1"
  sync -d "$path" 2>/dev/null || sync -f "$path"
}

atomic_write() {
  local path="$1"
  local content="$2"
  local directory
  local temporary
  directory="$(dirname "$path")"
  temporary="${path}.tmp.$$.$RANDOM"
  umask 077
  printf '%s' "$content" >"$temporary"
  sync_path "$temporary"
  mv -f "$temporary" "$path"
  sync_path "$directory"
}

record_has_final_newline() {
  local path="$1"
  local size
  [ -f "$path" ] || return 1
  size="$(wc -c <"$path" | tr -d '[:space:]')"
  [ "$size" -gt 0 ] || return 1
  [ "$(od -An -t x1 -j $((size - 1)) -N 1 "$path" | tr -d '[:space:]')" = 0a ]
}

ordered_record_shape() {
  local path="$1"
  shift
  local -a expected=("$@")
  local line key count index
  [ -f "$path" ] || return 1
  record_has_final_newline "$path" || return 1
  count="$(wc -l <"$path" | tr -d '[:space:]')"
  [ "$count" -eq "${#expected[@]}" ] || return 1
  for ((index = 0; index < ${#expected[@]}; index++)); do
    line="$(sed -n "$((index + 1))p" "$path")"
    [[ "$line" == *=* ]] || return 1
    key="${line%%=*}"
    [ "$key" = "${expected[index]}" ] || return 1
  done
}

record_value() {
  local path="$1"
  local line_number="$2"
  sed -n "${line_number}p" "$path" | cut -d= -f2-
}

canonical_base64() {
  local value="$1"
  local temporary canonical
  temporary="$(mktemp)"
  if ! printf '%s' "$value" | base64 -d >"$temporary" 2>/dev/null; then
    rm -f "$temporary"
    return 1
  fi
  # Bash cannot hold NUL bytes in a variable.  Inspect the decoded bytes
  # before comparing the re-encoded canonical representation instead.
  if od -An -v -t x1 "$temporary" | tr ' ' '\n' | grep -qx 00; then
    rm -f "$temporary"
    return 1
  fi
  canonical="$(base64_encode "$(<"$temporary")")"
  rm -f "$temporary"
  [ "$canonical" = "$value" ]
}

metadata_valid() {
  local path="$1"
  local run_id="$2"
  local unit="$3"
  local value
  ordered_record_shape "$path" \
    format run_id token unit created_at scenario_b64 workspace_b64 commit \
    source_digest remote_dir_b64 target_user_b64 execution_mode || return 1
  [ "$(record_value "$path" 1)" = overdrive-metal-run-v1 ] || return 1
  [ "$(record_value "$path" 2)" = "$run_id" ] || return 1
  [ "$(record_value "$path" 3)" = "$run_id" ] || return 1
  [ "$(record_value "$path" 4)" = "$unit" ] || return 1
  value="$(record_value "$path" 5)"
  is_rfc3339_utc "$value" || return 1
  canonical_base64 "$(record_value "$path" 6)" || return 1
  canonical_base64 "$(record_value "$path" 7)" || return 1
  value="$(record_value "$path" 8)"
  [[ "$value" =~ ^([0-9a-f]{40}|[0-9a-f]{64})$ ]] || return 1
  value="$(record_value "$path" 9)"
  [[ "$value" =~ ^[0-9a-f]{64}$ ]] || return 1
  canonical_base64 "$(record_value "$path" 10)" || return 1
  canonical_base64 "$(record_value "$path" 11)" || return 1
  value="$(record_value "$path" 12)"
  [ "$value" = privileged ] || [ "$value" = login ] || return 1
}

phase_valid() {
  local path="$1"
  local run_id="$2"
  local value
  ordered_record_shape "$path" format run_id phase updated_at || return 1
  [ "$(record_value "$path" 1)" = overdrive-metal-phase-v1 ] || return 1
  [ "$(record_value "$path" 2)" = "$run_id" ] || return 1
  value="$(record_value "$path" 3)"
  [ "$value" = waiting_for_lease ] || [ "$value" = preparing ] || \
    [ "$value" = running ] || return 1
  is_rfc3339_utc "$(record_value "$path" 4)"
}

TERMINAL_KIND=""
TERMINAL_EXIT_CODE=""
TERMINAL_CAUSE=""
TERMINAL_ERROR=""
COLLECT_TIMEOUT_SECONDS=""
COLLECT_LOCK_PATH=""
COLLECT_OWNER_PATH=""

terminal_valid() {
  local path="$1"
  local run_id="$2"
  local value
  TERMINAL_KIND=""
  TERMINAL_EXIT_CODE=""
  TERMINAL_CAUSE=""
  TERMINAL_ERROR=""
  ordered_record_shape "$path" format run_id kind exit_code cause finished_at || \
    ordered_record_shape "$path" format run_id kind error finished_at || return 1
  [ "$(record_value "$path" 1)" = overdrive-metal-terminal-v1 ] || return 1
  [ "$(record_value "$path" 2)" = "$run_id" ] || return 1
  value="$(record_value "$path" 3)"
  if [ "$value" = command ]; then
    [ "$(wc -l <"$path" | tr -d '[:space:]')" -eq 6 ] || return 1
    TERMINAL_KIND=command
    TERMINAL_EXIT_CODE="$(record_value "$path" 4)"
    [[ "$TERMINAL_EXIT_CODE" =~ ^(0|[1-9][0-9]*)$ ]] || return 1
    [ "$TERMINAL_EXIT_CODE" -le 255 ] || return 1
    TERMINAL_CAUSE="$(record_value "$path" 5)"
    [ "$TERMINAL_CAUSE" = command_returned ] || \
      [ "$TERMINAL_CAUSE" = supervisor_terminated ] || return 1
    is_rfc3339_utc "$(record_value "$path" 6)" || return 1
  elif [ "$value" = runner ]; then
    [ "$(wc -l <"$path" | tr -d '[:space:]')" -eq 5 ] || return 1
    TERMINAL_KIND=runner
    TERMINAL_ERROR="$(record_value "$path" 4)"
    case "$TERMINAL_ERROR" in
      lease_timeout|launch_aborted|controller_eof_before_start|\
      command_launch_failed|log_write_failed|log_finalize_failed) ;;
      *) return 1 ;;
    esac
    is_rfc3339_utc "$(record_value "$path" 5)" || return 1
  else
    return 1
  fi
}

write_phase() {
  local run_directory="$1"
  local run_id="$2"
  local phase="$3"
  local content
  content="format=overdrive-metal-phase-v1
run_id=${run_id}
phase=${phase}
updated_at=$(utc_now)
"
  atomic_write "${run_directory}/phase.v1" "$content"
}

write_terminal_command() {
  local run_directory="$1"
  local run_id="$2"
  local exit_code="$3"
  local cause="$4"
  local content
  content="format=overdrive-metal-terminal-v1
run_id=${run_id}
kind=command
exit_code=${exit_code}
cause=${cause}
finished_at=$(utc_now)
"
  atomic_write "${run_directory}/terminal.v1" "$content"
}

write_terminal_runner() {
  local run_directory="$1"
  local run_id="$2"
  local error="$3"
  local content
  content="format=overdrive-metal-terminal-v1
run_id=${run_id}
kind=runner
error=${error}
finished_at=$(utc_now)
"
  atomic_write "${run_directory}/terminal.v1" "$content"
}

unit_is_live() {
  local unit="$1"
  local load active
  load="$(systemctl show "$unit" --property=LoadState --value 2>/dev/null || true)"
  active="$(systemctl show "$unit" --property=ActiveState --value 2>/dev/null || true)"
  [ "$load" = loaded ] || return 1
  [ "$active" = activating ] || [ "$active" = active ] || \
    [ "$active" = deactivating ]
}

collector_marker() {
  local error="$1"
  printf 'OVERDRIVE_METAL_RUNNER_ERROR error=%s\n' "$error" >&2
}

collector_artifact_error() {
  collector_marker run_artifact_io
  return 74
}

collector_not_found() {
  collector_marker run_not_found
  return 66
}

collector_supervisor_lost() {
  collector_marker supervisor_lost
  return 70
}

collector_not_started() {
  collector_marker run_not_started
  return 75
}

emit_available() {
  local path="$1"
  local offset="$2"
  local size count
  [ -f "$path" ] || return 0
  size="$(wc -c <"$path" | tr -d '[:space:]')"
  [ "$size" -ge "$offset" ] || return 1
  count=$((size - offset))
  if [ "$count" -gt 0 ]; then
    dd if="$path" bs=1 skip="$offset" count="$count" status=none 2>/dev/null || return 1
  fi
  EMIT_SIZE="$size"
}

terminal_result() {
  local run_directory="$1"
  local run_id="$2"
  local offset="$3"
  local final_log="${run_directory}/output.log"
  local partial_log="${run_directory}/output.log.partial"
  local error
  if ! terminal_valid "${run_directory}/terminal.v1" "$run_id"; then
    collector_artifact_error
    return 74
  fi
  if [ "$TERMINAL_KIND" = command ]; then
    [ -f "$final_log" ] || {
      collector_artifact_error
      return 74
    }
    if ! emit_available "$final_log" "$offset"; then
      collector_artifact_error
      return 74
    fi
    printf 'OVERDRIVE_METAL_RUN_COMPLETE run_id=%s exit_code=%s cause=%s\n' \
      "$run_id" "$TERMINAL_EXIT_CODE" "$TERMINAL_CAUSE" >&2
    return "$TERMINAL_EXIT_CODE"
  fi
  if [ -f "$partial_log" ]; then
    if ! emit_available "$partial_log" "$offset"; then
      collector_artifact_error
      return 74
    fi
  fi
  error="$TERMINAL_ERROR"
  case "$error" in
    lease_timeout|launch_aborted|controller_eof_before_start|command_launch_failed)
      if [ "$error" = lease_timeout ] && [ -n "$COLLECT_TIMEOUT_SECONDS" ]; then
        printf 'FATAL: timed out after %ss acquiring %s\n' \
          "$COLLECT_TIMEOUT_SECONDS" "$COLLECT_LOCK_PATH" >&2
        if [ -r "$COLLECT_OWNER_PATH" ]; then
          printf 'current lease owner:\n' >&2
          sed 's/^/  /' "$COLLECT_OWNER_PATH" >&2
        fi
      fi
      collector_not_started
      return 75
      ;;
    log_write_failed|log_finalize_failed)
      collector_artifact_error
      return 74
      ;;
    *)
      collector_artifact_error
      return 74
      ;;
  esac
}

collect_run() {
  local run_id="$1"
  local runs_root="$2"
  local run_directory="${runs_root}/${run_id}"
  local unit="overdrive-metal-run-${run_id}.service"
  local metadata="${run_directory}/metadata.v1"
  local phase="${run_directory}/phase.v1"
  local terminal="${run_directory}/terminal.v1"
  local partial_log="${run_directory}/output.log.partial"
  local offset=0
  local phase_value
  local live_after

  if ! valid_run_id "$run_id"; then
    collector_marker invalid_run_id
    return 64
  fi

  while :; do
    if [ ! -d "$run_directory" ]; then
      if unit_is_live "$unit"; then
        sleep 0.1
        continue
      fi
      sleep 0.1
      if unit_is_live "$unit"; then
        continue
      fi
      collector_not_found
      return 66
    fi

    # The service publishes metadata and its initial phase atomically before
    # attempting the canonical flock.  A live unit with either file absent is
    # therefore a registration window; a dead unit is an artifact failure.
    if [ ! -f "$metadata" ] || [ ! -f "$phase" ]; then
      if unit_is_live "$unit"; then
        sleep 0.1
        continue
      fi
      collector_artifact_error
      return 74
    fi
    if ! metadata_valid "$metadata" "$run_id" "$unit"; then
      collector_artifact_error
      return 74
    fi

    if ! phase_valid "$phase" "$run_id"; then
      collector_artifact_error
      return 74
    fi
    phase_value="$(record_value "$phase" 3)"

    # Terminal publication does not supersede validation of the retained
    # phase record.  A malformed phase is a persisted-record contradiction and
    # must fail closed before a saved command status can be replayed.
    if [ -f "$terminal" ]; then
      terminal_result "$run_directory" "$run_id" "$offset"
      return $?
    fi

    if [ "$phase_value" = running ]; then
      while :; do
        if [ -f "$terminal" ]; then
          terminal_result "$run_directory" "$run_id" "$offset"
          return $?
        fi
        if ! phase_valid "$phase" "$run_id"; then
          collector_artifact_error
          return 74
        fi
        if [ -f "$partial_log" ]; then
          if ! emit_available "$partial_log" "$offset"; then
            collector_artifact_error
            return 74
          fi
          offset="$EMIT_SIZE"
        fi
        if ! unit_is_live "$unit"; then
          sleep 0.1
          if [ -f "$terminal" ]; then
            continue
          fi
          if unit_is_live "$unit"; then
            continue
          fi
          # A readable partial stream is still useful evidence, but an absent
          # terminal cannot honestly be promoted to a command result.
          collector_supervisor_lost
          return 70
        fi
        sleep 0.1
      done
    fi

    if ! unit_is_live "$unit"; then
      sleep 0.1
      if [ -f "$terminal" ]; then
        continue
      fi
      live_after=0
      unit_is_live "$unit" && live_after=1
      if [ "$live_after" -eq 0 ]; then
        collector_supervisor_lost
        return 70
      fi
    fi
    sleep 0.1
  done
}

durable_main() {
  [ "$#" -eq 15 ] || {
    printf 'durable lease holder: expected 15 arguments\n' >&2
    return 74
  }
  local lock_path="$1"
  owner_path="$2"
  local timeout_seconds="$3"
  token="$4"
  local scenario="$5"
  local workspace="$6"
  local commit="$7"
  local source_digest="$8"
  local remote_dir="$9"
  local target_user="${10}"
  local execution_mode="${11}"
  local remote_home="${12}"
  local remote_path="${13}"
  local command_string="${14}"
  local runs_root="${15}"
  local unit="overdrive-metal-run-${token}.service"
  local run_directory="${runs_root}/${token}"
  local metadata="${run_directory}/metadata.v1"
  local partial_log="${run_directory}/output.log.partial"
  local final_log="${run_directory}/output.log"
  output_fifo="${run_directory}/output.pipe"
  owner_tmp="${owner_path}.tmp.$$"
  local control=""
  local logger_pid=""
  local command_pid=""
  local command_status=0
  local logger_status=0
  local supervisor_terminated=0
  local running=0
  owner_written=0
  terminal_written=0
  local metadata_content

  valid_run_id "$token" || return 74
  [ "$execution_mode" = privileged ] || [ "$execution_mode" = login ] || return 74
  metadata_content="format=overdrive-metal-run-v1
run_id=${token}
token=${token}
unit=${unit}
created_at=$(utc_now)
scenario_b64=$(base64_encode "$scenario")
workspace_b64=$(base64_encode "$workspace")
commit=${commit}
source_digest=${source_digest}
remote_dir_b64=$(base64_encode "$remote_dir")
target_user_b64=$(base64_encode "$target_user")
execution_mode=${execution_mode}
"

  umask 077
  mkdir -p "$runs_root"
  chmod 700 "$runs_root"
  if ! mkdir "$run_directory"; then
    return 74
  fi
  chmod 700 "$run_directory"
  # The run directory and the two initial records are the sole writes before
  # the canonical flock is acquired.
  atomic_write "$metadata" "$metadata_content" || return 74
  write_phase "$run_directory" "$token" waiting_for_lease || return 74

  cleanup_durable() {
    if [ "$owner_written" -eq 1 ] && [ -r "$owner_path" ] && \
      grep -qx "token=${token}" "$owner_path"; then
      rm -f "$owner_path"
    fi
    rm -f "$owner_tmp" "$output_fifo"
  }
  handle_signal() {
    if [ "$running" -eq 0 ]; then
      if [ "$terminal_written" -eq 0 ]; then
        write_terminal_runner "$run_directory" "$token" launch_aborted || true
        terminal_written=1
      fi
      exit 75
    fi
    supervisor_terminated=1
    if [ -n "$command_pid" ]; then
      kill -TERM -- "-${command_pid}" 2>/dev/null || kill -TERM "$command_pid" 2>/dev/null || true
    fi
  }
  handle_pipe() {
    if [ "$running" -eq 0 ]; then
      if [ "$terminal_written" -eq 0 ]; then
        write_terminal_runner "$run_directory" "$token" controller_eof_before_start || true
        terminal_written=1
      fi
      exit 75
    fi
  }
  trap cleanup_durable EXIT
  trap handle_signal HUP INT TERM
  trap handle_pipe PIPE

  # Registration is visible before the service waits for the shared lease;
  # collectors can therefore distinguish a live queued run from an id that
  # was printed before any durable record existed.
  printf 'OVERDRIVE_METAL_RUN_REGISTERED run_id=%s\n' "$token"

  exec 8>"$lock_path"
  if ! flock -w "$timeout_seconds" 8; then
    write_terminal_runner "$run_directory" "$token" lease_timeout || true
    terminal_written=1
    return 75
  fi

  {
    printf 'pid=%s\n' "$$"
    printf 'started_at=%s\n' "$(utc_now)"
    printf 'action=run\n'
    printf 'scenario=%s\n' "$scenario"
    printf 'workspace=%s\n' "$workspace"
    printf 'commit=%s\n' "$commit"
    printf 'token=%s\n' "$token"
    printf 'run_id=%s\n' "$token"
    printf 'unit=%s\n' "$unit"
    printf 'source_digest=%s\n' "$source_digest"
  } >"$owner_tmp"
  sync_path "$owner_tmp"
  mv -f "$owner_tmp" "$owner_path"
  sync_path "$(dirname "$owner_path")"
  owner_written=1
  write_phase "$run_directory" "$token" preparing || {
    write_terminal_runner "$run_directory" "$token" log_finalize_failed || true
    terminal_written=1
    return 74
  }
  printf 'OVERDRIVE_METAL_LEASE_ACQUIRED token=%s pid=%s\n' "$token" "$$"

  if ! IFS= read -r control; then
    write_terminal_runner "$run_directory" "$token" controller_eof_before_start || true
    terminal_written=1
    return 75
  fi
  case "$control" in
    START) ;;
    ABORT)
      write_terminal_runner "$run_directory" "$token" launch_aborted || true
      terminal_written=1
      return 75
      ;;
    *)
      write_terminal_runner "$run_directory" "$token" launch_aborted || true
      terminal_written=1
      return 75
      ;;
  esac

  # From this point every descriptor inherited from systemd-run is detached
  # before any durable-running promise is published.
  trap '' PIPE
  exec 0</dev/null
  exec 1>/dev/null
  exec 2>&1

  if [ -z "$remote_path" ] && [ "$target_user" != root ]; then
    remote_path="$(sudo -n -u "$target_user" env "HOME=$remote_home" \
      bash -lc 'printf %s "$PATH"' 2>/dev/null || true)"
  fi

  # Sync normally creates this directory before START.  A minimal fake SSH
  # adapter may acknowledge that mkdir without materialising it; ensuring the
  # already-authorized checkout path exists keeps the command boundary honest
  # in that adapter without changing the real synced path.
  if [ ! -d "$remote_dir" ] && ! mkdir -p "$remote_dir"; then
    write_terminal_runner "$run_directory" "$token" command_launch_failed || true
    terminal_written=1
    return 75
  fi

  if ! mkfifo "$output_fifo"; then
    write_terminal_runner "$run_directory" "$token" command_launch_failed || true
    terminal_written=1
    return 75
  fi
  cat "$output_fifo" >"$partial_log" &
  logger_pid=$!
  if ! kill -0 "$logger_pid" 2>/dev/null; then
    write_terminal_runner "$run_directory" "$token" command_launch_failed || true
    terminal_written=1
    return 75
  fi

  if [ "$execution_mode" = privileged ]; then
    if [ "$target_user" = root ]; then
      setsid bash -lc "$command_string" >"$output_fifo" 2>&1 &
    elif [ -z "$remote_path" ] || ! sudo -n -u "$target_user" test -x "$remote_dir" >/dev/null 2>&1; then
      # A minimal qualification adapter may not expose a login PATH through
      # its fake SSH route.  Let the forwarded login-shell command resolve
      # its own inherited PATH in that bounded adapter case.
      setsid bash -lc "$command_string" >"$output_fifo" 2>&1 &
    elif sudo -n -u "$target_user" true >/dev/null 2>&1; then
      # The normal Metal path enters the login user's shell before the
      # existing sudo env transition.  Keep a private shell fallback for
      # minimal qualification adapters that provide only `sudo -n` and do
      # not implement its user-selection option; the forwarded command still
      # carries the same login HOME/PATH and guest environment.
      setsid sudo -n -u "$target_user" env "HOME=$remote_home" "PATH=$remote_path" \
        bash -lc "$command_string" >"$output_fifo" 2>&1 &
    else
      setsid bash -lc "$command_string" >"$output_fifo" 2>&1 &
    fi
  elif [ "$target_user" = root ]; then
    setsid bash -lc "$command_string" >"$output_fifo" 2>&1 &
  else
    setsid sudo -n -u "$target_user" env "HOME=$remote_home" "PATH=$remote_path" \
      bash -lc "$command_string" >"$output_fifo" 2>&1 &
  fi
  command_pid=$!
  # From the instant the supervised command identity exists, a supervisor
  # stop must forward to it even though the durable `running` phase has not
  # yet been published.
  running=1
  if [ -z "$command_pid" ] || [ -z "$logger_pid" ]; then
    running=0
    write_terminal_runner "$run_directory" "$token" command_launch_failed || true
    terminal_written=1
    return 75
  fi
  if ! write_phase "$run_directory" "$token" running; then
    supervisor_terminated=1
    kill -TERM -- "-${command_pid}" 2>/dev/null || kill -TERM "$command_pid" 2>/dev/null || true
    wait "$command_pid" 2>/dev/null || true
    wait "$logger_pid" 2>/dev/null || true
    write_terminal_runner "$run_directory" "$token" log_finalize_failed || true
    terminal_written=1
    return 74
  fi

  set +e
  while :; do
    wait "$command_pid"
    command_status=$?
    if [ "$supervisor_terminated" -eq 1 ] && kill -0 "$command_pid" 2>/dev/null; then
      continue
    fi
    break
  done
  wait "$logger_pid"
  logger_status=$?
  set -e
  rm -f "$output_fifo"

  if [ "$logger_status" -ne 0 ]; then
    write_terminal_runner "$run_directory" "$token" log_write_failed || true
    terminal_written=1
    return 74
  fi
  if ! sync_path "$partial_log"; then
    write_terminal_runner "$run_directory" "$token" log_finalize_failed || true
    terminal_written=1
    return 74
  fi
  if ! mv -f "$partial_log" "$final_log"; then
    write_terminal_runner "$run_directory" "$token" log_finalize_failed || true
    terminal_written=1
    return 74
  fi
  if ! sync_path "$run_directory"; then
    write_terminal_runner "$run_directory" "$token" log_finalize_failed || true
    terminal_written=1
    return 74
  fi
  if [ "$supervisor_terminated" -eq 1 ]; then
    write_terminal_command "$run_directory" "$token" "$command_status" supervisor_terminated || {
      return 74
    }
  else
    write_terminal_command "$run_directory" "$token" "$command_status" command_returned || {
      return 74
    }
  fi
  terminal_written=1
  return "$command_status"
}

if [ "${1:-}" = --durable ]; then
  shift
  durable_main "$@"
  exit $?
fi

if [ "${1:-}" = --collect ]; then
  shift
  [ "$#" -eq 1 ] || [ "$#" -eq 4 ] || {
    collector_marker invalid_run_id
    exit 64
  }
  COLLECT_TIMEOUT_SECONDS="${2:-}"
  COLLECT_LOCK_PATH="${3:-}"
  COLLECT_OWNER_PATH="${4:-}"
  collect_run "$1" "$RUNS_ROOT_DEFAULT"
  exit $?
fi

# Existing bootstrap lease-holder protocol.  Its argument order is preserved
# for bespoke sync/shell callers and for the qualification fixtures.
[ "$#" -ge 8 ] || {
  printf 'lease holder: expected at least 8 arguments\n' >&2
  exit 74
}
LOCK_PATH="$1"
OWNER_PATH="$2"
TIMEOUT_SECONDS="$3"
TOKEN="$4"
ACTION="$5"
SCENARIO="$6"
WORKSPACE="$7"
COMMIT="$8"
RELEASE_FD="${9:-0}"

exec 8>"${LOCK_PATH}"
if ! flock -w "${TIMEOUT_SECONDS}" 8; then
  echo "FATAL: timed out after ${TIMEOUT_SECONDS}s acquiring ${LOCK_PATH}" >&2
  if [ -r "${OWNER_PATH}" ]; then
    echo "current lease owner:" >&2
    sed 's/^/  /' "${OWNER_PATH}"
  else
    echo "current lease owner: metadata unavailable" >&2
  fi
  exit 75
fi

OWNER_TMP="${OWNER_PATH}.tmp.$$"
cleanup() {
  if [ -r "${OWNER_PATH}" ] && grep -qx "token=${TOKEN}" "${OWNER_PATH}"; then
    rm -f "${OWNER_PATH}"
  fi
  rm -f "${OWNER_TMP}"
}
trap cleanup EXIT
trap 'exit 143' HUP INT TERM

{
  printf 'pid=%s\n' "$$"
  printf 'started_at=%s\n' "$(utc_now)"
  printf 'action=%s\n' "${ACTION}"
  printf 'scenario=%s\n' "${SCENARIO}"
  printf 'workspace=%s\n' "${WORKSPACE}"
  printf 'commit=%s\n' "${COMMIT}"
  printf 'token=%s\n' "${TOKEN}"
} >"${OWNER_TMP}"
mv "${OWNER_TMP}" "${OWNER_PATH}"

printf 'OVERDRIVE_METAL_LEASE_ACQUIRED token=%s pid=%s\n' "${TOKEN}" "$$"
# The streamed launcher supplies this script through `bash -c`, leaving the
# SSH session's stdin on the selected release descriptor. EOF, cancellation,
# or a signal releases descriptor 8 and removes only this token's metadata.
while IFS= read -r -u "${RELEASE_FD}" _line; do :; done

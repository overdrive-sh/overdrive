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
  case "$(basename "$path")" in
    metadata.v1)
      METADATA_VERSION=1
      ordered_record_shape "$path" \
        format run_id token unit created_at scenario_b64 workspace_b64 commit \
        source_digest remote_dir_b64 target_user_b64 execution_mode || return 1
      [ "$(record_value "$path" 1)" = overdrive-metal-run-v1 ] || return 1
      ;;
    metadata.v2)
      METADATA_VERSION=2
      ordered_record_shape "$path" \
        format run_id token unit created_at scenario_b64 workspace_b64 commit \
        source_digest remote_dir_b64 target_user_b64 execution_mode execution_class || return 1
      [ "$(record_value "$path" 1)" = overdrive-metal-run-v2 ] || return 1
      value="$(record_value "$path" 13)"
      [ "$value" = ordinary ] || [ "$value" = exclusive ] || return 1
      ;;
    *) return 1 ;;
  esac
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

METADATA_VERSION=0
COMPLETION_KIND=""
COMPLETION_ERROR=""
COMPLETION_ROOT=""

completion_valid() {
  local path="$1"
  local run_id="$2"
  local count
  COMPLETION_KIND=""
  COMPLETION_ERROR=""
  COMPLETION_ROOT=""
  [ -f "$path" ] && [ ! -L "$path" ] || return 1
  record_has_final_newline "$path" || return 1
  [ "$(record_value "$path" 1)" = overdrive-metal-completion-v1 ] || return 1
  [ "$(record_value "$path" 2)" = "$run_id" ] || return 1
  COMPLETION_KIND="$(record_value "$path" 3)"
  count="$(wc -l <"$path" | tr -d '[:space:]')"
  case "$COMPLETION_KIND" in
    complete)
      ordered_record_shape "$path" format run_id kind finished_at || return 1
      [ "$count" -eq 4 ] || return 1
      is_rfc3339_utc "$(record_value "$path" 4)" || return 1
      ;;
    runner)
      ordered_record_shape "$path" format run_id kind error root finished_at || return 1
      [ "$count" -eq 6 ] || return 1
      COMPLETION_ERROR="$(record_value "$path" 4)"
      [ "$COMPLETION_ERROR" = cleanup_failed ] || return 1
      COMPLETION_ROOT="$(record_value "$path" 5)"
      case "$COMPLETION_ROOT" in guest_staging|vmm_runtime|command_tmp) ;; *) return 1 ;; esac
      is_rfc3339_utc "$(record_value "$path" 6)" || return 1
      ;;
    *) return 1 ;;
  esac
}

write_completion_complete() {
  local run_id="$1"
  local directory="$2"
  atomic_write "$directory/completion.v1" "format=overdrive-metal-completion-v1
run_id=${run_id}
kind=complete
finished_at=$(utc_now)
"
}

write_completion_cleanup_failed() {
  local run_id="$1"
  local directory="$2"
  local root="$3"
  atomic_write "$directory/completion.v1" "format=overdrive-metal-completion-v1
run_id=${run_id}
kind=runner
error=cleanup_failed
root=${root}
finished_at=$(utc_now)
"
}

safe_owned_root() {
  local path="$1"
  local expected="$2"
  local canonical component current
  local -a components=()
  [[ "$path" = /* ]] || return 1
  [[ "$path" != *$'\n'* && "$path" != *$'\r'* ]] || return 1
  case "${path}/" in *//*|*/./*|*/../*) return 1 ;; esac
  canonical="$(realpath -m -- "$path" 2>/dev/null)" || return 1
  [ "$canonical" = "$path" ] || return 1
  [ "$canonical" = "$expected" ] || return 1
  current=""
  IFS='/' read -r -a components <<<"${path#/}"
  for component in "${components[@]}"; do
    [ -n "$component" ] || continue
    current="${current}/${component}"
    [ ! -L "$current" ] || return 1
  done
}

remove_owned_root() {
  local path="$1"
  local expected="$2"
  safe_owned_root "$path" "$expected" || return 1
  if [ -L "$path" ]; then
    return 1
  fi
  if [ -e "$path" ]; then
    [ -d "$path" ] || return 1
    rm -rf -- "$path" || return 1
  fi
  [ ! -e "$path" ] && [ ! -L "$path" ]
}

finalize_run() {
  local run_id="$1"
  local runs_root="$2"
  local run_directory="${runs_root}/${run_id}"
  local metadata="${run_directory}/metadata.v2"
  local remote_dir
  local root
  local index

  if ! valid_run_id "$run_id"; then
    collector_marker invalid_run_id
    return 64
  fi
  if [ ! -d "$run_directory" ] || [ -L "$run_directory" ] || \
    ! metadata_valid "$metadata" "$run_id" "overdrive-metal-run-${run_id}.service" || \
    [ "$METADATA_VERSION" -ne 2 ]; then
    collector_marker run_artifact_io
    return 74
  fi
  remote_dir="$(printf '%s' "$(record_value "$metadata" 10)" | base64 -d 2>/dev/null)" || {
    collector_marker run_artifact_io
    return 74
  }
  [[ "$remote_dir" = /* ]] || { collector_marker run_artifact_io; return 74; }

  local -a roots=(
    "/srv/vm/metal/${run_id}"
    "/run/ovd-gk/${run_id}"
    "${remote_dir}/target/metal-run-tmp/${run_id}"
  )
  local -a expected=(
    "/srv/vm/metal/${run_id}"
    "/run/ovd-gk/${run_id}"
    "${remote_dir}/target/metal-run-tmp/${run_id}"
  )
  local -a names=(guest_staging vmm_runtime command_tmp)

  for index in 0 1 2; do
    root="${names[$index]}"
    if ! remove_owned_root "${roots[$index]}" "${expected[$index]}"; then
      write_completion_cleanup_failed "$run_id" "$run_directory" "$root" || true
      printf 'OVERDRIVE_METAL_RUNNER_ERROR error=cleanup_failed root=%s\n' "$root" >&2
      return 74
    fi
  done
  write_completion_complete "$run_id" "$run_directory" || {
    collector_marker run_artifact_io
    return 74
  }
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
      lease_timeout|maintenance_timeout|checkout_timeout|\
      checkout_identity_conflict|source_mismatch|runner_preparation_failed|\
      preflight_failed|launch_aborted|controller_eof_before_start|\
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

remaining_lock_timeout() {
  local deadline_ns="$1"
  local now_ns remaining_ns seconds milliseconds
  now_ns="$(date +%s%N)" || return 1
  [[ "$now_ns" =~ ^[0-9]+$ ]] || return 1
  remaining_ns=$((deadline_ns - now_ns))
  [ "$remaining_ns" -gt 1000000 ] || return 1
  seconds=$((remaining_ns / 1000000000))
  milliseconds=$(((remaining_ns % 1000000000) / 1000000))
  printf -v REMAINING_LOCK_TIMEOUT '%d.%03d' "$seconds" "$milliseconds"
}

poll_prestart_control() {
  local interval="$1"
  local read_status
  if [ "$control_received" -eq 1 ]; then
    sleep "$interval"
    return 0
  fi
  if IFS= read -r -t "$interval" -u 0 control; then
    # Preserve any existing control line verbatim; its current precedence is
    # still applied after the same admission and checkout locks are held.
    control_received=1
    return 0
  else
    read_status=$?
  fi
  [ "$read_status" -gt 128 ] && return 0
  return 2
}

cancel_prestart_lock_waiter() {
  [ -n "$lock_wait_pid" ] || return 0
  kill -TERM "$lock_wait_pid" 2>/dev/null || true
  if wait "$lock_wait_pid" 2>/dev/null; then :; fi
  lock_wait_pid=""
}

prestart_lock_waiter_running() {
  local active_pids
  active_pids="$(jobs -pr)"
  [[ $'\n'"$active_pids"$'\n' == *$'\n'"$lock_wait_pid"$'\n'* ]]
}

wait_for_prestart_lock() {
  local lock_mode="$1"
  local lock_fd="$2"
  local deadline_ns="$3"
  local poll_status
  remaining_lock_timeout "$deadline_ns" || return 1
  # Keep one flock waiter queued on the inherited descriptor while the
  # supervisor observes controller EOF. A successful flock remains held by
  # the parent descriptor after this child exits.
  flock "$lock_mode" -w "$REMAINING_LOCK_TIMEOUT" "$lock_fd" &
  lock_wait_pid=$!
  while prestart_lock_waiter_running; do
    if [ "$control_received" -eq 1 ]; then
      sleep 0.01
      continue
    fi
    if poll_prestart_control 0.05; then
      continue
    else
      poll_status=$?
      if [ "$poll_status" -eq 2 ]; then
        cancel_prestart_lock_waiter
        return 2
      fi
    fi
  done
  if wait "$lock_wait_pid"; then
    lock_wait_pid=""
    return 0
  fi
  lock_wait_pid=""
  return 1
}

wait_for_admitted_ordinary_units() {
  local deadline_ns="$1"
  local current_run_id="$2"
  local runs_root="$3"
  local directory run_id metadata phase unit execution_class pending
  while :; do
    pending=0
    if [ -d "$runs_root" ]; then
      for directory in "$runs_root"/*; do
        [ -d "$directory" ] && [ ! -L "$directory" ] || continue
        run_id="${directory##*/}"
        valid_run_id "$run_id" || continue
        [ "$run_id" = "$current_run_id" ] && continue
        # A run that has published its terminal record has finished; it can
        # no longer be an admitted ordinary unit, and any process still
        # exiting holds the shared barrier this caller has already won
        # exclusively.  Records are retained indefinitely, so the wait must
        # cost the number of unfinished runs, not the length of history:
        # validating a record forks dozens of processes.
        if [ -e "$directory/terminal.v1" ] || [ -L "$directory/terminal.v1" ]; then
          continue
        fi
        metadata="$directory/metadata.v2"
        phase="$directory/phase.v1"
        [ -e "$metadata" ] || [ -L "$metadata" ] || continue
        unit="overdrive-metal-run-${run_id}.service"
        # A record whose unit is not alive (a holder that died before it
        # could publish a terminal record) admits nothing, whatever it says.
        unit_is_live "$unit" || continue
        if ! metadata_valid "$metadata" "$run_id" "$unit" || \
          [ "$METADATA_VERSION" -ne 2 ] || ! phase_valid "$phase" "$run_id"; then
          return 2
        fi
        execution_class="$(record_value "$metadata" 13)"
        [ "$execution_class" = ordinary ] || continue
        case "$(record_value "$phase" 3)" in
          preparing|running)
            pending=1
            break
            ;;
          waiting_for_lease) ;;
          *) return 2 ;;
        esac
      done
    fi
    [ "$pending" -eq 0 ] && return 0
    remaining_lock_timeout "$deadline_ns" || return 1
    sleep 0.1
  done
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
    maintenance_timeout)
      if [ -n "$COLLECT_TIMEOUT_SECONDS" ]; then
        printf 'FATAL: timed out after %ss acquiring %s\n' \
          "$COLLECT_TIMEOUT_SECONDS" "$COLLECT_LOCK_PATH" >&2
        if [ -r "$COLLECT_OWNER_PATH" ]; then
          printf 'current lease owner:\n' >&2
          sed 's/^/  /' "$COLLECT_OWNER_PATH" >&2
        fi
      fi
      collector_marker "$error"
      return 75
      ;;
    checkout_timeout|runner_preparation_failed)
      collector_marker "$error"
      return 75
      ;;
    checkout_identity_conflict|source_mismatch|preflight_failed)
      collector_marker "$error"
      return 78
      ;;
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
  local metadata_v1="${run_directory}/metadata.v1"
  local metadata_v2="${run_directory}/metadata.v2"
  local metadata=""
  local completion="${run_directory}/completion.v1"
  local phase="${run_directory}/phase.v1"
  local terminal="${run_directory}/terminal.v1"
  local final_log="${run_directory}/output.log"
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
    if { [ -e "$metadata_v1" ] || [ -L "$metadata_v1" ]; } && \
      { [ -e "$metadata_v2" ] || [ -L "$metadata_v2" ]; }; then
      collector_artifact_error
      return 74
    elif [ -f "$metadata_v1" ] && [ ! -L "$metadata_v1" ]; then
      metadata="$metadata_v1"
    elif [ -f "$metadata_v2" ] && [ ! -L "$metadata_v2" ]; then
      metadata="$metadata_v2"
    else
      metadata=""
    fi
    if [ -z "$metadata" ] || [ ! -f "$phase" ] || [ -L "$phase" ]; then
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
      if [ "$METADATA_VERSION" -eq 2 ]; then
        while unit_is_live "$unit"; do sleep 0.1; done
        if ! terminal_valid "$terminal" "$run_id"; then
          collector_artifact_error
          return 74
        fi
        if ! completion_valid "$completion" "$run_id"; then
          collector_artifact_error
          return 74
        fi
        if [ "$COMPLETION_KIND" = runner ]; then
          if [ "$TERMINAL_KIND" = command ]; then
            if [ ! -f "$final_log" ] || ! emit_available "$final_log" "$offset"; then
              collector_artifact_error
              return 74
            fi
          fi
          printf 'OVERDRIVE_METAL_RUNNER_ERROR error=cleanup_failed root=%s\n' \
            "$COMPLETION_ROOT" >&2
          return 74
        fi
      fi
      terminal_result "$run_directory" "$run_id" "$offset"
      return $?
    fi

    if [ "$phase_value" = running ]; then
      while :; do
        if [ -f "$terminal" ]; then
          if [ "$METADATA_VERSION" -eq 2 ]; then
            while unit_is_live "$unit"; do sleep 0.1; done
            if ! terminal_valid "$terminal" "$run_id" || \
              ! completion_valid "$completion" "$run_id"; then
              collector_artifact_error
              return 74
            fi
            if [ "$COMPLETION_KIND" = runner ]; then
              if [ "$TERMINAL_KIND" = command ]; then
                if [ ! -f "$final_log" ] || ! emit_available "$final_log" "$offset"; then
                  collector_artifact_error
                  return 74
                fi
              fi
              printf 'OVERDRIVE_METAL_RUNNER_ERROR error=cleanup_failed root=%s\n' \
                "$COMPLETION_ROOT" >&2
              return 74
            fi
          fi
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
  [ "$#" -eq 18 ] || {
    printf 'durable lease holder: expected 18 arguments\n' >&2
    return 74
  }
  local maintenance_lock_path="$1"
  maintenance_owner_path="$2"
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
  local tree_lock_path="${16}"
  tree_owner_path="${17}"
  local execution_class="${18}"
  local unit="overdrive-metal-run-${token}.service"
  local run_directory="${runs_root}/${token}"
  local metadata="${run_directory}/metadata.v2"
  owner_path="${run_directory}/owner.v1"
  local partial_log="${run_directory}/output.log.partial"
  local final_log="${run_directory}/output.log"
  output_fifo="${run_directory}/output.pipe"
  owner_tmp="${owner_path}.tmp.$$"
  tree_owner_tmp="${tree_owner_path}.tmp.$$"
  maintenance_owner_tmp="${maintenance_owner_path}.tmp.$$"
  local control=""
  local logger_pid=""
  local command_pid=""
  local command_status=0
  local logger_status=0
  local supervisor_terminated=0
  local running=0
  owner_written=0
  tree_owner_written=0
  maintenance_owner_written=0
  terminal_written=0
  local metadata_content
  local deadline_start_ns
  local deadline_ns
  local wait_result
  lock_wait_pid=""
  control_received=0

  valid_run_id "$token" || return 74
  [ "$execution_mode" = privileged ] || [ "$execution_mode" = login ] || return 74
  [ "$execution_class" = ordinary ] || [ "$execution_class" = exclusive ] || return 74
  [[ "$timeout_seconds" =~ ^[0-9]+$ ]] || return 74
  deadline_start_ns="$(date +%s%N)" || return 74
  [[ "$deadline_start_ns" =~ ^[0-9]+$ ]] || return 74
  deadline_ns=$((deadline_start_ns + timeout_seconds * 1000000000))
  metadata_content="format=overdrive-metal-run-v2
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
execution_class=${execution_class}
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
    cancel_prestart_lock_waiter
    if [ "$owner_written" -eq 1 ] && [ -r "$owner_path" ] && \
      grep -qx "token=${token}" "$owner_path"; then
      rm -f "$owner_path"
    fi
    if [ "$tree_owner_written" -eq 1 ] && [ -r "$tree_owner_path" ] && \
      grep -qx "token=${token}" "$tree_owner_path"; then
      rm -f "$tree_owner_path"
    fi
    if [ "$maintenance_owner_written" -eq 1 ] && [ -r "$maintenance_owner_path" ] && \
      grep -qx "token=${token}" "$maintenance_owner_path"; then
      rm -f "$maintenance_owner_path"
    fi
    rm -f "$owner_tmp" "$tree_owner_tmp" "$maintenance_owner_tmp" "$output_fifo"
  }
  handle_signal() {
    if [ "$running" -eq 0 ]; then
      cancel_prestart_lock_waiter
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
      cancel_prestart_lock_waiter
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

  # Lock order is always checkout lease first, then the node-wide
  # physical-maintenance barrier (shared for ordinary, exclusive for
  # --exclusive).  A run therefore never holds the barrier while waiting for
  # a checkout lease only it needs; the barrier is taken only by holders of a
  # checkout lease and nobody waits for a checkout lease while holding the
  # barrier, so no wait cycle exists.  Release is the reverse (process exit).
  if ! exec 10>"$tree_lock_path"; then
    write_terminal_runner "$run_directory" "$token" runner_preparation_failed || true
    terminal_written=1
    return 75
  fi
  if wait_for_prestart_lock -x 10 "$deadline_ns"; then
    :
  else
    wait_result=$?
    if [ "$wait_result" -eq 2 ]; then
      write_terminal_runner "$run_directory" "$token" controller_eof_before_start || true
      terminal_written=1
      return 75
    fi
    write_terminal_runner "$run_directory" "$token" checkout_timeout || true
    terminal_written=1
    return 75
  fi

  if ! exec 8>"$maintenance_lock_path"; then
    write_terminal_runner "$run_directory" "$token" runner_preparation_failed || true
    terminal_written=1
    return 75
  fi
  if [ "$execution_class" = ordinary ]; then
    if wait_for_prestart_lock -s 8 "$deadline_ns"; then
      :
    else
      wait_result=$?
      if [ "$wait_result" -eq 2 ]; then
        write_terminal_runner "$run_directory" "$token" controller_eof_before_start || true
        terminal_written=1
        return 75
      fi
      write_terminal_runner "$run_directory" "$token" maintenance_timeout || true
      terminal_written=1
      return 75
    fi
  else
    if wait_for_prestart_lock -x 8 "$deadline_ns"; then
      :
    else
      wait_result=$?
      if [ "$wait_result" -eq 2 ]; then
        write_terminal_runner "$run_directory" "$token" controller_eof_before_start || true
        terminal_written=1
        return 75
      fi
      write_terminal_runner "$run_directory" "$token" maintenance_timeout || true
      terminal_written=1
      return 75
    fi
  fi

  if [ "$execution_class" = exclusive ]; then
    if wait_for_admitted_ordinary_units "$deadline_ns" "$token" "$runs_root"; then
      wait_result=0
    else
      wait_result=$?
    fi
    if [ "$wait_result" -eq 1 ]; then
      write_terminal_runner "$run_directory" "$token" maintenance_timeout || true
      terminal_written=1
      return 75
    elif [ "$wait_result" -ne 0 ]; then
      printf 'OVERDRIVE_METAL_RUNNER_ERROR error=preflight_failed probe=owner\n' >&2
      write_terminal_runner "$run_directory" "$token" preflight_failed || true
      terminal_written=1
      return 78
    fi
  fi

  if ! {
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
  } >"$owner_tmp"; then
    write_terminal_runner "$run_directory" "$token" runner_preparation_failed || true
    terminal_written=1
    return 75
  fi
  if ! sync_path "$owner_tmp" || ! mv -f "$owner_tmp" "$owner_path" || \
    ! sync_path "$(dirname "$owner_path")"; then
    write_terminal_runner "$run_directory" "$token" runner_preparation_failed || true
    terminal_written=1
    return 75
  fi
  owner_written=1

  if ! {
    printf 'pid=%s\n' "$$"
    printf 'started_at=%s\n' "$(utc_now)"
    printf 'action=run\n'
    printf 'scenario=%s\n' "$scenario"
    printf 'workspace=%s\n' "$workspace"
    printf 'commit=%s\n' "$commit"
    printf 'token=%s\n' "$token"
  } >"$tree_owner_tmp"; then
    write_terminal_runner "$run_directory" "$token" runner_preparation_failed || true
    terminal_written=1
    return 75
  fi
  if ! sync_path "$tree_owner_tmp" || ! mv -f "$tree_owner_tmp" "$tree_owner_path" || \
    ! sync_path "$(dirname "$tree_owner_path")"; then
    write_terminal_runner "$run_directory" "$token" runner_preparation_failed || true
    terminal_written=1
    return 75
  fi
  tree_owner_written=1

  if [ "$execution_class" = exclusive ]; then
    if ! {
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
    } >"$maintenance_owner_tmp"; then
      write_terminal_runner "$run_directory" "$token" runner_preparation_failed || true
      terminal_written=1
      return 75
    fi
    if ! sync_path "$maintenance_owner_tmp" || \
      ! mv -f "$maintenance_owner_tmp" "$maintenance_owner_path" || \
      ! sync_path "$(dirname "$maintenance_owner_path")"; then
      write_terminal_runner "$run_directory" "$token" runner_preparation_failed || true
      terminal_written=1
      return 75
    fi
    maintenance_owner_written=1
  fi

  write_phase "$run_directory" "$token" preparing || {
    write_terminal_runner "$run_directory" "$token" log_finalize_failed || true
    terminal_written=1
    return 74
  }
  printf 'OVERDRIVE_METAL_LEASE_ACQUIRED token=%s pid=%s\n' "$token" "$$"

  if [ "$control_received" -eq 0 ]; then
    if ! IFS= read -r control; then
      write_terminal_runner "$run_directory" "$token" controller_eof_before_start || true
      terminal_written=1
      return 75
    fi
  fi
  case "$control" in
    START) ;;
    ABORT)
      write_terminal_runner "$run_directory" "$token" launch_aborted || true
      terminal_written=1
      return 75
      ;;
    "ABORT error=checkout_identity_conflict"|"ABORT error=source_mismatch"|"ABORT error=preflight_failed")
      write_terminal_runner "$run_directory" "$token" "${control#ABORT error=}" || true
      terminal_written=1
      return 78
      ;;
    "ABORT error=runner_preparation_failed")
      write_terminal_runner "$run_directory" "$token" runner_preparation_failed || true
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

if [ "${1:-}" = --finalize ]; then
  shift
  [ "$#" -eq 2 ] || {
    collector_marker invalid_run_id
    exit 64
  }
  finalize_run "$1" "$2"
  exit $?
fi

if [ "${1:-}" = --collect ]; then
  shift
  [ "$#" -eq 1 ] || [ "$#" -eq 2 ] || \
    [ "$#" -eq 4 ] || [ "$#" -eq 5 ] || {
    collector_marker invalid_run_id
    exit 64
  }
  run_id="$1"
  if [ "$#" -eq 2 ] || [ "$#" -eq 5 ]; then
    runs_root="$2"
    shift 2
  else
    runs_root="$RUNS_ROOT_DEFAULT"
    shift
  fi
  COLLECT_TIMEOUT_SECONDS="${1:-}"
  COLLECT_LOCK_PATH="${2:-}"
  COLLECT_OWNER_PATH="${3:-}"
  collect_run "$run_id" "$runs_root"
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
LOCK_DEADLINE_START_NS="$(date +%s%N)"
LOCK_DEADLINE_NS=$((LOCK_DEADLINE_START_NS + TIMEOUT_SECONDS * 1000000000))

exec 8>"${LOCK_PATH}"
if ! remaining_lock_timeout "$LOCK_DEADLINE_NS" || \
  ! flock -x -w "${REMAINING_LOCK_TIMEOUT}" 8; then
  if [ "${ACTION}" = sync ] || [ "${ACTION}" = pull ]; then
    collector_marker checkout_timeout
  else
    collector_marker maintenance_timeout
  fi
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

if [ "${ACTION}" != sync ] && [ "${ACTION}" != pull ]; then
  if wait_for_admitted_ordinary_units "$LOCK_DEADLINE_NS" "${TOKEN}" "$RUNS_ROOT_DEFAULT"; then
    :
  else
    wait_result=$?
    if [ "$wait_result" -eq 1 ]; then
      collector_marker maintenance_timeout
      exit 75
    fi
    printf 'OVERDRIVE_METAL_RUNNER_ERROR error=preflight_failed probe=owner\n' >&2
    exit 78
  fi
fi

printf 'OVERDRIVE_METAL_LEASE_ACQUIRED token=%s pid=%s\n' "${TOKEN}" "$$"
# The streamed launcher supplies this script through `bash -c`, leaving the
# SSH session's stdin on the selected release descriptor. EOF, cancellation,
# or a signal releases descriptor 8 and removes only this token's metadata.
while IFS= read -r -u "${RELEASE_FD}" _line; do :; done

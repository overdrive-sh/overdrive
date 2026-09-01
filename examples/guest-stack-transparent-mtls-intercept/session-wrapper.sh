#!/usr/bin/env bash
# Establish the private OS session/process group before starting the fresh
# keyring/product chain. Parent ownership is acknowledged before any child.
set -euo pipefail

[[ "$#" -eq 10 ]] || {
  echo "gti-e07 session wrapper: expected 10 arguments" >&2
  exit 2
}

readonly LAUNCH_TOKEN="$1"
readonly READY_FILE="$2"
readonly ACK_FILE="$3"
readonly SERVE_PID_FILE="$4"
readonly DESCRIPTION="$5"
readonly CONFIG_DIR="$6"
readonly CREDS_DIR="$7"
readonly BIN="$8"
readonly BIND="$9"
readonly DATA_DIR="${10}"

IFS= read -r wrapper_stat <"/proc/$$/stat"
wrapper_fields="${wrapper_stat##*) }"
# shellcheck disable=SC2086 # intentional procfs field splitting
set -- $wrapper_fields
wrapper_pgid="$3"
wrapper_start="${20}"
[[ "$wrapper_pgid" == "$$" ]] || {
  echo "gti-e07 session wrapper: setsid did not establish a private process group" >&2
  exit 70
}

# No external command or process substitution has run yet. The wrapper waits
# with shell builtins until the parent records its exact setsid group. If the
# parent is interrupted before acknowledgement, direct-child termination is
# therefore sufficient and cannot strand a descendant.
ack_deadline=$((SECONDS + 5))
while :; do
  if [[ -s "$ACK_FILE" ]]; then
    ack_extra=""
    read -r ack_token ack_pid ack_pgid ack_start ack_extra <"$ACK_FILE"
    [[ -z "$ack_extra" && "$ack_token" == "$LAUNCH_TOKEN" \
      && "$ack_pid" == "$$" && "$ack_pgid" == "$wrapper_pgid" \
      && "$ack_start" == "$wrapper_start" ]] || {
      echo "gti-e07 session wrapper: invalid ownership acknowledgement" >&2
      exit 70
    }
    break
  fi
  (( SECONDS < ack_deadline )) || {
    echo "gti-e07 session wrapper: ownership acknowledgement timed out" >&2
    exit 70
  }
done

# Ownership is now race-safe. Atomically publish readiness before the first
# keyctl/bash/serve process, all of which inherit the recorded private group.
printf '%s %s %s %s\n' "$LAUNCH_TOKEN" "$$" "$wrapper_pgid" "$wrapper_start" \
  >"$READY_FILE.tmp.$$"
mv -- "$READY_FILE.tmp.$$" "$READY_FILE"

# shellcheck disable=SC2016 # positional parameters expand in the child shell
exec keyctl session - bash -c '
  set -euo pipefail
  launch_token="$1"
  pid_file="$2"
  description="$3"
  config_dir="$4"
  creds_dir="$5"
  bin="$6"
  bind="$7"
  data_dir="$8"
  keyctl describe @s >/dev/null \
    || { echo "gti-e07 run: fresh session keyring is not accessible" >&2; exit 70; }
  set +e
  keyctl search @s user "$description" >/dev/null 2>&1
  search_rc=$?
  set -e
  case "$search_rc" in
    0)
      echo "gti-e07 run: fresh session unexpectedly contains $description" >&2
      exit 70
      ;;
    1) ;;
    *)
      echo "gti-e07 run: could not verify absence of $description (keyctl exit $search_rc)" >&2
      exit 70
      ;;
  esac
  line="$(<"/proc/$$/stat")"
  rest="${line##*) }"
  # shellcheck disable=SC2086 # intentional procfs field splitting
  set -- $rest
  pgid="$3"
  start="${20}"
  printf "%s %s %s %s\n" "$launch_token" "$$" "$pgid" "$start" \
    >"$pid_file.tmp.$$"
  mv -- "$pid_file.tmp.$$" "$pid_file"
  exec env OVERDRIVE_CONFIG_DIR="$config_dir" CREDENTIALS_DIRECTORY="$creds_dir" \
    "$bin" serve --bind "$bind" --data-dir "$data_dir"
' gti-e07-session "$LAUNCH_TOKEN" "$SERVE_PID_FILE" "$DESCRIPTION" \
  "$CONFIG_DIR" "$CREDS_DIR" "$BIN" "$BIND" "$DATA_DIR"

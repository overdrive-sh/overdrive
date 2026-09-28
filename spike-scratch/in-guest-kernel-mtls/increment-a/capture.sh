#!/usr/bin/env bash
# Append-only evidence capture for one metal run of this increment.
#
#   capture.sh RUN_ID LABEL [--no-sudo] -- <remote script relative to repo> [args...]
#
# Runs `cargo xtask metal run` (rsync + shared lease + fail-closed native
# preflight) and writes runs/RUN_ID.{meta,stdout,stderr}. Refuses to overwrite
# an existing run. The metal target and host are redacted from the captures.
#
# The preflight requires a readable guest kernel/rootfs on the host. Prep runs
# (no VM boot) use the existing Linux staging files; the boot run overrides
# OVERDRIVE_METAL_KERNEL / OVERDRIVE_METAL_ROOTFS with the Nanos artifacts.
set -euo pipefail

readonly INCREMENT=spike-scratch/in-guest-kernel-mtls/increment-a
readonly RUN_ID="${1:?usage: capture.sh RUN_ID LABEL [--no-sudo] -- script [args]}"
readonly LABEL="${2:?label required}"
shift 2
SUDO_FLAG=()
if [[ "${1:-}" == "--no-sudo" ]]; then SUDO_FLAG=(--no-sudo); shift; fi
[[ "${1:-}" == "--" ]] || { echo "expected -- before remote command" >&2; exit 2; }
shift
[[ $# -gt 0 ]] || { echo "remote command required" >&2; exit 2; }

readonly RUN_BASE="$INCREMENT/runs/$RUN_ID"
[[ ! -e "$RUN_BASE.meta" && ! -e "$RUN_BASE.stdout" && ! -e "$RUN_BASE.stderr" ]] || {
  printf 'refusing to overwrite captured run %s\n' "$RUN_ID" >&2
  exit 2
}

TARGET_VALUE="$(sed -n 's/^[[:space:]]*OVERDRIVE_METAL_TARGET[[:space:]]*=[[:space:]]*//p' .env | tail -n 1)"
TARGET_VALUE="${TARGET_VALUE%\"}"; TARGET_VALUE="${TARGET_VALUE#\"}"
TARGET_VALUE="${TARGET_VALUE%\'}"; TARGET_VALUE="${TARGET_VALUE#\'}"
[[ -n "$TARGET_VALUE" ]]
TARGET_HOST="${TARGET_VALUE#*@}"
TARGET_USER="${TARGET_VALUE%@*}"

KERNEL="${OVERDRIVE_METAL_KERNEL:-/srv/vm/overdrive-testing/kernel}"
ROOTFS="${OVERDRIVE_METAL_ROOTFS:-/srv/vm/overdrive-testing/rootfs.ext4}"

TMP="$(mktemp -d "${TMPDIR:-/tmp}/igkm-a-capture.XXXXXX")"
trap 'rm -rf -- "$TMP"' EXIT HUP INT TERM

STARTED_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
set +e
RSYNC_BIN="$INCREMENT/rsync-without-local-env.sh" \
OVERDRIVE_METAL_KERNEL="$KERNEL" \
OVERDRIVE_METAL_ROOTFS="$ROOTFS" \
OVERDRIVE_METAL_SCENARIO="igkm-spike-a-$LABEL" \
cargo xtask metal run "${SUDO_FLAG[@]}" -- "$@" >"$TMP/stdout" 2>"$TMP/stderr"
EXIT_CODE=$?
set -e
ENDED_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

HOST_PREFIX2="$(printf '%s' "$TARGET_HOST" | cut -d. -f1-2)"
redact() {
  METAL_TARGET_REDACTION="$TARGET_VALUE" METAL_HOST_REDACTION="$TARGET_HOST" \
  METAL_USER_HOME_REDACTION="/home/$TARGET_USER" METAL_PREFIX2_REDACTION="$HOST_PREFIX2" \
    perl -pe 's/\Q$ENV{METAL_TARGET_REDACTION}\E/<redacted-metal-target>/g; s/\Q$ENV{METAL_HOST_REDACTION}\E/<redacted-metal-host>/g; s/\Q$ENV{METAL_USER_HOME_REDACTION}\E/<metal-home>/g; s/\b\Q$ENV{METAL_PREFIX2_REDACTION}\E\.\d+\.\d+/<redacted-provider-ip>/g' "$1"
}
redact "$TMP/stdout" > "$RUN_BASE.stdout"
redact "$TMP/stderr" > "$RUN_BASE.stderr"

{
  printf 'run_id=%s\n' "$RUN_ID"
  printf 'label=%s\n' "$LABEL"
  printf 'started_utc=%s\n' "$STARTED_UTC"
  printf 'ended_utc=%s\n' "$ENDED_UTC"
  printf 'exit_code=%s\n' "$EXIT_CODE"
  printf 'local_commit=%s\n' "$(git rev-parse HEAD)"
  printf 'command=OVERDRIVE_METAL_KERNEL=%s OVERDRIVE_METAL_ROOTFS=%s cargo xtask metal run %s -- %s\n' \
    "$KERNEL" "$ROOTFS" "${SUDO_FLAG[*]:-}" "$*"
  printf 'security_redaction=metal target, host and login home replaced in stdout/stderr\n'
  printf 'stdout_sha256=%s\n' "$(shasum -a 256 "$RUN_BASE.stdout" | awk '{print $1}')"
  printf 'stderr_sha256=%s\n' "$(shasum -a 256 "$RUN_BASE.stderr" | awk '{print $1}')"
} | perl -pe 's{/home/[A-Za-z0-9_.-]+}{<metal-home>}g' > "$RUN_BASE.meta"

printf 'capture complete: exit_code=%s evidence=%s.{meta,stdout,stderr}\n' "$EXIT_CODE" "$RUN_BASE"
exit "$EXIT_CODE"

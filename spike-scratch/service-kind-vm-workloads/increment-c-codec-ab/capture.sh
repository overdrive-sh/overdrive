#!/usr/bin/env bash
set -euo pipefail

readonly INCREMENT=spike-scratch/service-kind-vm-workloads/increment-c-codec-ab
readonly RUN_ID="${1:?usage: capture.sh RUN_ID (for example 0001)}"
readonly RUN_BASE="$INCREMENT/runs/$RUN_ID"
readonly SANITIZED_COMMAND="RSYNC_BIN=$INCREMENT/rsync-without-local-env.sh OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4 OVERDRIVE_METAL_SCENARIO=service-vm-codec-ab cargo xtask metal run -- bash $INCREMENT/run.sh"

[[ ! -e "$RUN_BASE.meta" && ! -e "$RUN_BASE.stdout" && ! -e "$RUN_BASE.stderr" ]] || {
  printf 'refusing to overwrite captured run %s\n' "$RUN_ID" >&2
  exit 2
}

TARGET_VALUE="$(sed -n 's/^[[:space:]]*OVERDRIVE_METAL_TARGET[[:space:]]*=[[:space:]]*//p' .env | tail -n 1)"
TARGET_VALUE="${TARGET_VALUE%\"}"
TARGET_VALUE="${TARGET_VALUE#\"}"
TARGET_VALUE="${TARGET_VALUE%\'}"
TARGET_VALUE="${TARGET_VALUE#\'}"
[[ -n "$TARGET_VALUE" ]]
TARGET_HOST="${TARGET_VALUE#*@}"

mkdir -p "$INCREMENT/runs"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/service-vm-codec-ab.XXXXXX")"
trap 'rm -rf -- "$TMP"' EXIT HUP INT TERM

STARTED_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
set +e
RSYNC_BIN="$INCREMENT/rsync-without-local-env.sh" \
  OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel \
  OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4 \
  OVERDRIVE_METAL_SCENARIO=service-vm-codec-ab \
  cargo xtask metal run -- bash "$INCREMENT/run.sh" >"$TMP/stdout" 2>"$TMP/stderr"
EXIT_CODE=$?
set -e
ENDED_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

METAL_TARGET_REDACTION="$TARGET_VALUE" METAL_HOST_REDACTION="$TARGET_HOST" \
  perl -pe 's/\Q$ENV{METAL_TARGET_REDACTION}\E/<redacted-metal-target>/g; s/\Q$ENV{METAL_HOST_REDACTION}\E/<redacted-metal-host>/g' \
  "$TMP/stdout" > "$RUN_BASE.stdout"
METAL_TARGET_REDACTION="$TARGET_VALUE" METAL_HOST_REDACTION="$TARGET_HOST" \
  perl -pe 's/\Q$ENV{METAL_TARGET_REDACTION}\E/<redacted-metal-target>/g; s/\Q$ENV{METAL_HOST_REDACTION}\E/<redacted-metal-host>/g' \
  "$TMP/stderr" > "$RUN_BASE.stderr"

{
  printf 'attempt=codec-ab-%s\n' "$RUN_ID"
  printf 'started_utc=%s\n' "$STARTED_UTC"
  printf 'ended_utc=%s\n' "$ENDED_UTC"
  printf 'exit_code=%s\n' "$EXIT_CODE"
  printf 'command=%s\n' "$SANITIZED_COMMAND"
  printf 'security_redaction=configured metal target and target host replaced in stdout/stderr\n'
  printf 'stdout_sha256=%s\n' "$(shasum -a 256 "$RUN_BASE.stdout" | awk '{print $1}')"
  printf 'stderr_sha256=%s\n' "$(shasum -a 256 "$RUN_BASE.stderr" | awk '{print $1}')"
} > "$RUN_BASE.meta"

printf 'capture complete: exit_code=%s evidence=%s.{meta,stdout,stderr}\n' "$EXIT_CODE" "$RUN_BASE"
exit "$EXIT_CODE"

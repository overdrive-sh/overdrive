#!/usr/bin/env bash
# Append-only evidence capture for one metal run of increment-j (GH #303 Spike E).
#
#   capture.sh RUN_ID LABEL [--no-sudo] -- <remote script relative to repo> [args...]
#
# Runs `cargo xtask metal run` (rsync + shared lease + fail-closed native
# preflight) and writes runs/RUN_ID.{meta,stdout,stderr}. Refuses to overwrite
# an existing run. The metal target and host are redacted from the captures.
# When the run embeds a tap.pcap as base64 between the markers
# `----- BEGIN tap.pcap base64 -----` / `----- END tap.pcap base64 -----`
# (run-mesh-fc.sh does), it is decoded into runs/RUN_ID.tap.pcap and its
# sha256 recorded in the .meta.
#
# The CH `kvm-tests` native preflight requires a readable guest kernel/rootfs on
# the host even for these non-CH commands; placeholder paths are passed (the
# prior increments set the same). This capture runs NO Cloud-Hypervisor VM --
# the probe boots Firecracker directly inside run-mesh-fc.sh.
set -euo pipefail

readonly INCREMENT=spike-scratch/in-guest-kernel-mtls/increment-j
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

TMP="$(mktemp -d "${TMPDIR:-/tmp}/igkm-i-capture.XXXXXX")"
trap 'rm -rf -- "$TMP"' EXIT HUP INT TERM

STARTED_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
set +e
RSYNC_BIN="$INCREMENT/rsync-without-local-env.sh" \
OVERDRIVE_METAL_KERNEL="$KERNEL" \
OVERDRIVE_METAL_ROOTFS="$ROOTFS" \
OVERDRIVE_METAL_SCENARIO="igkm-spike-e-accept-nonblocking-$LABEL" \
cargo xtask metal run "${SUDO_FLAG[@]}" -- "$@" >"$TMP/stdout" 2>"$TMP/stderr"
EXIT_CODE=$?
set -e
ENDED_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# Decode an embedded tap.pcap from the RAW stdout (before redaction) if present.
PCAP_SHA=""
if grep -q '^----- BEGIN tap.pcap base64 -----$' "$TMP/stdout"; then
  awk '/^----- BEGIN tap.pcap base64 -----$/{f=1;next} /^----- END tap.pcap base64 -----$/{f=0} f' "$TMP/stdout" \
    | python3 -c 'import sys,base64; sys.stdout.buffer.write(base64.b64decode("".join(sys.stdin.read().split())))' \
    > "$RUN_BASE.tap.pcap"
  PCAP_SHA="$(shasum -a 256 "$RUN_BASE.tap.pcap" | awk '{print $1}')"
fi

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
  if [[ -n "$PCAP_SHA" ]]; then printf 'tap_pcap_sha256=%s\n' "$PCAP_SHA"; fi
} | perl -pe 's{/home/[A-Za-z0-9_.-]+}{<metal-home>}g' > "$RUN_BASE.meta"

printf 'capture complete: exit_code=%s evidence=%s.{meta,stdout,stderr}%s\n' \
  "$EXIT_CODE" "$RUN_BASE" "$([[ -n "$PCAP_SHA" ]] && echo ',tap.pcap' || true)"
exit "$EXIT_CODE"

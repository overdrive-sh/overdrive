#!/usr/bin/env bash
# Canonical remote lease holder for every supported bare-metal writer.
set -euo pipefail

LOCK_PATH="${1:?lock path}"
OWNER_PATH="${2:?owner path}"
TIMEOUT_SECONDS="${3:?timeout seconds}"
TOKEN="${4:?lease token}"
ACTION="${5:?action}"
SCENARIO="${6:?scenario}"
WORKSPACE="${7:?workspace}"
COMMIT="${8:?commit}"

exec 8>"${LOCK_PATH}"
if ! flock -w "${TIMEOUT_SECONDS}" 8; then
  echo "FATAL: timed out after ${TIMEOUT_SECONDS}s acquiring ${LOCK_PATH}" >&2
  if [ -r "${OWNER_PATH}" ]; then
    echo "current lease owner:" >&2
    sed 's/^/  /' "${OWNER_PATH}" >&2
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
  printf 'started_at=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'action=%s\n' "${ACTION}"
  printf 'scenario=%s\n' "${SCENARIO}"
  printf 'workspace=%s\n' "${WORKSPACE}"
  printf 'commit=%s\n' "${COMMIT}"
  printf 'token=%s\n' "${TOKEN}"
} >"${OWNER_TMP}"
mv "${OWNER_TMP}" "${OWNER_PATH}"

printf 'OVERDRIVE_METAL_LEASE_ACQUIRED token=%s pid=%s\n' "${TOKEN}" "$$"
# The caller retains the write end of this session's stdin. EOF, cancellation,
# or a signal releases descriptor 8 and removes only this token's metadata.
while IFS= read -r _line; do :; done

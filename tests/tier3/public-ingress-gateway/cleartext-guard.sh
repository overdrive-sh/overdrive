#!/usr/bin/env bash

assert_capture_excludes_marker() {
  local capture="$1"
  local marker="$2"
  local label="$3"
  local decoded decode_error decode_status
  decoded="$(mktemp "${TMPDIR:-/tmp}/pig-cleartext.XXXXXX")"
  decode_error="${decoded}.err"
  decode_status=0
  tcpdump -A -nn -r "$capture" >"$decoded" 2>"$decode_error" || decode_status=$?
  if [[ "$decode_status" -ne 0 ]]; then
    echo "AT-PIG-E2E-1: $label capture decode failed" >&2
    rm -f -- "$decoded" "$decode_error"
    return 2
  fi
  local inspect_status=0
  grep -Fq "$marker" "$decoded" || inspect_status=$?
  case "$inspect_status" in
    0)
      echo "AT-PIG-E2E-1: $label capture exposed cleartext marker" >&2
      rm -f -- "$decoded" "$decode_error"
      return 1
      ;;
    1)
      rm -f -- "$decoded" "$decode_error"
      return 0
      ;;
    *)
      echo "AT-PIG-E2E-1: $label completed capture inspection failed" >&2
      rm -f -- "$decoded" "$decode_error"
      return 2
      ;;
  esac
}

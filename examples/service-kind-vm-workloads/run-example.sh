#!/usr/bin/env bash
# DISTILL handoff for the S-SVM black-box journeys. DELIVER replaces each
# pending mode with native-metal preparation and the exact built-binary path.
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly EXAMPLE_DIR
readonly PREPARE="$EXAMPLE_DIR/prepare.sh"

check_source() {
  "$PREPARE" check-source
}

pending_mode() {
  local mode="$1"
  echo "PENDING ${mode}: DELIVER must drive the built default-feature overdrive binary" >&2
  exit 75
}

case "${1:-}" in
  check-source)
    check_source
    ;;
  run)
    check_source
    case "${2:-}" in
      healthy|tcp-truthfulness-100|http-status-cross-driver|readiness-recovery|liveness-restart|zero-probes)
        pending_mode "$2"
        ;;
      *)
        echo 'usage: run-example.sh run healthy|tcp-truthfulness-100|http-status-cross-driver|readiness-recovery|liveness-restart|zero-probes' >&2
        exit 2
        ;;
    esac
    ;;
  *)
    echo 'usage: run-example.sh check-source|run <mode>' >&2
    exit 2
    ;;
esac

#!/usr/bin/env bash
# Run a command on the canonical Linux host for a git hook step.
#
#   infra/hooks/linux-run.sh <command> [args...]
#
# The native metal host (`cargo xtask metal run --`) is used when the
# workspace `.env` sets OVERDRIVE_METAL_TARGET; otherwise the Lima VM
# (`cargo xtask lima run --`). lefthook.yml routes every Rust step through
# this script so the choice is made in one place.
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
if [ -f "${root}/.env" ] && grep -qE '^[[:space:]]*OVERDRIVE_METAL_TARGET=[^[:space:]]+' "${root}/.env"; then
  exec cargo xtask metal run -- "$@"
fi
exec cargo xtask lima run -- "$@"

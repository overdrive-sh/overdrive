#!/bin/sh
# Compile-surface helper (NOT validation): runs a cargo command for the CH fork
# inside the overdrive Lima VM with a private CARGO_HOME/target dir.
# Usage: lima-ch.sh <cargo args...>
cd "$(dirname "$0")/../.." || exit 1
exec cargo xtask lima run --no-sudo -- bash -lc "cd vendors/cloud-hypervisor && export CARGO_HOME=\$HOME/.ch-spike-cargo-home CARGO_TARGET_DIR=\$HOME/.ch-spike-target && cargo $* > /tmp/ch-cargo.log 2>&1; rc=\$?; sed 's/\x1b\[[0-9;]*m//g' /tmp/ch-cargo.log | grep -E '^(error|warning)' -A18 | head -${LINES_MAX:-160}; tail -4 /tmp/ch-cargo.log | sed 's/\x1b\[[0-9;]*m//g'; echo CARGO_EXIT=\$rc"

#!/usr/bin/env bash
set -euo pipefail
printf 'rustup' >> "${OVERDRIVE_FAKE_COMMAND_LOG}"
printf '\t%s' "$@" >> "${OVERDRIVE_FAKE_COMMAND_LOG}"
printf '\n' >> "${OVERDRIVE_FAKE_COMMAND_LOG}"
dir="${CARGO_TARGET_DIR}/bpfel-unknown-none/release"
mkdir -p "${dir}"
printf host > "${dir}/overdrive-bpf"
printf guest > "${dir}/overdrive-guest-bpf"

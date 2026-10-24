#!/usr/bin/env bash
set -euo pipefail
printf 'bpftool' >> "${OVERDRIVE_FAKE_COMMAND_LOG}"
printf '\t%s' "$@" >> "${OVERDRIVE_FAKE_COMMAND_LOG}"
printf '\n' >> "${OVERDRIVE_FAKE_COMMAND_LOG}"
printf 'struct task_struct {};\n'

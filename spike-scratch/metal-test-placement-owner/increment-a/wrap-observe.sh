#!/usr/bin/env bash
# V-10 observer: runs where the repo's wrapper runs (nextest's direct child),
# records its own parent, then execs the repo's wrapper with the same argv.
# Written to the run-owned runtime dir and to stderr.
set -u
parent_comm=$(cat /proc/$PPID/comm 2>/dev/null)
parent_exe=$(readlink /proc/$PPID/exe 2>/dev/null)
gp=$(awk '{print $4}' /proc/$PPID/stat 2>/dev/null)
gp_comm=$(cat /proc/$gp/comm 2>/dev/null)
line="OWNERPROBE_WRAPPER test=${NEXTEST_TEST_NAME:-?} wrapper_pid=$$ wrapper_ppid=$PPID parent_comm=$parent_comm parent_exe=$parent_exe grandparent_pid=$gp grandparent_comm=$gp_comm uid=$(id -u)"
echo "$line" >&2
if [ -n "${OVERDRIVE_METAL_RUN_ID:-}" ] && [ -d "/run/ovd-gk/$OVERDRIVE_METAL_RUN_ID" ]; then
  mkdir -p -m 0700 "/run/ovd-gk/$OVERDRIVE_METAL_RUN_ID/shared" 2>/dev/null
  echo "$line" >> "/run/ovd-gk/$OVERDRIVE_METAL_RUN_ID/shared/wrapper-observe.log" 2>/dev/null
fi
exec "$NEXTEST_WORKSPACE_ROOT/../../../infra/metal/test-wrapper.sh" "$@"

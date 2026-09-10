#!/usr/bin/env bash
# Append-only local command capture for the issue-283 investigation.
set -euo pipefail
[[ $# -ge 2 ]] || { echo 'usage: capture-vm-lifecycle-283.sh LABEL COMMAND...' >&2; exit 2; }
label="$1"
shift
[[ "$label" =~ ^[a-zA-Z0-9_-]+$ ]] || exit 2
mkdir -p .context
capture_dir=$(mktemp -d ".context/issue283-${label}.XXXXXX")
{
  printf 'started_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'head=%s\n' "$(git rev-parse HEAD)"
  printf 'command='; printf '%q ' "$@"; printf '\n'
  git status --short
} >>"$capture_dir/meta"
git diff --binary HEAD >"$capture_dir/source.diff"
tar -czf "$capture_dir/diagnostic-sources.tgz" \
  docs/analysis/native-vm-lifecycle-283.sh \
  docs/analysis/capture-vm-lifecycle-283.sh \
  docs/analysis/root-cause-analysis-vm-lifecycle-latency-283.md \
  crates/overdrive-sim/tests/vm_lifecycle_latency_283_spike.rs
printf 'capture_dir=%s\n' "$capture_dir"
set +e
"$@" > >(tee "$capture_dir/stdout") 2> >(tee "$capture_dir/stderr" >&2)
result=$?
wait
set -e
{
  printf 'finished_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'exit=%s\n' "$result"
} >>"$capture_dir/meta"
printf 'capture_dir=%s exit=%s\n' "$capture_dir" "$result"
exit "$result"

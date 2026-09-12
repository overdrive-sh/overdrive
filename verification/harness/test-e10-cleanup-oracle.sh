#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly ROOT
readonly RUNNER="$ROOT/verification/expectations/E10-vm-service-http-cross-driver-status/runner.sh"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/e10-cleanup-oracle.XXXXXX")"
readonly TMP_ROOT
trap 'rm -rf -- "$TMP_ROOT"' EXIT HUP INT TERM

write_snapshot() {
  local path="$1"
  local cgroup="${2:-}"
  local network="${3:-}"
  local materialization="${4:-present}"
  local mount="${5:-}"
  local allocation_hypervisor="${6:-}"
  {
    echo '[hypervisors]'
    echo '[cgroup-scopes]'
    [[ -z "$cgroup" ]] || echo "$cgroup"
    echo '[vm-run-directories]'
    echo '[network]'
    [[ -z "$network" ]] || echo "$network"
    echo '[loop-devices]'
    echo '[mounts]'
    [[ -z "$mount" ]] || echo "$mount"
    echo '[materialization]'
    echo "$materialization"
    echo '[allocation-hypervisors]'
    [[ -z "$allocation_hypervisor" ]] || echo "$allocation_hypervisor"
  } >"$path"
}

validate_resource_case() {
  local cell="$1"
  EXPECTATION_DIR="$ROOT/verification/expectations/E10-vm-service-http-cross-driver-status" \
    REPO_ROOT="$ROOT" EVIDENCE_DIR="$TMP_ROOT/evidence" \
    E10_VALIDATE_RESOURCE_ONLY=1 E10_CELL="$cell" \
    E10_DRIVER=exec E10_STATUS=302 "$RUNNER"
}

make_case() {
  local name="$1"
  local current_cgroup="$2"
  local current_network="$3"
  local runtime_cgroup="$4"
  local observed_count="$5"
  local runtime_network="${6:-}"
  local final_mount="${7:-}"
  local current_hypervisor="${8:-}"
  local runtime_hypervisor="${9:-}"
  local final_hypervisor="${10:-}"
  local cell="$TMP_ROOT/$name"
  install -d "$cell"
  write_snapshot "$cell/current-session-resources.log" \
    "$current_cgroup" "$current_network" present '' "$current_hypervisor"
  write_snapshot "$cell/post-runtime-cleanup-resources.log" \
    "$runtime_cgroup" "$runtime_network" present '' "$runtime_hypervisor"
  write_snapshot "$cell/cleanup-resources-after.log" \
    "" "" absent "$final_mount" "$final_hypervisor"
  {
    echo 'allocation=alloc-service-exec-http-302-0'
    [[ -z "$current_cgroup" ]] \
      || echo 'cgroup=alloc-service-exec-http-302-0.scope'
    [[ -z "$current_network" ]] || echo 'network=ovd-hv-0000'
    [[ -z "$current_hypervisor" ]] || echo "hypervisor=$current_hypervisor"
  } >"$cell/current-session-owned-resources.log"
  printf 'E10 named cleanup: observed-before-stop=%s post-stop=absent serve=stopped preparation=absent\n' \
    "$observed_count" >"$cell/cleanup-resource-complement.log"
  printf '%s' "$cell"
}

# CONTRACT_SHAPE: bounded-change. Cleanup accepts already-absent and observed-
# then-absent owned resources, and rejects every retained owned resource.
clean_without_positive_delta="$(make_case clean-without-positive-delta '' '' '' 0)"
validate_resource_case "$clean_without_positive_delta"

observed_then_clean="$(make_case observed-then-clean \
  alloc-service-exec-http-302-0.scope \
  'ovd-hv-0000@if12 UP 00:00:00:00:00:00' '' 2)"
validate_resource_case "$observed_then_clean"

residual="$(make_case residual \
  alloc-service-exec-http-302-0.scope \
  'ovd-hv-0000@if12 UP 00:00:00:00:00:00' \
  alloc-service-exec-http-302-0.scope 2)"
if validate_resource_case "$residual"; then
  echo 'E10 cleanup oracle accepted a residual allocation cgroup' >&2
  exit 1
fi

residual_network="$(make_case residual-network \
  '' 'ovd-hv-0000@if12 UP 00:00:00:00:00:00' '' 1 \
  'ovd-hv-0000@if99 UP 00:00:00:00:00:00')"
if validate_resource_case "$residual_network"; then
  echo 'E10 cleanup oracle accepted a residual observed network name' >&2
  exit 1
fi

residual_mount="$(make_case residual-mount '' '' '' 0 '' \
  '/srv/vm/overdrive-testing/svm-e08 /dev/loop9')"
if validate_resource_case "$residual_mount"; then
  echo 'E10 cleanup oracle accepted a residual materialization mount' >&2
  exit 1
fi

residual_hypervisor="$(make_case residual-hypervisor \
  '' '' '' 1 '' '' 77 88 '')"
if validate_resource_case "$residual_hypervisor"; then
  echo 'E10 cleanup oracle accepted a replacement allocation hypervisor' >&2
  exit 1
fi

# CONTRACT_SHAPE: bounded-change. Probe preservation permits only a monotone
# observation timestamp; failure semantics and every other field stay exact.
readonly PROBE_BEFORE='  startup probe[0] http GET http://0.0.0.0:18080/ready last=fail (HTTP 302 (redirect not followed)) last_observed_at=100'
readonly PROBE_AFTER='  startup probe[0] http GET http://0.0.0.0:18080/ready last=fail (HTTP 302 (redirect not followed)) last_observed_at=101'
EXPECTATION_DIR="$ROOT/verification/expectations/E10-vm-service-http-cross-driver-status" \
  REPO_ROOT="$ROOT" EVIDENCE_DIR="$TMP_ROOT/evidence" \
  E10_VALIDATE_PROBE_ONLY=1 E10_PROBE_BEFORE="$PROBE_BEFORE" \
  E10_PROBE_AFTER="$PROBE_AFTER" "$RUNNER"

if EXPECTATION_DIR="$ROOT/verification/expectations/E10-vm-service-http-cross-driver-status" \
  REPO_ROOT="$ROOT" EVIDENCE_DIR="$TMP_ROOT/evidence" \
  E10_VALIDATE_PROBE_ONLY=1 E10_PROBE_BEFORE="$PROBE_BEFORE" \
  E10_PROBE_AFTER="${PROBE_AFTER/HTTP 302/HTTP 404}" "$RUNNER"; then
  echo 'E10 probe oracle accepted changed failure semantics' >&2
  exit 1
fi

if EXPECTATION_DIR="$ROOT/verification/expectations/E10-vm-service-http-cross-driver-status" \
  REPO_ROOT="$ROOT" EVIDENCE_DIR="$TMP_ROOT/evidence" \
  E10_VALIDATE_PROBE_ONLY=1 E10_PROBE_BEFORE="$PROBE_BEFORE" \
  E10_PROBE_AFTER="${PROBE_AFTER/last_observed_at=101/last_observed_at=99}" "$RUNNER"; then
  echo 'E10 probe oracle accepted a regressive observation timestamp' >&2
  exit 1
fi

echo 'E10 cleanup oracle: already-clean and observed-removal PASS; cgroup/network/hypervisor/mount residuals REJECTED; volatile timestamp PASS; semantic change and timestamp regression REJECTED'

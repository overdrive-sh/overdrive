#!/usr/bin/env bash
# Diagnostic behavioral regression for the existing E09-v2 failure predicate.
# <!-- DES-ENFORCEMENT : exempt -->
# Native fixtures: run.log at SHA b653e1ad1758d11be33be457849b284f78141333,
# 2026-09-08T19:12:00Z. Stream excerpts retain their first command exit status.
# The recovered snapshots are explicitly final-describe captures after a stop
# REQUEST, still Running; they are not mislabeled first-attempt descriptions.
# No product execution; no expectation evidence is emitted. Only a private
# describe-query adapter and logical polling clock replace external effects.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DIAG_DIR="$(mktemp -d "${TMPDIR:-/tmp}/e09-lifecycle.XXXXXX")"
trap 'rm -rf -- "$DIAG_DIR"' EXIT
source "$ROOT/examples/service-kind-vm-workloads-v2/run-example.sh" check-source

cat >"$DIAG_DIR/typed-stream" <<'NATIVE_CAPTURE'
Script started on 2026-09-08 19:13:59+00:00 [COMMAND="env OVERDRIVE_CONFIG_DIR=/srv/vm/overdrive-testing/svm-e08-v2/config /home/ubuntu/overdrive/target/debug/overdrive deploy /srv/vm/overdrive-testing/svm-e08-v2/cases/6/failure-service.toml" <not executed on terminal>]
Error: workload 'service-e09-v2-c006-f' did not converge to stable.
  reason: startup probe[0] failed after 3 attempts: connection refused
  reproducer: overdrive workload describe service-e09-v2-c006-f

Hint: see workload describe for full context

Script done on 2026-09-08 19:15:00+00:00 [COMMAND_EXIT_CODE="1"]
NATIVE_CAPTURE

cat >"$DIAG_DIR/failed-describe" <<'NATIVE_CAPTURE'
Service 'service-e09-v2-c006-f' (kind: Service)
Spec digest: b6f54eec0ebe7ce8d495ba840fc0403747f5a42a00fb6abb1983153b530639d7
Replicas (desired/running): 1/0
Alloc                    State        Restarts   Since               
alloc-service-e09-v2-c006-f-0 Failed       0          (c=70,w=local)      
    reason: driver started
Memory:        134217728
VIP:           10.96.0.17
Listeners:
  18081/tcp
Probes:
  startup probe[0] tcp 0.0.0.0:18999 last=fail (connection refused) last_observed_at=1788894900066
NATIVE_CAPTURE

cat >"$DIAG_DIR/recovered-describe" <<'NATIVE_CAPTURE'
Service 'service-e09-v2-c006-f' (kind: Service)
Spec digest: b6f54eec0ebe7ce8d495ba840fc0403747f5a42a00fb6abb1983153b530639d7
Replicas (desired/running): 1/1
Alloc                    State        Restarts   Since               
alloc-service-e09-v2-c006-f-0 Running      1          (c=71,w=local)      
    reason: driver started
    last terminated: Failed at (c=70,w=local) — driver started
    last terminated ran since: 1788894868.809421226
Memory:        134217728
Addresses:
  alloc-service-e09-v2-c006-f-0: 10.99.128.22
VIP:           10.96.0.17
Listeners:
  18081/tcp
Issued certificates:
  serial:        5f352140422d44ee55fbf5ca1642da72
    spiffe_id:     spiffe://overdrive.local/workload/service-e09-v2-c006-f/alloc/alloc-service-e09-v2-c006-f-0
    issuer_serial: 7e5c1620d243ea00c23706d10dbb5308
    not_after:     1788898447.106845192
Probes:
  startup probe[0] tcp 0.0.0.0:18999 last=fail (connection refused) last_observed_at=1788895109646
NATIVE_CAPTURE

cat >"$DIAG_DIR/bind-stream" <<'NATIVE_CAPTURE'
Script started on 2026-09-08 19:14:23+00:00 [COMMAND="env OVERDRIVE_CONFIG_DIR=/srv/vm/overdrive-testing/svm-e08-v2/config /home/ubuntu/overdrive/target/debug/overdrive deploy /srv/vm/overdrive-testing/svm-e08-v2/cases/8/failure-service.toml" <not executed on terminal>]
Error: workload 'service-e09-v2-c008-f' did not converge to stable.
  reason: startup probe[0] failed after 3 attempts: connection refused
  reproducer: overdrive workload describe service-e09-v2-c008-f

Hint: see workload describe for full context

Script done on 2026-09-08 19:15:09+00:00 [COMMAND_EXIT_CODE="1"]
NATIVE_CAPTURE

cat >"$DIAG_DIR/bind-describe" <<'NATIVE_CAPTURE'
Service 'service-e09-v2-c008-f' (kind: Service)
Spec digest: c75bb411a77d81c5f1a5771bd73dfdd3e179365b14016403aa5ef3182070b7db
Replicas (desired/running): 1/1
Alloc                    State        Restarts   Since               
alloc-service-e09-v2-c008-f-0 Running      1          (c=79,w=local)      
    reason: driver started
    last terminated: Failed at (c=73,w=local) — driver internal error: bind beacon listener: Address already in use (os error 98)
    last terminated ran since: 1788894895.620723020
    last terminated detail: bind beacon listener: Address already in use (os error 98)
Memory:        134217728
Addresses:
  alloc-service-e09-v2-c008-f-0: 10.99.128.30
VIP:           10.96.0.19
Listeners:
  18081/tcp
Issued certificates:
  serial:        a8a68dd8cec766dd283c27fc95678152
    spiffe_id:     spiffe://overdrive.local/workload/service-e09-v2-c008-f/alloc/alloc-service-e09-v2-c008-f-0
    issuer_serial: 7e5c1620d243ea00c23706d10dbb5308
    not_after:     1788898451.060568197
Probes:
  startup probe[0] tcp 0.0.0.0:18999 last=fail (connection refused) last_observed_at=1788895109592
NATIVE_CAPTURE

cat >"$DIAG_DIR/timeout-stream" <<'NATIVE_CAPTURE'
Script started on 2026-09-08 19:12:57+00:00 [COMMAND="env OVERDRIVE_CONFIG_DIR=/srv/vm/overdrive-testing/svm-e08-v2/config /home/ubuntu/overdrive/target/debug/overdrive deploy /srv/vm/overdrive-testing/svm-e08-v2/cases/1/failure-service.toml" <not executed on terminal>]
Error: workload 'service-e09-v2-c001-f' did not converge to stable.
  reason: workload did not converge within 90s
  reproducer: overdrive workload describe service-e09-v2-c001-f

Hint: see workload describe for full context

Script done on 2026-09-08 19:14:27+00:00 [COMMAND_EXIT_CODE="1"]
NATIVE_CAPTURE

cat >"$DIAG_DIR/timeout-describe" <<'NATIVE_CAPTURE'
Service 'service-e09-v2-c001-f' (kind: Service)
Spec digest: 74f09d64f98fb68ca34fb5888cada1f4d2e543295cd060dac969454d3561e282
Replicas (desired/running): 1/1
Alloc                    State        Restarts   Since               
alloc-service-e09-v2-c001-f-0 Running      1          (c=71,w=local)      
    reason: driver started
    last terminated: Failed at (c=70,w=local) — driver started
    last terminated ran since: 1788894862.820232557
Memory:        134217728
Addresses:
  alloc-service-e09-v2-c001-f-0: 10.99.128.2
VIP:           10.96.0.12
Listeners:
  18081/tcp
Issued certificates:
  serial:        160acf2807ba5e2cffcc0d487c36492c
    spiffe_id:     spiffe://overdrive.local/workload/service-e09-v2-c001-f/alloc/alloc-service-e09-v2-c001-f-0
    issuer_serial: 7e5c1620d243ea00c23706d10dbb5308
    not_after:     1788898441.135492330
Probes:
  startup probe[0] tcp 0.0.0.0:18999 last=fail (connection refused) last_observed_at=1788895045548
NATIVE_CAPTURE

# CONTRACT_SHAPE: bounded-change. The production predicate reads the same
# immutable stream and describe fixture each poll; advancing only SECONDS makes
# its existing 180s polling bound immediate without changing its logic.
check_case() (
  local name="$1" stream="$2" fixture="$3" expected="$4" observed
  assert_serve_identity() { :; }
  query_describe() { cp "$fixture" "$2"; }
  sleep() { SECONDS=$((SECONDS + 181)); }
  if wait_for_service_failure fixture "$DIAG_DIR/$stream" "$DIAG_DIR/query" "$DIAG_DIR/errors"; then
    observed=accepted
  else
    observed=rejected
  fi
  printf '%s: expected=%s observed=%s\n' "$name" "$expected" "$observed"
  [[ "$observed" == "$expected" ]]
)

# These cases intentionally stay separate: accepting no-bind recovery must not
# relax the original stream's typed failure requirement or accept mere timeout.
sed '/last terminated:/d' "$DIAG_DIR/recovered-describe" >"$DIAG_DIR/generic-running-describe"
awk '!inserted && /^Memory:/ {
  print "alloc-service-e09-v2-c006-f-1 Running      1          (c=72,w=local)"
  print "    last terminated: Failed at (c=71,w=local) — driver started"
  inserted=1
} { print }' "$DIAG_DIR/generic-running-describe" >"$DIAG_DIR/other-allocation-history-describe"
cp "$DIAG_DIR/typed-stream" "$DIAG_DIR/stable-stream"
printf 'Service is stable after a generic startup error\n' >>"$DIAG_DIR/stable-stream"
sed 's/startup probe\[0\] failed/internal driver error/' \
  "$DIAG_DIR/typed-stream" >"$DIAG_DIR/generic-error-stream"
failed=0
check_case failed_control typed-stream "$DIAG_DIR/failed-describe" accepted || failed=1
check_case bind_recovery_control bind-stream "$DIAG_DIR/bind-describe" accepted || failed=1
check_case original_c001_timeout_control timeout-stream "$DIAG_DIR/timeout-describe" rejected || failed=1
check_case no_bind_recovery_regression typed-stream "$DIAG_DIR/recovered-describe" accepted || failed=1
check_case generic_running_rejected typed-stream "$DIAG_DIR/generic-running-describe" rejected || failed=1
check_case other_allocation_history_rejected typed-stream \
  "$DIAG_DIR/other-allocation-history-describe" rejected || failed=1
check_case stable_stream_rejected stable-stream "$DIAG_DIR/failed-describe" rejected || failed=1
check_case generic_error_rejected generic-error-stream "$DIAG_DIR/failed-describe" rejected || failed=1
exit "$failed"

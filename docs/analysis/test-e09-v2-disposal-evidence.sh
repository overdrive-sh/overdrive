#!/usr/bin/env bash
# Post-fix native disposal evidence probe, capture 2026-09-08T20:14:18Z.
# <!-- DES-ENFORCEMENT : exempt -->
# Tests the actual helper against retained final snapshots; these are explicitly
# EXIT-trap retry results, not asserted to be first-call failing observations.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DIAG_DIR="$(mktemp -d "${TMPDIR:-/tmp}/e09-disposal.XXXXXX")"
trap 'rm -rf -- "$DIAG_DIR"' EXIT
source "$ROOT/examples/service-kind-vm-workloads-v2/run-example.sh" check-source
mkdir -p "$DIAG_DIR/8"
cat >"$DIAG_DIR/8/original-stream" <<'NATIVE_CAPTURE'
Script started on 2026-09-08 20:17:05+00:00 [COMMAND="env OVERDRIVE_CONFIG_DIR=/srv/vm/overdrive-testing/svm-e08-v2/config /home/ubuntu/overdrive/target/debug/overdrive deploy /srv/vm/overdrive-testing/svm-e08-v2/cases/8/failure-service.toml" <not executed on terminal>]
Error: workload 'service-e09-v2-c008-f' did not converge to stable.
  reason: startup probe[0] failed after 3 attempts: connection refused
  reproducer: overdrive workload describe service-e09-v2-c008-f

Hint: see workload describe for full context

Script done on 2026-09-08 20:17:20+00:00 [COMMAND_EXIT_CODE="1"]
NATIVE_CAPTURE
cat >"$DIAG_DIR/8/failure-describe.out" <<'NATIVE_CAPTURE'
Service 'service-e09-v2-c008-f' (kind: Service)
Spec digest: c75bb411a77d81c5f1a5771bd73dfdd3e179365b14016403aa5ef3182070b7db
Replicas (desired/running): 1/0
Alloc                    State        Restarts   Since               
alloc-service-e09-v2-c008-f-0 Failed       0          (c=84,w=local)      
    reason: driver started
Memory:        134217728
VIP:           10.96.0.13
Listeners:
  18081/tcp
Probes:
  startup probe[0] tcp 0.0.0.0:18999 last=fail (connection refused) last_observed_at=1788898639833
NATIVE_CAPTURE
cat >"$DIAG_DIR/8/stop-fixture" <<'NATIVE_CAPTURE'
Workload 'service-e09-v2-c008-f' was already stopped (no-op).
Endpoint: https://127.0.0.1:7644/
NATIVE_CAPTURE
cat >"$DIAG_DIR/8/final-fixture" <<'NATIVE_CAPTURE'
Service 'service-e09-v2-c008-f' (kind: Service)
Spec digest: c75bb411a77d81c5f1a5771bd73dfdd3e179365b14016403aa5ef3182070b7db
Replicas (desired/running): 1/0
Alloc                    State        Restarts   Since               
alloc-service-e09-v2-c008-f-0 Terminated   1          (c=99,w=local)      
    reason: stopped
    last terminated: Failed at (c=85,w=local) — driver internal error: bind beacon listener: Address already in use (os error 98)
    last terminated ran since: 1788898635.644696432
    last terminated detail: bind beacon listener: Address already in use (os error 98)
Memory:        134217728
VIP:           10.96.0.13
Listeners:
  18081/tcp
Probes:
  startup probe[0] tcp 0.0.0.0:18999 last=fail (timeout after 1s) last_observed_at=1788898768381
NATIVE_CAPTURE
mkdir -p "$DIAG_DIR/9"
cat >"$DIAG_DIR/9/original-stream" <<'NATIVE_CAPTURE'
Script started on 2026-09-08 20:17:05+00:00 [COMMAND="env OVERDRIVE_CONFIG_DIR=/srv/vm/overdrive-testing/svm-e08-v2/config /home/ubuntu/overdrive/target/debug/overdrive deploy /srv/vm/overdrive-testing/svm-e08-v2/cases/9/failure-service.toml" <not executed on terminal>]
Error: workload 'service-e09-v2-c009-f' did not converge to stable.
  reason: startup probe[0] failed after 3 attempts: connection refused
  reproducer: overdrive workload describe service-e09-v2-c009-f

Hint: see workload describe for full context

Script done on 2026-09-08 20:17:20+00:00 [COMMAND_EXIT_CODE="1"]
NATIVE_CAPTURE
cat >"$DIAG_DIR/9/failure-describe.out" <<'NATIVE_CAPTURE'
Service 'service-e09-v2-c009-f' (kind: Service)
Spec digest: 477086a5031632c99b0bce72f3904f7be02841e089600f33ddb0ad42eb1903b3
Replicas (desired/running): 1/0
Alloc                    State        Restarts   Since               
alloc-service-e09-v2-c009-f-0 Failed       0          (c=84,w=local)      
    reason: driver started
Memory:        134217728
VIP:           10.96.0.12
Listeners:
  18081/tcp
Probes:
  startup probe[0] tcp 0.0.0.0:18999 last=fail (connect failed: NetworkUnreachable: Network is unreachable (os error 101)) last_observed_at=1788898640183
NATIVE_CAPTURE
cat >"$DIAG_DIR/9/stop-fixture" <<'NATIVE_CAPTURE'
Workload 'service-e09-v2-c009-f' was already stopped (no-op).
Endpoint: https://127.0.0.1:7644/
NATIVE_CAPTURE
cat >"$DIAG_DIR/9/final-fixture" <<'NATIVE_CAPTURE'
Service 'service-e09-v2-c009-f' (kind: Service)
Spec digest: 477086a5031632c99b0bce72f3904f7be02841e089600f33ddb0ad42eb1903b3
Replicas (desired/running): 1/0
Alloc                    State        Restarts   Since               
alloc-service-e09-v2-c009-f-0 Terminated   1          (c=99,w=local)      
    reason: stopped
    last terminated: Failed at (c=85,w=local) — driver internal error: bind beacon listener: Address already in use (os error 98)
    last terminated ran since: 1788898625.932058449
    last terminated detail: bind beacon listener: Address already in use (os error 98)
Memory:        134217728
VIP:           10.96.0.12
Listeners:
  18081/tcp
Probes:
  startup probe[0] tcp 0.0.0.0:18999 last=fail (connection refused) last_observed_at=1788898778585
NATIVE_CAPTURE
mkdir -p "$DIAG_DIR/10"
cat >"$DIAG_DIR/10/original-stream" <<'NATIVE_CAPTURE'
Script started on 2026-09-08 20:17:05+00:00 [COMMAND="env OVERDRIVE_CONFIG_DIR=/srv/vm/overdrive-testing/svm-e08-v2/config /home/ubuntu/overdrive/target/debug/overdrive deploy /srv/vm/overdrive-testing/svm-e08-v2/cases/10/failure-service.toml" <not executed on terminal>]
Error: workload 'service-e09-v2-c010-f' did not converge to stable.
  reason: startup probe[0] failed after 3 attempts: connection refused
  reproducer: overdrive workload describe service-e09-v2-c010-f

Hint: see workload describe for full context

Script done on 2026-09-08 20:17:21+00:00 [COMMAND_EXIT_CODE="1"]
NATIVE_CAPTURE
cat >"$DIAG_DIR/10/failure-describe.out" <<'NATIVE_CAPTURE'
Service 'service-e09-v2-c010-f' (kind: Service)
Spec digest: 9259c41c5576e3ff1eabe974ac1292d6e2109aad11e57a236f1205374c25a0c9
Replicas (desired/running): 1/0
Alloc                    State        Restarts   Since               
alloc-service-e09-v2-c010-f-0 Failed       0          (c=85,w=local)      
    reason: driver started
Memory:        134217728
VIP:           10.96.0.17
Listeners:
  18081/tcp
Probes:
  startup probe[0] tcp 0.0.0.0:18999 last=fail (connection refused) last_observed_at=1788898641024
NATIVE_CAPTURE
cat >"$DIAG_DIR/10/stop-fixture" <<'NATIVE_CAPTURE'
Workload 'service-e09-v2-c010-f' was already stopped (no-op).
Endpoint: https://127.0.0.1:7644/
NATIVE_CAPTURE
cat >"$DIAG_DIR/10/final-fixture" <<'NATIVE_CAPTURE'
Service 'service-e09-v2-c010-f' (kind: Service)
Spec digest: 9259c41c5576e3ff1eabe974ac1292d6e2109aad11e57a236f1205374c25a0c9
Replicas (desired/running): 1/0
Alloc                    State        Restarts   Since               
alloc-service-e09-v2-c010-f-0 Terminated   1          (c=99,w=local)      
    reason: stopped
    last terminated: Failed at (c=86,w=local) — driver internal error: bind beacon listener: Address already in use (os error 98)
    last terminated ran since: 1788898636.829693115
    last terminated detail: bind beacon listener: Address already in use (os error 98)
Memory:        134217728
VIP:           10.96.0.17
Listeners:
  18081/tcp
Probes:
  startup probe[0] tcp 0.0.0.0:18999 last=fail (connection refused) last_observed_at=1788898791379
NATIVE_CAPTURE

# CONTRACT_SHAPE: bounded-change. The real helper consumes native original
# failure observations and the retained post-stop description. External adapters
# cannot alter the predicate. No successful helper result is called native proof.
check_retained() (
  local trial="$1" expected="$2" observed
  local case_dir="$DIAG_DIR/$trial"
  has_startup_probe_failure "$case_dir/original-stream"
  grep -Fq 'COMMAND_EXIT_CODE="1"' "$case_dir/original-stream"
  ! grep -Fq ' is stable ' "$case_dir/original-stream"
  bounded() { cat "$case_dir/stop-fixture"; }
  query_describe() { cp "$case_dir/final-fixture" "$2"; }
  assert_serve_identity() { :; }
  sleep() { SECONDS=$((SECONDS + 61)); }
  if dispose_failed_service "service-e09-v2-c$(printf '%03d' "$trial")-f" "$case_dir"; then
    observed=accepted
  else
    observed=rejected
  fi
  printf 'c%03d retained disposal: expected=%s observed=%s\n' "$trial" "$expected" "$observed"
  [[ "$observed" == "$expected" ]]
)

expected=accepted
if [[ "${1:-}" == --test-retained-rejection-hypothesis ]]; then expected=rejected; fi
failed=0
for trial in 8 9 10; do check_retained "$trial" "$expected" || failed=1; done
exit "$failed"

# Slice 05 — Liveness probe consecutive fails trigger Service restart

**Stories:** US-05
**Priority:** P3
**KPI:** K3 (Liveness-restart effectiveness: consecutive failures past threshold → RestartAllocation within 1 tick)
**Dependencies:** Slice 04 (readiness pathway proves the continuous-probe shape), Slice 01

## Outcome the operator can verify

Ana declares `[[health_check.liveness]] type = "http", path = "/healthz", failure_threshold = 3`. Workload's `/healthz` starts returning 503 at T0. At T0 + 3×interval, the reconciler emits `Action::RestartAllocation { reason: LivenessExhausted }`. New alloc has `restart_count = 1`.

## Adds onto Slice 04

| Component | Change |
|---|---|
| TOML parser | Accept `[[health_check.liveness]]` with body + `failure_threshold: u32` (default 3) |
| `ServiceLifecycleReconciler.View` | Persist `consecutive_failures_per_probe: BTreeMap<ProbeIdx, u32>` (inputs, NOT derived deadline) |
| `Action::RestartAllocation` reason | Extend reason enum with `LivenessExhausted { probe_idx, consecutive_failures, threshold }` |
| Restart budget | Reuses existing RESTART_BACKOFF_CEILING; eventual `BackoffExhausted` preserved |

## Acceptance test additions

- 3 consecutive liveness fails → restart emitted within 1 tick
- Recovery before threshold resets counter
- 5 liveness-driven restarts → eventual `Failed { BackoffExhausted { attempts: 5 } }`
- Liveness probe on Job/Schedule → parse error (Slice 07 covers this)

## Demoable check

Integration test with controllable HTTP server fixture that returns 503 on demand; asserts `restart_count` increments and eventual exhaustion.

## Out of scope

Per-probe restart budget (uses global); graceful drain on liveness restart (uses existing kill-and-respawn).

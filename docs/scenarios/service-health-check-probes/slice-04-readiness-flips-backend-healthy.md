# Slice 04 — Readiness probe flips Backend.healthy in dataplane fingerprint

**Stories:** US-04
**Priority:** P2
**KPI:** K2 (Dataplane-health convergence: readiness Fail → Backend.healthy = false within 1 reconciler tick)
**Dependencies:** Slice 01

## Outcome the operator can verify

A 3-backend `payments` Service has an HTTP readiness probe. Backend 2's `/healthz` starts returning 503. Within one reconciler tick the dataplane fingerprint changes (asserted via the existing `fingerprint_is_sensitive_to_health_flag` test pattern at `crates/overdrive-core/src/dataplane/fingerprint.rs:148`).

## Adds onto Slice 01

| Component | Change |
|---|---|
| TOML parser | Accept `[[health_check.readiness]]` with same body shape as startup |
| `ServiceLifecycleReconciler` | New reconcile branch: `ServiceBackendRow.healthy = (latest_readiness_status == Pass)` per alloc per tick |
| Initial state | `Backend.healthy = false` until first readiness Pass (avoids inverse race) |
| Default behaviour for Services without readiness | All backends `healthy = true` post-Stable (backward compat) |

## Acceptance test additions

- Readiness Pass → Fail flips `Backend.healthy` within 1 tick; fingerprint changes
- Readiness Fail → Pass restores `Backend.healthy = true` within 1 tick
- Service without readiness probe declared has all backends `healthy = true` post-Stable
- Readiness flapping (Fail/Pass/Fail/Pass) DOES NOT trigger restart (restart is liveness, Slice 05)

## Demoable check

Multi-replica integration test that simulates a single backend returning 503; assertion on dataplane fingerprint diff.

## Out of scope

Liveness restart (Slice 05); cross-replica health policies; per-region health.

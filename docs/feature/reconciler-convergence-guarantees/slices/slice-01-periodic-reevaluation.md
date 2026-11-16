# Slice 01 — periodic-reevaluation

Stories: US-01. Persona: Ana. Job: J-OPS-003. Order: 1.

## Goal

A deploy recovers by itself after a transient hydrate error, a View-write error, or a `Failed` result inside a backoff window. Every registered reconciler is periodically re-evaluated for every target in desired ∪ actual.

## IN

- Periodic re-evaluation for every registered reconciler, over targets enumerated from intent AND observation (D4), not `alloc_status` rows alone.
- The hydrate-error and View-write-error paths no longer leave a target without a future evaluation (`reconciler_runtime.rs:1588-1589`, `:1638-1641`).
- `service-map-hydrator` (no wake hooks today) is re-evaluated without waiting for an unrelated backend-row write.
- A target with intent and no `alloc_status` row is covered.
- Bounded cost: a target that has left desired and actual stops being evaluated.

## OUT

- Removing any View latch (slice 02 removes the hydrator's by deletion).
- Waking `workflow-lifecycle` for its own sake (not production-reachable, D6, D-8).
- Migrating `EnqueueEvaluation` wakes to `interests()` (#271); warm reflector cache (#270).
- A separate retry-with-backoff for failed ticks (the issue considered and dropped it; the periodic pass supersedes it).
- Boot-only host infra (#197, #198, #199, #234).

## Learning hypothesis

Disproves if: a periodic whole-set pass costs more than today's router relist at the same workload count (G1; V-3 measured it not above the baseline), or it cannot be reconciled with the row-backed/host-backed partition in ADR-0084 without user-approved amendment. Confirms if: seeded runs recover from all three fault stimuli within one period with zero non-`Noop` actions at the fixed point.

## Acceptance criteria

From US-01:

- AC1 Hydrate error (`ObservationRead`, `IntentRead`; `RunningAllocationWithoutWorkloadAddr` at Tier 1 only): `workload describe` shows Running and the service map is programmed within one period of the fault clearing, no second `deploy`.
- AC2 `persist_view` error: same oracle; no action dispatched on the failing tick. Tier 1, fault injected through `SimViewStore::inject_fsync_failure`.
- AC3 `Failed` dataplane update inside backoff: retried after backoff with no backend-row write.
- AC4 Target enumeration covers intent ∪ observation, including a target with no `alloc_status` row.
- AC5 Converged state: zero non-`Noop` actions; per-pass cost no greater than today's relist (G1). The scan cost that makes both quadratic is tracked by #270, deploy latency during a pass by #357.
- AC6 Tier-1 seeded `overdrive-sim` invariant for the three stimuli; seed printed on failure.

## Dependencies

None. Interacts with #271 and #270 (cited). The wakeup-model change is ADR-0296 (D-1, approved).

## Effort

About 3 days after the design decision. Highest design risk of the four slices.

## Spike

V-3 measured cost on native metal (feature delta).

## Production-drivability

The periodic pass runs in the real `run_server` composition. Faults are injected at existing `Sim*` port seams at Tier 1 (user ruling 2026-11-16): store read errors, a `persist_view` failure (`SimViewStore::inject_fsync_failure`), a `Failed` dataplane result, and `RunningAllocationWithoutWorkloadAddr`, which no production path produces (the shim writes `workload_addr` on every `Running` row).

Scope limitation, tracked: the Tier-1 `persist_view` recovery assumes the store can write again. With redb a transient disk error is permanent until restart (the `Database` is poisoned and `RedbViewStore` never reopens). Tracked by #360.

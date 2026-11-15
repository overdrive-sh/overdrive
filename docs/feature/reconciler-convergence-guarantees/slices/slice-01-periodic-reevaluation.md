# Slice 01 — periodic-reevaluation

Stories: US-01. Persona: Ana. Job: J-OPS-003. Order: 1.

## Goal

A deploy recovers by itself after a transient hydrate error, a View-write error, or a `Failed` result inside a backoff window. Every registered reconciler is periodically re-evaluated for every target in desired ∪ actual.

## IN

- Periodic re-evaluation for every registered reconciler, over targets enumerated from intent AND observation (D4), not `alloc_status` rows alone.
- The hydrate-error and View-write-error paths no longer leave a target without a future evaluation (`reconciler_runtime.rs:1589-1590`, `:1638-1642`).
- `service-map-hydrator` (no wake hooks today) is re-evaluated without waiting for an unrelated backend-row write.
- A target with intent and no `alloc_status` row is covered.
- Bounded cost: a target that has left desired and actual stops being evaluated.

## OUT

- Removing any View latch (slices 02, 03).
- Waking `workflow-lifecycle` for its own sake (not production-reachable, D6; O3).
- Migrating `EnqueueEvaluation` wakes to `interests()` (#271); warm reflector cache (#270).
- A separate retry-with-backoff for failed ticks (the issue considered and dropped it; the periodic pass supersedes it).
- Boot-only host infra (#197, #198, #199, #234).

## Learning hypothesis

Disproves if: a periodic whole-set pass cannot meet the 3% duty-cycle guardrail at 1000 targets, or it cannot be reconciled with the row-backed/host-backed partition in ADR-0084 without user-approved amendment. Confirms if: seeded runs recover from all three fault stimuli within one period with zero non-`Noop` actions at the fixed point.

## Acceptance criteria

From US-01:

- AC1 Hydrate error (`ObservationRead`, `IntentRead`, `RunningAllocationWithoutWorkloadAddr`): through `serve` + `deploy`, `workload describe` shows Running and the service map is programmed within one period of the fault clearing, no second `deploy`.
- AC2 `persist_view` error: same oracle; no action dispatched on the failing tick.
- AC3 `Failed` dataplane update inside backoff: retried after backoff with no backend-row write.
- AC4 Target enumeration covers intent ∪ observation, including a target with no `alloc_status` row.
- AC5 Converged state: zero non-`Noop` actions; at most 3% duty cycle at 1000 targets.
- AC6 Tier-1 seeded `overdrive-sim` invariant for the three stimuli; seed printed on failure.

## Dependencies

None. Interacts with #271 and #270 (cited). DESIGN requires an ADR-0084 amendment, surfaced to the user first (`design.md`). Resolve O1 and O5 before DESIGN.

## Effort

About 3 days after the design decision. Highest design risk of the four slices.

## Spike

None required up front. DESIGN decides whether the duty-cycle guardrail needs a measurement spike before the period and enumeration approach are approved (`design.md` § "Run contract-shaping validation").

## Production-drivable fault

A store read error or `persist_view` failure injected at the existing driven port while `serve` runs and `deploy payments.toml` is submitted; `RunningAllocationWithoutWorkloadAddr` is reachable from data alone. Tier 3 stimulus named in DISTILL.

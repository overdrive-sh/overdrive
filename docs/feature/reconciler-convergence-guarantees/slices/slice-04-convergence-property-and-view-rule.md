# Slice 04 — convergence-property-and-view-rule

Stories: US-05, US-06 (@infrastructure, shares this slice) and the executor doc fix (D-5). Persona: Devon. Jobs: J-PLAT-004. Order: last.

The VIP-release story moved out of this feature to GH #211 (user ruling D-5, 2026-11-13). This feature does not wait on #211. `service-lifecycle` is unchanged and scoped in the property as D-6 states.

## Goal

Every registered reconciler runs under one shared multi-tick property from arbitrary state, and the rule in `.claude/rules/reconcilers.md` states what a View may decide, so a record-on-emit latch fails mechanically instead of in review. The `release_service_vip` executor doc says what the executor does.

## IN

- Shared default-lane property for each reconciler registered in `run_server` (7, or 8 with the gateway): arbitrary independent `(desired, actual, view)` (the View generator includes `View::default()`, the forgotten View), multiple ticks, random dropped actions; assert progress (`actual == desired` within N ticks) and stability (no further non-`Noop` actions).
- `service-lifecycle` scoping (feature delta D-6): its probe-driven stop obligation is excluded from the drop/skew progress oracle, because with a dropped stop, or a marker equal to the latest crossed row's `run_seq`, it emits nothing until a newer probe row arrives (ADR-0191 decisions 5 and 6; bounded by the `specs/quint/probe-count` checks). Its generators draw markers only from rows present. Its other obligations stay under the oracle.
- Companion property: executor idempotency (apply twice equals apply once on observable state; US-05 S4, from issue #351).
- The property drives the production `run_convergence_tick` over a sim-composed `AppState` (D-7); no new sim executor.
- Correct the false hydrator DST claim ("any combination of BPF map state", `invariants/service_map_hydrator.rs:9`, `invariants/mod.rs:453`).
- US-06 rule text per D9 in `reconcilers.md`, replacing the fingerprint-shaped symptom. The rule names no exception.
- Correct the `release_service_vip.rs` doc comment that says errors are absorbed; the executor returns `ShimError::AllocatorRelease`.
- DISTILL authors full test bodies; scenarios for later steps stay as reasoned `#[ignore = "pending DELIVER ..."]`.

## OUT

- New test lanes beyond this shared property.
- A monotonicity checklist for LWW reads (#344).
- Real-kernel stress or soak tests.
- Reconcilers' own scripted scenario tests (unchanged).
- Replacing the `released_for_deletion` gate and the seeded reproduction of the possible VIP leak: GH #211.
- `observed_generation` (#325); `service-lifecycle`'s `terminal_announced` and act-once markers (not changed, D-6).
- Editing stale test fixtures.

## Learning hypothesis

Disproves if: arbitrary `(actual, view)` generation produces unreachable states that make the property fail on compliant reconcilers (too many false positives). Confirms if: reintroducing a record-on-emit latch fails the property with reconciler name, seed and tick.

## Acceptance criteria

- AC1 Every registered reconciler covered (7, or 8 with the gateway; `gateway-svid-lifecycle` is registered only when `config.gateway` is `Some`), or a reasoned no-obligation ruling per exempt one.
- AC2 Default lane, no real I/O, seed printed, failure reproducible from seed (lane feasibility is V-5).
- AC3 Fault stimulus = dropped actions and View/actual skew in both directions; oracle = convergence within N ticks, then zero actions (`service-lifecycle` scoped as in IN).
- AC4 Mutation check: DISTILL names the mutants from the live reconcilers; each turns the property red.
- AC5 Rule text merged with the pacing-input test (a marker keyed to an observed input suppresses re-emission only for that same input) and the "make it observable first" clause.
- AC6 Rule is timeless discipline (no ADR-as-status, no DELIVER notes).
- AC7 The executor doc comment matches behaviour (it propagates `ShimError::AllocatorRelease`; it does not absorb the error).

## Dependencies

Green status depends on slice 02; the bodies are authored up front. Reachable-state bound for generators follows the appliance threat model (prefixes of Overdrive's own action sequences, including crash-partial ones, and View/actual skew), not foreign mutation.

## Effort

About 3 to 4 days (the doc fix is minutes).

## Spike

No.

## Production-drivable fault

The property drives each reconciler's real `reconcile` and hydration through `Sim*` ports. The tie to `serve` + `deploy` comes from slice 01's faults, which the property reintroduces as regressions. The doc fix has no runtime behaviour. This slice has no operator-visible surface of its own (stated in US-05's DoR gap).

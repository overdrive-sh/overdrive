# Slice 04 — convergence-property-and-view-rule

Stories: US-05, US-06 (@infrastructure, shares this slice). Persona: Devon. Job: J-PLAT-004. Order: 4.

## Goal

Every registered reconciler runs under one shared multi-tick property from arbitrary state, and the rule in `.claude/rules/reconcilers.md` states what a View may decide, so a record-on-emit latch fails mechanically instead of in review.

## IN

- Shared default-lane property for each reconciler registered in `run_server` (8): arbitrary independent `(desired, actual, view)`, multiple ticks, random dropped actions; assert progress (`actual == desired` within N ticks) and stability (no further non-`Noop` actions).
- Companion properties: forgotten View (`View::default()` still converges); executor idempotency (apply twice equals apply once on observable state).
- A generic sim executor over action kinds, reusing the `vm_reclamation` `apply_actions` pattern (`overdrive-sim/src/invariants/vm_reclamation.rs:93-105`).
- Correct the false hydrator DST claim ("any combination of BPF map state", `invariants/service_map_hydrator.rs:9`, `invariants/mod.rs:453`) or make it true.
- US-06 rule text per D9 in `reconcilers.md`, replacing the fingerprint-shaped symptom, naming the one known exception (O7).
- DISTILL authors full test bodies; later-slice scenarios stay as reasoned `#[ignore = "pending DELIVER ..."]`.

## OUT

- New test lanes beyond this shared property.
- A monotonicity checklist for LWW reads (#344).
- Real-kernel stress or soak tests.
- Reconcilers' own scripted scenario tests (unchanged).

## Learning hypothesis

Disproves if: arbitrary `(actual, view)` generation produces unreachable states that make the property fail on compliant reconcilers (too many false positives). Confirms if: reintroducing either removed latch fails the property with reconciler name, seed and tick.

## Acceptance criteria

From US-05 and US-06:

- AC1 All 8 registered reconcilers covered, or a reasoned no-obligation ruling per exempt one.
- AC2 Default lane, no real I/O, seed printed, failure reproducible from seed.
- AC3 Fault stimulus = dropped actions and View/actual skew in both directions; oracle = convergence within N ticks, then zero actions.
- AC4 Mutation check: removing each fix from slices 02 and 03 turns the property red.
- AC5 Rule text merged with the "actual wins" test and the "make it observable first" clause.
- AC6 Rule is timeless discipline (no ADR-as-status, no DELIVER notes).

## Dependencies

Green status depends on slices 02 and 03; the bodies are authored up front. Reachable-state bound for generators follows the appliance threat model (prefixes of Overdrive's own action sequences, skew, restarts), not foreign mutation.

## Effort

About 3 days.

## Spike

No.

## Production-drivable fault

The property drives each reconciler's real `reconcile` and hydration through `Sim*` ports. The tie to `serve` + `deploy` comes from slices 02 and 03, whose faults the property reintroduces as regressions. This slice has no operator-visible surface of its own (stated in US-05's DoR gap).

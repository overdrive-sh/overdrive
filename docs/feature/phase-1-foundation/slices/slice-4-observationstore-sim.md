# Slice 4 — ObservationStore Trait + SimObservationStore LWW

**Story**: US-04
**Estimated effort**: ≤ 1 day
**Walking-skeleton member**: yes (backbone row 3)

## Hypothesis

If LWW semantics produce different outcomes under reordering, DST loses determinism and the whitepaper's §4 Intent/Observation split is unprovable as code.

## What ships

- `ObservationStore` trait in `overdrive-core::traits`: `read`, `write`, `subscribe` — typed over observation-row shapes, distinct from `IntentStore`
- `SimObservationStore` in a new `overdrive-sim` crate: in-memory LWW, logical timestamps, injectable gossip delay, injectable partition
- Seeded test asserting identical trajectories across two runs
- Invariant `intent_never_crosses_into_observation` declared in `overdrive-sim::invariants`
- Compile-time test: `&dyn ObservationStore` cannot be substituted for `&dyn IntentStore`

## Demonstrable end-to-end value

Ana can write an `alloc_status` row on node A, watch gossip deliver it to B and C under a seeded delay, and confirm convergence is deterministic. The compiler rejects any attempt to write intent-shaped data into the observation store.

## Carpaccio taste tests

- **Real data**: real LWW merge semantics over typed row shapes.
- **Ships end-to-end**: the trait, the sim impl, and the invariant-enforcement plumbing land together.
- **Independent of Slice 3**: can run in parallel.

## Definition of Done (slice level)

- [ ] `ObservationStore` trait defined, distinct from `IntentStore` at the type level.
- [ ] `SimObservationStore` with LWW, injectable gossip delay and partition.
- [ ] Compile-time cross-trait-substitution test passing.
- [ ] Invariant `intent_never_crosses_into_observation` declared and testable.
- [ ] Seeded trajectory test passing (twin runs identical).

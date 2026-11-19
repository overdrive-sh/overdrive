# Slice 04 — The guest family has a smoke test and the native families are unaffected

Story: US-GBS-04. Job: J-PLAT-008. Family: the guest family of slice 01, plus the four native families as controls.

## Goal
The guest family keeps building and running between Metal benchmark runs through the same smoke variant every declaration carries, and the Metal integration run shows the native families unchanged.

## In scope
- A smoke variant for the guest family: one replicate per arm, no performance assertion, gated on `integration-tests`, placed and lint-clean (how it gets a guest: open point O5).
- A broken in-guest fixture fails the smoke test naming the benchmark.
- The build-time refusal test for `guest` replaced by tests that accept it and still refuse an unregistered substrate by name.
- Native smoke tests and committed native records run unchanged.

## Out of scope
Any performance assertion; making the smoke test a measurement; changing native smoke placements.

## Learning hypothesis
Disproves: that the existing smoke declaration works for a substrate whose body needs its own guest. If the smoke test must be nested or must skip the in-guest half, the declaration model needs a rule for guest families.

## Acceptance
- [ ] Guest family smoke test passes after one replicate per arm, asserting no timing.
- [ ] A broken fixture fails it, naming the benchmark.
- [ ] Native smoke tests pass unchanged.
- [ ] `cargo xtask placement-lint` accepts the new tests.
- [ ] The unregistered-substrate refusal still names the value.

## Dependencies
Slice 01; O5 settled by DESIGN.

## Estimate
1 day. Reference class: the parent's smoke-variant slice.

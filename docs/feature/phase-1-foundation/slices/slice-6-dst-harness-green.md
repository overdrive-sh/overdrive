# Slice 6 — turmoil DST Harness Green

**Story**: US-06
**Estimated effort**: ≤ 1 day (integration, not net-new logic — assumes Slices 1–5 have shipped)
**Walking-skeleton member**: yes (backbone row 5 — this slice IS the walking-skeleton integration point)

## Hypothesis

If invariants can't be expressed as `assert_always!` / `assert_eventually!` against the harness, the whitepaper §21 testing model needs revision before we build anything else.

## What ships

- `overdrive-sim` crate completed with the turmoil harness
- `cargo xtask dst` entry point
- Invariant catalogue: `single_leader` (stubbed topology), `intent_never_crosses_into_observation`, `snapshot_roundtrip_bit_identical`, `sim_observation_lww_converges`, `replay_equivalent_empty_workflow`, `entropy_determinism_under_reseed`
- Each invariant name is an enum variant in `overdrive-sim::invariants`
- Seeded reproduction: `--seed N` produces bit-identical trajectory
- `--only <INVARIANT>` narrows to one invariant
- Failure output: invariant name + seed + tick + host + cause + exact reproduction command
- Self-test: harness runs twice on the same seed and asserts identical trajectories
- CI step wired to fail on non-zero exit

## Demonstrable end-to-end value

This is the **acceptance gate** for Phase 1. On a clean clone, `cargo xtask dst` compiles, boots a 3-node sim cluster composing real `LocalStore` with every Sim* trait, runs the invariant catalogue, prints a seed, and returns green in under 60s. A deliberate red run prints a reproduction command that reproduces the failure bit-for-bit.

## Carpaccio taste tests

- **Real data**: real `LocalStore` + real invariant evaluations, not synthetic.
- **Ships end-to-end**: this slice glues every prior slice into a product that a human can run and trust.
- **Commercial proof-point**: if this is green, the §21 claim is no longer performative.

## Definition of Done (slice level)

- [ ] `cargo xtask dst` runs green on a clean clone, < 60s wall-clock.
- [ ] Seed printed on every run.
- [ ] `--seed N` reproduces bit-for-bit on the same SHA and toolchain.
- [ ] `--only <INVARIANT>` narrows and matches an enum variant.
- [ ] Failure output carries name, seed, tick, host, cause, reproduction command.
- [ ] CI blocks merges on DST red.
- [ ] Twin-run identity self-test passing.

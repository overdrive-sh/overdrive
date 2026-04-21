# Slice 3 — IntentStore Trait + LocalStore on Real redb

**Story**: US-03
**Estimated effort**: ≤ 1 day
**Walking-skeleton member**: yes (backbone row 2)

## Hypothesis

If snapshot round-trip is lossy, the non-destructive single→HA migration story in `commercial.md` is broken and the control-plane density argument cannot be the commercial margin driver it needs to be.

## What ships

- `IntentStore` trait in `overdrive-core::traits`: `get`, `put`, `delete`, `watch`, `txn`, `export_snapshot`, `bootstrap_from`
- `LocalStore` implementation wrapping real redb
- proptest: snapshot round-trip bit-identical for arbitrary store contents
- Cold-start micro-benchmark (< 50ms target)
- RSS probe asserting < 30MB on empty store

## Demonstrable end-to-end value

A Overdrive platform engineer can create a `LocalStore`, write job specs to it, round-trip snapshots, restore on a different path, and observe the whitepaper-claimed resource envelope. No mock — real redb on disk.

## Carpaccio taste tests

- **Real data**: real redb files on tmpfs; real snapshots; rkyv-archived bytes.
- **Ships end-to-end**: the trait and a concrete impl ship together; no "trait now, impl next week."
- **Commercial proof-point**: cold-start and RSS benches encode the `commercial.md` density claim as an assertion.

## Definition of Done (slice level)

- [ ] `IntentStore` trait defined.
- [ ] `LocalStore` implements every method against real redb.
- [ ] proptest bit-identical round-trip passing.
- [ ] Cold-start bench < 50ms.
- [ ] RSS probe < 30MB on empty store.
- [ ] Compile-time separation from `ObservationStore` (types distinct).

# ADR-0012 — Phase 1 server reuses `SimObservationStore` behind a wiring adapter for observation reads

## Status

Accepted. 2026-04-23.

## Context

DISCUSS Key Decision 8 flagged three candidate paths for the Phase 1
control-plane server's `ObservationStore` implementation:

- **(a)** Reuse `SimObservationStore` (shipped in `overdrive-sim`)
  via a wiring layer.
- **(b)** Build a new trivial in-process LWW map over the same trait
  (lives in `overdrive-control-plane`).
- **(c)** Ship a zero-row stub that always returns empty rows.

All three satisfy the walking-skeleton AC (zero rows returned until the
scheduler + node agent ship in `phase-1-first-workload`). The choice
is on coupling, Phase 2 upgrade path, and behavioural honesty.

`SimObservationStore` is currently part of `overdrive-sim` — class
`adapter-sim`, which is dev-profile-only in the existing workspace
policy (its role is the DST harness).

## Decision

**Phase 1 reuses `SimObservationStore` as the server's
`ObservationStore` implementation, behind a wiring adapter in
`overdrive-control-plane`. The sim crate's class label is widened from
strictly dev-profile to a runtime-importable dependency of the
control plane binary for Phase 1 only — with a structured plan to
replace it with the real `CorrosionStore` in Phase 2.**

Concretely:

- `crates/overdrive-sim/Cargo.toml` metadata stays `crate_class =
  "adapter-sim"` but the crate is listed as a *non-optional*
  dependency of `overdrive-control-plane` for Phase 1.
- The control-plane binary constructs `SimObservationStore::new(
  GossipProfile::single_node())` at startup — zero-latency, no
  partitions, deterministic LWW under logical timestamps with the
  local NodeId as writer.
- The crate `overdrive-sim` remains the DST harness's home in
  parallel — the same type is used in two different wirings (DST
  harness, real-server single-node). This is expected and correct:
  `SimObservationStore` is in-process LWW, which IS the right shape
  for a single-node Phase 1 cluster.
- **Phase 2 cutover**: when `CorrosionStore` lands (cr-sqlite +
  SWIM/QUIC), the control-plane binary swaps
  `Box::new(SimObservationStore::new(...))` for
  `Box::new(CorrosionStore::new(...))`. One trait-object swap;
  no handler changes. The trait surface is the contract — both
  implementations honour it.

### Class-label note

`overdrive-sim`'s `adapter-sim` class label is intended to mean "implements
ports for simulation and testing scenarios." A single-node production
walking skeleton is legitimately in that category — the "real" Corrosion
store is a distributed system we don't need for a 1-node cluster. No
ADR override of ADR-0003 (crate-class labelling) is required; the label
accurately describes the crate's behaviour in both uses. The
`adapter-real` class remains reserved for adapters that exercise real
kernel / network / filesystem primitives.

## Considered alternatives

### Alternative A — Build a new trivial in-process LWW map in `overdrive-control-plane`

**Rejected.** Would duplicate logic already shipped and tested in
`SimObservationStore`. Single-node, zero-latency LWW is exactly the
behaviour `SimObservationStore::new(GossipProfile::single_node())`
already provides. Duplication contradicts the reuse-analysis rule and
adds a surface the DST harness doesn't exercise.

### Alternative C — Zero-row stub

**Rejected.** The AC reads "NodeList returns zero rows in Phase 1
because no node agent has registered", not "NodeList returns zero rows
because the handler short-circuits." The empty-state honesty rule
(`nw-ux-emotional-design`) requires the reason for emptiness to be
*actual emptiness*, not a hard-coded lie. A zero-row stub also blocks
any future Phase 1 test that wants to seed a node-health row through
the `ObservationStore::write` surface (e.g. a control-plane-internal
heartbeat from the `noop-heartbeat` reconciler, ADR-0013).

### Alternative D — Leave the ObservationStore unwired; handlers return empty inline

**Rejected.** Same reason as Alternative C, stated differently — the
trait boundary exists for a reason; skipping it for Phase 1 means
the crafter has to wire it in later, and the wiring point is *exactly*
the place bugs hide.

## Consequences

### Positive

- One `ObservationStore` implementation to test, one DST adapter, one
  wiring path — the sim crate's type IS the Phase 1 adapter.
- Phase 2 cutover is a single trait-object swap.
- The server's behaviour under Phase 1 load (zero writers, empty reads)
  is the same behaviour DST has been exercising continuously since
  phase-1-foundation.
- No duplicate in-process LWW code.

### Negative

- `overdrive-sim` becomes a runtime dep of the control-plane binary
  for Phase 1, widening its "dev-profile" reach. Documented here;
  re-evaluated at Phase 2.
- Anyone expecting "sim = fake" will need to update the mental model.
  The documentation update in `brief.md` (this ADR's companion edit)
  clarifies.

### Quality-attribute impact

- **Maintainability — modularity**: neutral. The trait boundary is
  the modularity mechanism; the choice of implementation behind it is
  a wiring concern.
- **Maintainability — testability**: positive. DST and production
  Phase 1 share the same implementation — any bug in the impl shows
  up in both paths.
- **Performance efficiency — time behaviour**: positive. In-memory
  LWW is O(1) per row; no Phase 1 performance concern.

### Enforcement

- `overdrive-control-plane`'s `Cargo.toml` declares `overdrive-sim`
  as a non-optional dep for Phase 1. A comment flags "Phase 2: replace
  with `overdrive-corrosion-store` when it lands."
- The handler module imports `overdrive_core::ObservationStore` (the
  trait) and never names `SimObservationStore` outside the wiring
  module. Handlers operate on `&dyn ObservationStore` (or `Arc<dyn
  ObservationStore>`) only.
- A cargo-deny check (or equivalent) flags a future `overdrive-sim`
  dependency creeping into other non-control-plane adapter-real
  crates — the sim crate is reachable from control-plane and
  xtask/dst, nowhere else.

## References

- `docs/whitepaper.md` §4 (ObservationStore — Live Cluster Map)
- `docs/product/architecture/brief.md` §6 (Observation-store row
  shapes — Phase 1 minimal set)
- `docs/feature/phase-1-control-plane-core/discuss/wave-decisions.md`
  Key Decision 8
- `docs/feature/phase-1-control-plane-core/slices/slice-3-api-handlers-intent-commit.md`
- ADR-0003 (Core-crate labelling)
- ADR-0004 (Single `overdrive-sim` crate, not split)

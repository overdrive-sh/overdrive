# ADR-0168 — A design that adds or changes a concurrent, ordered or crash-sensitive protocol carries a model-checked Quint specification, kept permanently and used as the DISTILL conformance oracle

## Status

**Proposed — approved by user 2026-10-06; pending independent DESIGN
review.** Process decision. First applied to GH #295 (the guest-flow owner
protocol, feature delta § *[REF] vsock Attachment Replacement DESIGN*,
§ *Formal protocol model*).

## Context

Several Overdrive designs are protocols whose correctness depends on the order
of events, on concurrency between owners, or on a crash between two steps:
the guest-flow owner's pairing and control session, intake listener
bring-up and take-down against the host firewall, the boot order of firewall
rules and the shared route, lease and CID assignment under concurrent
placement, the workflow journal, reconciler write-through ordering.

Prose review does not reliably find defects in such designs. The #295 vsock
replacement design went through two review iterations and an independent
review whose 25 findings were all remediated. A Quint model of the
remediated text, model-checked with Apalache and TLC
(`docs/feature/netns-density-295/spike/quint-owner-findings.md`), then found:

- a revoke failure path that leaves a firewall element admitting connections
  to a wildcard host service, surviving quiescence, teardown and lease
  release;
- a stated guarantee ("a CID-clash retry gets a different CID unless every
  other offset is held") that is false under concurrent placement and after a
  restart;
- a kernel assumption (a dead VM's queued connections are reset before its
  CID is reused) that no validation item covered.

Each rule the design pins was also shown to be load-bearing: switching off any
one of them produced a counterexample.

Seeded `overdrive-sim` invariants (`.claude/rules/testing.md`, Tier 1) check
an implementation's trajectories. They come after the design is accepted and
cannot show that the design text itself is consistent. A defect in the design
is then implemented faithfully and found late, or not at all.

The flat-cluster intent-log spike
(`raft-corrosion-vs-gossip-log/spike-scratch/flat-cluster-intent-log/`,
increments a and b) is the precedent for the tooling: a Quint specification
with Apalache bounded checks and TLC exhaustive checks, hazard ("bug-flag")
instances, per-check evidence files, pinned tool versions, and a
quint-connect model-based test driving the implementation from the
specification's traces.

## Decision

A DESIGN that adds or changes a concurrent, ordered or crash-sensitive
protocol carries a formal specification in Quint, and:

1. **It is model-checked before independent DESIGN review.** Every safety
   invariant the design states is checked; every liveness property under the
   stated fairness. Each design rule the invariants depend on has a hazard
   variant that switches that rule off and is shown to violate its invariant,
   and each reachable behaviour the invariants quantify over has a
   non-vacuity witness. Bounds and instance sizes are recorded with every
   verdict.
2. **Model assumptions are explicit.** Every kernel, environment or
   third-party fact the specification assumes rather than models is listed and
   mapped to the proven fact or validation item that discharges it. An
   assumption with no discharge is surfaced to the user as an open validation
   item.
3. **A counterexample against the design text returns to DESIGN** and is
   resolved by a user decision before review proceeds.
4. **The specification is the DISTILL conformance oracle.** DISTILL drives the
   implementation from the specification's traces through quint-connect, at
   the seam the design names. This complements, and does not replace, the
   seeded `overdrive-sim` invariants of `.claude/rules/testing.md`.
5. **Specifications are permanent.** They live at
   `specs/quint/<subsystem>/` in the repository, next to their pinned tool
   versions, check scripts and retained evidence, and are not archived at
   FINALIZE. A later design change to the protocol updates the specification
   in the same DESIGN wave.

"Concurrent, ordered or crash-sensitive protocol" means: two or more actors
whose interleaving can change the outcome; a pinned order of steps whose
reordering is a defect; or a sequence whose correctness depends on what
survives a crash between two of its steps. A pure function, a single-owner
state machine with no crash window, or a wire format alone does not qualify.

## Alternatives considered

- **Prose-only DESIGN review.** Cheapest; it missed the three findings above
  after three review passes. Rejected.
- **TLA+ written directly.** The same checkers (TLC, Apalache) and a longer
  record, but a notation further from the Rust the crafters write, and no
  quint-connect path to drive the implementation from traces. Rejected.
- **Stateright or another model checker embedded in Rust.** Checks Rust code,
  so it can only run once the implementation exists; it does not check the
  design before review, and it ties the model to one implementation's
  structure. Rejected.
- **Specifications inside the crates.** Couples a design artifact to a crate's
  build and test graph, and to one crate when a protocol spans several (the
  owner, the guest init, the mTLS worker). Rejected.
- **Specifications under `docs/feature/<id>/`.** Archived with the feature at
  FINALIZE, so the oracle disappears exactly when the protocol becomes
  production code that later changes must keep conforming to. Rejected.

## Consequences

- DESIGN gains a step before independent review for qualifying designs, and
  its cost: writing and checking the specification, including hazard
  variants and witnesses.
- A new top-level `specs/quint/` tree, with its toolchain (Quint, Apalache,
  TLC, a JVM) pinned beside the specifications. The tooling runs on the Lima
  VM like other checks.
- A model encodes the design text at a chosen abstraction. It shows the design
  is consistent within its bounds; it does not show that an implementation
  conforms (that is the quint-connect oracle's job in DISTILL) or that its
  kernel assumptions hold (that is the validation items' job).
- Apalache depths are bounds and TLC results are exhaustive only for the
  finite instance checked; findings and reviews cite them as such.
- Model assumptions without a discharge become visible validation items
  instead of implicit premises.

# Slice 1 — First-fit scheduler scaffold

**Story**: US-01
**Walking skeleton row**: 1 (Place an allocation)
**Effort**: ~1 day
**Depends on**: phase-1-control-plane-core (`Node`, `Job`, `Resources` aggregates, `node_health_rows` row already shipped — Phase 1 single-node has exactly one row, written by the server at startup as an implementation detail).

## Outcome

A pure scheduler module — `schedule(nodes: &BTreeMap<NodeId, NodeView>, job: &JobView, current_allocs: &[AllocStatusRow]) -> Result<NodeId, PlacementError>` — lives in the control-plane crate (or in a new crate if DESIGN picks; the codebase research suggests `overdrive-control-plane::scheduler` as the lightweight default). Phase 1 is single-node, so the input map carries exactly one entry; the scheduler still iterates via BTreeMap (deterministic order is load-bearing for DST replay, even with N=1) and returns either `Ok(the_only_node_id)` if capacity covers the job's resources, or `Err(PlacementError::NoCapacity { needed, max_free })`. No taint logic; no async; no I/O; just a pure function exhaustively covered by proptest.

## Value hypothesis

*If* the scheduler is not expressible as a pure deterministic function over typed inputs, *then* DST cannot replay scheduler behaviour from a seed — every later DST invariant about scheduler determinism, capacity respect, or convergence becomes flakey or silently bogus. *Conversely*, if this slice ships, the convergence-loop's brain has known-deterministic behaviour and Slice 3's lifecycle reconciler can call it without crossing the purity boundary.

## Disproves (what's the named pre-commitment we're falsifying)

- **"The scheduler needs DB-backed memory or async I/O to do its job."** No — first-fit is a pure function over `(nodes_view, job, current_allocs)`. The runtime hydrates the inputs; the function decides; the caller dispatches.
- **"BTreeMap is heavy enough to want HashMap for performance."** No — Phase 1 has exactly one node; BTreeMap iteration cost is in the noise vs the determinism gain (per `.claude/rules/development.md`).
- **"With one node, we don't need a real scheduler — the reconciler can just pick that one node directly."** No — the placement function exists to be a known-shape pure function under DST replay. Even N=1 needs the deterministic predicate so Phase 2+ multi-node work is a content change, not a structural one.

## Scope (in)

- `schedule(nodes, job, allocs) -> Result<NodeId, PlacementError>` pure fn.
- `PlacementError` enum: `NoCapacity { needed, max_free }`, `NoHealthyNode`. (Taint variants out — Phase 1 single-node has no taint/toleration.)
- Capacity-accounting helper: `free_capacity(node, allocs) -> Resources`.
- Proptest covering same-inputs-same-output across input vec reorderings.
- `NodeView` / `JobView` projection types (DESIGN may collapse these into the existing aggregates).

## Scope (out)

- Taint matching logic (Phase 1 has no taints; out of scope — Phase 2+ when multi-node lands).
- Lifecycle reconciler invocation (Slice 3).
- Action emission — the scheduler returns `NodeId`, not `Action::StartAllocation`; the reconciler builds the action.
- Driver involvement (Slice 2).
- Multi-node placement strategy beyond first-fit (Phase 2+).

## Target KPI

- Determinism: calling `schedule(nodes, job, allocs)` twice with the same inputs returns the same `Result<NodeId, PlacementError>`.
- Capacity accuracy: a 10 GiB job submitted against a 4 GiB node returns `NoCapacity { needed: 10 GiB, max_free: 4 GiB }`.
- Behavioural: a 1 GiB job submitted against the single registered node with sufficient capacity returns `Ok(<the_node_id>)`.

## Acceptance flavour

See US-01 scenarios. Focus: pure-function determinism, capacity accounting under partial pre-allocation, deterministic ordering across input vec permutations (even with N=1, the proptest covers shapes that Phase 2+ will encounter).

## Failure modes to defend

- A NodeView with `Resources { cpu_milli: 0, memory_bytes: 0 }` — capacity check rejects without numeric underflow.
- An empty node set — returns `Err(NoHealthyNode)`. (Phase 1 invariant: the node_health table always has exactly one row at runtime, so this branch is operationally unreachable, but the proptest still covers it.)
- Job whose resource request is `Resources { cpu_milli: 0, memory_bytes: 0 }` — DESIGN decides whether this is rejected at the aggregate constructor (likely) or accepted by the scheduler trivially.

## Slice taste-test

| Test | Status |
|---|---|
| ≤4 new components | PASS — `schedule` fn, `PlacementError` enum, `free_capacity` helper (3) |
| No hypothetical abstractions landing later | PASS — uses existing aggregates |
| Disproves a named pre-commitment | PASS — see above |
| Production-data-shaped AC | PASS — proptest is the production-shape exercise of the pure function |
| Demonstrable in single session | PASS — proptest pass + a unit-test walkthrough |
| Same-day dogfood moment | PASS — proptest output is the demo |

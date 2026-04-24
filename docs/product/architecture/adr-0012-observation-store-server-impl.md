# ADR-0012 — Phase 1 server uses a real `LocalObservationStore` (redb-backed, single-writer)

## Status

Accepted. 2026-04-23. **Revised in place 2026-04-24 — see Revision
2026-04-24 below.** The original decision (reuse `SimObservationStore`
via a wiring adapter) is reversed; the ADR number is kept because the
decision scope is the same (which `ObservationStore` implementation
backs the Phase 1 server), only the answer changes.

## Context

DISCUSS Key Decision 8 flagged three candidate paths for the Phase 1
control-plane server's `ObservationStore` implementation:

- **(a)** Reuse `SimObservationStore` (shipped in `overdrive-sim`)
  via a wiring layer.
- **(b)** Build a new trivial in-process LWW map over the same trait
  (lives in `overdrive-control-plane`).
- **(c)** Ship a zero-row stub that always returns empty rows.

The original 2026-04-23 decision picked **(a)**, with the argument that
(i) `SimObservationStore::single_peer(...)` is already a correct
single-node LWW, (ii) duplicating that logic would contradict the reuse
rule, and (iii) the `adapter-sim` class "accurately describes the
crate's behaviour in both uses."

A fourth option — a real redb-backed `LocalObservationStore` living in
the `adapter-host` class, single-writer, no CRDT machinery — was not
seriously evaluated. During DELIVER Phase 1 step 03-03 finalisation
(honest empty reads + canary-row injection) the user objected to the
original decision on three grounds:

1. **No persistence.** `SimObservationStore` is in-memory. A process
   restart loses every row ever written. Phase 1 gets away with this
   today only because there are no real writers yet; the moment the
   Phase 2 scheduler or node agent commits a single `alloc_status` row
   to the live server, restart is silent data loss. The honest reads
   story from step 03-03 dies on first restart.
2. **A sim adapter in the production wiring path is a category error.**
   The `crate_class = "adapter-sim"` label (ADR-0003) exists precisely
   so `dst-lint` scans those crates differently — they are the
   legitimate home for `turmoil`, `StdRng`, in-memory LWW, and every
   other bit of machinery that makes DST possible. The whitepaper
   design principle 1 ("own your primitives") and the whole point of
   the class taxonomy is that a crate's class describes *what it is in
   production*, not what a reviewer happens to accept as "also correct
   for a single node." Using `overdrive-sim` as a runtime dep of the
   control-plane binary blurs that boundary at the exact layer it was
   put there to protect. The class-label rationalisation in the
   original ADR ("sim = simulation *and* single-node") was after-the-fact
   justification, not architecture.
3. **CRDT machinery for a single-writer use case is overhead.**
   `SimObservationStore` carries owner-writer site IDs, LWW logical
   timestamps, injectable gossip delay, injectable partition matrix,
   tombstone discipline, and a subscription surface shaped around
   gossip fan-out. Phase 1 has one writer (the local node agent, when
   it lands) and no peers. Every one of those mechanisms pays a cost
   (code, test surface, mental model) for a property Phase 1 does not
   have and cannot exercise.

The user additionally ratified the following framing: the Phase 2
cutover point is the `CorrosionStore` landing, *not* "make the sim
store persist." Phase 1 should ship a primitive that is correct for
single-writer-single-node in the same way `LocalStore` is correct for
single-node linearizable intent; Phase 2 swaps the trait-object at
`wire_single_node_observation` for `CorrosionStore`. The trait-object
seam is unchanged; the adapter behind it is honest about what it is.

## Decision

**Reverse the 2026-04-23 decision.** Phase 1 introduces a new real
adapter, `LocalObservationStore`, that backs the `ObservationStore`
trait with redb on disk and implements subscriptions via
`tokio::sync::broadcast`. It lives in the existing
`overdrive-store-local` crate (class `adapter-host`) alongside
`LocalStore`. The control-plane wiring (`wire_single_node_observation`)
constructs a `LocalObservationStore` instead of a
`SimObservationStore::single_peer(...)`.

`SimObservationStore` stays in `overdrive-sim` (class `adapter-sim`)
and remains the DST harness's implementation of the trait — that is its
legitimate role. The `overdrive-sim` dependency is removed from the
`overdrive-control-plane` runtime graph; it stays a dev-dependency for
the crate's own DST-shaped tests only.

### Crate placement — extend `overdrive-store-local`

The `LocalObservationStore` lands in `crates/overdrive-store-local/`
rather than a new `overdrive-store-observation` crate. Rationale:

- **Single on-disk discipline.** `overdrive-store-local` already owns
  the "real redb with `tokio::sync::broadcast` for change
  notifications" pattern end-to-end for intent (`LocalStore` +
  `redb_backend.rs`). The observation side is the same pattern applied
  to a different trait and a different on-disk table layout. One crate
  owns both, one set of redb handling idioms, one place reviewers
  look.
- **Co-location matches the single-node deployment model.** The
  whitepaper's `mode = "single"` row (§4) runs exactly these two stores
  on one node, writing to one data directory. Splitting them across
  two crates buys no isolation the class system doesn't already
  provide, and costs one more `Cargo.toml` and one more dep edge on
  the control-plane.
- **ADR-0011 is enforced by the trait surface, not by crate
  boundaries.** Intent vs observation non-substitutability is
  compile-time-enforced through distinct trait names, distinct row
  types, and the `tests/compile_fail/*.rs` fixtures. Putting
  `LocalObservationStore` in the same crate as `LocalStore` does not
  weaken that enforcement — the type system still rejects every
  cross-layer shortcut. (A second crate would be a *reminder*, not an
  *enforcer*.)
- **Phase 2+ growth path.** When `RaftStore` lands, it goes alongside
  `LocalStore` in the same crate (per the existing Phase 1 plan). When
  `CorrosionStore` lands, it goes in its own crate
  (`overdrive-store-observation-corrosion` or similar) because it
  carries its own heavy transitive graph (SWIM, QUIC, cr-sqlite). At
  that point `LocalObservationStore` stays where it is as the
  single-node alternative. The taxonomy is "small stores in
  `overdrive-store-local`, heavy distributed stores in their own
  crate" — consistent and easy to predict.

The alternative (new `overdrive-store-observation` crate) was
considered and rejected: no reviewer-visible benefit, one extra crate
in the workspace, extra coupling surface to document.

### `LocalObservationStore` design

Crate: `overdrive-store-local` (extended).
Class: `adapter-host` (production posture; `dst-lint` skips it, real
I/O allowed).
Module: `crates/overdrive-store-local/src/observation_backend.rs`
(adjacent to `redb_backend.rs`).
Public surface:

```rust
pub use observation_backend::LocalObservationStore;

pub use overdrive_core::traits::observation_store::{
    ObservationRow, ObservationStore, ObservationStoreError,
    ObservationSubscription,
    AllocStatusRow, NodeHealthRow, AllocState, LogicalTimestamp,
};
```

Backing store:

- **redb database file** at `<data_dir>/observation.redb` (one file
  per store instance, sibling to the intent `store.redb`).
- **Per-row-kind redb tables**, keyed on the row's natural primary key
  (not on a logical-timestamp composite, because there is no LWW in
  Phase 1 — see below):

  | redb table | key | value |
  |---|---|---|
  | `alloc_status` | `AllocationId` bytes | rkyv-archived `AllocStatusRow` |
  | `node_health` | `NodeId` bytes | rkyv-archived `NodeHealthRow` |

- Rows are serialised with rkyv, matching the project-wide
  deterministic-serialisation rule in `development.md`. Archived
  bytes are fully canonical; any future on-disk hash consumes them
  directly without a re-encoding step.
- Phase 2+ row shapes (service backends, compiled policy verdicts,
  revoked operator certs) become new tables as they are defined by
  the owning ADR.

Writes:

- `write(ObservationRow)` is a single redb write transaction per call.
  Phase 1 is single-writer, so no merge logic runs: an incoming
  `alloc_status` row for a given `AllocationId` **overwrites** the
  existing entry. Full-row writes only — the §4 guardrail (never
  field-diff merges) is preserved because the trait accepts only a
  `ObservationRow`, never a patch.
- After the redb commit succeeds, the row is fanned out on a
  per-instance `tokio::sync::broadcast::Sender<ObservationRow>` for
  any active subscribers.

Subscriptions:

- `subscribe_all()` returns an `ObservationSubscription` wrapping a
  `BroadcastStream<ObservationRow>` (same shape as `LocalStore::watch`
  per the existing Phase 1 substitute). Lagging subscribers get
  `RecvError::Lagged` — the stream wrapper terminates them and a
  future reconciler-side retry resubscribes from scratch, consistent
  with the intent-side contract.
- Channel capacity matches the intent side (1024 events) until
  measurement justifies a different number. No per-prefix filter in
  Phase 1 — `subscribe_all` is deliberately "every row this peer
  writes" per the trait docstring.

Reads:

- `alloc_status_rows()` iterates the `alloc_status` redb table under
  a single read transaction, deserialises each rkyv value, and
  returns a `Vec<AllocStatusRow>` ordered by `AllocationId` (the
  redb table key ordering — deterministic by construction).
- `node_health_rows()` is the same shape against the `node_health`
  table, ordered by `NodeId`.

No CRDT machinery, by design:

- No owner-writer site IDs. The `LogicalTimestamp.writer` field on a
  row is still carried for *row authenticity* — a Phase 2 `CorrosionStore`
  consumer needs it, and the control-plane hands the field through
  from whichever source produced the row — but `LocalObservationStore`
  does not reject writes from "the wrong" writer. Single-writer
  deployment makes the check meaningless; Phase 2 adds it back at the
  Corrosion layer.
- No LWW merge. Last write wins by the trivial "most recent redb
  transaction commits" rule. The `updated_at` field is preserved in
  the row (it is part of the row's schema) so Phase 2 gossip can
  honour it once peers exist; Phase 1's own overwrite semantics do
  not consult it.
- No tombstones. Phase 1 has no `delete` surface on the trait; row
  removal is deferred to the §18 sweeper reconcilers in a later
  phase.
- No gossip delay, partition matrix, or injectable fault knobs. Those
  are `SimObservationStore`'s job and live where they belong.

Restart semantics:

- Opening an existing `observation.redb` file reads every row back at
  startup — no warmup, no rehydrate-from-sim step. A row written
  before restart is returned by the first read handler call after
  restart. This is the specific failure mode objection (1) named
  above; the test case covering it is called out in the
  implementation plan below.

### Trait-object swap site — unchanged in shape

`overdrive-control-plane::observation_wiring::wire_single_node_observation`
keeps its current signature (`-> Result<Box<dyn ObservationStore>, ...>`).
The construction site changes:

```rust
// Before (this revision reverses):
let store = SimObservationStore::single_peer(node_id, SINGLE_NODE_SEED);
Ok(Box::new(store))

// After:
let path = data_dir.join("observation.redb");
let store = LocalObservationStore::open(&path)?;
Ok(Box::new(store))
```

Every caller (handlers, tests, future reconcilers) depends on
`&dyn ObservationStore` / `Arc<dyn ObservationStore>`. The Phase 2
cutover to `CorrosionStore` is a change to exactly this one construction
line — the same shape the original ADR promised, but now the path that
gets replaced is a real persistent store, not a sim.

### Class-label boundary — clarification

The 2026-04-23 ADR argued that `overdrive-sim`'s `adapter-sim` label
"accurately describes the crate's behaviour in both uses" (DST harness
+ single-node server). This revision explicitly rejects that framing:
`adapter-sim` means "implements ports for simulation / test harness
scenarios", full stop. Using a sim adapter in the production wiring
path undermines the class system's whole point — the class tells
reviewers *what the code is for*, not *what the code happens to
compute*. A future reviewer reading `observation_wiring.rs` should see
an `adapter-host` import on the construction line and immediately know
"this is real I/O, production posture"; they should not have to track
down the explanation of "yes, `SimObservationStore` is actually the
production impl in Phase 1." This revision restores that invariant.

## Considered alternatives (updated)

### Alternative A — Reuse `SimObservationStore` via wiring *(the 2026-04-23 decision, now reversed)*

**Rejected.** See the three objections enumerated in Context. Summary:
no persistence (silent data loss on restart once Phase 2 introduces
real writers); sim adapter in production wiring is a category error
against ADR-0003's class taxonomy; CRDT machinery (site IDs, LWW,
tombstones, gossip knobs) is overhead for a single-writer deployment.

### Alternative B — New trivial in-process LWW map in `overdrive-control-plane`

**Rejected.** Also in-memory, also no persistence — has the same
restart-loses-data problem as Alternative A and does not solve
objection (1). Adds duplicate LWW logic next to `SimObservationStore`'s
existing implementation, which this revision's approach is careful to
*avoid* (the real store has no LWW at all).

### Alternative C — Zero-row stub

**Rejected for the same reason as the original ADR** — the
empty-state-honesty rule requires reads to return "actual emptiness,"
not a hardcoded empty array. The canary-row tests in step 03-03 fall
down without a working write surface.

### Alternative D — Real `LocalObservationStore` in `overdrive-store-local` *(chosen)*

See Decision above. Concretely addresses all three objections:

1. **Persistence.** redb on disk; restart round-trip is a testable
   invariant.
2. **Class taxonomy.** `adapter-host` crate; `adapter-sim` is reserved
   for its legitimate DST role.
3. **No CRDT overhead.** Single-writer overwrite semantics; merge,
   site-ID checks, and tombstones land with the Phase 2 store that
   needs them.

### Alternative E — Real `LocalObservationStore` in a new `overdrive-store-observation` crate

**Rejected.** Considered explicitly. Same functional outcome as
Alternative D. Costs: one more workspace crate, one more `Cargo.toml`,
one more dep edge on `overdrive-control-plane`, one more class label
to document. Benefits: zero — intent/observation non-substitutability
is already compile-time-enforced by distinct traits and row types
(ADR-0011, `tests/compile_fail/*.rs`). Co-location with `LocalStore`
is the honest "single-node deployment runs both these stores over one
data directory" shape the whitepaper describes.

## Implementation plan

This is a mid-DELIVER reversal, not a clean greenfield decision. The
timeline is: Phase 1 control-plane slices 01–04 shipped against the
original ADR; the revision lands as a remediation step *before* the
Phase 1 control-plane-core feature closes its walking-skeleton gate.

### Remediation shape — new roadmap step (option A)

The work is a single self-contained step appended to the
`phase-1-control-plane-core` roadmap. In-place revision of steps
03-01 / 03-03 / 04-04 was considered (option B) and rejected: the
existing steps' ACs described the sim-wiring behaviour honestly at
the time they were written, their execution logs are historical
record, and re-litigating them muddies the audit trail. Treating the
reversal as a follow-up mini-feature (option C) was also considered
and rejected: the scope is one crate module plus one wiring swap,
not a feature-sized loop through discuss/distill/design/deliver.

**Proposed step:**

- **ID**: `03-06`
- **Name**: `local-observation-store-redb`
- **Scenario name**: `local_observation_store_persists_rows_across_restart`
- **Test file**:
  `crates/overdrive-store-local/tests/acceptance/local_observation_store.rs`
  (new Tier 3 acceptance test — real redb file I/O, gated by the
  crate's `integration-tests` feature per `.claude/rules/testing.md`)
  plus a control-plane-side smoke assertion under
  `crates/overdrive-control-plane/tests/integration/observation_empty_rows.rs`
  adjusted to run against the new store (the canary-row cases already
  written continue to pass unchanged — they drive through the same
  `ObservationStore::write` trait method).

**Acceptance criteria:**

1. **Write-then-read within a single process lifetime.**
   `LocalObservationStore::open(path)` returns a store. Writing one
   `AllocStatusRow` via the trait's `write(...)` method makes the row
   visible on the very next `alloc_status_rows()` call. Same for
   `NodeHealthRow` + `node_health_rows()`.
2. **Restart round-trip.** Open a store at `path`, write a row, drop
   the store, open a new `LocalObservationStore::open(path)` against
   the same file, call `alloc_status_rows()` — the row from the
   previous lifetime appears, bit-identical. This is the objection-(1)
   regression gate; it is the AC that kills the sim-adapter approach.
3. **Subscription delivery.** Opening a subscription via
   `subscribe_all()` and then writing a row delivers that row on the
   subscription stream within a bounded tokio poll. Subscriber
   closed before the write receives no event; subscriber opened
   after the write does not see the historical row (subscription is
   future-only, same contract as the intent-side `watch`).
4. **Overwrite on same key.** Writing a second `AllocStatusRow` for
   the same `AllocationId` replaces the first — `alloc_status_rows()`
   returns exactly one row, the second-write copy. (Validates the
   single-writer "no LWW, overwrite wins" semantics.)
5. **Phase 2 cutover seam intact.**
   `overdrive-control-plane::observation_wiring::wire_single_node_observation`
   keeps its `Box<dyn ObservationStore>` return type; the control-plane
   handler tests (`submit_round_trip`, `describe_round_trip`,
   `observation_empty_rows`, `cluster_info`) all pass unchanged after
   the construction line swaps. Asserted by running the full existing
   `crates/overdrive-control-plane/tests/integration/` suite green
   under the new wiring.
6. **Dep-graph hygiene.** `overdrive-control-plane/Cargo.toml` no
   longer depends on `overdrive-sim`. Asserted by a `cargo tree`
   spot-check in the step's execution log; long-term this is a
   `cargo-deny` rule that lands in a later hygiene step.

**Implementation scope:**

- *Production files*:
  - `crates/overdrive-store-local/src/observation_backend.rs` (new
    module — the `LocalObservationStore` type and its redb tables).
  - `crates/overdrive-store-local/src/lib.rs` (re-export
    `LocalObservationStore` and the trait surface).
  - `crates/overdrive-store-local/Cargo.toml` (add
    `overdrive-core::traits::observation_store` to the used module set;
    no new external deps — redb, tokio, rkyv are already present for
    `LocalStore`).
  - `crates/overdrive-control-plane/src/observation_wiring.rs` (swap
    construction line).
  - `crates/overdrive-control-plane/Cargo.toml` (remove
    `overdrive-sim` from `[dependencies]`; keep it under
    `[dev-dependencies]` if any existing tests reference
    `SimObservationStore` — a grep at implementation time settles
    whether it stays as a dev-dep or leaves entirely).
- *Test files*:
  - `crates/overdrive-store-local/tests/acceptance/local_observation_store.rs`
    (new; Tier 3 under `integration-tests` feature).
  - `crates/overdrive-store-local/tests/acceptance/` module wiring
    follows the existing `acceptance.rs` entrypoint convention per
    ADR-0005 / `.claude/rules/testing.md`.
- *No changes* to: `overdrive-core::traits::observation_store` (trait
  surface unchanged); `overdrive-sim::adapters::observation_store`
  (sim impl stays where it is); `overdrive-core::id` (no new newtypes);
  the handler modules (they see `&dyn ObservationStore` and care not
  which adapter is behind it).

**Tier:** Tier 3 (real redb file I/O; Tier 1 DST coverage is still
provided by `SimObservationStore` in `overdrive-sim`).

**Dependencies:**
- `03-01` (LocalStore + IntentStore over real redb — established the
  `overdrive-store-local` redb idioms this step reuses).
- `03-03` (AllocStatus + NodeList honest reads with canary-row
  injection — establishes the trait-surface contract the new store
  must honour; its tests are the "nothing regresses" gate).
- `04-04` (control-plane wiring finalisation — the construction-line
  swap site).

**Notes / instructions for the crafter:**
- The trait-object swap site is exactly one line in
  `observation_wiring.rs`. No handler changes, no API changes, no
  roadmap-wide refactor.
- `rkyv::Archive + rkyv::Serialize + rkyv::Deserialize` derives on
  `AllocStatusRow` and `NodeHealthRow` may need to be added if they
  are not already present — the trait currently defines them with
  `#[derive(Debug, Clone, PartialEq, Eq)]` only. Adding the rkyv
  derives is a one-line change per struct, scoped to
  `traits/observation_store.rs`.
- Phase 1 has no on-disk schema versioning for observation rows; the
  crafter records that fact in the step's execution log alongside
  the Phase 2+ migration note (same disposition as the intent-side
  single-table layout).
- The `overdrive-sim` dep removal from `overdrive-control-plane` is
  part of this step, not a follow-up: leaving the dep in place after
  the wiring stops using it is exactly the kind of latent coupling
  the reversal is meant to fix.

Roadmap-level metadata changes (step insert, `total_steps` bump,
`walking_skeleton_gate` update if needed) are **not** performed by
this ADR. The crafter handles `roadmap.json` edits in a follow-up
dispatch.

## Consequences

### Positive

- **Persistence survives restarts.** The honest-reads story from step
  03-03 extends to "honest reads across a restart", which is what any
  reviewer would assume the word "store" means.
- **Class taxonomy stays honest.** `overdrive-sim` goes back to being
  the DST harness + sim adapters, nothing else. `overdrive-control-plane`
  has only `adapter-host` crate dependencies in its production graph.
  Future reviewers reading `Cargo.toml` can trust that shape.
- **Simpler code.** `LocalObservationStore` ships without site IDs,
  LWW merge, tombstones, or gossip-delay knobs. The Phase 1 surface
  is smaller than `SimObservationStore`'s by a meaningful margin.
- **Phase 2 cutover seam unchanged.** The construction-line swap
  promise in the original ADR survives the revision — and now the
  thing being replaced is a real, persistent store, not an in-memory
  sim.
- **DST coverage undiminished.** The invariant suite in `overdrive-sim`
  (`SimObservationLwwConverges`, etc.) continues to run against
  `SimObservationStore`; that is the legitimate sim use the revision
  is careful to preserve.

### Negative

- **One more crate module to maintain.** `LocalObservationStore` is
  new code with its own test surface. Mitigated by keeping it small
  (no CRDT surface) and co-located with `LocalStore` (same redb
  idioms, same crate).
- **Two live observation-store implementations.** `LocalObservationStore`
  (production single-node) and `SimObservationStore` (DST) both
  implement the same trait. A trait-level contract change must
  cascade to both. This is the same situation as `IntentStore` already
  has (`LocalStore` + future `RaftStore` + `SimIntentStore`-equivalent)
  and is an expected cost of the ports-and-adapters topology.
- **Reversal is visible in the audit trail.** Step 03-03's execution
  log records canary-row tests against a sim store; step 03-06 adds
  the persistence gate the sim could not satisfy. A reviewer reading
  the history sees a correction, not a clean linear history. This is
  recorded honestly here rather than elided.

### Quality-attribute impact

- **Reliability — maturity**: positive. A store that survives
  restarts is the baseline any "state store" is expected to clear;
  shipping without that baseline was the original ADR's largest
  risk.
- **Maintainability — modularity**: positive. The class taxonomy
  stays a reliable reviewer signal; `adapter-sim` in the production
  graph no longer requires a parenthetical explanation.
- **Maintainability — testability**: neutral. DST coverage is
  unchanged; the new acceptance test in `overdrive-store-local` adds
  one restart round-trip case that `SimObservationStore` structurally
  could not have covered.
- **Performance efficiency — time behaviour**: neutral. redb ACID
  writes on a local SSD are microsecond-scale; the Phase 1 target
  "REST round-trip under 100 ms" is not affected by the store change.
  The subscribe fan-out stays in-process.

### Enforcement

- `overdrive-control-plane/Cargo.toml` lists `overdrive-store-local`
  (adapter-host) and not `overdrive-sim` under `[dependencies]`.
- The existing compile-fail fixtures under
  `crates/overdrive-core/tests/compile_fail/` continue to assert
  intent/observation non-substitutability; no new fixtures needed —
  the contract is unchanged.
- The new Tier 3 acceptance test's restart-roundtrip case is the
  regression gate for objection (1). A future refactor that
  accidentally re-introduces an in-memory-only store (or "cache
  rows in a HashMap and hope for the best") fails this test.

## Changelog

- **2026-04-23** — Original decision: Phase 1 server reuses
  `SimObservationStore` via a wiring adapter in `overdrive-control-plane`.
  (Accepted.)
- **2026-04-24** — Revised in place. Trigger: user objection during
  DELIVER Phase 1 control-plane-core step 03-03 finalisation. Three
  concrete objections (no persistence, sim adapter in production
  wiring is a category error against ADR-0003, CRDT overhead for
  single-writer). New decision: real `LocalObservationStore` in
  `overdrive-store-local` (class `adapter-host`), single-writer,
  redb-backed, `tokio::sync::broadcast` subscriptions, no LWW.
  Implementation lands as new roadmap step `03-06`. Trait-object
  swap seam at `wire_single_node_observation` is preserved; the
  Phase 2 cutover to `CorrosionStore` is unchanged in shape. ADR
  number kept (same decision scope, corrected answer). This is a
  revision, not a supersession.

## References

- `docs/whitepaper.md` §4 (ObservationStore — Live Cluster Map)
- `docs/product/architecture/brief.md` §4 (state-layer discipline),
  §6 (ObservationStore row shapes), §18 (this ADR's companion edit)
- `docs/feature/phase-1-control-plane-core/discuss/wave-decisions.md`
  Key Decision 8
- `docs/feature/phase-1-control-plane-core/slices/slice-3-api-handlers-intent-commit.md`
- ADR-0003 (Core-crate labelling)
- ADR-0004 (Single `overdrive-sim` crate, not split)
- ADR-0011 (Intent-side `Job` aggregate and observation-side
  `AllocStatusRow` stay separate types)
- ADR-0016 (`overdrive-host` crate extraction, `adapter-real` →
  `adapter-host` rename)
- ADR-0017 (`overdrive-invariants` crate — `SimObservationStore` stays
  the DST harness's implementation)
- `crates/overdrive-store-local/src/redb_backend.rs` (the existing
  redb + `tokio::sync::broadcast` idiom `LocalObservationStore`
  reuses)
- `crates/overdrive-control-plane/tests/integration/observation_empty_rows.rs`
  (the canary-row tests that drive through `ObservationStore::write`
  and continue to pass against the new store)
- `.claude/rules/testing.md` §"Integration vs unit gating" (Tier 3
  acceptance-test layout)

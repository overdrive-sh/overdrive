# ADR-0021 — Reconciler `State` shape: per-reconciler typed `AnyState` enum mirroring `AnyReconcilerView`

## Status

Accepted. 2026-04-27. Decision-makers: Morgan (proposing), user
ratification 2026-04-27. Tags: phase-1, first-workload,
application-arch.

## Context

The reconciler primitive in
`crates/overdrive-core/src/reconciler.rs` ships with an opaque
placeholder for `desired` / `actual`:

```rust
#[derive(Debug, Default)]
pub struct State;
```

ADR-0013 §2 (the `Reconciler` trait surface) and the existing
`NoopHeartbeat` reconciler treat `State` as opaque. That works for
Phase 1's single proof-of-life reconciler — `noop-heartbeat` never
dereferences either argument — but does not work for Phase 1's first
real reconciler. The `JobLifecycle` reconciler (US-03) needs to read
the desired `Job` aggregate, the set of running `AllocStatusRow`s,
and (for placement) the set of registered `Node` aggregates.
`pub struct State;` cannot be dereferenced; the Phase 1 first-workload
feature is blocked until the shape is decided.

The DISCUSS wave's `dor-validation.md` flagged this as the single
HARD design dependency that gates US-03. Three options were enumerated
in US-03 Technical Notes:

- **(a)** Generic / parameterised `State<D, A>` carrying typed
  projections.
- **(b)** Concrete struct with `BTreeMap<AllocationId, AllocStatusRow>`
  + `Option<Job>` plus other shapes the lifecycle reconciler needs.
- **(c)** Per-reconciler typed state matching the existing
  `AnyReconciler` / `AnyReconcilerView` enum-dispatch pattern, e.g.
  `enum AnyState { JobLifecycle(JobLifecycleState), … }`.

The `desired` and `actual` parameters share the load. Both are
projections of the same underlying stores — `desired` is
intent-derived (rkyv access against `IntentStore::get`); `actual` is
observation-derived (`ObservationStore::alloc_status_rows` etc.).
The runtime hydrates both before invoking `reconcile`, exactly the
pattern ADR-0013 §2 / §2b establish for the `view` parameter.

## Decision

### 1. Per-reconciler typed `AnyState` enum, mirroring `AnyReconcilerView`

```rust
// in overdrive-core::reconciler

/// Sum of every `desired`/`actual` shape consumed by a registered
/// reconciler. One variant per reconciler kind, exactly mirroring
/// `AnyReconciler` and `AnyReconcilerView`.
///
/// Phase 1 ships two variants: `Unit` for `NoopHeartbeat` (the proof-
/// of-life reconciler that does not dereference its state) and
/// `JobLifecycle` for the first real reconciler shipped by the
/// `phase-1-first-workload` feature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnyState {
    /// Carried by reconcilers whose `desired`/`actual` projections
    /// are degenerate. `NoopHeartbeat` uses this.
    Unit,
    /// Job-lifecycle reconciler's desired/actual projection. Carries
    /// the `Job` aggregate, the registered `Node` set, and the
    /// current `AllocStatusRow` set for the target job.
    JobLifecycle(JobLifecycleState),
}

/// Desired/actual projection consumed by `JobLifecycle::reconcile`.
/// Hydrated by the runtime from `IntentStore` (job + nodes) and
/// `ObservationStore` (allocations).
///
/// The same struct serves both `desired` and `actual` — Phase 1
/// keeps the projection symmetric. The reconciler interprets
/// `desired.job` as "what should exist" and `actual.allocations` as
/// "what is currently running"; future variants may diverge if a
/// different shape is genuinely required, but Phase 1's needs are
/// simple enough that one shared struct is honest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobLifecycleState {
    /// The target job. `None` if the desired-state read returned no
    /// row (job was deleted) or the actual-state read found no
    /// surviving row to project against.
    pub job: Option<Job>,
    /// Registered nodes with their declared capacity. Drives the
    /// scheduler input map. Phase 1 single-node has exactly one
    /// entry; the `BTreeMap` discipline holds at N=1.
    pub nodes: BTreeMap<NodeId, Node>,
    /// Current allocations belonging to this job, keyed by alloc id.
    /// Read from `ObservationStore::alloc_status_rows` filtered by
    /// `job_id`. Empty when no allocations yet exist.
    pub allocations: BTreeMap<AllocationId, AllocStatusRow>,
}
```

The `Reconciler::reconcile` signature becomes:

```rust
fn reconcile(
    &self,
    desired: &Self::State,   // was: &State
    actual:  &Self::State,   // was: &State
    view:    &Self::View,
    tick:    &TickContext,
) -> (Vec<Action>, Self::View);
```

`Self::State` is a new associated type on `Reconciler`, sister to
`type View`. `AnyReconciler::reconcile` widens its match arms exactly
the way `AnyReconciler::reconcile`'s view-dispatch already does:

```rust
match (self, desired, actual, view) {
    (Self::NoopHeartbeat(r), AnyState::Unit, AnyState::Unit,
     AnyReconcilerView::Unit) => { … }
    (Self::JobLifecycle(r), AnyState::JobLifecycle(d),
     AnyState::JobLifecycle(a), AnyReconcilerView::JobLifecycle(v)) => { … }
}
```

Compile-time exhaustiveness: a new reconciler variant whose `State`
or `View` does not have a matching `AnyState` / `AnyReconcilerView`
arm produces a non-exhaustive-match compile error, exactly the way
the view-dispatch already enforces.

### 2. `desired` and `actual` collapse into one struct per reconciler

The Phase 1 lifecycle reconciler does not need different *shapes*
for desired vs actual — it needs different *interpretations* of the
same fields. The struct (`JobLifecycleState`) is symmetric; the
reconciler reads `desired.job` as the spec, `actual.allocations` as
the running set. Making them different types would force the runtime
to hydrate two distinct projection trees and would force the
reconciler to translate between them.

Future variants may legitimately need divergent shapes (e.g. a
right-sizing reconciler whose `desired` is "target replica count"
and whose `actual` is "current memory pressure samples"). At that
point the variant introduces its own state type — the
`AnyState::RightSizing(RightSizingState)` arm need not be symmetric
internally. The design rule is "one State type per variant," not
"all variants must be symmetric."

### 3. Hydration owned by the runtime, not the reconciler

The runtime — not the reconciler — populates `desired` and `actual`.
Per ADR-0013 §2b the tick loop is:

```
1. Pick reconciler from registry by name           (enum dispatch)
2. Open (or reuse) LibsqlHandle for name           (path from ADR-0013 §5)
3. tick <- TickContext::snapshot(clock)            (ADR-0013 §2c)
4. desired <- runtime.hydrate_desired(self, target)  (NEW — async; runtime owns)
5. actual  <- runtime.hydrate_actual(self, target)   (NEW — async; runtime owns)
6. view    <- reconciler.hydrate(target, db).await   (per ADR-0013)
7. (actions, next_view) =
       reconciler.reconcile(&desired, &actual, &view, &tick)
8. Persist diff(view, next_view) to libsql
9. Dispatch actions to the action shim (see ADR-0023)
```

The runtime's `hydrate_desired` / `hydrate_actual` perform the
async reads against `IntentStore` and `ObservationStore` and emit
the matching `AnyState` variant. The reconciler's existing
`hydrate(target, db)` method retains its narrow remit (the libSQL
private-memory read) — it is NOT extended to read other stores.
This preserves the ADR-0013 hygiene that puts the reconciler
author in charge of *one* async surface (its own private DB) and
the runtime in charge of all the others.

The runtime's hydrators can be implemented variant-by-variant via
an inherent method on `AnyReconciler` (e.g.
`async fn hydrate_desired(&self, target, intent, obs) -> Result<AnyState, _>`)
that match-dispatches and calls the right typed loader. Phase 1's
two variants are a `NoopHeartbeat` arm (returns `AnyState::Unit`)
and a `JobLifecycle` arm (reads job + nodes + allocations and
returns `AnyState::JobLifecycle(…)`). New reconcilers add a new
arm.

## Alternatives considered

### Alternative A — Generic `State<D, A>`

```rust
pub struct State<D, A> { desired: D, actual: A }
```

The trait would carry two associated types (`type Desired`,
`type Actual`) and the runtime would dispatch through a generic
parameter set on `AnyReconciler`. Conceptually this is the most
"type-theoretic" answer: the State shape is fully erased to its
constituents.

**Rejected** for two reasons:

1. **Generics interact badly with the existing `AnyReconciler`
   enum-dispatch shape** (ADR-0013 §2a). Adding two type
   parameters per variant means the dispatch match becomes
   parameterised, and the trait-object alternative
   (`Box<dyn Reconciler<Desired=…, Actual=…>>`) re-introduces the
   object-safety break ADR-0013 §2a explicitly avoided. The fix
   is the same enum-erasure pattern we already use for `View` —
   which is option (c) below, dressed up with extra ceremony.
2. **No Phase 1 reconciler needs the divergence.** The lifecycle
   reconciler's desired/actual share fields by construction.
   Paying for fully decomposed types when no consumer benefits is
   premature ceremony, and ADR-0013 §2a's "the registry stores
   enum values directly" simplification is preserved by option
   (c) but lost by option (a).

### Alternative B — Concrete struct with all-of-everything

```rust
pub struct State {
    pub job: Option<Job>,
    pub nodes: BTreeMap<NodeId, Node>,
    pub allocations: BTreeMap<AllocationId, AllocStatusRow>,
    // … fields added per future reconciler …
}
```

A single `State` struct that grows fields as new reconcilers land.
`NoopHeartbeat` ignores everything; `JobLifecycle` reads the
relevant subset.

**Rejected** for two reasons:

1. **God-object pattern.** Every new reconciler adds fields the
   others ignore. Within Phase 2+ (cert-rotation, right-sizing,
   chaos-engineering reconcilers all in §18's "Built-in
   reconcilers" list) the struct accumulates a dozen
   reconciler-specific shapes that have nothing to do with each
   other. The compiler stops helping — a typo on a field name in
   `JobLifecycle::reconcile` reads `state.allocaitons` as
   `Option::None` (fine, no field with that name… wait, that's
   not how Rust works) but a future right-sizing reconciler
   author has no compile-time signal that they should *not* be
   reading `state.job` if the right-sizing variant doesn't carry
   one.
2. **The runtime has to populate every field on every tick.**
   Hydrating a `JobLifecycle` evaluation reads job + nodes +
   allocations; under option (b) it would also have to populate
   the right-sizing fields, the cert-rotation fields, etc., even
   though the reconciler about to run will not consume them. The
   per-tick async I/O budget grows linearly in the count of
   reconciler kinds, regardless of which reconciler is running.

### Alternative C — Per-reconciler typed state via `AnyState` (ACCEPTED)

The decision above. Variant per reconciler kind; runtime hydrates
the matching variant; reconciler dispatches against a typed
projection it owns.

**Accepted because**:

1. **Symmetric with the existing `View` story** (ADR-0013 §2a).
   `AnyReconcilerView` already does this for `View`; doing the
   same for `State` keeps the dispatch shape uniform. There is
   one mental model: "every reconciler kind has a typed View, a
   typed State, and a typed Action footprint, all enum-dispatched
   through `AnyReconciler`."
2. **Per-tick I/O scales with the running reconciler, not the
   registered set.** Hydrating a `JobLifecycle` evaluation reads
   job + nodes + allocations and nothing else; hydrating a
   future cert-rotation evaluation reads cert state and nothing
   else. The runtime's `hydrate_desired` / `hydrate_actual`
   match-dispatch on the variant before doing any I/O.
3. **Compile-time exhaustiveness.** A new reconciler variant
   that omits its `AnyState` arm fails to compile, exactly the
   way the existing `AnyReconcilerView` arms do. The compiler
   catches the omission at extension time, not at runtime.
4. **No object-safety break.** No `Box<dyn Reconciler<State=…>>`
   anywhere; everything goes through `AnyReconciler`'s
   enum-dispatch as it already does for `View`.

## Consequences

### Positive

- **The first-workload feature is unblocked.** US-03 can land its
  `JobLifecycle` reconciler reading a typed `JobLifecycleState`,
  and the existing `NoopHeartbeat` continues to work against
  `AnyState::Unit` without any signature awkwardness.
- **The dispatch shape is uniform.** Future reconcilers extend
  three enums in lockstep: `AnyReconciler::X(X)`,
  `AnyReconcilerView::X(XView)`, `AnyState::X(XState)`. The
  rule is mechanical and the compiler enforces non-exhaustive
  arms.
- **Per-tick I/O cost is proportional to the running reconciler.**
  The cert-rotation hydrator never reads the alloc-status rows;
  the lifecycle hydrator never reads the cert table. Each
  reconciler pays for what it consumes.
- **`reconcile` stays pure.** The async hydration paths live in
  the runtime, not in the trait body — `reconcile` continues to
  see only pre-hydrated typed projections, exactly as ADR-0013
  established. The ESR-verification target (ADR-0013 §1) is
  preserved.
- **The `State` placeholder is finally retired.** The opaque
  `pub struct State;` was a Phase 1 scaffolding artefact that
  every reviewer flagged; replacing it with a typed sum closes
  out a long-standing architectural debt.

### Negative

- **Each new reconciler now extends three enums, not two.**
  Adding a reconciler means a variant on `AnyReconciler` (already
  required), a variant on `AnyReconcilerView` (already required),
  and now a variant on `AnyState`. Phase 1's two reconcilers are
  noise; Phase 2+ with single-digit more reconcilers continues
  to be tractable. If the registry ever crosses the threshold
  where enum boilerplate becomes painful (per ADR-0013's
  ≥10-reconciler note for `#[async_trait]` migration), the
  three enums together make the case more visible — they all
  migrate together.
- **`type State` adds a second associated type to the trait.**
  This narrows the trait's dyn-compatibility surface further;
  the existing `enum AnyReconciler` dispatch absorbs the cost
  (no `Box<dyn Reconciler>` anywhere). Workflow primitive shape
  in Phase 3+ is unaffected because workflows are a different
  trait surface.
- **The runtime gains two new async surfaces (`hydrate_desired`,
  `hydrate_actual`) that read both stores.** Their failure
  shapes need to be threaded through; the runtime's existing
  `HydrateError` envelope (ADR-0013) extends to carry
  `IntentStoreError` and `ObservationStoreError` via `#[from]`.
  The wrapping is mechanical and matches the existing
  `ControlPlaneError` `#[from]` pass-through pattern (ADR-0015).

### Quality-attribute impact

- **Maintainability — modifiability**: positive. The compiler
  catches missing State arms at extension time. The "every
  reconciler extends three enums" rule is deterministic and
  scriptable (a future xtask check could validate the three
  arm counts match).
- **Maintainability — testability**: positive. `AnyState::Unit`
  satisfies all existing `NoopHeartbeat` tests verbatim. The
  new `JobLifecycle` tests construct `AnyState::JobLifecycle(…)`
  directly; no shared mutable state across reconciler variants.
- **Performance — time behaviour**: positive (asymptotically
  per-tick I/O is `O(reads needed by the running reconciler)`
  rather than `O(reads needed by any reconciler ever
  registered)`).
- **Reliability — testability under DST**: preserved. The
  `ReconcilerIsPure` invariant continues to hold — reconcile
  remains pure over its inputs; the DST harness constructs
  matching `AnyState` variants for every registered reconciler
  kind.

### Migration and backwards compatibility

This is a Phase 1 internal trait change. No external surface is
affected. The `Reconciler` trait gains `type State`; existing
implementations (`NoopHeartbeat`, `HarnessNoopHeartbeat` under the
`canary-bug` feature) declare `type State = ()` and the
`AnyReconciler::reconcile` arm carries `AnyState::Unit` for the
unit case. The `JobLifecycle` reconciler, landing fresh in
US-03, declares `type State = JobLifecycleState`.

There is no Phase 0 or external code to migrate. The `dst-lint`
and trait-signature compile-fail tests are updated in the same
PR that lands the change.

## Compliance

- **ADR-0013 §2 / §2b**: the runtime owns hydration; the
  reconciler stays pure. Preserved by routing both
  `hydrate_desired` and `hydrate_actual` through the runtime, not
  the trait.
- **ADR-0013 §2a**: enum-dispatch via `AnyReconciler`; no
  `Box<dyn Reconciler>`. Preserved — `AnyState` is the
  parameter-side companion to the existing `AnyReconcilerView`.
- **`ReconcilerIsPure` invariant** (ADR-0017): twin invocation
  with identical `(desired, actual, view, tick)` produces
  byte-identical output. Preserved — `AnyState` derives
  `PartialEq` so the twin-input check is mechanical.
- **`development.md` ordered-collection rule**: every keyed map
  inside `JobLifecycleState` is `BTreeMap<…>`. Preserved by
  construction.
- **`development.md` newtypes-STRICT rule**: the State variant
  carries existing newtypes (`JobId`, `NodeId`, `AllocationId`).
  No new identifiers.
- **Trait-signature compile-fail test** (existing
  `reconciler_trait_signature_is_synchronous_no_async_no_clock_param`):
  extended to assert the `&Self::State` parameter shape, not
  `&State`. Compile-time pin.

## References

- ADR-0013 — Reconciler primitive: trait, runtime, libSQL private
  memory. The companion structural decision.
- ADR-0017 — `overdrive-invariants` crate; `ReconcilerIsPure` is
  the load-bearing purity invariant this ADR preserves.
- Whitepaper §18 — Reconciler and workflow primitives.
- `docs/feature/phase-1-first-workload/discuss/user-stories.md`
  — US-03 Technical Notes enumerate the three options A / B / C.
- `docs/feature/phase-1-first-workload/discuss/dor-validation.md`
  — DoR item 8 flags this as a HARD design dependency.
- `crates/overdrive-core/src/reconciler.rs` — the existing
  `pub struct State;` placeholder this ADR retires.
- `.claude/rules/development.md` — Reconciler I/O rules; State
  hydration via runtime is the canonical shape.

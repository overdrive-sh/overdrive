# ADR-0084 — Cadence and event-interest as first-class, object-safe `Reconciler` declarations (GH #266)

## Status

Proposed. 2026-08-22. Author: Morgan (`nw-solution-architect`), DESIGN wave,
APPLICATION layer. Builds on the SYSTEM layer (Titan) recorded in
`docs/feature/reconciler-framework-improvements/feature-delta.md` and
`docs/product/architecture/brief.md` § "System Architecture → Reconciler-framework
improvements (#266)".

Tags: phase-1, reconciler-primitive, application-arch.

**Does NOT amend** ADR-0021 (`AnyState` per-reconciler typed projection),
ADR-0035 (View/redb persistence), or **ADR-0036** (runtime owns all hydration).
This ADR is strictly *additive* to the `Reconciler` trait — see § "ADR-0036 stands".

Ratified inputs from the user carried into this ADR (do not reopen):
- **RN-2 = B-2.** Piece B ships **interests-only, NO warm cache** at Phase 1.
  The warm reflector-`Store` (B-1) is deferred to **GH #270**. This ADR
  introduces no warm cache, no `ReflectorApplyBeforeHydrate` invariant, and no
  Phase-2 cardinality bounding — those live on #270.
- **Open Question 5 RESOLVED** — resync submits LWW-coalesce through the broker
  on `(ReconcilerName, TargetResource)`; resync MUST route through
  `broker.submit`, never a side channel (Titan SD-3).

## Amendment — 2026-08-23 (lean Piece B surface; user design review)

A user design review found the Piece B declaration surface over-abstracted and
directed a lean re-cut. The interest surface is now a **single** row-kind slice
keyed off a **complete, `ObservationRow`-owned** discriminant; three
speculative/duplicative types are dropped. Concretely:

- **`RowKind { AllocStatus }` → `ObservationRowKind` (complete, owned by
  `ObservationRow`).** The old `RowKind` was a hand-rolled *partial* subset of
  `ObservationRow` living in `reconcilers/mod.rs`, which forced a bespoke partial
  `classify(&ObservationRow) -> Option<RowKind>` and the argument that "a total
  `From<&ObservationRow>` is not writable." That was true *only because* the enum
  was insisted to stay a trimmed subset. A **complete** discriminant owned by
  `ObservationRow` makes a total projection (`ObservationRow::kind()`) trivially
  writable and moves the drift-closure onto the type that owns the variants —
  strictly stronger than the old partial helper. A complete discriminant of an
  existing closed enum is **not** speculative surface (every variant already
  exists), so the "trim to the used set" argument that dropped `WholeManaged`
  does not apply to it.
- **`TargetFrom { Workload }` + `derive_target(...)` dropped.** They were a
  one-variant strategy enum plus a one-line resolver that *duplicated the inline
  `workload/<id>` derivation already at `exit_observer.rs:232`*. By this ADR's
  own "no unused/speculative surface" principle (the same one that dropped
  `WholeManaged` and kept enums single-variant), a one-strategy indirection earns
  nothing — the router derives the workload target **inline from the row**, keyed
  by row kind. A *per-interest* target strategy is re-introduced additively only
  if a future reconciler needs a different target from the same row kind.
- **`Interest { row_kind, target_from }` struct dropped.** With `target_from`
  gone, the interest declaration collapses to `&'static [ObservationRowKind]`.

Net surface delta: **`Interest`, `RowKind`, `TargetFrom`, `classify`,
`derive_target` are removed; `ObservationRowKind` + `ObservationRow::kind()` are
added** (the latter a read-only projection beside `ObservationRow` — the row type
is *not* modified, so zero rkyv/layout/discriminant impact). `interests()` now
returns `&'static [ObservationRowKind]`. The sections below are updated in place;
Piece A, RN-2 = B-2, ADR-0036, the List-then-Watch/Lagged-relist shape, and the
single-cut greenfield migration are **unchanged** except for the type names the
last uses.

## Context

`spawn_convergence_loop` (`crates/overdrive-control-plane/src/lib.rs:2427`) drives
one broker-drain + tick loop over an enum-dispatched reconciler registry
(`AnyReconciler`, `crates/overdrive-core/src/reconcilers/mod.rs:798`). Two smells,
both named by #266:

1. **Cadence (Piece A).** The loop has no first-class way for a reconciler to
   declare a periodic level-triggered resync. The incoming microvm-driver branch
   carries a `VM_RECLAMATION_SWEEP_INTERVAL` + `node/<id>` hardcode in the loop
   (not yet on `main` — research Gap 3). Absent a declaration surface, that
   hardcode reaches `main` when the 8th reconciler lands.

2. **Event-interest (Piece B).** *Which* observation-row change wakes *which*
   reconciler is wired imperatively at scattered producer sites that name their
   consumers — the exit-observer explicitly enqueues four downstream reconcilers
   after every `alloc_status` write
   (`crates/overdrive-control-plane/src/worker/exit_observer.rs:234,254,295,320`).
   The consumer half (the `EvaluationBroker` workqueue, cancelable-set
   storm-proofing) already exists and is correct; the missing structural surface
   is the **declarative producer half** (research Findings 3b/3e; candidate E).

The Rust constraint (research Finding 3d): `Reconciler` carries associated types
(`State`/`View`) and is therefore not `dyn`-compatible; the `AnyReconciler` /
`AnyState` / `AnyReconcilerView` enums are the hand-rolled erasure layer. The
*hydration* erasure (research candidates B/C/D, Open Questions 1/2/6) is a genuine
value judgment the evidence frames but does not settle. **Both hooks in this ADR
sidestep it entirely**: each returns a concrete type with no associated types, so
each threads through `AnyReconciler` with one forwarding arm and touches neither
`AnyState` nor `AnyReconcilerView`.

## Decision

### 1. Two additive, default-provided methods on `Reconciler`

Added to the trait at `crates/overdrive-core/src/reconcilers/mod.rs`. Both carry a
default impl, so all 7 existing reconcilers compile unchanged; only reconcilers
that opt in override them.

```rust
pub trait Reconciler: Send + Sync {
    const NAME: &'static str;
    type State: Send + Sync;
    type View: Serialize + DeserializeOwned + Default + Clone + Eq + Send + Sync;
    fn name(&self) -> &ReconcilerName;
    fn reconcile(
        &self,
        desired: &Self::State,
        actual: &Self::State,
        view: &Self::View,
        tick: &TickContext,
    ) -> (Vec<Action>, Self::View);

    // --- Piece A: cadence (additive; default None) ---
    /// Declarative level-triggered resync cadence — a safety net beside the
    /// edge-triggered broker (K8s `SyncPeriod`/`RequeueAfter`; kube-rs
    /// `Action::requeue_after`). Default `None` = edge-triggered only, no
    /// backstop.
    ///
    /// PURE + object-safe: returns concrete data, reads NO clock, holds no
    /// handle. The convergence loop owns the clock (`SimClock` under DST), the
    /// local `NodeId`, and scope→target resolution. No associated types ⇒ one
    /// `AnyReconciler` forwarding arm; touches no `AnyState`/`AnyReconcilerView`.
    fn resync_schedule(&self) -> Option<ResyncSchedule> {
        None
    }

    // --- Piece B: event-interest (additive; default empty) ---
    /// Declarative event-interest: which observation-row *kinds* wake this
    /// reconciler. Default `&[]` = **host-backed** (hydrates `actual` live from
    /// the host, never row-backed) ⇒ **resync-only**, never event-woken. The
    /// interest declaration IS the partition key (Titan SD-6): non-empty ⟺
    /// row-backed ⟺ event-woken with resync as backstop.
    ///
    /// PURE + object-safe: returns a borrowed static slice of
    /// `ObservationRowKind` — no payload, no severity, no occurrence semantics
    /// (contrast GH #265's `ObservationEvent`, which is the outbound "what
    /// happened"). No associated types ⇒ one `AnyReconciler` forwarding arm;
    /// touches no `AnyState`/`AnyReconcilerView`.
    fn interests(&self) -> &'static [ObservationRowKind] {
        &[]
    }
}
```

### 2. The declaration types (core, pure data)

Live in `overdrive-core` (`core`-class — no `Clock`/`Transport`/`Entropy`, dst-lint
clean). Label enums own their `as_str` per `development.md` § "Label enums own
their string representation".

```rust
/// Piece A — a resync cadence declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResyncSchedule {
    /// Minimum wall-clock period between level-triggered resyncs. The loop's
    /// per-reconciler next-wake table re-arms at most once per period (C-A2).
    pub period: Duration,
    /// Which target(s) each fire submits. The LOOP resolves this — the
    /// reconciler never names a target string.
    pub scope: ResyncScope,
}

/// The target-set a resync fires against. Resolved by the loop from state it
/// owns (the local `NodeId`).
///
/// Phase 1 ships exactly one variant — `LocalNode` — the only shape any
/// current or incoming reconciler needs. A coarse whole-set scope
/// (`WholeManaged`) is **deliberately NOT declared here**: its loop resolver
/// would need the managed-target-set source that is itself the #270 bounding
/// concern, so shipping it now would force an unimplementable (`todo!`)
/// resolver arm — the exact unused-surface smell the project forbids. It is
/// added additively (one enum variant + one resolver arm, in one change) the
/// day a reconciler declares it. Keeping the enum single-variant today means
/// the loop's `resolve_scope` is total and fully exercised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResyncScope {
    /// Resolves to exactly `node/<local_node_id>` (the loop supplies the id).
    /// The vm-reclamation motivating case.
    LocalNode,
}
```

The Piece B interest surface is a **complete discriminant of `ObservationRow`**,
one variant per row family, living in
`crates/overdrive-core/src/traits/observation_store.rs` **beside `ObservationRow`**
— the type owns its own discriminant (contrast the Piece A cadence types, which
live in `reconcilers/`). `ObservationRow` itself is **not modified**: only this
sibling enum + a read-only `kind()` projection are added, so there is **zero**
rkyv / layout / discriminant impact on the persisted row.

```rust
/// Complete discriminant of `ObservationRow` — one variant per row family.
///
/// `Ord` because it keys the interest table
/// `BTreeMap<ObservationRowKind, Vec<ReconcilerName>>` (development.md
/// ordered-collection rule). Label enum owns its `as_str`.
///
/// Unlike `WholeManaged` (dropped from `ResyncScope` above as speculative,
/// unimplementable surface), a **complete discriminant of an existing closed
/// enum is NOT speculative surface** — every variant already exists on
/// `ObservationRow`, so enumerating them is a total projection, not a forward
/// bet. The "trim to the used set" argument therefore does not apply to it: all
/// eight variants are listed. (A reconciler still declares interest only in the
/// kinds it consumes; at Phase 1 that is `AllocStatus` alone.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObservationRowKind {
    AllocStatus, NodeHealth, ServiceHydration, ServiceBackend,
    ReconcileConflict, IssuedCertificate, WorkflowTerminal, Signal,
}

impl ObservationRow {
    /// Total, NO-wildcard discriminant projection — one arm per `ObservationRow`
    /// variant, no `_`. A new `ObservationRow` variant fails to compile here
    /// until consciously mapped: the drift-closure now lives **on the type it
    /// describes** (strictly stronger than the old partial `classify`, which had
    /// to hand-list every unmapped variant `=> None` against a foreign type).
    pub const fn kind(&self) -> ObservationRowKind {
        match self {
            Self::AllocStatus(_)          => ObservationRowKind::AllocStatus,
            Self::NodeHealth(_)           => ObservationRowKind::NodeHealth,
            Self::ServiceHydration(_)     => ObservationRowKind::ServiceHydration,
            Self::ServiceBackend(_)       => ObservationRowKind::ServiceBackend,
            Self::ReconcileConflict(_)    => ObservationRowKind::ReconcileConflict,
            Self::IssuedCertificate(_)    => ObservationRowKind::IssuedCertificate,
            Self::WorkflowTerminal { .. } => ObservationRowKind::WorkflowTerminal,
            Self::Signal { .. }           => ObservationRowKind::Signal,
        }
    }
}
```

No `Interest` struct, no `TargetFrom` enum, no `derive_target` resolver: with a
complete discriminant the interest declaration is a bare
`&'static [ObservationRowKind]`, and the changed row's target is derived
router-local (§5). `Duration` (Piece A) uses the same monotonic type the
`TickContext` clock already uses.

### 3. `AnyReconciler` gains two forwarding arms (no erasure change)

Exactly parallel to the existing `name()` / `static_name()` forwarders — a single
`match` over the 7 variants delegating to the inner reconciler. **No `AnyState`
change, no `AnyReconcilerView` change, no new `reconcile`-dispatch triple-match.**

```rust
impl AnyReconciler {
    pub fn resync_schedule(&self) -> Option<ResyncSchedule> {
        match self { Self::NoopHeartbeat(r) => r.resync_schedule(), /* …7 arms… */ }
    }
    pub fn interests(&self) -> &'static [ObservationRowKind] {
        match self { Self::NoopHeartbeat(r) => r.interests(), /* …7 arms… */ }
    }
}
```

### 4. Piece A — the loop change (per-reconciler next-wake table)

At registration the runtime builds a cadence table from
`AnyReconciler::resync_schedule()` for every reconciler returning `Some`.
`spawn_convergence_loop` owns a `BTreeMap<ReconcilerName, UnixInstant>` next-wake
table (`BTreeMap` per `development.md` § "Ordered-collection choice") and, each
iteration:

```
now = clock.now()
for (name, schedule) in cadence_table:
    if next_wake[name] <= now:
        for target in resolve_scope(schedule.scope, local_node_id, managed):
            broker.submit(Evaluation { reconciler: name.clone(), target })   // C-A1
        next_wake[name] = now + schedule.period                              // C-A2
drain_pending() + dispatch     // existing
clock.sleep(cadence)           // existing
```

`resolve_scope(LocalNode, id) = [ node/<id> ]` using the `NodeId` the loop
already owns (`state.node_id`; `NodeId::new("local")`,
`crates/overdrive-control-plane/src/lib.rs:1701`). With `ResyncScope` a
single-variant enum at Phase 1, `resolve_scope` is **total** — no `todo!`/
`unreachable` arm. After this change the loop carries **no reconciler name, no
cadence constant, no hardcoded target scheme** — only the generic table + a scope
resolver over an enum it understands. `C-A1` (route through `broker.submit`) and
`C-A2` (re-arm at most once per period) are the two constraints that keep Open
Question 5's no-storm verdict true.

### 5. Piece B — the interest fan-out (one new runtime task) + single-cut migration

At registration the runtime builds an interest table
`BTreeMap<ObservationRowKind, Vec<ReconcilerName>>` from
`AnyReconciler::interests()`. A **new runtime task** (`spawn_interest_router`) is
**List-then-Watch**, the same shape as `ServiceBackendsResolve`'s resolve index:

1. **Open** the **existing** `ObservationStore::subscribe_all_events()` watcher
   (`observation_store.rs:1896`) — subscribe *first* so no accepted write is
   missed in the boot window (a `tokio::broadcast` subscriber does not see sends
   that predate its subscription).
2. **List** — read the current snapshot of the interested row families
   (`alloc_status_rows()`, …) and submit an Evaluation per row's derived target,
   so the router does not depend on a change arriving to first wake an interested
   reconciler.
3. **Watch** — for each stream item:
   - **`SubscriptionEvent::Row(row)`** — take the row's family via `row.kind()`
     (the total, no-wildcard `ObservationRow::kind()` projection, §2). If
     `interest_table.get(&row.kind())` is non-empty, derive the broker
     `TargetResource` **inline from the row** and `broker.submit` one Evaluation
     per interested reconciler. At Phase 1 every routed kind maps to a workload
     target:

     ```rust
     let kind = row.kind();
     if let Some(reconcilers) = interest_table.get(&kind) {
         let target = match &row {
             ObservationRow::AllocStatus(r) =>
                 TargetResource::new(format!("workload/{}", r.workload_id)),
             // No other kind routes at Phase 1: interest_table.get(&kind) is
             // empty for every non-AllocStatus kind, so no other arm is reachable.
             _ => continue,
         };
         for reconciler in reconcilers {
             broker.submit(Evaluation { reconciler: reconciler.clone(), target: target.clone() });
         }
     }
     ```

     The "how to derive the target from the row" now lives **router-local**,
     keyed by row kind — correct for Phase 1, where every routed kind derives a
     workload target and all four consumers key identically. A *per-interest*
     target strategy (the dropped `TargetFrom` enum + `derive_target` resolver)
     is re-introduced additively **only if** a future reconciler needs a
     *different* target from the *same* row kind — deferred as speculative
     surface, exactly like `WholeManaged` (§2). The watcher emits a `Row` only
     for an *accepted* write (LWW winner) — a rejected or no-op write is never
     delivered — so the fan-out fires on genuine changes only, matching the
     exit-observer's "nudge only when the row changed" gate.

     **The drift-closure now lives on `ObservationRow::kind()`.** The old partial
     `fn classify(row: &ObservationRow) -> Option<RowKind>` reached for a
     no-wildcard match *against a foreign type* to force a compile error when a
     new `ObservationRow` variant landed — precisely because it insisted the kind
     enum stay a trimmed subset, which is what made a total
     `From<&ObservationRow>` unwritable. `ObservationRowKind` inverts that:
     `ObservationRow::kind()` **is** the total, no-wildcard projection, owned by
     the type it describes (`observation_store.rs`, 8 variants). A new
     `ObservationRow` variant fails to compile at `kind()` until consciously
     mapped — the same drift-closure, now stronger (on the owning type, not a
     router-local helper) and simpler (no `Option`, no bespoke `classify`).
   - **`SubscriptionEvent::Lagged { .. }`** — **relist** (repeat step 2): re-read
     the snapshot and re-submit per derived target. No warm cache to rebuild
     (B-2) — the relist re-derives the *wakeups*, bounded by managed cardinality.
     This honours the mandatory `Lagged` contract (`observation_store.rs:1734`).

**Single-cut migration (greenfield, no shim).** In one change: delete the four
`exit_observer` producer submits and add `interests()` overrides on their four
consumers (`workload-lifecycle`, `backend-discovery-bridge`, `service-lifecycle`,
`svid-lifecycle`), each declaring `&[ObservationRowKind::AllocStatus]`, plus the
fan-out task. The exit-observer keeps writing the `AllocStatusRow` and
broadcasting its `LifecycleEvent`; it stops naming consumers.

**Design rule (no busy-loop).** The no-busy-loop guarantee rests on two precise
properties, not on the loose "no consumer authors `alloc_status`":
(a) the *author* of `alloc_status` rows is the action-shim / exit-observer /
driver path — **not** an interest-declaring reconciler — so a fan-out wake never
directly re-triggers the writer; and (b) each interested reconciler is
**convergent**: `action → alloc_status write → fan-out wake → reconcile` reaches a
fixpoint where the reconcile emits no further self-perpetuating write. A
reconciler MUST NOT declare interest in a row family it *authors* unless it
converges on that row by reading it back (ADR-0079). The four `AllocStatus`
consumers author no `alloc_status` and are convergent, so the cut is loop-free;
the broker's LWW key-collapse coalesces any burst. The acceptance-designer pins
this as a DST invariant over the full `action → write → wake → reconcile` cycle
(fixpoint reached; no storm) — see § Consequences.

### ADR-0036 stands (unchanged)

Under B-2 there is no warm cache: `hydrate_actual` continues to read the
node-local CR-SQLite replica per tick — exactly ADR-0036's "runtime owns all
hydration". The two new methods add no async surface, no `&LibsqlHandle`, no clock
read, and do not alter `reconcile`'s signature, so the compile-time guard
`reconciler_trait_signature_is_synchronous_no_async_no_clock_param`
(`mod.rs:271`) still passes. This ADR is additive; ADR-0036 is neither amended nor
superseded.

## Alternatives considered

- **Piece A cadence shape — `next_evaluation(now) -> Option<Evaluation>`
  (per-object `RequeueAfter` deadlines).** Rejected for Phase 1: no current
  reconciler needs per-target dynamic deadlines (vm-reclamation needs one
  `node/<id>` sweep). It also passes `now` into the hook, making the hook read
  time — a weaker purity posture than a static schedule that reads nothing.
  `Option<ResyncSchedule{period, scope}>` is the minimal sufficient surface;
  `next_evaluation` remains an additive future extension if a reconciler ever
  needs per-object deadlines.
- **Piece A cadence shape — bare `resync_period(&self) -> Option<Duration>`.**
  Rejected: it cannot express *which* target(s) to resync, forcing the loop back
  into a hardcoded target scheme (the exact smell #266 removes). `ResyncScope`
  carries the target intent while keeping resolution in the loop.
- **Piece B — reconciler-emitted `Action::EnqueueEvaluation` as the interest
  mechanism (status quo, producer-push).** Kept for reconciler→reconciler
  handoffs, but rejected as *the* interest surface: it names the consumer at the
  producer and is imperative. `interests()` is the consumer-pull dual the mature
  frameworks converge on (kube-rs `.watches()`; research 3b). Migrating the
  reconciler-emitted enqueues to `interests()` is **deferred to GH #271** (RN-A1);
  see it for the scope boundary between the two.
- **Piece B — warm reflector-`Store` (B-1) now.** Deferred to GH #270 by
  ratified RN-2 = B-2. The latency win is ~nil (node-local stores; Titan Caveat
  3); the unification win does not require the *warm cache*, only the *declarative
  interest fan-out*, which B-2 delivers.
- **Facet-2 hydration-erasure rework (erased-trait `downcast` / per-type
  monomorphization).** Out of scope — **deferred to GH #272**, scoped around
  open-world / WASM third-party reconcilers as the forcing function. Hydration
  stays runtime-owned (ADR-0036 unchanged).

## Consequences

**Positive.**
- The loop is reconciler-agnostic for cadence: no hardcode ever reaches `main`.
- Event-interest becomes declarative and object-safe; the exit-observer stops
  naming its consumers; the fan-out fires on *any* accepted `alloc_status` write,
  which is strictly more correct level-triggering than the prior scattered enqueues.
- Both hooks preserve the pure-`reconcile` / single-loop / single-clock DST story
  (Titan §11): the fan-out is a deterministic broker-submit source given the
  DST-controlled change-feed order; no cache-ordering invariant is needed under B-2.
- `AnyState`/`AnyReconcilerView`/the `reconcile` triple-match are untouched;
  compile-time exhaustiveness is preserved.

**Negative / to validate in DISTILL/DELIVER.**
- The fan-out broadens wakeups: every accepted `alloc_status` write now nudges all
  four interested consumers (previously only the exit-observer path did). This is
  benign — the broker coalesces and the consumers no-op on irrelevant targets
  without re-enqueue — but the acceptance-designer MUST pin a DST invariant
  (convergence holds; no broker storm; no busy-loop) over the fan-out.
- The `exit_observer` acceptance tests that assert "enqueues bridge/service/svid"
  migrate to "fan-out enqueues interested reconcilers on `alloc_status` change" in
  the same cut.
- A coarse whole-set resync scope is intentionally NOT shipped (§2): it is added
  additively when a reconciler needs it, together with the managed-target-set
  resolver that couples to the #270 bounding work. Phase-1 `resolve_scope` is total.

## References

- Feature delta (SYSTEM + APPLICATION):
  `docs/feature/reconciler-framework-improvements/feature-delta.md`
- Wave decisions:
  `docs/feature/reconciler-framework-improvements/design/wave-decisions.md`
- Research SSOT:
  `docs/research/architecture/cqrs-structural-mechanism-reconciler-framework-research.md`
  (§3b/3c/3d/3e, §4.1, §6)
- Governing precedent for cache/convergence discipline: ADR-0079.
- Unchanged: ADR-0021, ADR-0035, **ADR-0036**.
- GH #266 (this feature), GH #270 (B-1 warm cache deferral), GH #271 (RN-A1 —
  `Action::EnqueueEvaluation` → `interests()` migration deferral), GH #272
  (Facet-2 hydration-erasure rework deferral), GH #265 (durable events — separate
  track; inherits the resync-dedup constraint).

# ADR-0081 — Cadence and event-interest as first-class, object-safe `Reconciler` declarations (GH #266)

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
    /// Declarative event-interest: which observation-row changes wake this
    /// reconciler. Default `&[]` = **host-backed** (hydrates `actual` live from
    /// the host, never row-backed) ⇒ **resync-only**, never event-woken. The
    /// interest declaration IS the partition key (Titan SD-6): non-empty ⟺
    /// row-backed ⟺ event-woken with resync as backstop.
    ///
    /// PURE + object-safe: returns borrowed static routing metadata — no
    /// payload, no severity, no occurrence semantics (contrast GH #265's
    /// `ObservationEvent`, which is the outbound "what happened"). No associated
    /// types ⇒ one `AnyReconciler` forwarding arm; touches no
    /// `AnyState`/`AnyReconcilerView`.
    fn interests(&self) -> &'static [Interest] {
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

/// Piece B — a declarative event-interest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interest {
    /// Which observation-row family wakes the reconciler.
    pub row_kind: RowKind,
    /// How the changed row's identity maps to the broker `TargetResource` the
    /// reconciler is evaluated against.
    pub target_from: TargetFrom,
}

/// The observation-row families a reconciler can declare interest in.
///
/// Phase 1 ships exactly one variant — `AllocStatus` — the only family any
/// current `interests()` override declares (the four migrated `exit_observer`
/// consumers). Further families are added **additively, one variant per new
/// interest**, the day a reconciler declares one — identical treatment to why
/// `WholeManaged` was dropped from `ResyncScope` above (no unused surface).
/// Keeping the enum trimmed to the used set keeps `classify` (§5 step 3) a
/// total, fully-exercised mapping over the families that actually route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RowKind { AllocStatus }

/// How the fan-out derives the broker `TargetResource` from a changed row.
///
/// Phase 1 ships exactly one variant — `Workload` (→ `workload/<workload_id>`) —
/// the only mapping any current interest declares. Added additively per new
/// interest, as with `RowKind` above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetFrom { Workload }
```

`Duration` uses the same monotonic type the `TickContext` clock already uses.

### 3. `AnyReconciler` gains two forwarding arms (no erasure change)

Exactly parallel to the existing `name()` / `static_name()` forwarders — a single
`match` over the 7 variants delegating to the inner reconciler. **No `AnyState`
change, no `AnyReconcilerView` change, no new `reconcile`-dispatch triple-match.**

```rust
impl AnyReconciler {
    pub fn resync_schedule(&self) -> Option<ResyncSchedule> {
        match self { Self::NoopHeartbeat(r) => r.resync_schedule(), /* …7 arms… */ }
    }
    pub fn interests(&self) -> &'static [Interest] {
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
`BTreeMap<RowKind, Vec<(ReconcilerName, TargetFrom)>>` from
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
   - **`SubscriptionEvent::Row(row)`** — classify `row` via
     `fn classify(row: &ObservationRow) -> Option<RowKind>`, an **exhaustive
     per-variant match with NO wildcard**: the mapped family returns
     `Some(RowKind::…)`, and every unmapped `ObservationRow` variant is listed
     explicitly and returns `None`. At Phase 1:

     ```rust
     fn classify(row: &ObservationRow) -> Option<RowKind> {
         match row {
             ObservationRow::AllocStatus(_)          => Some(RowKind::AllocStatus),
             ObservationRow::NodeHealth(_)           => None,
             ObservationRow::ServiceHydration(_)     => None,
             ObservationRow::ServiceBackend(_)       => None,
             ObservationRow::ReconcileConflict(_)    => None,
             ObservationRow::IssuedCertificate(_)    => None,
             ObservationRow::WorkflowTerminal { .. } => None,
             ObservationRow::Signal { .. }           => None,
         }
     }
     ```

     A total `impl From<&ObservationRow> for RowKind` is **not writable**:
     `ObservationRow` has 8 variants (`observation_store.rs:610-690` — AllocStatus,
     NodeHealth, ServiceHydration, ServiceBackend, ReconcileConflict,
     IssuedCertificate, WorkflowTerminal, Signal) and `RowKind` does not (and must
     not) cover all of them, so a `From` would force inventing bogus `RowKind`
     variants. `classify` returns `Option` instead — `None` means "no reconciler
     routes off this family." The no-wildcard match preserves the drift-closure the
     `From` was reaching for: a future `ObservationRow` variant makes the match
     non-exhaustive and **fails compilation until the author consciously decides
     `Some`/`None`** — while being implementable. For each `Some(row_kind)` and each
     interested `(reconciler, target_from)`, derive the `TargetResource` from the
     row's identity (`Workload → workload/<workload_id>`) and `broker.submit`. The
     watcher emits a `Row` only for an *accepted* write (LWW winner) — a rejected
     or no-op write is never delivered — so the fan-out fires on genuine changes
     only, matching the exit-observer's "nudge only when the row changed" gate.
   - **`SubscriptionEvent::Lagged { .. }`** — **relist** (repeat step 2): re-read
     the snapshot and re-submit per derived target. No warm cache to rebuild
     (B-2) — the relist re-derives the *wakeups*, bounded by managed cardinality.
     This honours the mandatory `Lagged` contract (`observation_store.rs:1734`).

**Single-cut migration (greenfield, no shim).** In one change: delete the four
`exit_observer` producer submits and add `interests()` overrides on their four
consumers (`workload-lifecycle`, `backend-discovery-bridge`, `service-lifecycle`,
`svid-lifecycle`), each declaring `Interest { row_kind: AllocStatus, target_from:
Workload }`, plus the fan-out task. The exit-observer keeps writing the
`AllocStatusRow` and broadcasting its `LifecycleEvent`; it stops naming consumers.

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

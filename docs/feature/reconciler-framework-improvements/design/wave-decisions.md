# DESIGN wave decisions — `reconciler-framework-improvements` (GH #266) — APPLICATION layer (Morgan)

Authored **after** the SYSTEM layer (Titan, `../feature-delta.md` § "Wave: DESIGN /
[SYSTEM]"). This file records the APPLICATION-layer decisions: the exact
`Reconciler` trait surface, the loop/runtime wiring, the Reuse Analysis gate, and
the reconciliation of the SYSTEM RN-table against the ratified B-2.

Ratified inputs (do not reopen): **RN-2 = B-2** (interests-only, no warm cache;
B-1 → GH #270); **Open Question 5 resolved** (resync coalesces via
`broker.submit`); RN-1 is Morgan's to lock.

---

## Scoping verdict — CONFIRM the core, REFINE the site list

**CONFIRMED (core scoping).** With B-2 ratified, **neither piece touches the
`AnyState`/`AnyReconciler` type-erasure, and neither amends ADR-0036.**

- **Piece A** — `resync_schedule(&self) -> Option<ResyncSchedule>`: concrete
  return, no associated types → one `AnyReconciler` forwarding arm; touches no
  `AnyState`/`AnyReconcilerView`; loop owns the clock. Hydration unchanged.
- **Piece B** — `interests(&self) -> &'static [Interest]`: concrete return, no
  associated types → one `AnyReconciler` forwarding arm; touches no
  `AnyState`/`AnyReconcilerView`. Hydration stays runtime-owned, per-tick from the
  CR-SQLite replica (no warm cache, no reflector-`Store`, no
  `ReflectorApplyBeforeHydrate`) → **ADR-0036 stands**.
- The compile-time `reconcile` signature guard (`reconcilers/mod.rs:271`) still
  passes — the two new methods add no async/clock/DB surface and do not alter
  `reconcile`.
- Consequently the **Facet-2 hydration-erasure rework (research candidates B/C/D,
  OQ 1/2/6) is a THIRD deferral** (GH #272) — see § "Deferrals".

**REFINED (submit-site list — partial refutation, not of the core).** The dispatch
brief's belief that Piece B "replaces exit_observer ×3, handlers.rs,
action_shim/enqueue_evaluation.rs" is imprecise against live code:

| Site | Reality | In Piece B's single cut? |
|---|---|---|
| `exit_observer.rs:234,254,295,320` (**×4**, not ×3) | Fire on an accepted **`alloc_status` write** — observation-row-change producers naming 4 consumers | **YES** — replaced by the fan-out; the 4 consumers declare `interests()`. |
| `handlers.rs:76` (`enqueue_workload_lifecycle_eval`) | Fires on an **IntentStore write** (operator `deploy`/`stop` → `workload-lifecycle`) — an **intent edge**, not an observation-row change | **NO** — the fan-out is over the Observation change feed only (hard constraint). Stays. |
| `action_shim/enqueue_evaluation.rs:58` (+ emitters in `workload_lifecycle.rs`, `backend_discovery_bridge.rs`) | Dispatcher of **reconciler-emitted** `Action::EnqueueEvaluation` — reconcile-body logic + first-tick-latency behaviour + acceptance-test coverage | **RN-A1** — value judgment; recommend **keep** (see below). |
| `reconciler_runtime.rs:1591` (`has_work` self-re-enqueue) | Reconciler re-arms **itself** — not a producer-names-consumer smell | **NO** — stays. |

---

## Locked signatures

### Piece A — cadence (RN-1 LOCKED)

```rust
// overdrive-core/src/reconcilers/  (core-class; pure data; label enums own as_str)
pub struct ResyncSchedule { pub period: Duration, pub scope: ResyncScope }
pub enum   ResyncScope    { LocalNode }   // single-variant at Phase 1 (see below)

// on trait Reconciler (additive, default None)
fn resync_schedule(&self) -> Option<ResyncSchedule> { None }

// on AnyReconciler (one forwarding match, 7 arms; like name())
pub fn resync_schedule(&self) -> Option<ResyncSchedule>
```

`LocalNode → [node/<local_node_id>]` resolved in the loop from the `NodeId` it
owns. **Divergence from Titan's RN-1 recommendation (`scope ∈ {LocalNode,
WholeManaged}`), within Morgan's authority to lock the signature:** `WholeManaged`
is **dropped** from the Phase-1 enum. Its loop resolver needs the managed-target
set that is itself the #270 bounding concern, so shipping it now forces an
unimplementable (`todo!`) resolver arm — the unused-surface smell the project
forbids. Keeping `ResyncScope` single-variant makes `resolve_scope` total and
fully exercised; `WholeManaged` is added additively (one variant + one arm, one
change) the day a reconciler declares it. This is a strengthening, not a scope
change (WholeManaged was unused at Phase 1 regardless).

### Piece B — event-interest (LOCKED)

```rust
// overdrive-core/src/reconcilers/  (core-class; pure data; label enums own as_str)
pub struct Interest   { pub row_kind: RowKind, pub target_from: TargetFrom }
pub enum   RowKind    { AllocStatus }   // single-variant at Phase 1 (see below)
pub enum   TargetFrom { Workload }      // single-variant at Phase 1 (see below)

// on trait Reconciler (additive, default &[])
fn interests(&self) -> &'static [Interest] { &[] }

// on AnyReconciler (one forwarding match, 7 arms; like name())
pub fn interests(&self) -> &'static [Interest]
```

Current-cut consumers each declare `[Interest { row_kind: AllocStatus,
target_from: Workload }]`: `workload-lifecycle`, `backend-discovery-bridge`,
`service-lifecycle`, `svid-lifecycle`. Default `&[]` ⟺ host-backed ⟺ resync-only
(the partition key, Titan SD-6).

`RowKind`/`TargetFrom` are trimmed to the **used set** — only `{AllocStatus}` /
`{Workload}` is referenced by any Phase-1 `interests()` override. Further variants
are added **additively, one per new interest** (identical treatment to why
`WholeManaged` was dropped from `ResyncScope` above — no unused surface). This
keeps `classify` (below) a total, fully-exercised mapping over the families that
actually route.

**Fan-out task is List-then-Watch** (ADR-0081 § 5): subscribe first, then list the
interested snapshot families and submit per row (closing the boot-window gap where
a `tokio::broadcast` subscriber misses pre-subscription sends), then watch. Row →
`RowKind` classification is `fn classify(row: &ObservationRow) -> Option<RowKind>`
— an **exhaustive per-variant match, NO wildcard** (`AllocStatus =>
Some(RowKind::AllocStatus)`; the other 7 `ObservationRow` variants each explicitly
`=> None`), so a new `ObservationRow` variant fails to compile until the author
consciously maps it `Some`/`None`. A total `From<&ObservationRow>` is not writable
(8 row variants; `RowKind` covers only the routed family, so a `From` would force
bogus variants); `classify` returns `Option` and preserves the same drift-closure.
`Lagged` → relist (repeat the list step).

---

## Reuse Analysis (MANDATORY GATE)

Contract shape (Principle 12): **pure-fn** = return-only, no mutation;
**bounded-change** = declared mutation set over an enumerated effect universe;
**unbounded-preservation** = must return a Plan, never mutate. There is **no**
unbounded-preservation / preview / dry-run surface in this feature — the only
side-effecting component is the fan-out task, whose entire effect universe is
`broker.submit((reconciler, target))` and whose injected capability is a restricted
`EvaluationBroker` handle (not a god-object). The two driving declarations expose
no write methods (read-only trait accessors).

| Component | Verdict | Contract shape (universe / assertion) | Rationale |
|---|---|---|---|
| `Reconciler::resync_schedule` / `::interests` | **EXTEND** | **pure-fn** — return-only borrowed/`Copy` data, no mutation; asserted by the `mod.rs:271` signature guard + a per-method purity unit test | Two additive default methods; no existing method changed. |
| `AnyReconciler` (`:798`) | **EXTEND** | **pure-fn** — forwarders, no mutation | Two forwarding arms mirroring `name()`; no `AnyState`/`AnyReconcilerView` touch. |
| `spawn_convergence_loop` next-wake table (`lib.rs:2427`) | **EXTEND** | **bounded-change** — universe = `{ next_wake[name] writes; broker.submit(resync eval) }`; per-fire delta = one `submit` per resolved target + one re-arm; asserted by a DST invariant (≤1 re-arm/period; submits coalesce) | Piece A: per-reconciler next-wake table + total scope resolver. |
| Registration path (`reconciler_runtime.rs` `register`) | **EXTEND** | **bounded-change** — universe = `{ cadence table, interest table }` built once at boot from the trait methods | Build the cadence + interest tables. |
| `spawn_interest_router` (NEW) | **CREATE NEW** | **bounded-change** — universe = `broker.submit((reconciler, target))` only; per-`Row` delta = one submit per interested `(reconciler, target_from)`; per-`Lagged` delta = one submit per interested snapshot row; injected capability = restricted `EvaluationBroker` handle; asserted by a DST invariant over the fan-out trajectory | No existing interest-fan-out component; the scattered `exit_observer` submits are what it replaces, not a reusable component. Small, runtime-internal, mirrors the `exit_observer` / workflow-emit-drain spawned-task shape. |
| `Interest`/`RowKind`/`TargetFrom` types | **CREATE NEW** | **pure data** (no behaviour) | No existing declarative-interest types — the new declaration surface. |
| `ResyncSchedule`/`ResyncScope` types | **CREATE NEW** | **pure data** | No existing cadence-declaration types. |
| `ObservationStore::subscribe_all_events` (`:1896`) | **REUSE (unchanged)** | read-only stream consumer | Fan-out consumes the existing watcher; no new port method. |
| `EvaluationBroker` (`eval_broker.rs`) | **REUSE (unchanged)** | its own `submit`/`drain` contract unchanged | Fan-out + resync route through `submit`; LWW coalescing unchanged (OQ5). |
| `Evaluation` / `TargetResource` / `ReconcilerName` | **REUSE (unchanged)** | value types | Broker key components; `node/` / `workload/` prefixes already exist (`:733`). |
| Snapshot reads (`alloc_status_rows`, `all_service_backends_rows`) | **REUSE (unchanged)** | read-only | The List-then-Watch list step + relist-on-`Lagged` read these. |
| `exit_observer.rs` ×4 submits | **DELETE** | — | Replaced by the fan-out (single cut, no shim). |

No CREATE NEW is a reimplementation of an existing capability; each is a genuinely
new surface with no in-tree equivalent. No component is unbounded-preservation, so
the frame-problem "silent write" bug class is non-representable here by
construction (no preview/plan surface exists to be violated).

---

## Tech Stack

**Rust.** Object-oriented paradigm per project `CLAUDE.md`. No new crates, no new
dependencies. New types are `core`-class pure data (dst-lint clean — no
`Clock`/`Transport`/`Entropy`). The fan-out task uses the same `tokio` +
`parking_lot`-guarded broker the existing spawned tasks use. No proprietary
components; nothing to license.

---

## Constraints (must survive)

- `reconcile` stays **PURE + SYNC** `(desired, actual, view, tick) -> (Vec<Action>,
  View)`; the two new methods are additive, pure, declarative — no I/O, async,
  clock read, or DB handle. The `mod.rs:271` signature guard still passes.
- **Loop owns the clock** (`SimClock`/DST); `resync_schedule` reads nothing.
- **Intent/Observation non-substitutable**; the interest wiring reads the
  **Observation** change feed only.
- **No warm cache, no ADR-0036 amendment, no erasure rework** (all confirmed above).
- Resync + fan-out submits route through `broker.submit` (C-A1); the next-wake
  table re-arms at most once per period (C-A2) — the two conditions keeping OQ5's
  no-storm verdict true.

---

## Upstream changes

- **`overdrive-core`**: `+ ResyncSchedule/ResyncScope/Interest/RowKind/TargetFrom`
  (pure data); `+ Reconciler::resync_schedule` and `+ Reconciler::interests`
  (default-provided); `+ AnyReconciler::resync_schedule` and
  `+ AnyReconciler::interests` (forwarders).
- **`overdrive-control-plane`**: `spawn_convergence_loop` gains the next-wake
  table + scope resolver; the `register` path builds the cadence + interest
  tables; `+ spawn_interest_router` (new task); the four `exit_observer` submits
  are **deleted**; the four consumers gain `interests()` overrides.
- **No change** to `AnyState`, `AnyReconcilerView`, the `reconcile` dispatch match,
  `ObservationStore`, `EvaluationBroker`, ADR-0021/0035/0036.

---

## RATIFICATION NEEDED

- **RN-1 (LOCKED by Morgan):** cadence = `Option<ResyncSchedule { period: Duration,
  scope: ResyncScope }>`, `ResyncScope = { LocalNode }` (single variant at Phase 1;
  Titan recommended `{ LocalNode, WholeManaged }` but `WholeManaged` is dropped —
  see § "Locked signatures → Piece A" — as an unimplementable/unused arm, added
  additively when needed). Rationale in ADR-0081 § Alternatives (also rejects
  `resync_period()` and `next_evaluation(now)`).
- **RN-A1 (NEW — recommend KEEP / defer removal; deferral tracked at GH #271):**
  does the single-cut migration also remove the **reconciler-emitted**
  `Action::EnqueueEvaluation` (Mechanism 2) in favour of `interests()`?
  **Recommendation: NO for this feature.** It is the deeper #266 smell, but removing
  it changes reconcile bodies (`workload_lifecycle.rs`,
  `backend_discovery_bridge.rs`), rewrites their acceptance tests, and alters a
  first-tick-latency behaviour — that exceeds the ratified "surgical, separable,
  cheap" mandate (research §6.5). Keep `Action::EnqueueEvaluation` as the explicit
  reconciler→reconciler handoff primitive; migrate producer-push→consumer-pull for
  the reconciler-emitted enqueues in a separate, independently-drivable slice (GH
  #271) only if the open/closed hygiene is later judged worth the blast radius.

---

## Deferrals (issue-tracked)

1. **B-1 warm reflector-`Store`** — tracked at **GH #270** (RN-2 ratified to
   B-2). Not a new deferral.
2. **Facet-2 hydration-erasure rework (THIRD deferral) — tracked at GH #272.** The
   research candidates B (erased-trait `downcast`) / C (co-locate hydration) / D
   (per-type monomorphization) and Open Questions 1/2/6 are untouched by this
   design — hydration stays runtime-owned (ADR-0036). **Trigger / forcing
   function:** open-world extensibility is actually needed (third-party / WASM
   reconcilers, >1yr out per baseline #3 — the scoping forcing function), OR the
   `AnyState`/`AnyReconcilerView`/`hydrate_actual` five-sites-per-reconciler edit
   becomes a demonstrated maintenance sink at a materially larger reconciler count.
   Until a trigger fires, the closed-world enum is the right tool for the bounded
   ~10–12 first-party set (research §5, Titan §9 flags per-type monomorphization as
   a *regression* of the single-loop/single-clock DST design).
3. **RN-A1 — `Action::EnqueueEvaluation` → `interests()` migration — tracked at GH
   #271.** The reconciler-emitted enqueue-migration (above) is deferred as its own
   independently-drivable slice; recommendation is KEEP for this feature.

**RN-3 (Titan) — MOOT under B-2.** With no warm cache there is nothing to bound;
the Phase-2 cardinality-bounding debt dissolves. Recorded, not carried.

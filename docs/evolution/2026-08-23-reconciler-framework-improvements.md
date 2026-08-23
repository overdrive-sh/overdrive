# Evolution — `reconciler-framework-improvements` (GH #266)

**Finalized:** 2026-08-23
**Feature branch:** `marcus-sa/reconciler-framework-improvements`
**Requirements / design SSOT:** [`ADR-0084 — Cadence and event-interest as first-class, object-safe Reconciler declarations`](../product/architecture/adr-0084-reconciler-cadence-and-interest-declarations.md)
**GH issue:** [#266](https://github.com/overdrive-sh/overdrive/issues/266) (still OPEN — see § "Status & follow-ups")

> **ADR renumber note.** This feature's ADR was drafted under the working number
> **ADR-0081** (early commit messages say "ADR-0081 §…"). ADR-0081/0082/0083 were
> taken by concurrently-landing work, so it was renumbered to **ADR-0084** on
> landing. Wherever a commit message references "ADR-0081" for this feature, read
> **ADR-0084**. (ADR-0081 today is an unrelated document —
> `three-ending-classes-platform-reclamation-and-artifact-disposal`.)

---

## 1. Summary

Two additive, object-safe methods on the `Reconciler` trait that let a reconciler
**declare** its own resync cadence and its own event interests, replacing two
"the framework special-cases me" smells with declarative data the runtime reads:

- **Piece A — cadence.** `resync_schedule(&self) -> Option<ResyncSchedule>`
  (default `None`). The convergence loop builds a per-reconciler next-wake table
  from the declarations and drives level-triggered resync through
  `broker.submit`, once per period. This removes the cadence-constant /
  reconciler-name / hardcoded-target-scheme from the loop.
- **Piece B — event-interest.** `interests(&self) -> &'static [ObservationRowKind]`
  (default `&[]`). A new `spawn_interest_router` task (List-then-Watch over the
  existing `subscribe_all_events` change feed) fans a row change out to every
  interested reconciler via `broker.submit`. This replaces the four scattered
  `exit_observer` producer-submits that named their four consumers by hand
  ("producer names consumer") with a single declarative fan-out.

Neither piece touches `AnyState` / `AnyReconciler` type-erasure, and neither
amends ADR-0036 — `reconcile` stays pure/sync and hydration stays runtime-owned
(per-tick from the CR-SQLite replica; **no warm cache** at this phase).

---

## 2. Business / technical context

The convergence loop was accreting per-reconciler special cases: a hardcoded
cadence + target scheme for the (incoming) `vm-reclamation` sweep, and the
exit-observer path naming four specific consumers to nudge on an accepted
`alloc_status` write. Both are the same anti-pattern — the framework knowing
about specific reconcilers instead of reconcilers declaring their needs.

#266 makes the two needs **declarative and object-safe**:

- A reconciler that needs a periodic level-triggered sweep declares a
  `resync_schedule`; the loop owns the clock and the `NodeId` and resolves the
  scope. This is the K8s two-knob split (edge-triggered broker as the primary
  trigger; periodic level-triggered resync as the safety net) ported directly.
- A reconciler that should wake when an observation row changes declares its
  `interests()`; the interest-router derives the target inline from the changed
  row and submits. The **interest declaration IS the partition key** (SD-6):
  non-empty interests ⟺ row-backed ⟺ event-woken; empty ⟺ host-backed ⟺
  resync-only.

Landing this **before** vm-reclamation's host-state (8th) reconciler arrives is
deliberate — the partition is a forward provision expressed through the
declaration, not a retrofit.

---

## 3. Key decisions (ratified)

| # | Decision | Rationale / consequence |
|---|---|---|
| **RN-2 = B-2** | Piece B ships **interests-only, NO warm cache**. | The warm reflector-`Store` (B-1) is deferred to **GH #270**. Latency cost of per-tick hydration is ~nil (the cache would save a local SQL query, not a network hop), so this is a cheap, honest cut. Consequence: no `ReflectorApplyBeforeHydrate` invariant, no Phase-2 cardinality-bounding debt (RN-3 dissolves). |
| **RN-1 LOCKED** | Cadence = `Option<ResyncSchedule { period: Duration, scope: ResyncScope }>`, `ResyncScope = { LocalNode }` (single variant). | `WholeManaged` was **dropped** from the Phase-1 enum (diverging from the SYSTEM-layer recommendation): its resolver needs the #270 managed-target set, so shipping it now would force an unimplementable `todo!` arm — the unused-surface smell. Single-variant keeps `resolve_scope` total and fully exercised; `WholeManaged` is added additively (one variant + one arm) when a reconciler declares it. |
| **Lean Piece B re-cut (2026-08-23)** | The interest surface collapsed to a single `&'static [ObservationRowKind]` keyed off a **complete, `ObservationRow`-owned** discriminant. | `Interest` / `RowKind` / `TargetFrom` / `classify` / `derive_target` were **dropped** as over-abstraction (a user design review). `ObservationRow::kind()` (total, no-wildcard, 8 arms) + router-local inline target derivation replace them. The drift-closure now lives on the row type it describes — a new `ObservationRow` variant fails to compile at `kind()` until consciously mapped. |
| **Open Question 5 RESOLVED** | Resync submits LWW-coalesce through the broker; no eval-storm. | Bounded by managed cardinality, not event rate. Two constraints keep the verdict true: **C-A1** resync routes through `broker.submit` (never a side channel), **C-A2** the next-wake table re-arms ≤ once per period. |
| **SD-6 — interest = partition key** | Empty `interests()` ⟺ host-backed ⟺ resync-only; non-empty ⟺ row-backed ⟺ event-woken. | Both partitions have a **realized** backstop (`#270`-free): the row-backed partition's backstop is the interest-router periodic relist (below); the host-backed partition's backstop is Piece A `resync_schedule`. |
| **ADR-0036 STANDS** | Hydration stays runtime-owned, per-tick from the replica; `reconcile` stays pure/sync. | Neither piece adds an async/clock/DB surface to `reconcile`; the `reconcilers/mod.rs` signature guard still passes. The Facet-2 hydration-erasure rework is a third deferral (**GH #272**). |
| **Interest-router periodic relist (2026-08-23 amendment; Approach B)** | The router gains a **clock-injected, unconditional periodic relist** (K8s SharedInformer `resync-period`). | Level-triggered backstop for the row-backed partition, enumerated from the snapshot read — `#270`-free. See § 5. **Rejected Approach A** (bounded retry-on-error only) — closes the reported transient-error case but leaves no periodic backstop and does not realize SD-6; B subsumes it at equal surface cost. |

**Deferrals — all issue-tracked, all with user approval:**

- **GH #270** — B-1 warm reflector-`Store` cache (RN-2 ratified to B-2).
- **GH #271** — `Action::EnqueueEvaluation` → `interests()` migration (RN-A1;
  recommendation KEEP for this feature — removing it changes reconcile bodies +
  a first-tick-latency behaviour, exceeding the "surgical, separable, cheap"
  mandate). `handlers.rs:76` (IntentStore edge) and `reconciler_runtime.rs`
  self-re-enqueue are untouched by design — the fan-out is over the Observation
  change feed only.
- **GH #272** — Facet-2 hydration-erasure rework (research candidates B/C/D, OQ
  1/2/6). Hydration ownership is unchanged; the closed-world enum is the right
  tool for the bounded ~10–12 first-party reconciler set until an open-world
  trigger fires.

---

## 4. Work completed

Delivered as 5 TDD steps (RED → GREEN → COMMIT), all `DONE`. Traceability to the
DISTILL scenarios (`S-266-01..22`) is recorded in `deliver/roadmap.json`.

| Step | Scope | Primary commit(s) |
|---|---|---|
| **01-01** | Piece A trait surface — `resync_schedule` hook + `ResyncSchedule` / `ResyncScope` pure data + total `resolve_scope(scope, node_id) -> Vec<TargetResource>` (`LocalNode → [node/<id>]`). | `de158bfc` |
| **01-02** | Piece A loop wiring — `spawn_convergence_loop` per-reconciler next-wake `BTreeMap<ReconcilerName, UnixInstant>` drives resync via `broker.submit` once per period; cadence/target hardcode deleted. | `c48af796`, `055b7627` |
| **02-01** | Piece B trait surface — `interests` hook + complete `ObservationRowKind` (8 variants, `Ord`+`Hash`, owns `as_str`) beside `ObservationRow` + total no-wildcard `ObservationRow::kind()`. Landed the lean re-cut over the original `Interest`/`RowKind`/`TargetFrom` draft. | `820c0b99` → `08fd4463` → `1e32661a` |
| **02-02** | Piece B interest-router + `run_server` vertical slice — `spawn_interest_router` (List-then-Watch) wired into the production entry `run_server_with_obs_and_driver`; `S-266-01` asserts the entry SPAWNS the router (structural boot check, no hand-called spawn fn). | `ee72d660` |
| **02-03** | Piece B single-cut migration — DELETE the four `exit_observer` submits (`234/254/295/320`) + ADD `interests() = &[ObservationRowKind::AllocStatus]` on the four consumers (`workload-lifecycle`, `backend-discovery-bridge`, `service-lifecycle`, `svid-lifecycle`); pre-existing "enqueues bridge/service/svid" tests rewritten to fan-out equivalence in the same commit. | `78c030f6` |

**Post-delivery amendment / reconciliation commits (same branch):**

- `a74ae731` — reconcile with `main` + unify `vm-reclamation` onto the Piece A
  cadence hook (the motivating consumer now rides the declarative path).
- `288ddcae` — interest-router **unconditional periodic relist** closes the
  quiet-stream boot-LIST-error liveness gap (§ 5).
- `0068f36f` — reconcile `brief.md` SSOT with the vm-reclamation unification.
- `fed01877` — CI: restore mold install in `bpf-build` (empty `RUSTFLAGS`
  override never stripped the flag).

**Shipped surface (verified in tree at finalize):**

```rust
// crates/overdrive-core/src/reconcilers/mod.rs
fn resync_schedule(&self) -> Option<ResyncSchedule> { None }   // trait default
fn interests(&self)        -> &'static [ObservationRowKind] { &[] } // trait default
pub struct ResyncSchedule { pub period: Duration, pub scope: ResyncScope }
pub enum   ResyncScope    { LocalNode }
pub fn resolve_scope(scope: ResyncScope, node_id: &NodeId) -> Vec<TargetResource>
// AnyReconciler::resync_schedule / ::interests — one forwarding arm apiece

// crates/overdrive-core/src/traits/observation_store.rs
pub enum ObservationRowKind { AllocStatus, NodeHealth, ServiceHydration,
    ServiceBackend, ReconcileConflict, IssuedCertificate, WorkflowTerminal, Signal }
impl ObservationRow { pub const fn kind(&self) -> ObservationRowKind { /* 8 arms, no _ */ } }

// crates/overdrive-control-plane/src/lib.rs
pub fn spawn_interest_router(/* …, clock: Arc<dyn Clock>, relist_period: Duration, shutdown */)
const INTEREST_ROUTER_RELIST_PERIOD: Duration = Duration::from_secs(30);
```

No public API beyond ADR-0084 was introduced.

---

## 5. Issues encountered & lessons learned

**The lean re-cut (over-abstraction caught in design review).** The first Piece B
trait surface (`820c0b99`) shipped `Interest` / `RowKind` / `TargetFrom` +
exhaustive `classify` + total `derive_target` — a per-interest target-derivation
strategy indirection. A user design review (2026-08-23) judged it over-abstracted
for a Phase-1 world where every routed row kind maps to a workload target and all
four consumers key identically. It was collapsed (`08fd4463`, `1e32661a`) to a
single complete-discriminant slice + router-local inline derivation. **Lesson:**
a *complete discriminant of an existing closed enum* is not speculative surface
(every variant already exists — enumerating them is a total projection); a
*one-strategy `TargetFrom` abstraction* is. The drift-closure belongs on the row
type (`ObservationRow::kind()`), not in a parallel classifier.

**The quiet-stream liveness gap (fixed in-scope, no deferral).** A code review
found a real liveness gap after the router landed: on a transient
`alloc_status_rows()` read error the boot LIST logs-and-skips, and on a **quiet
stream** (a `serve` restart, where `register` submits no initial evaluation so
the boot LIST is the only boot-time wake) the four interest consumers were then
**never woken** for the boot snapshot until an unrelated write. SD-6's "resync as
backstop" was *unrealized* for the row-backed partition (only `vm-reclamation`
overrides `resync_schedule`; a per-`workload` backstop needs #270's
`WholeManaged`). Rather than defer, the router gained an **unconditional
clock-injected periodic relist** (`288ddcae`): List → Watch → relist every
`relist_period`, re-armed ≤ once/period, NOT reset by `Row` arrivals; the
`Lagged`-relist became a special case of it. Worst-case boot-error wake latency
is now **one `relist_period` (≤ 30 s)**, bounded (was unbounded). Single-clock
DST is preserved — the router reads the same `config.clock` instance the loop
reads. **Lesson:** "resync as a backstop" is only real if *something actually
resyncs the partition*; a partition whose only backstop lives behind a deferred
dependency has no backstop.

**`WholeManaged` dropped to keep `resolve_scope` total.** Shipping the second
scope variant early would have forced an unimplementable resolver arm. Keeping
the enum single-variant kept the pure function total and fully exercised —
additive-when-needed beats speculative-and-half-built.

**Vertical-slice discipline held.** `S-266-01` asserts that the production entry
`run_server_with_obs_and_driver` *spawns* the router (a structural boot check
that the task is live after the entry returns) — the test hand-calls no spawn fn
and hand-assembles no router, honoring "no test installs the one production call
site the feature omitted."

---

## 6. Status & follow-ups

- **GH #266 remains OPEN.** The issue's title frames the broader "reconcilers own
  their hydration" ambition; this feature delivered the cadence + event-interest
  declarations (RN-2 = B-2) and split the warm-cache / hydration-erasure
  ambitions into #270 / #272. Close or re-scope #266 per those follow-ups — not
  automated here.
- **ADR-0084 status is `Proposed`.** Its design is fully implemented and merged to
  the feature branch. Flipping it to `Accepted` is an ADR-lifecycle edit that
  goes through the architect agent (not done inline at finalize).
- **Deferrals live at GH #270 / #271 / #272** — cited above.

---

## 7. Artifact map

| Artifact | Location | Disposition |
|---|---|---|
| Design SSOT | `docs/product/architecture/adr-0084-…md` | Already permanent (no migration). |
| SYSTEM+APPLICATION feature-delta | `docs/feature/reconciler-framework-improvements/feature-delta.md` | Preserved in the (retained) feature workspace; key content distilled above. |
| APPLICATION wave decisions | `…/design/wave-decisions.md` | Preserved; decisions distilled into § 3. |
| DISTILL scenarios `S-266-*` | `…/distill/test-scenarios.md` | Preserved; traceability in `deliver/roadmap.json`. |
| Step plan + execution log | `…/deliver/roadmap.json`, `…/deliver/execution-log.json` | Preserved (repo convention keeps finalized `deliver/`). |
| Research | `docs/research/architecture/cqrs-structural-mechanism-reconciler-framework-research.md` | Already permanent. |

The `docs/feature/reconciler-framework-improvements/` workspace is **retained**
(the nWave wave-matrix derives status from it, per repo convention — every
finalized feature keeps its workspace). This evolution doc is the summary; the
feature directory is the history.

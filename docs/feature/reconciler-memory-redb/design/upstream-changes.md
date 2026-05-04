# Upstream changes — reconciler-memory-redb DESIGN

This DESIGN supersedes load-bearing prior decisions. The following
artifacts must be updated **as part of landing this feature** (DELIVER
wave) — not deferred:

---

## A. ADRs to supersede

### A1. ADR-0013 — Reconciler primitive: trait + runtime + libSQL private memory

**Status change**: Accepted → **Superseded by ADR-0035**.

**Load-bearing claims overturned by this DESIGN**:

- §2 (Trait shape — pre-hydration pattern): the four-method shape
  (`name` / `hydrate` / `reconcile` / `persist` per the issue-139
  in-flight roadmap) collapses to two methods (`name` / `reconcile`).
  `hydrate` and `persist` move into the runtime, not into the trait.
- §2a (Dyn-compatibility strategy — `enum AnyReconciler`): the
  `async fn` driver of dyn-incompatibility goes away. The
  `AnyReconciler` enum is *retained* for the typed-View-associated-
  type erasure (the rule survives), but the workaround's
  *necessity* changes — once this DESIGN ships, a future
  `Box<dyn Reconciler>` becomes a real option.
- §2b (Runtime hydrate-then-reconcile contract): the per-tick
  `reconciler.hydrate(target, db).await` step is deleted. The
  runtime's `bulk_load` runs once at register; steady-state ticks
  read from the in-memory `BTreeMap`.
- §5 (libSQL per-primitive path derivation): the per-reconciler path
  shape `<data_dir>/reconcilers/<name>/memory.db` is replaced by a
  single per-node `<data_dir>/reconcilers/memory.redb` file.
- §6 (Per-primitive storage — libSQL via `LibsqlHandle`): libSQL is
  no longer the reconciler-memory backend. The `LibsqlHandle`
  newtype is deleted in its entirety.
- Alternative G (Sync `reconcile` with `block_on` over libsql's
  async API) — the rejection rationale becomes academic; libsql
  is no longer in the trait surface.

**What survives**:

- §1 (Module ownership) — `Reconciler` trait still in
  `overdrive-core::reconciler`; `ReconcilerRuntime` still in
  `overdrive-control-plane::reconciler_runtime`.
- §2c (Time injection via `TickContext`) — entirely preserved.
- §3 (Action enum) — entirely preserved.
- §4 (`ReconcilerName` newtype) — preserved; reused as the redb-
  table-name source.
- §7 (Slice 4 ships whole) — historical; not relevant to this
  DESIGN.
- §8 (Evaluation broker shape) — entirely preserved.
- §9 (`noop-heartbeat` reconciler) — preserved; trivially
  satisfies the new shape.
- Enforcement infrastructure (dst-lint, trybuild, DST invariants)
  — preserved; new invariants added (see ADR-0035 §D6).

ADR-0035 quotes each load-bearing claim and marks it superseded
inline.

### A2. ADR-0021 — Reconciler `State` shape: per-reconciler typed `AnyState` enum mirroring `AnyReconcilerView`

**Status change**: Accepted → **Amended by ADR-0036** (NOT
superseded — the State shape decision survives; only the View-
storage interaction needs an amendment).

**Load-bearing claims that need amendment language**:

- §3 (Hydration owned by the runtime, not the reconciler): the
  reconciler's `hydrate(target, db)` method is removed; the runtime
  no longer awaits it. `hydrate_desired` and `hydrate_actual`
  remain on `AnyReconciler` (the runtime owns intent + observation
  reads); the third async surface (the per-reconciler `View` read)
  is replaced by a synchronous `BTreeMap::get`.

The `AnyState` enum, the `JobLifecycleState` shape, the per-
reconciler-State convention, and the compile-time exhaustiveness
contract all survive verbatim.

ADR-0036 documents the amendment with a one-paragraph delta. If the
user prefers a single combined ADR-0035 covering both
supersession + amendment, the DESIGN can be re-emitted with that
shape — the architect's recommendation is to keep them separate
because the structural decisions are independent (the View storage
shape and the State projection shape are orthogonal concerns).

---

## B. Whitepaper amendments

### B1. §17 — Storage Architecture, Reconciler Memory tier

**Replace** the current paragraph:

> ### Reconciler Memory — libSQL (per-reconciler)
>
> Each reconciler gets a private libSQL database for stateful memory
> across reconciliation cycles — restart tracking, placement history,
> resource sample accumulation. ...

**With** the wave-decisions.md §D5 paragraph:

> ### Reconciler Memory — redb (one file per node, runtime-owned key
> space, in-memory hot copy)
>
> Each reconciler's typed `View` is persisted in a single per-node
> redb file under `<data_dir>/reconcilers/memory.redb`, with one
> redb table per reconciler kind keyed on the `TargetResource`. The
> runtime bulk-loads every persisted view at register time into a
> per-reconciler in-memory `BTreeMap<TargetResource, View>`; this
> map is the steady-state read SSOT for `reconcile`. Writes flow
> through as `View → CBOR bytes → redb transaction (one fsync) →
> in-memory map update`, in that order. Reconcile remains a pure
> synchronous function over `(desired, actual, view, tick)` — the
> `View` is hydrated by the runtime's BTreeMap lookup, not by the
> reconciler. CBOR via `ciborium` carries the wire format; serde
> with `#[serde(default)]` handles additive schema evolution.
> libSQL is retained for incident memory and DuckLake catalog
> (Phase 3+), not for reconciler memory.

The architect performs this edit directly (per the user's
instruction not to dispatch a separate system-designer pass for one
paragraph). Co-located with the brief.md update.

### B2. §18 — Reconciler and Workflow Primitives

**Update** the trait shape example to reflect the collapsed
surface:

```rust
trait Reconciler: Send + Sync {
    type State: Send + Sync;
    type View:  Serialize + DeserializeOwned + Default + Clone + Send + Sync;
    fn name(&self) -> &ReconcilerName;
    fn reconcile(
        &self,
        desired: &Self::State,
        actual:  &Self::State,
        view:    &Self::View,
        tick:    &TickContext,
    ) -> (Vec<Action>, Self::View);
}
```

The narrative around "pure function over `(desired, actual, view,
tick)`" is unchanged. The "async hydrate / persist" bullet points
in the §18 prose (where they appear in the Reconciler-primitive
discussion) are removed; the section gains one sentence:

> Storage of the `View` is the runtime's responsibility. The runtime
> bulk-loads each reconciler's persisted views into an in-memory
> `BTreeMap<TargetResource, View>` at register time and writes
> through to a per-node redb file on each successful tick. The
> reconciler author derives `Serialize + Deserialize + Default +
> Clone` on the `View` struct and has no further storage surface to
> implement.

---

## C. Project-rule edits

### C1. `.claude/rules/development.md` § "Reconciler I/O"

This section currently documents the four-method trait shape
(`migrate` / `hydrate` / `persist` / `reconcile`) as the canonical
pattern, with extensive prose on async-await rules per method, the
pre-hydration pattern, and `LibsqlHandle` semantics. **All of this
needs rewriting** to reflect the collapsed trait.

The replacement section is shorter — the rule is simpler:

- `reconcile` is sync, pure, no `.await`, no I/O, no wall-clock
  outside `tick.now`. (Unchanged.)
- The `View` is a typed input — derive `Serialize + Deserialize +
  Default + Clone`. The runtime owns persistence.
- External I/O still flows through `Action::HttpCall` (the
  documented pattern is unchanged); the example block is preserved
  except that `migrate` and `hydrate` disappear from the example
  reconciler.
- Schema management: the runtime handles redb table creation. The
  author does not write CREATE TABLE.
- Schema evolution: `#[serde(default)]` on additive fields; a
  versioned envelope + custom upcaster on breaking changes (Phase
  3+ if it materialises; Phase 1 has none).

The `.claude/rules/development.md` § "Reconciler I/O" rewrite is
flagged here for DELIVER to land; the architect does NOT edit
project-wide rule files in DESIGN — those edits accompany the
implementation PR.

### C2. `.claude/rules/development.md` § "Persist inputs, not derived state"

**No change.** The rule continues to apply identically. The View
struct shape is still the inputs (`restart_counts`,
`last_failure_seen_at`, etc.); `next_attempt_at` is still
recomputed every tick from inputs and the live policy. The storage
mechanism is irrelevant to the rule.

### C3. `.claude/rules/development.md` § "Port-trait dependencies"

**No change.** The new `ViewStore` trait sits in
`overdrive-control-plane` (not core), with a `RedbViewStore` host
adapter and a `SimViewStore` sim adapter. The constructor-required
discipline applies — `ReconcilerRuntime::new` takes
`Arc<dyn ViewStore>` as a mandatory parameter, no builder, no
default, no production-binding fallthrough.

### C4. `.claude/rules/testing.md` — Tier 1 DST invariants

The new invariants from wave-decisions.md §D6 add to the Tier 1
catalogue:

- `ViewStoreRoundtripIsLossless` — proptest-backed, runs under
  `cargo xtask dst`.
- `BulkLoadIsDeterministic` — runs under `cargo xtask dst`.
- `WriteThroughOrdering` — runs under `cargo xtask dst` with
  `SimViewStore`'s injected fsync-failure shape.

The existing `ReconcilerIsPure` invariant continues to fire; the
contract is unchanged.

DELIVER to land the invariant catalogue update in
`crates/overdrive-sim/src/invariants/`.

---

## D. In-flight branch status

### D1. The current branch supersedes its own roadmap

`docs/feature/reconciler-memory-redb/deliver/roadmap.json` (7
steps, revision history shows it has already been revised once) is
**entirely superseded** by this DESIGN. The roadmap was built
against the *previous* design (libSQL `migrate` lifecycle hook,
eager `LibsqlHandle::open` at register, etc.); none of those steps
make sense under the new design.

DELIVER must **discard** this roadmap and replan from scratch
against ADR-0035 + ADR-0036 + the brief.md update + the whitepaper
§17/§18 amendments.

### D2. The 6 commits on `marcus-sa/libsql-view-cache` need a refactor decision

| Commit | Status under the new design |
|---|---|
| `1ea4ec2` feat(control-plane): migrate streaming + handlers off view_cache | **Keep**. Removing the in-memory `view_cache` shadow is correct under the new design too — the new in-memory `BTreeMap<TargetResource, View>` is a different structure with different invariants (steady-state SSOT, not a tick-grain cache). |
| `7f30519` docs(rules): correct cargo-mutants post-run guidance | **Keep**. Unrelated docs fix. |
| `cf0cd12` feat(control-plane): wire run_convergence_tick to libSQL hydrate/persist | **Discard**. This commit IS the previous design; the new design removes both `hydrate` and `persist` from the trait and replaces the convergence-tick wiring entirely. |
| `277db3f` docs(rules): document Reconciler::migrate lifecycle hook | **Discard**. The `migrate` lifecycle hook is removed; the rule documentation describes a method that no longer exists. |
| `9d68704` refactor(core): extract DDL into Reconciler::migrate lifecycle hook called once at register | **Discard**. Same — `migrate` is removed. |

The user owns the call between **refactor in place** and **reset**.
The architect's recommendation (per wave-decisions.md §O1) is
**reset** — cleaner history, the new DELIVER roadmap plans against
`main` rather than against a partially-undone branch. Decision
required before DELIVER planning starts.

### D3. ADR-0037 lands alongside the ADR-0035 reset; new DELIVER roadmap must wire `terminal` from day one

ADR-0037 (reconciler emits typed `TerminalCondition`; streaming
forwards it; `LifecycleEvent` no longer projects reconciler-private
View state) was authored 2026-05-03 to codify the recommendation in
`docs/research/control-plane/issue-139-followup-streaming-restart-budget-research.md`
(candidate (c)). It composes structurally with ADR-0035: under the
collapsed trait + runtime-owned redb backing, the View blob is
private to the runtime; the layering rule "no derived projections of
reconciler memory on `LifecycleEvent`" becomes structurally enforced
rather than convention.

**Sequencing constraint for the new DELIVER roadmap**: the roadmap
that follows the reset MUST wire `terminal` from day one. Concretely:

- The `Action` enum gains `terminal: Option<TerminalCondition>` on
  the relevant variants (`StopAllocation`, the synthetic
  Failed-row action shape per ADR-0023) in the **same step** that
  introduces the typed-View runtime contract. NOT a follow-up step.
- `JobLifecycle::reconcile` computes `terminal` from
  `view.restart_counts`, `view.last_failure_seen_at`, and the live
  `RESTART_BACKOFF_CEILING` policy; stamps it on the Action.
- The action shim writes `AllocStatusRow.terminal` and emits
  `LifecycleEvent.terminal` in the same dispatch.
- `streaming.rs::check_terminal` reads `event.terminal` directly;
  `streaming.rs::lagged_recover` drops its `restart_count_max_hint`
  parameter and reads `latest.terminal` off the observation row.
- `exit_observer.rs` emits `terminal: None` (replacing the
  step-02-04 `restart_count_max: 0` literal).
- The `LifecycleEvent.restart_count_max: u32` field is deleted in
  the same step. No parallel-fields transitional period — the reset
  absorbs the change.

**Do NOT plan a roadmap that lands ADR-0035 first and `terminal`
second.** The two changes share a publication boundary (the Action
→ row → event path); landing them separately means the action shim
is rewritten twice and the `restart_count_max: 0` literal-meaningless
smell persists between the two PRs. One coherent step.

The DESIGN/wave-decisions.md document for this feature is locked and
this point is recorded here, not by reopening that file.

---

## E. Documentation co-edits

### E1. `docs/product/architecture/brief.md` — Application Architecture section

The architect performs this edit as part of this DESIGN wave. The
brief.md updates:

- **§19 (Reconciler primitive)** — rewritten to describe the
  collapsed trait, the `ViewStore` port, and the
  bulk-load-then-write-through runtime contract.
- **§17 ADR table row** — `0013` marked Superseded by 0035; new
  rows for ADR-0035 and ADR-0036.
- **C4 Container diagram (§Container diagram in brief.md)** —
  updated: `libSQL files` container → `redb file (memory.redb)`;
  `LibSqlProvisioner` component → `RedbViewStore` (or removed if
  the diagram is at L2 abstraction).
- **C4 Component diagram (convergence-loop)** — `JobLifecycleView
  libSQL DB` → `JobLifecycleView redb table`; `Reads
  JobLifecycleView (async)` arrow → `Bulk-loads at register;
  reads from in-memory BTreeMap on tick`.
- **§24 (State shape — per-reconciler `AnyState` enum)** — single
  paragraph note that the per-reconciler `hydrate(target, db)`
  remit referenced in the original section is removed under
  ADR-0035; the runtime-owned `hydrate_desired` / `hydrate_actual`
  surfaces are unchanged.
- **Changelog entry** — dated 2026-05-03 documenting this DESIGN
  wave and naming ADR-0035 + ADR-0036.

### E2. C4 diagrams for this DESIGN

The wave-decisions.md output includes the new C4 Component diagram
for the reconciler subsystem (in `docs/product/architecture/brief.md`
embedded Mermaid block). The Container diagram amendment is
described in §E1; a fresh full Container diagram is NOT re-emitted
in this DESIGN (the change is local to one container row and one
edge).

---

## F. Sequencing constraint for DELIVER

The DELIVER wave **must** land the following in this order to keep
the workspace compilable on every commit:

1. Add `ciborium` to `[workspace.dependencies]`.
2. Introduce the new `ViewStore` trait + `RedbViewStore` adapter +
   `SimViewStore` adapter, behind a `#[allow(dead_code)]` because
   nothing yet calls them.
3. Migrate `ReconcilerRuntime::register` to call `ViewStore::probe()`
   then `ViewStore::bulk_load`; stash the in-memory `BTreeMap`
   alongside `AnyReconciler`.
4. Migrate the tick loop to read from the BTreeMap and write
   through `ViewStore::write_through`; remove the `hydrate` /
   `persist` calls.
5. Remove `migrate` / `hydrate` / `persist` from the `Reconciler`
   trait; remove the corresponding arms in `AnyReconciler` dispatch.
6. Delete `LibsqlHandle`; delete `libsql_provisioner` module;
   delete the per-reconciler libSQL files from the Phase-1
   `data_dir` layout (operator note for any existing dev installs).
7. Update `.claude/rules/development.md` § "Reconciler I/O" to the
   new shape.
8. Update whitepaper §17 / §18 (the amendments described in B1 /
   B2).
9. Mark ADR-0013 Superseded by ADR-0035; emit ADR-0035 + ADR-0036
   files (the architect emits these as part of this DESIGN —
   DELIVER does not author ADRs, just lands them in the file
   tree if they are not already committed).

---

## G. What DESIGN deliberately does NOT touch

- **Workflow primitive.** The §18 workflow trait is unrelated; this
  DESIGN does not amend it. Workflows continue to use libSQL
  journals per §18 (Phase 3+).
- **Incident memory.** Whitepaper §12 and §17's incident-memory
  tier continue to call for libSQL. The libSQL workspace dep stays.
- **Telemetry catalog (DuckLake).** Whitepaper §17's DuckLake
  catalog uses libSQL for the metadata layer; unaffected.
- **The roadmap.** DESIGN does not produce a roadmap (per the
  architect's role-boundary). DELIVER (`/nw-roadmap` or
  `/nw-deliver`) plans the new roadmap against this DESIGN's output
  + the user's answer to O1.

---

*End of upstream-changes.md.*

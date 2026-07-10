# ADR-0074 — Consolidate the placement scheduler into `overdrive-core`; delete the orphaned `overdrive-scheduler` crate; retype `schedule` to the kind-agnostic resource envelope

## Status

Accepted. 2026-07-11. Decision-makers: user (ratified — consolidation,
retype, and single-cut deletion all decided before this record); Morgan
(recording the decision + pinning the implementation contract). Mode:
propose (recording a user-ratified decision). Tags: phase-1,
first-workload, application-arch, crate-consolidation.

**Supersedes ADR-0024** ("Dedicated `overdrive-scheduler` crate
(class `core`); dst-lint scope expansion"). ADR-0024's entire decision
— Decisions §1 (create the crate), §2 (workspace registration), §3
(module surface *in the crate*), §4 (dependency direction
`overdrive-control-plane → overdrive-scheduler → overdrive-core`), and
§5 (dst-lint scope via a separate crate) — is overturned. The one
technical property ADR-0024 actually secured (mechanical dst-lint
enforcement of the BTreeMap-only + banned-API discipline on the
placement code) is **preserved** by this decision through a different
mechanism, enumerated in *Consequences → dst-lint discipline preserved*
below.

## Context

ADR-0024 extracted the pure placement scheduler into a dedicated
`core`-class crate `overdrive-scheduler`, on the stated premise (its
§4) that the dependency graph would be
`overdrive-core ← overdrive-scheduler ← overdrive-control-plane` and
that "the scheduler is consumed by the `JobLifecycle` reconciler
(US-03)." **That wiring never landed.** The four facts below are each
verified against the live tree at this ADR's date:

1. **The reconciler shipped inside `overdrive-core`, not
   control-plane.** The placement consumer is `WorkloadLifecycle`
   (`crates/overdrive-core/src/reconcilers/workload_lifecycle.rs`), a
   `core`-class reconciler — not a `overdrive-control-plane` module as
   ADR-0024 assumed.

2. **The dependency edge ADR-0024 required is impossible.**
   `overdrive-core` cannot depend on `overdrive-scheduler`, because the
   scheduler already depends on `overdrive-core` — the edge would
   invert into a cycle. So the reconciler could not call the crate. It
   **hand-inlined** the placement instead: `first_fit_place`
   (`workload_lifecycle.rs:908`) + `node_free_capacity`
   (`workload_lifecycle.rs:927`), called at `workload_lifecycle.rs:806`.

3. **The crate is orphaned.** Nothing in the workspace depends on
   `overdrive-scheduler` in a non-test, non-doc path: no `src/` file in
   any crate names `overdrive_scheduler`; `overdrive-control-plane`
   does **not** depend on it; only the crate's own `Cargo.toml`,
   its own tests, one xtask test, and stale doc comments reference it.
   `schedule` is exercised only by the crate's own acceptance tests.

4. **The two copies have already drifted.** The crate's `schedule`
   returns rich diagnostics — `PlacementError::{NoCapacity{needed,
   max_free}, NoHealthyNode}`. The inlined `first_fit_place` returns a
   bare `Option<NodeId>` with none of that. Two implementations of one
   algorithm, diverging, is precisely the drift the trait-contract and
   single-SSOT disciplines exist to prevent.

This is the "isolated mechanism / dead code wearing a green test suite"
that `CLAUDE.md` § "Build vertical slices through production entry
points — never isolated mechanisms" rejects. The mechanism (the crate)
was built and unit-tested; the wiring that would connect it to the one
production consumer (the reconciler) was never landed and *cannot* be
landed without inverting the dependency graph. The honest correction is
to move the pure surface next to its real consumer and delete the
orphan.

### Coupled sub-decision — the placer is a kind-agnostic resource placer mistyped as `Job`

The placement function currently takes `&Job` (= `JobV1`, the
run-to-completion **kind payload**) but reads only `.resources` — a
field all three workload kinds (`JobV1` / `ServiceV1` / `ScheduleV1`)
carry. The `WorkloadLifecycle` reconciler funnels **every** kind
through this placement via a kind-agnostic projection
(`WorkloadLifecycleState.job: Option<Job>`). The function is therefore
a **generic any-workload placer mistyped as a Job-kind consumer** — the
`Job` parameter is a misnomer that reads only the resource envelope.

The user ratified retyping the placement function to take the resource
envelope directly (`&overdrive_core::traits::driver::Resources`) so
that it is kind-agnostic at the type level and the `Job` misnomer is
gone. This ADR pins that signature as part of the implementation
contract.

## Decision

**Consolidate the pure placement scheduler into `overdrive-core` as a
dedicated module, retype `schedule` to take the resource envelope
(`&Resources`) instead of `&Job`, make the reconciler's production path
the sole caller of that one function, and delete the
`overdrive-scheduler` crate entirely — single-cut, no shim.**

The five numbered clauses of the *Implementation contract* section
below are the binding, design-sensitive surface. Per `CLAUDE.md`
§ "Implement to the design — never invent API surface", the crafter
implements exactly the module path, item names, and `schedule`
signature pinned there — no new public surface, no extra variants, no
alternate signature to "make tests green."

## Alternatives considered

### Alternative A — Keep the crate; wire the reconciler to it (finish ADR-0024's intent)

Land the dependency edge ADR-0024 described so the reconciler calls
`overdrive_scheduler::schedule` instead of the inlined copy.

**Rejected — structurally impossible.** The reconciler lives in
`overdrive-core`; the crate depends on `overdrive-core`. The edge
`overdrive-core → overdrive-scheduler` is a cycle. ADR-0024's premise
that the consumer would live in `overdrive-control-plane` did not hold
(fact 1), so its dependency direction (its §4) cannot be realised. The
only way to "finish ADR-0024" would be to move the reconciler out of
`overdrive-core` into a crate that depends on the scheduler — a far
larger, unmotivated re-layout of the `core`-class reconciler that
ADR-0013/ADR-0021 deliberately placed in `overdrive-core`.

### Alternative B — Keep the crate; move the reconciler's inlined helpers into it and have the reconciler re-inline nothing

Same cycle problem as Alternative A. Rejected for the same reason.

### Alternative C — Keep both copies (crate for external consumers, inline for the reconciler)

Ship the crate as a "library for future external schedulers" and keep
the reconciler's private inline copy as the production path.

**Rejected.** Two implementations of one algorithm is the drift this
ADR is correcting (fact 4), not a shape to bless. There is no external
consumer today; "a future third-party scheduler crate" was ADR-0024's
speculative justification and is exactly the premature-extraction cost
this consolidation removes. When a genuine second consumer appears
(Phase 2+ multi-driver / right-sizing), the placement lives in a single
`core`-class module they can import directly — no orphan crate in the
interim.

### Alternative D — Delete the crate but leave `first_fit_place` as the SSOT (drop the richer diagnostics)

Delete `overdrive-scheduler` and keep the reconciler's bare
`Option<NodeId>` inline copy as the only placement code.

**Rejected as the default, but its *behaviour* is adopted.** The
crate's `schedule` is the *more complete* surface (typed
`PlacementError` with `needed` / `max_free`, the `NoHealthyNode`
empty-set guard, `#[must_use]`, the determinism-contract docstring, and
the acceptance-test suite that pins all of it). Deleting the richer
surface to keep the bare one throws away the tested code and keeps the
weaker. The decision moves the **richer** `schedule` into
`overdrive-core` and deletes the bare inline copy — while keeping the
reconciler's *production behaviour* unchanged (see Implementation
contract §3: behaviour-preserving adoption, no diagnostic upgrade to
the reconciler's action output in this ADR).

## Implementation contract

Pin these exactly. The crafter implements to this shape and does not
re-decide any of it.

### 1. Consolidation target — module path + public items

Move the pure placement surface into `overdrive-core` as a new module:

```
crates/overdrive-core/src/scheduler.rs        ← the module
```

Register it in `crates/overdrive-core/src/lib.rs` as `pub mod
scheduler;`. Rationale for `scheduler.rs` (over `placement.rs`): the
existing tree, docstrings, and the whitepaper all call this subsystem
"the scheduler"; keeping the name minimises churn in the doc-comment
updates listed in §5. The module is a sibling to `reconcilers/` and
`aggregate/` — a pure `core`-class helper consumed by a `core`-class
reconciler, matching the Anvil `reconcile_core` pure-helper pattern
ADR-0024 §Context already cited.

The three public items move verbatim from
`overdrive-scheduler/src/lib.rs` into `overdrive_core::scheduler`, with
the retyped signature from §2:

- `pub fn schedule(...) -> Result<NodeId, PlacementError>` — retyped per §2.
- `pub fn free_capacity(node, current_allocs, per_alloc) -> Resources`
  — unchanged signature; it already takes `&Resources` (`per_alloc`).
- `pub enum PlacementError { NoCapacity { needed, max_free }, NoHealthyNode }`
  — moved verbatim (fields `needed: Resources`, `max_free: Resources`).
- the private `const fn covers(available, needed) -> bool` helper — moved verbatim.

Public path after the move:
`overdrive_core::scheduler::{schedule, free_capacity, PlacementError}`.

### 2. Retyped `schedule` signature (kind-agnostic)

The moved `schedule` takes the **needed resource envelope**
(`&Resources`) in place of `&Job`. This is the exact signature to
implement:

```rust
use std::collections::BTreeMap;

use overdrive_core::id::NodeId;                       // in-crate: crate::id::NodeId
use overdrive_core::aggregate::Node;                  // in-crate: crate::aggregate::Node
use overdrive_core::traits::driver::Resources;        // in-crate: crate::traits::driver::Resources
use overdrive_core::traits::observation_store::AllocStatusRow;

#[must_use = "scheduler placement decisions must be acted on"]
pub fn schedule(
    nodes: &BTreeMap<NodeId, Node>,
    needed: &Resources,
    current_allocs: &[AllocStatusRow],
) -> Result<NodeId, PlacementError>;
```

Changes from the crate's current `schedule`:

- Parameter 2 changes from `workload: &Job` to `needed: &Resources`.
- Every in-body `workload.resources` read becomes `needed` (it is
  already `&Resources`).
- `PlacementError::NoCapacity { needed, .. }` is populated from
  `*needed` (was `workload.resources`).
- The determinism-contract docstring updates its "for any fixed
  `(nodes, workload, current_allocs)`" wording to
  `(nodes, needed, current_allocs)`.

Do **not** add any new parameter, variant, or overload. The
`free_capacity` and `PlacementError` shapes are unchanged.

### 3. Single SSOT — the reconciler adopts the moved `schedule`; behaviour-preserving

**Delete** the reconciler's private `first_fit_place`
(`workload_lifecycle.rs:908`) and `node_free_capacity`
(`workload_lifecycle.rs:927`) — they are not kept as thin wrappers. The
reconciler's placement call site (`workload_lifecycle.rs:806`) calls
`overdrive_core::scheduler::schedule` (in-crate: `crate::scheduler::schedule`)
directly.

**Behaviour-preserving adoption (no diagnostic upgrade in this ADR).**
The reconciler today, on a placement miss, emits **no action** and
leaves the Pending row in observation (`workload_lifecycle.rs:807-814`).
It must keep doing exactly that. The moved `schedule` returns
`Err(PlacementError::{NoCapacity, NoHealthyNode})` where the inline copy
returned `None`; the crafter maps **both** `Err` variants to today's
no-action-on-miss branch (`(Vec::new(), view.clone())`). The reconciler
does **not** begin surfacing `NoCapacity{needed, max_free}` in its
action output or Pending render as part of this ADR — that is a separate,
independently-motivated change and is out of scope here.

Rationale for behaviour-preserving over diagnostic-upgrade: the richer
`NoCapacity` diagnostics are now *available* at the single SSOT for any
future consumer that wants them, but wiring them into the reconciler's
Pending-render path touches the render/observation surface and its
acceptance tests — scope this ADR deliberately excludes. Consolidation
and retype land as a pure refactor with identical observable
reconciler behaviour; the diagnostic surfacing is a follow-up.

Two call-site mechanics the crafter must handle (not new API — call
adaptation):

- The `schedule` signature takes `current_allocs: &[AllocStatusRow]`
  (owned-slice-of-rows), whereas the inline `first_fit_place` took
  `&[&AllocStatusRow]` (slice-of-refs, built at
  `workload_lifecycle.rs:464` as `actual.allocations.values().collect()`).
  Adapt the call site to the moved function's existing slice shape — do
  **not** change `schedule`'s signature to accept the ref-slice. The
  crate's `schedule` already takes `&[AllocStatusRow]`; keep it.
- The call passes `&job.resources` (a `&Resources`) for the retyped
  `needed` parameter, where `job` is the `Some(job)` bind at
  `workload_lifecycle.rs:458`. The reconciler keeps projecting the
  kind-agnostic `Option<Job>` to its `.resources` envelope at the call
  site.

### 4. dst-lint discipline preserved — no separate crate required

ADR-0024's *sole* real technical rationale was mechanical enforcement
of the BTreeMap-only iteration discipline + banned-API contract
(no `Instant::now`, no `rand::*`, no `tokio::net::*`) on the placement
code, via `dst-lint`'s `crate_class == "core"` scan. **That property is
preserved for free**: `overdrive-core` is **already**
`crate_class = "core"` (ADR-0003) and already dst-lint-scanned. Moving
the placement module *into* `overdrive-core` keeps it inside the exact
same scan. A `HashMap` snuck into `crate::scheduler`, or a banned API in
its source, fails `cargo xtask dst-lint` at PR time — identical
enforcement to what the separate crate bought, with one fewer crate. A
dedicated `core`-class crate was never *required* for the enforcement;
it was one way to get a crate into the scan set, and `overdrive-core`
was already in it.

The xtask acceptance test `overdrive_scheduler_passes_dst_lint`
(`xtask/tests/acceptance/dst_lint_banned_apis.rs:138-179`) is **deleted
with the crate** — it asserts the *crate's* `crate_class = "core"`
declaration and scans `crates/overdrive-scheduler/src/`, both of which
cease to exist. The pre-existing clean-workspace dst-lint test
(`xtask/tests/acceptance/dst_lint_banned_apis.rs`, the assertion above
line 121) already covers `overdrive-core`'s `src/` — including the
moved `scheduler.rs` — so the enforcement signal is retained without a
scheduler-named test.

### 5. Deletions & migrations (the checklist)

**Delete the crate:**

- Delete `crates/overdrive-scheduler/` in full (all of:
  `Cargo.toml`, `src/lib.rs`, `tests/acceptance.rs`,
  `tests/acceptance/{capacity_accounting, common, determinism,
  empty_node_set, first_fit_happy_path,
  free_capacity_strict_inequality}.rs`, and
  `tests/acceptance/determinism.proptest-regressions`).

**Deregister from the workspace `Cargo.toml`:**

- Remove `"crates/overdrive-scheduler",` from `[workspace] members`
  (line 10).
- Remove `"crates/overdrive-scheduler",` from `default-members`
  (line 29). *(Note: ADR-0024 recorded only one entry; the live tree
  has two — `members` and `default-members`. Remove both.)*

**Delete the crate-specific xtask test:**

- Delete `overdrive_scheduler_passes_dst_lint`
  (`xtask/tests/acceptance/dst_lint_banned_apis.rs:123-179` — the
  US-01 §1.8 comment block through the closing brace of the test fn).
  Do not delete the clean-workspace test above it.

**Migrate the acceptance tests into `overdrive-core`, adapted to the
`&Resources` signature:**

- Move the crate's acceptance suite into `overdrive-core`'s test tree
  (`crates/overdrive-core/tests/acceptance/…`, wired through the
  existing `crates/overdrive-core/tests/acceptance.rs` entrypoint per
  ADR-0005's `tests/acceptance/<scenario>.rs` layout). The scenarios to
  migrate: `capacity_accounting`, `determinism` (+ its
  `.proptest-regressions`), `empty_node_set`, `first_fit_happy_path`,
  `free_capacity_strict_inequality`, and the shared `common.rs`
  fixtures/strategies.
- Adapt each migrated test to call
  `overdrive_core::scheduler::schedule(&nodes, &resources, &allocs)` —
  pass `&make_job(...).resources` (or the fixture's `Resources`
  directly) for the retyped `needed` parameter, replacing the prior
  `&make_job(...)` / `&arb_job()` `Job` argument. The `common.rs`
  `make_job` / `arb_job` helpers may be retained for building the
  `Resources` envelope, or the tests may build `Resources` directly —
  crafter's choice, but the `schedule` call must pass `&Resources`.
- **Deduplicate against the existing mirror.**
  `crates/overdrive-core/tests/acceptance/first_fit_place_branches.rs`
  already mirrors part of this coverage against the (now-deleted) inline
  `first_fit_place` / `node_free_capacity`. Re-point its assertions at
  `overdrive_core::scheduler::{schedule, free_capacity}` and fold the
  migrated crate tests into it where they overlap, rather than shipping
  two copies of the same boundary tests. The net test count should not
  grow by the full migrated suite where `first_fit_place_branches.rs`
  already covers a scenario.

**Update the stale doc comments that reference the deleted crate:**

- `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs:71` —
  the `WorkloadLifecycle` struct docstring says the reconciler "calls
  `overdrive_scheduler::schedule(...)`". Update to
  `crate::scheduler::schedule(...)`.
- `workload_lifecycle.rs:460-463` — the "inlined from
  overdrive-scheduler::schedule … overdrive-core cannot depend on
  overdrive-scheduler" comment is now false; the code calls
  `crate::scheduler::schedule` directly. Rewrite to describe the direct
  call (the cycle rationale is obsolete once the module is in-crate).
- `workload_lifecycle.rs:901-926` — the `first_fit_place` /
  `node_free_capacity` docstrings (which reference
  `overdrive_scheduler::schedule` / `overdrive_scheduler::free_capacity`
  and the "cannot depend on overdrive-scheduler" rationale) are deleted
  along with those functions (§3), so these comments vanish with them.
- `crates/overdrive-control-plane/src/lib.rs:2578` — the
  `workload_lifecycle()` factory docstring references
  `overdrive_scheduler::schedule`. Update to
  `overdrive_core::scheduler::schedule`.
- `crates/overdrive-core/tests/acceptance/first_fit_place_branches.rs:312-314`
  — the note pointing at `overdrive_scheduler::free_capacity` and
  `overdrive-scheduler/tests/acceptance/free_capacity_strict_inequality.rs`
  updates to point at `overdrive_core::scheduler::free_capacity` and the
  migrated in-crate test.

### 6. Single-cut greenfield

No shim, no deprecation window, no `overdrive-scheduler` re-export
crate, no `pub use` alias preserving the old
`overdrive_scheduler::schedule` path. Removed is removed, per
`CLAUDE.md` § single-cut greenfield migrations and
`feedback_single_cut_greenfield_migrations.md`. The crate deletion, the
module move + retype, the reconciler adoption, the test migration, and
the doc-comment fixes all land in the **same PR** — there is no
intermediate commit where both the crate and the in-core module exist,
and no commit where the reconciler calls a path that no longer exists.

## Consequences

### Positive

- **Single SSOT for placement.** One `schedule` function
  (`overdrive_core::scheduler::schedule`), one `free_capacity`, one
  `PlacementError`. The drift between the crate's rich diagnostics and
  the reconciler's bare `Option<NodeId>` (fact 4) is eliminated: the
  reconciler now calls the same function whose tests defend the
  determinism and capacity contracts.
- **Kind-agnostic at the type level.** The retype to `&Resources`
  removes the `Job` misnomer; the function's signature now truthfully
  says "given a resource envelope, place it," matching the reconciler's
  kind-agnostic `Option<Job>` projection through which every workload
  kind (`JobV1` / `ServiceV1` / `ScheduleV1`) already funnels.
- **Richer diagnostics available at the SSOT.** `NoCapacity{needed,
  max_free}` and `NoHealthyNode` are now the canonical placement-failure
  shape reachable from the reconciler's crate — a future Pending-render
  upgrade (out of scope here) has the data without re-deriving it.
- **One fewer crate.** The workspace drops from its current member set
  by one; one `Cargo.toml`, one `src/lib.rs`, one workspace-member
  registration (×2 lists), and one crate-specific xtask test go away.
  The premature-extraction cost ADR-0024's own *Negative* section
  flagged ("Phase 1 has exactly one consumer of the new crate") is
  paid down — the "one consumer" turned out to be *zero* real consumers
  plus a hand-inlined copy.
- **No isolated mechanism.** The placement code now lives on the real
  production path (the `WorkloadLifecycle` reconciler driven through
  `overdrive serve` + `overdrive deploy`), satisfying `CLAUDE.md`
  § "Build vertical slices through production entry points" — the exact
  rule ADR-0024's orphaned crate violated.

### Negative

- **`overdrive-core`'s surface grows by one module.** ADR-0024's
  Alternative B rejected putting the scheduler in `overdrive-core` on
  the grounds that "domain logic that consumes the seams belongs in
  adjacent crates." That principle is knowingly relaxed here: the
  scheduler is a pure helper consumed *only* by a `core`-class
  reconciler that already lives in `overdrive-core`, so co-locating it
  keeps consumer and helper in one crate with zero cross-crate edges,
  and the dst-lint enforcement (the only property that motivated
  separation) is identical. The cost is a slightly larger
  `overdrive-core` surface; the benefit is deleting an orphan and a
  single SSOT. The trade is judged worth it: ADR-0024's separation
  bought nothing that survived contact with the real reconciler
  placement (facts 1-3).
- **A future genuine second consumer must re-extract.** If Phase 2+
  grows a placement consumer *outside* `overdrive-core` (e.g. a
  control-plane-side right-sizing path), it imports
  `overdrive_core::scheduler::schedule` directly — no new crate needed,
  since `overdrive-core` is a universal dependency. Only if placement
  ever needs to be depended on *without* pulling `overdrive-core` (not
  foreseeable) would extraction recur; that is a real-consumer-driven
  decision, not a speculative one.

### dst-lint discipline preserved

Restated for the record (contract §4): the BTreeMap-only + banned-API
enforcement that ADR-0024 §5 secured via a separate `core`-class crate
is **preserved unchanged** because `overdrive-core` is already
`crate_class = "core"` and already in `dst-lint`'s scan set. Moving the
module in-crate keeps it scanned. No xtask code change is required
(the scan walks `overdrive-core/src/**` already). The scheduler-named
xtask test is deleted only because it asserted the *crate's* existence;
the clean-workspace dst-lint test retains the enforcement signal over
the moved module.

### Quality-attribute impact

- **Maintainability — modifiability**: positive. One placement
  function to change; no two-copy drift to keep in sync.
- **Maintainability — analyzability**: positive. The placement
  subsystem is one module next to its only consumer; no cross-crate
  hop to trace "where does the reconciler place."
- **Maintainability — testability**: neutral-to-positive. The
  determinism / capacity proptests move with the code and now defend
  the *production* path (the reconciler calls the tested function),
  not an orphan.
- **Reliability — fault tolerance**: neutral. Pure function; the only
  failure modes remain the typed `PlacementError` variants. Reconciler
  behaviour on a miss is unchanged (behaviour-preserving, §3).
- **Performance — time behaviour**: neutral. Identical algorithm; a
  direct in-crate call replaces a hand-inlined copy.

## Compliance

- **ADR-0003 (crate-class labelling)**: `overdrive-core` is already
  `crate_class = "core"`; the moved module inherits its dst-lint scan.
  The core-class set stays non-empty; deleting `overdrive-scheduler`
  removes the redundant second core-class member ADR-0024 added.
- **ADR-0005 (test distribution)**: migrated tests use the
  `tests/acceptance/<scenario>.rs` layout wired through the crate's
  `tests/acceptance.rs` entrypoint.
- **ADR-0006 (CI dst gates)**: `cargo xtask dst-lint` scans
  `overdrive-core/src/` including the moved module; no CI config change.
- **ADR-0013 / ADR-0021 (reconciler primitive & state shape)**: the
  `WorkloadLifecycle` reconciler stays in `overdrive-core` as those
  ADRs placed it; this ADR moves the pure placement helper *to* the
  reconciler rather than the reverse.
- **`development.md` § Ordered-collection choice**: the BTreeMap-only
  discipline on the placement code remains mechanically enforced by
  dst-lint (contract §4).
- **`testing.md` § Workspace convention**: deleting a member reduces the
  member set; the `every_workspace_member_declares_integration_tests_feature`
  xtask test still passes (every *remaining* member declares the
  feature; the deleted crate is simply no longer walked).
- **`CLAUDE.md` § "Build vertical slices …" / § single-cut greenfield
  migrations / § "Implement to the design — never invent API surface"**:
  the consolidation puts placement on the production path, lands
  single-cut with no shim, and pins the exact `schedule` signature so
  the crafter builds only the named surface.

## References

- **ADR-0024** — the superseded decision (dedicated `overdrive-scheduler`
  crate). This ADR overturns its Decisions §1-§5 and preserves only the
  dst-lint enforcement property, via a different mechanism.
- ADR-0003 — core-crate labelling via
  `package.metadata.overdrive.crate_class` (the scan-set membership rule
  that makes contract §4 work).
- ADR-0005 — test distribution / `tests/acceptance/<scenario>.rs` layout
  (test-migration target shape).
- ADR-0006 — `cargo xtask dst-lint` is the required CI check.
- ADR-0013 — reconciler primitive; the `core`-class reconciler home.
- ADR-0021 — `AnyState` / reconciler state shape; the `WorkloadLifecycle`
  consumer.
- ADR-0073 — backend-instance-replacement; current `WorkloadLifecycle`
  generation logic surrounding the placement call site.
- `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs` — the
  reconciler holding the inlined placement being replaced
  (`first_fit_place` @908, `node_free_capacity` @927, call site @806).
- `crates/overdrive-scheduler/` — the crate being deleted.
- `crates/overdrive-core/src/traits/driver.rs:126` — the `Resources`
  envelope the retyped `schedule` takes.
- `.claude/rules/development.md` § Ordered-collection choice — the
  discipline dst-lint mechanically enforces on the moved module.
- `CLAUDE.md` § "Build vertical slices through production entry points",
  § single-cut greenfield migrations, § "Implement to the design —
  never invent API surface" — the three rules this consolidation and
  its pinned contract satisfy.

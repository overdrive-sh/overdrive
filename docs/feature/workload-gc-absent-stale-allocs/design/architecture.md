# Architecture — Workload GC for Absent-Intent Stale Allocations

**Feature**: `workload-gc-absent-stale-allocs`
**Wave**: DESIGN
**Author**: Morgan (Solution Architect)
**Date**: 2026-05-14
**Status**: Proposed; awaiting peer review and user ratification of ADR-0037 amendment scope.

---

## 1. Requirements anchor

**Single SSOT: GitHub issue #148** — *"WorkloadLifecycle reconciler: cleanup stale allocations when desired Job is absent"*. There is no DISCUSS wave for this feature; the issue body is the requirements record. All sections below cite #148 as the binding statement of intent.

The issue identifies a gap at `crates/overdrive-core/src/reconciler.rs:1206-1211` (today's line numbers), in the `match desired.job.as_ref() { None => (Vec::new(), view.clone()), ... }` arm. The arm is annotated:

> `// Absent: no desired job. The Stop branch above handles explicit`
> `// stops; an absent job with stale Running allocs is TODO(#148)`
> `// (cleanup reconciler) — for now we emit nothing and pass the view`
> `// through unchanged.`

### 1.1 Triggering scenarios (issue #148 § "Why it matters")

Three concrete scenarios produce the absent-desired-with-stale-actual shape:

1. **Hard-delete** — operator deletes the `Job` intent (e.g. via a future `DELETE /v1/workloads/{id}` whose semantics include intent removal, not just a stop intent). Whether hard-delete is supported is explicitly **out of scope per #148**; what matters here is that *if* the intent row vanishes, the reconciler must converge.
2. **Multi-node drain** — under Phase 2+ where allocations of a workload run on multiple nodes, draining node A removes its local `Job` projection while node B's allocations remain Running until the GC arm fires per-node.
3. **Crash recovery** — a node restarts with a redb file that has lost the intent row (corruption, manual operator surgery, or a future supersession path) but retained observation-store rows projecting Running allocations.

In every scenario the steady-state-correct outcome is the same: every non-terminal `AllocStatusRow` for the orphaned `workload_id` must converge to a terminal state with a *distinct* `TerminalCondition` that audit consumers can tell apart from operator-initiated stops.

### 1.2 Out of scope per #148

- **ObservationStore row GC mechanism**. How (and when) terminal `AllocStatusRow`s themselves are evicted from `alloc_status` is a separate concern, deferred to Phase 2+. This design converges *state*, not *storage occupancy*.
- **Whether hard-delete intent is a supported operator action**. The three triggering scenarios above include hard-delete as a hypothetical, but the design does not require it to be wired. The GC arm must work whenever the absent-desired shape occurs, regardless of how it arose.

### 1.3 Acceptance criteria (verbatim from #148)

- Convergence: every Running alloc for an absent-`Job` workload reaches a terminal `AllocState`.
- Distinct `TerminalCondition`: the terminal row carries a variant operators can match against to distinguish "stopped by the system because no intent remained" from "stopped by the operator" or "stopped by the process".
- DST scenario: a property/invariant test under `cargo dst` that drives the absent-intent shape and asserts both convergence and the typed-terminal claim.

### 1.4 Terminology note (per observation 40899)

Issue #148 references `JobLifecycle` (the pre-ADR-0047 name). The reconciler was renamed to `WorkloadLifecycle` in ADR-0047 (workload-kind discriminator); the state type is `WorkloadLifecycleState`. This document uses the current names. Issue body wording is otherwise binding.

---

## 2. Constraints inherited from prior ADRs

These ADRs constrain the design space. Any option that violates them is rejected before the trade-off comparison.

| ADR | Constraint | Implication for this design |
|---|---|---|
| **ADR-0021** | `AnyState::WorkloadLifecycle(WorkloadLifecycleState)`; `desired` and `actual` share one struct shape; `job: Option<Job>` makes the absent case representable. | The mechanism IS already in place. `desired.job.is_none()` is exactly the discriminator the GC logic branches on. No new state shape required for Option A. |
| **ADR-0035 + ADR-0036** | `reconcile` is pure sync over `(desired, actual, view, tick) → (Vec<Action>, View)`. No `.await`, no I/O, no DB handle. Hydration owned by runtime. | The GC arm must be expressible without I/O. The runtime's existing `hydrate_actual` already filters `alloc_status_rows()` to rows matching the target's `workload_id` (`reconciler_runtime.rs:1078`); when `desired.job` is `None`, `actual.allocations` correctly carries only the orphan rows for that target. **No new hydration logic is needed.** |
| **ADR-0037** | `Action::StopAllocation` carries `terminal: Option<TerminalCondition>`. The Stop branch stamps `Stopped { by: StoppedBy::Operator }`. `StoppedBy` is `{ Operator, Reconciler, Process }`. | A new `StoppedBy::SystemGc` variant is required to satisfy AC §1.3's "distinct TerminalCondition". This is the SemVer-additive variant addition ADR-0037 §5 describes; it falls inside the existing `TerminalCondition::Stopped { by }` shape (no new top-level variant). |
| **ADR-0047** | Reconciler is `WorkloadLifecycle`; `WorkloadKind ∈ { Service, Job, Schedule }`. Job-kind has its own natural-exit terminal handling (`Completed`/`Failed`); Service-kind uses restart-budget terminals. | The GC arm must be kind-agnostic. An orphan-row scenario can occur for any workload kind. The emitted terminal claim is `Stopped { by: SystemGc }` regardless of kind — semantically the system is *withdrawing* the workload, not concluding its natural lifecycle. |
| **`development.md` § "Reconciler I/O"** | `reconcile` returns Actions; the runtime owns persistence. View carries inputs, not derived state. | If the GC arm needs any per-target memory (e.g. "first-observed-as-orphan timestamp" for a soft-stop grace window), the View carries the input timestamp, not a derived deadline. **This design does NOT introduce such memory** — the simplest correct behaviour fires `StopAllocation` immediately on the orphan-observed tick, no grace window. See § 6 Open question 1 for why. |
| **`development.md` § "Persist inputs, not derived state"** | Persisted fields must be inputs to the live policy, not cached outputs. | If a future grace-window policy lands, the View input is `first_observed_orphan_at: UnixInstant`; the deadline is recomputed every tick against the live policy. Not required for the current cut. |

---

## 3. Reuse Analysis (MANDATORY — F-1)

Default = **EXTEND** existing surface. CREATE NEW must be justified with evidence.

| Existing surface | Overlap with this feature | Decision | Evidence |
|---|---|---|---|
| `WorkloadLifecycle` reconciler (`overdrive-core/src/reconciler.rs`) | Reconcile loop; runtime registration; broker enqueue path; hydration of `WorkloadLifecycleState`; action emission to `Action::StopAllocation`; View persistence via runtime ViewStore. | **EXTEND.** Replace the `None => (Vec::new(), view.clone())` arm with logic that emits `StopAllocation` per non-terminal row. Every downstream surface (action shim, observation row write, lifecycle event broadcast) is already wired for `Action::StopAllocation`. | The arm IS the documented gap (TODO(#148) in source). Extending it costs one branch body; creating a sibling reconciler costs a new `AnyReconciler` / `AnyState` / `AnyReconcilerView` variant, a new hydrator arm, and a new target-resource registration story — none of which a Phase 1 single-node feature needs. |
| `WorkloadLifecycleState.allocations: BTreeMap<AllocationId, AllocStatusRow>` | Already populated by `hydrate_actual` filtered to the target's `workload_id`. | **REUSE AS-IS.** When `desired.job` is `None`, this map already contains exactly the orphan rows we need to stop. No hydrator change. | `reconciler_runtime.rs:1078` — `for row in rows.into_iter().filter(|r| r.workload_id == workload_id)`. |
| `Action::StopAllocation { alloc_id, terminal }` | Action variant that already writes the row's terminal field and broadcasts the lifecycle event with the same value (ADR-0037 §4). | **REUSE AS-IS.** No new action variant. The action shim's write path is unchanged. | `reconciler.rs:519-525` — the variant docstring explicitly says the reconciler stamps `terminal` and the action shim writes both surfaces. |
| `TerminalCondition::Stopped { by: StoppedBy }` | Closest variant for "this allocation reached Stopped because of withdrawn intent". | **EXTEND `StoppedBy` enum.** Add a new `SystemGc` variant. No new `TerminalCondition` top-level variant. ADR-0037 §5 covers this exact shape ("New variants are additive minor"). | The Stop-by-operator branch is the structural precedent. The mechanism is identical; only the *by-source* differs. |
| `ObservationStore::alloc_status_rows()` | Source of orphan-row evidence. | **REUSE AS-IS.** | Already returns every row; hydrator already filters per target. |
| Stop-branch convergence pattern (`reconciler.rs:1180-1205`) | The "iterate Running rows, emit StopAllocation per row, clear `last_failure_seen_at` when complete" shape. | **REUSE PATTERN.** The GC arm's body is structurally identical — different `terminal` value, same convergence shape. | Same idempotency story (next tick: rows are Terminated, filter is empty, no more Actions). Same view-cleanup story. |

**Outcome: zero new types beyond one `StoppedBy::SystemGc` variant. Every other surface extends in-place.** This is the strongest possible signal that Option A (extend the `None` arm) is the right shape.

---

## 4. Options

### Option A — Extend `WorkloadLifecycle::reconcile`'s `None` arm (RECOMMENDED)

Replace the `None => (Vec::new(), view.clone())` arm with:

```text
None => {
    // GC branch: desired Job is absent (hard-delete, multi-node drain,
    // crash-recovery surgery). Withdraw any non-terminal allocations
    // by stamping a system-GC terminal claim.
    let gc_terminal = Some(TerminalCondition::Stopped {
        by: StoppedBy::SystemGc,
    });
    let stop_actions: Vec<Action> = actual
        .allocations
        .values()
        .filter(|r| r.state == AllocState::Running)
        .map(|r| Action::StopAllocation {
            alloc_id: r.alloc_id.clone(),
            terminal: gc_terminal.clone(),
        })
        .collect();
    let mut next_view = view.clone();
    if stop_actions.is_empty() {
        // No work left — clear backoff inputs so the broker stops
        // re-enqueueing this target. Mirrors the Stop branch shape.
        next_view.last_failure_seen_at.clear();
    }
    (stop_actions, next_view)
}
```

**Hydration cost.** Zero new I/O. Runtime's `hydrate_desired` reads `Job` from intent (returns `None` for the absent case — already wired). `hydrate_actual` reads alloc rows for the workload id — already wired. No table scan, no orphan-detection observation.

**Concurrency safety vs racing Submit.** ADR-0037 §1 establishes LWW on intent. If a `Submit { id: X }` lands intent immediately after a GC tick stops X's row, the next tick of the same target sees `desired.job = Some(...)` again and follows the Run branch — placing a fresh allocation. The stopped row is durably terminal-stamped with `SystemGc`. This is correct: the operator's resubmit creates a *new* allocation, and the audit log shows the prior allocation was withdrawn by GC. No state corruption; level-triggered convergence resolves the race the way operators expect.

**Termination model.** The `actual.allocations` map already filters to non-terminal-by-default semantics (terminal rows remain in `alloc_status` until ObservationStore GC, which is out of scope). After all rows reach Terminated, the `filter(state == Running)` collects zero Actions; the View's backoff input is cleared on that tick; the broker stops re-enqueueing per the existing `view_has_backoff_pending` predicate. The arm becomes a no-op once steady state is reached.

**DST scenario shape.** A turmoil scenario:

1. Submit `Job(id=X)` → intent written, observation eventually shows Running alloc.
2. Operator hard-deletes intent (test harness removes the `jobs/X` key from `IntentStore`).
3. Tick the WorkloadLifecycle reconciler for target X.
4. **Invariant** (`assert_eventually!`): every `AllocStatusRow.workload_id == X` reaches a terminal `AllocState` AND carries `terminal == Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc })`.
5. **Invariant** (`assert_always!`): no Action other than `StopAllocation` is emitted while `desired.job` is `None`.
6. **Invariant** (`assert_always!`): no fresh allocation is placed for X while the intent remains absent.

**New `StoppedBy::SystemGc` variant.** Single-line addition to the `StoppedBy` enum at discriminant index `3` (after `Process=2`). ADR-0037 amendment formalises the SemVer claim (additive minor).

> **NOTE — rkyv discriminant stability AND comment update.** The existing `StoppedBy` enum carries an explicit comment at `crates/overdrive-core/src/transition_reason.rs:212-213`: *"MUST remain the last variant to preserve rkyv discriminant compatibility: `Operator=0`, `Reconciler=1`, `Process=2`."* The wording "last variant" is the **mechanism** claim; the **invariant** is discriminant stability for pre-existing variants. Appending `SystemGc` at index `3` preserves the invariant (Operator/Reconciler/Process keep their discriminants) but invalidates the mechanism wording. **The variant addition MUST land alongside a comment update** that switches the wording to the same shape used elsewhere in this file — see `TerminalCondition::Completed` at lines ~395-401 for the canonical phrasing: *"Appended after `<prior_last>` to keep the pre-existing rkyv discriminants stable. This variant takes discriminant `3`. Existing archived rows decode unchanged."* The implementing crafter MUST land both (variant + comment fix) in step 01-01's single commit; landing the variant without the comment fix leaves a stale invariant claim in the SSOT.
>
> **Forward roundtrip coverage (not a new fixture).** Per `.claude/rules/development.md` § "rkyv schema evolution" → "Version-bump procedure", existing `FIXTURE_V1` (already pinned at `crates/overdrive-core/tests/schema_evolution/alloc_status_row.rs:59`) is **NEVER touched**. Adding a `StoppedBy` variant does NOT bump the envelope to V2 — the layout is unchanged; only an enum gained an additive variant. Step 01-01 therefore: (a) verifies existing `FIXTURE_V1` continues to hex-decode + project correctly through the new enum (the existing test already does this — no edit needed to assert it); (b) adds a NEW unit-level roundtrip test (not a fixture constant) constructing a fresh `AllocStatusRow` with `terminal = Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc })`, archiving via the current envelope, deserialising, and asserting `Eq`. This is forward coverage, not historical pinning. Minting a `FIXTURE_V2` is wrong shape.

**Within-shim ordering (action shim → observation write).** The design assumes the action shim writes `AllocStatusRow.terminal` durably before broadcasting the lifecycle event (i.e., write-through-then-broadcast). This is a property of the existing action shim (per ADR-0023 / ADR-0037 §4) — not introduced by this feature. **Crafter validation gate at step 01-03**: confirm `crates/overdrive-control-plane/src/action_shim.rs` (or wherever the shim lives) sequences `AllocStatusRow.terminal` durability before the lifecycle event broadcast for `Action::StopAllocation`. If the shim broadcasts before durability, a fresh `Submit` between emission and durability could place a fresh alloc before the original's `SystemGc` terminal stamp is observable — a within-shim race outside this design's surface. Surface as a blocker to the user if the shim does not sequence correctly; do NOT silently absorb the race.

### Option B — Sibling `WorkloadGC` reconciler

Create a new reconciler kind: `WorkloadGC` with its own `AnyReconciler` / `AnyState` / `AnyReconcilerView` variants. Hydrator performs a full-table scan of `alloc_status_rows()` to find every distinct `workload_id` that has no `Job` entry in intent; emits `StopAllocation` per orphan row.

**Hydration cost.** **Higher.** Requires a full-table scan of `alloc_status` to enumerate orphan workload ids, *or* a new "orphan-detection" observation row that some other component maintains. Either way, the runtime grows a new code path that reads cross-target state on every tick. The existing per-target hydration pattern (one `WorkloadLifecycle` evaluation = one workload's worth of I/O) is broken.

**Concurrency safety.** A racing Submit during a GC reconciler tick is more awkward: the GC reconciler reads intent-absent at time T₀, decides X is orphan, emits Stop at T₁; meanwhile a Submit at T₀.₅ races with the GC's own action dispatch. With Option A the WorkloadLifecycle reconciler always reads target X's intent in the same hydration as the actual rows, so the LWW resolution is local to one evaluation.

**Termination model.** Same idempotency story (re-tick after all rows Terminated produces no Actions), but the per-target → per-orphan-set scoping difference means the GC reconciler's "is there work" predicate has to walk the whole alloc table each tick, vs. WorkloadLifecycle's existing per-target broker enqueue.

**DST scenario shape.** Same invariants as Option A; additionally must check that the GC reconciler does NOT race the WorkloadLifecycle reconciler (e.g. by both trying to stop the same alloc when intent is absent transiently).

**Reuse Analysis verdict.** CREATE NEW would be required for `WorkloadGC` reconciler, hydrator, target-resource registration story, and a new dispatch arm in `AnyReconciler::reconcile`. **No evidence Option A is impossible.** Per F-1, CREATE NEW is rejected.

### Option C — Hybrid (rejected: collapses to A)

A hybrid would split the responsibility: WorkloadLifecycle's `None` arm emits the GC stops; a separate sweeper reconciler handles ObservationStore tombstone GC. **But the latter is explicitly out of scope per #148.** With observation-row eviction excluded, Option C is structurally identical to Option A. Listed for completeness; not a real third option.

---

## 5. Recommendation

**Option A.** Three load-bearing reasons:

1. **Strongest reuse signal.** Every relevant surface already exists. The only new code is the GC arm body and a `StoppedBy::SystemGc` variant. F-1 default ("EXTEND over CREATE NEW") is honoured by construction.
2. **Cheapest hydration.** Per-target hydration is already in place; the GC arm fires for free as part of the existing tick. No new cross-target observation row, no full-table scan.
3. **Best concurrency model.** Local LWW resolution per evaluation: each tick of target X sees one consistent snapshot of `(intent[X], obs[X])`. A race between hard-delete and resubmit always resolves to "the most recent intent wins" with no inter-reconciler coordination required.

**Quality-attribute ranking (per #148 implicit priorities):**

- **Correctness/convergence** — both options converge. Option A converges per target every tick; Option B converges per orphan-set every tick. Equal under steady state, simpler under races (A wins on simplicity).
- **Auditability** — both options emit `TerminalCondition::Stopped { by: StoppedBy::SystemGc }`. Equal.
- **Simplicity/reviewability** — Option A is ~20 LOC; Option B is a new reconciler + new dispatch + new hydrator (~200 LOC). **A wins decisively.**
- **Performance** — cold path; not a perf concern. Effectively equal.

**ADR action.** **Amend ADR-0037** to add `StoppedBy::SystemGc`. No new ADR is required. The reconciler-side mechanism is a body change inside a documented branch (TODO(#148)); the only architecturally-significant decision is the new terminal-condition vocabulary item, which is the ADR-0037 SemVer-additive scope.

---

## 6. Component boundaries

This change is intra-reconciler. The component diagram (`./c4-component.md`) shows the affected surfaces.

**Touched surfaces:**

- `overdrive-core::transition_reason::StoppedBy` — add `SystemGc` variant (last position to preserve rkyv discriminants).
- `overdrive-core::reconciler::WorkloadLifecycle::reconcile` — replace `None` arm body.
- `overdrive-core/tests/schema_evolution/alloc_status_row.rs` — verify golden fixtures still decode (new variant is forward-compatible; old archives carrying `Stopped { by: ∈ {Operator, Reconciler, Process} }` continue to decode unchanged).
- `overdrive-sim/tests/dst/` — new DST scenario for the absent-intent shape.

**Untouched surfaces:**

- The runtime (`reconciler_runtime.rs`) — `hydrate_desired` and `hydrate_actual` already return the correct shapes; no new I/O.
- The action shim — `Action::StopAllocation` carries `terminal` already; the shim writes both row and event.
- `LifecycleEvent` — gains `Stopped { by: SystemGc }` events automatically via the same shim path that already echoes `terminal`.
- `Job` intent persistence path — read-only consumer.
- The evaluation broker — re-enqueues per target on observation deltas and on submit; the absent-intent + non-terminal-rows shape already produces a tick (because the rows themselves are observation events) and the GC arm's view-cleanup quiesces the predicate when work is done.

---

## 7. DST invariant shape

Three invariants (Tier 1, turmoil). **Every `assert_eventually!` carries an explicit tick budget** — an unbounded "eventually" is satisfied by a buggy implementation that converges through some unrelated path (a tick storm, a panic-reset, an unrelated GC). The budget is the falsifiability gate.

```text
SCENARIO orphan_workload_converges_to_terminal_gc
    seed: deterministic
    setup:
        Submit Job(id=X)
        wait_until: actual.allocations[X].any_state == Running
    fault_inject:
        IntentStore::delete("jobs/X")   // simulate hard-delete; method name per
                                        // crates/overdrive-core/src/traits/intent_store.rs:193
    tick:
        run WorkloadLifecycle reconciler for target X
    assert_eventually!("gc.converges", max_ticks = 3, |obs|
        // Bound: at one tick the GC arm emits StopAllocation per Running row;
        // the action shim writes terminal + state on the next tick boundary;
        // by the third tick, all rows for X are terminal. Anything slower is a bug.
        obs.alloc_status_rows()
           .filter(|r| r.workload_id == X)
           .all(|r| r.state.is_terminal())
    )
    assert_eventually!("gc.terminal_claim", max_ticks = 3, |obs|
        obs.alloc_status_rows()
           .filter(|r| r.workload_id == X)
           .all(|r| matches!(r.terminal,
               Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc })))
    )
    assert_always!("gc.no_fresh_alloc", |obs|
        // No new alloc placed for X while intent absent. `assert_always!` is
        // checked every tick — no budget needed; the invariant must hold at
        // every observation point post-fault.
        obs.alloc_status_rows()
           .filter(|r| r.workload_id == X && r.created_after(fault_inject_time))
           .count() == 0
    )
```

Plus a complementary invariant for the race shape:

```text
SCENARIO resubmit_after_gc_creates_fresh_alloc
    setup: same as above through fault_inject
    after gc has stopped the original alloc:
        Submit Job(id=X)   // operator changes mind
    assert_eventually!("resubmit.places_fresh", max_ticks = 5, |obs|
        // Bound: resubmit writes intent; one tick to hydrate, one to schedule,
        // one for the driver to mark Running. Five ticks is a generous ceiling.
        obs.alloc_status_rows()
           .filter(|r| r.workload_id == X && r.state == Running)
           .count() >= 1
    )
    assert_always!("resubmit.preserves_prior_gc_terminal", |obs|
        // The original alloc's terminal claim is durable across resubmit.
        obs.alloc_status_rows()
           .filter(|r| r.workload_id == X && r.alloc_id == original_alloc_id)
           .all(|r| matches!(r.terminal,
               Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc })))
    )
```

The DST suite already drives WorkloadLifecycle convergence; these scenarios extend the existing harness with one fault primitive (`IntentStore::delete` on the harness's intent handle — already present in the trait at `crates/overdrive-core/src/traits/intent_store.rs:193`; no new primitive to wire) and the invariants above. If the harness's `assert_eventually!` macro does not yet support a `max_ticks` parameter, the crafter wires the budget at the harness level (existing scenarios that rely on a default budget continue working; this feature's scenarios opt in to an explicit one).

---

## 8. Open questions for the crafter

These are implementation-detail questions that the architecture does not bind. The crafter (or DISTILL/DEVOPS waves if any) resolves them at TDD time:

1. **Grace window before GC fires?** The current cut fires `StopAllocation` immediately on the first tick where `desired.job.is_none()` AND non-terminal rows exist. A future operator-tunable grace window (e.g. "wait 30 s before withdrawing in case the intent comes back") is *not* in scope per the simplest-correct-cut principle. If a grace window proves necessary, it lands as a follow-up via a tracked GitHub issue with explicit user approval; the View input shape would be `first_observed_orphan_at: UnixInstant` per `development.md` § "Persist inputs, not derived state". **Not deferred; not promised.**
2. **Workload-kind branching inside the GC arm?** The current cut treats all kinds identically (system-GC withdrawal is kind-agnostic). If a future kind needs different GC semantics (e.g. `WorkloadKind::Job` might want `Failed { exit_code: SIGTERM_synthesized }` for a withdrawn run-to-completion workload), the arm gains a `match desired.workload_kind { ... }` body. **Not in scope today.**
3. **`AllocState::Pending` and `Draining` rows.** The current cut filters `state == Running` (matching the Stop branch). Pending and Draining rows reach terminal via their own existing observers (exec driver exit observer, drain observer). The crafter should verify whether the absent-intent shape leaves any Pending/Draining row hanging without an observer to advance it — if so, the filter widens to `!state.is_terminal()`. **The architecture does not bind the filter shape; the crafter validates against the four state transitions.**

---

## 9. Deferrals requiring user approval BEFORE issue creation

Per CLAUDE.md "Deferrals require GitHub issues — AND user approval BEFORE creation", any forward pointer in this design that names a future work item must surface to the user first. **This design contains zero such forward pointers**: the three open questions above are scoped as "not in scope today, no promise of a future ticket" — they are explicitly *not* deferrals. If during DISTILL or DELIVER any of them resurfaces as needing a tracking issue, the agent of that wave surfaces it to the user at that moment.

---

## 10. References

- **GitHub issue #148** — requirements SSOT.
- **ADR-0021** — `AnyState::WorkloadLifecycle(WorkloadLifecycleState)` shape; `desired.job: Option<Job>` is the discriminator.
- **ADR-0035** — collapsed reconciler trait; pure sync `reconcile`.
- **ADR-0036** — amendment removing per-reconciler `hydrate`; runtime owns I/O.
- **ADR-0037** — `TerminalCondition` + `StoppedBy`; this design proposes the additive `StoppedBy::SystemGc` amendment.
- **ADR-0047** — `WorkloadKind` taxonomy + reconciler rename to `WorkloadLifecycle`.
- **`.claude/rules/development.md` § "Reconciler I/O"** — `reconcile` purity contract.
- **`.claude/rules/development.md` § "Persist inputs, not derived state"** — View-shape rule (not exercised by this cut).
- **`.claude/rules/development.md` § "rkyv schema evolution"** — discriminant-stability rule that pins the new variant to the *end* of `StoppedBy`.
- **`.claude/rules/testing.md` § "Tier 1 DST"** — invariant catalogue model the DST scenarios above follow.
- **`crates/overdrive-core/src/reconciler.rs:1206-1211`** — the TODO(#148) marker this design closes.
- **`crates/overdrive-core/src/transition_reason.rs:203-215`** — `StoppedBy` enum (insertion site for the new variant).

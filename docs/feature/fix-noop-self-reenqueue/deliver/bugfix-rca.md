# Bugfix RCA — `Action::Noop` self-re-enqueue

**Feature ID**: `fix-noop-self-reenqueue`
**Surfaced via**: code review on `crates/overdrive-control-plane/src/reconciler_runtime.rs:250-275`
**RCA validated by**: @nw-troubleshooter (5 Whys, evidence-cited)
**User approval**: 2026-04-29 — Candidate A (filter `Action::Noop` in `has_work`) approved over Candidate C (change `NoopHeartbeat` to `vec![]`).

---

## Defect

`run_convergence_tick` evaluates `has_work = !actions.is_empty()` (line 256) before dispatch. `NoopHeartbeat::reconcile` unconditionally returns `vec![Action::Noop]` (`crates/overdrive-core/src/reconciler.rs:773-775`), so `has_work = true` for that reconciler on every tick. The §18 self-re-enqueue gate then re-submits `(noop-heartbeat, target)` perpetually.

Amplifier: the convergence loop iterates **every** registered reconciler against **every** drained target (`reconciler_runtime.rs:217-218`), so a single `(job-lifecycle, job/<id>)` submit causes `noop-heartbeat` to fire against that target and re-enqueue itself permanently.

`Action::Noop` is documented as "nothing to do this tick" (`core/reconciler.rs:447`) and `action_shim::dispatch` already treats it as a no-op (`action_shim.rs:108`). The runtime's `has_work` predicate fails to honor that documented semantic.

## Observable consequence

`broker.counters().dispatched` grows by ≥1 per tick for the lifetime of any active job, degrading the storm-proofing signal that operators read off the `cluster_status` counter.

## Root cause (one line)

`NoopHeartbeat`'s contract ("nothing to do this tick") is encoded as `vec![Action::Noop]` rather than `vec![]`, and the §18 level-triggered re-enqueue gate `has_work = !actions.is_empty()` operates on syntactic emptiness rather than the documented semantic.

## Fix surface — Candidate A (approved)

**File**: `crates/overdrive-control-plane/src/reconciler_runtime.rs`

**Edit 1** (imports, line 36-39): add `Action` to the `overdrive_core::reconciler::*` import.

**Edit 2** (line 256):

```rust
// Before
let has_work = !actions.is_empty();

// After — `Action::Noop` is the documented "nothing to do this tick"
// sentinel and `action_shim::dispatch` treats it as a no-op. The §18
// level-triggered re-enqueue gate must honor that.
let has_work = actions.iter().any(|a| !matches!(a, Action::Noop));
```

A short comment block above the predicate explaining the rationale (one paragraph, why-not-what, per `CLAUDE.md` comment discipline).

## Why Candidate C was rejected

Changing `NoopHeartbeat::reconcile` to return `vec![]` instead of `vec![Action::Noop]` would:

- **Contradict ADR-0013 §9 "proof-of-life" intent.** The reconciler exists specifically to demonstrate the runtime IS ticking via observable broker activity. Removing the only observable signal it produces silently defeats that purpose.
- **Break the doctest at `core/reconciler.rs:138`** which uses `vec![Action::Noop]` as an example.
- **Require updates to multiple acceptance tests** that pin the `vec![Action::Noop]` contract (`runtime_registers_noop_heartbeat.rs:179-180`, `reconciler_trait_surface.rs:486,511`, `any_reconciler_dispatch.rs:48,58`).
- **Make `Action::Noop` vestigial** — the variant would have no production emitter, leaving its declared semantics unenforced.

Candidate A is single-site, preserves the documented `Action::Noop` semantics, and keeps the proof-of-life intent intact.

## Risk — low

Confirmed by direct read of `JobLifecycle::reconcile` (`crates/overdrive-core/src/reconciler.rs:1089-1203`):

- Every converged branch returns `(Vec::new(), view.clone())` — identical behavior under the fix.
- Every active branch returns `vec![concrete_action]` — `any` correctly trips on the concrete action.
- `JobLifecycle` never emits `Action::Noop`, so the fix cannot mask any real "desired ≠ actual" signal for the only real reconciler in the registry.
- Mixed-action vecs (`vec![Action::Noop, Action::StartAllocation]`) still trip `any` correctly — only all-Noop vecs are suppressed.
- `HarnessNoopHeartbeat` (canary-bug feature) failure mode is the `ReconcilerIsPure` twin-invocation check inside the invariant evaluator, not broker activity — no impact on canary semantics.
- `eval_broker.rs` storm-proofing semantics (cancelable-eval-set, LWW collapse) are unaffected — the fix narrows the upstream submit rate; storm-proofing is downstream and remains correct.

## Regression test

**Test name**: `noop_heartbeat_against_converged_target_does_not_re_enqueue`

**Location**: `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs` (new file under the existing acceptance tests dir; gated by the workspace `integration-tests` feature per `.claude/rules/testing.md` § "Integration vs unit gating").

**Shape**:

1. Build a converged-state `AppState`: sim observation/intent/driver, IntentStore preloaded with one job whose `desired.replicas == actual.allocations.running.count`.
2. Submit ONE `Evaluation { reconciler: "job-lifecycle", target: "job/<id>" }` to the broker.
3. Drive the convergence loop manually for 10 ticks: drain pending → call `run_convergence_tick` per drained eval.
4. Assert `state.runtime.broker().counters().dispatched == 1` (pre-fix this grows to ≥10).
5. Assert `state.runtime.broker().counters().queued == 0` (no pending evals after convergence).

The test must FAIL against current code and PASS after the fix lands. RED → GREEN ordering enforced via `nw-deliver`.

## Files affected (production code)

- `crates/overdrive-control-plane/src/reconciler_runtime.rs` — imports + `has_work` predicate.

## Files affected (test code)

- `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs` — new file (regression test).
- `crates/overdrive-control-plane/tests/acceptance.rs` — declare the new submodule.

## Out of scope (separate work item)

Add a DST invariant in `crates/overdrive-sim/src/invariants/evaluators.rs` asserting "after K ticks against a converged cluster, `broker.dispatched` is bounded by the number of distinct edge-triggered submits." The reviewer's hint identified this gap; would have caught the bug pre-merge under DST replay. Tracking this separately rather than bundling.

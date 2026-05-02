# Bugfix RCA — `run_convergence_tick` discards `eval.reconciler`

**Feature ID**: `fix-eval-reconciler-discarded`
**Surfaced via**: code review on `crates/overdrive-control-plane/src/reconciler_runtime.rs:207-287` and call site `crates/overdrive-control-plane/src/lib.rs:465-481`.
**RCA validated by**: @nw-troubleshooter (5 Whys, evidence-cited).
**Related**: `docs/feature/fix-noop-self-reenqueue/deliver/bugfix-rca.md` (commit `7a60743`) — that fix narrowed the *symptom* (Noop-driven self-re-enqueue loop). This bug is the *upstream* dispatch-routing defect that made the symptom reachable in the first place. After the noop fix, Phase 1 became harmless; the dispatch-routing defect remains.

---

## Defect (one line)

The convergence-loop drainer at `lib.rs:470-481` consumes `eval.target` from each drained `Evaluation` but discards `eval.reconciler`. Inside `run_convergence_tick` (`reconciler_runtime.rs:217-285`), every registered reconciler is run against every drained target — N broker entries × M reconcilers = N×M reconcile calls per tick, instead of the N dispatches per distinct `(reconciler, target)` key that ADR-0013 §8 / whitepaper §18 promise.

## Observable consequence

**Phase 1 (current)**: harmless — the only two registered reconcilers are `job-lifecycle` and `noop-heartbeat`; the latter emits only `Action::Noop` whose `has_work` gate was just narrowed (`reconciler_runtime.rs:265`, commit `7a60743`). The fan-out wastes one extra reconcile + dispatch per tick per drained eval but does not stall convergence and does not produce wrong observation.

**Phase 2+ (regression-bait)**: adding any third reconciler with non-trivial `hydrate` or `reconcile` against a target shape it does not own (e.g. `cert-rotation`, `node-drain`) will:

- Run that reconciler's `hydrate_desired` / `hydrate_actual` against `job/<id>` targets it has no contract with, parsing the target via `job_id_from_target` (`reconciler_runtime.rs:463-468`) and issuing an IntentStore + ObservationStore read per tick per drained eval.
- Reach `reconcile` with a `desired` / `actual` projection that may not validate cleanly under the new reconciler's match arms — at best a no-op branch, at worst a panic or a wrongly-emitted action under a target the reconciler does not own.
- Defeat the §8 storm-proofing invariant end-to-end: the broker collapses to one entry per `(reconciler, target)`, then the dispatch path immediately re-fans-out across all reconcilers, restoring the very N×M cost the broker exists to suppress.

The structural problem is that `eval.reconciler` is **dead code** in the dispatch path. The broker keys on it; the drainer ignores it.

---

## 5 Whys

```
PROBLEM: run_convergence_tick ignores eval.reconciler — every registered
         reconciler runs against every drained target, defeating
         per-key collapse from the EvaluationBroker.

WHY 1 (Symptom): The drain loop at lib.rs:470 passes only eval.target
to run_convergence_tick; eval.reconciler never reaches the function.
[Evidence: lib.rs:470-481 — `for eval in pending { ... run_convergence_tick(
&state, &eval.target, now, tick_n, deadline).await ... }` — eval.reconciler
is read by no statement after the destructure. The function signature at
reconciler_runtime.rs:207-213 takes only `target: &TargetResource`.]

  WHY 2 (Context): Inside run_convergence_tick the body iterates
  `state.runtime.registered()` (registry-wide) for each invocation, running
  hydrate + reconcile + dispatch per registered reconciler.
  [Evidence: reconciler_runtime.rs:217-285 — `let registered =
  state.runtime.registered(); for name in &registered { ... }`. The for
  loop has no filter against an inbound reconciler-name parameter; one
  exists in the broker (`Evaluation { reconciler, target }` at
  eval_broker.rs:53-57) but never reaches this scope.]

    WHY 3 (System): The function was authored for a single-reconciler
    Phase 1 with an iterate-the-registry shape; the doc-comment still
    declares this contract today. The function signature was never
    updated when the second reconciler was registered.
    [Evidence: reconciler_runtime.rs:174-176 — `Drive ONE convergence
    tick against `target` for the registered `JobLifecycle` reconciler
    (Phase 1 single-target shape).` (Note the singular "the registered
    `JobLifecycle` reconciler"); reconciler_runtime.rs:214-216 — body
    comment `Phase 1: drive only the JobLifecycle reconciler against
    the target. NoopHeartbeat has no convergence behaviour against a
    resource target — it emits Action::Noop unconditionally.` The body
    contradicts both comments — it iterates ALL reconcilers, not just
    JobLifecycle. Git: commit 100b48e (Apr 28 2026, "feat(control-plane):
    runtime tick loop + DST invariants + backoff", step 02-03) — this is
    the commit that introduced the function with the for-loop-over-all-
    reconcilers shape, despite the doc-comment promising single-reconciler
    dispatch. The shape was the right *behaviour* for a registry of one
    (NoopHeartbeat at the time of the design) but became dispatch fan-out
    the moment JobLifecycle was registered alongside it.]

      WHY 4 (Design): The §18 storm-proofing invariant ("1 dispatch per
      distinct (reconciler, target) key per tick") is asserted at the
      broker boundary only. There is no end-to-end test that submits N
      Evaluations across M reconcilers and asserts M reconcile calls
      total.
      [Evidence: tests/acceptance/eval_broker_collapse.rs tests the broker
      surface in isolation (broker.submit / drain_pending counters).
      tests/acceptance/runtime_convergence_loop.rs:107-108 explicitly
      documents the wrong contract in its own comment: `the convergence-
      tick loop in lib.rs::run_server_with_obs_and_driver drains the
      broker per tick and runs every registered reconciler against each
      drained target` — the test author (and reviewer) believed the
      fan-out shape was correct because it was undocumented anywhere
      that it should not be. The §8 invariant is asserted on broker
      counters (eval_broker.rs:120-128), not on reconciler call counts.
      Even fix-noop-self-reenqueue's RCA called the fan-out an
      "amplifier" rather than a defect — bugfix-rca.md:14 — because the
      noop self-loop made it irrelevant in Phase 1.]

        WHY 5 (Root Cause): When `run_convergence_tick` was authored
        (commit 100b48e, step 02-03), the registry contained one
        reconciler (`NoopHeartbeat`), the broker keyed on `(name,
        target)` for forward-compatibility, and "iterate the registry"
        was indistinguishable from "look up by name" — both produced
        identical behaviour. The trait-object signature took only
        `target` because the per-reconciler dispatch shape was not yet
        load-bearing. JobLifecycle registration in commit 8f4aaa7
        ("feat(control-plane): job stop end-to-end closes convergence
        loop") added the second reconciler without revising the
        dispatch contract — the registry-iteration body silently became
        fan-out at that moment.
        [Evidence: git log on reconciler_runtime.rs (100b48e introduces
        the tick body; 8f4aaa7 is later); lib.rs:425-428 registers both
        reconcilers in the production boot path; eval_broker.rs:64-65
        keys on `(ReconcilerName, TargetResource)` from inception —
        confirmation that the `reconciler` field was *intended* to drive
        dispatch but the dispatcher never wired it through.]

        -> ROOT CAUSE: The dispatch path was written to a single-
           reconciler era and never updated when the registry grew. The
           `eval.reconciler` field is the broker's key half but
           contributes nothing to dispatch, and no end-to-end invariant
           catches the dead-code condition.
```

### Cross-validation (forward chain)

If the root cause holds — dispatch path written single-reconciler, never updated — then:

1. **Doc-comment / body mismatch must exist.** ✓ Verified: the doc-comment at `reconciler_runtime.rs:174-176` and body comment at `:214-216` both say *single reconciler*; the loop at `:217-218` iterates the registry. Three independent code-comment statements on the same function disagree about its contract.
2. **The single-reconciler era must be visible in git history.** ✓ Verified: commit `100b48e` (step 02-03) introduced the function with the registry-loop shape; `8f4aaa7` later added the second reconciler to the production boot path (`lib.rs:427-428`) without touching the dispatch loop.
3. **No test asserts "1 dispatch per distinct key, end-to-end."** ✓ Verified: `eval_broker_collapse.rs` tests the broker counters; `runtime_convergence_loop.rs` and the integration tests drive the loop directly without going through `eval.reconciler` at all (e.g. `submit_to_running.rs:79-87` calls `run_convergence_tick(&state, &target, ...)` directly, never constructing an `Evaluation`).
4. **Phase 1 happens to be benign.** ✓ Verified: NoopHeartbeat's `Action::Noop` is filtered from `has_work` (`reconciler_runtime.rs:265`, fix `7a60743`), so the wasted run does not produce a self-re-enqueue. The fan-out costs one extra hydrate+reconcile+dispatch per tick per eval, no semantic damage.

All four predicted artifacts exist; the root-cause chain is consistent.

---

## Contributing factors

- **Single-reconciler era ambient assumption.** When the function was authored the registry had one reconciler. "Iterate the registry" and "look up by name" were behaviourally identical, so the cheaper shape (no `find()` call, no `Option` to thread through error handling) won — the design constraint that `eval.reconciler` *must* drive dispatch was implicit, not enforced.
- **The doc-comments at `:174-176` and `:214-216` were not revised when `JobLifecycle` joined the registry.** A reviewer reading either comment would believe the function dispatches a single reconciler. The body's `for name in &registered` is the only place where the actual contract is visible, and the loop's body and the comments contradict each other. The bug is structurally hard to spot without a test that pins the contract.
- **The §8 invariant is broker-scoped, not end-to-end.** ADR-0013 §8 / `eval_broker.rs:120-128` assert collapse at the broker counter level. There is no acceptance or DST test that submits N Evaluations across M `(reconciler, target)` keys and asserts the dispatcher made M reconcile calls — the layer where the bug lives is exactly the layer the invariant suite skips.
- **Sibling RCA `fix-noop-self-reenqueue` named the fan-out as an "amplifier"** (`docs/feature/fix-noop-self-reenqueue/deliver/bugfix-rca.md:14`) but did not separate it as its own defect because the noop fix made it benign in Phase 1. The fan-out has been visible in code review at least once before; it survived because the symptom it produced was already being addressed elsewhere.

---

## Proposed fix — Option A (recommended)

**Dispatch only the reconciler named in `eval.reconciler`.**

### Code shape

The signature of `run_convergence_tick` changes to take the reconciler name. The loop over `registered()` is replaced by a single lookup via `reconcilers_iter().find(|r| r.name() == &name)`. If not found, log + skip cleanly (the reconciler may have been deregistered between submit and drain — Phase 2+ concern, but cheap to handle defensively).

```rust
// reconciler_runtime.rs:207
pub async fn run_convergence_tick(
    state: &AppState,
    reconciler_name: &ReconcilerName,
    target: &TargetResource,
    now: Instant,
    tick_n: u64,
    deadline: Instant,
) -> Result<(), ConvergenceError> {
    let Some(reconciler) =
        state.runtime.reconcilers_iter().find(|r| r.name() == reconciler_name)
    else {
        tracing::warn!(
            target: "overdrive::reconciler",
            reconciler = %reconciler_name,
            target = %target.as_str(),
            "convergence tick: reconciler not registered; skipping"
        );
        return Ok(());
    };

    let tick = TickContext { now, tick: tick_n, deadline };
    let desired = hydrate_desired(reconciler, target, state).await?;
    let actual  = hydrate_actual(reconciler, target, state).await?;
    let db = LibsqlHandle::default_phase1();
    let _ = reconciler.hydrate(target, &db).await.map_err(ConvergenceError::Hydrate)?;
    let view = cached_view_or_default(reconciler, target, state);

    let (actions, next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);
    store_cached_view(reconciler, target, state, next_view);

    let has_work = actions.iter().any(|a| !matches!(a, Action::Noop));

    action_shim::dispatch(actions, state.driver.as_ref(), state.obs.as_ref(), &tick)
        .await
        .map_err(ConvergenceError::Shim)?;

    if has_work {
        state
            .runtime
            .broker()
            .submit(Evaluation {
                reconciler: reconciler_name.clone(),
                target: target.clone(),
            });
    }
    Ok(())
}
```

Caller (`lib.rs:470-481`):

```rust
for eval in pending {
    if let Err(e) =
        run_convergence_tick(&state, &eval.reconciler, &eval.target, now, tick_n, deadline).await
    {
        tracing::warn!(
            target: "overdrive::reconciler",
            ?e,
            reconciler = %eval.reconciler,
            target_name = %eval.target.as_str(),
            "convergence tick error"
        );
    }
}
```

The doc-comments at `:174-176`, `:189-194`, and the body comment at `:214-216` are revised to describe the lookup-by-name dispatch — single reconciler per call, named by the inbound `Evaluation`.

### Why Option A over Option B (keep fan-out, document it)

- **Storm-proofing intent restored.** The broker's `(reconciler, target)` key becomes load-bearing in the dispatch path. ADR-0013 §8 / whitepaper §18 promise "N redundant submits collapse to 1 dispatch per distinct key" end-to-end; Option A is the only shape where the dispatcher honours that contract.
- **`eval.reconciler` stops being dead code.** The field exists in the type for a reason; Option A is the only fix where the type's shape and the dispatcher's behaviour agree.
- **Phase 2+ correctness without further changes.** Adding `cert-rotation` or `node-drain` reconcilers in later phases requires zero changes to the dispatch path. Under Option B, every new reconciler must defensively handle every other reconciler's target shapes.
- **Wasted I/O on the hydrate path is eliminated.** `hydrate_desired` / `hydrate_actual` for `JobLifecycle` parse the target via `job_id_from_target` and read from IntentStore + ObservationStore on every tick. Today, those reads happen for *both* reconcilers per drained eval — a 2× waste in Phase 1, scaling with M reconcilers in Phase 2+. Option A reduces this to one reconciler's worth of reads per drained eval.
- **The body contradicts its own comments today.** The comments at `:174-176` and `:214-216` already describe Option A semantics ("Drive ONE convergence tick … for the registered `JobLifecycle` reconciler"; "drive only the JobLifecycle reconciler against the target"). Option A makes the implementation match what the comments already promise; Option B requires *rewriting* the comments to defend a shape the comments themselves disclaim.

Option B (keep the fan-out, document it explicitly) is rejected. It would require:

- A new contract document explaining why the broker's key includes a field the dispatcher ignores.
- Defensive target-shape handling in every reconciler that joins the registry.
- A revised §8 invariant that asserts something weaker than the whitepaper currently promises.

The trade is "preserve a one-line, no-allocation `for` loop" against "preserve the storm-proofing invariant end-to-end." Option A wins on every axis.

---

## Files affected (production code)

- `crates/overdrive-control-plane/src/reconciler_runtime.rs` — function signature, body (replace registry loop with name-based lookup), revise three doc-comment blocks (`:174-176`, `:189-194`, `:214-216`) to describe the lookup-by-name contract.
- `crates/overdrive-control-plane/src/lib.rs:470-481` — pass `eval.reconciler` alongside `eval.target` into `run_convergence_tick`.

## Files affected (test code)

- `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs:107-108` — revise the comment that documents the wrong contract; update the call sites to pass the reconciler name (the existing test happens to submit `job-lifecycle` evals, so behaviour is preserved).
- `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs:128-130` — pass `&eval.reconciler` into `run_convergence_tick`.
- `crates/overdrive-control-plane/tests/acceptance/job_lifecycle_backoff.rs` — update any direct `run_convergence_tick` call sites to pass the reconciler name.
- `crates/overdrive-control-plane/tests/integration/job_lifecycle/submit_to_running.rs:79-87` — same; today this calls `run_convergence_tick(&state, &target, ...)` without going through the broker; the test must construct or pass a `ReconcilerName` for `job-lifecycle`.
- `crates/overdrive-control-plane/tests/integration/job_lifecycle/stop_to_terminated.rs` — same.
- `crates/overdrive-control-plane/tests/integration/job_lifecycle/crash_recovery.rs` — same.
- New regression test (see *Suggested regression test* below).

## Risk assessment

**Low risk.** Concretely:

1. **No current test depends on fan-out behaviour.** Searched call sites: every `run_convergence_tick` invocation is either (a) targeting the JobLifecycle path (`submit_to_running.rs`, `stop_to_terminated.rs`, `crash_recovery.rs`, `job_lifecycle_backoff.rs`) where the loop happens to find JobLifecycle by iteration but expects only that reconciler's effects, or (b) targeting the converged-state path (`runtime_convergence_loop.rs`) where NoopHeartbeat's emissions are now Noop-filtered (`reconciler_runtime.rs:265`, fix `7a60743`). No test asserts a side effect from a *different* reconciler than the one its `Evaluation` names.

2. **Reconciler-name match is exact and correct.** `JobLifecycle::canonical()` constructs `ReconcilerName::new("job-lifecycle")` (`crates/overdrive-core/src/reconciler.rs:1062`); `enqueue_job_lifecycle_eval` in `handlers.rs:43-50` constructs the same string at submit time. Identity by structural equality on the newtype. The lookup will succeed for every job-lifecycle submit produced by the production submit_job / stop_job paths.

3. **NoopHeartbeat's lookup will succeed too** (`crates/overdrive-core/src/reconciler.rs:742`, `noop-heartbeat`), but no production code path submits `noop-heartbeat` Evaluations to the broker today. If `noop-heartbeat` is later wired to a periodic reaper (per ADR-0013 §8 N=16 ticks), the lookup will succeed and dispatch will be correct.

4. **Self-re-enqueue narrowing is preserved.** The fix preserves the `has_work` check at `reconciler_runtime.rs:265` exactly as-is. Re-submission still includes the reconciler name (now from the inbound parameter rather than the loop variable).

5. **Integration tests bypass the broker drain.** Tests like `submit_to_running.rs:78-87` call `run_convergence_tick(&state, &target, ...)` directly. Under the current shape, this happens to work because the loop runs JobLifecycle. Under Option A, these test sites must be updated to pass `&job_lifecycle_name` explicitly. Mechanical update, no semantic change to test intent.

6. **No effect on storm-proofing of duplicate submits at the same key.** The broker's collapse logic (`eval_broker.rs:87-93`) is upstream of dispatch and unchanged.

7. **Dispatch error isolation unchanged.** The single-reconciler dispatch returns `Err(ConvergenceError)` exactly as today; the caller's `tracing::warn!` envelope at `lib.rs:474-479` continues to log per-eval errors without aborting the drain.

The only sharp edge: tests that today happen to exercise the JobLifecycle path "for free" because the for-loop runs all reconcilers will fail to compile until updated to pass the reconciler name. This is the surface area listed under *Files affected (test code)* — finite and mechanical.

## Suggested regression test shape

**Test name**: `eval_dispatch_runs_only_the_named_reconciler`

**Location**: `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs` (extend the existing file rather than create a new one).

**Tier**: Tier 1 default unit lane (no `integration-tests` feature gate; in-process serde + sim-adapter only, matches the existing `noop_heartbeat_against_converged_target_does_not_re_enqueue` test next to it).

**Shape**:

1. Build an `AppState` with both reconcilers registered (`noop_heartbeat()` and `job_lifecycle()`), backed by `LocalIntentStore`, `SimObservationStore`, `SimDriver`. Same fixture as `build_converged_state`.
2. Wrap or instrument the IntentStore (or the `hydrate_desired` / `hydrate_actual` paths) so the test can count "did `JobLifecycle::hydrate*` execute against `job/payments`?" and "did `NoopHeartbeat::hydrate*` execute against `job/payments`?". Cleanest shape: a counting wrapper around `IntentStore` and `ObservationStore` that counts `get(jobs/payments)` and `alloc_status_rows()` calls respectively. Alternative: assert via `broker.counters().dispatched` on a state where only one reconciler emits a non-Noop action.
3. Submit ONE `Evaluation { reconciler: "job-lifecycle", target: "job/payments" }`.
4. Drive ONE tick (drain → run_convergence_tick per eval).
5. Assert: `JobLifecycle`'s hydrate path executed exactly once against `job/payments` (IntentStore `get` count incremented). `NoopHeartbeat`'s hydrate did NOT execute against `job/payments` — under Option A, NoopHeartbeat is never looked up because no `(noop-heartbeat, …)` eval was submitted.

Today (pre-fix) this test fails because the registry-iteration body runs both reconcilers per drained eval, even though the eval named only one. After the fix it passes because the `find_by_name` call locates exactly the `job-lifecycle` reconciler and the loop body runs once.

A weaker but cheaper variant — assert `dispatched == 1` after one tick and one submit, with a counter verifying that only `job-lifecycle` reconcile was invoked — is acceptable if the IntentStore-wrapper plumbing is heavier than the test's value warrants. The strong shape is preferred because it pins the *causal* invariant (named reconciler → that reconciler's hydrate ran; unnamed reconciler → its hydrate did NOT run), not just the counter end-state.

## Out of scope (separate work item)

Add a DST invariant in `crates/overdrive-sim/src/invariants/evaluators.rs` asserting "for any drained `Evaluation { reconciler: R, target: T }`, exactly one reconciler — R — executes `hydrate` against T per tick." This is the end-to-end §8 invariant the suite is missing today (the broker collapse invariant exists; the dispatch invariant does not). Tracking this separately rather than bundling.

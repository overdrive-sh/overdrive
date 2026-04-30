# fix-eval-reconciler-discarded — Feature Evolution

**Feature ID**: fix-eval-reconciler-discarded
**Type**: Bug fix (`/nw-bugfix` → `/nw-deliver`)
**Branch**: `marcus-sa/phase1-first-workload`
**Date**: 2026-04-30
**Commits**:
- `76cc464` — `test(reconciler): pin drained Evaluation must dispatch only the named reconciler` (Step 01-01 — RED scaffold; committed `--no-verify` per `.claude/rules/testing.md` §RED scaffolds, since the test was authored against the post-fix arity of `run_convergence_tick` and the workspace is intentionally left in a RED state until 01-02 lands).
- `e6f5e5e` — `fix(reconciler): dispatch only the named reconciler per drained evaluation` (Step 01-02 — GREEN minimal fix + cascade test-call-site updates + un-ignore of the regression test, single cohesive commit).

**Status**: Delivered.

**Predecessor**: `docs/evolution/2026-04-29-fix-noop-self-reenqueue.md` (commit `7a60743`). That fix narrowed the *symptom* (Noop-driven self-re-enqueue loop). The bug captured here is the *upstream* dispatch-routing defect that made the symptom reachable in the first place. After the noop fix, Phase 1 became harmless; the dispatch-routing defect remained until this fix.

---

## Symptom

The convergence-loop drainer at `crates/overdrive-control-plane/src/lib.rs:470-481` consumed `eval.target` from each drained `Evaluation` but discarded `eval.reconciler`. Inside `run_convergence_tick` (`reconciler_runtime.rs:217-285`), the body iterated `state.runtime.registered()` (registry-wide) for every drained eval, running every registered reconciler against every drained target — N broker entries × M reconcilers = N×M reconcile calls per tick, instead of the N dispatches per distinct `(reconciler, target)` key that ADR-0013 §8 / whitepaper §18 promise.

The structural problem: `eval.reconciler` was **dead code** in the dispatch path. The `EvaluationBroker` keyed on `(ReconcilerName, TargetResource)` from inception (`eval_broker.rs:64-65`); the drainer ignored half the key.

**Phase 1 (current registry)** — harmless. The only two registered reconcilers are `job-lifecycle` and `noop-heartbeat`; the latter emits only `Action::Noop`, whose `has_work` gate was just narrowed by commit `7a60743` (filter `Action::Noop` from `has_work`). The fan-out wasted one extra hydrate+reconcile+dispatch per tick per drained eval but did not stall convergence and did not produce wrong observation.

**Phase 2+ (regression-bait)** — adding any third reconciler with non-trivial `hydrate` or `reconcile` against a target shape it does not own (e.g. `cert-rotation`, `node-drain`) would have:
- Run that reconciler's `hydrate_desired` / `hydrate_actual` against `job/<id>` targets it has no contract with, parsing the target via `job_id_from_target` and issuing IntentStore + ObservationStore reads per tick per drained eval.
- Reached `reconcile` with a `desired` / `actual` projection that may not validate cleanly under the new reconciler's match arms — at best a no-op branch, at worst a panic or a wrongly-emitted action under a target the reconciler does not own.
- Defeated the §8 storm-proofing invariant end-to-end: the broker collapses to one entry per `(reconciler, target)`, then the dispatch path immediately re-fans-out across all reconcilers, restoring the very N×M cost the broker exists to suppress.

## Root cause

When `run_convergence_tick` was authored (commit `100b48e`, step 02-03), the registry contained one reconciler (`NoopHeartbeat`), the broker keyed on `(name, target)` for forward-compatibility, and "iterate the registry" was indistinguishable from "look up by name" — both produced identical behaviour. The trait-object signature took only `target` because the per-reconciler dispatch shape was not yet load-bearing. JobLifecycle registration in commit `8f4aaa7` added the second reconciler to the production boot path (`lib.rs:425-428`) without revising the dispatch contract — the registry-iteration body silently became fan-out at that moment. Three independent code-comment statements on the function disagreed about its contract: the doc-comment at `:174-176` and body comment at `:214-216` both promised "single reconciler" while the loop at `:217-218` iterated the registry. The bug was structurally hard to spot without an end-to-end test pinning the contract; the §8 invariant is asserted on broker counters (`eval_broker.rs:120-128`), not on dispatched-reconciler call counts.

The dispatch path was written to a single-reconciler era and never updated when the registry grew. The `eval.reconciler` field was the broker's key half but contributed nothing to dispatch, and no end-to-end invariant caught the dead-code condition.

The five-whys chain, the doc-comment-versus-body contradiction, and the cross-validated forward chain are documented in full at the (preserved) RCA — see *References* below.

## Fix — RCA Option A

**Dispatch only the reconciler named in `eval.reconciler`.** Surgical, single-cut: production edits in `crates/overdrive-control-plane/src/reconciler_runtime.rs` (signature, body, three doc-comment blocks) plus `crates/overdrive-control-plane/src/lib.rs:470-481` (caller pass-through plus the `tracing::warn!` envelope's reconciler-name field), with mechanical cascade updates at five direct test call sites.

1. **Signature change** — `run_convergence_tick` takes `reconciler_name: &ReconcilerName` as its second parameter, ahead of `target: &TargetResource`.
2. **Body** — replace `let registered = state.runtime.registered(); for name in &registered { ... }` with `let Some(reconciler) = state.runtime.reconcilers_iter().find(|r| r.name() == reconciler_name) else { tracing::warn!(...); return Ok(()) };`. The `else` branch logs and skips cleanly when the reconciler has been deregistered between submit and drain (a Phase 2+ concern, defended against cheaply).
3. **Doc-comment blocks** — three blocks at `:174-176`, `:189-194`, `:214-216` rewritten to describe the lookup-by-name dispatch (single reconciler per call, named by the inbound `Evaluation`) instead of the registry-iteration shape they no longer described.
4. **Caller** — `lib.rs:470-481` passes `&eval.reconciler` alongside `&eval.target` into `run_convergence_tick`. The error-envelope `tracing::warn!` macro now renders `reconciler = %eval.reconciler` for log correlation.
5. **Cascade test updates** — five direct `run_convergence_tick(&state, &target, ...)` call sites updated to `(&state, &reconciler_name, &target, ...)`:
   - `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs` (the prior bugfix's regression test, plus the new one)
   - `crates/overdrive-control-plane/tests/acceptance/job_lifecycle_backoff.rs`
   - `crates/overdrive-control-plane/tests/integration/job_lifecycle/submit_to_running.rs`
   - `crates/overdrive-control-plane/tests/integration/job_lifecycle/stop_to_terminated.rs`
   - `crates/overdrive-control-plane/tests/integration/job_lifecycle/crash_recovery.rs`
6. **Un-ignore of the regression test** — `eval_dispatch_runs_only_the_named_reconciler` in `runtime_convergence_loop.rs` transitions ignored → executed-and-passing within the same commit (`e6f5e5e`).

**Rejected: RCA Option B** (keep the fan-out, document it explicitly). Would have required a new contract document explaining why the broker's key includes a field the dispatcher ignores, defensive target-shape handling in every reconciler joining the registry, and a revised §8 invariant weaker than the whitepaper currently promises. The trade was "preserve a one-line, no-allocation `for` loop" against "preserve the storm-proofing invariant end-to-end." Option A wins on every axis: storm-proofing intent restored, `eval.reconciler` stops being dead code, Phase 2+ correctness without further changes, wasted hydrate-path I/O eliminated, and the implementation finally matches what the doc-comments at `:174-176` and `:214-216` already promised.

## Verification

- **Regression test** — `eval_dispatch_runs_only_the_named_reconciler` at `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs`. Tier 1 default unit lane (no `integration-tests` feature gate; in-process serde + sim-adapter only, matches the existing `noop_heartbeat_against_converged_target_does_not_re_enqueue` test next to it). Builds an `AppState` with both reconcilers registered, submits one `Evaluation { reconciler: "job-lifecycle", target: "job/payments" }`, drives one tick, and asserts via the existing `view_cache` observation surface that exactly one reconciler — the named one — executed against the target. Pre-fix: assertion fails because the registry-iteration body runs both reconcilers per drained eval. Post-fix: assertion passes because `find_by_name` locates exactly the `job-lifecycle` reconciler and the loop body runs once.
- **Prior-bug regression preserved** — `noop_heartbeat_against_converged_target_does_not_re_enqueue` (commit `7a60743`'s test) continues to pass under the new arity. The two regression tests jointly defend the §8 invariant from both directions: "named reconciler runs and only that one runs" + "Noop emissions do not self-re-enqueue."
- **Mutation gate** — 90.5% kill rate (19/21 caught, threshold 80% met) on the diff-scoped run. The two missed mutations are pre-existing-surface mutations on `lib.rs:457` (deadline arithmetic) and `reconciler_runtime.rs:277` (`has_work` predicate — owned by the prior `fix-noop-self-reenqueue` change). Both are factually outside this RCA's diff surface; explicit out-of-scope notes captured in the roadmap and execution log.
- **Reviewer (nw-software-crafter-reviewer) verdict** — APPROVED with zero required changes. Zero testing-theater patterns detected.
- **DES integrity** — `verify_deliver_integrity` exit 0; both steps have complete DES traces.

## Lessons learned

- **Broker-key shape and dispatch shape must agree.** When a queue keys on `(A, B)` for collapse purposes, every consumer of the drained items must honour both halves of the key. A consumer that drains and uses only `B` — even if it happens to behave correctly today — defeats the queue's collapse intent the moment the registry grows. The audit at registration-time should enumerate every site that consumes the drained items and confirm each operates on the documented key shape, not on syntactic projections of one half.
- **Doc-comments should be a load-bearing contract.** Three independent code-comment statements on `run_convergence_tick` disagreed about its contract for weeks. The body's `for name in &registered` was the only place where the actual contract was visible. When fixing the bug, the comments at `:174-176` and `:214-216` already described Option A semantics — the implementation simply caught up to what the comments had promised all along. Reviewers reading either comment would have believed the function dispatched a single reconciler; the bug survived because no end-to-end test pinned the contract the comments described.
- **Detection gap.** The §8 invariant is broker-scoped (asserts collapse at the broker counter level via `eval_broker.rs:120-128`). There is no acceptance or DST test that submits N Evaluations across M `(reconciler, target)` keys and asserts the dispatcher made M reconcile calls — the layer where the bug lived was exactly the layer the invariant suite skipped. The sibling RCA `fix-noop-self-reenqueue` even named the fan-out as an "amplifier" rather than separating it as its own defect, because the noop fix made it benign in Phase 1. The fan-out had been visible in code review at least once before; it survived because the symptom it produced was already being addressed elsewhere.

## Out-of-scope follow-up

**DST invariant for `(reconciler, target)` dispatch routing.** Add a DST invariant in `crates/overdrive-sim/src/invariants/evaluators.rs` asserting *"for any drained `Evaluation { reconciler: R, target: T }`, exactly one reconciler — R — executes `hydrate` against T per tick."* This is the end-to-end §8 invariant the suite is missing today (the broker collapse invariant exists; the dispatch invariant does not). Tracked as a future `fix-` feature, NOT a step in this delivery.

## References

- RCA: `docs/feature/fix-eval-reconciler-discarded/deliver/bugfix-rca.md` (preserved in feature workspace; user-validated 2026-04-30). The 5-Whys chain, cross-validated forward chain, contributing factors, and Option-A-vs-B trade analysis are captured there in full and reproduced in compressed form above.
- Predecessor evolution doc: `docs/evolution/2026-04-29-fix-noop-self-reenqueue.md` (commit `7a60743` filtered `Action::Noop` from `has_work`, narrowing the symptom this fix addresses upstream).
- ADR: `docs/product/architecture/adr-0013-control-plane-reconciler-runtime.md` §8 (storm-proofing invariant: 1 dispatch per distinct `(reconciler, target)` key per tick).
- Whitepaper §18 *Reconciler and Workflow Primitives* — *Triggering Model — Hybrid by Design*, *Evaluation Broker — Storm-Proof Ingress*.
- Test discipline: `.claude/rules/testing.md` §RED scaffolds, §Mutation testing.
- Source artifacts: `roadmap.json` (2-step plan with full ACs) and `execution-log.json` (DES trace) live under `docs/feature/fix-eval-reconciler-discarded/deliver/` for the immediate post-mortem window; per the project's finalize protocol the per-feature directory is preserved (the wave matrix derives status from it) while session markers are removed.

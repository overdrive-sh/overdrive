# fix-noop-self-reenqueue — Feature Evolution

**Feature ID**: fix-noop-self-reenqueue
**Type**: Bug fix (`/nw-bugfix` → `/nw-deliver`)
**Branch**: `marcus-sa/phase1-first-workload`
**Date**: 2026-04-29
**Commits**:
- `61a1eb0` — `test(reconciler): pin noop-heartbeat must not self-re-enqueue under converged target (RED scaffold — see step 01-01 of feature fix-noop-self-reenqueue)`
- `7a60743` — `fix(reconciler): filter Action::Noop from has_work re-enqueue gate (Candidate A — see RCA at docs/feature/fix-noop-self-reenqueue/deliver/bugfix-rca.md)`

**Status**: Delivered.

---

## Symptom

`NoopHeartbeat::reconcile` (per its documented contract at `crates/overdrive-core/src/reconciler.rs:447`) returns `vec![Action::Noop]` on every tick to signal "nothing to do this tick." The `ReconcilerRuntime` level-triggered re-enqueue gate at `crates/overdrive-control-plane/src/reconciler_runtime.rs:256` (pre-fix) computed:

```rust
let has_work = !actions.is_empty();
```

— operating on **syntactic emptiness** rather than the documented `Action::Noop` sentinel **semantic**. Result: every `noop-heartbeat` tick produced a non-empty actions vec, tripped the `has_work` predicate, and self-re-enqueued the `(noop-heartbeat, target)` pair into the `EvaluationBroker`. For every active job, the broker's `dispatched` counter grew by ≥1 per tick forever; the convergence loop never settled even when intent and observation were perfectly aligned.

The `action_shim::dispatch` path at `action_shim.rs:108` already correctly treated `Action::Noop` as a no-op for cluster mutations — the bug was confined to the broker's re-enqueue gate, which was the one consumer that did not honor the documented sentinel.

## Root cause

The `noop-heartbeat` reconciler exists per ADR-0013 §9 to provide observable "proof of life" — broker activity that operators can monitor regardless of whether business reconcilers are emitting work. Its contract, established at `core/reconciler.rs:447`, is that it emits `vec![Action::Noop]` (not `vec![]`) on every tick: the emission must be **observable** to satisfy the proof-of-life intent, while semantically representing "no real work."

The §18 *Level-triggered inside the reconciler* design (whitepaper §18) requires the runtime to honor that semantic — re-enqueue only when there is genuine convergence work pending, not when a sentinel is the only signal. Two independent consumers of `Vec<Action>` existed in the runtime: `action_shim::dispatch` (cluster-mutation path) honored the sentinel correctly via per-variant dispatch; the broker re-enqueue gate operated on `is_empty()` and missed it. Adding a sentinel value without auditing every site that branches on `is_empty()` / `len() == 0` was the structural failure.

## Fix

**Approved fix shape (RCA Candidate A)**: filter `Action::Noop` from the `has_work` predicate at the runtime level. Surgical, single-cut: 4 edits in one cohesive commit (commit `7a60743`):

1. **Import** — add `Action` to the existing `overdrive_core::reconciler::{...}` import in `reconciler_runtime.rs`.
2. **Predicate replacement** — `let has_work = actions.iter().any(|a| !matches!(a, Action::Noop));` replaces `let has_work = !actions.is_empty();` at line 256.
3. **Comment block extension** — preserve the existing rationale about consume-by-value ordering with `action_shim::dispatch`, and append a paragraph explaining *why* `Action::Noop` is filtered: it is the documented "nothing to do this tick" sentinel per `core/reconciler.rs:447`, `action_shim::dispatch` already treats it as a no-op, and the §18 level-triggered re-enqueue gate must honor the documented semantic.
4. **Un-ignore regression test** — remove the `#[ignore = "RED scaffold..."]` attribute from `noop_heartbeat_against_converged_target_does_not_re_enqueue` in `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs`. The test transitions ignored → executed-and-passing within the same commit.

**Rejected: Candidate C** (change `NoopHeartbeat::reconcile` to return `vec![]` instead of `vec![Action::Noop]`) — would have made `Action::Noop` vestigial across the codebase, broken the doctest at `core/reconciler.rs:138`, broken several acceptance tests pinning the `vec![Action::Noop]` contract (`runtime_registers_noop_heartbeat.rs:179-180`, `reconciler_trait_surface.rs:486,511`, `any_reconciler_dispatch.rs:48,58`), and contradicted ADR-0013 §9's proof-of-life intent — the heartbeat must observably emit broker activity, which `vec![]` cannot.

## Why this fix is preferred

ADR-0013 §9 defines `noop-heartbeat` as the proof-of-life reconciler: its emission MUST be observable to satisfy the operational intent. Candidate A preserves the reconciler-side contract (`vec![Action::Noop]` is still emitted; the doctest at `core/reconciler.rs:138` continues to pass; every acceptance test pinning the contract continues to pass) and changes only the runtime's interpretation of that emission at the re-enqueue gate. The fix is confined to the one consumer that was misreading the sentinel; it does not touch the contract or any other consumer.

## Verification

- **Regression test** — `noop_heartbeat_against_converged_target_does_not_re_enqueue` at `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs`. Scaffolded RED in commit `61a1eb0` (marked `#[ignore]` so the lefthook gate stayed green without `--no-verify`); un-ignored in commit `7a60743`. Builds a converged-state `AppState` (sim observation/intent/driver, IntentStore preloaded with one job whose `desired.replicas == actual.allocations.running.count`), submits one `Evaluation { reconciler: "job-lifecycle", target: "job/payments" }`, drives 10 convergence ticks, and asserts both `counters.dispatched == 1` AND `counters.queued == 0`. Pre-fix: assertion 1 fails (actual ≥ 10). Post-fix: both assertions pass.
- **Mutation gate** — 100% kill rate (2/2 caught) on the step-scoped diff via `cargo xtask mutants --diff HEAD~1 --package overdrive-control-plane`. The two assertions of the regression test jointly killed every canonical predicate mutation: `has_work = true` (bugged behaviour) killed by `dispatched == 1`; `has_work = false` (suppress all reconciler emissions) killed by the existing `runtime_registers_noop_heartbeat.rs` proof-of-life invariant; `!matches!` flipped to `matches!` killed by both assertions jointly.
- **Workspace gates green**: `cargo nextest run --workspace` (510 tests), `cargo test --doc -p overdrive-control-plane`, `cargo nextest run --workspace --features integration-tests --no-run` (macOS typecheck), `cargo xtask dst` (14 invariants), `cargo xtask dst-lint`, `cargo clippy -p overdrive-control-plane --all-targets -- -D warnings`.
- **DES integrity** — `verify_deliver_integrity` exit 0; both steps have complete DES traces.

## Lessons learned

- **Sentinel-honouring discipline.** A sentinel value that means "nothing to do" must be honored by every consumer that branches on the actions vec, not just the dispatcher. When introducing a sentinel, the audit must enumerate every site that branches on `is_empty()` / `len() == 0` / `.is_empty()` / equivalent — those sites must operate on the documented semantic, not on syntactic emptiness. The bug here was that `Action::Noop` was added to satisfy ADR-0013 §9's proof-of-life intent without auditing the broker re-enqueue path that already existed.
- **Detection gap.** A DST invariant of the form *"after K ticks against a converged cluster, `broker.dispatched` is bounded by the number of distinct edge-triggered submits"* would have caught this pre-merge under DST replay. The reviewer flagged this as a separate work item — see *Follow-ups* below.

## Follow-ups

- **DST invariant for bounded broker dispatch on converged clusters.** RCA §Out of scope flagged that adding an invariant to `crates/overdrive-sim/src/invariants/evaluators.rs` asserting *"after K ticks against a converged cluster, `broker.dispatched` is bounded by the number of distinct edge-triggered submits"* would have caught this pre-merge. Tracked as a separate follow-up; explicitly NOT a step in this bugfix's roadmap.

## References

- RCA: `docs/feature/fix-noop-self-reenqueue/deliver/bugfix-rca.md` (preserved in feature workspace; user-validated 2026-04-29)
- ADR: `docs/product/architecture/adr-0013-control-plane-reconciler-runtime.md` §9 (proof-of-life intent for `noop-heartbeat`)
- Whitepaper §18 *Reconciler and Workflow Primitives* — *Triggering Model — Hybrid by Design*, *Level-triggered inside the reconciler*
- Reconciler contract: `crates/overdrive-core/src/reconciler.rs:447` (`NoopHeartbeat::reconcile` documented "nothing to do this tick" semantic)
- Test discipline: `.claude/rules/testing.md` §RED scaffolds, §Mutation testing

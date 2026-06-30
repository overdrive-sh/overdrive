# Adversarial Review - backend-instance-replacement step 01-02

**Reviewer**: Codex (`/nw-review`, adversarial)  
**Date**: 2026-06-30  
**Step**: `01-02` - Desired-run generation precursor + current-instance-scoped reconciler veto  
**Reviewed HEAD**: `c74d2d55`  
**Verdict**: `APPROVED_WITH_RESIDUAL_RISK`

## Scope Reviewed

- `docs/feature/backend-instance-replacement/deliver/roadmap.json` step `01-02`
- `docs/feature/backend-instance-replacement/distill/test-scenarios.md` S-BIR scenarios
- `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs`
- `crates/overdrive-core/tests/acceptance/workload_lifecycle_restart.rs`
- `crates/overdrive-control-plane/src/reconciler_runtime.rs`
- `crates/overdrive-store-local/src/redb_backend.rs`
- `docs/feature/backend-instance-replacement/deliver/mutants-01-02.md`
- Prior `review-01-02` blocker fixes: `b228982d`, `c74d2d55`

Review posture: adversarial code/test review against the step 01-02 acceptance criteria, with special attention to whether the green tests actually pin R2/R3/R4/R5, coalescing, scoped veto, numeric current-instance selection, and mutation evidence.

## Findings

### No blocking issues found

The two prior blockers are closed in the current code:

- Prior BLOCKER-1 (R5 draining instance gets `RestartAllocation`) is fixed by the `restart_pending && current_alloc(...).Draining` guard in `workload_lifecycle.rs:525-543`, and the test now asserts the full action set is empty in `workload_lifecycle_restart.rs:396-415`.
- Prior BLOCKER-2 (missing 01-02 mutation evidence) is closed by `mutants-01-02.md`, which records a non-vacuous diff-scoped run: 12 mutants, 12 caught, 0 missed, plus whole-file 57 caught / 2 missed / 3 unviable and a manual proof for the stamp blind spot.

### Residual risk - generation hydrator boundary is still not directly tested

`generation_value` in `reconciler_runtime.rs:2418-2445` is production code for the real desired-generation read, but step 01-02 tests primarily construct `WorkloadLifecycleState { generation: ... }` directly. The `runtime_convergence_loop` updates I found still use `generation: 0`, and the mutation evidence is scoped to `overdrive-core/src/reconcilers/workload_lifecycle.rs`, not the control-plane hydrator.

Static inspection says the key and byte order are coherent: `IntentKey` already establishes `workloads/{id}` and `workloads/{id}/stop` in `aggregate/mod.rs:1181-1193`, the store writes big-endian bytes in `redb_backend.rs:357-370`, and the hydrator reads `workloads/{id}/generation` with `u64::from_be_bytes`. But a wrong literal key or a broken fallback in `generation_value` would not be caught by the 01-02 pure reconciler suite.

This is not a blocker for this precursor step because 01-03 is the first slice expected to wire the handler/route producer. It must be closed there with a control-plane or route-level test that drives `IncrementU64` -> hydrate desired -> `reconcile` and proves the bumped generation actually reaches the reconciler.

### Trace nit - execution log still does not record the mutation gate

`mutants-01-02.md` is sufficient evidence for this review, but `deliver/execution-log.json` still lists only `PREPARE`, `RED_ACCEPTANCE`, `RED_UNIT`, `GREEN`, and `COMMIT` for `01-02`. If DES tooling treats `execution-log.json` as the canonical phase ledger, append a mutation/evidence event so the per-step status agrees with the landed artifact.

## Positive Evidence

- R2/R3/R4/R5 are pinned at the pure driving port. The R2 stop does not stamp, R3/R4 placement stamps `observed_generation = desired.generation`, and R5 now asserts `actions.is_empty()`, not merely "no second stop."
- The scoped veto is materially tested in both directions: stale superseded operator-stop rows no longer wedge crashed fresh instances, while a same-spec deploy with the current instance operator-stopped still emits no action.
- The numeric current-instance risk is covered twice: the reconciler-boundary property in `workload_lifecycle_restart.rs:690-738`, and the direct in-crate `current_alloc_tests` module in `workload_lifecycle.rs:1739-1868`.
- Mutation evidence accounts for the mandatory targets, including the cargo-mutants blind spot on `observed = desired` via a manual mutation proof.

## Validation Run

Passed under Lima:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-core --features integration-tests workload_lifecycle_restart
12 tests run: 12 passed

cargo xtask lima run -- cargo test -p overdrive-core --features integration-tests current_alloc_tests
4 tests run: 4 passed
```

Native macOS test attempts were not usable for this workspace because `linux-keyutils` fails to compile against Darwin libc symbols; the Lima path is the repo's expected route for these feature tests.

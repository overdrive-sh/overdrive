# Adversarial Review - backend-instance-replacement step 01-03

**Reviewer**: Codex (`/nw-review`, adversarial)  
**Date**: 2026-06-30  
**Step**: `01-03` - `restart_workload` HTTP handler + API types + `POST /v1/jobs/:id/restart` route  
**Reviewed HEAD**: `82683731`  
**Verdict**: `APPROVED_WITH_TRACE_NIT`

## Scope Reviewed

- `docs/feature/backend-instance-replacement/deliver/roadmap.json` step `01-03`
- `docs/feature/backend-instance-replacement/distill/test-scenarios.md` S-BIR handler scenarios
- `docs/feature/backend-instance-replacement/deliver/mutants-01-03.md`
- `docs/feature/backend-instance-replacement/deliver/execution-log.json`
- `crates/overdrive-control-plane/src/handlers.rs`
- `crates/overdrive-control-plane/src/api.rs`
- `crates/overdrive-control-plane/src/lib.rs`
- `api/openapi.yaml`
- `crates/overdrive-control-plane/tests/acceptance/restart_workload_unknown.rs`
- `crates/overdrive-control-plane/tests/acceptance/restart_workload_intent_key.rs`
- `crates/overdrive-control-plane/tests/acceptance/restart_workload_outcome.rs`

Review posture: adversarial code/test/evidence review against step `01-03`, with special attention to the 404 no-mutation posture, atomic bump+clear intent mutation, cosmetic outcome classification, route/API surface, and whether the earlier mutation-evidence blocker is now closed.

## Findings

### No blocking issues found

The prior blocker is closed. `mutants-01-03.md` records that the diff-scoped cargo-mutants run produced one unviable whole-function replacement and therefore no tool signal, then supplies executed manual mutation proofs for all named kill targets:

- 404 guard inversion is killed by `S-BIR-HANDLER-404`.
- Dropping `TxnOp::IncrementU64` is killed by `S-BIR-HANDLER-TXN`.
- Swapping `RestartOutcome` arms is killed by both outcome ATs.

That is enough evidence for this step because the step's required kill targets are explicitly covered and the non-signalling tool run is documented honestly rather than counted as a phantom pass.

### Trace nit - execution log still omits mutation evidence

`execution-log.json:75-108` records `01-03` through `PREPARE`, `RED_ACCEPTANCE`, `RED_UNIT`, `GREEN`, and `COMMIT`, but does not include a mutation/evidence phase pointing at `mutants-01-03.md`. This is not a blocker because the evidence artifact exists and is specific, but the canonical delivery ledger should be kept in sync to avoid future review ambiguity.

## Positive Evidence

- The handler sequence matches ADR-0073: parse id, 404 on absent aggregate, classify `/stop` presence for the response label only, commit one `txn([IncrementU64{generation}, Delete{stop}])`, enqueue the workload lifecycle evaluation, and return `{ workload_id, outcome }` (`handlers.rs:895-947`).
- The API surface is present and schema-registered: `RestartWorkloadResponse` and `RestartOutcome` use snake_case wire variants (`api.rs:125-156`) and are included in OpenAPI registration (`api.rs:419-435`). `api/openapi.yaml` also contains `/v1/jobs/{id}/restart` and the restart schemas.
- The production route is wired at `POST /v1/jobs/:id/restart` (`lib.rs:2329-2334`).
- The 404 AT asserts the `NotFound { resource: "workloads/nonexistent" }` shape, no generation key appears, no stop key appears, and broker queued count does not change (`restart_workload_unknown.rs:77-135`).
- The success-path AT asserts the observable state delta: generation `0 -> 1`, `/stop` cleared, `workloads/payments` retained byte-for-byte, and exactly one broker enqueue (`restart_workload_intent_key.rs:114-181`).
- Outcome classification is covered in both directions through the public handler response: present `/stop` returns `Resumed`, absent `/stop` returns `Restarted` (`restart_workload_outcome.rs:107-146`).

## Residual Risk

The roadmap asked for the op-set to be captured with a counting/fault-injecting store double, but `AppState.store` is concretely `Arc<LocalIntentStore>` (`lib.rs:173-176`), so adding that double would require production state reshaping. The shipped test uses real redb state-delta assertions instead. I accept that substitution because `01-01` already validates `IncrementU64`, and `mutants-01-03.md` manually kills the load-bearing op-set drift by dropping `IncrementU64` and observing `S-BIR-HANDLER-TXN` fail.

## Validation

Per instruction, I did not run AC gates in this review pass. This review is based on static inspection plus the committed evidence in `mutants-01-03.md`.

## Decision

Approve step `01-03` with a non-blocking trace nit. The handler, API/route surface, acceptance coverage, and mutation evidence satisfy the step contract.

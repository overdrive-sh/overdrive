# Adversarial Review - backend-instance-replacement step 01-04

**Reviewer**: Codex (`/nw-review`, adversarial)  
**Date**: 2026-07-01  
**Step**: `01-04` - `overdrive workload restart` CLI verb  
**Reviewed HEAD**: `c9a48a6a`  
**Verdict**: `APPROVED_WITH_TRACE_NITS`

## Scope Reviewed

- `docs/feature/backend-instance-replacement/deliver/roadmap.json` step `01-04`
- `docs/feature/backend-instance-replacement/distill/test-scenarios.md` S-BIR CLI scenarios
- `docs/feature/backend-instance-replacement/deliver/execution-log.json`
- `docs/feature/backend-instance-replacement/deliver/mutants-01-04.md`
- `crates/overdrive-cli/src/cli.rs`
- `crates/overdrive-cli/src/commands/mod.rs`
- `crates/overdrive-cli/src/commands/workload.rs`
- `crates/overdrive-cli/src/http_client.rs`
- `crates/overdrive-cli/src/main.rs`
- `crates/overdrive-cli/src/render.rs`
- `crates/overdrive-cli/tests/integration.rs`
- `crates/overdrive-cli/tests/integration/workload_restart.rs`

Review posture: adversarial code/test/evidence review against step `01-04`, with special attention to the CLI driving adapter, binary dispatch wiring, `ApiClient::restart_workload`, output-label preservation, unknown-id error handling, and the mandatory mutation gate. Per instruction, I did not run AC gates.

## Findings

### No blocking issues found

The two previous blockers are closed.

`BLOCKER-1` is closed because the CLI tests no longer assert over the whole `RestartOutcome` enum. The declared-workload scenario asserts the deterministic `Restarted` label for an absent `/stop` sentinel (`workload_restart.rs:144`), and the added stopped-workload scenario asserts deterministic `Resumed` after the production stop verb writes `/stop` (`workload_restart.rs:188`, `workload_restart.rs:209`). Together these tests kill both hardcode directions and prove `commands::workload::restart` preserves `resp.outcome` (`commands/workload.rs:79`).

`BLOCKER-2` is closed because `mutants-01-04.md` now records the diff-scoped cargo-mutants run as non-signalling (`total=0`, `INFO No mutants to filter`) and supplies executed manual mutation proofs for the named kill targets. The evidence covers hardcoded `RestartOutcome::Resumed`, hardcoded `RestartOutcome::Restarted`, wrong route literal `restart-WRONG`, and swallowed 404 fabricated success (`mutants-01-04.md:44`, `mutants-01-04.md:150`, `mutants-01-04.md:200`, `mutants-01-04.md:245`).

### Trace nit - execution log still describes the old two-scenario shape

**Dimension**: completeness / traceability  
**Severity**: low  
**Location**: `docs/feature/backend-instance-replacement/deliver/execution-log.json:125`

The `RED_UNIT` skip rationale still says "the two port-to-port acceptance scenarios" and names only `S-BIR-CLI-RESTART-SUCCESS / S-BIR-CLI-RESTART-UNKNOWN`. The current reviewed test set has three scenarios after the `Resumed` addition. This does not affect behavior, but the delivery ledger should reflect the final reviewable shape.

Recommended fix: update the log text to include `S-BIR-CLI-RESTART-RESUMED`.

### Trace nit - execution log does not point at mutation evidence

**Dimension**: completeness / traceability  
**Severity**: low  
**Location**: `docs/feature/backend-instance-replacement/deliver/execution-log.json:138`

The mutation evidence exists in `mutants-01-04.md`, but the execution log still stops at the original `COMMIT` event and has no mutation/evidence phase or artifact pointer. This is the same non-blocking ledger drift noted in the prior step review pattern: the artifact is present and specific, but the canonical timeline is not synchronized.

Recommended fix: add a `MUTATION` or `EVIDENCE` entry for `01-04` pointing to `deliver/mutants-01-04.md`.

### Trace nit - test module registry comment omits the added `Resumed` scenario

**Dimension**: documentation consistency  
**Severity**: low  
**Location**: `crates/overdrive-cli/tests/integration.rs:99`

The module registry comment lists the success and unknown scenarios, but not `workload_restart_of_stopped_workload_returns_resumed`. The test is compiled through `mod workload_restart`, so this is not a coverage issue.

Recommended fix: add the `S-BIR-CLI-RESTART-RESUMED` bullet to the registry comment.

## Positive Evidence

- The CLI namespace and command surface are present: `Command::Workload(WorkloadCommand)` and `WorkloadCommand::Restart { id }` (`cli.rs:65`, `cli.rs:105`).
- The new handler module is exported from `commands/mod.rs`, and `commands/workload.rs` validates `WorkloadId`, loads the configured endpoint, calls `ApiClient::restart_workload`, and returns `RestartOutput` preserving `workload_id`, `outcome`, and `endpoint` (`commands/workload.rs:71`).
- `ApiClient::restart_workload` posts to the expected production route, `v1/jobs/{id}/restart`, through the existing typed POST path (`http_client.rs:292`).
- The binary dispatch gap is closed: `main.rs` has a `Command::Workload(WorkloadCommand::Restart { id })` arm that calls the library handler, renders `workload_restart_accepted`, and maps errors through `cli_error_to_exit_code` (`main.rs:155`).
- The renderer distinguishes `Restarted` and `Resumed` operator-facing output and includes the endpoint (`render.rs:169`).
- The direct-handler integration tests exercise the production route through a real in-process server and real `LocalIntentStore`: deploy + restart for `Restarted`, deploy + stop + restart for `Resumed`, and unknown restart for typed 404 + non-zero exit mapping (`workload_restart.rs:110`, `workload_restart.rs:171`, `workload_restart.rs:224`).

## Residual Risk

The direct-call test shape intentionally does not spawn the actual `overdrive` binary, per `crates/overdrive-cli/CLAUDE.md`. Static review covers the thin `main.rs` dispatcher. That is acceptable for this step because the mutable handler and route surfaces are covered by the direct-handler tests and manual mutation evidence, while the dispatcher mirrors the existing deploy/stop/alloc posture.

`mutants-01-04.md` cites code-under-test SHA `f2646da3`; HEAD is now `c9a48a6a`, whose diff is the mutation evidence artifact itself. That is acceptable because no production or test code changed after `f2646da3`.

## Validation

Per instruction, I did not run AC gates. This review is based on static inspection plus the committed evidence in `mutants-01-04.md`.

## Decision

Approve step `01-04` with trace nits. The CLI verb, route binding, binary dispatch, renderer, acceptance coverage, and mutation evidence satisfy the step contract. The remaining issues are ledger/comment synchronization only.

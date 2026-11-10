# DELIVER Review — Step 02-01

## Review metadata

| Field | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `02-01` — Restart authority names pre-startup crash loops |
| Review iteration | 1 |
| Reviewer | Codex, fresh isolated DELIVER reviewer |
| Implementation commit | `2e6a466cc08836a3aacb03cebdf19c167f8c8462` |
| COMMIT audit commit | `84f363119d70c2751c7a0cf6ac5b87f8f7caf200` |
| Comparison base | `1ac102f1c58c1a02f3b234af06c4caede463cf5f` |
| Authoritative contract | `feature-delta.md` D6, B3, DG-12 and interface changes; ADR-0258; approved roadmap step 02-01 |
| Review verdict | **APPROVED** |

This review is bounded to D6, B3, DG-12 and step 02-01's listed tests. D5/D7 startup-probe stop behavior and the later 02-02 obligations remain in their later roadmap step. No defect finding, design change, or implementation change is proposed.

## Executive summary

`WorkloadLifecycle` now makes the D6 cause-aware give-up decision from the current attempt's durable `AllocStatusRow.reason`, declared startup-probe descriptors, and the latest startup result for that attempt. The decision happens only in the existing exhausted-budget branch, after the liveness terminal case. It selects the existing `ServiceFailed { EarlyExit { exit_code } }` shape for a failed Service attempt with a declared startup probe and no current-attempt `Pass`; `WorkloadCrashedImmediately` preserves its optional exit code and `VmOutOfMemory` maps to `None`. Other failure reasons, a recorded `Pass`, an opted-out Service, a non-Service workload, and a `Terminated` clean exit remain on the existing terminal path.

`ServiceLifecycle` no longer classifies or finalizes a `Failed` row. At the head of its per-allocation loop it records every `Failed` or `Terminated` allocation in `terminal_announced` and emits no action for that allocation, regardless of `started_at`. The public `ServiceAllocFact.exit_code` field and its hydration are deleted as D6 specifies. The current `ServiceFailureReason::EarlyExit` variant shape and all wire, archived, and persisted shapes remain unchanged.

The D6/B3/DG-12 acceptance and named conformance selection is recorded as 17/17 passing. The feature's full Quint evidence has 84/84 expected outcomes and passes `verify-evidence`; strict Clippy and the affected-crate formatting check are recorded as passing. The wider control-plane suite has known resource-budget failures whose comparison-base result is unestablished; they are not attributed to this step. I reviewed the retained logs and did not rerun tests or mutation testing.

## Contract and production-path review

### D6 and B3 — sole give-up classifier and exact terminal shape

The accepted decision in `feature-delta.md` D6 and ADR-0258 limits classification to the existing restart-ceiling decision and says it must not alter restart selection, backoff, budget accounting, or the platform-reclamation and generation-replacement exemptions. The code remains inside the existing `!restart_decision(...)` branch at `crates/overdrive-reconcilers/src/workload_lifecycle.rs:1150-1238`; the prior idempotency guard now also recognizes the existing `EarlyExit` terminal at `:1161-1171`. The existing liveness result retains precedence at `:1220-1226`, followed by the D6 selection at `:1227-1230`, then the unchanged `BackoffExhausted` fallback at `:1231-1233`.

The selection predicate at `workload_lifecycle.rs:1196-1217` matches the accepted semantic set: `Service`, at least one `Startup` descriptor, row state `Failed`, and a latest current-attempt startup status other than `Pass`; only `WorkloadCrashedImmediately` and `VmOutOfMemory` reasons are classified. The `VmOutOfMemory` arm yields `Some(None)` internally, which constructs the existing `EarlyExit { exit_code: None }`; it does not add or change an enum variant. `CleanExit` is observer-authored as `Terminated` with `Stopped { by: Process }` (`crates/overdrive-control-plane/src/worker/exit_observer.rs:608-624`), so it does not meet the `Failed` predicate and remains `BackoffExhausted` as the accepted D6 contract requires. Fail-closed reasons fall through to `BackoffExhausted`.

The production hydration path at `workload_lifecycle.rs:690-738` reads the durable allocation rows, selects `current_alloc`, and reads probe results only for a current `Failed` or `Terminated` row. It filters to `ProbeRole::Startup` and probe index 0, selects the greatest `run_seq`, and populates the authorized `WorkloadLifecycleState.current_startup_probe` field. Superseded attempts cannot supply the classification input. This matches the field's pinned meaning in D6 item 4; the field was an accepted-design scaffold, not a new API introduced by this commit.

### DG-12 — every terminal fact closes the ServiceLifecycle watch

`ServiceLifecycleReconciler::reconcile` checks terminal row state first at `crates/overdrive-reconcilers/src/service_lifecycle.rs:568-574`; it inserts every `Failed` or `Terminated` fact in `terminal_announced` and continues before Stable, startup-stop, or any other per-allocation decision. This is independent of `started_at`, probe result, or startup-probe declaration. The deleted EarlyExit branch and `ServiceAllocFact.exit_code` hydration are absent; the live startup-stop path remains below the guard (`service_lifecycle.rs:658-675`). The S-56 control preserves the `Running` case as outside `terminal_announced`.

The branch removal and terminal-membership insertion are the exact DG-12 rule, not a new lifecycle policy. The insertion closes the mid-startup window so `has_alloc_mid_startup_window` and `next_evaluation_at` no longer keep re-enqueueing a terminal allocation; the D6 S-56 case checks the returned view, action list, scheduling boundary, and repeat-evaluation stability.

### Hydration error contract

The approved semantic error is the existing `HydrateError::ObservationRead(String)` variant. Both the allocation-row read and the current-attempt probe-result read map observation-store errors to that exact variant in `workload_lifecycle.rs:694-698` and `:707-710`. A missing probe row maps to `None`, but a failed store read propagates as an error; it is not treated as “no Pass.” `HydrateError` converts through the existing `ConvergenceError::Hydrate` variant (`crates/overdrive-control-plane/src/reconciler_runtime.rs:1878-1903`), and the convergence loop records a failed evaluation (`crates/overdrive-control-plane/src/lib.rs:5211-5227`). The activated acceptance case asserts the nested `HydrateError::ObservationRead` representation. The designed failure-to-error mapping is complete, so no `SPECIFICATION_AMBIGUITY` is present.

### Restart-budget read and Service stream stderr

`restart_budget_from_rows` maps the existing `ServiceFailed { EarlyExit }` claim to `used = RESTART_BACKOFF_CEILING` and `exhausted = true` while leaving other variants, including `LivenessProbeFailed`, unchanged (`crates/overdrive-control-plane/src/handlers.rs:1409-1420` and the following match). No public response shape changes.

For the existing Service stream, `build_service_stream` now takes a crash's stderr tail from the typed `WorkloadCrashedImmediately` reason and retains `event.detail` as the fallback for other causes (`crates/overdrive-control-plane/src/streaming.rs:1098-1112`). The action shim forward-carries the crash reason and prior stderr tail to the final row (`crates/overdrive-control-plane/src/action_shim/mod.rs:1985-2000, 2084-2103`); `lifecycle_event_from_occurrence` then projects the written row's reason and detail (`action_shim/mod.rs:497-506`). This makes the accepted D6 stream outcome available without changing `LifecycleEvent`, the Service event, or any wire/persisted format.

## Exact API and typed-error contract

| Designed change or failure | Approved representation | Implementation review |
|---|---|---|
| Remove ServiceLifecycle's crash-ending input | Delete `ServiceAllocFact.exit_code` and its hydration | Deleted from the public fact and hydration; no new replacement field on that interface |
| Read the current attempt's startup result | Existing approved `WorkloadLifecycleState.current_startup_probe: Option<ProbeStatus>` | Populated by `hydrate_actual` from the current attempt's startup row only |
| Give up on a qualifying pre-startup crash | Existing `TerminalCondition::ServiceFailed { reason: ServiceFailureReason::EarlyExit { exit_code: Option<i32> } }` | Exact existing shape; crash code copied, OOM uses `None` |
| Probe-result read failure | `HydrateError::ObservationRead(String)` | Both result-read failure and allocation-row read failure map to the approved variant; failure does not collapse to `None` |
| EarlyExit restart-budget projection | Existing `RestartBudget { used, max, exhausted }` | Used equals policy ceiling and exhausted is true; no response fields added |

No public method, type, enum variant, trait method, parameter, persisted representation, wire field, or configuration surface was added. The only public interface deletion is the D6-pinned `ServiceAllocFact.exit_code`. `ServiceFailureReason::EarlyExit` remains unchanged in shape; only its documentation now describes give-up timing rather than an elapsed startup-deadline claim (`crates/overdrive-core/src/transition_reason.rs:703-707`).

## Test integrity and coverage

### Activated obligations and observability

The roadmap step activates the D6/B3/DG-12 composed crash cases, current-attempt hydration/error cases, EarlyExit restart-budget projection, S-56 terminal-membership grid, and three named `restart_authority` conformance runs. The authored bodies now exercise:

- repeated guest crashes through the production exit observer, action shim, `WorkloadLifecycle`, `ServiceLifecycle`, probe runner, and Service stream;
- successor success after a predecessor crash, crash-loop give-up, restart before the ceiling decision across control-plane restart, and `VmOutOfMemory` with no exit code;
- latest startup status for the current attempt, superseded-row exclusion, and typed read failure;
- budget exhaustion for EarlyExit;
- `Failed` and `Terminated` terminal membership over the S-56 start-history/probe/declaration grid, plus a non-terminal `Running` control;
- model conformance for whole-workload ServiceLifecycle ticks and the resulting D6/DG-12 membership.

The composed acceptance bodies use production owner paths with `Sim*` adapters at driven ports; they do not seed a final EarlyExit row or drive the built Overdrive binary. The Quint conformance driver calls the production convergence composition and reads the approved row/View projections. Its model update addresses the recorded seed `0x3fbbc5ad` divergence: a whole-workload ServiceLifecycle tick acknowledges every terminal fact before the target action, matching D6/DG-12. The DISTILL review records the reproduction, corrected model abstraction, and unchanged production driver/projection in `distill/review-crash-authority-fixture-remediation.md`.

### Test changes and Contract Shape

The deleted `ServiceAllocFact.exit_code` field caused concrete compiler failures in existing fixtures. The approved DISTILL remediation records 13 pre-remediation diagnostics, removes only that obsolete fixture member, relocates the crash-cause assertion to the durable observation row, strengthens terminal membership assertions, removes the two tests for the deleted EarlyExit branch, and updates the hydration golden by exactly one field. The same review approves correcting the finalizer hook oracle to account for the retained D4 hook (one call per crash plus the finalizer's repeated final-id call, with one idempotent supervision effect). I verified these changes against the DISTILL remediation artifact and its independent APPROVED iteration-2 review.

No surviving assertion was weakened or skipped to accommodate the implementation. The deleted EarlyExit-branch tests defended behavior D6 removes; S-56 now tests the approved terminal-row outcome across its state grid. D7-specific pending cases remain ignored for step 02-02. All acceptance bodies newly activated or modified for this step retain per-test `CONTRACT_SHAPE` declarations. The production changes add no source-local properties.

### Model and evidence integrity

The whole-workload membership adjustment to `specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt` is limited to the already approved DG-12 rule. It does not introduce a new model state, owner, persistence mechanism, policy, or production seam. The change keeps the existing production driver and observable projection; the seed that previously diverged now passes. The retained full formal check reports 84/84 expected results, and `cargo xtask quint verify-evidence --subsystem service-allocation-readiness-and-ending` reports current evidence hashes and expected outcomes match. Mutation testing was not run, consistent with the roadmap's single final-wave mutation gate.

## TDD and quality-gate review

The DES log records the step's `RED / FAIL`, two intermediate `GREEN / FAIL` attempts, final `GREEN / PASS`, and `COMMIT / PASS` in timestamp order (`deliver/execution-log.json`). The RED log fails the activated composed crash case because the first crash prematurely closes the Service stream, which is the accepted pre-fix failure. The final focused log records the same 17-test D6/B3/DG-12 selection passing. The COMMIT phase is on disk and the commit includes `Step-Id: 02-01`, `Task-Id: service-allocation-readiness-and-ending`, and exactly `Co-Authored-By: Codex <codex@openai.com>`; the Git author remains Marcus and no Claude/Anthropic attribution is present.

| Gate | Result | Evidence |
|---|---|---|
| RED/GREEN/COMMIT order | PASS | `deliver/execution-log.json`; semantic RED failure and final GREEN/COMMIT pass logs |
| Correct RED failure | PASS | `.context/step-02-01-red-main.log` fails because the first crash ends the deploy stream |
| Focused step selection | PASS, reviewed retained evidence | `.context/step-02-01-green-selection-iteration2.log` reports 17 passed, 0 failed |
| Strict Clippy | PASS, reviewed retained evidence | `.context/step-02-01-clippy-final.log` runs Clippy with `-D warnings` for control-plane, reconcilers, core, and sim |
| Affected-crate formatting | PASS, reviewed retained evidence | `.context/step-02-01-fmt-iteration2-final.log` runs `cargo fmt --check` for the four affected crates |
| Quint full check and evidence | PASS, reviewed retained evidence | DISTILL iteration-2 logs report 84/84; `.context/step-02-01-quint-evidence-final.log` verifies evidence |
| Test modifications | PASS | Test and fixture diffs match the independently approved DISTILL remediation; no unapproved weakening or skipped D6 test |
| Mutation gate | Deferred by design | The roadmap reserves one mutation gate after all step reviews |

The earlier wider control-plane acceptance run is recorded as 413 passed, 3 failed, and 49 skipped, with the three `job_stop_*` failures reporting `FdBudget { required: 376832, available: 65536 }`. Their result at the comparison base is unestablished. I treat them as a documented validation limit, not as proven pre-existing failures or as a regression in this step. Workspace-wide formatting status is likewise not claimed from the affected-crate formatting pass; no unrelated formatting change is required by this step.

## Findings and remediation disposition

No findings. The exact public interface and typed-error contracts match the accepted design; no production defect requiring remediation was identified. No design gap or `SPECIFICATION_AMBIGUITY` is present. No remediation iteration is required.

## Final verdict

**APPROVED** — Step 02-01 implements the D6/B3 give-up classification, DG-12 terminal membership, EarlyExit budget projection, and typed crash-stderr projection within the approved architecture and exact interface shape. The accepted D6 failure modes map to their approved typed terminal/error representations, the activated composed tests and named conformance evidence pass, and the changed test/support evidence has independent DISTILL approval. Step 02-02 may proceed after the orchestrator completes its mechanical checks and confirms this on-disk review artifact.

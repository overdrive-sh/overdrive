# DELIVER Review — Step 02-02

## Review metadata

| Field | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `02-02` — Startup-probe failure stops the allocation |
| Review iteration | 1 |
| Reviewer | Codex, fresh isolated DELIVER reviewer |
| Implementation commit | `b036498c5a1cbf942f89d2abc2a55a83e3becbd7` |
| COMMIT audit commit | `859e1bafb818b6907fc6fa6233f2f44b18b8b1de` |
| Comparison base | `9b186aa3ddf8bec048810ab2d956936a6705f138` |
| Authoritative contract | Feature delta D5, D7 and D7 interface changes; ADR-0257, ADR-0259, ADR-0261; `brief.md`; approved roadmap step 02-02 |
| Review verdict | **APPROVED** |

This review covers the approved startup-stop disposition, give-up ownership and projections required by step 02-02. The per-row failed-stop retry remains step 02-03, and the default-window stream deadline remains step 04-01. Neither is treated as a step 02-02 omission.

## Executive summary

`ServiceLifecycleReconciler` now emits the existing `Action::StopAllocation` with `StoppedBy::StartupProbe` when the startup-failure streak, deadline and no-pass gates all hold. It declines to act on `Failed` and `Terminated` rows. `WorkloadLifecycle` recognizes the final attempt's startup-stop disposition, continues restarting it below the existing ceiling, and at give-up emits the existing `Action::FinalizeFailed` with `ServiceFailed { StartupProbeFailed }`, the final attempt's probe failure and the consumed restart budget. The action shim writes the restartable `Terminated` row for each stop; `service_event_from_terminal` suppresses its stream line, while the give-up and a successor's `Stable` still end the stream. The restart-budget query and `workload describe` expose the approved outcomes.

The implementation uses the approved fieldless tail variant `StoppedBy::StartupProbe` and adds no other public interface or error variant. Test-body changes activate the pre-authored cases and replace an obsolete debug-string oracle with exact action and view-membership checks. The separate DISTILL model/oracle correction was approved after two iterations; its seeded reproductions and formal evidence are reviewed below.

## Contract and production-path review

### D5 — terminal rows receive no startup-probe verdict

The per-allocation loop checks `Failed | Terminated`, records the allocation in `terminal_announced`, and continues before startup decisions (`crates/overdrive-reconcilers/src/service_lifecycle.rs:567-573`). The private startup action helper also uses the accepted `AllocState::is_terminal()` guard before evaluating the gates (`:1224-1245`). For the qualifying non-terminal row, the action is the exact existing `StopAllocation` shape with `Stopped { by: StartupProbe }`; the old live `FinalizeFailed { StartupProbeFailed }` is gone. The returned view stamps the existing run marker and `terminal_announced` at emission (`:665-677`), consistent with the approved emission-time state; failed-stop retry semantics remain for 02-03.

### D7 — stop is restartable; `WorkloadLifecycle` alone gives up

The new `StoppedBy::StartupProbe` representation is fieldless and tail-appended after `LivenessProbe` at discriminant 6, as ADR-0259 and the feature delta require (`crates/overdrive-core/src/transition_reason.rs:412-419`). The exhaustively updated human-readable and reclamation mappings classify it as `stopped (startup probe)` and not platform-reclaimed (`:917-918`, `:1038-1051`). The variant was already an approved scaffold; the implementation commit removes its scaffold marker and uses that exact shape.

The existing `StopAllocation` production arm awaits driver stop and cleanup, writes the `Terminated` row carrying the supplied terminal, and fires the terminal hook only after the row is durable (`crates/overdrive-control-plane/src/action_shim/mod.rs:3238-3393`). `WorkloadLifecycle` leaves the intentional-stop set limited to Operator and SystemGc (`crates/overdrive-reconcilers/src/workload_lifecycle.rs:1606-1621`), so a startup stop remains restartable by the unchanged `is_restartable` predicate (`:1728-1732`). At the ceiling, its existing idempotency guard recognizes `StartupProbeFailed`, and the new private `is_startup_killed` predicate reads only the final row's `Terminated` state and `StoppedBy::StartupProbe` terminal (`:1161-1172`, `:1221-1248`, `:1649-1655`). It selects the already-approved `ServiceFailureReason::StartupProbeFailed { probe_idx: 0, last_fail, attempts }`; `attempts` is the consumed restart count, and `last_fail` uses the designed total mapping from the final attempt's startup row. The existing reason's shape is unchanged; its documentation now states the approved give-up meaning (`transition_reason.rs:694-702`).

### Stream, budget and describe projections

`service_event_from_terminal` returns `None` for both restartable probe stops, including `StartupProbe`, while other stop causes retain their projection (`crates/overdrive-control-plane/src/streaming.rs:883-918`). `build_service_stream` continues its receive loop when the projection is `None` (`:1113-1125`). The final `ServiceFailed { StartupProbeFailed }` still maps to a `Failed` event. `restart_budget_from_rows` marks that give-up exhausted with `used = RESTART_BACKOFF_CEILING` (`handlers.rs:1409-1438`), and the Service describe row prints the presence-guarded `terminal: stopped (startup probe failed)` line only for the approved stopped-by variant (`crates/overdrive-cli/src/render.rs:1186-1202`).

The designed semantic failure-to-representation mapping is complete:

| Designed condition | Approved interface representation | Implementation |
|---|---|---|
| Startup gates hold on a non-terminal Service allocation | `StopAllocation { terminal: Some(Stopped { by: StartupProbe }) }` | Exact action from `ServiceLifecycleReconciler`; no live `StartupProbeFailed` finalization |
| A startup-stopped final attempt remains below the restart ceiling | Restartable `Terminated` row carrying `StoppedBy::StartupProbe`; existing `RestartAllocation` policy and budget | `is_intentionally_stopped` does not match startup; existing `is_restartable` includes the row |
| Restart budget is spent on a startup-stopped final attempt | `ServiceFailed { StartupProbeFailed { probe_idx: 0, last_fail, attempts } }`, with final-attempt failure detail and consumed restart count | Exact ceiling branch in `WorkloadLifecycle`; no new terminal variant or field |
| Restartable startup stop is observed by the open Service stream | No `ServiceSubmitEvent` line | Exact `None` projection; the stream continues |
| Startup give-up is queried/rendered | Existing `RestartBudget` exhausted at the policy ceiling; startup-stop describe line on the predecessor row | Exact existing response and rendering shapes |

No new fallible interface was introduced. The D7 current-attempt probe read uses the already-approved `HydrateError::ObservationRead(String)` mapping; both allocation-row and probe-row read failures remain errors rather than becoming an absent probe (`workload_lifecycle.rs:694-715`). No `SPECIFICATION_AMBIGUITY` is present.

## Test integrity and coverage

The roadmap names 15 acceptance bodies for this step: two ceiling-classification cases, two D5 crash cases, the startup-stop property, three reconcile-branch cases, the Stable branch, three composed startup-stop journeys, the restart-budget projection, stream projection and CLI describe. The implementation diff activates those bodies and the seven named D7 `restart_authority` runs. The changed bodies retain their required `CONTRACT_SHAPE` declarations. The composed control-plane `startup_stop_authority` journeys drive the production reconciler/runtime, action shim, exit observer, probe runner and Service stream with `Sim*` adapters at the driven ports; the direct `overdrive-core` reconciler acceptance tests assert returned actions and view. The Rust tests do not spawn the production binary or act as expectation runners.

The commit diff removes pending `#[ignore]` markers for the accepted 02-02 obligations. It does not weaken or delete their assertions. The S-54 `service_lifecycle_startup_stop_gate` helper was changed from searching `Debug` output for the removed live `StartupProbeFailed` action to matching the exact `StopAllocation { Stopped { StartupProbe } }`, rejecting a wrong stop or obsolete failure finalization, and asserting matching `terminal_announced` membership. The independent DISTILL review records the reproduced stale-oracle failure and approves this stricter correction. The default-window scenario remains pending D10/04-01; the D8 failed-stop retry cases remain pending 02-03; the GuestListeners threshold cells remain right-reason RED at streak one and the deadline. Those pending tests were not falsely activated or claimed green.

The D7 Quint `WLGiveUp` model correction was reviewed in `distill/review-startup-give-up-model-remediation.md` and its independent review artifact. At seed `0x50d50042`, the named runs had matched the typed `StartupProbeFailed` claim and `WorkloadLifecycle` author but disagreed on the final row class. The model now records the production `FinalizeFailed` row state (`Failed`) while keeping the typed claim and author as separate oracles (`restart_authority.qnt:348-365`; driver projection at `quint_restart_authority.rs:763-791`). The retained review verifies the real production composition path and that the driver/projection were not rewritten to fit the model. The two fixed-seed runs pass after the correction, all seven D7 named conformance runs pass, and the recorded formal suite remains 84/84 with all 13 input hashes matching. The unrelated full-feature evidence was regenerated; no expected check was changed.

The crafter's handoff reports 69 relevant final checks, 15 activated acceptance bodies, seven conformance runs, both fixed-seed cases, evidence verification, strict control-plane Clippy, scoped formatting and diff checks passing. The workspace retains the independent DISTILL logs (`.context/step-02-02-distill-named-after.log`, `step-02-02-distill-seed-before.log`, `step-02-02-distill-seed-after.log`, `step-02-02-distill-quint-verify.log`, and the gate-control logs); the crafter's final selection log was not present in `.context`, so I record its final count as handoff-reported and did not claim to have independently rerun it. I ran `git diff --check` on the implementation commit and it passed. I did not rerun the broad suite or mutation testing; mutation testing remains the single final DELIVER-wave gate.

## TDD and quality-gate review

The DES log has step 02-02 `RED / FAIL`, two intermediate `GREEN / FAIL` events, two `GREEN / PASS` events and `COMMIT / PASS`, in timestamp order (`deliver/execution-log.json`). The audit commit adds only the final COMMIT event. The two intermediate failures remain recorded; the final pass is not presented as the only attempt. I reviewed the commit and retained DISTILL evidence rather than repeat broad verification.

| Gate | Result | Evidence |
|---|---|---|
| RED → GREEN → COMMIT sequence | PASS | `deliver/execution-log.json`; final GREEN and COMMIT pass are present in order |
| Required acceptance activation | PASS | 15 roadmap-named bodies activated; seven D7 conformance bodies activated |
| Test modifications | PASS | No weakened/deleted assertions; S-54 oracle change is an exact typed strengthening independently approved by DISTILL |
| Contract Shape | PASS | Newly activated/transitioned step bodies carry their required per-test declaration |
| Public API shape | PASS | Only the approved scaffolded `StoppedBy::StartupProbe` shape is used; no additional method, type, variant or parameter |
| Typed error contract | PASS | No new error surface; current-attempt probe read retains `HydrateError::ObservationRead(String)` |
| Final acceptance and quality selection | PASS, handoff-reported | 69 relevant checks, 15 activated bodies and seven conformance runs; no independent rerun |
| Quint evidence | PASS, independently reviewed retained logs | 84/84 expected checks and 13 matching input hashes; seeded correction reviewed separately |
| Formatting and diff | PASS | Crafter-reported scoped format; independent `git diff --check` passed |
| Mutation gate | Deferred by design | One final DELIVER-wave gate, not per step |

The implementation commit and audit commit retain Marcus as Git author and each has exactly `Co-Authored-By: Codex <codex@openai.com>`, `Step-Id: 02-02` and `Task-Id: service-allocation-readiness-and-ending`; neither carries Claude or Anthropic attribution. The implementation commit is bounded to the D5/D7 production paths, the named projections and tests, the accepted Quint model/evidence correction and its reviewed support artifacts. The audit commit changes only `execution-log.json`.

## Findings and remediation disposition

No findings. The D5 guard, D7 stop/give-up ownership, typed terminal mapping, restartability, stream projection and user-facing projections match the accepted contract. The public API and error contracts are complete for this step. The independently reviewed model/oracle correction preserves the production path and acceptance assertions. No DESIGN gap, `SPECIFICATION_AMBIGUITY`, unproven defect promoted to a finding, or remediation iteration is required.

## Final verdict

**APPROVED** — Step 02-02 implements the accepted startup-stop and give-up behavior with the exact approved interface shapes, activates its specified tests without weakening their assertions, and records the reviewed D7 model correction and COMMIT event. Step 02-03 may proceed after the orchestrator completes its mechanical checks and confirms this on-disk review artifact.

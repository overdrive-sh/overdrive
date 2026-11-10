# DELIVER Review — Step 03-01: Liveness supervision begins at Stable

## Review metadata

| Field | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `03-01` — Liveness supervision begins at Stable |
| Iteration | 1 |
| Reviewer | Codex, fresh isolated DELIVER reviewer |
| Implementation commit | `641f546aec009cfa4a5af416ea919948d76334e` |
| COMMIT audit commit | `eb1d3e56332c0695706d977b1f4e0a0470ef7b46` |
| Comparison base | `4652c403e8aff6bd1cbe7d146608e48bab97df5d` |
| Authoritative contract | Approved roadmap step 03-01; feature-delta D9 and G-6; ADR-0261; ADR-0259 and ADR-0266 for the existing typed liveness-stop and stop-write retry behavior; `docs/product/architecture/brief.md` |
| Upstream prerequisite | Step 02-04 review is `APPROVED`; roadmap validation is approved (`deliver/review-roadmap.md`) |
| Verdict | **APPROVED** |

## Executive summary

The implementation moves only the liveness task start point. `ProbeRunner::start_alloc` now starts Startup and Readiness tasks and retains already-projected Liveness descriptors on the allocation supervisor. The new public method has the exact D9 signature, `ProbeRunner::start_liveness(&self, alloc_id: &AllocationId)`. `VmDriver::on_alloc_stable` retires Startup and starts retained Liveness; the action shim invokes that existing hook only after the Stable row write succeeds. The supervisor lock serializes the start with `stop_alloc`, and consuming retained descriptors makes repeated Stable hooks idempotent.

The former `stable_this_tick` detector skip is removed as D9 specifies. The service detector still emits the existing typed `Stopped { by: LivenessProbe }` action after the liveness threshold, and `WorkloadLifecycle` still owns restart and gives up with `ServiceFailed { LivenessProbeFailed }` at the shared budget ceiling. I found no public API divergence, typed-error gap, weakened acceptance oracle, or unproven production defect in the reviewed step.

## Contract and production-path review

### D9 lifecycle and task ownership

`ProbeRunner::start_alloc` registers one supervisor, projects each descriptor, retains Liveness descriptors, and spawns Startup and Readiness (`crates/overdrive-worker/src/probe_runner/mod.rs:314–353`). The retained vector is private supervisor state; `take_retained_liveness` consumes it once (`crates/overdrive-worker/src/probe_runner/supervisor.rs:91–107, 173–182`). This preserves descriptor projection and prevents liveness tasks from creating rows before Stable.

`ProbeRunner::start_liveness` takes the same supervisor-map lock used by `stop_alloc`, returns when the allocation is absent or its root token is cancelled, then consumes and spawns the retained descriptors while still holding the lock (`probe_runner/mod.rs:362–405`). If `stop_alloc` obtains the lock first, the subsequent start sees no supervisor. If start obtains it first, it creates the tasks under the existing supervisor and the later stop removes and cancels that supervisor. No liveness task remains associated with an ended allocation. A second Stable hook finds no retained descriptors and spawns no duplicate task.

`VmDriver::on_alloc_stable` calls `stop_role(Startup)` followed by `start_liveness` (`crates/overdrive-worker/src/vm_driver.rs:2063–2074`). At the production composition boundary, the Stable arm awaits `write_alloc_lifecycle(...).await?` before resolving the allocation's driver and calling `on_alloc_stable`; a failed row write returns before the hook (`crates/overdrive-control-plane/src/action_shim/mod.rs:2145–2155`). The task's tick loop sleeps for the descriptor interval before its first probe (`probe_runner/mod.rs:796–845`), so the first newly-produced liveness row and its streak begin after the committed Stable hook.

The `stable_this_tick` set and skip were removed from `ServiceLifecycleReconciler::reconcile` and `collect_liveness_actions` (`crates/overdrive-reconcilers/src/service_lifecycle.rs:546–548, 628–634, 1014–1028`). This is bounded to D9: liveness rows are no longer produced by the new allocation's runner before the Stable hook. The existing detector still branches on the Running row, the threshold-crossing failure streak, and whether the run is newer than the last acted run (`service_lifecycle.rs:1041–1063`).

### Production composition and observable coverage

The composed liveness fixture uses the production `ReconcilerRuntime`, action shim, `VmDriver`, and real `ProbeRunner`; Sim adapters occupy the driven-port boundaries (`crates/overdrive-control-plane/src/reconciler_runtime/liveness_gate_fixture.rs:1–14`). Its tests verify no pre-Stable role or row, the first post-Stable row and streak, opt-out Stable behavior, the existing failed-stop-row retry, and the restart-budget give-up path (`liveness_gate_composed.rs:48–225, 228–355`). The restart case checks the exact `ServiceFailed { LivenessProbeFailed { probe_idx: 0, attempts: RESTART_BACKOFF_CEILING } }` outcome and that no guest-network lease remains. This is in-process production composition; the test does not start the built Overdrive binary or emit expectation evidence.

The worker acceptance test drives the real `VmDriver` and shared `ProbeRunner` lifecycle hooks. Its changed oracle now checks Liveness absent at Running, present after Stable, and absent after terminal while retaining the Startup, Readiness, and supervisor-count complements (`crates/overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs:470–515`). The independent DISTILL remediation and its review document that this was an obsolete pre-D9 Running-phase expectation; they approve the bounded oracle correction and confirm the production hook path (`distill/liveness-stable-hook-fixture-remediation.md`, `distill/review-liveness-stable-hook-fixture-remediation.md`).

### Exact interface contract and typed error mapping

The only new public surface is `ProbeRunner::start_liveness(&self, alloc_id: &AllocationId)`, matching D9 item 2 exactly (`probe_runner/mod.rs:388–405`; feature-delta D9 interface table). It has no explicit result type, as specified. `Driver::on_alloc_stable` keeps its existing signature and gains only the approved doc sentence (`crates/overdrive-core/src/traits/driver.rs:823–842`). `retained_liveness`, `retain_liveness`, and `take_retained_liveness` are private or `pub(super)` implementation details. No public type, enum variant, trait method, parameter, persisted field, wire representation, or new error variant was added.

| Designed semantic condition | Approved and implemented representation |
|---|---|
| A liveness failure occurs before Stable | No Liveness task or row is produced; the detector therefore emits no liveness stop. This is the D9 gate, not a new error result. |
| The post-Stable liveness failure streak reaches its threshold | `ServiceLifecycle` emits `Action::StopAllocation` with `TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }` (`service_lifecycle.rs:1054–1063`). |
| The shared restart budget is exhausted after liveness stops | `WorkloadLifecycle` emits `TerminalCondition::ServiceFailed { reason: ServiceFailureReason::LivenessProbeFailed { probe_idx: 0, attempts } }` (`crates/overdrive-reconcilers/src/workload_lifecycle.rs:1219–1227`); the composed acceptance asserts the ceiling payload. |
| The Stable hook names an unknown, ended, or already-stopped allocation | `start_liveness` is the specified infallible no-op; the signature returns no generic/string-carried failure. |

No new semantic error condition was introduced by D9, so no unresolved typed-error contract or `SPECIFICATION_AMBIGUITY` remains. The existing liveness failure mapping is explicit and unchanged.

## Test integrity and TDD review

The step activates 16 D9 acceptance/composed/conformance cases named by the roadmap and runs six VM-hook control cases: 22 selected tests total. These are composed acceptance and conformance checks rather than new standalone input-variation unit tests, so the unit-test budget gate is not applicable. The active new/transitioned test functions carry their per-test Contract Shape declaration; there is no new source-local pure-function property requiring the `pure-function` declaration.

The implementation diff removes the pending ignore markers from the assigned D9 scenarios. The only acceptance-body oracle change is the separately approved VM hook chronology correction described above: its Running-phase assertion now checks the D9 outcome, and Stable/terminal complements remain asserted. No assertion was dropped or weakened to accommodate the implementation. The tests exercise the production composition or the actual VM-driver-to-runner hook, with Sim adapters at driven ports; the Quint tests activate the existing conformance model without modifying it.

The DES log records RED/FAIL, an intermediate GREEN/FAIL, final GREEN/PASS, and COMMIT/PASS in order (`deliver/execution-log.json`, events for `03-01`). The RED logs fail on the old behavior: Liveness supervision or rows exist before Stable, and the Quint implementation diverges at that gate. The intermediate GREEN failure was the existing worker test still asserting the pre-D9 Running-phase behavior; it was routed to DISTILL, corrected in the bounded oracle remediation, and independently approved before the step resumed. No acceptance-test body or production change was made by this reviewer.

## Findings and remediation disposition

No blocking, high, medium, or low findings were identified in iteration 1. The implementation follows the D9 ownership, state, ordering, idempotence, lifecycle, and typed-output contract. The upstream DISTILL oracle correction is approved and closed; there is no remediation request for the step crafter.

## Verification evidence

| Check | Result |
|---|---|
| RED: `liveness_begins_at_stable` five assigned tests | Expected failure against the pre-D9 implementation; failures identify liveness supervision or rows before Stable (`.context/step-03-01-red-runner.log`). |
| RED: composed D9 and Quint selection | Expected pre-fix failures; the existing passing controls remain passing (`.context/step-03-01-red-composed-quint.log`). |
| GREEN: D9 acceptance and composed/conformance selection | **16/16 passed** via `cargo xtask metal run -- cargo nextest run` (`.context/step-03-01-green-focused-final.log`). |
| GREEN: full selected step and six VM controls | **22/22 passed** across four binaries via Metal (`.context/step-03-01-green-final.log`). |
| Clippy | **PASS** for `overdrive-core`, `overdrive-worker`, `overdrive-reconcilers`, and `overdrive-control-plane`, all targets, `integration-tests`, `-D warnings` (`.context/step-03-01-green-clippy-final.log`). |
| Doctests | **PASS**: four passed, three existing ignored, zero failed across the four affected crates (`.context/step-03-01-green-doctest.log`). |
| Formatting | The recorded whole-workspace `cargo fmt --all --check` reports only the existing diffs in untouched `crates/overdrive-testing/bin/fwd_guest_suite.rs` and `xtask/tests/acceptance.rs` (`.context/step-03-01-green-fmt-initial.log`). Scoped formatting passed per the crafter handoff. |
| Diff whitespace | Reviewer ran `git diff --check 4652c403e8aff6bd1cbe7d146608e48bab97df5d..641f546aec009cfa4a5af416ea919948d76334e`; **PASS**. |
| Mutation testing | Not run; the repository requires one final DELIVER-wave mutation gate after all step reviews. |

Both step commits retain Marcus as Git author. Each contains exactly `Co-Authored-By: Codex <codex@openai.com>`, `Step-Id: 03-01`, and `Task-Id: service-allocation-readiness-and-ending`, with no Claude or Anthropic attribution. The implementation commit's scope is limited to the D9 worker lifecycle, the now-unreachable detector skip, assigned tests, DES evidence, and the independently reviewed DISTILL oracle record. The audit commit changes only `execution-log.json`. The workspace was clean before this review artifact was written; this reviewer made no implementation, test, model, DES, or commit changes.

## Final verdict

**APPROVED — iteration 1.** Step 03-01 matches the accepted D9 interface and production lifecycle, preserves the typed liveness-stop and shared-budget give-up representations, and passes the assigned executable verification evidence.

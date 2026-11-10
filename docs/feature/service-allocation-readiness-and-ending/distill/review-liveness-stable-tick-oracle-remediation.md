# Independent DISTILL review — obsolete same-tick liveness oracle

## Metadata and verdict

| Item | Value |
|---|---|
| Scope | Removal of one unreachable D9 pure-input test and its `host-pure` Metal placement entry during step 05-04 |
| Trigger | DELIVER `review-05-04.md` iteration-1 finding D1, routed to DISTILL rather than production remediation |
| Protected work | Original step 05-04 crafter's D16-G1/D14-E2 implementation, DES events and eventual review cycle |
| Verdict | **APPROVED** for this bounded DISTILL test-contract correction; step 05-04 GREEN still must be rerun |

## Reproduced failure and real owner path

The active `stable_this_tick_suppresses_liveness_restart_same_tick` test failed on Metal before deletion (`.context/step-05-04-stable-tick-before.log`). It directly constructed a `ServiceLifecycleReconciler::reconcile` input with a Running allocation, `stable_committed=false`, a Startup `Pass`, and an already-crossed Liveness `Fail` row. Current reconcile returned both `FinalizeFailed { Stable }` and `StopAllocation { Stopped { LivenessProbe } }`; the test demanded only Stable. The failure was real for that synthetic input, but the input was **not reachable through the accepted D9 production owners**.

The actual sequence is `ProbeRunner::start_alloc` retaining Liveness descriptors instead of spawning their tasks at Running (`probe_runner/mod.rs:336–356`), the action shim committing the `Stable` allocation row **before** calling `Driver::on_alloc_stable` (`action_shim/mod.rs:2180–2191`), and `VmDriver::on_alloc_stable` retiring Startup then invoking `ProbeRunner::start_liveness` (`vm_driver.rs:2109–2115`). A Liveness probe row cannot be produced before that Stable commit and hook. ADR-0261's accepted consequence explicitly removes the now-unreachable `stable_this_tick` liveness skip; the current production `ServiceLifecycle` no longer has such a guard. The deleted test's crossed pre-Stable Liveness row was test-authored state, not a failure through the real producer/owner path. Reinstating the guard would contradict the approved producer-side gate and expand step 05-04 beyond its D16-G1/D14-E2 scope.

## Subtraction and retained observable outcomes

The DISTILL diff removes only that test body, its placement identity and comments that claimed a live `stable_this_tick` suppression mechanism (`service_lifecycle_reconcile_branches.rs:1556–1690`; `infra/metal/test-placement.json`). No assertion in a reachable test was weakened, no test marker was changed, and no production code, model, public interface or test seam was added. The retained tests cover the meaningful outcomes at their proper boundaries:

| Outcome | Retained evidence |
|---|---|
| Startup `Pass` on a Running allocation yields one exact `FinalizeFailed { Stable }` | Core `stable_fires_when_running_and_startup_probe_pass` and composed Stable behavior. |
| Liveness tasks and rows are absent before Stable; the first row follows the committed Stable hook | Composed `liveness_supervision_begins_when_the_allocation_becomes_stable` and `first_liveness_row_lands_one_interval_after_stable_with_streak_one` (D9/S-31/S-32). |
| A crossed Liveness streak **after** committed Stable yields one `StopAllocation { Stopped { LivenessProbe } }` without re-announcing Stable | Core `liveness_resumes_after_prior_tick_stable_announcement` and `s_roh_a_01_liveness_threshold_emits_stop_allocation`. |
| A due startup stop has its own typed terminal and no competing liveness stop in the direct reconciler input | Retained `startup_probe_failure_suppresses_liveness_stop_same_tick`; a defensive pure-input check, not a claim that pre-Stable liveness rows are production-reachable. |

The retained transitioned tests carry per-test `/// CONTRACT_SHAPE: bounded-change.` declarations. The deleted placement entry was the only inventory change; the already accepted D9 owner order and all valid state/threshold complements remain.

## Verification and disposition

| Evidence | Result and limit |
|---|---|
| Before | The obsolete test failed exactly on the two-action synthetic input, exit 100 (`.context/step-05-04-stable-tick-before.log`). |
| After | Four focused core Stable/startup/liveness controls passed **4/4** and two composed D9 hook/first-row controls passed **2/2** (`.context/step-05-04-stable-tick-controls-core.log`, `-controls-composed.log`). |
| Static | Author reports changed-file rustfmt, placement JSON parse and `git diff --check` passing. My read-only diff review, JSON parse and `git diff --check` passed. No full suite or mutation run belongs to this bounded review. |

**Verdict: APPROVED for DISTILL's removal of an impossible D9 input oracle.** The DELIVER 05-04 review finding is resolved as a test-contract issue, with the valid Stable and post-Stable liveness outcomes still covered. The original step 05-04 crafter must rerun the required GREEN selection and commit before its reviewer may re-evaluate the production step; this artifact does not approve that step.

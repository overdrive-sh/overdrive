# Step 05-04 DISTILL remediation: obsolete Stable-tick liveness oracle

## Reproduced failure and reachability

The active `stable_this_tick_suppresses_liveness_restart_same_tick` core acceptance test failed on Metal before this edit (`.context/step-05-04-stable-tick-before.log`). Its direct `ServiceLifecycleReconciler::reconcile` input combined a Running allocation with `stable_committed=false`, a Startup `Pass`, and an already-crossed Liveness `Fail` row. Current reconcile emitted both `FinalizeFailed { Stable }` and `StopAllocation { Stopped { LivenessProbe } }`; the test required only Stable.

That input is not reachable through the accepted D9 production owner path. `ProbeRunner::start_alloc` retains Liveness descriptors and starts no Liveness task at Running. The action shim writes the allocation's Stable row before it calls `VmDriver::on_alloc_stable`; that hook retires Startup and calls `ProbeRunner::start_liveness`. The first Liveness row is therefore produced only **after** `stable_committed=true` can be hydrated. ADR-0261 expressly removes the former `stable_this_tick` skip as unreachable. The direct synthetic state supplies a row no production owner could have written before Stable, so this failure does not prove a production defect or authorize restoring that skip.

## Bounded correction and replacement coverage

The obsolete single test body and its `host-pure` Metal placement entry were removed. No production code or other test body changed. The adjacent comments now describe D7's retained startup-stop defensive pure-input check and D9's reachable post-Stable liveness path without claiming a surviving `stable_this_tick` mechanism.

The deleted test's meaningful observable pieces remain covered at their actual boundaries:

| Outcome | Retained evidence |
| --- | --- |
| A Running allocation with Startup `Pass` emits one exact `FinalizeFailed { Stable }` | Core `stable_fires_when_running_and_startup_probe_pass` and the composed Stable journey. |
| No Liveness task or row exists before Stable; Startup retires and Liveness starts after the Stable hook | Composed `liveness_supervision_begins_when_the_allocation_becomes_stable` and `first_liveness_row_lands_one_interval_after_stable_with_streak_one` (S-31/S-32). |
| After committed Stable, a crossed Liveness streak emits exactly `StopAllocation { Stopped { LivenessProbe } }` without a second Stable | Core `liveness_resumes_after_prior_tick_stable_announcement` and `s_roh_a_01_liveness_threshold_emits_stop_allocation`. |
| A startup stop carries its own terminal and does not emit a competing liveness stop in the pure reconciler input | Existing `startup_probe_failure_suppresses_liveness_stop_same_tick`, retained unchanged. |

All retained transitioned tests above already declare per-test `/// CONTRACT_SHAPE: bounded-change.`. No action count, threshold, terminal, readiness, or state complement was relaxed.

## Validation and handoff

| Check | Result |
| --- | --- |
| Before: Metal `cargo nextest run -p overdrive-core --test acceptance -E 'test(stable_this_tick_suppresses_liveness_restart_same_tick)'` | Reproduced the exact two-action counterexample, exit 100. |
| After: four named core Stable/startup/liveness controls | 4/4 passed (`.context/step-05-04-stable-tick-controls-core.log`). |
| After: two composed D9 hook/first-row controls | 2/2 passed (`.context/step-05-04-stable-tick-controls-composed.log`). |
| Changed-file rustfmt, placement JSON parse, `git diff --check` | Passed. |

This is DISTILL test-contract fallout from the already accepted D9 design, separate from the otherwise accepted 05-04 D16-G1/D14-E2 implementation. The original 05-04 crafter can rerun its full required GREEN selection after independent DISTILL review. No DES event, production change, mutation pass, or commit was made here.

# Step 02-04 DISTILL remediation: committed Stable test fallout

## Contract and reproduced failure

B1/ADR-0265 makes the allocation's committed `Stable` row, hydrated as `ServiceAllocFact.stable_committed`, the announcement authority. R2-2 deletes `ServiceLifecycleView.stable_announced` and its View-only `has_alloc_mid_startup_window` predicate. D12/ADR-0267 gates a startup-probed backend on its own committed `Stable`; R2-1 deletes the terminal startup veto and its order before the startup stop. D7/D8 retain the restartable `StopAllocation { Stopped { StartupProbe } }` and one emission per new probe row. S-38 expressly keeps the CBOR check that an old payload with the deleted key decodes.

Before these edits, the activated S-38 test `blob_carrying_the_removed_stable_announced_key_is_ignored` failed on Metal: the current production decoder retained `{svc-0}` in `stable_announced`, while the expected field-free view had no such state. The current production view still has the field; this is a deliberate semantic RED until the step 02-04 crafter implements R2-2. Other old test assertions directly read or construct that soon-to-be-deleted field and would then prevent compilation.

## Old oracle → approved outcome

| Existing assertion or fixture | Bounded correction and retained coverage |
| --- | --- |
| `next_view.stable_announced` proves a `Stable` emission | Keep the exact `FinalizeFailed { Stable }` action and witness assertions; check the observed allocation where a view assertion is useful. The composed runtime test now checks the committed `AllocStatusRow.terminal == Stable`, the actual durable outcome. |
| A seeded `stable_announced` set suppresses a second `Stable` | Set `ServiceAllocFact.stable_committed = true` and assert no action for both probe-bearing and opt-out branches. The no-marker View is checked in `service_lifecycle_stable.rs`. A failed write must re-announce through S-43's two activated composed scenarios; no emitted marker can suppress it. |
| View-only `has_alloc_mid_startup_window` tests | Removed three tests of the explicitly deleted method whose outcomes are already covered by the active deadline, committed Stable, and terminal-state grid. Rewrote the empty-view and mixed-population cases to drive `next_evaluation_at` directly, preserving the no-work and any-active-allocation outcomes. Other core `reconcile` + `next_evaluation_at` cases check the startup stop, pre-Running failure, full terminal-state grid, and Running control. `service_lifecycle_runtime_reenqueue` retains the composed deferred-key/drain checks; the source-local earliest-window property and S-43 cover scheduling. |
| A prior `stable_announced` marker allows liveness after the same-tick guard | Represent the prior tick by `stable_committed = true`; retain the exact liveness stop and no second `Stable` assertions. The same-tick test still requires only `Stable` and zero liveness stops. |
| Startup stop's old finalizer, backend veto before finalizer, or a pre-Stable healthy backend | Assert `StopAllocation { Stopped { StartupProbe } }` followed by the backend projection, with `healthy: false` both at a due stop and in the still-open startup window. Keep the unchanged-row and readiness flap complements; the flap starts with `stable_committed = true`, the accepted precondition for eligibility. The source-local pure property now drives `ServiceLifecycleReconciler::reconcile` across opt-out, committed claim, own/unrelated terminal membership, startup result, readiness result and threshold; it asserts the backend health rule and unchanged unrelated view inputs without assuming a private helper signature. |
| Old CBOR payload mixes D8's missing retry field with R2-2's removed key | The D8 fixture carries the current fields except `startup_last_acted_run`. A separate serializable *legacy payload* carries `stable_announced` for the R2-2 unknown-key check. The older `probe_run_count` legacy payload retains its historical key but its decoded current-View expectation drops it. No current `ServiceLifecycleView` construction names the deleted field. |

`infra/metal/test-placement.json` removes the three deleted method-test entries and updates the three renamed test keys, preserving each existing `host-pure` placement. No production behavior, public API, test seam, DES event, or step activation was changed in this remediation. The only source-file edit is inside the existing `#[cfg(test)]` block.

## Verification against the current partial production implementation

| Metal command or check | Result |
| --- | --- |
| `cargo xtask metal run -- cargo nextest run -p overdrive-reconcilers --test acceptance -E 'test(blob_carrying_the_removed_stable_announced_key_is_ignored)'` before edits | Failed for the intended R2-2 mismatch: decoded old marker was retained. |
| Reconcilers acceptance, selected `startup_last_acted_run_evolution`, `probe_run_count`, `service_kind_vm_workloads` | Compiled; 6/11 passed, 5 expected RED. Failures are the deleted-key decoder (two legacy cases and D8's field-free payload), committed-Stable readiness, and pre-Stable backend health. |
| Reconcilers source-local `backend_policy_uses_committed_stable_and_readiness_without_a_veto` | Compiled; expected D12 RED, minimal input `(same_status_runs=1, threshold=1)` produced old `healthy=true` against required `false`. |
| Core acceptance: committed dedup, later-tick liveness, same-tick suppression, active deadline | Compiled; 2/4 passed. The two expected B1 RED failures re-emitted `Stable` despite `stable_committed=true`. |
| Core acceptance: empty view and mixed population deadline | Compiled; empty view passed, mixed population expected B1 RED: the current hook chose the committed allocation's 61 s deadline instead of the still-waiting allocation's 62 s deadline. |
| Control-plane acceptance: four affected modules and two opt-out cases | Compiled; 6/8 passed. The two expected B1 RED failures re-emitted `Stable` despite `stable_committed=true`; composed runtime row and other controls passed. |
| Metal strict Clippy | `overdrive-reconcilers --tests`, `overdrive-control-plane --test acceptance --features integration-tests`, and `overdrive-core --test acceptance`: all passed with `-D warnings`. |
| Changed-file rustfmt, JSON parse, `git diff --check` | Passed. |

The field deletion and B1/D12 production behavior remain the original step 02-04 crafter's GREEN work. Independent DISTILL review is required before that crafter resumes. No commit was made here.

## Iteration 2: liveness threshold fixture after the Stable commit

The step 02-04 crafter's focused 91-test GREEN run reached 90 passes and one failure in `vm_liveness_threshold_emits_only_the_existing_liveness_stop`. I reproduced that single failure on Metal before editing: the first below-threshold liveness row produced an action, so `first_fail.is_empty()` failed. The test had asserted the initial `FinalizeFailed { Stable }` action, then cloned its pre-dispatch fact with `stable_committed=false` for every later liveness evaluation. With B1 implemented, `ServiceLifecycle::reconcile` correctly re-announces `Stable` on each of those evaluations because the fixture still says the row never committed. Under the approved D9 ordering, liveness rows arrive only after the Stable row write and its hook.

The fixture now sets `observed.allocs[alloc_id].stable_committed = true` immediately after the initial Stable action and before the first liveness probe row. It represents the row commit that the test's between-evaluation setup had omitted. The initial Stable emission, empty below-threshold actions, Pass reset, exact threshold `StopAllocation { Stopped { LivenessProbe } }`, acted-run memory, Running state and no-restart assertions are unchanged. No production path, D9 implementation, test seam, or DES event changed.

| Verification | Result |
| --- | --- |
| Before: Metal `cargo nextest run -p overdrive-reconcilers --test acceptance -E 'test(vm_liveness_threshold_emits_only_the_existing_liveness_stop)'` | Reproduced the first-fail assertion, exit 100. |
| After: Metal named liveness threshold, readiness flap, startup Stable, and WorkloadLifecycle liveness-restart controls | 4/4 passed, exit 0. |
| Declaration and hygiene | Existing `/// CONTRACT_SHAPE: bounded-change.` retained; changed-file rustfmt and `git diff --check` passed. |

After-fix Metal command:

```sh
cargo xtask metal run -- cargo nextest run -p overdrive-reconcilers --test acceptance --no-fail-fast -E 'test(vm_liveness_threshold_emits_only_the_existing_liveness_stop) | test(vm_readiness_flaps_only_backend_eligibility_and_recovers) | test(vm_startup_pass_changes_only_service_stable) | test(workload_lifecycle_alone_decides_restart_after_liveness_stop)'
```

This iteration changed only `crates/overdrive-reconcilers/tests/acceptance/service_kind_vm_workloads.rs` and this remediation record. Independent DISTILL re-review remains the handoff gate before the original step 02-04 crafter reruns GREEN and COMMIT.

## Iteration 3: post-Stable readiness inputs

The original crafter's broader three-crate run exposed five concrete D12 fixture failures among other failures whose baseline or host cause has not been established (`.context/step-02-04-green-three-crates-collect.log`, Metal run `3f6df47884e7672cabdccd5c`). I reproduced only these five named failures independently on the current production implementation: all four `service_lifecycle_readiness` acceptance cases failed because their post-Stable fixtures projected `healthy=false` with `stable_committed=false`; the source-local `readiness_observation_drives_backend_healthy_false_then_true` failed at its recovered `healthy=true` assertion for the same reason. The accepted D12 predicate requires a startup-probed allocation's **own committed Stable row** before readiness can admit it.

| Fixture | Exact correction and retained oracle |
| --- | --- |
| `fact_with_readiness` and `fact_without_readiness` in `service_lifecycle_readiness.rs` | Their existing comments and callers describe post-Stable allocations. Both now set `stable_committed=true`. The generated Pass/Fail statuses, producer streaks, thresholds, backend-count universe, exact `healthy` outcomes, one-tick recovery and no-restart assertions are unchanged. The no-first-readiness-Pass and flapping controls still run. Pre-Stable ineligibility remains covered by S-46/S-47 and the source-local D12 property. |
| Source-local runtime hydration/reconciliation test in `reconciler_runtime.rs` | The test first hydrates the real Running row and verifies the readiness `Fail` observation reached the fact. It then takes the production reconciler's `FinalizeFailed { Stable }` action for that allocation, writes that typed terminal to the existing Running `AllocStatusRow` through `ObservationStore::write_alloc_lifecycle` with a newer logical timestamp, and rehydrates through the production path. It asserts `stable_committed=true` before checking readiness `Fail → Pass` yields `healthy=false → true`. This represents the missing between-evaluation row commit while preserving the original per-role probe-index and backend oracles. No fabricated fact field or public test seam was added. |

The source-local transitioned test now declares `/// CONTRACT_SHAPE: bounded-change.`; the four acceptance tests retain their declarations. Only test fixtures inside `reconciler_runtime.rs`'s `#[cfg(test)]` block and `service_lifecycle_readiness.rs` changed. No production behavior, DES event, activation, or commit changed.

| Verification | Result |
| --- | --- |
| Before: Metal four named `service_lifecycle_readiness` cases | 0/4 passed; each failed on a post-Stable `healthy=true` expectation. |
| Before: Metal source-local `readiness_observation_drives_backend_healthy_false_then_true` | Failed with `Some(false)` versus `Some(true)` after readiness recovered. |
| After: Metal four named acceptance cases plus `backend_healthy_false_before_first_readiness_pass` and `readiness_flapping_never_restarts` controls | 6/6 passed. |
| After: Metal source-local target plus `startup_and_readiness_observations_at_index_zero_do_not_shadow_each_other` control | 2/2 passed. |
| Metal `cargo clippy -p overdrive-control-plane --tests --features integration-tests -- -D warnings` | Passed. |
| Changed-file rustfmt and `git diff --check` | Passed. |

The wider 2,288-test sweep was not rerun for this bounded fixture correction. Independent DISTILL re-review remains the next gate before the original step 02-04 crafter resumes.

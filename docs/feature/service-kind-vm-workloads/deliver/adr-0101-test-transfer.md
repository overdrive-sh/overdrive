# ADR-0101 test-owner transfer

Date: 2026-09-08
Author: acceptance designer (Codex)
Scope: approved revision-3 D5 retirement and step 02-03 test/compiler fallout.
Baseline: `cbcf9a6d205b187735c5a080be9711ed32a2116d`.

## Authority and preserved evidence

The executing crafter owns production changes, DES events and the atomic step
commit. The acceptance designer owns these test changes. The orchestrator
released the edit gate after confirming the crafter's fresh RED event at
`2026-09-08T10:43:20Z`. No acceptance-designer edit preceded that release.

The approved BE-01–BE-12 composed tests and their assertions are unchanged.
BE-P1 changes only its `ServiceAllocFact` address field to `backend_ip`; its
fixed seed 257222, 128 cases, policy truth table and full View complement remain.
The original diagnostic's seed 257209 and final terminal-veto invariant remain
unchanged. Its retired bridge registration is removed, and its three membership
ticks now drive ServiceLifecycle. The startup loop stops on the real Failed
observation so the original deciding-row assertion executes before the next
authoritative membership tick publishes the now-empty row. No terminal,
backend, probe-result or View row is fabricated by this diagnostic.

The checkpoint retains the original pre-change diagnostic. Earlier observed
RED evidence remains in `distill/red-classification.md`; this transfer does
not rewrite that evidence or claim a compiler failure as behavioral RED.

## Coverage disposition

| Retired or transferred surface | Retained evidence |
|---|---|
| Bridge-only type/variant/old-View CBOR tests | Delete with the retired type. No replacement compatibility test. Current AnyReconciler dispatch and hydrate forwarding still cover the remaining seven owners. |
| Three bridge Sim evaluators, wrappers, enum variants, harness arms and catalogue entries | BE-02/03/05 retain complete membership, listeners and withdrawal; BE-01/02 retain equal-observed-content idempotence; BE-04/06 retain actual write-failure repair through production owners. |
| Bridge-to-hydrator Sim evaluator and direct pure bridge enqueue test | BE-07 pins ordered write/handoff actions and exact identities/stamps/correlations. BE-10 exercises actual asynchronous hydrator dispatch and local-map withdrawal/recovery. Standalone hydrator evaluators and retry properties remain intact. |
| Bridge canonical-address tests | Retained ServiceLifecycle hydration control checks canonical workload IP and host-IP fallback. Capture/advertise port-set property now observes ServiceLifecycle's emitted backend addresses for the same generated multi-listener intent. |
| Bridge listener hydration controls | Transfer existing Service, Job, Schedule and absent-allocator-memo cases to ServiceLifecycle `hydrate_actual` and its exact `service_dataplane` map. Existing digest and probe-filter controls remain. |
| Bridge View write-through equality test and bridge hydration golden | Delete only the retired variant's cases. WorkloadLifecycle, ServiceMapHydrator, ServiceLifecycle and SVID runtime persistence controls remain. Current hydration and workload trajectory goldens change only the approved removed owner/field shape. |
| WorkloadLifecycle bridge wake assertions | Transfer Service Start/Stop/GC Stop/FinalizeFailed wake assertions to the sole ServiceLifecycle owner. Preserve SVID wakes, Job negative control, converged no-op and VIP-only-release no-op. BE-11/12 retain real dispatch, same-ID restart and budget/finalization coverage. |
| Reclamation and AllocStatus interest fan-out | Remove only the bridge from the expected names; retain WorkloadLifecycle, ServiceLifecycle and SVID. Existing accepted-write, List/Watch, lag/relist, coalescing, fixpoint and determinism controls remain. |
| Hydrator constructor/registry control and real-kernel walking skeleton | Retain registry/type and kernel/operator assertions. The registry control does not claim actual server boot coverage. Rename the handoff's owner to ServiceLifecycle without turning Rust tests into binary expectations. Historical directory names are not a second publisher. |

Existing pure reconcile controls now feed their actual emitted backend rows
back as observed content when asserting idempotence. They no longer treat a
persisted View alone as write acknowledgement. Failed/Pending exclusion controls
assert an explicit empty current-listener row, not absence of publication.
All newly transitioned tests carry per-test Contract Shape declarations.

## Execution and limitations

Acceptance-designer formatting and `git diff --check` passed after transfer.
Focused Lima compilation and behavioral results are recorded below. The shared-source test-only patch was supplied to the crafter as
`.context/adr-0101-test-transfer.patch` and its application is visible in the
three intended source test modules.

No production API/behavior, new scenario family, compatibility/migration
mechanism, expectation runner, native suite, full-100 run, mutation run, DES
event or commit was authored by the acceptance designer. Publication completion
still does not mean all consumers have acknowledged withdrawal, and existing
connections remain outside the contract.

## Observed focused results

All commands below ran through Lima, in short-yielded sessions. Failed nextest
behavioral runs returned inner exit 100 / outer xtask exit 1. No failed run is
classified GREEN.

| Command | Run ID | Result |
|---|---|---|
| 1: approved BE composition | `21245211-7507-4f57-b5c1-48d450097fc5` | 12 run, 10 passed, 2 failed, 24 skipped. |
| 2: original seed 257209 witness | `44f9ac80-e3e3-43b0-b53f-9903c5bf0f5e` | 1 passed, 0 skipped; exit 0. Same-ID terminal veto remains; final backend is unhealthy. |
| 3: transferred and preserved controls | `dabad9de-a684-4105-b64b-1b52cdad88bc` | 134 run, 129 passed, 5 failed, 1163 skipped. |
| 4: compact first-failure confirmation | `d93388cc-a240-4249-8a5d-ba63d8cc9969` | The same five controls failed; 552 skipped. |

Command 3 also ran as `7a8b44f3-ff48-4fea-b521-ccda89d46946`; its enormous
output was truncated at the tool boundary. The final repetition above retained
and filtered the output to verify the exact summary and passing paired property.
BE-P1 (`backend_policy_preserves_veto_threshold_and_unrelated_view_inputs`)
passed, as did all seven transferred source-local hydration controls.

Exact commands:

1. `cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test integration -E 'test(service_backend_projection)' --no-capture --no-fail-fast`

2. `cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test e09_v2_failed_service_reachability_spike --no-capture`

3. `cargo xtask lima run -- cargo nextest run -p overdrive-core -p overdrive-reconcilers -p overdrive-control-plane --features integration-tests --lib --test acceptance -E 'test(service_lifecycle) | test(service_kind_vm_workloads) | test(interest_router) | test(service_backend_hydrate) | test(workload_lifecycle_enqueues_bridge_on_alloc_transitions) | test(listener_fact_hydrate_equivalence) | test(hydration_characterization_golden) | test(service_map_hydrator_registered_at_boot) | test(backend_policy_preserves) | test(submits_three_evaluations)' --no-fail-fast --no-capture --status-level all --final-status-level all`

4. `cargo xtask lima run -- cargo nextest run -p overdrive-core --features integration-tests --test acceptance -E 'test(reconcile_terminal_failed_clears_mid_window_and_dedups) | test(startup_attempt_counter_increments_by_one_per_observed_fail) | test(startup_probe_failed_fires_when_all_three_gates_met) | test(startup_probe_failed_reachable_at_exactly_max_and_prevented_by_pass) | test(job_kind_start_allocation_emits_no_service_enqueue)' --no-fail-fast --status-level fail --final-status-level fail`

Two earlier setup failures are not behavioral RED: the first Sim invocation
omitted `overdrive-control-plane/integration-tests` and could not see six
existing feature-gated test helpers; a control compilation found a
borrow-after-move in the existing startup fact helper. The latter was repaired
by placing the observation-time field initializer before moving the same probe
value. No production seam was added. An intermediate control execution returned
failure without useful captured test output; it is not independent pass evidence.

### BE02: current workload-owner boundary

Seed 257210's single-listener control passes. Seed 257211 reaches the first
two-member assertion at `service_backend_projection.rs:440` with one member,
not two. The two-replica input goes through real WorkloadLifecycle execution.
Current `workload_lifecycle.rs:690` finds any Running allocation and returns
without further placement at line 703; its placement code explicitly assumes
at most one Running allocation at line 982. This is a fixture/owner-path
limitation, not evidence that the new projection lost a second existing member.
The subsequent multi-member exact membership/order/idempotence assertions
have not executed. The approved oracle is unchanged. No second allocation row,
replica scheduler, new API or simulated placement mechanism was invented.

Current-behavior confirmation requested by the user: Service intent hydration
does preserve `svc.replicas` in the projected Job at
`workload_lifecycle.rs:348`–358. The reconcile path does not use that count:
fresh placement calls `scheduler::schedule(nodes, resources, rows)` once at
line 986, creates one StartAllocation at line 1048, and returns one action at
line 1103. The scheduler returns one NodeId and accepts no replica count
(`overdrive-core/src/scheduler.rs:79`–83). Runtime dispatch awaits that action
(`reconciler_runtime.rs:1566`), and StartAllocation awaits one driver start
(`action_shim/mod.rs:1932`) before publishing Running at line 1991. On the next
normal evaluation the any-Running guard at `workload_lifecycle.rs:690`–703
stops further placement. Therefore this is **not merely one start per
evaluation**: successful normal convergence stops at one concurrently Running
allocation even when the declared replica count is two.

This does **not** mean only one allocation record can exist. Fresh IDs count
pre-existing rows at lines 1009–1023; historical stopped/failed/terminated
records can coexist with the current instance. Explicit replacement ends the
Running instance first (lines 694–700), waits while it is Draining (line 719),
and then places the next instance. Nor is the any-Running guard a demonstrated
global invariant forbidding every possible pair of Pending/Draining/Running
records: its local `active_allocs_vec` name only means intentionally-stopped
rows were filtered (line 668), not that every element is concurrently live.

The owner path and seeded BE02 result confirm a **current single-Running
normal-convergence limitation**. They do not establish whether that limitation
is the intended supported product contract or a regression against historical
replica-aware scheduling claims. The orchestrator found contradictory issue
history (#140 and #21 closed with broader claims; #254 open for multi-replica
replacement). Those issue dispositions alone cannot resolve current support
intent. No scheduler-capacity defect is demonstrated: capacity selection is
never asked for another instance once a Running instance exists. BE02 remains
unchanged pending the user's contract decision.

### BE10: map retention versus backend selection

Seed 257221 reaches `service_backend_projection.rs:1018` after the healthy
local-map baseline, real ProbeRunner readiness failure, authoritative unhealthy
row, successful mesh/DNS withdrawal, and the actual queued hydrator dispatch.
The Sim local-map query returns `Some(192.0.2.10:18081)`, not `None`.
The recovery suffix at lines 1020–1037 remains unexecuted in BE10; independent
BE09 executes and passes mesh/DNS withdrawal and recovery.

The concrete production path is:

1. `ServiceMapHydrator::reconcile` at
   `overdrive-reconcilers/src/service_map_hydrator.rs:353` excludes mesh-subnet
   addresses, then partitions local addresses at line 362 without filtering
   `Backend.healthy`. The changed local fingerprint re-emits registration at
   line 456. `push_register_local_backend_actions` at line 599 emits
   `RegisterLocalBackend` for the unhealthy local backend; it neither filters
   health nor emits deregistration.
2. `overdrive-control-plane/src/action_shim/register_local_backend.rs:66`
   awaits the existing Dataplane registration port. The production
   `EbpfDataplane` implementation at `overdrive-dataplane/src/lib.rs:2158`
   upserts the forward local-backend map at line 2176. The map entry has an
   address/port, not a health field.
3. `overdrive-bpf/src/programs/cgroup_connect4_service.rs:44` is the actual
   new-connect consumer: it looks up the destination VIP/port/protocol at
   line 78 and, on a hit, rewrites the destination to the recorded backend at
   lines 88–104. It does not consult the authoritative row or a healthy bit.
   Thus a retained entry is a usable selection, not merely an unused cache.
4. Production attaches this hook to the configured ancestor cgroup in
   `overdrive-dataplane/src/lib.rs:679`–716 (default path at line 119).
   The existing kernel walking skeleton exercises direct assigned-VIP
   connections under that hook. The BE10 failure requires no cancellation,
   process shutdown, forced abort or retry race: the allocation remains
   Running while readiness fails. Normal hook teardown does not withdraw an
   individual backend during this live process.

Classification: the control-plane-to-Sim-adapter unhealthy local-map retention
is **reproduced through production owners**. Source tracing establishes that
such a retained entry can select a backend for a new direct-VIP connection in
the attached cgroup, despite the unhealthy row. Actual native kernel/wire
routing after readiness failure was **not reproduced in this work**; no native
suite was authorized. Mesh-subnet backends are excluded from this local path,
so this does not establish that a local-map remedy is necessary for the original
VM E09 defect.

Relative to ADR D6's retention of existing consumer behavior and the bounded
authoritative-publication remedy, BE10's map-removal oracle exposes an unresolved
scope/contract mismatch. It is not permission to add a consumer mechanism.
Neither a claim that the map is harmless nor a claim that end-to-end native
unhealthy routing was reproduced is justified. The user must decide the
disposition of this applicable-consumer requirement; the approved assertion
remains unchanged pending that decision.

### Additional preserved controls

The five failures in command 3 are isolated from the two BE failures:

| Existing control | First failure and disposition |
|---|---|
| `job_kind_start_allocation_emits_no_service_enqueue` | At `workload_lifecycle_enqueues_bridge_on_alloc_transitions.rs:560`, expected StartAllocation + SVID enqueue, observed only StartAllocation. The old test's SVID assertion remains unchanged; only bridge removal changed the expected count from 3 to 2. The current production diff places the pre-existing SVID wake inside the new Service-kind outer guard at `workload_lifecycle.rs:228`, although the adjacent SVID contract explicitly remains kind-independent. This is a concrete reconcile-port regression, not permission to weaken the test. No full Job dispatch/seeded liveness failure is claimed. |
| `startup_attempt_counter_increments_by_one_per_observed_fail` | At `service_lifecycle_reconcile_branches.rs:1038`, expected counter 2, observed 1. The fixture reuses the identical observation and timestamp on its second call. |
| `startup_probe_failed_fires_when_all_three_gates_met` | At line 572, expected one terminal action, observed none. Fixture seeds a prior counter but no prior observation identity. |
| `reconcile_terminal_failed_clears_mid_window_and_dedups` | At line 906, expected one terminal action, observed none; its dedup suffix does not execute. Fixture likewise supplies the prior counter without observation identity. |
| `startup_probe_failed_reachable_at_exactly_max_and_prevented_by_pass` | At line 1115, expected the third failure's terminal action, observed none. Repeated calls reuse the same observation; the following Pass-prevention suffix does not execute. |

These four startup fixtures meet unchanged production logic at
`service_lifecycle.rs:1240`: a counter lacking its observation identity is
reset to one, and the identical observed failure is not counted again.
That logic predates this production diff. Their semantic expectations were not
changed during the compiler/owner transfer, and these failures do not prove a
new ADR-0101 startup regression. No fabricated observations or speculative
production fix were introduced to conceal them.

## Exact acceptance-designer file ownership

Transferred/updated files (all paths relative to the repository root):

- `crates/overdrive-control-plane/tests/acceptance.rs`
- `crates/overdrive-control-plane/tests/acceptance/fixtures/hydration_golden/service_lifecycle.txt`
- `crates/overdrive-control-plane/tests/acceptance/hydration_characterization_golden.rs`
- `crates/overdrive-control-plane/tests/acceptance/interest_router.rs`
- `crates/overdrive-control-plane/tests/acceptance/listener_fact_hydrate_equivalence.rs`
- `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_hydrate.rs`
- `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_liveness.rs`
- `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_probe_to_stable.rs`
- `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_readiness.rs`
- `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_stable.rs`
- `crates/overdrive-control-plane/tests/acceptance/service_map_hydrator_registered_at_boot.rs`
- `crates/overdrive-control-plane/tests/acceptance/service_submit_event_taxonomy.rs`
- `crates/overdrive-control-plane/tests/acceptance/single_restart_authority_liveness_trajectory.rs`
- `crates/overdrive-control-plane/tests/integration/backend_discovery_bridge/walking_skeleton.rs`
- `crates/overdrive-control-plane/tests/integration/reconciler_runtime_view_store.rs`
- `crates/overdrive-core/tests/acceptance.rs`
- `crates/overdrive-core/tests/acceptance/any_reconciler_dispatch.rs`
- `crates/overdrive-core/tests/acceptance/service_lifecycle_reconcile_branches.rs`
- `crates/overdrive-core/tests/acceptance/vm_reclamation_plan_purity.rs`
- `crates/overdrive-core/tests/acceptance/workload_lifecycle_enqueues_bridge_on_alloc_transitions.rs`
- `crates/overdrive-core/tests/capture_advertise_port_set_equality.rs`
- `crates/overdrive-reconcilers/tests/acceptance/crate_extraction_import_rewrite_compiles.rs`
- `crates/overdrive-reconcilers/tests/acceptance/service_kind_vm_workloads.rs`
- `crates/overdrive-sim/src/harness.rs`
- `crates/overdrive-sim/src/invariants/mod.rs`
- `crates/overdrive-sim/src/invariants/service_map_hydrator.rs`
- `crates/overdrive-sim/tests/acceptance/fixtures/hydration_trajectory/workload_lifecycle_trajectory.txt`
- `crates/overdrive-sim/tests/acceptance/hydration_move_equivalence.rs`
- `crates/overdrive-sim/tests/e09_v2_failed_service_reachability_spike.rs`
- `crates/overdrive-sim/tests/integration/dst_clean_clone_green.rs`
- `crates/overdrive-sim/tests/integration/dst_harness_smoke.rs`
- `crates/overdrive-sim/tests/invariant_roundtrip.rs`

Retired files (recoverable from the checkpoint commit):

- `crates/overdrive-core/tests/backend_discovery_bridge_types.rs`
- `crates/overdrive-core/tests/canonical_address_bridge_advertise.rs`
- `crates/overdrive-control-plane/tests/acceptance/bridge_emits_enqueue_evaluation_for_hydrator.rs`
- `crates/overdrive-sim/src/invariants/backend_discovery_bridge.rs`
- `crates/overdrive-control-plane/tests/acceptance/fixtures/hydration_golden/backend_discovery_bridge.txt`

The crafter applied the supplied test-only patch in these shared files:
`overdrive-control-plane/src/reconciler_runtime.rs` (the
`service_backend_hydrate` test module),
`overdrive-control-plane/src/action_shim/reclamation.rs` (existing fan-out
test), and `overdrive-reconcilers/src/service_lifecycle.rs` (BE-P1 field
rename only). Their other edits belong to the crafter. This handoff document
is the only coverage/disposition document authored in this transfer.

## Completion boundary

Test transfer and bounded failure classification are complete; step GREEN is
not achieved. The nWave test-design/completeness discipline kept the original
safety witness, healthy controls, negative complements and paired property
separate from unexecuted suffixes and fixture limitations. Existing DISTILL
scenario scope and audit remain unchanged; no new scenario family was added.

The remaining standalone capture/advertise property, Sim catalogue/roundtrip
and trajectory wrappers, persistent-view integration cases and native walking
skeleton were updated but not executed in this handoff. Their success must not
be inferred from the focused six-binary compilation. No whole-workspace,
native E09, full-100 or mutation result is claimed.

All production edits and DES state remain owned by the crafter. The
orchestrator reported RED PASS / GREEN FAIL / COMMIT SKIPPED; no commit was
made. Worktree changes are preserved for the user's decision. Both approved
BE oracles remain intact, and no replica scheduling, local-map consumer change
or new test seam was implemented.

## Later user-approved BE02 correction — 2026-09-08

This section supersedes only the earlier BE02 pending-disposition statements;
all historical failed runs and their first-failure boundaries remain evidence.
The user explicitly authorized: “Yes, apply the bounded BE02 correction.”

`complete_listener_projection_is_idempotent` retains both seeds and cases.
Seed 257210 remains one allocation and 18081/TCP. Seed 257211 retains
18082/UDP, 18081/TCP and 18081/UDP, with its requested allocation count changed
from two to one. The allocation is still scheduled through WorkloadLifecycle,
not synthesized. Every original assertion remains: exact listener universe,
allocator-issued VIP, backend identity, listener-specific address/port, health,
weight, and full-row equality after three further reconciliations, including
the original stamp. The per-test Contract Shape remains.

The prior two-replica fixture asked for unsupported current normal convergence,
not evidence of a projection defect. Multi-allocation membership and meaningful
allocation-ID ordering are explicitly deferred to
[#282](https://github.com/overdrive-sh/overdrive/issues/282) and its
[BE02 follow-up](https://github.com/overdrive-sh/overdrive/issues/282#issuecomment-5584665927).
The singleton identity check does not prove them.

Focused verification:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test integration -E 'test(complete_listener_projection_is_idempotent)' --no-capture
```

Nextest run `c46116bd-0a5d-402d-ae26-e2941f6e60ec`: **1 passed, 35 skipped**,
exit 0. Both seeds 257210 and 257211 printed, and both cases completed all
retained assertion suffixes. Formatting and `git diff --check` passed.
No fresh pre-edit RED was needed; the preceding unchanged seed257211 failure
is preserved above. This is not independent test-review approval.

Exact files changed by this bounded correction:

- `crates/overdrive-sim/tests/integration/service_backend_projection.rs`
- `docs/feature/service-kind-vm-workloads/distill/adr-0101-acceptance.md`
- `docs/feature/service-kind-vm-workloads/distill/red-classification.md`
- `docs/feature/service-kind-vm-workloads/deliver/adr-0101-test-transfer.md`

Only BE02 and its directly matching documentation changed. BE10, the other
five preserved controls, production code, feature-delta/DES/design/review
artifacts and other owners' dirty edits were untouched. No commit, mutation
or native run occurred; the broader step's GREEN status is not reassessed.

## Later bounded startup-control fixture alignment — 2026-09-08

The four previously failing startup controls are now aligned with accepted
ADR-0097 D2, not with new production behavior. D2 explicitly counts only
distinct latest Startup/index-0 observation timestamps, clears both maps on
Pass, and treats a counter without its last-counted timestamp as an unpaired
input whose next failure resets the count to one. Current
`ServiceAllocFact.latest_startup_probe_observed_at: Option<UnixInstant>` and
`ServiceLifecycleView.startup_last_fail_seen_at` supply exactly these existing
inputs. `update_startup_attempts` in
`overdrive-reconcilers/src/service_lifecycle.rs:1230` implements that contract;
this production helper is unchanged from checkpoint `cbcf9a6d`.

The earlier `dabad9de-a684-4105-b64b-1b52cdad88bc` and
`d93388cc-a240-4249-8a5d-ba63d8cc9969` failures remain historical evidence.
They do not prove an ADR-0101 production regression or an earlier GREEN
baseline: the old fixtures either replayed one observed failure while claiming
distinct failures or seeded an unpaired counter while expecting an increment.

Only these four tests changed in
`crates/overdrive-core/tests/acceptance/service_lifecycle_reconcile_branches.rs`:

| Test | Exact fixture alignment and retained evidence |
|---|---|
| `startup_attempt_counter_increments_by_one_per_observed_fail` | First failure identity 1500ms, distinct second identity 2000ms, both at/before the 2000ms reconcile time and after the 1000ms start. Original exact counter 1 then 2 assertions remain. An intervening repeat asserts empty actions and complete unchanged View, explicitly pinning once-per-observation accounting. |
| `startup_probe_failed_fires_when_all_three_gates_met` | Prior count 29 now includes last-counted identity 59000ms; current failure identity is 60000ms, before the unchanged 61000ms deadline tick. Exact terminal variant, allocation, probe index, reason and attempts 30 assertions remain. |
| `reconcile_terminal_failed_clears_mid_window_and_dedups` | Prior count 4 is paired with 59000ms, followed by failure 60000ms. Original terminal, closed startup-window and repeated-terminal suppression assertions all execute. |
| `startup_probe_failed_reachable_at_exactly_max_and_prevented_by_pass` | Three distinct failure identities 12000/13000/14000ms, before the unchanged 20000ms reconcile time, reach the same threshold 3. The independent Fail(12000ms) → Pass(13000ms) branch retains Stable-not-failure and counter-reset assertions, plus checks removal of the last-counted identity. Its comment now accurately stops at Stable; it does not claim a later startup failure was supplied. |

These are existing pure reconcile input fixtures, not production-store or Sim
observation injection. No new public/test seam or scenario family was added.
All original assertions and per-test `CONTRACT_SHAPE: bounded-change.` lines
remain; the shared fact helper and other tests were not changed by this
follow-up. The acceptance-test discipline keeps a pure fixture's paired inputs
distinct from composed production reachability evidence.

Exact focused command:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-core --features integration-tests --test acceptance -E 'test(startup_attempt_counter_increments_by_one_per_observed_fail) | test(startup_probe_failed_fires_when_all_three_gates_met) | test(reconcile_terminal_failed_clears_mid_window_and_dedups) | test(startup_probe_failed_reachable_at_exactly_max_and_prevented_by_pass)' --no-capture --no-fail-fast
```

Initial aligned run `266dae10-e9b5-44e4-9500-a3241d7dfaeb`: **4 passed,
553 skipped**, exit 0. Final run after fixture diagnostics/comment cleanup
`88f48258-336b-49e1-8b1b-56b4113365d9`: **4 passed, 553 skipped**, exit 0.
All previously blocked threshold/terminal-dedup/Pass-prevention suffixes now
execute. Formatting and `git diff --check` pass. No whole-suite GREEN claim.

Exact follow-up file list: the Rust file above and this handoff document only.
BE02, BE10, other controls, production, design/roadmap/review/DES artifacts and
other owners' dirty work are untouched. No commit, mutation or native run.
No remaining blocker for this bounded fixture alignment; independent step
review and any broader implementation continuation remain with the orchestrator.

## Later BE10 revision-4 DISTILL handoff — 2026-09-08

The independent DESIGN review approved the bounded local register/deregister
selection. The earlier BE10 RED and native/recovery limits remain historical
evidence. The [new scoped test handoff](../distill/be10-local-withdrawal-test-handoff.md)
records fresh seed257221 RED, one necessary paired pure-property complement,
retained green controls and the exact unapplied source-local test-only patch.
BE10's composed assertion is unchanged; no production implementation or GREEN
result is claimed by that handoff. BE02 and startup alignments above remain.

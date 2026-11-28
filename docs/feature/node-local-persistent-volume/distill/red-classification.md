# RED classification: node-local persistent volume (GH #348)

A pending test is a complete body under `#[should_panic(expected = "...")]`, and its `expected` substring names the assertion the test
exists to make, or the precondition the production scaffolds deny its body (`.claude/rules/testing.md` § "RED acceptance scaffolds
and activation"). A body that panics for an unrelated reason (a fixture, a setup step, an upstream stub) does not count as RED, so no
test expects a scaffold's own phrase: the pure-function files catch the scaffold's unwind and re-panic with their own contract
assertion. This file is the fail-for-the-right-reason gate for the whole feature: the volume (ADR-0310 to ADR-0320), launch
supersession (ADR-0334), the restart generation (ADR-0335, ADR-0337, ADR-0338) and the capacity wait (ADR-0336). The exact `expected`
strings are in the source; this file states where each file's tests stop today and which activation row moves them.

**Counts are in `test-scenarios.md` § K and nowhere else.** Classification: every running pending test is MISSING_FUNCTIONALITY, every
ignored one is IGNORED-RESOURCE (the `volume-pool` guest's pool is absent), the rest are guards. Every pending test carries a marker: no
`#[ignore]` without `#[should_panic]` remains.

**Two kinds of stop.** A test *stops at the headline* when its `expected` is the assertion that names the property under test. It
*stops at a precondition* when an earlier assertion or setup step, which a missing capability denies the body, fails first; the headline is
first exercised when that capability lands, and the marker then moves to the next RED (or the test goes GREEN). The per-file blocks name
the precondition stops; the activation table at the end gives, per row, which markers move.

## Scaffold inventory

Production scaffolds are permitted only where the accepted design pins the exact shape; each row names the pinned shape, what it does
today, and the DELIVER capability that replaces it. No scaffold adds API the design did not pin.

| Scaffold | Pinned shape (source) | Behaviour today | Replaced by (activation row) |
|---|---|---|---|
| `LaunchSupersession` port, `Driver::start(&self, spec, supersession: &dyn LaunchSupersession)`, `DriverError::StartSuperseded` | `overdrive-core/src/traits/driver.rs` (feature delta § Superseding a launch) | real surface; no driver returns `StartSuperseded`, `VmDriver` names the parameter `_supersession`, the shim has no `StartSuperseded` arm | 8: observation points 1 to 6 in `VmDriver::start`; the shim's no-row arms |
| `ReconcilerRuntime::supersede_lane`, `supersede_all_lanes`, `lane_supersession` | `reconciler_runtime.rs` | `supersede_lane` and `supersede_all_lanes` are no-ops; `lane_supersession` returns a never-raised signal | 8: the lane table (a lane opened in the tick before hydration, a child of the shutdown token, removed by key) |
| `WorkloadLifecycleView.issued_generation` (replaces `observed_generation`), `ReconcilerRuntime::issued_generation_for_alloc` | `workload_lifecycle.rs` | real field and read, written at both issuing sites; the reconciler decides with the private compat rule `restart_pending_until_successor_observed`, restated inline in `restart_status_for_alloc` | 9: `generation_replacement_pending` adopted at the four sites; the compat rule and its inline copy deleted |
| `generation_replacement_pending(view, desired_generation, ran)` | `workload_lifecycle.rs`, ADR-0335 | panics `RED scaffold`; not used by the reconciler | 9 |
| `start_blocked_only_by_capacity(kind, run_desired, rows, view, desired_generation, observation)` | `workload_lifecycle.rs`, ADR-0336 | panics `RED scaffold`; not used by the reconciler | 10: the shared predicate in the reconciler's rules and the handler |
| `GuestAttachmentOccupancy::at_cap` | `overdrive-core/src/guest_network.rs` | real; scheduler, reconciler and pool admission use it | unchanged |
| `api::CapacityWait`, `CapacityResource`, `AllocStatusResponse.capacity_wait` | `api.rs`; `api/openapi.yaml` regenerated (only these types) | real wire types; the handler writes `capacity_wait: None` | 10: the handler derivation and the render |
| `VOLUME_DISK_SERIAL`, `VolumeAttachment { device }`, `DiskAttachment::volume(device)` | `overdrive-core/src/vm/config.rs` | the constant and the struct are real; `volume(device)` sets `direct` and `serial` but `to_disk_arg` renders the rootfs shape | 5: `to_disk_arg` renders `direct=on` and `serial=` |
| `VolumeUnavailableCause::Held` (unit variant) | `overdrive-core/src/volume.rs` | real; maps to `DriverInternalError` | 3: the typed mapping and wording |
| `VolumeSize`, `ByteCount` grammars | `overdrive-core/src/volume.rs` | `FromStr` refuses every input; `Display` renders nothing | 2 |
| `Vm.volume` spec surface, `VmPayload.volume`, `Vm::to_payload` | `aggregate`, `driver.rs` | the parser refuses `[vm.volume]`; the reconciler builds `volume: None`; the intent envelope knows `[0]` | 2 |
| `VolumeBacking`, `VolumePoolProbe`, `SimVolumeBacking`, `SimVolumePoolProbe` | `overdrive-core/src/traits/volume_backing.rs`, `overdrive-sim` | trait real; the sim holds no state (`ensure` fails `PoolCommand`, `claim_state` reports `NotFound`, hooks ignored) | 5: the sim bodies |
| `LvmVolumePoolProbe`, `LvmVolumeBacking` | `overdrive-host/src/volume_backing.rs` | the probe reports "no labelled partition"; the backing has no constructor (N-12) | 6 |
| `compose_volume_backing` | `overdrive-control-plane/src/lib.rs` | passes a healthy probed pool through; every other outcome is no pool, no event | 7: the refusal and the `volume_backing.not_composed` event |
| `AllocStatusResponse.volume`, describe `Volume` line | `api.rs`, `render` | the field exists; no handler fill, no render | 4 |

Test support added this wave (DISTILL's to shape, not pinned API): `overdrive_sim::adapters::SimLaunchSupersession` (`new`, `raised`, `raise`),
`SimDriver::park_next_start(observe_lane_signal) -> StartPark` (`has_entered`, `observed_supersession`, `release`), and in
`crates/overdrive-sim/src/adapters/volume_backing.rs` the hooks `ensure_calls`, `claim_state_calls`, `park_next_claim_state`, `claim_state_parked`,
`release_claim_state`, `volumes_created`, `ensure_parked`, `pause_ensure_after_create`, `resume_ensure`, `fail_next_claim_state`, `hold_claim_while_live` (scaffolds that record nothing until
row 5, except the park hooks and the scripted deadline outcome, which are functional test support). No `.config/nextest.toml` change.

## The pending tests, one block per file

Paths are under `crates/`. "Fails today" is the failure of the file's headline assertions; "Precondition stops" names the tests that
stop earlier and the marker text they stop on; "Moves" names the activation row.

### `overdrive-core/tests/acceptance/vm_volume_size.rs`
- Fails today: `VolumeSize` scaffold: `FromStr` refuses every input and `Display` renders nothing, so each body's positive control (a valid size must parse, or 40 GiB must display) fails. Every test stops at the headline.
- Moves: row 2 (GREEN).

### `overdrive-core/tests/acceptance/vm_volume_spec.rs`
- Fails today: the production parser refuses `[vm.volume]` (`deny_unknown_fields`). Headline stops: `a_job_with_a_volume_declaration_parses`, `a_service_with_a_volume_declaration_parses`; `the_intent_envelope_gains_version_two_and_keeps_version_one` (the envelope knows `[0]` only).
- Precondition stops, on "a [vm.volume] declaration must be part of the wire spec" (the wire spec cannot carry the declaration): `an_admitted_job_echoes_the_declared_size`, `the_echo_uses_the_canonical_size_spelling`, `sad_a_malformed_size_is_rejected_naming_the_size_field`, `sad_a_volume_with_several_replicas_is_rejected_for_job_and_service`, `a_single_replica_service_with_a_volume_is_admitted`, `the_volume_declaration_survives_the_intent_store_round_trip`, `the_spec_digest_changes_with_the_volume_declaration`; and on "the declared [vm.volume] size = "40GiB" must parse before a bad key can be refused": `sad_the_volume_table_refuses_unknown_keys_an_array_and_a_missing_size`.
- Moves: row 2 (GREEN; the echo, digest and round-trip assertions that follow the precondition are row 2's too).

### `overdrive-core/tests/acceptance/volume_unavailable_reason.rs`
- Fails today: `TransitionReason::from(&DriverStartFailure)` maps a `VolumeUnavailable` class to `DriverInternalError`, and the wording is the generic "VM volume unavailable". Every test stops at the headline.
- Moves: row 3 (GREEN).

### `overdrive-core/tests/acceptance/volume_value_types.rs`
- Fails today: `ByteCount` scaffold (`FromStr` refuses, `Display` renders nothing) for the byte-count tests; `VolumeSize` `FromStr` refuses 40GiB for `the_size_in_bytes_is_the_declared_binary_multiple`; the cause's serde form lacks `"free":"<n>B"` for `the_insufficient_space_cause_carries_free_space_as_a_string`; `to_disk_arg` renders the rootfs shape for `the_volume_disk_renders_the_golden_argument`. Every test stops at the headline.
- Moves: row 2 (the byte-count, size-bytes and cause tests), row 5 (the golden argument).

### `overdrive-cli/tests/acceptance/workload_describe_volume.rs`
- Fails today: `render::workload_describe` ignores `AllocStatusResponse.volume`: no `Volume` line. Headline.
- Moves: row 4 (GREEN).

### `overdrive-cli/tests/acceptance/workload_describe_capacity_wait.rs`
- Fails today: the render ignores `AllocStatusResponse.capacity_wait` (the wire type exists): no `Waiting` line. Headline.
- Moves: row 10 (GREEN).

### `overdrive-cli/tests/integration/workload_describe.rs` (module `capacity_wait_empty_state`)
- Fails today: against a stub HTTPS control plane, `describe` of a waiting workload shows the eligibility hint and no `Waiting` line. Headline.
- Moves: row 10 (GREEN).

### `overdrive-cli/tests/integration/volume_boot_probe.rs`
- Fails today: `compose_volume_backing` passes a probed pool through and treats every other probe outcome as no pool: no `health.startup.refused`, boot succeeds. `sad_a_broken_volume_pool_refuses_startup` is table-driven and its marker names the first case (`tools_or_device_mapper_unusable`), so it stops at that row; the later rows are first exercised when it passes. `a_node_with_no_labelled_partition_boots_and_logs_not_composed` stops at the missing `volume_backing.not_composed` event. Headline stops.
- Precondition stop: `a_deployed_volume_workload_launches_through_the_composed_volume_pool` (`nested-launch`, `kvm-tests`) stops at "a VM workload that declares [vm.volume] must deploy (vol-compose)".
- Moves: row 7 (GREEN; the H2 test needs row 2 first).

### `overdrive-cli/tests/integration/volume_survives.rs` (`volume-pool`, `kvm-tests`)
- Fails today: ignored until the pool exists (row 1). Once run, every test stops at a precondition: "a VM workload that declares [vm.volume] must deploy (vol-restart | vol-killed | vol-stop | vol-drain | vol-flush | vol-stop-held)", because the parser refuses `[vm.volume]`. After row 2 the next RED is the first read of the volume's device node (for example `wait_for_records` on `volume_device()`), because no launch ensures a volume until row 7.
- Fixture change: the restart test's workload is a Service. The acceptance designer changed it at the orchestrator's request on 2026-11-20, because a crashed Job is a natural exit and is never restarted (J-1/H1). The crafter did not author this change.
- Moves: row 1 removes the `#[ignore]`; row 2 moves the marker as above; rows 7 and 8 turn the tests GREEN (`a_workload_stop_during_a_held_claim_returns_promptly_with_no_failed_row` needs row 8, the stop interrupting the claim wait).

### `overdrive-host/tests/integration/volume_pool_backing.rs` and `volume_pool_probe_pool.rs` (`volume-pool`)
- Fails today: ignored until the pool exists (row 1). Once run, the probe scaffold reports no pool: the backing tests and the four healthy probe tests stop at a precondition ("the guest's pool is found (<test>)", "the labelled partition is found (<test>)"); the five sad probe tests stop at the headline ("a broken pool must fail its probe (<test>)").
- Moves: row 1 removes the `#[ignore]`; row 6 (GREEN).
- Added 2026-11-21 (P-4, P-5), both running pending tests in the `cgroup-default` guest (they need no pool, so no `#[ignore]`), each proven RED on its marker:
  - `sad_a_probe_blocked_on_the_group_lock_times_out_as_an_io_failure` stops at the headline, "the probe must return within its 30 s deadline, the adapter's own": today the probe waits on the held lock and returns only when the 45 s hold ends. The later assertions (`TimedOut`, the in-flight command's scenario, the stalled `lvm` neither killed nor lost, the next probe clean) are first exercised when the deadline lands.
  - `sad_an_unreadable_sysfs_block_directory_fails_the_probe_sysfs_unreadable` stops at the headline, "a failed sysfs read must be reported as SysfsUnreadable": today the failed read reports `ToolsOrDeviceMapperUnusable`. The scenario is compared by its `Debug` name because `VolumeProbeScenario::SysfsUnreadable` does not exist yet.
  - Moves: both go GREEN when the probe deadline (P-4) and the `SysfsUnreadable` scenario (P-5) land with the probe; the crafter removes the two markers.

### `overdrive-sim/tests/acceptance/sim_volume_backing_contract.rs`
- Fails today: `SimVolumeBacking` holds no state (`ensure` fails `PoolCommand`, `claim_state` reports `NotFound`, hooks ignored).
- Headline stops: `sad_insufficient_space_fails_typed_and_creates_nothing`, `sad_a_failing_command_is_a_pool_command_error_and_the_next_ensure_recovers` ("the fault was one-shot").
- Precondition stops (a first `ensure` that must succeed, or a hook the scaffold ignores): all the others, on "ensure ... must create the volume ...", "the first ensure is parked after its create", "1TiB holds three 10GiB volumes" (the model's own capacity) and "ensure must return the volume's device" (the golden-name test).
- Moves: row 5 (GREEN).

### `overdrive-sim/tests/acceptance/volume_launch_invariants.rs`
- Fails today: the launch ignores `VmPayload.volume`: the invariants' own oracles fail ("a launch that declares a volume must hold it through its VMM", "every launch must attach the volume's one device", "the first launch must attach the volume"). Headline.
- Moves: GREEN at row 7 (step 04-02): the markers are off.

### `overdrive-worker/tests/acceptance/node_volume_launch.rs`
- Fails today: `VmDriver::start` does not read `VmPayload.volume`, so a launch is never inside `ensure` or `claim_state`. Headline stops: `a_declared_volume_is_ensured_and_attached_as_the_second_disk`, the four typed `VolumeUnavailable` sad tests, the claim-state-error test, `a_replacement_waits_for_the_claim_not_for_the_process_exit`, and `sad_a_claim_held_past_the_bound_fails_held_and_attaches_nothing` ("a declared volume whose claim stays held must fail VolumeUnavailable": the sim now holds the claim, and the launch, which ignores `VmPayload.volume`, ends `BootDeadlineExceeded` instead).
- Precondition stops: `pool_faults_after_the_launch_do_not_reach_the_running_allocation_or_its_stop` ("the launch must have created the volume the pool faults then leave in place"), `after_a_node_reboot_the_next_launch_reactivates_the_same_volume` ("the first launch must attach a volume disk"), `a_control_plane_restart_between_create_and_attach_reuses_the_volume` ("the launch must park after the create effect of its ensure").
- Moves: GREEN at row 7 (step 04-02): every marker is off.

### `overdrive-worker/tests/acceptance/launch_supersession.rs`
- Fails today: `VmDriver::start` names its supersession parameter `_supersession` and never observes it. Headline stops: every non-volume test and the volume tests of points 1 and 5 ("... must be abandoned on the supersession signal, not run on to its own bound"), and the two `the_signal_wins_over_the_...boot_deadline_at_the_same_instant` tests ("must end StartSuperseded, got Err(StartRejected").
- Precondition stops: the volume tests of points 2, 3 and 4: `a_signal_during_ensure_waits_for_ensure_then_abandons_whatever_ensure_returns` ("the launch must be inside ensure before the signal is raised"), `a_signal_during_the_claim_poll_sleep_returns_at_once_without_polling_again`, `the_signal_wins_over_a_claim_held_at_the_bound` ("the launch must be inside ensure before its volume's claim is held"), `a_signal_during_the_last_claim_poll_creates_nothing_after_the_claim_wait` ("the launch must be inside claim_state before the signal is raised"). `a_volume_launch_whose_signal_is_never_raised_is_the_ordinary_volume_launch` stops at its own headline ("an unsuperseded volume launch must ensure the volume once").
- GREEN at row 7 (step 04-02, marker off): `a_volume_launch_whose_signal_is_never_raised_is_the_ordinary_volume_launch` is GREEN. The four precondition tests of points 2, 3 and 4 now stop at the `expect_superseded` assertion, the marker retargeted to the text each produces: `a_signal_during_ensure_waits_for_ensure_then_abandons_whatever_ensure_returns` ("a launch superseded while ensure was in flight and then succeeded must be abandoned on the supersession signal, not run on to its own bound"), `a_signal_during_the_claim_poll_sleep_returns_at_once_without_polling_again` ("a launch superseded while it sleeps between claim polls must be abandoned ..."), `a_signal_during_the_last_claim_poll_creates_nothing_after_the_claim_wait` ("a launch superseded as its claim wait ends must be abandoned ..."), and `the_signal_wins_over_a_claim_held_at_the_bound` ("a launch superseded at its claim bound must end StartSuperseded, got Err(StartRejected"). The fifth volume test, `a_volume_launch_superseded_in_its_boot_wait_tears_the_vmm_down_and_keeps_the_volume` (point 5), keeps its marker. Row 8 turns every remaining test GREEN.

### `overdrive-reconcilers/tests/acceptance/workload_volume_projection.rs`
- Fails today: precondition, "a [vm.volume] workload must be admitted by the production wire spec" (the wire spec refuses the declaration).
- Moves: row 2: the marker moves to "<site> must carry the declared volume in its VmPayload" (the reconciler builds `volume: None`), and the tests go GREEN when `Vm::to_payload` lands, which is also row 2.

### `overdrive-reconcilers/tests/acceptance/generation_replacement_pending.rs`
- Fails today: `generation_replacement_pending` panics `RED scaffold`; the helper wrapper catches exactly that payload and re-panics with the contract assertion. Every test stops at the headline.
- Moves: row 9 (GREEN).

### `overdrive-reconcilers/tests/acceptance/start_blocked_only_by_capacity.rs`
- Fails today: `start_blocked_only_by_capacity` panics `RED scaffold`; `verdict` / `predicate` catch exactly that payload (any other panic is resumed) and yield no verdict, so each test fails on its named assertion ("must classify ... as <verdict>", "must agree with the reconciler ... (the predicate returned no verdict)", "must be reached with both verdicts"). Every test stops at the headline; the non-vacuity guard passes today.
- Moves: row 10 (GREEN; no marker moves in between: the markers already name the verdict assertions, which fail today only because the predicate yields no verdict). `failure_persistence: None` in the proptest config goes with the markers.

### `overdrive-reconcilers/tests/acceptance/restart_generation_reconcile.rs`
- Fails today: the stamp-on-emit rule treats a generation as handled once an id is issued. Headline: `a_lost_launch_is_relaunched_once_for_the_pending_generation`, `an_abandoned_replacement_relaunch_charges_nothing`; and `a_restart_never_gives_an_operator_stopped_job_a_new_allocation` (the Job fence).
- Moves: row 9 (GREEN).

### `overdrive-control-plane/tests/acceptance/alloc_status_volume.rs`
- Fails today: precondition, "a [vm.volume] declaration must be part of the service wire spec".
- Moves: row 2 moves the marker to "the response must carry the declared volume size"; row 4 turns it GREEN.

### `overdrive-control-plane/tests/integration/alloc_status_capacity_wait.rs` and `capacity_wait_composition.rs`
- Fails today: the handler writes `capacity_wait: None`. Headline: "a withheld first placement on a full node: the response must carry the pool's wait at the cap" and "a deploy onto a full node: describe must show the capacity wait while a deploy is withheld" (the message still says "describe"; the assertion is on the response's `capacity_wait`, see `test-scenarios.md` § L, t). The acceptance files keep two no-fill guards and the shared rig.
- Moves: row 10 (GREEN).

### `overdrive-control-plane/tests/acceptance/launch_lane_supersession.rs`
- Fails today: `supersede_lane` and `supersede_all_lanes` are no-ops and `lane_supersession` returns a never-raised signal, so the signal is never raised. Headline: the restart, stop and repeated-stop tests and `a_signal_after_the_lane_opens_and_before_hydration_is_visible_on_the_lane`.
- Precondition stops (a no-leak claim that cannot be observed until the signal can be raised at all): `supersede_lane_for_one_workload_never_raises_another_workloads_lane`, `supersede_lane_never_cancels_the_shutdown_token`, `a_closing_lane_removes_only_its_own_entry_and_leaves_no_raised_signal_behind`, and `supersede_all_lanes_at_any_point_of_a_seeded_schedule_abandons_every_launch`.
- Moves: row 8 (GREEN), except `a_restart_the_evaluation_already_hydrated_sees_its_own_start_superseded_once`, which needs rows 8 and 9 (see "Tests that need two activation rows").

### `overdrive-control-plane/src/node_volume_boundaries.rs`
- Fails today: `node_shutdown_ends_a_waiting_launch_with_no_row_and_the_loop_exits` stops at "shutdown must raise the signal of the lane a waiting launch is dispatching under" (the headline). The two tests that deploy a volume workload stop at a precondition, "a [vm.volume] workload must be accepted by the production spec parser"; with the sim bodies of row 5 in place, `a_launch_held_past_its_bound_writes_the_typed_failed_row_and_charges_the_restart_budget` stops at "the held claim must fail the launch typed" (the launch ignores the volume and ends `VmBootDeadlineExceeded`) and `a_stop_interrupts_the_start_and_its_result_is_never_used` at "a stop recorded during a launch's claim wait must end the launch at once with StartSuperseded" (the "must keep waiting" assertion holds today because the launch waits on its boot beacon, not a claim).
- Moves: both markers already name their post-row-5 stops (see above). `a_launch_held_past_its_bound_writes_the_typed_failed_row_and_charges_the_restart_budget` is GREEN since row 7 (step 04-02, marker off); `a_stop_interrupts_the_start_and_its_result_is_never_used` keeps its marker ("a stop recorded during a launch's claim wait must end the launch at once with StartSuperseded", the row-8 RED: with row 7 the launch waits on the claim, and only the stop's lane signal is missing) and turns GREEN at row 8. The loop-shutdown test is GREEN at row 8.

### `overdrive-control-plane/src/vm_reclamation_boot.rs` (module `mid_boot_volume_reclamation`)
- Fails today: precondition, "a workload declaring a volume must be accepted" (the parser refuses `[vm.volume]`); the reclamation assertions (orphan not live before any launch and at the one create, the claim drains, the successor attaches the volume with nothing charged) are first exercised once the spec surface, the launch path and the sim bodies land.
- Moves: both tests GREEN at row 7 (step 04-02, markers off). The fixture's pool is 100 GiB, the size the volume quint driver uses, so the 40 GiB volume the spec declares fits; an 8 GiB pool failed the launch `InsufficientSpace` before the scenario.

### `overdrive-control-plane/src/action_shim/prelaunch_outcome_acceptance.rs`
- Fails today: the shim has no `StartSuperseded` arm; `ensure_intercept_identity` failures and an empty driver registry return `Err` and write no row. Headline stops: the two S-SHIM-01 tests ("a superseded launch is a clean no-row outcome: the ... dispatch returns Ok(()), got Err("), the identity-issue tests ("an identity issuance failure is recorded as a row: ..."), the teardown test, the R3 test.
- Precondition stops: the two charge tests (`a_persistent_identity_failure_is_charged_until_the_restart_ceiling_then_exhausted`, `a_job_identity_failure_is_final_and_never_relaunched`) stop at "recorded as a row" before any charge, exhaustion or Job-finality assertion; the two wait-predicate tests stop on the scaffold's panic carried into their own message ("the wait predicate must be true after an admission refusal: it panicked: RED scaffold: start_blocked_only_by_capacity is not implemented").
- Moves: row 8 (the two superseded-launch tests, GREEN), row 9 (the identity, R3 and charge tests, GREEN), row 10 (the two predicate tests, GREEN).

### `overdrive-control-plane/tests/integration/restart_generation_invariant.rs`
- Fails today, by cell: the stop, second-restart, shutdown and crash-restart-successor-shutdown cells stop at "the interruption must raise the supersession signal of the held launch's lane, ending it with StartSuperseded (ADR-0334)" (the lane table is a scaffold; precondition stop before their placement assertions). The two control-plane-crash cells, which need no lane table, fail on the generation rule itself ("the abandoned generation must be relaunched: after Crash the workload has no Running allocation for the committed generation 1": the stamp-on-emit rule marks the generation handled). The persistent, transient and Job cells stop at their first recorded-row assertion ("a persistent identity_issue failure must write a charged Failed row naming MtlsInterceptInstallFailed", "transient failures must present as Failed rows MtlsInterceptInstallFailed", "a Job whose SVID issuance fails must write exactly one Failed row (J-1)", "a transient identity_issue failure must write a charged Failed row and be relaunched until a launch reaches the driver (ADR-0337)": precondition stops before any charge, exhaustion or finality assertion; the Job-restart test stops at "a restart of a running Job must end it and issue nothing after ... (H1)", the headline).
- Matrix tests: `restart_of_a_running_workload_{interrupted_by_stop,overridden_by_a_second_restart,abandoned_by_shutdown_is_relaunched_on_the_next_boot,lost_with_the_control_plane_is_relaunched_on_the_next_boot}` (the first two end `..._places_exactly_once`) and the four `resume_of_a_stopped_workload_*` counterparts; `a_crash_restart_successor_abandoned_by_shutdown_is_relaunched_with_the_same_count`; `a_transient_identity_fault_then_an_interrupted_launch_places_once`; `a_persistent_identity_failure_ends_backoff_exhausted`; `a_persistent_identity_failure_launches_a_bounded_number_of_times_within_one_backoff_window`; `a_transient_identity_failure_recovers_to_running`; `a_restart_of_a_running_job_ends_it_and_issues_nothing_after`; `a_job_whose_identity_issuance_persistently_fails_is_finalised_failed_and_never_relaunched`.
- Moves: see the activation table (rows 8 and 9) and "Tests that need two activation rows" (the shutdown cells).

### `overdrive-control-plane/src/quint_node_volume.rs`
- Fails today: every trace passes `Init` and diverges at step 2, `Declare`: "the production spec parser must accept the [vm.volume] declaration: [vm]: unknown field `volume`" (the real parser text). Precondition stop for all; every handler after `Declare` is compile-checked only.
- `quint_vol_randomized` pins `seed = "1"`.
- GREEN at row 7 (step 04-02), markers off: `happy_path_deploy_to_running`, `restart_waits_for_drain_then_attaches`, `stop_keeps_volume_and_restart_reattaches`, `crash_after_create_before_attach_reuses_volume`, `node_reboot_reactivates_and_data_survives`, `launch_bound_exceeded_fails_typed_and_charges_budget`. The spec fails a launch that cannot provide the volume inside `WlStart` (as production does inside its first tick), so no `PENDING` state is modelled for it; `store_absent_fails_typed_and_never_attaches` is GREEN. `no_space_fails_typed_and_never_attaches` is GREEN: the conformance main `vol_conf` instantiates the spec's restart limit `LIMIT` with 6, production's `RESTART_BACKOFF_CEILING` + 1, so `exhausted` holds after the sixth typed failure in both (the exhaustive and hazard mains keep `LIMIT = 3`). Pending, marker "LaunchSuperseded: the step did not settle within the settle bound: the superseded launch's dispatch returns" (row 8): the four supersession runs. `quint_vol_randomized` is pending until step 05-02, marker "State invariant failed" (quint-connect prints only this text for a state mismatch): at seed 1, step 7 (Shutdown, then Declare with the store absent or the pool full, then `WlStart(1)`), the spec holds `alloc[1]` PENDING because a lane born raised observes the shutdown signal before the volume check, while production fails it typed, because the volume launch path does not yet observe the shutdown/supersession signal; step 05-02 wires that.

### `overdrive-control-plane/src/quint_restart_generation.rs`
- Fails today: `quint_rg_control_plane_crash_during_restart_launch_relaunches_once` at step 8, `WlStart` ("the evaluation reached no launch dispatch for allocation 3": the generation rule); `quint_rg_second_restart_loses_launch_then_one_placement` at step 10, `LaunchLost` ("supersede_lane must raise the signal of the lane the held launch ... is dispatching under": precondition, the lane table); the four pre-driver runs (`restart_of_exhausted_workload_relaunches_uncharged`, `generation_replacement_forgives_predecessor_failure`, `persistent_prelaunch_failure_ends_in_backoff_exhausted`, `transient_prelaunch_failure_recovers`) at the step-5 projection ("the dispatch ended with `shim failure: issue_svid dispatch failed` and wrote no row": precondition, the pre-driver row); `quint_rg_resume_of_stopped_job_does_nothing_and_issues_no_id` at "the Job issued allocation id" (headline: production gives a resumed operator-stopped Job a new allocation id; it moves with row 9); `quint_rg_randomized` at the first divergence a seed hits.
- Moves: row 8 (the second-restart run, expected GREEN), row 9 (the rest), randomized last (row 11).

## Tests that need two activation rows

Two groups of the restart-generation tests are fixed only by the lane table (row 8) and the generation rule (row 9) together. The marker in the source today is the row-8 stop; the marker that applies after row 8 alone is the row-9 stop. DISTILL retargets the marker as part of row 8's activation, read from the bodies (the assertion that fails after row 8 alone was read, not run).

| Test | Marker today (stops until row 8) | After row 8 alone | GREEN at |
|---|---|---|---|
| `restart_of_a_running_workload_abandoned_by_shutdown_is_relaunched_on_the_next_boot`, `resume_of_a_stopped_workload_abandoned_by_shutdown_is_relaunched_on_the_next_boot` (the shutdown cells) | "the interruption must raise the supersession signal of the held launch's lane, ending it with StartSuperseded (ADR-0334)" | the signal is raised and the launch ends `StartSuperseded`; the stamp-on-emit compat rule still counts the abandoned launch's generation as handled, so the next boot places nothing: "the abandoned generation must be relaunched: after Shutdown the workload has no Running allocation for the committed generation 1" (the sibling of the crash cells' marker) | row 9 |
| `a_restart_the_evaluation_already_hydrated_sees_its_own_start_superseded_once` | "a restart whose commit the evaluation already hydrated must see its own start superseded once" | the tick ends and the driver observed the supersession, then the next evaluation relaunches nothing under the compat rule while the shim's predecessor cleanup has ended the old allocation: "the generation ends with exactly one Running allocation" | row 9 |

The other cells of the invariant need only row 8 and go GREEN there: the stop and second-restart cells (the new commit's generation is placed by the compat rule) and `a_crash_restart_successor_abandoned_by_shutdown_is_relaunched_with_the_same_count` (a crash restart has no generation; the failed predecessor stays the current row, which is why its crash twin already passes). The crash cells, the persistent, transient and Job cells and the pre-launch conformance runs need only row 9.

## Guards (hold before and after, not pending)

Each passes today because the behaviour already holds or the scaffold already satisfies the property, and must keep passing. A guard that depends on a scaffold being a no-op
(`a_launch_that_won_its_boot_race_before_the_stop_signal_is_stopped_as_an_ordinary_running_allocation`, which holds while `supersede_lane` does nothing) must also pass once the lane table lands.

- `overdrive-core/tests/acceptance/vm_volume_spec.rs`: `guard_a_job_without_a_volume_echoes_no_volume`
- `overdrive-core/tests/acceptance/volume_unavailable_reason.rs`: `guard_a_volume_failure_is_a_failure`
- `overdrive-core/tests/acceptance/volume_value_types.rs`: `guard_volume_operation_labels_match_serde`, `guard_the_rootfs_disk_argument_is_unchanged`
- `overdrive-cli/tests/acceptance/workload_describe_volume.rs`: `guard_a_workload_without_a_volume_prints_no_volume_section`
- `overdrive-cli/tests/acceptance/workload_describe_capacity_wait.rs`: `guard_a_workload_that_is_not_waiting_renders_without_the_waiting_line`
- `overdrive-reconcilers/tests/acceptance/start_blocked_only_by_capacity.rs`: `the_agreement_generator_reaches_starts_withheld_starts_and_released_starts`
- `overdrive-reconcilers/tests/acceptance/restart_generation_reconcile.rs`: `an_evaluation_with_the_issued_ids_row_present_issues_nothing`, `bumps_before_the_placement_coalesce_into_one_placement`, `an_evaluation_after_the_successors_row_exists_does_not_replace`, `a_relaunched_crash_restart_is_charged_once_not_twice`, `a_first_placement_records_the_committed_generation`, `a_restart_never_gives_a_completed_or_failed_job_a_new_allocation`, `a_job_row_with_only_a_reason_is_replaced_as_on_main`
- `overdrive-store-local/tests/acceptance/volume_unavailable_row_roundtrip.rs`: `guard_a_volume_unavailable_row_round_trips_every_cause_across_a_store_reopen`
- `overdrive-worker/tests/acceptance/node_volume_launch.rs`: `guard_a_workload_without_a_volume_launches_unchanged_on_a_node_with_no_pool`, `guard_a_composed_pool_is_untouched_by_a_workload_without_a_volume`
- `overdrive-worker/tests/acceptance/launch_supersession.rs`: `guard_a_launch_that_won_its_boot_race_before_the_signal_is_an_ordinary_success`, `guard_a_launch_whose_signal_is_never_raised_is_the_existing_launch`, `guard_a_signal_raised_for_another_launch_does_not_affect_this_one`
- `overdrive-control-plane/tests/acceptance/alloc_status_capacity_wait.rs`: `guard_a_running_workload_with_room_on_the_node_carries_no_capacity_wait`, `guard_a_deployed_workload_on_a_node_with_room_is_not_waiting`
- `overdrive-control-plane/tests/acceptance/launch_lane_supersession.rs`: `supersede_lane_with_no_open_lane_does_nothing`, `submit_workload_new_unchanged_and_conflicting_never_raises_the_lane_signal`, `evaluations_submitted_for_a_busy_target_never_raise_its_lane`, `a_signal_with_no_lane_open_does_nothing_and_hydration_sees_the_commit`, `a_teardown_failure_while_abandoning_reaches_the_shim_as_an_unclassified_rejection`, `a_driver_that_ignores_the_signal_is_unchanged`
- `overdrive-control-plane/src/node_volume_boundaries.rs`: `shutdown_leaves_a_running_allocation_and_a_non_launch_evaluation_alone`
- `overdrive-control-plane/src/action_shim/prelaunch_outcome_acceptance.rs`: `start_arm_guest_network_provision_failure_row_is_unchanged`, `start_arm_with_no_driver_composed_row_is_unchanged`, `start_arm_failing_prior_row_read_returns_the_observation_error_with_no_effect`, `restart_arm_failing_prior_row_read_returns_the_observation_error_with_no_effect`, `start_arm_admission_refusal_writes_no_row_charges_nothing_and_the_next_tick_emits_nothing`, `restart_arm_admission_refusal_releases_the_predecessor_and_the_next_tick_emits_again`, `start_arm_for_a_finalised_job_row_is_a_no_op`, `restart_arm_with_a_finalised_job_predecessor_is_fenced`, `restart_arm_with_a_running_predecessor_is_fenced`, `restart_arm_guest_network_provision_failure_writes_a_failed_successor_row`, `restart_arm_duplicate_vm_owner_is_a_no_row_success_that_still_cleans_the_predecessor`, `start_arm_activation_withheld_at_fail_stop_leaves_the_running_row_and_returns_ok`, `restart_arm_activation_withheld_at_fail_stop_skips_the_predecessor_cleanup`, `a_launch_that_won_its_boot_race_before_the_stop_signal_is_stopped_as_an_ordinary_running_allocation`
- `overdrive-control-plane/tests/integration/restart_generation_invariant.rs`: `a_crash_restart_successor_lost_with_the_control_plane_is_relaunched_with_the_same_count`, `a_restart_with_no_interruption_places_once_and_a_second_evaluation_issues_nothing`, `an_operator_stop_keeps_the_workload_down_until_the_next_restart`, `the_same_seed_gives_the_same_trace`
- `overdrive-control-plane/src/quint_restart_generation.rs`: `quint_rg_resume_while_running_stops_and_replaces`, `quint_rg_admission_refusal_leaves_no_row_and_waits_for_capacity`, `quint_rg_restart_of_completed_job_does_nothing_and_issues_no_id`
- `overdrive-cli/tests/integration/volume_boot_probe.rs`: `guard_a_healthy_pool_boots_without_a_refusal_or_a_not_composed_event`
- `overdrive-cli/tests/integration/workload_describe.rs`: `describe_with_no_wait_and_no_rows_keeps_the_generic_empty_state`
- `overdrive-host/tests/integration/volume_pool_probe.rs`: `guard_a_node_without_a_labelled_partition_composes_no_volume_pool`

## DELIVER activation order

The roadmap is DELIVER's; this is the dependency order the tests require. The guest-kit volume-pool row is first: the pool-dependent Tier-3 tests become runnable and each is proven RED on the
marker it carries before the real LVM adapters are written. "Turns GREEN" means DELIVER removes the marker (and, at row 1, the `#[ignore]`). "Markers that move" are the pending tests whose
`expected` is a precondition message that stops being the first failure when the row lands: DISTILL retargets the marker as part of the row's activation, to the new semantic RED named here
(or the test goes GREEN in the same row, and the retarget is transient).

| Row | Capability | Turns GREEN | Markers that move (precondition now -> next RED) |
|---|---|---|---|
| 1 | Guest-kit volume pool for the `volume-pool` profile (`guest_kit_prepare.rs`, `guest_kit.rs`): labelled partition, PV and `overdrive` group, `lvm`, `sfdisk`, `flock`, `dmsetup`, `losetup` with loop-device support, `dm-flakey` and its loader, a static busybox with `dd`/`head`/`printf` (`upstream-issues.md` N-5) | none | none: the `#[ignore]` comes off every `volume-pool` test and each is proven RED on the marker it already carries |
| 2 | `VolumeSize`, `ByteCount` grammars, `Vm.volume` spec surface and validation, envelope V2 with its fixtures (S-VOL-21), `Vm::to_payload` | `vm_volume_size`, `vm_volume_spec` (every pending test), the `volume_value_types` byte-count, size-bytes and cause tests | `workload_volume_projection` ("must be admitted by the production wire spec" -> "<site> must carry the declared volume in its VmPayload", GREEN in this row once `Vm::to_payload` lands); `alloc_status_volume` ("must be part of the service wire spec" -> "the response must carry the declared volume size", GREEN at row 4); the two volume tests of `node_volume_boundaries` ("must be accepted by the production spec parser" -> "the volume exists before the claim is held", the `SimVolumeBacking` scaffold's `ensure` failing typed inside `hold_volume_claim`; row 5 (the sim bodies, landed with step 03-01) moves them to "the held claim must fail the launch typed" and "a stop recorded during a launch's claim wait must end the launch at once with StartSuperseded"); M7 ("a workload declaring a volume must be accepted" -> "the launch must attach the declared volume to its VMM", the first launch's attach assertion, since the launch path does not yet wire `VmConfig.volume`; confirmed at activation); all `quint_vol_*` ("Declare ... unknown field `volume`" -> "LaunchEnsureCreate: the step did not settle within the settle bound: the launch's ensure parks after its create effect" for the nine runs that reach `LaunchEnsureCreate`; "LaunchUnavailable: the step did not settle within the settle bound: the allocation row reaches the state" for `no_space` and `store_absent`; `quint_vol_randomized` -> "the step did not settle within the settle bound", the one stem both share; confirmed at activation); the seven `volume_survives` tests ("must deploy (...)" -> "exactly one volume node", the first read of the volume's device node, confirmed at activation) and `a_deployed_volume_workload_launches_through_the_composed_volume_pool` ("must deploy (vol-compose)" -> "one workload, one volume", the launch creates its VMM with no volume attached so the composed backing created none; confirmed at activation) |
| 3 | `VolumeUnavailable` mapping and wording (incl. `InsufficientSpace`, `PoolCommandFailed`, unit `Held`) | `volume_unavailable_reason` | none |
| 4 | `AllocStatusResponse.volume` and the describe section | `workload_describe_volume`, `alloc_status_volume` | none |
| 5 | `SimVolumeBacking` (all bodies and the scripted-deadline, park, call-count and `hold_claim_while_live` hooks), port docstrings, `DiskAttachment::volume` rendering | `sim_volume_backing_contract` (every pending test, including the precondition stops on "ensure ... must create the volume ..."), `the_volume_disk_renders_the_golden_argument` | `sad_a_claim_held_past_the_bound_fails_held_and_attaches_nothing` ("the volume whose claim is held must exist" -> "a declared volume whose claim stays held must fail VolumeUnavailable"); the two `node_volume_boundaries` tests above (as row 2 states). The other launch-path tests keep their launch-path markers until row 7 |
| 6 | `LvmVolumePoolProbe` and `LvmVolumeBacking` | `volume_pool_backing`, `volume_pool_probe_pool` (S-VOL-T3-02..05; T3-01 stays a guard) | none |
| 7 | Launch-path `ensure` plus claim wait, `compose_volume_backing` (event and refusal), the `VmConfig.volume` and `--disk` wiring, node chown | `node_volume_launch`, `volume_launch_invariants`, `a_volume_launch_whose_signal_is_never_raised_is_the_ordinary_volume_launch` (GREEN here, before the supersession points exist), `pool_faults_after_the_launch...`, `volume_boot_probe`, `a_launch_held_past_its_bound_writes_the_typed_failed_row_and_charges_the_restart_budget`, `the_driver_holds_no_ensure_timer_a_parked_ensure_outlives_the_deadline`, M7, `volume_survives` T3-07, T3-09, T3-11, T3-13 ("must deploy (vol-stop-boot)" -> the VMM-terminated assertion; over rows 2 and 6); the activation also adds the three DST invariants to `Invariant::ALL` and updates the catalogue count | the five volume tests of `launch_supersession` at points 2, 3 and 4 ("the launch must be inside ensure/claim_state ..." (for the ensure test the `ensure` parked by `SimVolumeBacking::pause_ensure_after_create`) -> "<what> must end StartSuperseded, got ...") |
| 8 | Launch supersession: the lane table in `ReconcilerRuntime` (`supersede_lane`, `supersede_all_lanes`, `lane_supersession`, the lane guard, lanes as children of the shutdown token), the producers (`stop_workload` on `Inserted` and `KeyExists`, `restart_workload`, the loop's shutdown arm), `VmDriver::start` observation points 1 to 6 with `StartSuperseded`, and the shim's no-row `StartSuperseded` arm in both launch arms (release lease, SVID and supervision claim, `launch.superseded`, `finish_restart` in the restart arm) | `launch_supersession` (all remaining), `launch_lane_supersession` (all but the hydrated-restart test), `node_volume_boundaries` (loop shutdown, `a_stop_interrupts...`), the two superseded-launch tests of `prelaunch_outcome_acceptance`, the stop, second-restart and crash-restart-shutdown cells of the invariant, `a_workload_stop_during_a_held_claim_returns_promptly_with_no_failed_row`, `quint_rg_second_restart_loses_launch_then_one_placement` (expected; confirmed at activation) | the two shutdown cells of the invariant and `a_restart_the_evaluation_already_hydrated_sees_its_own_start_superseded_once` move to their generation-rule stops (table above) |
| 9 | Restart generation: `generation_replacement_pending` adopted at the four reconciler sites and in `restart_status_for_alloc` (the compat rule and its inline restatement deleted); the shim's pre-driver `Failed` rows (S5, R7, R3: `MtlsInterceptInstallFailed { stage: "identity_issue" }` and the `DriverInternalError` successor row) | `generation_replacement_pending`, `restart_generation_reconcile`, the identity, R3 and charge tests of `prelaunch_outcome_acceptance`, every remaining cell of the invariant (crash, shutdown, persistent, transient, Job), the hydrated-restart test, `quint_rg_control_plane_crash_during_restart_launch_relaunches_once`, the four pre-driver `quint_rg_*` runs, `quint_rg_resume_of_stopped_job_does_nothing_and_issues_no_id` | none after row 8 |
| 10 | Capacity wait: `start_blocked_only_by_capacity` implemented and used by the reconciler's rules, the `alloc_status` handler fills `capacity_wait`, the render prints the `Waiting` line and suppresses the empty-state hint | `start_blocked_only_by_capacity` (the table, the three properties and the verdict-reach test; their markers already name the verdict assertions, so none moves), the two wait-predicate tests of `prelaunch_outcome_acceptance`, the integration handler and composition tests, `workload_describe_capacity_wait`, the integration `capacity_wait_empty_state` test | none |
| 11 | Conformance markers: `quint_node_volume.rs` comes off its markers as rows 2, 5, 7 and 8 land (eight runs came off at row 7; the no-space and store-absent runs wait on a seam for the pre-park failure); `quint_restart_generation.rs` once 8 and 9 land; each driver's randomized trace last. The volume driver's `Reclaim` arm additionally needs the sim claim to drain on holder death (`test-scenarios.md` § L, f) | `quint_vol_*`, `quint_rg_randomized` | none |

S-VOL-21 (the V2 fixtures) is not authored: the V2 bytes exist only in DELIVER's one atomic envelope-bump commit.

## What carries over

- Scaffold bodies do not panic where a panic would hide the test's own assertion: the value-type scaffolds refuse or render nothing, the sim scaffold holds no state, and every test that rejects text carries a positive control a refuse-everything scaffold cannot satisfy.
- Tests that need a size build it through `overdrive_core::testing::volume::{volume_size_of_bytes, volume_size_gib, volume_size_tib}` (a `test-utils` fixture on the pinned archived form), never through the grammar.
- The reconcile-trajectory golden (`hydration_trajectory/workload_lifecycle_trajectory.txt`) carries the added `volume: None,` line (user decision D-3) and the renamed `issued_generation` view field.
- The three volume DST invariants stay outside `Invariant::ALL` (open decision D-2): they run through `Harness::new().only(..).run(seed)`, seeds 1 to 8, each printed.

## Runs (metal, ordinary grammar)

Totals of tests run are in `test-scenarios.md` § M; this table gives the outcome of each run of the final tree.

| Command | Run id | Result |
|---|---|---|
| `cargo check --workspace --all-targets --features integration-tests` | `eae716a581f47dc6d01c53f8` | exit 0 |
| `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | `c9643aff5faf689e6b96dac6` | exit 0 (an earlier run, `0694f9d99c83acb6b77cb9cb`, found one `manual_assert` in `generation_replacement_pending.rs`, fixed) |
| clippy of `overdrive-cli`, `overdrive-host`, `overdrive-control-plane`, `overdrive-worker` with `integration-tests,kvm-tests` | `01478ddeb7b76f78c9308790` | exit 0 |
| `cargo nextest run` over the nine crates of the feature, `--features integration-tests --no-fail-fast` | `53508ca92e3749eb6c39e386` | all passed except two: the pre-existing `vm_lifecycle_latency_283_spike::admission_close_drains_owned_starts_without_admitting_ninth` (it fails identically on the unmodified HEAD `07a0d8a5b`) and `quint_rg_resume_of_stopped_job_does_nothing_and_issues_no_id`, a vacuous guard exposed by the stricter Job projection and now marked pending |
| the quint and seeded invariants after the marker change | `7a6fa2d1ace12f2e39f110ca` | passed |
| the seeded invariants, second run | `ccd74ec767c02548d0d88db7` | passed, identical printed seeds |
| `cargo run -p overdrive-sim --bin dst -- --seed 7` | `d1185fdbd847d0b3be0a5805` | 43 invariants passed |
| `cargo xtask placement-lint` | `f5a281faa219d3bfe188cac8` | exit 0 |
| `cargo xtask conformance-lint` | `76eb1bb9ec78f1957d823706` | exit 0 |

## The stuck-VM design (RB-1 to RB-13; ADR-0348 to ADR-0355) — added 2026-11-24

Every test below is MISSING_FUNCTIONALITY: it reaches its own assertion today and fails on the stated line. Each ran on metal (run
`aba1dcee8e40b97f71601595`, 75 passed: a pending test passes when its body panics with its `expected` text). No test stops at a
precondition except where named. Tests the design requires but that cannot compile before an owner API exists are in
`test-scenarios.md` § N3; they have no classification until they are written.

| Test | Fails today on | Activation (what lands, then the marker is removed) |
|---|---|---|
| `start_arm_records_the_pending_row_with_the_leases_address_before_the_driver_starts`, `restart_arm_records_the_successors_pending_row_...` | "the store holds the launch's Pending row when the driver starts": no row exists when `Driver::start` is reached | the shim's `Pending` write after `provision_and_inject_netns` (ADR-0353) |
| `start_arm_failing_pending_write_never_reaches_the_driver_and_releases_the_lease` | "the driver is never reached when the Pending write fails": there is no write, so the driver starts | the same write, with its N-REC failure arm |
| `start_arm_failed_start_row_carries_the_pending_rows_address`, `a_stops_terminal_row_carries_the_prior_rows_workload_addr`, `finalize_failed_genuine_terminal_forward_carries_workload_addr`, `a_platform_reclaimed_row_carries_the_prior_rows_address` | the row's `workload_addr` is `None` | the terminal-row carry table of ADR-0353 |
| `start_arm_superseded_launch_ends_its_row_as_abandoned_...`, `restart_arm_superseded_successor_ends_its_row_as_abandoned_...`, `start_arm_teardown_failure_does_not_suppress_the_abandoned_row`, `start_arm_failing_abandoned_row_write_is_returned_as_the_typed_observation_error` | "a superseded launch ends its Pending row with an abandoned row": the allocation has no row | W1 and W2 of ADR-0355 with the two `TransitionReason` variants (until then the tests read the reason through `human_readable`) |
| `start_arm_a_non_start_driver_error_ends_in_a_failed_driver_internal_error_row` | "...ends the launch in a Failed DriverInternalError row, got no row" | W3 of ADR-0355 |
| `a_stop_keeps_the_lease_held_while_the_vms_scope_is_present_and_a_reclaim_releases_it_after` | "the lease is held while the VM's scope is present": the stop releases it | the lease gate in `teardown_guest_network` (ADR-0156 amended) |
| the five rewritten `launch_lane_supersession.rs` tests, `a_stop_interrupts_the_start_and_its_result_is_never_used`, `node_shutdown_ends_a_waiting_launch_with_an_abandoned_row_and_the_loop_exits` | "a superseded launch ends its Pending row as an abandoned row" (zero rows) | W1 and W2 |
| `launches_lost_with_the_control_plane::{a_pending_row_with_no_host_artifact_..., a_second_boot_over_the_closed_rows_writes_nothing}` | "a launch lost with the control plane ends Terminated": the `Pending` row is left `Pending` | the first step of `converge` (RB-11, W4) |
| `launches_lost_with_the_control_plane::a_pending_row_whose_vm_left_a_settled_scope_...` | "the row names why: `launch abandoned: lost with the control plane`": the executor ends it platform-reclaimed | the same step |
| `launches_lost_with_the_control_plane::{a_busy_scope_boots_ends_the_running_row_and_reserves_its_address, a_busy_scope_of_a_pending_row_...}` | "a busy scope never refuses the boot": `converge` returns `ConvergeError::Reclamation(Host(ResourceBusy))` | ADR-0350 and ADR-0354: busy collection, the reservation and the busy row write |
| `mid_boot_volume_reclamation` (both cells) | "the parked launch's Pending row is on record before its VMM exists": the precondition (stops at a precondition; the outcome assertions are first exercised after ADR-0353 lands) | ADR-0353, then RB-11 |
| `vm_scope_has_one_remover::a_vm_scope_is_removed_only_by_the_populated_first_scope_kill` | "a second remover of a VM scope exists": the offender is `overdrive-worker/src/cgroup_manager.rs` (`remove_workload_scope`) | ADR-0351: the method and its tests deleted |
| `vm_host_state_scope_kill::kill_scope_never_removes_a_scope_whose_last_task_is_held_in_exit` | "kill_scope returns ResourceBusy while the scope's last task is held in exit": it returns `Ok` and removes the scope under the held task (the spike's finding, reproduced on `7.0.0-38-generic`) | ADR-0349 |
| `vmm_launch_in_scope::a_created_vmm_is_a_member_of_its_scope_when_create_returns` | "the VMM is a member of its scope when create returns": it is in `serve`'s cgroup until the driver writes its pid | ADR-0352 |
| `vmm_launch_in_scope::{create_against_a_removed_scope_..., create_against_a_scope_with_subtree_control_set_...}` | "create against a scope ... fails": `create` succeeds and boots the VMM | ADR-0352 item 8 |
| `volume_survives.rs::a_workload_stop_during_a_held_claim_returns_promptly_with_no_failed_row` | GREEN at 09-03 (marker lifted): the bounded reap ends the predecessor row, so every assertion is reached, including scope kept while the flush is held, scope removed by the reclamation pass after the drain, and the restart that reattaches the volume with its data (which needs the claim released). Passed 3 of 3 on metal. No 09-04 obligation is observable at Tier 3 (the lease is a host-side value; its hold-until-scope-gone cells are the RB-2 Tier 1 tests) | ADR-0348 and ADR-0349 |

Guards added (pass before and after): `guard_a_stop_releases_the_lease_at_once_when_no_scope_remains`,
`guard_a_duplicate_start_dispatch_never_regresses_a_running_row_to_pending`, `guard_a_duplicate_restart_dispatch_never_regresses_the_successors_running_row`,
`guard_another_nodes_pending_row_is_left_alone`, `guard_a_non_busy_executor_error_still_refuses_the_boot`,
`guard_the_scan_sees_the_one_allowed_remover`, `guard_the_scan_flags_a_scope_removal_and_ignores_a_run_dir_removal_and_test_code`,
`guard_a_scope_that_empties_during_the_wait_returns_ok_before_the_bound`.

**Tests the crafter deletes with the production code** (ADR-0351, ADR-0352, P-6; none is deleted by this pass):
`overdrive-worker/tests/acceptance/cgroup_manager/{remove_workload_scope_idempotent,cgroup_kill_idempotent,cgroup_kill_writes_one_newline,place_pid_in_scope}.rs`
and their `mod` lines; `overdrive-worker/tests/integration/cgroup_manager/{remove_workload_scope_propagates_real_io_error,cgroup_kill_propagates_real_io_error}.rs`,
`tests/integration/real_cgroup_fs/{cgroup_kill_terminates_pids,refused_thread_kill}.rs` and the `mod` lines at `tests/integration.rs:37`, `:68`, `:69`
(`rmdir_auto_reap.rs` and `procs_pid_movement.rs` only name the methods in their rustdoc: reword, keep the tests); the two cleanup callers in
`overdrive-control-plane/tests/integration/cgroup_isolation/alloc_scope_has_writable_cpu_weight_and_memory_max.rs:192,233` move to `kill_scope` or
`tokio::fs::remove_dir`; `overdrive-worker/tests/acceptance/sim_cgroup_fs/write.rs::b_write_cgroup_kill_stores_one_newline` is a `CgroupFs::write` test and stays;
`overdrive-host/src/vmm.rs` tests `each_launch_filter_probe_cause_maps_to_its_typed_error` and `the_startup_probe_installs_the_exact_launch_program` (delete),
`launch_on_a_target_without_a_program_is_refused_before_any_effect` (delete its probe half, keep the `create` half),
`vmm_probe_preserves_stage_order_and_rejects_each_injected_ip_execution_failure` (drop `launch-seccomp` from its stage list), and the test double's
`check_launch_seccomp`. Tests the crafter must edit when the Pending write lands (they will fail, correctly, and are not weakened here):
`restart_arm_duplicate_vm_owner_is_a_no_row_success_that_still_cleans_the_predecessor` (§ N6 question 1) and the fail-closed
`assert_superseding_failed_row` A-4 assertion in `action_shim/mod.rs` (§ N6 question 2).

## Pass 3: the N3 tests, authored after the stuck-VM API compiled

Verified on metal (ordinary grammar). A PENDING test passes only while its body fails with its `expected` text; each text below was read from a real failing run
(`--no-capture` where the first reached failure needed to be confirmed). The full table of tests, files, tiers and placements is `test-scenarios.md` § N7.

| Test | Fails today on | Activation |
|---|---|---|
| `vm_driver_scope_single_remover::{a_reaped_stop_..., a_start_failure_cleanup_...}` | "a reaped stop (a start-failure cleanup) must remove the VM scope through VmHostState::kill_scope": the driver removes it through the cgroup port | ADR-0351 (09-02) |
| `vm_driver_scope_single_remover::the_driver_never_removes_a_scope_directory_through_the_cgroup_port` | "the VM driver removed the scope directory itself" | 09-02 |
| `vm_driver_scope_single_remover::{a_busy_scope_leaves_the_stop_outcome_..., a_busy_scope_leaves_the_reported_start_failure_unchanged}` | "the stop (the start-failure cleanup) must call VmHostState::kill_scope for the scope" | 09-02 |
| `vm_driver_scope_single_remover::the_driver_does_not_place_the_vmm_pid_in_its_scope` | "the driver must not write the VMM pid to the scope's cgroup.procs" | ADR-0352 (09-05) |
| `vm_host_state_equivalence::a_failed_removal_with_a_non_busy_kind_returns_at_once_without_retry` | "a removal failure that is not busy must return at once" (it retries for the 3 s settle bound) | ADR-0349 (09-02) |
| `vmm_equivalence::{a_created_vmm_is_a_member_of_its_scope_on_the_real_adapter, create_into_a_missing_scope_fails_on_the_real_adapter}` | "the VMM must be a member of config.cgroup_scope"; "create into a scope that does not exist must fail" | 09-05 |
| `vm_reclamation_boot::stuck_vm_boot::*` (13) | "a busy scope never refuses the boot" (`converge` returns `Host(ResourceBusy)`); the corrupt-row cases "a corrupt leftover row refuses the boot naming the allocation"; the sweep and several-VMs cases their named assertions ("every stuck scope's wait starts together", "one scope's failure never drops another scope's reservation") | ADR-0350, ADR-0354 (09-07) |
| `prelaunch_outcome_acceptance` RB-2 (a) to (d) (4) | "the unreaped VM's lease stays held Retiring while its scope is present"; "the lease is held until the VMM is reaped"; "a failing scope observation at the stop is the typed reclamation error"; "the lease stays held while the scope is held, attempt 1" | ADR-0156 amended (09-04) |
| `action_shim::install_failure_supersedes_running_with_failed_and_never_releases_the_gate` | "the superseding Failed row must carry the Running row's workload_addr" | ADR-0353 carry (09-06) |
| `launch_abandoned_count::*` (six count cells, the `is_natural_exit` Job test, the deletion-equivalence proptest) | the relaunch's count is taken from the newest row, not the newest row that ran, e.g. "an operator restart over a crash restart that never ran must carry the count of the launch that ran (0)"; "a lost launch with no operator action must charge the crash exactly once"; "a Job whose launch never ran must not be finalised Failed { exit_code: Some(0) }" | ADR-0355, RB-13 (09-08) |
| `launch_abandoned_count::the_pending_generation_verdict_agrees_...` | "generation_replacement_pending must read the generation from the newest row that ran" (the function is still a scaffold) | 06-01 |
| `vm_crash_mid_launch::a_serve_crash_just_after_the_vmm_is_spawned_leaves_nothing_outside_a_scope_boot_reclaims` | "the hypervisor's first act must run inside its allocation's cgroup scope .../alloc-crash-mid-launch-0.scope; it recorded [\"0::/overdrive.slice/control-plane.slice\"]": the process is born in `serve`'s slice and placed after the spawn | ADR-0352 (09-05) |
| `volume_survives::a_node_boots_past_a_vm_stuck_in_exit_left_over_by_a_serve_crash` | "a node must boot past a VM whose scope cannot be removed; boot refused: ... reclamation executor failed: VmHostState operation failed: Device or resource busy (os error 16)" | ADR-0350, ADR-0354 (09-07) |

Guards added (pass before and after): the seven `vm_driver_stop_unreaped::guard_*`, `vm_driver_stop_totality::stop_is_total_over_a_killed_not_reaped_termination`, the 15
`sim_stuck_vm_contract` tests, the pool tests of `pool_acceptance` (`reserve_leftover`, `CorruptLeftoverRow`, `Reserved`), `stuck_vm_boot::guard_*` (three),
`prelaunch_outcome_acceptance::restart_arm_duplicate_vm_owner_writes_no_row_of_its_own_...` (R9), the six `launch_abandoned_predicates` tests, the `last_ran_alloc` and
backend-candidate guards, the three `workload_describe_abandoned_launch` goldens, the two Job relaunch cells of `restart_generation_invariant.rs`.

**ADR-0348 Tier 1 (a) to (e) are guards, not pending tests.** The bound lives in the adapter and `SimVmm` (09-01) and `VmDriver::stop` already completes on
`KilledNotReaped` (it ignores the termination value), so the clauses hold today. The tests observe a `kill_scope` call through the scripted-failure queue of
`SimVmHostState` (it has no call recorder).

**Existing tests the crafter must rewrite when 09-02 lands** (they assert the opposite of ADR-0351): the start-failure tests in `vm_driver_stop_totality.rs` and
`vm_driver_start_failure_contract.rs` that read the scope's removal from `SimCgroupFs`.

## Pass 4: 09-02 review follow-up (the refused-thread coverage and the removed-mid-wait race)

Verified on metal (ordinary grammar, `integration-tests,kvm-tests`); each text was read from a real failing run (`--no-capture`).

| Test | Fails today on | Activation |
|---|---|---|
| `vm_host_state_refused_thread::a_kill_scope_under_thread_refusal_returns_instead_of_panicking` (`cgroup-default` guest) | "kill_scope returns instead of panicking when its blocking thread is refused": tokio panics inside `kill_scope` with "OS can't spawn worker thread: Resource temporarily unavailable (os error 11)" (`tokio-1.52.1/src/fs/mod.rs:317`), because the `cgroup.kill` write and the `rmdir` go through `tokio::fs` | the crafter's fix of `kill_scope`'s blocking-pool dependence (E23: under `panic = "abort"` this ends the node) |
| `vm_host_state_scope_kill::kill_scope_returns_ok_when_another_remover_deletes_the_scope_mid_wait` (`cgroup-default`, `volume-pool` guest) | "kill_scope returns Ok when another remover deletes the scope mid-wait, got Err(Kind(ResourceBusy))": the removal wakes no waiter on `cgroup.events` (`kernfs_drain_open_files`, `fs/kernfs/file.c:811 @ v7.0`, notifies no poller), so `kill_scope` waits out its 3 s bound and reports busy for a scope that is gone. The `ENODEV` read (`file.c:174,260`) is reached only if a wake arrives | the crafter's fix: an absent scope is `Ok`, however the wait ends |

`refuse_thread_creation` / `ThreadCreationRefused` (`overdrive-testing/src/pids_max.rs`) are kept: the refused-thread test reaches `kill_scope` under the fixture.
`failed_start_cleanup_twice_converges_to_the_same_residue_free_state` again asserts that a failed start and its cleanup touch no cgroup entry outside the sibling's
subtree beyond the target's own scope (whose leftover `cpu.weight` and `memory.max` entries are the same on both attempts).

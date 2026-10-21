# Pre-DELIVER fail-for-the-right-reason classification — netns-density-295 (EVMOD)

DISTILL integration pass of 2026-10-21 over the approved EVMOD design
(with the NO-OVERRIDE-SEAMS full scope, STORES-IN, PORTS-CONFIG, GUEST-PORTS,
FWD-PREREQ and PROBE-ROOT-ENV edits) (feature-delta
§ *[REF] Interface contracts*, the EVMOD section, § *Restart decision*
(RESTART-SSOT), Lifecycle Gate Ownership, Required boundary scenarios,
Formal protocol model) and `specs/quint/{guest-flow-owner,probe-count}/`.
DELIVER reads this file at each step's PREPARE: an activated body must fail
for the semantic reason recorded here (a RED scaffold of the step's own
production surface, or the assertion that pins the defect the step fixes),
never for an import, fixture or setup error.

## Latest integration pass

Lima (Linux `7.0.0-38-generic` aarch64) unless named; Quint record
`114403-1792606203.594` (279 checks). No new production surface.

| Command | Result |
|---|---|
| `cargo xtask lima run -- cargo check --workspace --all-targets` with `integration-tests`, and with `integration-tests,kvm-tests` | clean |
| `… cargo clippy --workspace --all-targets … -- -D warnings` with no features, `integration-tests`, and `integration-tests,kvm-tests` | clean |
| `cargo fmt --all --check`; `cargo xtask conformance-lint`; `cargo xtask quint verify-evidence` | clean; exit 0 (0 violations); 2 subsystems verified (`guest-flow-owner`, `probe-count`) |
| `xtask` (all tests, `integration-tests`: 325 of 325; 276 of 276 without the feature); `kmod_load` | pass: the decision core over a scripted kernel (any `EBUSY` on a live module is busy/retry) |
| `acceptance::dst_lint_banned_apis::dst_lint_exits_zero_on_the_real_overdrive_workspace` (DWD-09) | passes: no `--disk` or `--vsock` literal remains in `overdrive-testing/src` (the guest kit launches through `CloudHypervisorVmm`, U-4) |
| default lane `… nextest run --no-fail-fast -p overdrive-{control-plane,fwd,host,init,sim,testing,cli}` | 1,210 run: 1,169 passed, 41 failed, 476 skipped; `overdrive-cli` has no failure. All 41 are the FWD-PREREQ precondition in `overdrive-control-plane` (3 `acceptance` + 38 `integration`), each `forwarder_module.rs:225` "lane prerequisite missing: no ovd_fwd.ko built from this workspace"; no other failure (§ *FWD-PREREQ in the default lane*) |
| one `#[quint_test]` per driver, `--features integration-tests --run-ignored only` (12: CP `owner_flows`, `fwd_pair`, `flow_quota`, `udp_slots`, `quiescence`, `probe_count`, `cid_lease` x2 (`AssignThenVmmOpensItsOwnDevice`, `PoolExhaustedRestartCleansThePredecessorThenPlaces`); worker `flows_beacon`; init `owner_flows`, `fwd_pair`, `udp_slots`) | 12 run, 12 fail, each at the RED scaffold of the § *Driver activation* table (`net.rs:41`, `guest_transport/mod.rs:66/343/369`, `flow_owner.rs:130/540`, `owner_acceptance/support.rs:236`, `action_shim/mod.rs:1488`, `vm_driver.rs:1034`); the `flow_quota` driver (`open_model_flow` records production's verdict) fails at `support.rs:236` before its first `open_model_flow` |
| metal (`cargo xtask metal run --`, `7.0.0-38-generic` x86_64) | 76 cells run, 0 pass, each at its precondition or its RED scaffold (§ *Walking skeleton on metal*); the guest kit also needs `GuestCid::new` (step 2) and the leased-CID launch argument (step 3) once `guest-kit-prepare` works |

### FWD-PREREQ in the default lane

The `overdrive-control-plane` `acceptance` and `integration` binaries
compile into the default lane (`overdrive-sim` depends on the crate with
`integration-tests`). Their composed `serve` fixtures call
`overdrive_testing::forwarder_module::expect_forwarder_module` first, which
refuses "lane prerequisite missing: no ovd_fwd.ko built from this workspace
at …/target/kmod/7.0.0-38-generic/ovd_fwd.ko" until DELIVER step K lands
`cargo xtask kmod-build`. The 41 default-lane failures are all in
`overdrive-control-plane`; `overdrive-cli` has none in the default lane. The 41 failures (all of `job_stop_*` (3),
`backend_discovery_bridge::boot_composition` (8),
`built_in_ca_operator_composition::serve_persistent_ca` (7),
`concurrent_submit_toctou` (2), `describe_round_trip` (6),
`idempotent_resubmit` (5), `server_lifecycle` (5), `submit_round_trip` (5))
are this refusal and nothing else. This is a
precondition, not a semantic RED; routed to the user (`upstream-issues.md`
U-3).

## Driver activation (one named run each)

| Driver | Run | RED at |
|---|---|---|
| CP `quint_owner_flows` | `ControlSessionOpensAndCidActivates` | `guest_network/flow_owner.rs:130` (`flow_observation`) |
| CP `quint_fwd_pair` | `Paired` | `overdrive-core/src/net.rs:41` (`ListenPort::new`, step 2), then the owner scaffolds |
| CP `quint_flow_quota` | `AdmissionStopsAtTheAllocationQuota` | `owner_acceptance/support.rs:236`, "boot first: converge_shared binds the owner's flow and control listeners": `HostSharedGuestNetworkOwner::flow_ports` is a scaffold returning `None` until attachment-cut step (6) binds the listeners |
| CP `quint_udp_slots` | `DatagramAssociationClaimsPairsAndRoutes` | `support.rs:236`, as `flow_quota` |
| CP `quint_cid_lease` | `AssignThenVmmOpensItsOwnDevice` | `guest_network/flow_owner.rs:540` (`lease_observation`) |
| CP `quint_cid_lease` | `PoolExhaustedRestartCleansThePredecessorThenPlaces` (`lease_conf_three`) | `action_shim/mod.rs:1488`, the step-(6) scaffold for an owner that reports no flow ports (the driver reports it as "the tick did not panic") |
| CP `quint_quiescence` | `SingleRecoveryQuiescesRepairsRestores` | `net.rs:41` (step 2), then the owner scaffolds |
| CP `quint_probe_count` | `HappyPathThreeFailsTerminateAtThreshold` | `overdrive-core/src/guest_transport/mod.rs:66` (`GuestCid::new`, step 2) |
| worker `quint_flows_beacon` | `BeaconReadyReleasedThenDriverStopReleasesClaim` | `overdrive-worker/src/vm_driver.rs:1034` (`beacon_observation`) |
| init `quint_owner_flows` | `ControlSessionOpensAndCidActivates` | `guest_transport/mod.rs:343` (`LaunchToken`, step 2), in driver construction |
| init `quint_fwd_pair` | `Paired` | `guest_transport/mod.rs:369` (`LaunchToken`, step 2), in driver construction |
| init `quint_udp_slots` | `DatagramAssociationClaimsPairsAndRoutes` | `guest_transport/mod.rs:369`, in driver construction |

The eight control-plane and worker drivers panic after quint-connect's
`Replaying traces …`: the driver was built, the trace generated, and the
initial action dispatched. The three init drivers now panic while the driver
is constructed, before the trace is replayed: under the lint ruling "Owner
field" (S-7, closed) each holds `GuestFlowOwner` directly, and its
construction draws a `LaunchToken` (the driver's `token()` helper, backtrace
`OwnerFlowsGuestDriver::new` → `token`), a step-2 scaffold. This is a
production RED, not a fixture error; once step 2 lands each reaches
`GuestFlowOwner`'s step-5 scaffolds (steps are ordered, step 2 first). The
`cid_lease` run `PoolExhaustedRestartCleansThePredecessorThenPlaces` targets
the declared main `lease_conf_three` (D-18 closed) and carries the
`attachment-cut step (15)` marker like the rest of the driver. Every driver step that fails before its
own pending step is a scaffold of an earlier step (2, (6)), so each turns
into the driver's own step's RED as the steps land in order.

## RESTART-SSOT (DELIVER step 0, GH #324)

| Obligation | Body | RED reason |
|---|---|---|
| 1 ceiling boundary (accessor) | `reconciler_runtime::tests::restart_status_matches_the_reconciler_at_the_ceiling_boundary` (replaces `restart_status_flips_at_ceiling_boundary`, which pinned the defect) | `restart_status_for_alloc` scaffold |
| 2 platform-reclaim exemption | `…::restart_status_exempts_a_platform_reclaimed_row_past_the_ceiling` | scaffold |
| 3 generation exemption | `…::restart_status_exempts_a_generation_replacement_past_the_ceiling` | scaffold |
| 4 agreement property (proptest, every kind × row class × attempts × generation × intent × lineage) | `overdrive-reconcilers` `workload_lifecycle::restart_ssot_agreement::restart_after_failure_agrees_with_one_reconcile` | `restart_after_failure` scaffold |
| 5 Job stream | `overdrive-control-plane` acceptance `streaming_submit::a_job_crash_streams_attempt_failed_that_will_not_restart` | `will_restart` is `true` (the defect) |

The integration tests `streaming_attempt_failed::{attempt_failed_first_attempt_of_a_job_will_not_restart,
attempt_failed_mid_budget_reports_correct_index,
attempt_failed_empty_view_reports_the_first_attempt_that_will_not_restart}`
pinned the pre-fix `will_restart: true` for a Job crash; they now pin
`false` with no delay and are pending step 0 (RED on that assertion). The
Job stream keeps today's value until step 0 (`streaming.rs`, a
`// SCAFFOLD` comment), and its three index-only callers use
`attempt_index_for_alloc`.

## Walking skeleton on metal

Metal `7.0.0-38-generic` x86_64, 76 cells, run through `cargo xtask metal run`
(canonical lease `/run/lock/overdrive-metal-shared.lock`, released on exit;
BPF object rebuilt with `OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build`).
`cargo xtask guest-kit-prepare` fails at `kmod-build` ("unrecognized
subcommand", step K). The walking skeleton, both density cells and the five
fault-matrix cells stop at `vsock_support.rs:84` (`LaneNotPrepared`,
`ovd_fwd.ko` for the host kernel); the five `guest_flow_through_serve` cells
(two D-17, three U-D17-DIAL) at `guest_flow_through_serve.rs:107` (the same
precondition); the four `overdrive-fwd` `unload` cells at `unload.rs:128`
(FWD-PREREQ); the two `unload_in_guest` host cases and the eleven
`in_guest_lane` cases (six D-22) at `LaneNotPrepared`; the two in-guest
`unload_in_guest::in_guest_*` cells refuse at `unload_in_guest.rs:74` ("not
inside a scripted guest-kit guest" — they fail closed on a host by design);
the `attach_watch` (20), `datagram` (15) and `paused_and_stall` (9) cells at
`KernelFlowForwarder::open`'s RED scaffold (`overdrive-fwd/src/lib.rs:84`,
step 4), one `datagram` cell first at the wire codec's scaffold
(`guest_transport/wire.rs:505`, step 2). This is the step-K dependency, not
the cells' own RED; DELIVER step K's PREPARE re-runs these cells after
`guest-kit-prepare` to confirm they then reach the attachment-cut scaffolds.

## S-1 — boot-refusal cells against the no-effect stubs

Run once against the no-effect `probe_startup` / `probe_guest_flow_startup`
stubs (`upstream-issues.md` S-1), so DELIVER does not inherit a cell that is
green by accident:

| Cell | Against the stubs |
|---|---|
| `probe_startup_refuses_a_vip_range_iff_one_of_its_ranges_meets_the_guest_prefix` (proptest) | fails on its first case (`successes: 0`, minimal input `ranges = [100.0.0.0/8]`: refusal expected, `Ok` returned) — the overlapping half of the generator is reached |
| `a_vip_range_overlapping_the_prefix_refuses_before_any_boot_effect` | fails: "probe_startup refuses: ()" |
| `phase_one_probes_the_forwarder_on_a_context_it_releases_and_refuses_a_bad_one` | fails: "BootSeam DeviceAbsent: phase 1 refuses with the forwarder's cause, got Ok(())" |
| `an_owner_outside_the_connect4_attach_path_refuses_before_any_boot_effect`, `an_rlimit_nofile_below_the_owner_budget_refuses_before_any_boot_effect` | fail on their refusal assertions (`OwnerCgroupPlacement`, `FdBudget`) |
| `probe_startup_refuses_a_node_address_iff_one_lies_inside_the_guest_prefix` (proptest), `a_node_address_inside_the_prefix_refuses_before_any_boot_effect`, `a_node_that_overlaps_the_prefix_by_address_and_by_vip_range_names_the_vip_range` | stop first at `InterfaceIndex::new` (`net.rs:53`, step 2); after step 2 they reach the stub (the proptest draws half its addresses inside the prefix) |
| `node_addresses_outside_the_prefix_pass_probe_startup_without_effect` | stops at `InterfaceIndex::new` (step 2); it is a pass case, so after step 2 it is GREEN against the stub — not evidence until step 13 lands the checks |

## Cells pending their steps

| Cell | RED at |
|---|---|
| `owner_acceptance::legs_dial` (4: authenticated dial and teardown release, no dial on a refused client, `GuestDial { NotServing }` closes leg C, no owner dial outside the prefix) — attachment-cut (9) | `net.rs:41` (`ListenPort::new`, step 2) in the lease set-up, then the owner and enforcement scaffolds |
| `overdrive-worker` `vsock_leg_f_repair::leg_f_repair_rebinds_transparent_listener` (Tier 3, root) — 14-02 | `mtls_intercept_worker.rs:3133` (`leg_f_addr`, step 8) |
| `prefix_boot_acceptance::phase_one_probes_the_forwarder_on_a_context_it_releases_and_refuses_a_bad_one` — (12-01 rework) | its refusal assertion against the phase-1 stub (§ *S-1*) |
| `shared_network_recovery_acceptance::a_forwarder_that_fails_its_boot_probe_refuses_boot_before_any_effect` (G-V1 through the composed rig's phase 1, which now calls `run_server`'s own boot-step-1 function and its `health.startup.refused`) — (12-01 rework) | "AbiMismatch: the boot refuses with the forwarder's typed cause, got Ok(())" (the first scripted failure) |
| `shared_network_recovery_acceptance::a_boot_opens_and_probes_the_forwarder_before_any_listener_or_route` — (12-01 rework) | "the forwarder opened" (`converge_shared` is a no-effect stub, S-1) |
| `overdrive-fwd` `end_record_abi::take_end_returns_records_in_end_order` (abort and clean-close paths) — step 4 | not run (needs `ovd_fwd`, step 4); `KernelFlowForwarder::open` is its first scaffold |
| `vsock_density` R5-25 fail-closed oracle (S-10); `vsock_fault_matrix` R5-62 plateau polling | the metal precondition above |

## Semantic RED by marker

Counts are `#[ignore = "pending …"]` attributes in the tree on 2026-10-21 (1,061 in all).

| Marker (count) | RED reason when activated |
|---|---|
| `step 0` (8) | RESTART-SSOT, above |
| `step 1` (3) | the fork without RESET-EXIT: a guest PID-1 crash or triple fault reboots in place |
| `step 2` (119, including the "GuestCid / LaunchToken newtypes" and "core types and wire codec" groups) | the core forwarder / wire / dial / guest-transport scaffolds; the two deploy-ingress checks (`ListenerPortReserved`, `ProbeCountExceeded`) are not wired |
| `step 3` (8) | the VMM launch arguments with the leased CID |
| `step 4` (131) | `KernelFlowForwarder` (every `overdrive-fwd` case, `open` first; the `guest_boundary` kvm cells) and `SimFlowForwarder` / `SimSocket` |
| `step 5` (187) | the guest owner, transport setup and `KernelRelease` scaffolds; the guest drivers; the in-guest lane cells |
| `step 8` (13), `step 11` (7), `step 12` (20), `step 13` (15), `step 14` (13) | the mTLS registration, VIP placement, firewall single cut, guest-prefix boot checks and host-internal deny set scaffolds |
| `step 9` (35) | the probe-run counting and VM probe runner dial |
| `step 14-01` (6), `step 14-02` (6, the leg-F repair cell included), `step 14-05` (2) | the vsock serve + deploy lanes on metal (precondition until step 4, above; then the attachment cut) |
| `attachment-cut step (3)`, `(5)`, `(6)`, `(7)`, `(8)`, `(9)`, `(15)` (4, 3, 63, 166, 3, 48, 68) | the owner flows, the beacon relocation and the bound-port listeners (PORTS-CONFIG: `NodeVsockPortConfig::new` validation, `flow_ports`, `BeaconIntake::bound_port` on the kernel command line, `serve_kernel_assigned_vsock_ports`), the host dial and LEGS-DIAL relay, the pool and reclamation scaffolds; the R5-59 (a) accept-path cells (`(6)` beacon seam, `(7)` flow seam) |
| `attachment-cut step (12-01 rework)` (27), `(12-01 rework)` (6) | the supervisor's QHOLD recovery and the activation gate |
| `attachment-cut step (DG-2 …)` (55 + 1 + 14 + 13 + 4 + 3 + 2 + 1) | a VM start leasing the guest transport and taking READY from the node beacon intake; the guest lane leasing the transport; the IpRules recovery |
| `step 14-03` (1), `step 12-01` (1) | carried composition cells |
| `step K` (1) | V-EV-UNLOAD's context half (`unload.rs`): the module skeleton and its references do not exist before step K |
| `#325` (1) | `RedeployLeasedAtServeCrashIsPlacedAgain` on `lease_conf_325` (GH #325 is not fixed in #295) |

Expected GREEN on activation (classify as such, not RED): the four
`overdrive-sim` `adapters/vmm.rs` device cells marked `(3)`. The three
`overdrive-worker` acceptance cells
`vsock_beacon_intake::{binding_the_intake_binds_the_beacon_listener,
a_bind_failure_is_typed_and_binds_nothing, the_bind_refusal_renders_its_pinned_message}`
are live in the default lane (their subject, `BeaconIntake::bind(io, port)`
and `bound_port`, is complete) and pass.
The other `vsock_beacon_intake` cells RED at `LaunchToken` / `GuestCid`
(step 2), then `VmDriver::beacon_observation`. `e10_vm_early_exit_spike` and
`vm_finalize_failed_ownership_spike` (`(15)`) get past `BeaconIntake::bind`
and RED at `ListenPort::new` (step 2), then at `GuestAddressPool::assign`.
`node_addresses_outside_the_prefix_pass_probe_startup_without_effect` turns
GREEN against the phase-1 stub once step 2 lands (§ *S-1*).

## Serve tests: un-marked and kept pending

The crate-private composition `compose_guest_network` (D-UP-16) made
`run_server` boot without a VM attachment. Of the 135 serve-booting tests
the first DISTILL pass had re-marked, 77 across 25 files were un-marked;
75 pass and two are re-marked:

- `overdrive-control-plane` acceptance
  `service_kind_vm_workloads::one_server_boot_shares_exactly_one_trusted_probe_runner_with_vm_driver`
  — `pending DELIVER step 2 (GuestCid / LaunchToken newtypes)`: the fixture
  builds a `GuestCid`.
- `overdrive-cli` integration
  `service_submit_streaming_cli_dispatch::cli_submit_streaming_service_routes_to_service_submit_event_consumer`
  — `pending DELIVER attachment-cut step (DG-2 rows 2, 15 …)`: the Service's
  listener is a step-2 `ListenPort` (its scaffold panics in the server's
  submit path) and the VM start then needs the attachment cut; it timed out
  at 120 s waiting for the stream.

Ten un-marked tests deploy an allocation but assert only on the deploy
acknowledgement or intent; the VM start's RED panic lands in the
background convergence task, which shutdown discards.

The 58 kept pending, each `pending DELIVER attachment-cut step (DG-2 rows
2, 15: a VM start leases the guest transport and takes READY from the node
beacon intake)` except the three `serve_lifetime_fail_stop` cells (`DG-2
row 8, the 12-01 rework`), all driving a VM attachment:

- `overdrive-cli/tests/integration/vm_resources_sizing.rs`: `declared_cpu_milli_2000_yields_two_guest_online_cpus`, `sub_core_cpu_milli_250_yields_one_guest_online_cpu_and_reaches_running`, `declared_memory_2gib_is_guest_observed_and_reported_by_describe`, `declared_cpu_and_memory_match_guest_observed_on_single_private_backing`
- `overdrive-cli/tests/integration/vm_reclamation_tier3.rs`: `vm_survivor_with_no_vm_registry_entry_is_reclaimed_via_observed_empty`, `boot_epoch_reclamation_settles_before_adopt_on_restart_recovery`, `steady_state_sweep_reclaims_a_stranded_scope_without_restarting_serve`, `steady_state_sweep_kills_a_surviving_vmm_at_a_later_tick`, `restart_orphan_terminal_row_is_byte_unchanged_after_reclamation`, `failed_stop_orphan_terminal_row_is_byte_unchanged_after_reclamation`, `reclaiming_an_svid_holding_allocation_submits_the_fourth_evaluation`, `reclaim_then_fresh_start_retains_predecessor_and_resets_history`, `guest_self_exit_without_stop_leaves_no_rootfs_clone_in_operator_dir`, `hypervisor_death_without_stop_leaves_no_rootfs_clone_in_operator_dir`, `serve_restart_losing_in_memory_rootfs_plan_leaves_no_rootfs_clone`
- `overdrive-cli/tests/integration/guest_stack_mtls_egress.rs`: `the_same_guest_reaches_a_non_mesh_destination_in_the_clear`, `the_operator_sees_the_microvm_workloads_own_mesh_address_not_its_transit_hop`, `operator_exit_78_after_ready_is_an_ordinary_result`
- `overdrive-cli/tests/integration/serve_lifetime_fail_stop.rs`: `a_shared_network_fail_stop_wins_over_a_ready_interrupt_and_exits_status_one`, `a_shared_network_fail_stop_shutdown_is_abandoned_when_the_ten_second_bound_elapses`, `a_repaired_shared_network_loss_never_ends_serve`
- `overdrive-cli/tests/integration/vm_stop_restart_and_vmm_death.rs`: `guest_exit_without_agent_report_is_unreported_crash_never_completed`, `host_killed_hypervisor_is_a_crash_treated_like_a_crashed_process`, `agent_reported_exit_zero_reaches_the_completed_terminal`, `operator_stop_is_terminated_and_consumes_no_restart_budget`, `stopping_a_vm_reaches_the_operator_stop_terminal_like_a_process`, `unresponsive_guest_is_stopped_within_bounded_grace_never_a_crash`, `restarted_vm_boots_from_a_clean_unmodified_rootfs_copy`, `predecessor_cleanup_cannot_bind_or_remove_replacement_vm_artifacts`, `hypervisor_runs_bounded_nonroot_and_landlock_confined`, `confinement_ruleset_follows_declared_rootfs_path_not_a_hardcoded_dir`, `vm_landlock_rules_are_exactly_the_exclusive_run_dir_write`, `confined_deploy_leaves_operator_kernel_and_rootfs_mode_and_bytes_unchanged`, `host_that_cannot_confine_refuses_the_workload_and_never_starts_unconfined`, `confinement_adds_no_new_operator_surface`, `guest_supervisor_reaps_its_command_group_and_preserves_direct_child_status`, `guest_control_stream_errors_keep_bounded_teardown_and_original_error`, `native_lifecycle_profiles_meet_stage_targets_without_dropping_trials`
- `overdrive-cli/tests/integration/vm_walking_skeleton.rs`: `vm_workload_runs_to_completion_and_exit_code_reaches_operator`, `vm_non_zero_guest_exit_code_is_reported_not_the_hypervisors`, `vm_guest_that_never_starts_is_never_reported_running`, `vm_workload_deploys_through_the_same_verb_as_a_process_workload`, `vm_platform_contains_the_hypervisor_it_started`, `every_cloud_hypervisor_thread_carries_the_launch_filter_under_its_own_filters`, `vm_deadline_arm_leaks_nothing`, `vm_guest_exit_report_is_never_overwritten_by_vmm_teardown_exit`, `vm_absent_boots_node_with_no_vm_entry_and_classifies_deploy_naming_capability`, `vm_that_exceeds_declared_memory_is_diagnosed_as_oom_not_bare_signal_9`, `two_vm_jobs_on_one_serve_each_boot_from_the_rootfs_their_own_spec_named`
- `overdrive-cli/tests/integration/vm_boot_failure_vocabulary.rs`: `missing_configured_rootfs_is_named_precisely_and_leaks_no_vm_resources`, `kernel_deleted_after_composition_is_named_precisely_and_spawns_no_hypervisor`, `hypervisor_removed_after_composition_names_every_searched_path`, `guest_that_never_beacons_reports_the_boot_deadline_and_console_tail`, `kernel_replaced_with_an_incompatible_image_reads_as_a_format_problem`, `unmapped_start_failure_reads_as_unclassified_and_preserves_its_verbatim_cause`, `job_plus_vm_spec_is_accepted_and_its_allocation_reaches_running`, `absent_vm_capability_names_what_is_missing_not_an_internal_platform_error`, `unservable_rootfs_directory_names_the_directory_and_the_clone_requirement`

## Kmod, kit-lane and in-guest cells

| Cell | Expected classification | Reason |
|---|---|---|
| `xtask::kmod_load::tests::*` | GREEN | the decision core over a scripted kernel (any `EBUSY` on a live module is busy/retry) |
| `overdrive_testing::forwarder_module::tests::*` (5) | GREEN | D-20 `version` parsing and verdict |
| `required_serve_ports_source_scan::the_file_wide_construction_rule_sees_helpers_unit_structs_and_defaults` | GREEN | planted sources |
| `required_serve_ports_source_scan::no_optional_switch_gates_protection_dns_or_supervisor_composition` | GREEN (the real tree holds no host construction in `src/lib.rs`) | the file-wide rule's real-tree run |
| `overdrive-fwd` `unload::*` | precondition (FWD-PREREQ, until step K); then RED `MISSING_FUNCTIONALITY` (`pending DELIVER step K` / `step 4`) | the module skeleton and its references do not exist |
| `overdrive-fwd` `paused_and_stall`, `datagram`, `attach_watch` | RED at `KernelFlowForwarder::open`'s scaffold (`pending DELIVER step 4`) | the adapter and module contract land in step 4 |
| `overdrive-fwd` `unload_in_guest::*` (2 host) | precondition (`LaneNotPrepared` until step K, then the in-guest cell's `KernelFlowForwarder::open` scaffold, step 4); the stress case's oracles are no oops and the refcount on the stock 7.0 kernel; the ULP control polls each transition (`wait_refcnt`) and holds over `ULP_HOLD_WINDOW` only after its positive half, the stress's far ends are non-blocking | in-guest cells of the `integration` binary, staged by `guest-kit-prepare` |
| `overdrive-init` `in_guest_lane` D-22 cases (6) | precondition (`LaneNotPrepared` until step K), then the in-guest owner cell's first scaffold (`GuestFlowOwner` construction / `dispatch_next`, `pending DELIVER step 5`); the aborted-flow checks run only after the owner has processed the aborted V_g (`serve_control_flow_after_abort`) and the R5-28 guest-accepted cell pins `consumed_ends` | the production guest owner lands in step 5 |
| `guest_flow_through_serve::*` (D-17 and U-D17-DIAL) | precondition on metal until step K, then RED at `ServerHandle::guest_flow_observation`'s scaffold (`pending DELIVER step 14-01`) | the accessor is a RED scaffold |
| `overdrive-testing` `guest_kit::GuestKit::boot` (U-4 (a)) | compiles against the production `CloudHypervisorVmm`; on metal each kit cell stops at `LaneNotPrepared` until step K, then at `GuestCid::new` (step 2, `cid_bands` and the kit's launch) and the leased-CID launch argument (step 3) | the kit launches through `Vmm::create`; the production launch scaffolds land in steps 2 and 3 |
| `vsock_host_internal_enforce` F-3, F-5, link-local | `pending DELIVER step 14` | controls precede the absence checks |

## H3 test ownership: the persistent-TAP creator

Roadmap step 12-01 deletes the production netlink persistent-TAP functions (`create_persistent_tap`, `set_persistent_tap_owner`, `observe_persistent_tap*`, `TapLinkState` and the decoders). DISTILL re-homed or removed every retained test that called them before 12-01 starts, under `.claude/rules/testing.md` and `development.md` § Deletion discipline. No production API was added.

| Test or fixture | Decision | Reason |
|---|---|---|
| `overdrive-host/src/vmm.rs` `ScratchTap` and every `launch_seccomp` cell that uses it (S-ND295-43 `every_denied_request_returns_eperm_on_every_thread_under_the_production_hook`, `every_filtered_thread_reports_no_new_privs_and_filter_mode`, `foreign_syscall_abis_end_the_filtered_process`, S-ND295-41 descriptor cells) | Keep, re-homed | They defend a retained contract: the ADR-0143 launch filter refuses TAP-mutating requests on every thread (V-1(c); the design retains it, roadmap 11-04). `ScratchTap::create` now calls a test-local `create_persistent_scratch_tap` in the same test module (raw `/dev/net/tun`: `TUNSETIFF` with `IFF_TAP | IFF_NO_PI`, `TUNSETOFFLOAD` 0, `TUNSETOWNER` 0, `TUNSETPERSIST` 1; the TAP the old function made). Test bodies and assertions are untouched. Cleanup still uses `Client::del_link`, which is not in the deletion inventory |
| `alloc_netns_lifecycle.rs` `terminal_teardown_reaps_platform_tap_stranded_before_netns_move` | Deleted now | Defended only `teardown_workload_netns` reaping a host-stranded platform-owned persistent TAP, i.e. the bridge-era per-VM TAP plan (`derive_vm_tap_plan`, `host_persistent_tap_state`) that 12-01 deletes. No retained contract |
| `alloc_netns_lifecycle.rs` `terminal_teardown_never_deletes_a_foreign_host_tap` | Deleted now | Defended only the `TapIncompatible` fail-closed collision on that deleted TAP path. No retained contract |
| `alloc_netns_lifecycle.rs` `HostTapGuard`, `host_persistent_tap_present`, the `create_persistent_tap` and `VethProvisionError` imports | Deleted now | Used only by the two deleted tests |

Callers that remain when 12-01 starts, all inventoried deletions in the feature delta (§ bridge / TAP era A and B) and deleted with the functions in the same commit: `veth_provisioner.rs` persistent-TAP helpers; `overdrive-netlink/tests/integration/{scratch_tap,managed_link_address,tap_debug_msg_mask}.rs` (whole files); the `alloc_netns_lifecycle.rs` tests that call `derive_vm_tap_plan` / `VmTapPlan` (deleted with the plan: roadmap 12-01 "DELETE with tests"). The dataplane `guest_tcx_*` fixtures create TAPs with `ip tuntap` and never called the netlink functions.

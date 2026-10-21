# P3b — Lease, quiescence, recovery and boot (netns-density-295, EVMOD)

Owners: `GuestAddressPool` and `vm_reclamation_boot::converge`
(`overdrive-control-plane`), the shared-network supervisor and the host owner's
`quiesce` / `restore` / `converge_shared` / `probe_startup` (QHOLD,
ADR-0169, ADR-0184), `run_server_with_obs_and_drivers` (the composition
root), the activation sequencer (G-V3), and the DST invariants in
`overdrive-sim`. Composed rigs boot the production composition with every
driven port a sim (architecture of reference); the supervisor is stepped at
the DNS port's audit (`TestGuestDnsFactory::hold_audits` /
`release_audit`), because `FlowForwarder::inventory()` is synchronous
(EVMOD-PORT) and the seam's `Inventory` effect is never held; repairs still
park at the seam's `BindListener` / `OpenForwarder`.

Markers: `attachment-cut step (15)` pool and reclamation, `(12-01 rework)`
the supervisor, `step 12` the firewall single cut / ADR-0184 boot write,
`step 13` guest-prefix boot checks, `(12-01 rework)` the activation gate,
`(7)` owner flows, `(3)` the `SimVmm` device cells.

## Conformance drivers

| Driver | Spec · main | Named runs + `#[quint_run]` | Projection | Pending |
|---|---|---|---|---|
| `guest_network::quint_cid_lease` | `cid_lease.qnt` · `lease_conf_ok` | 11 + 1: `AssignThenVmmOpensItsOwnDevice`, `ReleaseWhileTheVmmRunsIsRefused`, `StopExitsTheVmmThenReleaseIsLast`, `NextFitContinuesFromTheCursor`, `GuestResetEndsTheVmmAndPlacementRetries`, `VmmCrashFreesItsCidAndPlacementRetries`, `ServeCrashOrphanIsReclaimedBeforeAnyAssign`, `ReclamationReleasesTheOrphanDeviceThenAssignProceeds`, `ReclamationKillsEveryOrphanBeforeAssign` (D-UP-19), `OrphanExitsByItselfBeforeReclamation`, `ServeCrashWithUnlaunchedLeaseLeavesNothingToReclaim` | QPROJ-2 `GuestAddressPool::lease_observation()`; `dev` ← `SimVmm::vsock_device_holders` (marked arbiter; Tier-3 pair V-30 Q2a / Q2c / Q2d, V-RECLAIM) | (15) |
| `quint_quiescence` | `quiescence.qnt` · `quiesce_conf` | 10 + 1: `SingleRecovery…`, `TwoOverlappingRecoveriesKeep…`, `NoFlowOpensWhileCountedHoldRemains`, `TwoOverlappingRecoveriesReopenOnlyAtZero`, `DroppedHoldKeepsForwardingQuiescedThenFailStops` (`LeakHold` = composed owner `quiesce()` + drop, D-UP-3), `RepairFailureCountsAttemptThenRepairs`, `PostRepairAuditFailureCountsAttempt`, `SpentBudgetFailStopsAndRestartConverges`, `CrashDuringRecoveryThenRestartRecovers`, `FailedBootTableWriteRefusesBootThenRetrySucceeds` (step 12). `ForeignHoldRestoreChangesNothingThenFailStops` is checked by the model and the FHOLD-CHECK unit cell, not replayed (FHOLD-CHECK ruling) | QPROJ-3 `flow_observation()` + QPROJ-6 `recovery_observation()`; `table` ← `SimMtlsIntercept::shared_table_state` (marked arbiter; Tier-3 pair R5-30); `ForeignRestore` = restore of a second `QuiescenceLedger`'s hold | (12-01 rework), step 12 |

## Example and seeded cells

| Where | Scenarios | Pending |
|---|---|---|
| `src/shared_network_recovery_acceptance.rs` | P6-S1; R5-32 two recoveries (`StillQuiesced`, dial `Quiesced`) — `overlapping_recoveries_reopen_only_when_the_last_hold_is_restored`; R5-32 leaked hold fail-stop — `a_leaked_hold_keeps_forwarding_quiesced_until_the_supervisor_fail_stops`; R5-32 / D-UP-4 foreign hold — `a_foreign_hold_changes_nothing_at_the_owner_and_leaks_at_its_issuing_ledger`; FHOLD-CHECK — `a_restore_answered_foreign_hold_fail_stops_with_foreign_quiescence_hold`; R5-58 (seeded) — `the_audit_detects_every_component_exit_within_one_audit_period`, `each_failed_attempt_counts_once_and_every_episode_ends_by_restore_or_fail_stop`; R5-61 (seeded) — `the_forwarder_recovery_aborts_every_flow_closes_the_old_context_then_reopens`; G-V1 — `a_forwarder_that_fails_its_boot_probe_refuses_boot_before_any_effect`, `a_boot_opens_and_probes_the_forwarder_before_any_listener_or_route`; R5-11 — `the_gateway_route_is_converged_once_per_boot_and_never_at_runtime`; R5-30 — `a_failed_boot_table_write_refuses_boot_and_the_retry_converges`, `the_shared_table_is_absent_or_complete_across_crash_and_failed_write`; P6-S4 / S5 / S6 | (12-01 rework); R5-30 step 12 |
| `src/lib.rs` `shared_network_task_owner_acceptance` (kept bodies restored, default lane, active) | `actual_tokio_exit_matrix_fail_stops_before_returning_the_exact_snapshot`, `explicit_request_is_returned_unchanged_and_intentional_shutdown_is_not_failure`, `dns_task_owner_classifies_real_exits_and_never_overwrites_a_live_handle`, `dns_replacement_joins_the_old_task_before_spawning_from_live_and_exited_states`, `dns_shutdown_prefers_cooperative_stop_and_awaits_the_bounded_abort_backstop`, `dns_replacement_is_refused_from_every_invalid_state_without_spawning` | active |
| `src/guest_network/prefix_boot_acceptance.rs` | D8-PREFIX-VIP / D8-PREFIX-CFG cells + proptests (step 13); R5-40 proptest + no-effect cell; D24 placement (step 11); the fd budget ((7)); `phase_one_probes_the_forwarder_on_a_context_it_releases_and_refuses_a_bad_one` (G-V1 phase 1, D-8, through `probe_guest_flow_startup` and the owner, (12-01 rework)) | as listed |
| `src/action_shim/guest_activation_acceptance.rs` | G-V3 `QuiescenceLatched` + `ControlSessionPending` then activate; activate after teardown refused (`PostconditionMismatch`, owner contract); feature disabled — substitute transport `None` ((12-01 rework)); pre-READY exit ((15)); VMM death / stale launch ((7)) | as listed |
| `src/guest_network/pool_acceptance.rs` | R5-55a next-fit and unique offset-derived CIDs (proptest); launch tokens from the injected entropy; retirement monotonic; G-V0 `AdmissionCapReached` (proptest); `PoolExhausted` typed; NO-SURVIVE | (15) |
| `src/vm_reclamation_boot.rs` | R5-55d reclamation before the first assign; one converge kills every orphan; a launch on an orphan's CID fails at device setup; a supervised VMM untouched | (15) |
| `overdrive-sim` `invariants/netns_density_boot_order.rs` | G-V1 order re-derived: sweep ≺ stale-member clear ≺ observe ≺ converge(prior) ≺ zero-member read-back ≺ owner `converge_shared` (proptest `reclamation_completes_before_stale_shared_network_sweep_for_every_seeded_prior_vm`) | step 12 |
| `overdrive-sim` `adapters/vmm.rs` device cells | four `SimVmm` device tests (already-implemented sim semantics; may be GREEN on activation — classify GREEN) | (3) |
| `overdrive-sim` spikes `driver_neutral_allocation_replacement`, `e10_vm_early_exit_spike`, `vm_finalize_failed_ownership_spike` | re-derived: no VM workload netns (D-UP-15); e10 and vm_finalize on the guest lane (lease-backed READY token) | (7), (15) |
| `overdrive-sim` acceptance `netns_density_activation_order`, `netns_density_retiring_cleanup`; integration `netns_density_node_admission`, `netns_density_reclaim` | the activation order against the EXEC gate; retiring-lease cleanup; node-wide admission under the cap (S-ND295-05D + G-V0); reclaim | as marked |
| `overdrive-control-plane/tests/integration/{shared_guest_network_startup, shared_network_supervisor_recovery, guest_attachment_pool_per_server}.rs`, `tests/acceptance/netns_density_guest_network.rs`, `tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs` | boot order and refusals through `run_server`; supervisor recovery; per-server pool; fail-stop recovery through the exported server handler | as marked |

## Deleted

`tests/conformance/src/prefix_landing.rs` and
`tests/vsock_prefix_landing_conformance.rs` (spec deleted, QPROJ-8), with the
conformance crate's now-unused dependencies (`overdrive-dataplane`,
`overdrive-host`, `overdrive-netlink`, `overdrive-testing`, `rustls`,
`ipnet`); `guest_network/prefix_boot_conformance.rs`.

## Not authored / notes (`upstream-issues.md`)

`quint_cid_lease` is rewritten (DR2-CLOSE): the production
`WorkloadLifecycle` in a `ReconcilerRuntime`, driven only by the convergence
tick and the real `submit_workload` / `stop_workload` / `restart_workload`
handlers; a substituted provisioner holds `provision` / `teardown` at the
step boundaries; the production `VmDriver` over `SimVmm` with READY through
`SimBeaconIo`; the production exit observer; boot reclamation `converge`; a
`serve` crash drops its runtime while the node's persistent state survives
(S-4). It projects the pool (`lease_observation`) and the reconciler's
decision (`restart_status_for_alloc`, `attempt_index_for_alloc`,
`current_alloc`; RESTART-SSOT, step 0); all 21 `lease_conf_ok` runs plus
`RedeployLeasedAtServeCrashIsPlacedAgain` on `lease_conf_325` (`pending
#325`), and `PoolExhaustedRestartCleansThePredecessorThenPlaces` on the
three-offset main `lease_conf_three` (two background workloads take the
first two `/29` offsets; `AssignRefused` maps to the `RestartAllocation`
dispatch that gets `PoolExhausted` and runs `finish_restart`), pending that
spec main (`upstream-issues.md` D-18). It reads `leased` from the owner
(Admitted ∧ no VMM holds the CID's device ∧ no row; S-4) and treats `up` /
`reclaimed` as stimulus. Notes: the composed rig's pacing point (T-4); the
composed rig boots phase 1 then `boot_server` with
`GuestNetworkSource::Composed` (D-8); the
`VmProvisionFailureCleansNetworkAndReusesSlot` DST invariant goes with
step 7 (O-3). RESTART-SSOT's regression obligations 1–5 are in
`red-classification.md` § *RESTART-SSOT*.

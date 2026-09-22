# Pre-DELIVER RED classification — `netns-density-295`

The earlier phase-02 commands below ran in an aarch64 Linux Docker container
with a zero-byte build-only `OVERDRIVE_BPF_OBJECT` override and therefore make
no kernel-evidence claim. D15 was rerun through the working Lima-root runner
with an isolated writable target directory and the same build-only BPF object;
its three real-adapter failures are actual nft/netlink evidence. Workspace
`.env` supplies the native-metal target. D15 itself assigns no body to metal;
the later 02-03 remediation assigns S25/S26 real-enforcement evidence there.

## Step 03-02 acceptance-oracle correction

The original S-ND295-28 claim-before-detection assertion counted raw
`observed.windows(5) == b"EXEC "` prefixes and required zero. That observation
contradicted the accepted EXEC-close linearization: a claim linearized before
detection may make progress or finish, and already-written command bytes are
not paused or frozen. A prefix without the Published Language's terminating
newline is not a complete command.

The corrected body preserves the same scenario, production `VmDriver`, real
beacon socket, forced backpressure, Open-to-Recovering transition, release-task
cancellation, and EOF ownership evidence. It changes only the command oracle:
every newline-terminated frame must be valid UTF-8, parse through the existing
`BeaconMessage` parser, and be the sole permitted `Exec`. Parser rejection,
unexpected typed message kinds, and a genuine second `Exec` fail the body;
zero or one complete `Exec` is accepted. Only the final unterminated suffix is
ignored as allowed pre-detection partial progress. The cancelled release-task
join proves the task-owned opaque claim lifetime ended; EOF proves the
transferred production writer closed instead of detaching. No private
`active_claims` inspection or new hook is used.

Iteration-1's interleaving falsifier was reproduced before remediation through
the production parser: the byte stream contained two newline-complete frames,
while the old fallible `filter_map` chain retained zero. This was an oracle
weakness only; it did not reproduce a production second writer.

| Scenario / exact body | Exact command | Observed result | Classification |
|---|---|---|---|
| S-ND295-28 `claim_before_detection_backpressure_and_cancellation_do_not_create_a_second_writer` | `cargo xtask lima run -- sh -c 'CARGO_TARGET_DIR=/tmp/codex-netns-density-target TMPDIR=/tmp cargo nextest run -p overdrive-worker --test acceptance -E "test(=acceptance::netns_density_exec_release::claim_before_detection_backpressure_and_cancellation_do_not_create_a_second_writer)" --run-ignored ignored-only --no-fail-fast'` | Writable Lima run `e9bbaf27-f380-4442-81b7-7ecc3ba3f7a9`: **1 passed**, 101 skipped, 0 failed. The release task was cancelled and joined, the production beacon reached EOF, every complete frame parsed as the sole permitted typed EXEC, and no second complete EXEC appeared. Focused production-parser run `1a01cd05-6939-4e4b-9810-ef9d7d323a95` independently passed malformed-EXEC rejection: 1 passed, 528 skipped. | **GREEN — ACCEPTED BEHAVIOR ALREADY PRESENT AFTER FAIL-CLOSED ORACLE CORRECTION.** This body is not a missing-functionality RED and authorized no production change. Step `03-02` subsequently activated it unchanged; the complete mapped set is recorded below. |

## Step 03-02 evidence-boundary correction

The first DELIVER review exposed two acceptance/roadmap defects rather than a
production failure:

- **D1 reproduced:** literal writable-Lima run
  `91765670-b745-444f-9d13-5a1ba75ba202` selected zero tests and exited 4
  because `generated_operation_sequences_match_the_gate_model` is an
  `overdrive-core` body, not an `overdrive-worker` body.
- **D2 reproduced:** writable-Lima run
  `48d71b7f-9d22-493f-a721-d061cdb79e5e` timed out at `release_entered`
  because the old fixture called historical `dispatch`, which composed
  `HostNetworkProvisioner` and never reached the post-#295 release owner.

The corrected roadmap retains the core model as step `03-01`'s PBT prerequisite
and assigns only deterministic real-`VmDriver`/writer schedules to the worker.
The existing action-shim body now drives the sanctioned post-#295
`dispatch_with_guest_network_provisioner_for_test` composition through an
`AppState` holding the existing `HoldingReleaseDriver` and healthy
`MtlsInterceptWorker`, with `SimSharedGuestNetworkOwner` at the accepted driven
port. It reaches `release_for_exit_emission`; aborting the same dispatch task
drops that held release future, sets `release_cancelled`, and still leaves
`release_completed` and `on_alloc_running_called` false. No test seam or
production API was added.

| Complete S-ND295-28 evidence group | Exact selector result |
|---|---|
| Core generated model, `PROPTEST_CASES=1024` | Run `0224a89e-37a6-4c30-a922-2b37e5b51719`: 1 passed, 528 skipped. |
| Three real-`VmDriver` `netns_density_exec_release` schedules | Run `924ca327-f941-4559-99fc-a5ff0c6d039a`: 3 passed, 99 skipped. |
| Writer lifetime, stop deadline, and cancellation schedules | Run `9f906ad7-b592-4b5c-861e-ecf9e2d6968a`: 3 passed, 99 skipped. |
| Post-#295 action-owner await/cancellation schedule | Run `ee147208-d028-477d-be3c-57c415b9f673`: 1 passed, 208 skipped. |

All eight mapped bodies are therefore GREEN through their accepted owners. The
roadmap returns to `validation.status = pending` because this material
acceptance/verification correction requires independent review; this DISTILL
pass does not self-approve it.

## Phase-02 non-waived remediation bodies

| Scenario / body | Explicit command | Observed failure | Classification |
|---|---|---|---|
| S-ND295-00 `every_locked_aya_map_kind_projects_to_exact_or_opaque_semantics` | `cargo test -p overdrive-dataplane --lib guest_tcx::tests::every_locked_aya_map_kind_projects_to_exact_or_opaque_semantics -- --ignored --exact` | `D12 map-kind projection` RED panic | **RED — MISSING_FUNCTIONALITY**; body reached the exact private production projection scaffold. |
| S-ND295-00 `wrong_valid_map_properties_remain_opaque_and_schema_mismatch_is_source_less` | `cargo test -p overdrive-dataplane --lib guest_tcx::tests::wrong_valid_map_properties_remain_opaque_and_schema_mismatch_is_source_less -- --ignored --exact` | `D12 map-schema projection` RED panic | **RED — MISSING_FUNCTIONALITY**; imports, opaque semantic types, and error oracle compile. |
| S-ND295-00 `capture_failure_keeps_an_observation_identity_and_only_the_first_genuine_source` | `cargo test -p overdrive-dataplane --lib guest_tcx::tests::capture_failure_keeps_an_observation_identity_and_only_the_first_genuine_source -- --ignored --exact` | `D12 inventory capture` RED panic | **RED — MISSING_FUNCTIONALITY**; program and link enumeration both retain outer `Program` while distinguishable nested `IOError(EIO)`/`IOError(ENOENT)` values prove the earlier program source wins; link observation remains exact source-less unavailability. |
| S-ND295-00 `a_unique_unreceipted_candidate_is_ambiguous_and_never_an_owned_count` | `cargo test -p overdrive-dataplane --lib guest_tcx::tests::a_unique_unreceipted_candidate_is_ambiguous_and_never_an_owned_count -- --ignored --exact` | `D12 inventory capture` RED panic | **RED — MISSING_FUNCTIONALITY**; no-receipt ambiguity contract compiles. |
| S-ND295-00 `every_receipted_family_returns_one_and_clean_families_return_exact_zero` | `cargo test -p overdrive-dataplane --lib guest_tcx::tests::every_receipted_family_returns_one_and_clean_families_return_exact_zero -- --ignored --exact` | `D12 inventory capture` RED panic | **RED — MISSING_FUNCTIONALITY**; all eight exact zero/positive receipt assertions compile behind the missing capture implementation. |
| S-ND295-00 `startup_tcp_probe_projects_semantics_and_all_eight_counter_pairs_without_raw_abi` | `cargo test -p overdrive-dataplane --lib guest_tcx::tests::startup_tcp_probe_projects_semantics_and_all_eight_counter_pairs_without_raw_abi -- --ignored --exact` | `D14 TCP probe projection` RED panic | **RED — MISSING_FUNCTIONALITY**; the private table now includes complete-length EtherType/version/IHL/protocol/destination-port failures plus short output, both inputs, exact counters, and decrease/wrap without raw ABI leakage. |
| S-ND295-00 `every_d14_semantic_mismatch_and_lower_source_reaches_the_production_validator` | `cargo test -p overdrive-control-plane --lib guest_network::scratch_probe_packet_acceptance::every_d14_semantic_mismatch_and_lower_source_reaches_the_production_validator -- --ignored --exact` | `D14A semantic TCP validator` RED panic | **RED — MISSING_FUNCTIONALITY**; the table covers both invalid verdicts, all three invalid marks, isolated destination IP/port errors, Intercept zero/oversized deltas, an identity permutation, every single mismatch/lower source, and every adjacent first-mismatch precedence through ordered counter identity → decrease → delta. |
| S-ND295-00 `classifier_runs_precede_close_and_each_stage_is_fresh` | `cargo test -p overdrive-control-plane --lib guest_network::scratch_probe_packet_acceptance::classifier_runs_precede_close_and_each_stage_is_fresh -- --ignored --exact` | current D5 closes/adopts/queries before classifier exercise | **RED — MISSING_FUNCTIONALITY**; the new and transitioned D5 tables require the sole exercise-before-close order, typed/semantic refusal, lazy adoption, and complete cleanup/inventory. |
| S-ND295-00 `production_startup_exercises_classifier_and_detached_guard_before_admission` | `cargo xtask lima run -- cargo test -p overdrive-control-plane --test integration --features integration-tests integration::shared_guest_network_startup::production_startup_exercises_classifier_and_detached_guard_before_admission -- --ignored --exact` | executable body compiled/listed, but the existing Lima VM still refused SSH after a forced restart | **PENDING_ENVIRONMENT**; ordinary `run_server` is wrapped by the existing tracing subscriber and asserts the exact five ordered production events, continuous counters, one program id, D9 deltas, zero delivery, and fifteen `Observed(0)` fields; no monitor, poll, sleep, hook, or test panic exists. |
| S-ND295-11 `persistent_tap_and_bridge_projection_preserves_every_observable_identity_field` | `cargo test -p overdrive-netlink --lib client::tests::persistent_tap_and_bridge_projection_preserves_every_observable_identity_field -- --ignored --exact` | `D12A TAP identity projection` RED panic | **RED — MISSING_FUNCTIONALITY**; raw absent/TAP-with-exact-or-missing-owner/TUN/dummy/veth/other and correct/wrong-kind bridge rows reach the retained private projection boundary. |
| S-ND295-11 `provision_reads_every_attachment_fact_before_reporting_success` | `cargo test -p overdrive-control-plane --lib guest_network::allocation_owner_acceptance::provision_reads_every_attachment_fact_before_reporting_success -- --ignored --exact` | production owner returned with call trace `[]`, not the exact 16-call D12A sequence | **RED — MISSING_FUNCTIONALITY**; fails on current no-op provision, not the scripted leaf. |
| S-ND295-11 `every_incompatible_tap_or_bridge_identity_refuses_owner_publication` | `cargo test -p overdrive-control-plane --lib guest_network::allocation_owner_acceptance::every_incompatible_tap_or_bridge_identity_refuses_owner_publication -- --ignored --exact` | first finite-table case expected an owner-authored `TapObserve` mismatch; current provision returned `Ok(())` | **RED — MISSING_FUNCTIONALITY**; both checkpoints' exact Tap/BridgeLinkIdentity/LinkMaster facts and publication refusal assertions compile. |
| S-ND295-12 `every_teardown_leaf_failure_continues_cleanup_and_retry_reaches_the_exact_complement` | `cargo test -p overdrive-control-plane --lib guest_network::allocation_owner_acceptance::every_teardown_leaf_failure_continues_cleanup_and_retry_reaches_the_exact_complement -- --ignored --exact` | expected first `EndpointDelete` source while a later TAP-delete failure is retained for continuation; current teardown returned `Ok(())` | **RED — MISSING_FUNCTIONALITY**; the separate eleven-row table covers every cleanup mutation/read and both TAP-observation occurrences with exact operation/source, complete continuation, and retained state; same-owner retry, held lease, complement, and unrelated facts also compile. |
| S-ND295-13 `reclamation_completes_before_stale_shared_network_sweep_for_every_seeded_prior_vm` | `PROPTEST_CASES=1024 cargo test -p overdrive-sim --lib invariants::netns_density_boot_order::tests::reclamation_completes_before_stale_shared_network_sweep_for_every_seeded_prior_vm -- --ignored --exact --nocapture` | actual sweep-call snapshot retained seeded run directory/scope; proptest shrank to `seed = 0` | **RED — MISSING_FUNCTIONALITY / reproduced ordering defect**; production helper, same Sim host, real sweep port call, and seed printing all executed. |

## D-295-DISTILL-15 authored evidence

All ten layered D15 bodies compile, exact selectors discover one body each, and
every authored body carries the approved bounded-change declaration and reasoned
step marker. S14..18, S19-A source-local/Lima, the non-closing worker
prerequisite, and S19-B are creditable RED for current missing behavior. S19-B
now carries the reviewed P02-20/21/22 boundary-exact clock, request, journal,
and sole-terminal-owner assertions; no fixture retry/request is accepted.

| Scenario / exact required body | Exact command | Observed result | Classification |
|---|---|---|---|
| S-ND295-14..18 `shared_program_post_commit_failure_rolls_back_source_honestly_for_every_prior` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --lib -E 'test(=mtls_intercept_port::shared_program_rollback_acceptance::shared_program_post_commit_failure_rolls_back_source_honestly_for_every_prior)' --run-ignored ignored-only --no-fail-fast` | The first semantic-trigger rollback used the current pre-D15 conditional arguments, so the stateful seam returned `conditional-identity-mismatch/EAGAIN` instead of the scripted rollback-operation source. | **RED — MISSING_FUNCTIONALITY**; both optional priors, both trigger classes, four rollback outcomes, state/complement/full journal/fault schedule, structured fields, and the actual Rust `Error::source()`/source-less chain compile behind the incorrect production rollback state machine. |
| S-ND295-14..18 `shared_program_replace_refusal_idempotence_and_guard_cleanup_preserve_complete_state_delta` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --lib -E 'test(=mtls_intercept_port::shared_program_rollback_acceptance::shared_program_replace_refusal_idempotence_and_guard_cleanup_preserve_complete_state_delta)' --run-ignored ignored-only --no-fail-fast` | After successful absence-create/read-back, dropping the returned guard left `owned_program = Some(requested)` instead of exact absence. | **RED — MISSING_FUNCTIONALITY**; rejection, create, retarget, reapply, and cleanup each assert the exact conditional mutation journal; current `SharedInterceptGuard` has no conditional cleanup ownership. |
| S-ND295-15 `shared_program_prior_snapshot_mismatch_preserves_complete_state_and_complement` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --lib -E 'test(=mtls_intercept_port::shared_program_rollback_acceptance::shared_program_prior_snapshot_mismatch_preserves_complete_state_and_complement)' --run-ignored ignored-only --no-fail-fast` | The stale-prior partition remains complete and unchanged; the added independent zero-leg-F row then returned the current non-D15 error after consuming I/O instead of `shared-ip-expected` before I/O. | **RED — MISSING_FUNCTIONALITY**; both zero-port axes preserve the complete state/journal/fault schedule. |
| S-ND295-14/16 `shared_program_absence_create_readback_idempotence_and_guard_drop` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test integration --features integration-tests -E 'test(=integration::mtls_intercept_install::shared_program_absence_create_readback_idempotence_and_guard_drop)' --run-ignored ignored-only --no-fail-fast` | Real `HostMtlsIntercept::converge_shared(None, …)` reached the real nft adapter and returned `NftSharedReplaceFailed` from `atomic-rule-transaction` with `ENODATA`. | **RED — MISSING_FUNCTIONALITY / REAL KERNEL**; exact set ABI and atomic full-object create are missing, not fixture setup. |
| S-ND295-14 `shared_program_replaces_only_listener_targets_and_preserves_foreign_complement` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test integration --features integration-tests -E 'test(=integration::mtls_intercept_install::shared_program_replaces_only_listener_targets_and_preserves_foreign_complement)' --run-ignored ignored-only --no-fail-fast` | Real public-adapter prior creation reached the same `atomic-rule-transaction` `ENODATA` before retarget assertions. | **RED — MISSING_FUNCTIONALITY / REAL KERNEL**; the body now compares old/new read-back byte-for-byte with D15's exact semantic identities for both requested non-zero target pairs. |
| S-ND295-15 `shared_program_refuses_ambiguous_owned_state_without_mutation` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test integration --features integration-tests -E 'test(=integration::mtls_intercept_install::shared_program_refuses_ambiguous_owned_state_without_mutation)' --run-ignored ignored-only --no-fail-fast` | The first exact valid-IP seed reached real prior creation and failed with `atomic-rule-transaction` `ENODATA`. | **RED — MISSING_FUNCTIONALITY / REAL KERNEL**; the finite kernel table now seeds valid IP plus same-name bridge coexistence, uses only pre-D15 raw delete/insert/append fixtures for unknown userdata, and restores all eight rules around one independently conflicting set schema. |
| S19-A `runtime_present_wrong_target_and_observe_error_are_non_mutating` | `cargo nextest run -p overdrive-worker --lib -E 'test(=mtls_intercept_port::shared_program_rollback_acceptance::runtime_present_wrong_target_and_observe_error_are_non_mutating)' --run-ignored ignored-only --no-fail-fast` | Exact body selected and stopped at D15's `SharedIpInterceptIdentity::for_listener_ports` RED scaffold. | **RED — MISSING_FUNCTIONALITY**, step `02-02`; canonical wrong target `Ok(Some(identity))` plus partial/foreign/duplicate/malformed/lower typed-error rows compile with exact one-Observe/no-mutation universes. |
| S19-A `shared_program_valid_wrong_target_observation_is_non_mutating` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test integration --features integration-tests -E 'test(=integration::mtls_intercept_install::shared_program_valid_wrong_target_observation_is_non_mutating)' --run-ignored ignored-only --no-fail-fast` | Exact Lima body reached real public-host creation and failed at current `atomic-rule-transaction/ENODATA`. | **RED — MISSING_FUNCTIONALITY / REAL KERNEL**, step `02-02`; exact canonical wrong identity, generation/notification no-mutation, target inventory, and foreign complement compile. |
| S19 published-worker prerequisite `published_wrong_shared_target_is_observe_only_until_bounded_fail_stop` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test acceptance -E 'test(=acceptance::netns_density_shared_owner::published_wrong_shared_target_is_observe_only_until_bounded_fail_stop)' --run-ignored ignored-only --no-fail-fast` | Body selects and stops at `start_shared_owner`; canonical different non-zero target and single observe-only worker conflict compile. | **RED — PREREQUISITE ONLY**, step `02-03`; no cadence/deadline/request credit. |
| S19-B `published_wrong_shared_target_retries_on_production_cadence_and_emits_one_typed_fail_stop` | `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --lib -E 'test(=shared_network_task_owner_acceptance::published_wrong_shared_target_retries_on_production_cadence_and_emits_one_typed_fail_stop)' --run-ignored ignored-only --no-fail-fast` | Exact body compiled, selected one test, and stopped at the earlier `MtlsInterceptWorker::start_shared_owner` RED scaffold. The later compiled oracle asserts 249 ms elapsed-only subintervals, attempts 1..19 as exact Recovering snapshots, attempt 20 only through the typed `20/5s` request, no attempt 21/second request, exact closed journals, and terminal ownership only through `ServerHandle::shutdown`. | **RED — MISSING_FUNCTIONALITY**, step `03-03`; P02-20/21/22 specification remediation is complete and the current failure is the expected earlier production scaffold. |

Every row requires `/// CONTRACT_SHAPE: bounded-change.` plus the exact
reasoned marker recorded in `test-scenarios.md`. The source-local complement is
all three dynamic-set element inventories plus ordered foreign bytes; the Lima
complement is the complete target-table semantic inventory plus an unrelated
foreign-table sentinel; the worker prerequisite complement is retained
published socket/task/guard state. The control-plane pre-terminal journal is
exactly one detection observe, one designed quiesce, and twenty attempt
observes; provision/teardown/probe/sweep/shared-converge/shared-audit/repeat-
quiesce/bind/fresh-converge/install/relinquish are forbidden. Terminal
relinquishment appears only after existing `ServerHandle::shutdown` invokes the
worker owner, before supervisor cancel/join. Required receipts are exactly
`shared_owner.calls() == [TapSetDown]` and intercept delta
`(bind=0, fresh_converge=0, observe=21, guard_drop=0)` pre-terminal.
No DELIVER crafter may author or materially repair these bodies.

The 2026-09-21 S19-B rerun encountered the Lima guest root filesystem already
remounted read-only, so the canonical shared `CARGO_TARGET_DIR` could not create
`.cargo-lock`. The exact Lima selector was rerun against a disposable writable
target seeded from the same guest cache; compilation completed and the body
failed only at the production `start_shared_owner` scaffold above. No
real-kernel claim depends on this source-local body.

## Step 02-03 D1-D7 review-remediation evidence

All bodies below are reasoned-pending for step `02-03`, carry the exact
bounded-change declaration and Outcome anchor, and compile through the accepted
private/public production boundaries. A GREEN ignored run is recorded honestly
where the reviewed production behavior already exists and only its evidence was
missing; DISTILL does not fabricate a failure. D1, D2, and the action-shim
ordering body reproduce current defects semantically.

| Scenario / exact body | Exact command | Observed result | Classification |
|---|---|---|---|
| D11 `dropping_every_observer_aborts_its_listener_and_closes_the_real_event_channel` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --lib -E 'test(=mtls_intercept_worker::shared_listener_task_owner_acceptance::dropping_every_observer_aborts_its_listener_and_closes_the_real_event_channel)' --run-ignored ignored-only --no-fail-fast` | Listener-task Drop witnesses remained live for two seconds after both observers were aborted. | **RED — MISSING_FUNCTIONALITY**; the observer does not own `AbortOnDropListenerTask`, and the owner-held strong sender prevents genuine channel closure. Once disconnected, the row consumes the owner without attempting a `WeakSender::upgrade`; live-observer rows retain the strong receiver-drop probe. |
| D11 `one_real_join_event_removes_only_its_terminal_slot_before_replacement_and_consumed_shutdown` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --lib -E 'test(=mtls_intercept_worker::shared_listener_task_owner_acceptance::one_real_join_event_removes_only_its_terminal_slot_before_replacement_and_consumed_shutdown)' --run-ignored ignored-only --no-fail-fast` | The actual leg-F join event was classified but its terminal slot remained present. | **RED — MISSING_FUNCTIONALITY**; exact event consumption, independent leg-C retention, replacement, and the strong probe proving `shutdown(self)` dropped the sole receiver before return execute against the real private owner. |
| D11 `replacement_refuses_a_still_live_occupied_slot_without_detaching_either_listener` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --lib -E 'test(=mtls_intercept_worker::shared_listener_task_owner_acceptance::replacement_refuses_a_still_live_occupied_slot_without_detaching_either_listener)' --run-ignored ignored-only --no-fail-fast` | Refusal and both live slots are executable; current borrowed shutdown leaves the receiver probe open. | **RED — MISSING_FUNCTIONALITY** until consuming shutdown lands; the occupied-slot refusal itself is exact and neither original child detaches. |
| D7 `generation_boundaries_and_every_lifecycle_conflict_precede_effects` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --lib -E 'test(=mtls_intercept_worker::capability_registry_acceptance::generation_boundaries_and_every_lifecycle_conflict_precede_effects)' --run-ignored ignored-only --no-fail-fast` | Passed with byte-equal full registry snapshots before/after exhaustion and every Pending/Active/Retiring conflict. | **GREEN — EVIDENCE CORRECTION**; current behavior already preserves generation, records, reservations, both indexes, effects, claims, and handles. |
| D7/S25 `publication_before_retirement_is_owned_by_only_that_generation_and_allocation` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --lib -E 'test(=mtls_intercept_worker::capability_registry_acceptance::publication_before_retirement_is_owned_by_only_that_generation_and_allocation)' --run-ignored ignored-only --no-fail-fast` | Passed with the first exact record/index universe removed and the unrelated capability complement byte-equal. | **GREEN — EVIDENCE CORRECTION**; the prior selective assertions are replaced by a complete registry-universe oracle. |
| D3/S23 `enforcement_returning_after_retirement_tears_down_the_real_returned_handle_before_drain` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --lib -E 'test(=mtls_intercept_worker::tests::enforcement_returning_after_retirement_tears_down_the_real_returned_handle_before_drain)' --run-ignored ignored-only --no-fail-fast` | Passed through `start_shared_owner` + real shared `start_alloc` + production `handle_shared_outbound`/`spawn_shared_enforcement`, with stop parked on the claim and the late handle torn down once. | **GREEN — PRODUCTION-PATH EVIDENCE**; the rejected direct `enforcement.enforce`/`claim.publish` test path is gone. |
| D2 `shared_teardown_failure_retains_the_exact_handle_drain_and_reservation_until_same_owner_retry` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --lib -E 'test(=mtls_intercept_worker::tests::shared_teardown_failure_retains_the_exact_handle_drain_and_reservation_until_same_owner_retry)' --run-ignored ignored-only --no-fail-fast` | After the exact first teardown error, a contender was accepted: the assertion that the Retiring reservation survived failed. | **RED — MISSING_FUNCTIONALITY**; exact first source, stable handle identity `<allocation>#0`, retained retry identity, two identical teardown identities, Retiring reservation, retry, completion fence, and successor admission are executable. |
| S14-S18 owner refusal `initial_leg_f_bind_refusal_returns_to_absent_without_partial_publication` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test acceptance -E 'test(=acceptance::netns_density_shared_owner::initial_leg_f_bind_refusal_returns_to_absent_without_partial_publication)' --run-ignored ignored-only --no-fail-fast` | Passed all `EADDRINUSE`/`EPERM`/`EMFILE` rows with the complete owner surface equal except one observe and one refused bind receipt. | **GREEN — COMPLETE COMPLEMENT EVIDENCE**. |
| S14-S18 owner refusal `leg_c_bind_refusal_closes_the_already_bound_leg_f_and_publishes_no_owner` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test acceptance -E 'test(=acceptance::netns_density_shared_owner::leg_c_bind_refusal_closes_the_already_bound_leg_f_and_publishes_no_owner)' --run-ignored ignored-only --no-fail-fast` | Passed exact two bind attempts/one observed partial address, rebind proof, zero task/program/guard publication, and otherwise byte-equal owner surface. | **GREEN — COMPLETE COMPLEMENT EVIDENCE**. |
| S14-S18 owner refusal `shared_rule_convergence_refusal_closes_both_sockets_and_publishes_no_tasks_or_guard` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test acceptance -E 'test(=acceptance::netns_density_shared_owner::shared_rule_convergence_refusal_closes_both_sockets_and_publishes_no_tasks_or_guard)' --run-ignored ignored-only --no-fail-fast` | Passed exact two binds/one convergence refusal/one prior observation, both-socket rebind proof, and zero task/program/guard publication. | **GREEN — COMPLETE COMPLEMENT EVIDENCE**. |
| S20 `shared_owner_starts_once_audits_and_shutdown_drains_the_owner_tree` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test acceptance -E 'test(=acceptance::netns_density_shared_owner::shared_owner_starts_once_audits_and_shutdown_drains_the_owner_tree)' --run-ignored ignored-only --no-fail-fast` | Passed exact two binds, one convergence, four startup-plus-explicit observations, two live sockets/tasks, one retained guard, idempotent byte-equal restart, socket closure, and sealed relinquishment. | **GREEN — EVIDENCE CORRECTION**; the prior weak Sim-only cardinality oracle is replaced by the existing recording adapter. |
| D7 Pending race `stop_and_owner_shutdown_during_pending_registration_return_registration_retired_and_drain_once` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test acceptance -E 'test(=acceptance::netns_density_shared_owner::stop_and_owner_shutdown_during_pending_registration_return_registration_retired_and_drain_once)' --run-ignored ignored-only --no-fail-fast` | Passed exact `RegistrationRetired`, three element drops, no allocation publication, and byte-equal shared-owner complement for stop and owner-shutdown partitions. | **GREEN — COMPLETE COMPLEMENT**. |
| S25 `stopping_one_shared_allocation_preserves_the_unrelated_handle_and_complete_listener_owner` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test acceptance -E 'test(=acceptance::netns_density_shared_owner::stopping_one_shared_allocation_preserves_the_unrelated_handle_and_complete_listener_owner)' --run-ignored ignored-only --no-fail-fast` | Passed with the unrelated `EnforcedConnectionId`, F/C sockets/tasks, program identity, call journal, and node guard unchanged; the second allocation completed another byte-distinct connection. | **GREEN — SOURCE-LOCAL PRODUCTION-OWNER EVIDENCE**. |
| S26 `owner_shutdown_waits_the_active_claim_then_drains_every_shared_capability_and_listener` | `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test acceptance -E 'test(=acceptance::netns_density_shared_owner::owner_shutdown_waits_the_active_claim_then_drains_every_shared_capability_and_listener)' --run-ignored ignored-only --no-fail-fast` | Passed with shutdown parked on the third claim, three handles drained, six allocation elements removed, both sockets closed, and zero node-guard Drop. | **GREEN — SOURCE-LOCAL PRODUCTION-OWNER EVIDENCE**. |
| D5 `registration_retired_from_real_start_alloc_keeps_exec_closed_and_releases_the_address_last` | `cargo xtask lima run -- sh -c 'mkdir -p "$PWD/target/netns-density-0203-test-tmp"; TMPDIR="$PWD/target/netns-density-0203-test-tmp" cargo nextest run -p overdrive-control-plane --test integration --features integration-tests -E "test(=integration::mtls_install_fail_closed::registration_retired_from_real_start_alloc_keeps_exec_closed_and_releases_the_address_last)" --run-ignored ignored-only --no-fail-fast'` | The real action-shim race produced `[NetworkProvision, DriverStart, MtlsElementDrop, MtlsElementDrop, DriverStop, NetworkTeardown]`, not the accepted driver-stop-before-mTLS-drain order. | **RED — MISSING_FUNCTIONALITY**; exact stage, zero EXEC release/running hook, one shared retirement owner, mTLS-before-network, address-last, and cleanup-primary precedence all execute. The writable `TMPDIR` only avoids the guest's pre-existing full `/tmp`; it changes no SUT boundary. |
| S25 native `two_real_shared_capabilities_keep_the_unrelated_tls_handle_live_after_one_stops` | `cargo xtask metal run -- cargo nextest run -p overdrive-worker --test integration --features integration-tests -E 'test(=integration::outbound_enforce_substrate_splice::two_real_shared_capabilities_keep_the_unrelated_tls_handle_live_after_one_stops)' --run-ignored ignored-only --no-fail-fast` | Metal bootstrap acquired the configured lease and synchronized the tree, then refused before test execution because the selected guest kernel artifact was absent. | **PENDING_ENVIRONMENT**; Lima compilation proves the exact real-enforcement body and selector, but no native result is claimed. |
| S26 native `real_owner_shutdown_closes_admission_waits_one_claim_and_drains_every_shared_handle` | `cargo xtask metal run -- cargo nextest run -p overdrive-worker --test integration --features integration-tests -E 'test(=integration::outbound_enforce_substrate_splice::real_owner_shutdown_closes_admission_waits_one_claim_and_drains_every_shared_handle)' --run-ignored ignored-only --no-fail-fast` | Same native bootstrap prerequisite: the configured target lacks the selected guest kernel artifact. | **PENDING_ENVIRONMENT**; the body uses real `HostMtlsEnforcement`, TLS peers, shared listener dispatch, two published handles, and a third enforcement-held claim. |

The prior bounded gate scored ten non-waived bodies. D14A transitions S00 to
the four exact bodies above: all three source-local filters are invoked
independently and fail on missing production behavior; the ordinary-boot body is compile/list verified and
remains `PENDING_ENVIRONMENT` until Lima root is available. No D14 body fails
in import, collection, or fixture construction, and no waived placeholder is
counted.

## User-waived real-I/O placeholders — not rerun and not scored

The user explicitly waived these eight unconditional real-I/O panic
placeholders from this remediation gate. They remain reasoned-pending and are
neither RED classifications nor approval conditions here:

- `clean_and_receipted_inventory_observes_all_eight_exact_families`
- `retained_unpinned_maps_programs_and_links_survive_handle_release_and_remain_observable`
- `wrong_exact_path_owner_or_valid_map_schema_is_typed_and_never_fabricates_zero`
- `deliberate_link_loss_reaches_default_drop_and_the_exact_production_audit_cause`
- `ordinary_provision_reads_back_the_complete_attachment_before_injected_vmm_start`
- `two_attachment_teardown_releases_last_and_preserves_the_unrelated_attachment_byte_equal`
- `production_boot_trace_completes_vm_reclamation_before_stale_sweep_starts`
- `native_prior_vmm_reclamation_precedes_full_attachment_sweep_and_first_lease_acceptance`

## Compiler and unchanged-suite evidence

D15 verification update:

- Lima-root `cargo check --workspace --all-targets --features integration-tests`
  passed with the authored bodies and exact D15 error-algebra scaffold.
- Focused worker `cargo clippy -p overdrive-worker --lib --tests --features
  integration-tests --no-deps -- -D warnings` passed. The broader dependency-
  lint run remains blocked by already-recorded `overdrive-control-plane`
  production warnings outside D15; no D15 warning remains.
- Every fully-qualified selector discovers exactly one ignored body. S19-B's
  current early RED is non-creditable until its cadence/journal/terminal oracle
  is remediated; the other failures retain the classifications above.
- The unchanged worker library lane remains green: **60 passed, 0 failed, 14
  reasoned ignored**.
- `nextest show-config test-groups` assigns all four exact Lima body names to
  `host-kernel-shared` with `max-threads = 1` through the whole worker
  integration-binary override.
- Lima is currently reachable and all three D15 host-adapter bodies reached
  real nft/netlink. No D15 body is mapped to native metal; `.env` supplies the
  metal target for the separate native-VMM lanes.
- Roadmap JSON parses, validation remains `pending`, all original seven names/markers/
  Contract Shape declarations are mechanically present, and `cargo fmt --all
  -- --check` plus `git diff --check` pass.

Historical D14A verification update:

- Linux compile passed for `overdrive-dataplane --lib --tests` and
  `overdrive-control-plane --lib --tests --features integration-tests`.
- D14A dataplane clippy passed with `-D warnings`; focused control-plane source
  and integration targets pass after allowing only the unrelated production
  worktree's already-reported lint categories.
- The control-plane integration target passes clippy after allowing only the
  independently pre-existing production-file lint categories in the active
  D5/D9 worktree; full-package `-D warnings` remains blocked by those unrelated
  production edits, not by the three D14 bodies.
- The Lima body passes `cargo test --no-run` and exact-name listing. Lima SSH
  remains unavailable, so no real-kernel execution is claimed.
- `.config/nextest.toml` retains the explicit whole-binary
  `package(overdrive-control-plane) & binary(integration)` →
  `host-kernel-shared` override, and the roadmap retains the exact
  `nextest show-config test-groups` command. Host execution of that command was
  disk-limited; no weaker source-level serialization claim replaces it.

- Linux compile passed: `cargo check -p overdrive-dataplane --lib --tests`.
- Linux compile passed: `cargo check -p overdrive-netlink --lib --tests`.
- Linux compile passed: `cargo check -p overdrive-control-plane --lib --tests --features integration-tests`.
- Linux compile passed: `cargo check -p overdrive-sim --all-targets`.
- Linux lint passed: `cargo clippy -p overdrive-dataplane --lib --tests -- -D warnings`.
- Linux lint passed: `cargo clippy -p overdrive-netlink --lib --tests -- -D warnings`.
- Linux lint passed: `cargo clippy -p overdrive-control-plane --lib --tests --features integration-tests -- -D warnings`.
- Linux lint passed: `cargo clippy -p overdrive-sim --lib -- -D warnings`; the broader pre-existing `--all-targets` scope remains blocked by unrelated doc-markdown findings in three spike test files.
- Dataplane guest-TCX unit scope: **4 non-ignored passed, 0 failed**; five
  remediation bodies and one pre-existing body remained ignored.
- Netlink client unit scope: **7 non-ignored passed, 0 failed**; the one
  remediation body remained ignored.
- Control-plane guest-network unit scope: **9 non-ignored passed, 0 failed**;
  the three remediation bodies remained ignored.
- `cargo fmt --all -- --check` and `git diff --check` pass.

No scored failure is classified as BROKEN: collection, imports, exact
D12/D12A/D13 signatures, helper caller fallout, and fixtures compile. The
waived real-kernel and native-metal placeholders are not claimed as substrate
executions.

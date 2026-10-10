# Pre-DELIVER fail-for-the-right-reason classification — netns-density-295 (vsock)

Gate per `nw-distill` § "Pre-DELIVER fail-for-the-right-reason gate". Every
authored ignored test and driver was un-ignored for the run (`--run-ignored
only`), run foreground in the Lima VM (kernel 7.0), classified from its first
failure, and left `#[ignore = "pending DELIVER …"]`. Run date **2026-10-10**
(the second DISTILL increment, after DG-1..DG-16).

Classes: **MISSING_FUNCTIONALITY** — the first failure is a `Not yet
implemented -- RED scaffold` panic in `src/` or an assertion of behaviour the
production code does not have yet (correct RED). **BROKEN** — import, fixture,
setup or environment failure before the system under test (wrong RED).
**WRONG_ASSERTION** — the assertion couples to something other than the
port-observable outcome. **GUARD** — a structural or negative property that
holds today and must keep holding (no RED is possible). **AC** — not authored
now: the body calls a DG-2 replaced signature and is written at the
attachment cut (feature-delta § *Attachment-cut DISTILL increment*). Every
round-1 `UI-*` item is closed by a DG decision. Round 2 leaves one open model
decision that bears on authored bodies: the three guest Quint drivers are
authored, but their model fidelity is open (UI-R2-5) — a correct
implementation may fail them until the model decision.

## Runs and results

```
cargo xtask lima run -- cargo nextest run -p overdrive-core --run-ignored only -E 'test(/vsock_|mtls_mark/)' --no-fail-fast
```

55 run, 55 failed, all MISSING_FUNCTIONALITY: 51 first panic at a `Not yet
implemented -- RED scaffold` panic in `overdrive-core` `src/`; 4 behavioural
assertions (`vsock_deploy_validation`: both ingresses accept the reserved /
too-many ports today).

```
cargo xtask lima run -- cargo nextest run -p overdrive-dataplane -p overdrive-sim -p overdrive-worker --features integration-tests --run-ignored only -E 'test(/vsock_|guest_steering|beacon|shared_table|steering_key|stage_labels/)' --no-fail-fast
```

69 run (re-run after the review fixes, which added two
`vsock_mesh_egress_mtls` DG-10 cells and brought the stage-label lib test into
the filter). 65 failed, MISSING_FUNCTIONALITY: 54 RED scaffold panics
(including `mtls_intercept_worker::tests::forwarded_outbound_refusals_have_pinned_stage_labels`
at `MtlsInterceptInstallError::stage`); 10 `vsock_host_internal_enforce`
behavioural failures (the marked connect / transmit succeeds today); 1
`vsock_mesh_egress_mtls::a_serve_exit_mid_flow_resets_the_remote_peer`, whose
serve child exits on the worker's RED scaffold panic. 4 passed: one GUARD,
`vsock_egress_unframe::the_forwarder_crate_never_references_udp_segment`, and
three `vsock_host_internal_enforce` controls (ignored with their step; pass
against today's table):
`a_marked_tcp_connect_to_a_neighbour_stays_reachable`,
`a_marked_udp_transmit_to_an_off_host_address_is_not_rejected`,
`a_packet_carrying_only_the_leg_s_or_tproxy_mark_reaches_a_local_service`.
The three controls activate together with the reject cells they bound.

```
cargo xtask lima run -- cargo nextest run -p overdrive-host --features integration-tests,kvm-tests --run-ignored only -E 'test(/vsock_cid_claim/)' --no-fail-fast
```

6 run, 6 RED scaffold (MISSING_FUNCTIONALITY). Their real lane is native metal
(`kvm-tests`, real `/dev/vhost-vsock`); they were classified in Lima.

```
cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --features integration-tests --lib --run-ignored only -E 'test(/prefix_boot|flow_io|guest_network::flow/)' --no-fail-fast
```

20 run (17 acceptance tests + 3 `prefix_boot` driver instances), 20 RED
scaffold (`with_flow_io` / `InterfaceIndex::new`), MISSING_FUNCTIONALITY.

```
cargo xtask lima run -- cargo nextest run -p overdrive-init --features integration-tests --run-ignored only --no-fail-fast
```

49 run (45 acceptance tests + 4 driver instances), 49 RED scaffold,
MISSING_FUNCTIONALITY. Most first reach the core `LaunchToken` scaffold of
new-step 2, which lands before new-step 5; the rest the `overdrive-init`
scaffolds.

**Default lane** (the non-ignored new tests: the sims' own tests and the
`Display` / label / mark GUARDs): 82 run, 82 passed.

**Totals:** 199 ignored tests and drivers run in the five runs above; **0 BROKEN, 0
WRONG_ASSERTION**; 1 ignored GUARD and 3 ignored controls (pass against
today's table) plus the passing default-lane guards; every other ignored test
MISSING_FUNCTIONALITY. The four guest-driver instances are MISSING_FUNCTIONALITY
today, but their model fidelity is open (UI-R2-5): once the guest owner lands
they may fail for a model reason rather than an implementation reason, until
the model decision.

## Ignore reasons in the code

The reasons below are quoted from the tree. Steps 6, 7, 8, 9 (owner wiring),
13 (owner), 14 and 15, and the owner-level `probe_startup` refusals of steps 4
and 11 (IFACE-NO-STACK, D5a-IFACE-BOOT, D24), carry the attachment-cut form;
additive steps and the adapter halves of 4, 9, 11 and 13 keep the `new-step N`
form.

| File | Reason(s) |
|---|---|
| `overdrive-core` `vsock_wire_codec`, `vsock_datagram_frame`; `overdrive-sim` `adapters::beacon_io` | `pending DELIVER vsock new-step 2 (core types and wire codec)` |
| `overdrive-core` `vsock_guest_transport_vocabulary` | `pending DELIVER vsock new-step 2 (core types and wire codec)`; `pending DELIVER vsock new-step 5 (guest intakes, D25)`; `pending DELIVER vsock new-step 9 (intake host ports, INTAKE-BIND)`; `pending DELIVER vsock attachment cut (new-step 15, move-only claim handle in the transport handoff)`; `pending DELIVER vsock attachment cut (new-step 15, launch token drawn with the claim)` |
| `overdrive-core` `vsock_deploy_validation` | `pending DELIVER vsock new-step 9 (deploy-ingress validation: D25 reserved-port refusal, INTAKE-PORTS)` |
| `overdrive-dataplane` `vsock_guest_steering`, `guest_steering` unit tests | `pending DELIVER vsock new-step 9 (guest-prefix steering)`; the swap test `pending DELIVER vsock new-step 13 (steering load and swap)` |
| `overdrive-dataplane` `vsock_egress_unframe` | `pending DELIVER vsock new-step 4 (host forwarder)`; `pending DELIVER vsock new-step 4 (USO-NO-SEGMENT source check)` |
| `overdrive-dataplane` `vsock_fragment_map` | `pending DELIVER vsock new-step 4 (host forwarder fragment map)` |
| `overdrive-host` `vsock_cid_claim_handoff` | `pending DELIVER vsock attachment cut (new-step 15, VhostVsockCidClaim, D16-CLAIM)` |
| `overdrive-worker` `vsock_beacon_intake` | `pending DELIVER vsock attachment cut (new-step 6, beacon relocation, D17)` |
| `overdrive-worker` `vsock_mesh_egress_mtls`, `mtls_intercept_worker` stage-label test | `pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)` |
| `overdrive-worker` `vsock_host_internal_enforce` | `pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)` |
| `overdrive-control-plane` `guest_network::prefix_boot_acceptance` | each `pending DELIVER vsock attachment cut (…)` with: `new-step 13, steering load and swap, shared local route`; `new-step 13, probe_startup guest-prefix checks, D8-PREFIX-VIP`; `new-step 13, probe_startup guest-prefix checks, D8-PREFIX-CFG`; `new-step 9, intake host ports, INTAKE-BIND`; `new-step 4, host forwarder: egress unframe attach at boot, D5a-IFACE-BOOT`; `new-step 4, probe_startup stacked-interface refusal, IFACE-NO-STACK`; `new-step 7, owner flows: fd budget in probe_startup`; `new-step 11, owner cgroup placement under the connect4 attach point, D24` |
| `overdrive-control-plane` `guest_network::prefix_boot_conformance` | `pending DELIVER vsock attachment cut (new-step 13, steering load and swap, shared local route)` |
| `overdrive-init` `transport::tests`, `guest_flow::tests::*` | `pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)` |
| `overdrive-init` `guest_flow::quint_{owner_flows,flows_half,udp_slots}` | `pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init); model fidelity open, upstream UI-R2-5` |
| `tests/conformance` `vsock_prefix_landing_conformance` | `pending DELIVER vsock new-step 5 and attachment cut (new-steps 9, 12, 13: guest intake adaptation, guest-prefix steering with intake listeners, constant prefix rules, the prefix route); then stays ignored: native metal only, run by run-native-metal.sh with quint provisioned — on activation replace this reason, never remove the attribute` |

## Per scenario

### P1 — admission, CID lease, deploy validation

- P1-S1 — `overdrive-core` `vsock_guest_transport_vocabulary::{every_leased_offset_is_a_cid_that_round_trips, every_value_outside_the_span_is_out_of_range, the_kernel_reserved_cids_and_the_probe_cid_are_never_leasable, launch_token_round_trips, launch_token_rejects_all_zero_upper_case_and_wrong_length, a_launch_token_is_drawn_from_the_injected_entropy}` — MISSING_FUNCTIONALITY.
- P1-S2, S3, S4, S5, S7, S9 — AC (DG-2 rows 1, 2): pool tests over `GuestAddressPool::new(/29, gateway, SimGuestCidClaim, SimEntropy)` / `assign(a, &ports)` / `retire` / `release`, asserting O2 `lease_observation()` and `held_cids`.
- P1-S6 — `vsock_guest_transport_vocabulary::{concurrent_clones_take_the_claim_exactly_once, the_assignment_reports_its_claim_and_equality_ignores_the_slot, an_untaken_claim_is_released_when_the_last_clone_drops, a_claim_handed_to_the_vmm_holds_the_cid_until_the_device_drops}` — MISSING_FUNCTIONALITY. Failed-`create` half: AC (row 6).
- P1-S8 — `vsock_guest_transport_vocabulary::{intake_host_ports_are_injective_inside_the_block, indices_beyond_the_block_map_to_nothing, the_intake_host_block_sits_below_the_host_ephemeral_range_and_apart_from_the_guest_reservation}` — MISSING_FUNCTIONALITY. `IntakeCapacityReached` half: AC (row 2).
- P1-S10 — `vsock_deploy_validation::{deploy_refuses_reserved_listener_port_naming_the_range, deploy_refusal_iff_port_in_reserved_range, api_ingress_refuses_reserved_listener_port_with_validation}`, `vsock_guest_transport_vocabulary::the_reserved_range_holds_the_tcp_intake_and_every_udp_slot_above_the_ephemeral_range` — MISSING_FUNCTIONALITY (the API-ingress test is a behavioural assertion: the ingress accepts the reserved port today).
- P1-S11 — `vsock_deploy_validation::{deploy_accepts_exactly_the_block_size_and_refuses_one_more, deploy_refusal_iff_distinct_tcp_ports_over_the_constant, api_ingress_refuses_too_many_tcp_listener_ports_with_validation}` — MISSING_FUNCTIONALITY (behavioural: both ingresses accept the extra port today); the newtype cells `vsock_guest_transport_vocabulary::{listen_port_round_trips, interface_index_round_trips, listen_ports_order_by_value, non_decimal_text_is_unparseable, zero_is_refused_zero_by_the_constructors_from_str_and_serde, out_of_range_and_malformed_text_is_unparseable}` — MISSING_FUNCTIONALITY; `listen_port_and_interface_index_errors_render_their_pinned_messages` — default-lane GUARD.
- P1-S12 — `overdrive-host` `vsock_cid_claim_handoff::{claim_on_unowned_instance_holds_cid_exclusively, a_second_claim_while_the_first_is_open_is_in_use_and_holds_nothing, claim_released_at_last_reference_close, claim_probe_holds_on_the_real_device, claimed_device_held_refuses_reclaim_until_released, claims_of_distinct_cids_are_independent}` — MISSING_FUNCTIONALITY (classified in Lima; lane native metal); `the_claim_adapter_names_the_real_device` — default-lane GUARD. VMM-handoff cell: AC (row 6: `Vmm::create(&cfg, claim)` on the real fork; the CID is held by the VMM until it exits).
- P1-S13 `cid_lease` driver — AC (rows 1, 2, 3, 6).

### P2 — owner flows, control session, beacon, wire codec

- P2-S1 — `vsock_wire_codec::{every_flow_request_round_trips, every_accepted_request_is_canonical, every_mutated_request_is_canonical_or_refused, control_open_round_trips_and_is_canonical, launch_token_round_trips, golden_flow_requests_pin_every_kind, golden_control_open, an_all_zero_launch_token_is_refused_everywhere, the_flow_id_carries_its_origin_in_the_top_bit, malformed_flow_requests_are_refused_with_their_cause, malformed_control_opens_are_refused_with_their_cause, launch_token_text_is_exactly_32_lowercase_hex_characters}` — MISSING_FUNCTIONALITY. READY `launch=` parse cells: AC (row 13, DG-7).
- P2-S2 — `vsock_wire_codec::{every_control_message_round_trips, every_accepted_control_message_is_canonical, every_reserved_port_in_a_listen_state_is_refused, golden_control_messages_pin_every_type_and_refusal_code, malformed_control_messages_are_refused_with_their_cause, kinds_and_refusals_have_distinct_labels}` — MISSING_FUNCTIONALITY.
- P2-S3 … S9, S11, S13 — AC (rows 1-3, 7-9): owner-flow tests over `SimGuestFlowIo`, asserting O3 `flow_observation()` and S `closed_by_owner`, `outbox`.
- P2-S10, P2-S12 guest halves — `overdrive-init` `guest_flow::tests::tcp_accept::{tcp_accept_with_the_guests_token_pairs_in_the_pinned_order, tcp_accept_with_another_launchs_token_closes_v_g_silently, refused_application_connect_answers_application_not_listening, a_v_g_the_host_closed_after_the_request_ends_with_no_c_and_no_message, a_queued_v_g_closed_by_the_host_never_reaches_the_guest_and_its_abort_is_discarded, control_messages_for_unknown_or_closed_flows_are_discarded_without_reply, abort_of_a_routed_flow_resets_the_application_leg, a_connect_completing_after_session_loss_sends_nothing_and_closes_c}`, `tcp_connect::{a_captured_connect_opens_v_g_writes_the_request_once_and_routes_on_paired, a_capture_without_a_parking_cell_is_reset_locally, a_refused_tcp_connect_resets_the_application, an_application_reset_before_paired_aborts_and_sends_abort, an_early_application_eof_is_carried_out_at_paired}`, `half_close::{the_flow_vsock_is_shut_for_writing_only_after_half_close_ack, a_peer_half_close_is_acked_and_the_outer_leg_shut_after_the_vsock_eof, an_unannounced_vsock_end_aborts_and_resets_the_application, a_drain_that_misses_the_deadline_aborts_the_flow, a_clean_close_clears_linger_before_closing}`, `newtypes::{socket_cookie_roundtrips, cell_generation_roundtrips, non_decimal_input_is_refused, zero_cookie_and_overflow_are_refused}` — MISSING_FUNCTIONALITY. Host halves: AC (rows 2, 3, 7, 9).
- P2-S14 — additive half `overdrive-worker` `vsock_beacon_intake::{binding_the_intake_binds_the_beacon_listener, a_bind_failure_is_typed_and_binds_nothing, a_fresh_driver_reports_an_empty_beacon_observation}` — MISSING_FUNCTIONALITY; `the_bind_refusal_renders_its_pinned_message` — default-lane GUARD; `overdrive-sim` `adapters::beacon_io` tests `a_guest_cannot_connect_before_the_listener_is_bound`, `a_queued_ready_is_observed_then_accepted_and_read`, `a_queued_connection_with_nothing_buffered_or_no_valid_token_projects_none`, `an_accepted_connection_whose_vm_died_fails_its_next_io`, `a_parked_read_fails_when_its_vm_dies`, `a_queued_connection_whose_vm_died_delivers_its_buffered_ready`, `a_line_over_4096_bytes_is_invalid_data_and_the_connection_stays`, `the_guest_closing_is_end_of_stream_after_its_buffer`, `dropped_calls_lose_no_connection_and_no_line`, `stepped_mode_holds_every_delivery_until_granted` — MISSING_FUNCTIONALITY (RED scaffold); `bind_is_idempotent`, `a_scripted_bind_failure_binds_nothing_and_is_one_shot`, `accept_before_bind_is_refused`, `a_handle_another_adapter_issued_is_unreachable` — default lane, pass. Attribution cells and the `flows_beacon` driver: AC (rows 13, 15).
- P2-S15 — guest-side drivers `guest_flow::quint_owner_flows::quint_owner_flows_ok_guest`, `guest_flow::quint_flows_half::quint_owner_flows_half_guest` — MISSING_FUNCTIONALITY; authored, model fidelity open (UI-R2-5) — a correct implementation may fail them until the model decision. Host side, `flows_half_host`, `flows_beacon`: AC.

### P3 — datagram associations, UDP slots, framing

- P3-S1 — `overdrive-core` `vsock_datagram_frame::{every_payload_up_to_the_maximum_round_trips, every_accepted_byte_string_is_canonical, every_non_canonical_frame_is_refused, golden_empty_frame_is_the_header_alone, golden_maximum_frame_carries_the_inclusive_bound, the_spike_sizes_round_trip, a_payload_over_the_maximum_is_refused_oversize_by_new, a_declared_length_over_the_maximum_is_refused_oversize_by_decode, fewer_bytes_than_the_header_are_refused_truncated, a_wrong_magic_is_refused_bad_magic, a_body_shorter_or_longer_than_declared_is_refused_length_mismatch}` — MISSING_FUNCTIONALITY; `refusals_render_their_pinned_messages` — default-lane GUARD.
- P3-S2, S4, S5, S6, S9 (guest half), S10 — `overdrive-init` `guest_flow::tests::udp_slots::{one_slot_per_socket_destination_pair, an_idle_association_is_released_and_a_busy_one_kept, a_lost_release_report_is_recovered_by_the_audit, a_release_report_releases_every_slot_of_the_socket, recreated_cells_discard_parked_frames_and_stale_generation_events, a_cell_creation_failure_quarantines_the_slot_until_the_audit_retry, a_refused_burst_keeps_the_slot_and_starts_one_association_per_send, an_aborted_paired_association_reassociates_only_on_the_next_datagram, the_pairing_deadline_aborts_the_association_and_keeps_the_slot, a_late_paired_for_a_released_slot_is_answered_with_abort, exhaustion_refuses_a_new_destination_and_leaves_existing_associations}` — MISSING_FUNCTIONALITY.
- P3-S3 — AC: P6-S13 cell (needs a real guest kernel; rows 1-6).
- P3-S7, P3-S9 host half — AC (rows 2, 3, 7, 9; new-step 10).
- P3-S8 — guest-side driver `guest_flow::quint_udp_slots::{quint_udp_slots_guest_slots_ok, quint_udp_slots_guest_slots_live}` — MISSING_FUNCTIONALITY; authored, model fidelity open (UI-R2-5) — a correct implementation may fail them until the model decision. Host side: AC.

### P4 — steering, intake, boot, landing

- P4-S1 … S5 — `overdrive-dataplane` `vsock_guest_steering::{only_a_steered_listener_is_reachable_at_the_prefix_declared_port, an_unassigned_prefix_address_reaches_no_host_socket, udp_to_a_prefix_address_reaches_no_host_socket, a_connection_arriving_on_a_device_is_dropped_even_for_a_steered_intake, closing_a_listener_frees_its_steering_slot_in_the_same_step, closing_steered_listeners_frees_every_slot_for_new_listeners, an_intake_outside_the_prefix_or_at_the_gateway_is_refused, a_socket_that_is_not_a_listener_bound_to_the_intake_address_is_refused, an_intake_steered_to_another_open_listener_is_refused_without_change, a_first_attach_pins_a_link_that_outlives_the_handle, boot_over_a_left_behind_pinned_link_swaps_atomically_and_never_detaches, a_host_local_client_whose_source_lies_in_the_prefix_completes_to_a_steered_intake}` and `guest_steering::{golden_steering_key_pins_the_layout, the_key_is_address_then_port_in_network_order_then_zero, distinct_intakes_have_distinct_keys}` — MISSING_FUNCTIONALITY; `guest_steering::the_steering_key_is_eight_bytes` — default-lane GUARD.
- P4-S6 … S11 — AC (rows 2, 3, 7, 9).
- P4-S12 — `overdrive-control-plane` `guest_network::prefix_boot_acceptance::{boot_runs_load_then_swap_then_route_and_a_verifier_reject_is_never_attached, a_failed_or_interrupted_boot_step_leaves_the_earlier_program_and_route_local_only_over_a_pinned_link, a_failed_route_write_refuses_startup_with_the_route_as_it_was, a_boot_crashed_at_any_steering_step_is_converged_by_the_next_boot}` — MISSING_FUNCTIONALITY. Failed step-8 table cell: AC (row 9).
- P4-S13 and the planned "`run_server` boot refusal" cell — AC (row 9).
- P4-S14 — `prefix_boot_acceptance::{a_vip_range_overlapping_the_prefix_refuses_before_any_boot_effect, probe_startup_refuses_a_vip_range_iff_one_of_its_ranges_meets_the_guest_prefix, a_node_address_inside_the_prefix_refuses_before_any_boot_effect, a_node_that_overlaps_the_prefix_by_address_and_by_vip_range_names_the_vip_range, node_addresses_outside_the_prefix_pass_probe_startup_without_effect, probe_startup_refuses_a_node_address_iff_one_lies_inside_the_guest_prefix}` — MISSING_FUNCTIONALITY.
- P4-S15 — `prefix_boot_acceptance::{the_intake_block_conflicting_with_the_ephemeral_range_or_a_wildcard_listener_refuses, probe_startup_refuses_iff_the_ephemeral_range_or_a_wildcard_port_meets_the_intake_block}` — MISSING_FUNCTIONALITY. `run_server` structural cell: AC (row 9).
- Other `probe_startup` refusals in the same file: `an_owner_outside_the_connect4_attach_path_refuses_before_any_boot_effect` (new-step 11), `an_rlimit_nofile_below_the_owner_budget_refuses_before_any_boot_effect` (new-step 7) — MISSING_FUNCTIONALITY.
- P4-S16 — covered by P4-S19.
- P4-S17 — driver `prefix_boot_conformance::{prefix_boot_conformance_boot_ok, prefix_boot_conformance_boot_faulty, prefix_boot_conformance_boot_live}` — MISSING_FUNCTIONALITY.
- P4-S18 `intake` driver — AC (rows 2, 3, 7, 8, 9).
- P4-S19 — `vsock_prefix_landing_conformance::native_metal_kvm_prefix_landing_matches_lands_for_every_client_class_and_reachable_configuration` — not run in this increment (native metal with KVM and `quint`; `quint` on the metal host is DELIVER new-step 1). Predicted class on qualified metal: MISSING_FUNCTIONALITY at the first serving comparison.

### P5 — egress unframe, fragments, host-internal deny, mesh egress

- P5-S1 — `overdrive-dataplane` `vsock_egress_unframe::{load_attaches_every_link_and_the_unframe_program_to_lo_only, load_refuses_the_root_cgroup, every_boot_interface_carries_unframe_and_datagrams_arrive_byte_exact, on_a_registered_tuple_only_a_valid_frame_is_removed, an_unregistered_datagram_passes_a_converged_interface_unchanged, oversize_and_malformed_guest_datagrams_are_dropped_and_counted, a_second_attach_to_an_attached_interface_fails_naming_it, an_interface_gone_before_its_attach_fails_naming_it_and_the_others_attach, a_descriptor_limit_at_attach_fails_as_attach_naming_the_interface}`; boot refusal `prefix_boot_acceptance::a_failed_boot_unframe_attach_refuses_startup_and_writes_nothing_else` — MISSING_FUNCTIONALITY.
- P5-S2 — `prefix_boot_acceptance::{serve_refuses_to_boot_on_a_stacked_root_namespace_interface, probe_startup_refuses_iff_some_root_namespace_interface_is_stacked}` — MISSING_FUNCTIONALITY.
- P5-S3 — `vsock_egress_unframe::guest_datagrams_survive_a_uso_interface_byte_exact` — MISSING_FUNCTIONALITY; `vsock_egress_unframe::the_forwarder_crate_never_references_udp_segment` — GUARD (passes, kept ignored with its step).
- P5-S4 — `vsock_fragment_map::{a_fragmented_guest_datagram_is_unframed_once_over_its_own_train, cut_train_then_unrelated_bare_fragmented_datagram_arrives_unshifted, map_filled_beyond_capacity_loses_whole_datagrams_never_corrupts, congested_qdisc_dropping_fragments_leaves_later_datagrams_byte_exact}` — MISSING_FUNCTIONALITY.
- P5-S5, S6, S7, S9 (owner decision) — AC (rows 2, 3, 7, 9, 14); P5-S9 end-to-end cell is P6-S13 (DG-16).
- P5-S8 — `overdrive-worker` `vsock_host_internal_enforce::{a_marked_tcp_connect_to_a_loopback_service_is_refused_econnrefused_without_hang, a_marked_tcp_connect_to_a_host_interface_address_is_refused, a_marked_udp_first_transmit_to_a_local_service_fails_eperm, a_marked_udp_socket_reusing_an_unmarked_flows_source_port_is_still_rejected, a_marked_udp_transmit_to_a_subnet_directed_broadcast_reaches_no_service, a_marked_udp_transmit_to_the_limited_broadcast_reaches_no_service, a_marked_udp_transmit_to_the_all_hosts_multicast_group_is_rejected_eperm, a_marked_tcp_connect_to_a_link_local_service_is_refused, a_packet_carrying_the_owner_bit_among_other_bits_is_refused, a_marked_connect_storm_across_repeated_table_replacements_reaches_no_local_service}` — MISSING_FUNCTIONALITY (behavioural: the marked connect / transmit succeeds today); `a_marked_tcp_connect_to_a_neighbour_stays_reachable`, `a_marked_udp_transmit_to_an_off_host_address_is_not_rejected`, `a_packet_carrying_only_the_leg_s_or_tproxy_mark_reaches_a_local_service` — controls (ignored with their step; pass against today's table); `overdrive-core` `dataplane::mtls_mark::{overdrive_marks_are_single_bits_and_pairwise_disjoint, overdrive_marks_and_table_carry_their_pinned_values}` — default-lane GUARD. `gateway:53` accept and guest-broadcast reject cells: AC, P6-S13 (UI-R2-14).
- P5-S10 — `overdrive-worker` `vsock_mesh_egress_mtls::{registered_mesh_flow_reaches_its_registered_backend_over_mtls_without_leg_f_resolving_again, a_replayed_peer_and_an_unregistered_peer_are_closed_by_leg_f_without_resolution, a_second_registration_for_the_same_forwarder_address_is_refused, an_allocation_without_a_live_intercept_cannot_register_a_forwarded_flow, dropping_an_unclaimed_registration_closes_the_flow_before_it_reaches_the_backend, dropping_a_claimed_registration_with_the_loopback_leg_open_resets_both_sides, dropping_a_claimed_registration_after_a_clean_close_keeps_the_remote_fin, forwarded_registration_refusals_follow_the_pinned_precedence, a_clean_close_of_both_directions_ends_the_remote_stream_with_fin, a_serve_exit_mid_flow_resets_the_remote_peer}` — MISSING_FUNCTIONALITY (the last through its serve child's scaffold panic at `register_forwarded_outbound`); `mtls_intercept_worker` lib test `forwarded_outbound_refusals_have_pinned_stage_labels` — MISSING_FUNCTIONALITY (RED scaffold at `MtlsInterceptInstallError::stage`).

### P6 — recovery, VMM, guest READY, walking skeleton

- P6-S1 … S6, S10, S12, S12b (feature disabled, C5a / C5b), P6-C1 `quiescence`, P6-C2 `shared_table` — AC (rows 2, 3, 7, 8, 9, 10, 12): runtime supervisor over the owner and `SimMtlsIntercept`; O3 `holders`, `admission`, `intake`, `flow_listeners_running`; O6 `recovery_observation()`; S `shared_table_state()`. The `SimMtlsIntercept` table semantics are default-lane tests that pass: `shared_table_state_is_absent_until_a_transaction_completes_it`, `a_failed_table_transaction_leaves_the_table_as_it_was`, `clear_faults_disarms_a_pending_table_failure`.
- P6-S7 — withdrawn (DG-1).
- P6-S8 — withdrawn with the VMM backend probe (DG-1); its `fd=` handoff obligations are P1-S12's VMM cell (AC, row 6) and P6-S13.
- P6-S9 — DELIVER new-step 1 provisioning (the provisioning checksum is the identity check); no DISTILL body.
- P6-S11 — `overdrive-init` `transport::tests::{well_formed_token_parses_and_other_parameters_are_ignored, missing_and_duplicate_tokens_are_refused, first_invalid_position_names_the_field, missing_trailing_field_names_the_first_missing_one, token_rendering_roundtrips, configure_runs_every_step_in_the_pinned_order, configure_stops_at_the_first_failure_with_that_steps_variant}` and `guest_flow::tests::session::{session_opens_with_a_control_open_carrying_the_guests_own_token, listen_state_reports_the_full_map_then_each_change, host_owner_restart_resets_flows_and_reopens_the_session_without_fatal, every_session_loss_shape_is_recovered_not_fatal, only_event_wait_and_listener_failures_are_fatal}` — MISSING_FUNCTIONALITY; `transport::tests::{every_setup_error_renders_its_pinned_display_text, label_enums_render_lower_snake_case}`, `session::guest_owner_fatal_renders_its_pinned_display_texts` — default-lane GUARD. `main` wiring cell: AC (row 16).
- P6-S13, S14, S15 — AC (rows 1-6): `run_server` + public API on native metal.

## Raw evidence

Run logs are host-local and not committed. Classifier: the first `panicked
at` location per test; `src/` + "RED scaffold" → MISSING_FUNCTIONALITY; a
test-file assertion → inspected by hand (the four deploy-validation
assertions report the parser accepting the spec; the ten host-internal
assertions report the marked connect or transmit succeeding).

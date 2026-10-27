# P6 — Real-kernel mTLS ports and firewall (netns-density-295, carried from the earlier pass)

Owners: `MtlsInterceptWorker` / `HostMtlsIntercept`
(`overdrive-worker`), `HostMtlsEnforcement` (leg-B dial),
`ServiceBackendsResolve` (VIP branch). Tier-3 on the Lima VM (Linux 7.0)
unless noted; every integration binary is in `host-kernel-shared`.
Example-based (Mandate 9, layer 3+); each sad path a named test.

Markers name the delta's DELIVER steps: 8 mesh registration, 11 VIP branch, 12 firewall single cut, 13 leg-B dial, 14
host-internal deny set.

The redirect-era forwarder cells (`vsock_forwarder_verdict`,
`vsock_seqpacket_length`, `vsock_flow_end_classifier`, `vsock_drain`) were
deleted with the sockmap forwarder (EVMOD-ADOPT); the forwarder's Tier-3
suite is P2 (`overdrive-fwd`).

## mTLS and firewall (`crates/overdrive-worker/tests/integration/`)

| File / tests | Contract | Pending |
|---|---|---|
| `vsock_host_internal_enforce` (13 cells) | P5-S8 D26-PINS F-1..F-6 through `start_shared_owner` on real nft | step 14 |
| `vsock_mesh_egress_mtls` (12 cells) | P5-S10 forwarded-outbound port (DG-10 readings); the existing half-ended claimed-registration cell observes the peer socket's exact post-FIN `TCP_CLOSE` + pending `SO_ERROR=EPIPE`, never a second read after EOF (PFRO-1) | step 8 |
| `vsock_legb_dial::a_same_node_mesh_dial_is_marked_source_bound_and_reaches_leg_c` | LEGB-DIAL / R5-43 leg-B half | step 13 |
| `vsock_legb_dial::a_dial_to_a_peer_outside_the_guest_prefix_is_neither_marked_nor_bound` | V-LEGB-MARK S5 | step 13 |
| `vsock_legb_dial::a_marked_dial_to_an_unregistered_tuple_is_refused_and_reaches_no_host_socket` | S3 fail-closed | step 13 |
| `vsock_constant_guest_prefix_rules::a_host_local_connection_to_a_guest_prefix_address_is_rejected_at_once`, `…datagram…fails_eperm`, `a_remote_connection_to_an_unassigned_guest_prefix_address_is_dropped`, `a_leg_c_diverted_remote_connection_reaches_leg_c_both_ways` | P4-S5 / ADR-0172 / R5-43 | step 12 |
| `mtls_intercept_install` (D15 bodies), `shared_intercept_members` (S-ND295-61), `netns_density_shared_owner` (4 bodies) | inbound-only owned program after the firewall single cut | step 12 |
| `outbound_enforce_substrate_splice` (2 retained shared-capability lifecycle bodies) | Real-kernel D9 forwarded registrations: independent shared handles and owner shutdown/claim drain; the deleted D14 per-allocation egress-TPROXY splice scenario is not restored | step 13-01 fixture alignment |
| `mtls_intercept_equivalence` | inbound-only guard release / refusals | **active** |
| `mtls_intercept_worker::tests::forwarded_outbound_refusals_have_pinned_stage_labels` | DG-10 reading 1 | step 8 |
| `mtls_resolve_adapter::tests` VIP branch (5 cells) | P5-S7 D24a over the configured `vip_ranges` | step 11 |

## Deleted (old contract)

`vsock_egress_unframe.rs`, `vsock_fragment_map.rs`, `vsock_guest_steering.rs`
and `helpers/guest_net.rs` (list C); `tests/conformance` `prefix_landing` and
its test; the worker TAP-era egress capture tests
`egress_tproxy_capture.rs`, `start_alloc_installs_both_tproxy.rs`,
`bidirectional_walking_skeleton.rs` and
`outbound_enforce_substrate_splice::outbound_enforce_substrate_bidirectional_splice_zero_copy`
with its strace oracle, wire scan and topology helpers (they assert the
per-allocation egress nft-TPROXY capture that D14 removes; the forwarded
outbound port replaces it). The `undetermined_tap_quiescence…` conformance
test was deleted; `shared_owner_fail_stop_shuts_down_before_a_fresh_handler_reopens_admission`
was re-derived onto `FlowListeners`.

## Blockers

PFRO-1 corrects only the existing half-ended claimed-registration cell's
post-FIN evidence contract. The same peer socket first consumes leg B's FIN;
after the owner drops the claimed registration, its terminal observation must
be `TCP_CLOSE` with pending `SO_ERROR=EPIPE` on Linux 7.0. A read after EOF is
not evidence for or against the later reset because Linux checks `SOCK_DONE`
before `sk_err`. The sibling controls remain exact: an open drop is `Reset`
before FIN, while both clean-close cells require terminal `TCP_CLOSE` with
zero pending socket error. This adds no scenario, lane, model, production
accessor, or test seam. The bounded source and handoff are recorded in
`../deliver/distill-13-01-postfin-reset-alignment.md`.

The STEP 13-01 package gate also found three retained shared-owner lifecycle
bodies connecting to leg F without D9 registration. Their production refusal
is correct. DISTILL retained all three bodies and every shutdown, drain,
listener, member, node-program, traffic, and unrelated-handle assertion, while
migrating only the source-bound client setup through the existing
`register_forwarded_outbound` port. The two real-kernel bodies keep their
already-selected `AllMeshResolve` backend as the registration backend; the
recording-owner body keeps its pre-existing loopback backend. No deleted D14
TPROXY/pass-through scenario, public API, seam, lane, or body was restored or
added.

After registration alignment, the first focused replay passed the recording
owner body and exposed only the real peer recorder's inverted teardown
taxonomy: it accepted EOF/`UnexpectedEof` but rejected the abortive reset that
the retained `SO_LINGER{1,0}` contract requires. A diagnostic replay observed
errno 104 / `ConnectionReset` for all three owner-shutdown peers and the first
stopped allocation. Those exact peers are now `AbortRequired` and accept only
`ConnectionReset`, `ConnectionAborted`, or `BrokenPipe` after their mTLS
handshake completes. The unrelated second client is dropped before its final
allocation stop and alone permits clean/truncated EOF or the same reset class.
Handshake/authentication error, failed echo, timeout, and every other error
fail. Existing byte-exact exchanges still prove application traffic before
each stop or owner shutdown, and the third shutdown claim can pass only after
its server handshake completes.

LEGS-DIAL (D-14, user 2026-10-19) moved the guest dial from the worker to
the enforcement: `HostMtlsEnforcement::new(identity, limits, guest_prefix,
guest_dial)`, `MtlsInterceptWorker::new` with four ports. R5-54's seeded half
is authored (`overdrive-control-plane/src/guest_network/owner_acceptance/legs_dial.rs`);
its Tier-3 half stays unauthored (`upstream-issues.md` T-9). The leg-F
repair's real-kernel cell is `vsock_leg_f_repair.rs` (D-9, 14-02). The
guest-program Tier-3 cells run through the serve + deploy lanes
(`vsock_walking_skeleton`, `vsock_fault_matrix`, `vsock_density`) and the
in-guest lane.

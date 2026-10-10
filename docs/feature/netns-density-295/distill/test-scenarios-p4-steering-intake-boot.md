# P4 — Guest-prefix steering, intake listeners, boot convergence & landing

> **DISTILL status (2026-10-10) — supersedes the file-placement, "parked" and
> "blocked" statements of earlier passes.** Every round-1 `UI-*` item that
> blocked P4 (UI-C1, UI-C2, UI-COV-1, UI-CORE-1, UI-PL-1..7) is closed by a DG
> decision (DG-2, DG-3, DG-9, DG-13, DG-14). Per DG-2 each scenario is either
> **authored now** (compiled, `#[ignore = "pending DELIVER vsock …"]`) or
> **authored at the attachment cut** (its body calls a DG-2 replacement row and
> cannot compile before the cut; plan in feature-delta § *Attachment-cut
> DISTILL increment*):
>
> - authored: P4-S1..S5 (`overdrive-dataplane` `vsock_guest_steering`, Tier-3,
>   plus the `guest_steering.rs` key-encoder unit tests); P4-S12, P4-S14,
>   P4-S15 (`overdrive-control-plane` `guest_network::prefix_boot_acceptance`,
>   source-local); P4-S17 (`guest_network::prefix_boot_conformance`); P4-S19
>   (`tests/conformance/tests/vsock_prefix_landing_conformance.rs`, native
>   metal).
> - attachment cut: P4-S6..S11 and the P4-S18 `intake` driver (DG-2 rows 2, 3,
>   7, 8, 9); P4-S13, the P4-S15 `run_server` structural cell and the P4-S12
>   failed step-8 table cell (row 9).
> - P4-S16 is covered by P4-S19 (`vsock_prefix_exposure.rs` no longer exists).
>
> The steering failure classes the Tier-3 adapter cannot induce (verifier
> reject, `Update`, `Pin`, `KernelMemory`, `MapUpdate`) are scripted on
> `SimGuestFlowIo` (DG-3, closes UI-COV-1). Classes: `red-classification.md`.

DISTILL scenario specification (netns-density-295, vsock Attachment Replacement
DESIGN). This doc is the authored DISTILL product for piece P4; the per-scenario
Rust bodies live in the files below (§ *Files owned*), authored now or at the
attachment cut as stated per scenario. No `.feature` files (project rule);
scenarios are GWT spec here and each is a Rust `#[test]` / `#[tokio::test]`.

Scope: `workload_addr` host-local delivery via the pinned `sk_lookup` steering
program over a listener-keyed socket map (D8, D8a, D8a-LOOKUP), boot
load→swap→route convergence with crash-between-steps (D8a-LOAD-SWAP, U-4),
intake listeners mirrored from the guest listen-state (D23) bound in the fixed
host-port block (INTAKE-BIND), all-or-nothing activation (U-6), the constant nft
reply-accept (PREFIX-REPLY), startup overlap refusals (D8-PREFIX-CFG,
D8-PREFIX-VIP, INTAKE-BIND block conflict), and the serve-created veth pair
carrying the unframe link (D5a-SERVE-IFACE, R5-39 — the attach itself is P5).

Owns decisions: D8, D8-INTAKE, D8-PEER, D8-PREFIX-CFG, D8-PREFIX-VIP, D8a,
D8a-LOOKUP, D8a-LOAD-SWAP, D8a-CAP (steering side), D23, INTAKE-BIND,
PREFIX-REPLY, D5a-SERVE-IFACE, U-3, U-4, U-6, M-7. Gates G-V1 (steering/boot/route
half) + G-V3's listener clause + G-V8 (intake listener). Quint oracle
`prefix_boot` (inv 6), `intake` (inv 7, 8, 12), `prefix_landing` (landing table).
R5-1, R5-2, R5-3, R5-5 (`ListenState` half), R5-11, R5-13 (intake-child/V-20),
R5-22, R5-23, R5-24, R5-25, R5-29, R5-34, R5-37 (steering half), R5-38 (stacked
half), R5-39, R5-40, R5-42, R5-43, R5-44. V-19, V-12 (DELIVER-step, RED), V-26
(done P-40, restated as Tier-3 CI).

See [shared-test-infrastructure.md](shared-test-infrastructure.md) for the
harness, Sim-adapter, quint-driver, real-kernel-fixture and entrypoint contracts.

---

## Files owned (disjoint) + registration contract

Authored now (compiled and registered; no always-false `cfg`, no unregistered
file):

| File | Lane | nextest group | Scenarios |
|---|---|---|---|
| `crates/overdrive-dataplane/tests/integration/vsock_guest_steering.rs` | Tier-3 (`sk_lookup` kernel facts) | `host-kernel-shared` (whole `integration` binary) | P4-S1..S5 |
| `crates/overdrive-dataplane/src/guest_steering.rs` unit tests | default lane (pure key encoder, DG-14) | — | steering key layout read by P4-S19 |
| `crates/overdrive-control-plane/src/guest_network/prefix_boot_acceptance.rs` (source-local, `#[cfg(test)]`) | seeded sim over `SimGuestFlowIo` / `SimGuestNode` (default lane) | — | P4-S12, P4-S14, P4-S15 (+ P5-S1, P5-S2 legs) |
| `crates/overdrive-control-plane/src/guest_network/prefix_boot_conformance.rs` (`#[cfg(all(test, feature = "integration-tests"))]`) | quint-connect `#[quint_run]` (Lima) | — | P4-S17 |
| `tests/conformance/tests/vsock_prefix_landing_conformance.rs` | real kernel, native metal (`quint repl` oracle, no quint-connect trace) | standalone binary, `host-kernel-shared` | P4-S19 (covers P4-S16) |

Authored at the attachment cut (DG-2; written in the cut's working tree before
the crafter lands it), source-local in `overdrive-control-plane` beside
`HostSharedGuestNetworkOwner`:

| Item | DG-2 rows | Scenarios |
|---|---|---|
| Intake tests (starting draft: `overdrive-control-plane/tests/acceptance/vsock_intake_listener.rs` @91f9436ff, removed) and the `intake` driver | 2, 3, 7, 8, 9 | P4-S6..S11, P4-S18 |
| `run_server` composition cells (the AC table names no starting draft; the earlier `tests/acceptance/vsock_prefix_boot_order.rs` placement was removed) | 9 | P4-S13, P4-S15 structural cell, P4-S12 failed step-8 table |

The earlier `tests/conformance/tests/vsock_prefix_boot_conformance.rs` and
`vsock_intake_conformance.rs` (`#![cfg(any())]`-gated) do not exist: drivers are
in-crate modules of their owner (QPROJ-1), and DG-2 forbids an uncompiled,
unregistered or always-false-`cfg` file.

---

## Lanes & PBT mode (Mandate 9 / 11)

| Layer | What | Mode |
|---|---|---|
| Unit (layer 1) | `guest_intake_host_port` injectivity; the vip/prefix overlap predicate | **PBT** (`proptest`) |
| Seeded sim (layer 3, production owner path) | intake lifecycle, boot order, overlap refusals | **example-based** (sad paths enumerated, never PBT-generated) |
| Tier-3 (layer 5-6) | steering kernel facts; prefix landing (P4-S19, native metal) | **example-based** (1-2 representative) |
| Quint conformance (Lima) | `prefix_boot` / `intake` trace replay; `prefix_landing` oracle table | `#[quint_run]` randomized traces only (seed on failure); no `#[quint_test]` — the specs define no Quint `run` (UI-R2-1) |

Sad-path share: S1/S2 refusals, S3 cap-E2BIG, S4/S12 failed swap/load, S7 rollback,
S8 steer-failure classes, S12 crash matrix, S14 overlaps, S15 block conflict, S16
exposure-down — well over the ≥ 40 % error/edge target.

---

## Scenario specification (P4-S1 … P4-S19)

Each row: GWT (domain language) · contract/invariant · production owner path ·
fault stimulus · oracle · observable universe (Mandate 8) · evidence tier.

### Steering kernel facts — Tier-3 (`vsock_guest_steering.rs`, `host-kernel-shared`)

Status (P4-S1..S5): authored in
`crates/overdrive-dataplane/tests/integration/vsock_guest_steering.rs` over a
`TestCidrLease` `/24` prefix, every test ignored `pending DELIVER vsock
new-step 9 (guest-prefix steering)` except
`boot_over_a_left_behind_pinned_link_swaps_atomically_and_never_detaches`
(`pending DELIVER vsock new-step 13 (steering load and swap)`). The 8-byte
steering key the landing harness decodes (DG-14) is pinned by the
`crates/overdrive-dataplane/src/guest_steering.rs` unit tests:
`the_steering_key_is_eight_bytes` (default lane, not ignored, GUARD) and
`golden_steering_key_pins_the_layout`,
`the_key_is_address_then_port_in_network_order_then_zero`,
`distinct_intakes_have_distinct_keys` (ignored `pending DELIVER vsock new-step 9
(guest-prefix steering)`). Failure classes the adapter test cannot induce
(verifier reject, `Update`, `Pin`, `KernelMemory`, `MapUpdate`) are scripted on
`SimGuestFlowIo` (DG-3) in P4-S12 / P4-S17 and in the intake tests at the cut.

Adapter-integration: drives the real `GuestPrefixSteering` adapter against the
real kernel with hand-bound listeners standing in for the owner's intakes. The
full-composition "reaches the guest application" fact is P4-S16 / the P6 WS.

- **P4-S1 — only a steered listener is reachable at the prefix declared port** `@contract-shape:bounded-change`
  (`only_a_steered_listener_is_reachable_at_the_prefix_declared_port`; R5-1,
  R5-2 steering half; G-V8; V-26 K-L1/K-L5/K-L10).
  *Given* the boot steering is attached, a wildcard host service on the declared
  port, and no entry; *When* a host-local connect reaches the prefix with no
  entry; *Then* the kernel drops the lookup (TCP reset) and never the wildcard
  host service (B-2 closed). *When* the owner binds the intake on
  `workload_addr:guest_intake_host_port(0)` and `steer((workload_addr, p), fd)`;
  *Then* a connect to `workload_addr:p` reaches the intake and exchanges both
  ways, and a connect to the listener's own bound block port is reset (decided by
  its own absent entry — K-L10). · **Contract:** D8a-LOOKUP decision function;
  INTAKE-BIND. · **Owner path:** `GuestPrefixSteering::{load,attach,steer}`. ·
  **Fault:** none (happy + the no-entry refusal). · **Oracle:** real `lo`
  `connect()` outcome + byte-distinct REQUEST/RESPONSE exchange. · **Universe:**
  `{connect(workload_addr:p) class, connect(bound_block_port) class,
  wildcard_service.accepted}`. · **Tier-3.** · **Status:** authored
  (`only_a_steered_listener_is_reachable_at_the_prefix_declared_port`, new-step 9).

- **P4-S2 — an unassigned/released prefix address reaches no host socket** `@contract-shape:unbounded-preservation`
  (`an_unassigned_prefix_address_reaches_no_host_socket`,
  `udp_to_a_prefix_address_reaches_no_host_socket`,
  `a_connection_arriving_on_a_device_is_dropped_even_for_a_steered_intake`;
  R5-3, K-L1; G-V1/G-V8).
  *Given* steering attached + a wildcard host service; *When* a host-local
  connect targets a prefix address with no lease/entry (with and — the DELIVER
  Tier-3 cell — without the nft constant rules); *Then* the lookup is dropped and
  no host socket is reached (the steering alone keeps the prefix closed). ·
  **Oracle:** `connect()` → `ConnectionRefused`. · **Tier-3 (V-19, V-26).** ·
  **Status:** authored (the three tests above, new-step 9). The adapter file has
  no nft rules installed at all, so every P4-S2 cell is the "steering alone"
  case; the "with the constant rules" case is P4-S19's landing table.

- **P4-S3 — a port stops accepting the moment its listener closes, and its capacity is reusable at once** `@contract-shape:bounded-change`
  (`closing_a_listener_frees_its_steering_slot_in_the_same_step`,
  `closing_steered_listeners_frees_every_slot_for_new_listeners`; steer
  preconditions `an_intake_outside_the_prefix_or_at_the_gateway_is_refused`,
  `a_socket_that_is_not_a_listener_bound_to_the_intake_address_is_refused`,
  `an_intake_steered_to_another_open_listener_is_refused_without_change`; R5-22
  close, D8a-CAP, K-L3; G-V8). *Given* a steered listener reachable; *When* the
  listener closes; *Then* the kernel removes the entry in the same step and the
  next connect is reset; a fresh listener re-steered on the same port is reachable
  at once (freed slot). *Given* the map at `GUEST_INTAKE_STEERING_MAX`; *When* one
  more insert; *Then* `MapUpdate` (E2BIG); *When* one earlier listener closes;
  *Then* the insert succeeds. · **Contract / invariant:** closing the listener removes its steering entry
  in the same step, entry lifetime = listener lifetime (K-L3); the map
  capacity cycles (D8a-CAP). · **Oracle:** `connect()` class + `GuestSteeringError`
  variant. · **Tier-3 (V-26; the capacity-fill is a DELIVER real-kernel fixture).**
  · **Status:** authored (the five tests above, new-step 9). The authored
  slot-reuse cell steers 256 intakes, closes them and re-steers them; the
  E2BIG-at-`GUEST_INTAKE_STEERING_MAX` (262,144) fill is not authored as a
  Tier-3 cell. The `MapUpdate` refusal class reaches the owner through
  `SimGuestFlowIo` (DG-3) in the intake tests and driver at the cut (DG-2 rows
  2, 3, 7, 8, 9).

- **P4-S4 — a node upgrading its binary never exposes the prefix to a host service, and a failed upgrade keeps the earlier steering** `@contract-shape:unbounded-preservation`
  (`a_first_attach_pins_a_link_that_outlives_the_handle`,
  `boot_over_a_left_behind_pinned_link_swaps_atomically_and_never_detaches`;
  R5-24/R5-34,
  D8a-LOAD-SWAP, K-L9; G-V1). *Given* a left-behind pinned link running an earlier
  program + `sshd` on `0.0.0.0:22`; *When* a new binary boots (`load` → `attach`
  `BPF_LINK_UPDATE`); *Then* the link is never detached and a connect-loop across
  the swap never reaches `sshd`, decided only by the earlier or the new program.
  *When* the swap is scripted to fail; *Then* `GuestSteeringError::Update` and the
  earlier program stays attached (K-L9). · **Contract:** pinned steering link +
  atomic program swap (`BPF_LINK_UPDATE`), never a detach (D8a-LOAD-SWAP). ·
  **Oracle:** pinned-link presence
  (`bpftool`), connect class across the swap, error variant. · **Tier-3 (V-26
  K-L9; the concurrent connect-loop + scripted update are DELIVER fixtures).**
  `ingress_ifindex` distinguishes loopback from device (V-26) is asserted by the
  remote-vs-loopback cells here (P4-S2's device-ingress test). · **Status:**
  authored (`a_first_attach_pins_a_link_that_outlives_the_handle` new-step 9;
  `boot_over_a_left_behind_pinned_link_swaps_atomically_and_never_detaches`
  new-step 13). The scripted failed swap (`GuestSteeringError::Update`, earlier
  program stays attached) is not a Tier-3 cell: a real kernel cannot be made to
  fail `BPF_LINK_UPDATE` on demand, so it is driven on `SimGuestFlowIo` (DG-3)
  by P4-S12 `a_failed_or_interrupted_boot_step_leaves_the_earlier_program_and_route_local_only_over_a_pinned_link`
  and the P4-S17 `boot_faulty` instance.

- **P4-S5 — a host-local client whose address lies in the prefix completes its connection to a guest service** `@contract-shape:bounded-change`
  (`a_host_local_client_whose_source_lies_in_the_prefix_completes_to_a_steered_intake`;
  R5-43, PREFIX-REPLY; G-V8). *Given* the production constant rules (the
  established/related accept sits before each reject/drop, after the leg-S
  exemption and the leg-C divert) + a steered intake; *When* a host-local client
  whose SOURCE lies in the prefix (the gateway) connects; *Then* the connection
  completes and exchanges both ways (replies pass whatever their address); *And*
  a NEW unmarked host-local connect with no entry is still rejected at once. ·
  **Contract:** the reply path for an in-prefix source (PREFIX-REPLY). ·
  **Oracle:** connect-from-source outcome + exchange; fresh-connect reset. ·
  **Tier-3.** · **Status:** authored
  (`a_host_local_client_whose_source_lies_in_the_prefix_completes_to_a_steered_intake`,
  new-step 9) as the steering half only: the constant nft reply-accept rules
  are the owner's and are not installed in the adapter file. The rules half
  (replies pass; a fresh unmarked connect is rejected) is owed to the
  production composition: P4-S19's landing classes (unmarked host-local
  connects) and the P6-S13 walking skeleton at the cut.

### Intake listener lifecycle — seeded sim (attachment cut, source-local in `overdrive-control-plane`)

**Status (P4-S6..S11): attachment cut (DG-2 rows 2, 3, 7, 8, 9).** The bodies
call `assign(a, &ports)`, the `GuestNetworkPlan` reshape, `ForwardingActivation`,
`quiesce_forwarding` / `restore_forwarding` and
`HostSharedGuestNetworkOwner::new(ports)`, so they are written in the cut's
working tree, source-local beside the owner (starting draft
`overdrive-control-plane/tests/acceptance/vsock_intake_listener.rs` @91f9436ff,
removed). Per the feature-delta *Attachment-cut DISTILL increment*: they drive
`assign(a, ports)` → `provision` → session + `ListenState` → `activate` with
scripted bind / steer outcomes, and assert O3 `flow_observation()` `intake`,
`parked`, `last_listen_state` and the `SimGuestFlowIo` arbiter reads
`steering_observation().entries` and `steering_effects()`. Test names below are
the draft names.

Drives the production owner (`provision`/`activate`/`teardown`/
`quiesce_forwarding`/`restore_forwarding`) through `with_flow_io` over
`SimGuestFlowIo` (DG-3; the steering seam is part of `GuestFlowIo`, there is no
separate `SimGuestPrefixSteering`) and a scripted `ListenState`/session feed.
Universe = owner observable surface (never private fields): the abstract
`{cid_state[cid], open_intake[cid], listener_bind[(cid,port)], steer_entries,
bind_failed_events}`, read through the accessors above.

- **P4-S6 — `ListenState` interleavings yield the invariant state** `@contract-shape:bounded-change`
  (`a_listen_state_during_activation_teardown_or_quiescence_yields_the_invariant_state`;
  R5-5 listen half, M-7; G-V3/G-V8; intake inv 7). The design serving invariant —
  `serving(port) ⇔ Active ∧ no quiescence holder ∧ session Live ∧ last
  ListenState(port)=listening ∧ port ∈ listen_ports()` — holds for every
  interleaving; per-allocation serialization. · **Fault:** event ordering. ·
  **Oracle:** before/after universe delta = exactly the steered listeners for
  reported-listening ports; fail-closed (no other CID moved). · **Seeded sim.**

- **P4-S7 — all-or-nothing activation rollback** `@contract-shape:unbounded-preservation`
  (`activation_with_a_failing_steer_rolls_back_wholly_and_leaves_the_cid_provisioned`;
  R5-29, U-6; G-V3/G-V8; intake inv 8). *Given* three reported-listening ports,
  the second `steer` a defect; *Then* `activate` returns `Err`, the CID stays
  Provisioned, and NO listener / entry of the CID remains (the rollback closes
  listeners and cannot fail). · **Fault:** scripted `MapUpdate` defect. ·
  **Oracle:** `cid_state=Provisioned`, `open_intake[cid]=∅`, entry-set unchanged
  (fail-closed). · **Seeded sim.**

- **P4-S8 — a port the kernel could not steer stays refused and recovers on retry** `@contract-shape:bounded-change`
  (`a_kernel_memory_steer_failure_is_retried_at_the_audit_cadence`,
  `a_defect_steer_failure_is_not_retried_and_serves_again_only_after_the_next_listen_state`,
  `teardown_returns_ok_without_waiting_on_any_steering_write`; R5-22,
  D8a-LOOKUP/D8a-CAP; G-V8). `KernelMemory` × N → port refused meanwhile,
  listener closed, `bind_failed { retried: true }`, retried at audit cadence,
  serves after N+1; a defect (`MapUpdate`) → closed, `retried: false`, no retry,
  serves again only on the next `ListenState`; teardown returns `Ok` without
  waiting on any steering write. · **Contract:** steer failure classes —
  `KernelMemory` retried, a defect not retried (D8a-LOOKUP). · **Fault:**
  scripted `KernelMemory`/`MapUpdate`.
  · **Oracle:** `bind_failed_events` retried-flag + `steer_entries` presence +
  `cid_state`. · **Seeded sim.**

- **P4-S9 — intake bind in the fixed block** `@contract-shape:bounded-change`
  (`every_intake_listener_binds_the_workload_address_in_the_fixed_block`;
  `intake_host_port_properties`; R5-42, INTAKE-BIND; G-V1/G-V8). Every listener
  binds `workload_addr:guest_intake_host_port(i)` for the declared port's
  ascending index (never `0.0.0.0`, never the declared port); two VMs declaring
  the same ports bind the same block ports on their own addresses and both serve.
  **PBT (layer 1):** `guest_intake_host_port` injective over
  `0..GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX`, lands in `guest_intake_host_ports()`,
  `None` beyond. · **Oracle:** `listener_bind[(cid,port)] == workload_addr:(base+i)`.
  · **Seeded sim + proptest.**
  · **Status:** the PBT half is authored in `overdrive-core`
  `tests/acceptance/vsock_guest_transport_vocabulary.rs`
  (`intake_host_ports_are_injective_inside_the_block`,
  `indices_beyond_the_block_map_to_nothing`,
  `the_intake_host_block_sits_below_the_host_ephemeral_range_and_apart_from_the_guest_reservation`;
  ignored `pending DELIVER vsock new-step 9 (intake host ports, INTAKE-BIND)`);
  the owner bind half is at the attachment cut (rows 2, 3, 7, 9).

- **P4-S10 — only connections to a workload's own intake are treated as intake traffic, never an mTLS connection on the same address and port** `@contract-shape:bounded-change`
  (`only_intake_listeners_are_tagged_never_a_leg_c_child_on_the_same_address_port`;
  R5-13, V-20, INTAKE-TAG-MAP; G-V8). The owner tags exactly its intake listeners;
  a leg-C transparent child sharing the local address/port is never in the
  clone-flagged intake-tag map. (The kernel `sock_ops` fact — V-20 HOLDS, P-46 —
  is the P6 WS Tier-3 cell; here the owner contract is pinned.) · **Contract:**
  intake-child identity — the clone-flagged intake-tag map holds exactly the
  owner's intake listeners (INTAKE-TAG-MAP). · **Oracle:** the
  recorded tagged-child set = exactly the intake set. · **Seeded sim** (Tier-3 in
  P6). · **Status:** attachment cut (owner half); the kernel `sock_ops`
  cell is P6-S13 at the cut.

- **P4-S11 — G-V8 boundary rows 2-5** `@contract-shape:bounded-change`
  (`g_v8_boundary_rows_row2_bind_failed_retried_row3_isolation_row4_late_row5_session_loss`).
  row2 `bind_failed` retried; row3 another allocation's ports/state unchanged;
  row4 a late `ListenState` after teardown binds nothing; row5 session loss closes
  every intake listener of the CID, reconnect re-sends full state and re-binds. ·
  **Oracle:** per-row universe assertions. · **Seeded sim.**

### Boot convergence + overlap refusals — seeded sim (`guest_network::prefix_boot_acceptance`)

File: `crates/overdrive-control-plane/src/guest_network/prefix_boot_acceptance.rs`
(source-local `#[cfg(test)]`, default lane; replaces the removed
`tests/acceptance/vsock_prefix_boot_order.rs`). Every test drives the production
owner `HostSharedGuestNetworkOwner::with_flow_io(ports, SimGuestFlowIo)` (DG-3)
through `probe_startup` → `converge_shared` over a `SimGuestNode`, scripting
effects on `SimGuestFlowIo`. Universe = `{boot outcome, steering link, link
program, prefix route, steering entries, effects issued}`, read from the
returned `GuestNetworkError`, `steering_observation()`, `steering_effects()` and
the ordered effect kinds. The same file also carries P5-S1 / P5-S2 legs and two
`probe_startup` refusals outside P4 (`an_owner_outside_the_connect4_attach_path_refuses_before_any_boot_effect`,
attachment cut, new-step 11; `an_rlimit_nofile_below_the_owner_budget_refuses_before_any_boot_effect`,
attachment cut, new-step 7), whose attach-point path form and fd-budget reserve term are
unpinned values (UI-R2-9).

- **P4-S12 — a boot that fails or crashes part-way never opens the prefix to a host service, and the next boot completes it** `@contract-shape:unbounded-preservation`
  (`boot_runs_load_then_swap_then_route_and_a_verifier_reject_is_never_attached`,
  `a_failed_or_interrupted_boot_step_leaves_the_earlier_program_and_route_local_only_over_a_pinned_link`,
  `a_failed_route_write_refuses_startup_with_the_route_as_it_was`,
  `a_boot_crashed_at_any_steering_step_is_converged_by_the_next_boot`,
  `a_failed_step8_table_transaction_leaves_the_table_absent_and_boot_still_steers`; R5-24, G-V1;
  prefix_boot inv 6; shared_table inv 11 at boot). load→swap→route; a
  verifier-rejected program is never attached; a failed/interrupted step leaves
  the earlier program (K-L9) and refuses `GuestPrefixSteeringConverge` with the
  route unwritten; the route is `local` only over a **pinned** link (M-4: a first
  attach before its pin does not count); a failed step-8 table transaction (the
  table stays absent: it has two states, absent or complete, U-QP-1) does not open
  the prefix to a host service (the table is not a route precondition). ·
  **Contract:** boot order load → swap → route, and convergence after a crash
  between steps (prefix_boot inv 6). · **Fault:**
  scripted load/swap failures; crash between steps / mid-attach. · **Oracle:**
  `boot_outcome`, `steering_link`, `prefix_route`. · **Seeded sim + Quint
  `prefix_boot`.** · **Status:** authored in `prefix_boot_acceptance`:
  `boot_runs_load_then_swap_then_route_and_a_verifier_reject_is_never_attached`,
  `a_failed_or_interrupted_boot_step_leaves_the_earlier_program_and_route_local_only_over_a_pinned_link`,
  `a_failed_route_write_refuses_startup_with_the_route_as_it_was`,
  `a_boot_crashed_at_any_steering_step_is_converged_by_the_next_boot` — ignored
  `pending DELIVER vsock attachment cut (new-step 13, steering load and swap, shared local route)`. The failed step-8 table cell
  (`a_failed_step8_table_transaction_leaves_the_table_absent_and_boot_still_steers`)
  is at the attachment cut (DG-2 row 9): it drives `run_server` composing
  `new(ports)` and asserts the table through `SimMtlsIntercept::shared_table_state()`.

- **P4-S13 — veth pair carries the unframe link + boot order** `@contract-shape:bounded-change`
  (`serve_creates_the_veth_pair_before_converge_shared`; R5-39, D5a-SERVE-IFACE;
  G-V1). `serve` with default dataplane NIC names creates
  `ovd-veth-cli`/`ovd-veth-bk` (step 3) BEFORE `converge_shared` (step 9); the
  structural boot-order test fails if veth provisioning runs after
  `converge_shared`; with operator-named NICs no pair is created. *(The attach
  assertion is P5 R5-38/39; here only the ORDER.)* · **Oracle:** recorded
  boot-step order. · **Seeded sim** (structural) + Tier-3 (production `run_server`,
  in P5). · **Status:** attachment cut (DG-2 row 9): the body drives `run_server`
  composing `HostSharedGuestNetworkOwner::new(ports)` and asserts the
  composition order (veth provisioning before `converge_shared`).

  **Planned attachment-cut cell "`run_server` boot refusal"** (DG-2 row 9;
  `@driving_port @error`, `@contract-shape:unbounded-preservation`). The
  startup-refusal projection that P4-S14, P4-S15, P5-S1 and P5-S2 do not
  observe (their authored bodies stop at the owner's `GuestNetworkError` and
  the sim seam's effects). *Given* `run_server` composing
  `HostSharedGuestNetworkOwner::new(ports)` over `SimGuestFlowIo`; *When*
  `probe_startup` refuses (a node address or VIP range overlapping the
  prefix, an intake-block conflict, a stacked interface, `FdBudget`,
  `OwnerCgroupPlacement`, `OwnerCgroupIsRoot`), `converge_shared` fails (an
  unframe attach, a route write), the CID-claim probe fails (a scripted
  `SimGuestCidClaim` probe mismatch) or the beacon bind fails (a scripted
  `SimBeaconIo` bind failure); *Then* `run_server` returns the refusal and
  emits `health.startup.refused` with reason `guest_network.probe` (probe
  refusals), `guest_network.converge` (converge failures),
  `guest_cid_claim.probe` (claim probe) or `vm_driver.beacon_bind` (beacon
  bind), one data row each of the same body; `open_after_boot` is never reached; no veth pair and
  no nft table exists; and the recorded composition order shows
  `probe_startup` before the veth step. · **Oracle:** the returned error, the
  `health.startup.refused` event and its reason, the composition-order
  record, the absence of the veth pair and of `MTLS_SHARED_NFT_TABLE`
  (`SimMtlsIntercept::shared_table_state()`). · **Status:** planned at the
  attachment cut (DG-2 row 9), with P4-S13.

- **P4-S14 — node-address / vip-range overlap refusals** `@contract-shape:unbounded-preservation`
  (`a_node_address_inside_the_prefix_refuses_before_any_boot_effect`,
  `a_vip_range_overlapping_the_prefix_refuses_before_any_boot_effect`,
  `a_node_that_overlaps_the_prefix_by_address_and_by_vip_range_names_the_vip_range`,
  `node_addresses_outside_the_prefix_pass_probe_startup_without_effect`,
  `probe_startup_refuses_a_vip_range_iff_one_of_its_ranges_meets_the_guest_prefix`,
  `probe_startup_refuses_a_node_address_iff_one_lies_inside_the_guest_prefix`;
  R5-11/R5-40, D8-PREFIX-CFG/D8-PREFIX-VIP; G-V1
  row2). A node interface address inside the guest prefix →
  `GuestPrefixOverlapsNodeAddress`; a `vip_range` range overlapping it →
  `GuestPrefixOverlapsVipRange` (checked first); both in `probe_startup` before
  any boot effect the seam records (no route, no steering link or entry, no
  attach); disjoint/adjacent ranges boot. The authored bodies observe the
  owner's `GuestNetworkError` and the sim seam's effects only; the veth pair,
  addresses, nft table and the `health.startup.refused` projection are
  `run_server`'s, asserted by the attachment-cut cell "`run_server` boot
  refusal" under P4-S13. **PBT:** refusal iff some range intersects the prefix. · **Oracle:**
  `GuestNetworkError` variant + `boot_effects.is_empty()`. · **Seeded sim +
  proptest.** · **Status:** authored in `prefix_boot_acceptance`, the six tests
  above (the two `…_iff_…` tests are the proptests that replace
  `overlap_predicate_property`); the VIP-range tests are ignored `pending DELIVER vsock attachment cut (new-step 13, probe_startup guest-prefix checks, D8-PREFIX-VIP)`, the
  node-address tests `pending DELIVER vsock attachment cut (new-step 13, probe_startup guest-prefix checks, D8-PREFIX-CFG)`. A node address *outside* the prefix whose
  subnet covers it (for example `100.64.0.1/10` over `GUEST_NODE_PREFIX`
  `100.95.0.0/16`) does not refuse: `probe_startup` refuses "an address …
  inside the guest prefix" (feature-delta L19136-19138; UI-R2-8, closed). The
  authored refusal predicate is "some address lies inside the prefix", which
  matches. The generator draws addresses inside the prefix or wholly disjoint
  ones; a covering-subnet cell is not authored. Residual noted upstream: a
  covering subnet's broadcast route lands inside the prefix.

- **P4-S15 — no wildcard TCP bind after `open_after_boot` / block conflict** `@contract-shape:unbounded-preservation`
  (`the_intake_block_conflicting_with_the_ephemeral_range_or_a_wildcard_listener_refuses`,
  `no_explicit_port_wildcard_tcp_bind_happens_after_open_after_boot`; R5-42/R5-44,
  INTAKE-BIND; G-V1). `IntakePortBlockConflict::EphemeralRange` when the block
  meets `ip_local_port_range`; `::HostListener { port }` when a configured wildcard
  listener lies in the block; both before any boot effect. Structural + source
  check: no explicit-port wildcard TCP bind after `open_after_boot`. *(Shared
  boundary with P1 R5-44: P4 owns the boot-order/source-check assertion; P1 owns
  the deploy/limit refusal `TooManyTcpListenerPorts`.)* · **Oracle:**
  `IntakePortBlockConflict` variant + `boot_effects.is_empty()`; the recorded
  wildcard-bind-vs-`open_after_boot` order. · **Seeded sim + structural.** ·
  **Status:** authored in `prefix_boot_acceptance`:
  `the_intake_block_conflicting_with_the_ephemeral_range_or_a_wildcard_listener_refuses`,
  `probe_startup_refuses_iff_the_ephemeral_range_or_a_wildcard_port_meets_the_intake_block`
  — ignored `pending DELIVER vsock attachment cut (new-step 9, intake host
  ports, INTAKE-BIND)`; `IntakePortBlockConflict` lives in `overdrive-control-plane::guest_network`
  (DG-13). The structural cell
  (`no_explicit_port_wildcard_tcp_bind_happens_after_open_after_boot`) is at the
  attachment cut (DG-2 row 9): `run_server` composing `new(ports)`; asserts no
  wildcard bind after `open_after_boot`.

### Prefix exposure (production) — covered by P4-S19

- **P4-S16 — guest-prefix exposure through the composition** `@contract-shape:unbounded-preservation`
  (`no_connection_to_a_guest_prefix_address_reaches_a_host_socket_other_than_a_steered_intake_or_leg_c`,
  `a_graceful_stop_leaves_route_rules_and_pinned_steering_and_the_next_boot_converges_all_three`;
  V-19, R5-3/R5-25, U-4; G-V1/G-V8). Through `run_server_with_obs_and_driver`: no
  connection to a guest-prefix address reaches a host socket other than a steered
  intake or leg-C, with `serve` up and down; a graceful stop leaves the
  route/rules/pinned steering; the next boot converges all three. The "reaches the
  guest application" half needs a real microVM → native x86_64 KVM host
  (`cargo xtask metal run --`). · **Oracle:** real `lo`/device connect class;
  `ip route` / `bpftool` / nft queries for the three node-global objects. ·
  **Status:** covered by P4-S19. `vsock_prefix_exposure.rs` no longer exists;
  `prefix_landing` drives the same configurations (never booted, serving,
  serve killed, serving after restart, serve stopped gracefully) through
  `run_server` + the public API on native metal and compares every landing
  with `lands(...)`.

### Conformance — three drivers (spec referenced in place)

> **Driver status (2026-10-10).** Placement, `State` projection and action map
> are pinned by feature-delta QPROJ-1 to QPROJ-8 (§ *vsock Conformance
> Drivers*). Drivers are in-crate modules of their owner; there is no
> `tests/conformance/tests/vsock_prefix_boot_conformance.rs` /
> `vsock_intake_conformance.rs`, no shared `quint_driver` base, no
> `e18_test_support` (deleted at the cut, DG-2 row 17) and no `ConformanceNode`.
> The seam sim is `SimGuestFlowIo` / `SimGuestNode` (`overdrive-control-plane`
> `#[cfg(test)] guest_network::flow_io::sim`). No driver uses `#[quint_test]`:
> the specs define no Quint `run` (UI-R2-1), so the `checks.toml` entries are
> invariant / witness / hazard checks run by `cargo xtask quint check`, and each
> driver replays `#[quint_run]` randomized traces per conformance instance.

- **P4-S17 — `prefix_boot` conformance** `@contract-shape:bounded-change` (prefix_boot.qnt inv 6). **Status:**
  authored in
  `crates/overdrive-control-plane/src/guest_network/prefix_boot_conformance.rs`
  (`#[cfg(all(test, feature = "integration-tests"))]`):
  `prefix_boot_conformance_boot_ok`, `prefix_boot_conformance_boot_faulty`,
  `prefix_boot_conformance_boot_live` (`#[quint_run]` over the `boot_ok`,
  `boot_faulty`, `boot_live` instances) — ignored `pending DELIVER vsock attachment cut (new-step 13, steering load and swap, shared local route)`. Actions:
  `init` a never-booted `SimGuestNode`; `Restart(b)` a new `SimGuestFlowIo`
  over the persisted node with program `b` + `with_flow_io`; `BootStep` call
  `converge_shared` / release the held `Load`, `Attach`, `Pin`, route effect;
  `Fail` fail the held effect (`Load`, `Update` on an adopted link, `Attach`,
  `Pin`, route write); `Crash` abort the pending boot, drop the owner, end the
  process's seam. `State`: `link`, `prog`, `route` from
  `steering_observation()`; `phase` from the owner (`converge_shared` not called
  / pending / `Ok`) with `steering_effects()`. The `SimGuestFlowIo` reads are
  named arbiter reads of node-global kernel facts the owner does not hold,
  paired with the Tier-3 cells for R5-24, R5-34 (P4-S4) and R5-25 (P4-S19).
  The spec's safety, local-only-over-link, no-wildcard, nothing-while-down,
  open-converged and opens properties and the `route-first` / `ignore-failure`
  / `detach` / `unpinned` hazards are model checks in `checks.toml`, not
  driver runs.

- **P4-S18 — `intake` conformance** `@contract-shape:bounded-change` (intake.qnt inv 7, 8, 12). **Status:**
  attachment cut (DG-2 rows 2, 3, 7, 8, 9), in `overdrive-control-plane` beside
  the owner, instances `intake_ok` / `two` / `live` replayed as `#[quint_run]`.
  The QPROJ action map (Provision / SessOpen / SessLoss / ListenState /
  Activate / Teardown / Quiesce / Restore / Steer(fail)) drives the production
  owner over `SimGuestFlowIo`; `State` from O2 `lease_observation()`, O3
  `flow_observation()` (`intake`, `parked`, `last_listen_state`) and the arbiter
  reads `steering_observation().entries`, `steering_effects()`. The spec
  properties (safety, serving-reachable, activation-completes, the
  within-bounds checks, no-capacity-steer-failure, steer-forever-unreachable,
  witnesses, hazards) are `checks.toml` model checks, not driver runs. Earlier
  projection, for reference:
  `{alloc, session, lastLS, quiesced, open intake ports, steering entry ports,
  reserved}`.

- **P4-S19 — `prefix_landing` conformance** `@contract-shape:pure-function` (`tests/conformance/tests/vsock_prefix_landing_conformance.rs`; native metal; see the checklist in `test-scenarios.md`).
  **Status:** authored:
  `native_metal_kvm_prefix_landing_matches_lands_for_every_client_class_and_reachable_configuration`
  — ignored `pending DELIVER vsock new-step 5 and attachment cut (new-steps 9, 12, 13: guest intake adaptation, guest-prefix steering with intake listeners, constant prefix rules, the prefix route); then stays ignored: native metal only, run by run-native-metal.sh with quint provisioned — on activation replace this reason, never remove the attribute`. Configurations
  are reached through `run_server` + the public API only. The harness reads the
  exported DG-9 / DG-14 constants (`MTLS_SHARED_NFT_TABLE`, `GUEST_NODE_PREFIX`,
  `GUEST_PREFIX_ROUTE_PROTOCOL`, `GUEST_PREFIX_STEERING_PIN_DIR`,
  `GUEST_INTAKE_HOST_PORT_BASE`) and decodes map entries with
  `guest_steering_key`, so the earlier harness-spelled values (G-PL-1, G-PL-2,
  G-PL-5, G-PL-7) are closed by DG-9 / DG-14; G-PL-3 (`quint` on metal) is the
  DELIVER provisioning step's native preflight; `marked` comes from the
  caller's configuration (DG-14).
  `prefix_landing.qnt` is a PURE stateless definition (`lands(net, client, port)`)
  shared by the three stateful modules — no trace to replay. Conformance is the
  Hebert MODELING/oracle pattern: for every representative `Net` config × client
  class (marked / unmarked / remote) × port, the model's `lands(...)` is the
  oracle and the production steering + nft landing must fall in the same class
  (`intake` / `legC` / `refused` / `wildcard` / `elsewhere`), with the explicit
  check that the model never says `wildcard` while steering is attached (B-2
  closed) and neither may the kernel. Covers V-19 (K-D1/K-D2/K-D4) + V-26
  (K-L1/K-L2/K-L5/K-L10).

---

## Coverage matrices

### Gate → scenario

| Gate | Scenarios |
|---|---|
| G-V1 (steering/boot/route half) | P4-S2, P4-S4, P4-S12, P4-S13, P4-S14, P4-S15, P4-S16, P4-S17 |
| G-V3 (listener clause) | P4-S6, P4-S7 |
| G-V8 (intake listener) | P4-S1, P4-S2, P4-S3, P4-S5, P4-S6, P4-S7, P4-S8, P4-S9, P4-S10, P4-S11, P4-S16, P4-S18 |

### Decision → scenario

| Decision | Scenarios |
|---|---|
| D8 / D8a / D8a-LOOKUP | P4-S1, P4-S2, P4-S5, P4-S16, P4-S17, P4-S19 |
| D8a-LOAD-SWAP | P4-S4, P4-S12, P4-S17 |
| D8a-CAP (steering side) | P4-S3, P4-S8, P4-S18 |
| D23 / U-6 / M-7 | P4-S6, P4-S7, P4-S11 |
| INTAKE-BIND | P4-S1, P4-S9, P4-S15 |
| PREFIX-REPLY | P4-S5, P4-S16 |
| D8-PREFIX-CFG / D8-PREFIX-VIP | P4-S14 |
| D5a-SERVE-IFACE | P4-S13 |
| U-3 / U-4 | P4-S3, P4-S8 (U-3), P4-S16 (U-4), P4-S2 |
| INTAKE-TAG-MAP (D7/M-1) | P4-S10 |

### R5 → scenario

| R5 | Scenario | R5 | Scenario |
|---|---|---|---|
| R5-1 | P4-S1 | R5-25 | P4-S16 |
| R5-2 | P4-S1 | R5-29 | P4-S7 |
| R5-3 | P4-S2, P4-S16 | R5-34 | P4-S4 |
| R5-5 (listen) | P4-S6 | R5-37 (steer side) | P4-S3, P4-S18 |
| R5-11 | P4-S14 | R5-38 (stacked half) | P4-S14/P4-S15 context; attach in P5 |
| R5-13 | P4-S10 | R5-39 | P4-S13 |
| R5-22 | P4-S3, P4-S8 | R5-40 | P4-S14 |
| R5-23 | P4-S11 (restore row); quiescence in P6 | R5-42 | P4-S9, P4-S15 |
| R5-24 | P4-S4, P4-S12 | R5-43 | P4-S5 |
| | | R5-44 (boot half) | P4-S15 |

### V-item → scenario

| V | Status | Scenario |
|---|---|---|
| V-19 | covered by P4-S19 (authored; runs on native metal once its steps land) | P4-S19 (covers P4-S16; ignored `pending DELIVER vsock new-step 5 and attachment cut (new-steps 9, 12, 13 …)`), P4-S2 |
| V-26 | Done (P-40, Tier-3 CI) | P4-S1, P4-S2, P4-S3, P4-S4, P4-S5, discharges A-33 |
| V-12 | Open (DELIVER 9) | (D15 identity only — platform-client source; informational, no steering/safety rule keys on it; carried by the exposure test's peer observation, not a P4 safety assertion) |

### Quint check → scenario

The `checks.toml` entries below are model checks (invariants, witnesses,
hazards) run by `cargo xtask quint check`; they are not Quint `run`s and no
driver pins them as `#[quint_test]` (UI-R2-1). The drivers replay `#[quint_run]`
randomized traces of the instances named in § *Conformance*.

| Spec | `checks.toml` checks | Driver |
|---|---|---|
| `prefix_boot.qnt` (inv 6) | prefix-boot-safety(-apalache), -faulty-safety, -local-only-over-link(-while-down), -no-wildcard, -nothing-while-down, -open-converged, -opens, witnesses ×7, hazards ×5 | P4-S17 (`boot_ok`, `boot_faulty`, `boot_live`) |
| `intake.qnt` (inv 7, 8, 12) | intake-safety(-tlc/-two/-apalache), -serving-reachable(-with-defects), -activation-completes, -reserved/listeners/entries-within-…, -no-capacity-steer-failure, -steer-forever-unreachable, witnesses, hazards ×9 | P4-S18 (attachment cut; `intake_ok`, `two`, `live`) |
| `prefix_landing.qnt` | (no `[[check]]` — pure module) oracle table | P4-S19 |

### G-V8 required boundary rows (feature-delta § "Required boundary scenarios")

| Row | G-V8 cell | Scenario |
|---|---|---|
| 1 Available | listener bound when the guest listens | P4-S1, P4-S6 |
| 2 Unavailable/timeout | `guest_intake.bind_failed`, retried | P4-S8, P4-S11 (row2) |
| 3 Unrelated unchanged | allocation states unchanged | P4-S11 (row3) |
| 4 Late success | late `ListenState` after teardown binds nothing | P4-S11 (row4) |
| 5 Disconnect/reconnect | session loss closes every listener; reconnect re-binds | P4-S11 (row5) |
| 6 Feature disabled | single-cut; sim-driver `transport: None` behaves as today | the planned attachment-cut cell P6-S12b |

---

## Completeness audit (nw-at-completeness-check, 15-item)

Computed over the P4 candidate AT set:

| Item | Pass | Evidence |
|---|---|---|
| C1a empty/zero/min | ✅ | P4-S9 (index 0); P4-S14 (empty range set); P4-S3 (zero free slots) |
| C1b partition boundary | ✅ | P4-S3 (`GUEST_INTAKE_STEERING_MAX`, +1); P4-S9 (4095/4096); P4-S15 (ephemeral boundary) |
| C2a state machine documented | ✅ | serving invariant + alloc lifecycle in this doc + module docstrings + intake.qnt |
| C2b illegal-event-from-state | ✅ | P4-S6 (ListenState during activation/teardown/quiescence); P4-S11 row4 (late); P4-S8 (defect) |
| C3 count 0/1/N | ✅ | P4-S6 (0/1/2 ports); P4-S7 (3 ports); P4-S9 (two VMs); P4-S3 (cap N) |
| C4a apply-twice idempotency | ✅ | P4-S1 (re-steer same listener = Ok no change); P4-S12 (converge_shared idempotent re-run after crash) |
| C4b inverse without prerequisite | ✅ | P4-S3 (close removes entry); P4-S8 (teardown Ok without waiting on a steering write) |
| C5a mode-flag combos | specified; owed to the cut | authored: P4-S14 (node-addr × vip-range), P4-S15 (ephemeral × wildcard-listener); at the attachment cut: P4-S13 (default vs operator NICs) and the feature-level `transport` cell (P6-S12b) |
| C5b flag orthogonality | specified; owed to the cut | P4-S13 (operator NICs → no veth, link still carried) and P6-S12b, both attachment-cut bodies |
| C6a malformed per param | ✅ | P4-S14 proptest (arbitrary ranges); steer preconditions (IntakeOutsideGuestPrefix/UnsupportedSocket) |
| C6b each declared error | ✅ | Load/Update/GuestPrefixSteeringConverge (S12), KernelMemory/MapUpdate (S8/S3), Overlaps* + IntakePortBlockConflict (S14/S15) |
| C6c closed error set | specified; owed to the cut | P4-S7 all-or-nothing (no partial escapes), an attachment-cut body; universe fail-closed assertions |
| C7a degraded resource | ✅ | P4-S8 (`KernelMemory` = kernel memory pressure) |
| C7b interruption mid-op | ✅ | P4-S12 (crash between steps / mid-attach); P4-S4 (failed swap) |
| C7c concurrent actors | ✅ | P4-S9 (two VMs); P4-S11 row3 (isolation); `host-kernel-shared` serialization (full density concurrency = V-9, DELIVER) |

Evidence status (2026-10-10): the C-items citing P4-S6..S11, P4-S13 and the
P4-S15 structural cell rest on bodies authored at the attachment cut (DG-2);
over what is authored now they are `AT_GAP_IN_DELIVERY_SCOPE`, planned in the
feature-delta *Attachment-cut DISTILL increment* (the feature-wide verdict is
ACCEPTABLE_WITH_DOCUMENTED_GAPS, 12/15). The P4-S3 cap-N cell is the 256-intake
slot-reuse cell, not a full `GUEST_INTAKE_STEERING_MAX` fill.

**Verdict (specification): 15/15; authored: C5a, C5b and C6c owed to the attachment cut.** Zero `SPECIFICATION_AMBIGUITY` blockers — every
upstream artifact is present: the state machine (intake.qnt / this doc), the mode
flags (boot config: NIC names, `vip_range`, `ip_local_port_range`, wildcard
listener set), the error contracts (`GuestNetworkError` / `GuestSteeringError`
taxonomies), the env/concurrency matrix (`host-kernel-shared`; V-9 density). No
`CLARIFICATION_NEEDED`.

---

## Mandate compliance (CM-A … CM-I)

- **CM-A (hexagonal boundary):** seeded-sim scenarios enter through the owner's
  driving ports (`provision`/`activate`/`teardown`/quiesce/restore,
  `probe_startup`/`converge_shared`); Tier-3 steering is an adapter-integration
  file driving the `GuestPrefixSteering` adapter against the kernel (not a
  hexagon-internal component). No internal component is instantiated to assert
  internal state.
- **CM-B (business/domain language):** scenario titles and step prose use
  domain terms (steered intake, declared port, guest prefix, workload address,
  boot convergence). Technical surface (nft, `sk_lookup`, `bpftool`, `ss`) lives
  in oracle helpers / bodies, not scenario names.
- **CM-C (walking skeleton / focused):** P4 contributes focused + exposure
  scenarios; the vsock walking skeleton + crash matrix is P6.
- **CM-D (pure-function extraction):** the two pure functions
  (`guest_intake_host_port` injectivity; the vip/prefix overlap predicate) are
  tested directly by `proptest` (layer 1); impure boot/steering effects are
  behind the owner/adapter ports.
- **CM-E (Mandate 8 universe):** seeded-sim scenarios assert via before/after
  snapshots of port-exposed observables (the Rust realization of
  `assert_state_delta`; the codebase has no Python `state_delta` port) with
  fail-closed "otherwise unchanged" checks; Tier-3 (layer 5-6) uses traditional
  assertions (Mandate 8 permits this at layers 4+).
- **CM-F (Mandate 9):** `proptest` appears ONLY on the two layer-1 pure
  functions; seeded-sim (layer 3) and Tier-3 are example-only.
- **CM-G (Mandate 10 — Tier B):** not applicable at P4 as a separate Tier-B
  file — the state-machine exploration lives in the Quint `prefix_boot` / `intake`
  specs (replayed by the conformance drivers P4-S17/S18), which is the project's
  model-checked substitute for in-memory `RuleBasedStateMachine` (ADR-0168 /
  `.claude/rules/testing.md` § "Quint conformance"); the shared step vocabulary
  is the owner's driving-port methods reused across the seeded-sim and
  conformance drivers.
- **CM-H (Mandate 11):** every Tier-3 / conformance sad path is a named
  example-based test; no PBT machinery at layer 3+.
- **CM-I (Mandate-12 SSOT):** domain vocabulary = `overdrive-core` newtypes the
  design pins (`ListenPort`, `GuestCid`, `GuestNetworkError`,
  `GuestSteeringError`, the intake constants), per shared-infra § 1 (the Rust
  adaptation of `domain_types.py`); step bodies invoke the owner/adapter services
  and inline no business logic. Step-reuse-ratio is informational, not gated.

---

## RED / attachment-cut notes

Authored now (compiled; every body complete; the crafter removes only the
`#[ignore]` marker):

- `vsock_guest_steering` (P4-S1..S5): `pending DELIVER vsock new-step 9
  (guest-prefix steering)`; the swap test `pending DELIVER vsock new-step 13
  (steering load and swap)`.
- `guest_steering.rs` key tests: `the_steering_key_is_eight_bytes` default-lane
  GUARD (not ignored); the other three `pending DELIVER vsock new-step 9
  (guest-prefix steering)`.
- `prefix_boot_acceptance` (P4-S12, P4-S14, P4-S15): each `pending DELIVER
  vsock attachment cut (…)` with `new-step 13, steering load and swap, shared
  local route`, `new-step 13, probe_startup guest-prefix checks,
  D8-PREFIX-VIP`, `new-step 13, probe_startup guest-prefix checks,
  D8-PREFIX-CFG` or `new-step 9, intake host ports, INTAKE-BIND`.
- `prefix_boot_conformance` (P4-S17): `pending DELIVER vsock attachment cut
  (new-step 13, steering load and swap, shared local route)`.
- `vsock_prefix_landing_conformance` (P4-S19, covers P4-S16): `pending DELIVER
  vsock new-step 5 and attachment cut (new-steps 9, 12, 13: guest intake
  adaptation, guest-prefix steering with intake listeners, constant prefix
  rules, the prefix route); then stays ignored: native metal only, run by
  run-native-metal.sh with quint provisioned — on activation replace this
  reason, never remove the attribute`.

Authored at the attachment cut (DG-2; feature-delta § *Attachment-cut DISTILL
increment*): P4-S6..S11 and the `intake` driver P4-S18 (rows 2, 3, 7, 8, 9);
P4-S13, the P4-S15 `run_server` structural cell and the P4-S12 failed step-8
table cell (row 9).

Closed upstream items: UI-C1 (DG-2), UI-C2 and UI-COV-1 (DG-3), UI-CORE-1
(DG-13), UI-PL-1..7 (DG-14, DG-9 for G-PL-5), UI-R2-8 (the existing
D8-PREFIX-CFG text). Open round-2 items touching P4:
UI-R2-1 (no `#[quint_test]`, HIGH), UI-R2-9 (unpinned owner-cgroup attach-point path form and fd-budget reserve
term in the non-P4 `probe_startup` refusals of `prefix_boot_acceptance`).
Nothing is blocked on a validation item: V-26 ran and HOLDS (P-40); V-19 / V-12
are DELIVER-step confirmations.

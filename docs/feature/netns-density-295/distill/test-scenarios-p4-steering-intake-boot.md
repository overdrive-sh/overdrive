# P4 — Guest-prefix steering, intake listeners, boot convergence & landing

DISTILL scenario specification (netns-density-295, vsock Attachment Replacement
DESIGN). This doc is the authored DISTILL product for piece P4; the per-scenario
Rust RED scaffolds live in the owned files below (parked build-safe until the
DELIVER roadmap reconciliation — see § RED / blocked). No `.feature` files
(project rule); scenarios are GWT spec here and each is a Rust
`#[test]` / `#[tokio::test]` in the owned files.

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

| File | Lane | nextest group | Registration (DELIVER adds) |
|---|---|---|---|
| `crates/overdrive-dataplane/tests/integration/vsock_guest_steering.rs` | Tier-3 (`sk_lookup` kernel facts) | `host-kernel-shared` (by MODULE) | `mod vsock_guest_steering;` in `tests/integration.rs` |
| `crates/overdrive-control-plane/tests/acceptance/vsock_intake_listener.rs` | seeded sim (owner path) | — (default lane) | `mod vsock_intake_listener;` in `tests/acceptance.rs` |
| `crates/overdrive-control-plane/tests/acceptance/vsock_prefix_boot_order.rs` | seeded sim (boot path) | — (default lane) | `mod vsock_prefix_boot_order;` in `tests/acceptance.rs` |
| `crates/overdrive-control-plane/tests/integration/vsock_prefix_exposure.rs` | Tier-3 (V-19) | `host-kernel-shared` (by MODULE) | `mod vsock_prefix_exposure;` in `tests/integration.rs` |
| `tests/conformance/tests/vsock_prefix_boot_conformance.rs` | quint-connect (Lima) | standalone binary | remove `#![cfg(any())]` gate |
| `tests/conformance/tests/vsock_intake_conformance.rs` | quint-connect (Lima) | standalone binary | remove `#![cfg(any())]` gate |
| `tests/conformance/tests/vsock_prefix_landing_conformance.rs` | quint-connect (Lima) | standalone binary | remove `#![cfg(any())]` gate |

Per shared-infra § 6, the planner did NOT pre-register these (registration +
the compiling body co-arrive with the DELIVER production scaffold they call).
The 4 crate-subdir files are therefore left UNREGISTERED (Cargo ignores
unreferenced files under `tests/{acceptance,integration}/`), and the 3
auto-discovered conformance binaries are gated `#![cfg(any())]` (always-false,
no feature name → no `unexpected_cfgs` warning → compiles to an empty crate).
Both mechanisms keep `cargo check --all-targets --features integration-tests`
green while the production surface is absent.

---

## Lanes & PBT mode (Mandate 9 / 11)

| Layer | What | Mode |
|---|---|---|
| Unit (layer 1) | `guest_intake_host_port` injectivity; the vip/prefix overlap predicate | **PBT** (`proptest`) |
| Seeded sim (layer 3, production owner path) | intake lifecycle, boot order, overlap refusals | **example-based** (sad paths enumerated, never PBT-generated) |
| Tier-3 (layer 5-6) | steering kernel facts; prefix exposure | **example-based** (1-2 representative) |
| Quint conformance (Lima) | `prefix_boot` / `intake` trace replay; `prefix_landing` oracle table | named runs + randomized traces (seed on failure) |

Sad-path share: S1/S2 refusals, S3 cap-E2BIG, S4/S12 failed swap/load, S7 rollback,
S8 steer-failure classes, S12 crash matrix, S14 overlaps, S15 block conflict, S16
exposure-down — well over the ≥ 40 % error/edge target.

---

## Scenario specification (P4-S1 … P4-S19)

Each row: GWT (domain language) · contract/invariant · production owner path ·
fault stimulus · oracle · observable universe (Mandate 8) · evidence tier.

### Steering kernel facts — Tier-3 (`vsock_guest_steering.rs`, `host-kernel-shared`)

Adapter-integration: drives the real `GuestPrefixSteering` adapter against the
real kernel with hand-bound listeners standing in for the owner's intakes. The
full-composition "reaches the guest application" fact is P4-S16 / the P6 WS.

- **P4-S1 — only a steered listener is reachable at the prefix declared port**
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
  wildcard_service.accepted}`. · **Tier-3.**

- **P4-S2 — an unassigned/released prefix address reaches no host socket**
  (`an_unassigned_prefix_address_reaches_no_host_socket`; R5-3, K-L1; G-V1/G-V8).
  *Given* steering attached + a wildcard host service; *When* a host-local
  connect targets a prefix address with no lease/entry (with and — the DELIVER
  Tier-3 cell — without the nft constant rules); *Then* the lookup is dropped and
  no host socket is reached (the steering alone keeps the prefix closed). ·
  **Oracle:** `connect()` → `ConnectionRefused`. · **Tier-3 (V-19, V-26).**

- **P4-S3 — close removes the entry + cap cycle**
  (`closing_a_listener_frees_its_steering_slot_in_the_same_step`,
  `steering_map_at_capacity_refuses_one_more_then_a_close_frees_a_slot`; R5-22
  close, D8a-CAP, K-L3; G-V8). *Given* a steered listener reachable; *When* the
  listener closes; *Then* the kernel removes the entry in the same step and the
  next connect is reset; a fresh listener re-steered on the same port is reachable
  at once (freed slot). *Given* the map at `GUEST_INTAKE_STEERING_MAX`; *When* one
  more insert; *Then* `MapUpdate` (E2BIG); *When* one earlier listener closes;
  *Then* the insert succeeds. · **Invariant:** entry lifetime = listener lifetime
  (K-L3); map capacity. · **Oracle:** `connect()` class + `GuestSteeringError`
  variant. · **Tier-3 (V-26; the capacity-fill is a DELIVER real-kernel fixture).**

- **P4-S4 — pinned link + atomic swap**
  (`boot_over_a_left_behind_pinned_link_swaps_atomically_and_never_detaches`,
  `a_failed_swap_leaves_the_earlier_program_attached`; R5-24/R5-34,
  D8a-LOAD-SWAP, K-L9; G-V1). *Given* a left-behind pinned link running an earlier
  program + `sshd` on `0.0.0.0:22`; *When* a new binary boots (`load` → `attach`
  `BPF_LINK_UPDATE`); *Then* the link is never detached and a connect-loop across
  the swap never reaches `sshd`, decided only by the earlier or the new program.
  *When* the swap is scripted to fail; *Then* `GuestSteeringError::Update` and the
  earlier program stays attached (K-L9). · **Oracle:** pinned-link presence
  (`bpftool`), connect class across the swap, error variant. · **Tier-3 (V-26
  K-L9; the concurrent connect-loop + scripted update are DELIVER fixtures).**
  `ingress_ifindex` distinguishes loopback from device (V-26) is asserted by the
  remote-vs-loopback cells here.

- **P4-S5 — reply path for an in-prefix source**
  (`a_host_local_client_whose_source_lies_in_the_prefix_completes_to_a_steered_intake`;
  R5-43, PREFIX-REPLY; G-V8). *Given* the production constant rules (the
  established/related accept sits before each reject/drop, after the leg-S
  exemption and the leg-C divert) + a steered intake; *When* a host-local client
  whose SOURCE lies in the prefix (the gateway) connects; *Then* the connection
  completes and exchanges both ways (replies pass whatever their address); *And*
  a NEW unmarked host-local connect with no entry is still rejected at once. ·
  **Oracle:** connect-from-source outcome + exchange; fresh-connect reset. ·
  **Tier-3.**

### Intake listener lifecycle — seeded sim (`vsock_intake_listener.rs`)

Drives the production owner (`provision`/`activate`/`teardown`/quiesce/restore)
with `SimGuestPrefixSteering` at the steering seam and a scripted
`ListenState`/session feed. Universe = owner observable surface (never private
fields): `{cid_state[cid], open_intake[cid], listener_bind[(cid,port)],
steer_entries, bind_failed_events}`.

- **P4-S6 — `ListenState` interleavings yield the invariant state**
  (`a_listen_state_during_activation_teardown_or_quiescence_yields_the_invariant_state`;
  R5-5 listen half, M-7; G-V3/G-V8; intake inv 7). The design serving invariant —
  `serving(port) ⇔ Active ∧ no quiescence holder ∧ session Live ∧ last
  ListenState(port)=listening ∧ port ∈ listen_ports()` — holds for every
  interleaving; per-allocation serialization. · **Fault:** event ordering. ·
  **Oracle:** before/after universe delta = exactly the steered listeners for
  reported-listening ports; fail-closed (no other CID moved). · **Seeded sim.**

- **P4-S7 — all-or-nothing activation rollback**
  (`activation_with_a_failing_steer_rolls_back_wholly_and_leaves_the_cid_provisioned`;
  R5-29, U-6; G-V3/G-V8; intake inv 8). *Given* three reported-listening ports,
  the second `steer` a defect; *Then* `activate` returns `Err`, the CID stays
  Provisioned, and NO listener / entry of the CID remains (the rollback closes
  listeners and cannot fail). · **Fault:** scripted `MapUpdate` defect. ·
  **Oracle:** `cid_state=Provisioned`, `open_intake[cid]=∅`, entry-set unchanged
  (fail-closed). · **Seeded sim.**

- **P4-S8 — steer failure classes**
  (`a_kernel_memory_steer_failure_is_retried_at_the_audit_cadence`,
  `a_defect_steer_failure_is_not_retried_and_serves_again_only_after_the_next_listen_state`,
  `teardown_returns_ok_without_waiting_on_any_steering_write`; R5-22,
  D8a-LOOKUP/D8a-CAP; G-V8). `KernelMemory` × N → port refused meanwhile,
  listener closed, `bind_failed { retried: true }`, retried at audit cadence,
  serves after N+1; a defect (`MapUpdate`) → closed, `retried: false`, no retry,
  serves again only on the next `ListenState`; teardown returns `Ok` without
  waiting on any steering write. · **Fault:** scripted `KernelMemory`/`MapUpdate`.
  · **Oracle:** `bind_failed_events` retried-flag + `steer_entries` presence +
  `cid_state`. · **Seeded sim.**

- **P4-S9 — intake bind in the fixed block**
  (`every_intake_listener_binds_the_workload_address_in_the_fixed_block`;
  `intake_host_port_properties`; R5-42, INTAKE-BIND; G-V1/G-V8). Every listener
  binds `workload_addr:guest_intake_host_port(i)` for the declared port's
  ascending index (never `0.0.0.0`, never the declared port); two VMs declaring
  the same ports bind the same block ports on their own addresses and both serve.
  **PBT (layer 1):** `guest_intake_host_port` injective over
  `0..GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX`, lands in `guest_intake_host_ports()`,
  `None` beyond. · **Oracle:** `listener_bind[(cid,port)] == workload_addr:(base+i)`.
  · **Seeded sim + proptest.**

- **P4-S10 — intake-child identity**
  (`only_intake_listeners_are_tagged_never_a_leg_c_child_on_the_same_address_port`;
  R5-13, V-20, INTAKE-TAG-MAP; G-V8). The owner tags exactly its intake listeners;
  a leg-C transparent child sharing the local address/port is never in the
  clone-flagged intake-tag map. (The kernel `sock_ops` fact — V-20 HOLDS, P-46 —
  is the P6 WS Tier-3 cell; here the owner contract is pinned.) · **Oracle:** the
  recorded tagged-child set = exactly the intake set. · **Seeded sim** (Tier-3 in
  P6).

- **P4-S11 — G-V8 boundary rows 2-5**
  (`g_v8_boundary_rows_row2_bind_failed_retried_row3_isolation_row4_late_row5_session_loss`).
  row2 `bind_failed` retried; row3 another allocation's ports/state unchanged;
  row4 a late `ListenState` after teardown binds nothing; row5 session loss closes
  every intake listener of the CID, reconnect re-sends full state and re-binds. ·
  **Oracle:** per-row universe assertions. · **Seeded sim.**

### Boot convergence + overlap refusals — seeded sim (`vsock_prefix_boot_order.rs`)

Drives `probe_startup` → … → `converge_shared` with `SimGuestPrefixSteering`
scripting. Universe = `{boot_outcome, steering_link, prefix_route, boot_effects}`.

- **P4-S12 — boot order + crash between steps**
  (`boot_runs_load_then_swap_then_route_and_a_verifier_reject_is_never_attached`,
  `a_failed_or_interrupted_boot_step_leaves_the_earlier_program_and_route_local_only_over_a_pinned_link`,
  `a_partial_nft_table_does_not_make_the_prefix_reach_a_host_service`; R5-24, G-V1;
  prefix_boot inv 6; shared_table inv 11 at boot). load→swap→route; a
  verifier-rejected program is never attached; a failed/interrupted step leaves
  the earlier program (K-L9) and refuses `GuestPrefixSteeringConverge` with the
  route unwritten; the route is `local` only over a **pinned** link (M-4: a first
  attach before its pin does not count); a partial nft table does not open the
  prefix to a host service (the table is not a route precondition). · **Fault:**
  scripted load/swap failures; crash between steps / mid-attach. · **Oracle:**
  `boot_outcome`, `steering_link`, `prefix_route`. · **Seeded sim + Quint
  `prefix_boot`.**

- **P4-S13 — veth pair carries the unframe link + boot order**
  (`serve_creates_the_veth_pair_before_converge_shared`; R5-39, D5a-SERVE-IFACE;
  G-V1). `serve` with default dataplane NIC names creates
  `ovd-veth-cli`/`ovd-veth-bk` (step 3) BEFORE `converge_shared` (step 9); the
  structural boot-order test fails if veth provisioning runs after
  `converge_shared`; with operator-named NICs no pair is created. *(The attach
  assertion is P5 R5-38/39; here only the ORDER.)* · **Oracle:** recorded
  boot-step order. · **Seeded sim** (structural) + Tier-3 (production `run_server`,
  in P5).

- **P4-S14 — node-address / vip-range overlap refusals**
  (`a_node_address_inside_the_prefix_refuses_before_any_boot_effect`,
  `a_vip_range_overlapping_the_prefix_refuses_before_any_boot_effect`,
  `overlap_predicate_property`; R5-11/R5-40, D8-PREFIX-CFG/D8-PREFIX-VIP; G-V1
  row2). A node interface address inside the guest prefix →
  `GuestPrefixOverlapsNodeAddress`; a `vip_range` range overlapping it →
  `GuestPrefixOverlapsVipRange` (checked first); both in `probe_startup` before
  any boot effect (no veth, address, route, nft, steering link); disjoint/adjacent
  ranges boot. **PBT:** refusal iff some range intersects the prefix. · **Oracle:**
  `GuestNetworkError` variant + `boot_effects.is_empty()`. · **Seeded sim +
  proptest.**

- **P4-S15 — no wildcard TCP bind after `open_after_boot` / block conflict**
  (`the_intake_block_conflicting_with_the_ephemeral_range_or_a_wildcard_listener_refuses`,
  `no_explicit_port_wildcard_tcp_bind_happens_after_open_after_boot`; R5-42/R5-44,
  INTAKE-BIND; G-V1). `IntakePortBlockConflict::EphemeralRange` when the block
  meets `ip_local_port_range`; `::HostListener { port }` when a configured wildcard
  listener lies in the block; both before any boot effect. Structural + source
  check: no explicit-port wildcard TCP bind after `open_after_boot`. *(Shared
  boundary with P1 R5-44: P4 owns the boot-order/source-check assertion; P1 owns
  the deploy/limit refusal `TooManyTcpListenerPorts`.)* · **Oracle:**
  `IntakePortBlockConflict` variant + `boot_effects.is_empty()`; the recorded
  wildcard-bind-vs-`open_after_boot` order. · **Seeded sim + structural.**

### Prefix exposure (production) — Tier-3 (`vsock_prefix_exposure.rs`, `host-kernel-shared`)

- **P4-S16 — guest-prefix exposure through the composition**
  (`no_connection_to_a_guest_prefix_address_reaches_a_host_socket_other_than_a_steered_intake_or_leg_c`,
  `a_graceful_stop_leaves_route_rules_and_pinned_steering_and_the_next_boot_converges_all_three`;
  V-19, R5-3/R5-25, U-4; G-V1/G-V8). Through `run_server_with_obs_and_driver`: no
  connection to a guest-prefix address reaches a host socket other than a steered
  intake or leg-C, with `serve` up and down; a graceful stop leaves the
  route/rules/pinned steering; the next boot converges all three. The "reaches the
  guest application" half needs a real microVM → native x86_64 KVM host
  (`cargo xtask metal run --`). · **Oracle:** real `lo`/device connect class;
  `ip route` / `bpftool` / nft queries for the three node-global objects. ·
  **Tier-3; RED `#[ignore]` — DELIVER step 9/12 (V-19 confirmation, authored RED).**

### Conformance — three drivers (Lima; spec referenced in place)

- **P4-S17 — `prefix_boot` conformance** (`vsock_prefix_boot_conformance.rs`;
  prefix_boot.qnt inv 6). Replays the named `checks.toml` runs as `#[quint_test]`
  (safety(-apalache)/faulty-safety; local-only-over-link(-while-down);
  no-wildcard; nothing-while-down; open-converged; opens[liveness]; witnesses;
  hazards `route-first`/`ignore-failure`/`detach`/`unpinned`) + a randomized
  `#[quint_run]` over `boot_faulty`. State projected from
  `{serve up, boot phase, steering link kind+program, prefix route kind}`;
  actions `Restart`/`BootStep`/`Fail`/`Crash` → production boot effects + scripted
  sim steering.

- **P4-S18 — `intake` conformance** (`vsock_intake_conformance.rs`; intake.qnt inv
  7, 8, 12). Replays safety(-two/-apalache); serving-reachable(-with-defects);
  activation-completes; reserved/listeners/entries-within-…; no-capacity-steer-failure;
  steer-forever-unreachable (A-MEM off, expect violation); witnesses; hazards
  (entry-outlives-close / takedown-before-close / rollback-partial / act-ok-partial /
  teardown-keeps-listeners / quiesce-keeps-listeners / no-admission-over-capacity /
  cap-retry-unreachable) + a randomized `#[quint_run]`. State projected from
  `{alloc, session, lastLS, quiesced, open intake ports, steering entry ports,
  reserved}`; actions Provision/SessOpen/SessLoss/ListenState/Activate/Teardown/
  Quiesce/Restore/Steer(fail) → production owner calls + scripted sim steering.

- **P4-S19 — `prefix_landing` conformance** (`vsock_prefix_landing_conformance.rs`).
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
| V-19 | Open (DELIVER 9/12) | P4-S16 (RED `#[ignore]`), P4-S2, P4-S19 |
| V-26 | Done (P-40, Tier-3 CI) | P4-S1, P4-S2, P4-S3, P4-S4, P4-S5, discharges A-33 |
| V-12 | Open (DELIVER 9) | (D15 identity only — platform-client source; informational, no steering/safety rule keys on it; carried by the exposure test's peer observation, not a P4 safety assertion) |

### Quint run → scenario

| Spec | Named runs | Driver |
|---|---|---|
| `prefix_boot.qnt` (inv 6) | prefix-boot-safety(-apalache), -faulty-safety, -local-only-over-link(-while-down), -no-wildcard, -nothing-while-down, -open-converged, -opens, witnesses ×7, hazards ×5 | P4-S17 |
| `intake.qnt` (inv 7, 8, 12) | intake-safety(-tlc/-two/-apalache), -serving-reachable(-with-defects), -activation-completes, -reserved/listeners/entries-within-…, -no-capacity-steer-failure, -steer-forever-unreachable, witnesses, hazards ×9 | P4-S18 |
| `prefix_landing.qnt` | (no `[[check]]` — pure module) oracle table | P4-S19 |

### G-V8 required boundary rows (feature-delta § "Required boundary scenarios")

| Row | G-V8 cell | Scenario |
|---|---|---|
| 1 Available | listener bound when the guest listens | P4-S1, P4-S6 |
| 2 Unavailable/timeout | `guest_intake.bind_failed`, retried | P4-S8, P4-S11 (row2) |
| 3 Unrelated unchanged | allocation states unchanged | P4-S11 (row3) |
| 4 Late success | late `ListenState` after teardown binds nothing | P4-S11 (row4) |
| 5 Disconnect/reconnect | session loss closes every listener; reconnect re-binds | P4-S11 (row5) |
| 6 Feature disabled | n/a (single-cut; sim-driver `transport: None`) | covered by P1/P6 harness policy |

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
| C5a mode-flag combos | ✅ | P4-S13 (default vs operator NICs); P4-S14 (node-addr × vip-range); P4-S15 (ephemeral × wildcard-listener) |
| C5b flag orthogonality | ✅ | P4-S13 (operator NICs → no veth, link still carried) |
| C6a malformed per param | ✅ | P4-S14 proptest (arbitrary ranges); steer preconditions (IntakeOutsideGuestPrefix/UnsupportedSocket) |
| C6b each declared error | ✅ | Load/Update/GuestPrefixSteeringConverge (S12), KernelMemory/MapUpdate (S8/S3), Overlaps* + IntakePortBlockConflict (S14/S15) |
| C6c closed error set | ✅ | P4-S7 all-or-nothing (no partial escapes); universe fail-closed assertions |
| C7a degraded resource | ✅ | P4-S8 (`KernelMemory` = kernel memory pressure) |
| C7b interruption mid-op | ✅ | P4-S12 (crash between steps / mid-attach); P4-S4 (failed swap) |
| C7c concurrent actors | ✅ | P4-S9 (two VMs); P4-S11 row3 (isolation); `host-kernel-shared` serialization (full density concurrency = V-9, DELIVER) |

**Verdict: 15/15 → COMPLETE.** Zero `SPECIFICATION_AMBIGUITY` blockers — every
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

## RED / blocked notes

Per shared-infra § 0 and the planner's slicing: the P4 bodies compile once
`GuestPrefixSteering` (load/attach/swap/steer/close), the owner intake-listener
seam, boot `converge_shared`/`probe_startup` overlap checks, and
`SimGuestPrefixSteering` exist (DELIVER roadmap reconciliation + steps 2/9).
All P4 `.rs` files are authored now (the real DISTILL product) and parked
build-safe (unregistered crate-subdir files; `#![cfg(any())]`-gated conformance
binaries). Tier-3 steering + exposure carry RED `#[ignore = "pending DELIVER
step 09/12"]`; seeded-sim intake/boot carry `#[ignore = "pending DELIVER step
2/9 …"]`. Nothing is blocked on a validation item: V-26 ran and HOLDS (P-40);
V-19/V-12 are DELIVER-step confirmations authored RED. No `CLARIFICATION_NEEDED`.

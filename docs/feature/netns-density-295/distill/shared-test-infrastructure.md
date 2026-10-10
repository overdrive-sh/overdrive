# DISTILL — shared test infrastructure (netns-density-295, vsock replacement)

SSOT for the test-support surface every piece shares, so the six parallel
piece-authors do not each invent it. Authored by the DISTILL planner.
Binding rule for this project: **no `.feature` files** — scenarios are
specified as GIVEN/WHEN/THEN in the per-piece `test-scenarios-*.md` docs
(spec only, never parsed), and the crafter/author translates each into a Rust
`#[test]` / `#[tokio::test]` body under `crates/{crate}/tests/{acceptance,integration}/*.rs`,
source-local in the owner crate's `src/` where the surface is crate-private
(QPROJ-1, DG-5), or `tests/conformance/tests/*.rs` (`.claude/rules/testing.md`
§"Testing").

The design handed to DISTILL is `docs/feature/netns-density-295/feature-delta.md`
§"Wave: DESIGN / [REF] vsock Attachment Replacement DESIGN" (L16552+) and
ADR-0145..0181. The Quint oracle is `specs/quint/guest-flow-owner/`.

---

## 0. Load-bearing sequencing constraint (read first)

DG-2 (feature-delta § *DISTILL design-gap closure*, 2026-10-09) sorts every
body and driver into one of two authoring times; this increment (2026-10-10)
follows it exactly:

- **Additive surface — authored now.** New modules and types with no live
  counterpart are RED production scaffolds (Mandate 7, `// SCAFFOLD: true`,
  bodies `panic!("Not yet implemented -- RED scaffold")`); plain data and
  constants are real. This covers the core vocabulary and codecs
  (`overdrive_core::guest_transport` + `::wire`, `overdrive_core::net`, DG-7's
  `BeaconParseError::InvalidLaunchToken`, DG-8 `DatagramFrame`, DG-11, DG-12),
  the DG-9 marks and table name, the DG-14 constants and
  `overdrive_dataplane::guest_steering::guest_steering_key`,
  `overdrive_dataplane::guest_flow`, the three seams and their sims
  (`GuestFlowIo` / `SimGuestFlowIo`, `BeaconIo` / `SimBeaconIo` /
  `BeaconIntake`, `GuestOwnerIo` / `SimGuestOwnerIo`), `GuestFlowOwnerPorts` and
  `HostSharedGuestNetworkOwner::with_flow_io`, the observation accessors
  QPROJ-2 to QPROJ-6, the worker's forwarded-outbound port, and the whole
  `overdrive-init` `transport` and `guest_flow` modules (not yet called by
  `main`). Every body over this surface is compiled and marked
  `#[ignore = "pending DELIVER vsock new-step N (…)"]`, or `"pending DELIVER
  vsock attachment cut (…)"` where the behaviour lands in the cut although the
  body names no replaced signature.
- **Bodies that name a replaced signature — authored at the attachment cut.**
  The pool (`GuestAddressPool::new` / `assign`), the plan reshape,
  `AllocationSpec.transport`, `Vmm::create(&cfg, claim)`, `ForwardingActivation`,
  `quiesce_forwarding` / `restore_forwarding`,
  `HostSharedGuestNetworkOwner::new(ports)`, the component and operation enums,
  `recovery_progress`, READY's `launch`, `VmDriver::new(.., beacon)` and
  `vm_override` are DG-2 rows 1-13 and 15-18. A body or driver calling any of
  them cannot compile before the cut and is not in the tree (no unregistered
  file, no always-false `cfg`). The cut's DISTILL increment writes them first,
  in the cut's working tree, from the plan in feature-delta § *Attachment-cut
  DISTILL increment*.
- All contract-shaping validations (V-10, V-22, V-24, V-25, V-26, V-5(c),
  V-13, V-16, V-17, V-18, V-20, V-21) have run (P-37..P-48). V-6, V-8, V-9,
  V-19, V-1(c), V-12 and V-28 are DELIVER-step validations; their scenarios
  are authored RED with a reasoned `#[ignore]` or planned at the cut.

---

## 1. Mandate-12 / domain-vocabulary (Rust adaptation)

The Python "domain_types.py" pilot maps, in this Rust codebase, to the
**`overdrive-core` newtype vocabulary** — the design already pins it in
feature-delta §"[REF] Core vocabulary — overdrive-core (D2, D3, D5, D8, D8a-CAP, D16)":
`GuestCid`, `LaunchToken`, `FlowId`, `GuestAddressLease`, `WorkloadAddr`,
the flow/control/datagram wire types, `GuestNetworkError` taxonomy, the
intake / steering capacity constants. Test bodies invoke production services
(the owner, the pool, the forwarder port) and never inline business logic.
No `domain_types.py`, no `pytest-bdd`, no `.feature` files. Newtype
completeness (Display/FromStr/serde/Arbitrary roundtrip) is a mandatory
proptest call site (`.claude/rules/testing.md` §"Property-based testing").

---

## 2. Production composition — `run_server` and the public API

The walking skeleton and the cross-owner crash matrix (P6-S13..S15) drive the
**production composition** in-process through its entry points (`run_server`
and the public HTTPS API that `overdrive deploy` calls — never the binary,
`.claude/rules/testing.md`), with `Sim*` only at the external or
non-deterministic ports (clock, entropy, KEK). They need the attachment cut
(DG-2 rows 1-6) and run on native metal.

There is no shared `boot_vsock_node` builder and no `ConformanceNode`. The one
production-composition harness in the tree is
`tests/conformance/src/prefix_landing.rs`: `LandingNode` (`:904`) owns the
`ServerHandle` from `run_server` (`boot` `:912`, `kill` `:951`, `shutdown`
`:956`) and reads the kernel through independent queries (`read_net` `:456`).
The P6-S13 bodies written at the cut extend this harness or sit beside it in
`tests/conformance`.

`tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs` is an
OLD-model fail-stop test, retired in the attachment cut with the production
code it defends (§7).

---

## 3. Sim adapters and seam sims

The seeded-sim lane drives the production owner through its driving ports with
`Sim*` at the driven ports (`.claude/rules/testing.md` § "Tier 1"). Sim
placement follows QPROJ-1's crate-graph rule: core-trait sims in
`overdrive-sim`; every owner-seam sim source-local `#[cfg(test)]` beside its
seam in the owner crate. Every sim enforces the preconditions the real
adapter enforces (`.claude/rules/development.md` § "Trait definitions specify
behavior").

| Sim | Crate / module | Models | Stepping and scripting surface | Read surface (QPROJ-7 arbiter) |
|---|---|---|---|---|
| `SimGuestCidClaim` | `overdrive-sim` `adapters::guest_cid_claim` | the host-kernel CID holder table, incl. holders outside the pool's leases (R5-19) | `new`, `script_claim_error`, `script_probe_deviation`, `survivor_hold` | `held_cids()`; Tier-3 pairing `vsock_cid_claim_handoff` (P1-S12) |
| `SimBeaconIo` (`SimBeaconSocketObservation`) | `overdrive-sim` `adapters::beacon_io` | the node beacon listener (DG-4), coupled to a `SimVmm`: a connection whose VM died fails its next I/O | `new(vmm)` (delivers what is available) / `stepped(vmm)` (every `accept` / `read_line` waits for a grant), `queue_connection`, `release_accept`, `release_read`, `parked`, `fail_next_bind`, `is_bound`, `guest_write`, `guest_close`, `written_lines`, `observe_vm_exits` | `beacon_socket_observation()`: the accept queue with each queued connection's buffered READY token (`Option<LaunchToken>` from the READY `launch=` field; a line without a valid token projects `None`, UI-R2-15) and the connections the driver closed; Tier-3 pairing R5-41 (beacon part) |
| `SimVmm::spawn_bare_process` | `overdrive-sim` `adapters::vmm` | a live VMM process with no rootfs clone, ended by `Vmm::terminate` like a `create`d one | test hook for exercising `SimBeaconIo` without staging an image | `is_live(pid)` |
| `SimMtlsIntercept::shared_table_state` (`SimSharedTable`) | `overdrive-sim` `adapters::mtls_intercept` | the shared nft table's two states, absent or complete (U-QP-1); a failed transaction leaves it as it was | `fail_next_table_transaction(fault)`; `clear_faults` disarms it | `shared_table_state()`; Tier-3 pairing `vsock_host_internal_enforce` (P5-S8) |
| `SimGuestFlowIo` / `SimGuestNode` | `overdrive-control-plane` `#[cfg(test)] guest_network::flow_io::sim` | the host owner seam `GuestFlowIo` (DG-3): vsock sessions and flows, intake listeners and steering, the steering link / program / prefix route, the forwarder and unframe attaches. `SimGuestNode` holds node-global objects that persist across `serve` processes | `SimGuestNode::new`, `set_host_facts`, `update_host_facts`, `fail_host_facts`, `set_stale_objects`; `SimGuestFlowIo::new(&node, SimProgramVersion)`, `exit_process`; effect stepping `hold`, `auto_release`, `auto_release_all`, `issued`, `held`, `held_of`, `release`, `fail`, `script_failure`, `script_unframe_failure`; peer stimuli `queue_connection`, `release_accept`, `release_read`, `end_connection`, `send_on_connection`, `vm_stop`, `fail_listener`, `consume_outbox`, `connect_intake_client`, `release_intake_accept`, `end_intake_client`, `pair_hangup`, `set_drained`, `deliver_vsock_end`; host facts `set_route_type`, `set_connect4_rewrite`, `set_counters` | `socket_observation()`, `steering_observation()`, `steering_effects()`; Tier-3 pairings R5-24, R5-34, R5-25 (steering) and the owner-socket R5 rows |
| `SimGuestOwnerIo` + `SimGuestClock` | `overdrive-init` `#[cfg(test)] guest_flow::sim` | the guest owner seam `GuestOwnerIo` (DG-5): the control session, flow and slot sockets, application legs, the listener map, readiness events; one injected clock (`GuestClock`) | `SimGuestClock::new`, `advance`; `SimGuestOwnerIo::new(workload_addr, clock)`; faults `fail_next`, `fail_event_wait`, `fail_cell_creation`, `flow_vsock_error`; connects `pending_connects`, `complete_connect`, `complete_host_connect`, `complete_app_connect`; host side `host_send_control`, `host_control`, `release_control_read`, `take_outbox`, `host_take_message`, `host_take_next`, `host_take_first`, `host_open_accept`, `release_inbound_accept`, `release_inbound_request`, `host_vsock_end`, `host_observe_vsock_end`, `host_observe_legs_end`, `host_datagram`, `end_session`, `host_end_session_deferred`, `session_end_pending`, `release_session_end`, `host_owner_dies`, `guest_owner_dies`; applications `app_connect`, `app_end`, `app_send`, `app_send_on`, `app_close_socket`, `release_socket_report`, `release_slot_ready`, `set_drained`, `set_drained_default`, `set_listener_map`, `listener_map_wake`, `listener_failed`; inspection `effects`, `clear_effects`, `deliverable`, `last_wait`, `session_socket`, `control_opens`, `slot_state`, `exhausted_sends` | `guest_socket_observation()`; Tier-3 pairings R5-10, R5-14, R5-27, R5-28, R5-31, R5-45 |

The OLD-model `overdrive-sim` `adapters/guest_network.rs` and
`invariants/netns_density_boot_order.rs` are retired in the attachment cut
(§7). The earlier plan's `SimGuestFlowForwarder` and `SimGuestPrefixSteering`
were never built: the forwarder and steering effects are part of the
`GuestFlowIo` seam and `SimGuestFlowIo` models them. There is no VMM probe
sim: Overdrive runs only with the vendored Cloud Hypervisor fork, no probe
stage identifies the installed build, and the provisioning checksum is the
identity check (DG-1); `/dev/vhost-vsock` access is the claim probe's
(`GuestCidClaim::probe`).

---

## 4. Quint conformance drivers — placement per QPROJ-1

Each driver is a `quint-connect` module in its owner's crate,
`#[cfg(all(test, feature = "integration-tests"))]`, beside the owner;
`State` is projected from the owner's public read-only observation accessors
(QPROJ-2 to QPROJ-6) and the named sim arbiters (QPROJ-7), each arbiter read
marked `// conformance-lint: sim-arbiter-ok …` with its Tier-3 pairing. Only
`prefix_landing` lives in `tests/conformance` on the real kernel (QPROJ-8).
There is no shared `quint_driver` base and no `tests/conformance/tests/vsock_*_conformance.rs`
driver other than `prefix_landing`.

Authored now (4 driver modules, 7 `#[quint_run]` instances, ignored until
their DELIVER step):

| Driver | Module | Instances | Seam sim |
|---|---|---|---|
| `prefix_boot` | `overdrive-control-plane` `guest_network::prefix_boot_conformance` | `boot_ok`, `boot_faulty`, `boot_live` | `SimGuestFlowIo` / `SimGuestNode` |
| `owner_flows` guest side | `overdrive-init` `guest_flow::quint_owner_flows` | `flows_ok` | `SimGuestOwnerIo` + `SimGuestClock` |
| `owner_flows` `flows_half_guest` | `overdrive-init` `guest_flow::quint_flows_half` | `flows_half_guest` | `SimGuestOwnerIo` + `SimGuestClock` |
| `udp_slots` guest side | `overdrive-init` `guest_flow::quint_udp_slots` | `slots_ok`, `slots_live` | `SimGuestOwnerIo` + `SimGuestClock` |

At the attachment cut (7 drivers): `cid_lease`, `owner_flows` host side,
`flows_half_host`, `udp_slots` host side, `intake`, `quiescence`,
`shared_table` in `overdrive-control-plane`; `flows_beacon` in
`overdrive-worker`. The specs define no Quint `run`, so every driver runs
`#[quint_run]` only and no `#[quint_test]` pin exists (UI-R2-1). Shared ITF
decoding for the three `overdrive-init` drivers is the module
`guest_flow::quint_itf`.

`quint-connect` is a workspace dependency; `overdrive-control-plane`,
`overdrive-worker` and `overdrive-init` add it (with `itf` and `anyhow`) under
`[dev-dependencies]`. The per-driver checklist is in
[`test-scenarios.md` § Quint conformance drivers](test-scenarios.md#quint-conformance-drivers-qproj-1-to-qproj-8-approved-2026-10-09).
**Do NOT edit `specs/quint/**`.**

---

## 5. Real-kernel fixture policy (Tier-3)

The vsock model has **no per-VM netns/veth/`NetSlot`** (D1). The per-allocation
net-slot band registry (`crates/overdrive-control-plane/tests/common/net_slots.rs`)
is therefore **not** the isolation mechanism for most new Tier-3 tests.
Instead:

- **Node-global kernel state** — the host root-namespace `sk_lookup` steering
  link + pinned map, the `overdrive-mtls` / guest-prefix nft tables, the TCX
  egress unframe links on `lo` and every interface, the shared `local` route,
  the `/dev/vhost-vsock` CID holder table, process-global `AF_VSOCK` listeners
  (1234/1240/1241/1242/1243) — is shared by every vsock Tier-3 test. **Every
  new Tier-3 test binary/module that touches it joins the `host-kernel-shared`
  nextest group** (`.config/nextest.toml`, `max-threads = 1`, matched by
  MODULE — rename-proof), per `.claude/rules/testing.md` §"Node-global kernel
  state requires a catch-all nextest group". Each piece's scenario doc names
  the exact module→group entries its author adds.
- **Hand-built netns** in an egress/non-mesh test (e.g. a veth to reach an
  off-host destination, P5) MUST draw a route-disjoint CIDR via
  `overdrive_testing::cidr_lease::TestCidrLease::acquire(owner)` — never a CIDR
  literal (`.claude/rules/testing.md` §"Hand-built real-netns fixtures").
- The `serve`-created `ovd-veth-cli` / `ovd-veth-bk` pair (R5-39) is
  **production-owned** (its `NetSlotAllocator` is internal to `run_server`);
  tests observe it, they do not provision it.
- Tier-3 runs through `cargo xtask lima run --` (CI matrix) and, for the real
  microVM boot + CH-fork + `/dev/kvm` paths (P1 V-25, P6 V-1(c)/V-6), through
  `cargo xtask metal run --` on the native x86_64 KVM host
  (`.claude/rules/testing.md` §"native x86_64 bare-metal KVM host"). Never
  pipe Lima/metal runs through `tail`/`head`.

### 5.1 Test binaries and `host-kernel-shared` membership (2026-10-10)

Every authored binary that touches node-global kernel state is a whole-binary
member of `host-kernel-shared` (`max-threads = 1`, `.config/nextest.toml`):

| Binary | Lane / entry | Authored vsock tests in it |
|---|---|---|
| `overdrive-core` `acceptance` | default lane, no kernel state | `vsock_datagram_frame`, `vsock_deploy_validation`, `vsock_guest_transport_vocabulary`, `vsock_wire_codec` |
| `overdrive-core` lib | default lane | `dataplane::mtls_mark` unit tests (DG-9 GUARDs) |
| `overdrive-dataplane` lib | default lane | `guest_steering` key-encoder unit tests |
| `overdrive-dataplane` `integration` | Tier-3, whole binary in `host-kernel-shared` | `vsock_guest_steering`, `vsock_egress_unframe`, `vsock_fragment_map` |
| `overdrive-sim` lib | default lane | `adapters::beacon_io` and `adapters::mtls_intercept` unit tests (the sims' own semantics) |
| `overdrive-host` `integration` | `kvm-tests` (module `#[cfg(feature = "kvm-tests")]`), native metal; whole binary in `host-kernel-shared` | `vsock_cid_claim_handoff` (real `/dev/vhost-vsock`) |
| `overdrive-worker` `acceptance` | default lane | `vsock_beacon_intake` |
| `overdrive-worker` lib | default lane | `mtls_intercept_worker` stage-label unit test |
| `overdrive-worker` `integration` | Tier-3, whole binary in `host-kernel-shared` | `vsock_mesh_egress_mtls`, `vsock_host_internal_enforce` |
| `overdrive-control-plane` lib | source-local; `--features integration-tests` for the driver | `guest_network::prefix_boot_acceptance` (`#[cfg(test)]`), `guest_network::prefix_boot_conformance` (`#[cfg(all(test, feature = "integration-tests"))]`), `guest_network::flow_io::sim::tests` (the sim's own tests) |
| `overdrive-init` bin | source-local in the binary crate (no library target, DG-5) | `transport::tests`, `guest_flow::tests::{newtypes, session, tcp_accept, tcp_connect, half_close, udp_slots}`, and the drivers `guest_flow::quint_{owner_flows, flows_half, udp_slots}` (`integration-tests`) |
| `overdrive-system-conformance` `vsock_prefix_landing_conformance` | native metal, own `host-kernel-shared` entry | `native_metal_kvm_prefix_landing_matches_lands_for_every_client_class_and_reachable_configuration` |

The `overdrive-init` tests pass the kernel command line as a string
(`parse_transport_token`); they touch no kernel state. The old
`overdrive-host` `vsock_vmm_backend_probe` module was deleted with the probe
(DG-1).

---

## 6. Test entrypoint + module registration contract

Each authored module is registered in its crate's inline entrypoint block
(`tests/acceptance.rs` → `mod acceptance { … }`, `tests/integration.rs` →
`#![cfg(feature = "integration-tests")] mod integration { … }`;
`tests/conformance/tests/*.rs` are standalone binaries) or, for source-local
tests and drivers, by a `mod` declaration in the owner's `src/` under
`#[cfg(test)]` / `#[cfg(all(test, feature = "integration-tests"))]`. No
unregistered file and no always-false `cfg` is in the tree. Bodies that name
a DG-2 replaced signature are registered in the attachment cut, in the commit
that lands the cut.

---

## 7. Retired by DELIVER roadmap reconciliation (NOT deleted by the planner)

The following OLD-attachment-model (TAP / shared-bridge / intercept-program)
GREEN tests defend old production code **still in tree**. DG-2 places their
removal in the attachment cut. Per
`.claude/rules/development.md` §"Deletion discipline" (delete production code
AND its tests in the SAME commit) and the user decision that roadmap
reconciliation is the first DELIVER activity, they are retired **in DELIVER**,
coupled with removal of the production code they defend — not by this DISTILL
planner (deleting them now would leave an uncommittable, inconsistent tree).
Cataloged here so the reconciliation does not miss them:

- `crates/overdrive-core/tests/acceptance/`: `netns_density_exec_gate`,
  `netns_density_placement_cap`, `netns_density_cleanup_pending`,
  `vm_beacon_roundtrip`
- `crates/overdrive-control-plane/tests/acceptance/`: `netns_density_guest_network`
- `crates/overdrive-worker/tests/`: `netns_density_exec_release` (acceptance),
  `shared_intercept_members`, `netns_density_shared_owner` (integration),
  `mtls_intercept_install`, `leg_listener`, `outbound_enforce_substrate_splice` (revalidate vs replace — reconciliation decides per the Contract impact matrix)
- `crates/overdrive-sim/tests/`: `netns_density_activation_order`,
  `netns_density_retiring_cleanup` (acceptance), `netns_density_node_admission`,
  `netns_density_reclaim` (integration); `src/invariants/netns_density_boot_order.rs`;
  OLD parts of `src/adapters/guest_network.rs`
- `crates/overdrive-bpf/tests/integration/guest_tcx_classifier_test_run`,
  `crates/overdrive-dataplane/tests/integration/{guest_tcx_egress_lifecycle,guest_tcx_inventory}`
- `crates/overdrive-host/tests/integration/vmm_tap_queue_errors`
- `crates/overdrive-netlink/tests/integration/{tap_queue_attach,bridge_guard_lifecycle,managed_link_address,tap_debug_msg_mask}`
- `tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs`
- `crates/overdrive-control-plane/bin/netns_density_benchmark.rs` and
  `guest_network::e18_test_support` (DG-2 row 17; V-9 replaces E18)

The Contract-impact matrix (feature-delta §"Contract impact matrix") classes
each prior ADR Replaced / Retained / Revalidate — the reconciliation uses it
to decide revalidate-vs-delete per file. Prior DISTILL **docs** for the old
design (`distill/test-scenarios.md`, `distill/red-classification.md`) ARE
deleted by the planner (they are DISTILL-wave artifacts, superseded, and
deleting them touches no production code).

---

## 8. Completeness-audit + review gate (per-feature, applies to all pieces)

- Target ≥40% error/edge scenarios per piece (there is a very rich sad-path
  set: the 6-row boundary table rows 2/4/5, and R5-* enumerate failure modes).
- Layer-dependent PBT mode (Mandate 9 / `.claude/rules/testing.md`): PBT
  (`proptest`) at unit / pure-function layer (codec roundtrips, pool
  invariants, overlap predicates, fragment-decision fn); **example-based**
  at Tier-3 / integration / seeded-sim (sad paths enumerated, never
  PBT-generated).
- The `nw-at-completeness-check` 15-item gate + the four-reviewer Final Wave
  Review Gate (Sentinel always dispatches) run over the assembled scenario set
  once all six pieces land.

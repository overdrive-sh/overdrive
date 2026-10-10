# DISTILL scenario index — netns-density-295 (vsock attachment replacement)

This index replaces the prior (deleted) `test-scenarios.md`, which specified
the superseded TAP / shared-bridge / intercept-program attachment model
(S-ND295-*). The replacement DESIGN (feature-delta L16552+, ADR-0145..0181)
forwards guest traffic over virtio-vsock (kernel vhost) + Aya SK_SKB / strparser
forwarding + `sk_lookup` guest-prefix steering; there is **no** guest NIC, TAP,
bridge, per-VM netdevice or per-VM netns (D1).

Binding rules: no `.feature` files; GWT scenarios are spec-only here and in the
per-piece docs; the crafter translates each to a Rust `#[test]`/`#[tokio::test]`
(`.claude/rules/testing.md`). Shared test-support contract:
[`shared-test-infrastructure.md`](shared-test-infrastructure.md). The Quint
oracle is `specs/quint/guest-flow-owner/` (do **not** edit).

## Pieces (disjoint file ownership)

| Piece | Title | Scenario doc |
|---|---|---|
| **P1** | Admission, CID lease & kernel claim, intake capacity, deploy validation | [test-scenarios-p1-admission-cid-lease.md](test-scenarios-p1-admission-cid-lease.md) |
| **P2** | Guest-flow owner: control session, flow admission, pairing, launch attribution | [test-scenarios-p2-owner-flows.md](test-scenarios-p2-owner-flows.md) |
| **P3** | Datagram associations, UDP slots, framing, cells | [test-scenarios-p3-udp-slots-framing.md](test-scenarios-p3-udp-slots-framing.md) |
| **P4** | Guest-prefix steering, intake listeners, boot convergence & landing | [test-scenarios-p4-steering-intake-boot.md](test-scenarios-p4-steering-intake-boot.md) |
| **P5** | Egress unframe (hybrid), host-internal deny, mesh/non-mesh egress, VIP | [test-scenarios-p5-egress-unframe-hostinternal.md](test-scenarios-p5-egress-unframe-hostinternal.md) |
| **P6** | Quiescence & recovery, shared-table, VMM device & fork, guest READY, walking skeleton + crash matrix | [test-scenarios-p6-recovery-vmm-ws.md](test-scenarios-p6-recovery-vmm-ws.md) |

## DISTILL status (2026-10-10)

Every round-1 `UI-*` item of [`upstream-issues.md`](upstream-issues.md) is
closed by a DG decision (feature-delta § *DISTILL design-gap closure*,
DG-1..DG-16). Round 2 (same file) leaves DESIGN decisions open that bear on
scenarios: the three guest Quint drivers (P2-S15, P3-S8) are authored, but
their model fidelity is open (UI-R2-5, HIGH) — a correct implementation may
fail them until the model decision; no `#[quint_test]` pin exists (UI-R2-1,
HIGH); and P5-S7's resolve-adapter bodies have no pinned authoring time
(UI-R2-16). DG-2 sorts each scenario
into one of two authoring times:

- **Authored now** — every body and driver over additive surface. Each is a
  complete, compiled, registered body marked `#[ignore = "pending DELIVER
  vsock new-step N (…)"]` (or `"… attachment cut (…)"` where its behaviour
  lands in the cut although the body names no replaced signature), RED for
  the right reason (per-test classes in
  [`red-classification.md`](red-classification.md)).
- **Authored at the attachment cut** — every body or driver that calls a
  DG-2 row-1..18 replacement. It cannot compile before the cut and is not in
  the tree; its plan (drives / asserts per item) is feature-delta
  § *Attachment-cut DISTILL increment*, and each piece doc restates it per
  scenario as `attachment cut (DG-2 rows …)`.

Authored files (ignored count / default-lane count):

| File | Crate / binary | Scenarios | Ignored | Default lane |
|---|---|---|---|---|
| `crates/overdrive-core/tests/acceptance/vsock_guest_transport_vocabulary.rs` | `overdrive-core` `acceptance` | P1-S1, P1-S6 (core half), P1-S8 / P4-S9 pure parts, P1-S11, P3-S3 range fact | 20 | 1 |
| `crates/overdrive-core/tests/acceptance/vsock_wire_codec.rs` | `overdrive-core` `acceptance` | P2-S1, P2-S2 | 18 | 0 |
| `crates/overdrive-core/tests/acceptance/vsock_datagram_frame.rs` | `overdrive-core` `acceptance` | P3-S1 | 11 | 1 |
| `crates/overdrive-core/tests/acceptance/vsock_deploy_validation.rs` | `overdrive-core` `acceptance` | P1-S10, P1-S11 | 6 | 0 |
| `crates/overdrive-core/src/dataplane/mtls_mark.rs` (unit tests) | `overdrive-core` lib | DG-9 marks and table (P5-S8 layer 1) | 0 | 2 |
| `crates/overdrive-dataplane/src/guest_steering.rs` (unit tests) | `overdrive-dataplane` lib | DG-14 steering key (P4-S1..S5) | 3 | 1 |
| `crates/overdrive-dataplane/tests/integration/vsock_guest_steering.rs` | `overdrive-dataplane` `integration` (Tier-3) | P4-S1 … P4-S5 | 12 | 0 |
| `crates/overdrive-dataplane/tests/integration/vsock_egress_unframe.rs` | same | P5-S1, P5-S3 | 11 | 0 |
| `crates/overdrive-dataplane/tests/integration/vsock_fragment_map.rs` | same | P5-S4 | 4 | 0 |
| `crates/overdrive-sim/src/adapters/beacon_io.rs` (unit tests) | `overdrive-sim` lib | `SimBeaconIo` semantics (P2-S14) | 10 | 4 |
| `crates/overdrive-sim/src/adapters/mtls_intercept.rs` (new unit tests) | `overdrive-sim` lib | `SimMtlsIntercept::shared_table_state` (P6 shared table) | 0 | 3 |
| `crates/overdrive-host/tests/integration/vsock_cid_claim_handoff.rs` | `overdrive-host` `integration`, `kvm-tests` (native metal) | P1-S12 | 6 | 1 |
| `crates/overdrive-worker/tests/acceptance/vsock_beacon_intake.rs` | `overdrive-worker` `acceptance` | P2-S14 additive half | 3 | 1 |
| `crates/overdrive-worker/tests/integration/vsock_mesh_egress_mtls.rs` | `overdrive-worker` `integration` (Tier-3) | P5-S10 | 10 | 0 |
| `crates/overdrive-worker/src/mtls_intercept_worker.rs` (unit test) | `overdrive-worker` lib | P5-S10 stage labels | 1 | 0 |
| `crates/overdrive-worker/tests/integration/vsock_host_internal_enforce.rs` | `overdrive-worker` `integration` (Tier-3) | P5-S8 | 13 | 0 |
| `crates/overdrive-control-plane/src/guest_network/prefix_boot_acceptance.rs` | `overdrive-control-plane` lib, `#[cfg(test)]` | P4-S12, P4-S14, P4-S15, P5-S1 (boot refusal), P5-S2 | 17 | 0 |
| `crates/overdrive-control-plane/src/guest_network/prefix_boot_conformance.rs` | same, `integration-tests` | P4-S17 (driver, 3 instances) | 3 | 0 |
| `crates/overdrive-control-plane/src/guest_network/flow_io/sim/tests.rs` | `overdrive-control-plane` lib | `SimGuestFlowIo` semantics | 0 | 11 |
| `crates/overdrive-init/src/transport/tests.rs` | `overdrive-init` bin (source-local) | P6-S11, P2-S1 token form | 7 | 2 |
| `crates/overdrive-init/src/guest_flow/tests/{newtypes,session,tcp_accept,tcp_connect,half_close,udp_slots}.rs` | same | P2-S10 / S12 guest halves, P3-S2, S4..S6, S9 guest half, S10, P6-S11 | 38 | 1 |
| `crates/overdrive-init/src/guest_flow/quint_{owner_flows,flows_half,udp_slots}.rs` | same, `integration-tests` | P2-S15 guest side, P3-S8 guest side (drivers, 4 instances) | 4 | 0 |
| `tests/conformance/tests/vsock_prefix_landing_conformance.rs` | `overdrive-system-conformance` (native metal) | P4-S19 (QPROJ-8), covers P4-S16 | 1 | 0 |

Withdrawn: P6-S7 (DG-1) and P6-S8 (with the VMM backend probe, DG-1; its
`fd=` handoff obligations are P1-S12's VMM cell at the cut and P6-S13).

RED scaffolds added in this increment (every one `// SCAFFOLD: true`) are
listed with their contract in feature-delta § "Wave: DISTILL / [REF] vsock
Scaffolds".

## Gate → piece

| Gate | Owner / promise | Piece |
|---|---|---|
| G-V0 node attachment admission | `GuestAddressPool::assign` (lease + CID claim + port reservation) | P1 |
| G-V1 shared forwarding substrate startup | owner boot: probe/converge, steering load+swap, unframe attach, route | P4 (steering/boot/route) + P5 (unframe attach) |
| G-V2 guest command release | VM driver under EXEC claim (after Active) | P6 |
| G-V3 allocation forwarding activation | owner `activate` (all-or-nothing listeners) | P2 (activate/flow) + P4 (listeners) |
| G-V4 VMM vsock device | `CloudHypervisorVmm` + fork (`backend=vhost-kernel`, `fd=`) | P6 |
| G-V5 runtime recovery reopen | supervisor, per-component quiescence holders | P6 |
| G-V6 per-flow pairing | flow acceptor (`Paired` after install) | P2 (TCP flows) + P3 (datagram) + P5 (egress legs) |
| G-V7 guest network config before READY | `overdrive-init` | P6 |
| G-V8 intake listener + steering entry | owner (`GuestPrefixSteering::steer`) | P4 |

## Quint conformance drivers (QPROJ-1 to QPROJ-8, approved 2026-10-09)

Placement, projection and action map are pinned by feature-delta
§ *Conformance observation surface and action map* (QPROJ-1 to QPROJ-8) and
§ "Wave: DISTILL / [REF] vsock Conformance Drivers", under
`.claude/rules/testing.md` § "Quint conformance". Each driver is a
`quint-connect` module in its owner's crate, gated only
`#[cfg(all(test, feature = "integration-tests"))]`; only `prefix_landing`
lives in `tests/conformance` (real kernel). Every spec is referenced in place
under `specs/quint/guest-flow-owner/`. The specs define no Quint `run`, so no
`#[quint_test]` pin exists (UI-R2-1); each instance replays randomized traces
through `#[quint_run]` with the seed printed on failure. The `checks.toml`
entries named below are the design's model checks (invariants, liveness
properties and non-vacuity witnesses), not Quint runs. `cargo xtask
conformance-lint`: 0 violations over the four authored drivers.

### Driver table

| Module (instance) | Owner / driver crate | `checks.toml` model checks (invariants / witnesses) | `#[quint_run]` instances | Status |
|---|---|---|---|---|
| `prefix_boot` | host owner boot `converge_shared` / `overdrive-control-plane` `guest_network::prefix_boot_conformance` | `prefix-boot-safety`, `-faulty-safety`, `-local-only-over-link`, `-local-only-over-link-while-down`, `-no-wildcard`, `-nothing-while-down`, `-open-converged`, `-opens`, `-pin-only-after-first-attach`, `prefix-boot-witness-*`; inv 6 | `prefix_boot_conformance_boot_ok`, `_boot_faulty`, `_boot_live` | **authored** — ignored `pending DELIVER vsock attachment cut (new-step 13, steering load and swap, shared local route)` |
| `owner_flows` `flows_ok`, guest side (TcpAccept half) | `GuestFlowOwner` / `overdrive-init` `guest_flow::quint_owner_flows` | `owner-flows-safety-tlc`, `owner-flows-witness-*` (accept half); inv 4+ (accept half), 9 | `quint_owner_flows_ok_guest` | **authored; model fidelity open (UI-R2-5)** — a correct implementation may fail it until the model decision; ignored `pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init); model fidelity open, upstream UI-R2-5` |
| `owner_flows` `flows_half_guest` (S-4) | `GuestFlowOwner` / `overdrive-init` `guest_flow::quint_flows_half` | `owner-flows-half-guest-*`; inv 13 (HALFCLOSE-SIGNAL) | `quint_owner_flows_half_guest` | **authored; model fidelity open (UI-R2-5)** — same ignore |
| `udp_slots`, guest side | `GuestFlowOwner` slot pool / `overdrive-init` `guest_flow::quint_udp_slots` | `udp-slots-witness-*`, `udp-slots-safety-tlc`, `udp-slots-release-liveness`; inv 2, 14; `ReleasedSlotBecomesFree` | `quint_udp_slots_guest_slots_ok`, `_guest_slots_live` | **authored; model fidelity open (UI-R2-5)** — same ignore |
| `prefix_landing` (stateless; QPROJ-8) | the kernel over the production composition / `tests/conformance` | none — `lands(net, client, port)` evaluated per observation (`quint repl` over the spec in place) | — | **authored** — `native_metal_kvm_prefix_landing_matches_lands_for_every_client_class_and_reachable_configuration`, ignored `pending DELIVER vsock new-step 5 and attachment cut (new-steps 9, 12, 13 …)` |
| `cid_lease` (`lease_conf_ok`, `lease_conf_live`) | `GuestAddressPool` / `overdrive-control-plane` | `cid-lease-conf-*`; inv 5, `EventuallyPlaced` | `lease_conf_ok`, `lease_conf_live` | attachment cut (DG-2 rows 1, 2, 3, 6): QPROJ action map; O2 `lease_observation` + S `held_cids` / `is_live` |
| `owner_flows` `flows_ok`, host side; `flows_half_host`; `udp_slots` host side | `HostSharedGuestNetworkOwner` / `overdrive-control-plane` | `owner-flows-*`, `owner-flows-half-host-*`, `udp-slots-*` (host variables) | `flows_ok`, `flows_half_host`, `slots_ok` / `slots_live` | attachment cut (DG-2 rows 2, 3, 7, 8, 9): QPROJ host-side action maps over `SimGuestFlowIo`; O2, O3, S `socket_observation()` |
| `intake` | host owner with the pool / `overdrive-control-plane` | `intake-*`; inv 7, 8, 12 | `intake_ok`, `intake_two`, `intake_live` | attachment cut (DG-2 rows 2, 3, 7, 8, 9): O3 `intake`, `parked`, `last_listen_state`; S `steering_observation().entries`, `steering_effects()` |
| `quiescence`, `shared_table` | host owner + runtime supervisor + `SimMtlsIntercept` / `overdrive-control-plane` | `quiescence-*`, `shared-table-*`; inv 10, 11 | `quiesce_ok` / `quiesce_live`, `table_ok` / `table_live` | attachment cut (DG-2 rows 2, 3, 7, 8, 9, 10, 12): O3 `holders`, `admission`, `intake`, `flow_listeners_running`; O6 `recovery_observation()`; S `shared_table_state()` |
| `owner_flows` `flows_beacon` | `VmDriver` beacon intake / `overdrive-worker` | `owner-flows-beacon-*`; inv 4+ (beacon; ATTR-LIVE-CLAIM) | `flows_beacon` | attachment cut (DG-2 rows 13, 15): `VmDriver::start` / `stop`, `SimVmm` exit, beacon connections over `SimBeaconIo`; O4 `beacon_observation`, S `beacon_socket_observation` |

Four drivers (7 instances) are authored now (the three guest drivers with model fidelity open, UI-R2-5); seven are written at the
attachment cut because each calls a DG-2 replaced signature
(`GuestAddressPool::new` / `assign`, `HostSharedGuestNetworkOwner::new(ports)`,
`quiesce_forwarding` / `restore_forwarding`, `VmDriver::new(.., beacon)`,
READY `launch=`). `udp_slots` carries no per-side instances in `checks.toml`;
the per-side replay uses `slots_ok` / `slots_live` with the other side as
stimulus (QPROJ-1).

### Per-driver checklist — the four authored drivers (testing.md § "Quint conformance")

**`prefix_boot`** — `crates/overdrive-control-plane/src/guest_network/prefix_boot_conformance.rs`

1. *Placement / gate — yes.* Source-local in the owner crate, declared
   `#[cfg(all(test, feature = "integration-tests"))] mod prefix_boot_conformance;`
   (`guest_network.rs:79-80`); no always-false `cfg`; each instance carries
   the reasoned ignore above (`:457-472`).
2. *Owner field — yes.* `PrefixBootDriver` (`:82`) holds `serve:
   Option<Serve>` whose `Serve` (`:77`) holds the production
   `Arc<HostSharedGuestNetworkOwner>` built with `with_flow_io`, plus the seam
   `io: Arc<SimGuestFlowIo>` and the node-global `node: Arc<SimGuestNode>`.
3. *Every action mapped — yes.* `switch!` (`:243-253`) maps the spec's whole
   `step = any { Restart, BootStep, Fail, Crash }` (`prefix_boot.qnt:205`)
   plus `init`: `Restart(b)` a new `SimGuestFlowIo` over the persisted node
   with program `b` and a new owner; `BootStep` call `converge_shared` /
   release the held `Load`, `Attach`, `Pin`, route effect; `Fail` fail the
   held effect; `Crash` abort the pending boot, drop the owner, end the seam's
   process. No empty or catch-all arm. The `step` arm maps quint 0.32's
   mislabelled initial state to `init` (commented, `:245-248`; UI-R2-13).
4. *State fields and accessors — yes.* `PrefixBootState` (`:284`): `link`,
   `prog`, `route` ← `SimGuestFlowIo::steering_observation()`; `phase` ← the
   driver's record of its own `converge_shared` call (not called / pending /
   `Ok`) combined with `steering_effects()` (`from_driver`, `:395-411`;
   `boot_phase`, `:416`).
5. *No sim read except marked arbiters — yes.* The two seam reads are marked
   `// conformance-lint: sim-arbiter-ok …` (`:397`, `:399`): the node-global
   pinned link / program / tagged route and the boot effects this `serve`'s
   owner issued, paired with Tier-3 R5-24, R5-34, R5-25.
6. *Spec in place, trace mode — yes.* `#[quint_run(spec =
   "../../specs/quint/guest-flow-owner/prefix_boot.qnt", main = …)]` for
   `boot_ok`, `boot_faulty`, `boot_live` (`:457`, `:463`, `:469`); no
   `#[quint_test]` (UI-R2-1).

**`owner_flows` `flows_ok`, guest side** — `crates/overdrive-init/src/guest_flow/quint_owner_flows.rs`

1. *Placement / gate — yes.* `#[cfg(all(test, feature = "integration-tests"))]
   mod quint_owner_flows;` (`guest_flow/mod.rs:47-48`); reasoned ignore on the
   instance (`:965-969`).
2. *Owner field — yes.* `FlowsOkDriver` (`:185`) holds `guest:
   GuestProcess`, the current launch's production `GuestFlowOwner`, with every
   launch's `SimGuestOwnerIo` and the launch's `SimGuestClock`. The remaining
   fields are the host-side stimulus model (QPROJ-1: the host owner, the
   lease, the VM and the applications are stimulus).
3. *Every action mapped — yes.* `switch!` (`:815-855`) maps every action of
   the spec's `step`, including the host, lease and VM actions as seam
   stimuli (`VmStart` a new owner per launch, `VmStop` `guest_owner_dies` and
   drop) and the guest actions as `SimGuestOwnerIo` releases followed by
   `dispatch_next`. The hazard-only `HostClaimSession`, `HostCheckFlow`,
   `HostRegisterFlow` return a typed `HazardOnly` error (`:832`, `:837-838`),
   never a no-op. No catch-all arm.
4. *State fields and accessors — yes.* `FlowsOkGuestState` (`:864`):
   `gf(h).st` ← O5 `GuestFlowOwner::guest_observation().accepted`
   (`Accepted` → `acc`, `Routed` → `routed`); `reset` ← S `reset_in_queue`;
   `toHostPaired`, `toHostRefused` ← S `outbox` of every launch
   (`from_driver`, `:904-961`). `closed` is "seen in `accepted` or
   `reset_in_queue` and no longer present", using the identity bijection's
   record `held` of production flow ids it has seen (`:205`); that record
   supplies a pre-image for a history fact, never a phase.
5. *No sim read except marked arbiters — yes.* Two marked reads (`:910`,
   `:945`), QPROJ-7 guest-owner seam arbiter, paired with Tier-3 R5-31 /
   R5-28.
6. *Spec in place, trace mode — yes.* `#[quint_run(spec =
   "../../specs/quint/guest-flow-owner/owner_flows.qnt", main = "flows_ok")]`
   (`:965`).

**`owner_flows` `flows_half_guest`** — `crates/overdrive-init/src/guest_flow/quint_flows_half.rs`

1. *Placement / gate — yes.* `guest_flow/mod.rs:45-46`; reasoned ignore
   (`:805-809`).
2. *Owner field — yes.* `FlowsHalfDriver` (`:203`) holds `owner:
   GuestProcess` (the production `GuestFlowOwner`, `Dead` once dropped), its
   seam clone `io: SimGuestOwnerIo` and `clock: Arc<SimGuestClock>`; `host`
   and `ctl_down` are stimulus.
3. *Every action mapped — yes.* `switch!` (`:631-651`): `init(o)` and the
   mislabelled `step(o)`; `HcFarHalfClose`, `HcFarReset` → `app_end`;
   `HcPair` → `complete_app_connect` / host `Paired`; `HcAnnounce` →
   `set_drained`; `HcRecv` → `host_control`; `VsockEndArrives`,
   `VsockEndReachesSurvivor` → `host_vsock_end`; `HcLossObserved` →
   `release_session_end`; `HcFlowError` → `SimGuestClock` past
   `FLOW_DRAIN_DEADLINE` or `flow_vsock_error`; `HcOwnerDies` →
   `guest_owner_dies` and drop; `HcCleanClose` → settle; `HcSessionDrop` →
   stimulus; `HcLateAck` → typed `HazardOnly`; `HcPeerDefectEnd(g)` → typed
   error (disabled for the side under test, S-4). No catch-all arm.
4. *State fields and accessors — yes.* `HalfGuestState` (`:674`) /
   `OpenSide` (`:659`): `paired`, `outEof`, `ann`, `acked`, `outShut`,
   `open` ← O5 `guest_observation().flows` / `awaiting_paired` / `accepted`;
   `loss`, `ch`, `vEnd`, `vEndSent`, `far`, `ctlDown` ← S
   `guest_socket_observation()` (`session_ended`, `outbox`,
   `vsock_ends_in_flight`, `vsock_ended`, `far_end`); `dead` ← C (the driver
   dropped the owner) (`from_driver`, `:743-800`).
5. *No sim read except marked arbiters — yes.* One marked read (`:745`),
   QPROJ-7 arbiter, paired with Tier-3 R5-10 / R5-45 / R5-28.
6. *Spec in place, trace mode — yes.* `#[quint_run(spec = …/owner_flows.qnt,
   main = "flows_half_guest")]` (`:805`).

**`udp_slots`, guest side** — `crates/overdrive-init/src/guest_flow/quint_udp_slots.rs`

1. *Placement / gate — yes.* `guest_flow/mod.rs:49-50`; reasoned ignore on
   both instances (`:668-678`).
2. *Owner field — yes.* `UdpSlotsDriver` (`:125`) holds `owner:
   GuestProcess` (the production `GuestFlowOwner`), `io: SimGuestOwnerIo`,
   `clock: Arc<SimGuestClock>`; `stimulus` is the host and application side;
   `fids` is the association-id bijection.
3. *Every action mapped — yes.* `switch!` (`:456-480`): `AppClaim`,
   `AppSendOnSlot` → `app_send_on`; `GuestReady(s, cur)` → `release_slot_ready`
   and both leg connects; `HostPair`, `HostRefuse`, `HostAbortAssoc` →
   stimulus; `GuestRecvPaired`, `GuestRecvEnd` → `host_control`;
   `GuestPairingTimeout`, `GuestAudit`, `IdleRelease` → `SimGuestClock`;
   `HostRecvEnd` → host consumption; `SockClose`, `SockCloseReportLost` →
   `app_close_socket`; `GuestReleaseSock` → `release_socket_report`;
   `AppPick`, `AppTake`, `Refill` → typed `HazardOnly`. No catch-all arm. A
   trace that exceeds the model's association-id bound returns
   `DriverError::ModelBound` (`:267`; UI-R2-5(b)).
4. *State fields and accessors — yes.* `UdpSlotsState` (`:499`):
   `slots(s).owned / sock / dest / fid / routed` ← O5
   `guest_observation().slots`; `slots(s).parked` ← S `parked`; `ready` ← S
   `readiness`, current iff its generation equals O5's for the slot;
   `releasePending` ← S `release_reports`; `toHostAbort` ← S `outbox`;
   `toHostEnd` ← S `legs_closed_unobserved` (`from_driver`, `:579-663`).
5. *No sim read except marked arbiters — yes.* One marked read (`:581`),
   QPROJ-7 arbiter, paired with Tier-3 R5-14 / R5-27.
6. *Spec in place, trace mode — yes.* `#[quint_run(spec = …/udp_slots.qnt,
   main = "slots_ok")]` and `main = "slots_live"` (`:668`, `:674`).

Open fidelity items on the three guest drivers (UI-R2-5, HIGH): one guest
clock drives every timer, `HcCleanClose` is not a separate production step
(production clears linger and closes in the dispatch that completes the second
half-close, as `a_clean_close_clears_linger_before_closing` asserts), how an
owner learns a half-closed direction has drained is unpinned on both sides
(no drain-complete event in `GuestIoEvent`, no re-check trigger in § *Flow
end*), an error on an unrouted V_g is not observable to the owner before
routing, and some `flows_ok` orders are not producible. A correct
implementation may fail these drivers until the model decision. The drivers
are therefore authored coverage with model fidelity open, not settled
evidence; the decision is upstream.

### `prefix_landing` — per-driver checklist (testing.md § "Quint conformance")

The driver evaluates a stateless definition, so quint-connect's trace
machinery does not apply; `cargo xtask conformance-lint` scans only
quint-connect files and does not scan it. The checklist is answered for the
same intent (test file `tests/conformance/tests/vsock_prefix_landing_conformance.rs`,
harness `tests/conformance/src/prefix_landing.rs`):

1. **Placement / gating — yes.** `tests/conformance` (multi-owner, QPROJ-8);
   only gate `#![cfg(feature = "integration-tests")]` (test file `:33`); no
   always-false `cfg`; reasoned `#[ignore = "pending DELIVER vsock new-step 5 and attachment cut (new-steps 9, 12, 13 …)"]` (`:83`).
2. **Driver holds the production composition — yes.** `LandingNode`
   (`prefix_landing.rs:904`) owns the `ServerHandle` from `run_server`; the
   scenario boots it (test file `:120`, `:163`), deploys through the HTTPS API
   (`:122`), kills it (`LandingNode::kill`, `prefix_landing.rs:951`) and stops
   it gracefully (`LandingNode::shutdown`, `:956`).
3. **Every action calls the owner — yes (no action set).** The module has no
   `step` relation; each configuration is reached through production paths;
   the clients are real connects (`observe_landing`, `prefix_landing.rs:721`).
4. **State from pinned sources — yes.** `Net` inputs are independent kernel
   queries per QPROJ-8: `read_route` (`:291`), `read_prefix_rules` (`:318`),
   `read_leg_c` (`:376`), `read_intake_listeners` (`:386`),
   `read_steering_program` (`:403`), `read_steering_entries` (`:421`),
   assembled by `read_net` (`:456`); `marked` is the caller's fact (QPROJ-1
   source (d)); the observed landing is the accepting socket's first bytes
   (`classify_accepted`, `:745`) or the connect error plus `ip route get`
   (`routes_locally`, `:473`).
5. **No sim, no driver bookkeeping — yes.** No `Sim*` adapter is read; the
   only sim in the composition is `SimKek` at the KEK port. The model side is
   `model_landings` (`:220`) over the spec in place (`SPEC`, `:79`).
6. **Spec in place; trace modes — partly N/A.** The spec is referenced in
   place; `#[quint_test]` / `#[quint_run]` do not apply to a stateless module;
   every comparison row is retained in an append-only record
   (`ComparisonRecord::compare` `:1044`, `assert_conforms` `:1080`).

Items on this driver: G-PL-1 (steering key layout), G-PL-2 (route protocol
tag), G-PL-5 (shared table name) and G-PL-7 (harness-spelled constants) are
closed by DG-9 and DG-14 — the harness now reads the exported
`MTLS_SHARED_NFT_TABLE`, `GUEST_NODE_PREFIX`, `GUEST_PREFIX_ROUTE_PROTOCOL`,
`GUEST_PREFIX_STEERING_PIN_DIR`, `GUEST_INTAKE_HOST_PORT_BASE`,
`GUEST_STEERING_KEY_LEN` and `guest_steering_key`. G-PL-3 (`quint` on the
metal host) is DELIVER new-step 1 provisioning. G-PL-4 (the oracle runs
`quint repl`, quint-connect has no definition-evaluation surface) and G-PL-6
(`run-native-metal.sh` selects every ignored `native_metal_kvm_*` test, so it
runs this one RED until its steps land) remain notes.

## Decision → piece (every approved decision lands in exactly one piece)

| Piece | Decisions owned |
|---|---|
| P1 | D3, D11, D16, D16-CLAIM, D8a-CAP, INTAKE-PORTS, D15-R2 (reserved-range deploy refusal) |
| P2 | D7, D9, D14, D17, D18, D18a, D15-R1, D15-R3, ATTR-TOKEN, ATTR-LIVE-CLAIM (U-1, U-5) |
| P3 | D4, D5, D10, D25, D25-BIND, D15-R2 (guest bind/pool/idle), CELL-RECREATE (U-2) |
| P4 | D8, D8-INTAKE, D8-PEER, D8-PREFIX-CFG, D8-PREFIX-VIP, D8a, D8a-LOOKUP, D8a-LOAD-SWAP, D23, INTAKE-BIND, PREFIX-REPLY, D5a-SERVE-IFACE (veth carries unframe), U-3, U-4, U-6, M-7 |
| P5 | D5a, D5a-IFACE, D5a-IFACE-BOOT, D5a-LATE, IMG-IFACE-BOOT, USO-NO-SEGMENT, IFACE-NO-STACK, FRAG-MAP-LIFECYCLE, D24, D24a, D26, D26-PINS |
| P6 | D2, D13, D21, D8a-HOLD, D19, D19-LINKS, HALFCLOSE-SIGNAL (crash matrix), G-V7 guest config, QUINT process |

## R5-* boundary obligation → piece

| Piece | R5-* owned |
|---|---|
| P1 | R5-19, R5-21, R5-37, R5-44 (deploy + intake-block boot refusal) |
| P2 | R5-9, R5-16, R5-17, R5-18, R5-20, R5-26, R5-28, R5-31, R5-41, R5-45 (owner-seam + codec half-close) |
| P3 | R5-14, R5-15, R5-27 |
| P4 | R5-1, R5-2, R5-3, R5-22, R5-23, R5-24, R5-25, R5-29, R5-34, R5-39, R5-40, R5-42, R5-43 |
| P5 | R5-6, R5-7, R5-8, R5-38, R5-46, R5-47, R5-48 |
| P6 | R5-4, R5-5, R5-10, R5-13, R5-30, R5-32, R5-45 (Tier-3 inside R5-10); the Tier-3 halves of R5-41 (kill-and-reuse through `serve` + `deploy`) and R5-28 / R5-31 (the V-24 shape through `serve`), as P6-S13 cells |

(R5-5 state-interleaving is split: `ListenState` interleaving in P4 (`intake`),
quiesce/restore interleaving in P6; each doc restates its half. R5-13 intake-child
identity lives in P4's `intake` steering scope with the V-20 INTAKE-TAG-MAP map.
R5-45 half-close: codec + owner-seam in P2; the Tier-3 crash-matrix cell in P6.)

## Validation-item → piece (DELIVER-step items; scenarios RED `#[ignore]`)

| V-item | Status | Piece | Note |
|---|---|---|---|
| V-1(c) fork `fd=` under production launch | open (DELIVER step 3) | P6 | handoff proven root (P-39) |
| V-6 walking skeleton serve+deploy | open (DELIVER 10-01) | P6 | includes V-15 |
| V-8 fail-closed in production composition | open (DELIVER 10-02) | P6 | |
| V-9 density & costs | open (DELIVER ≈08-04/10-05) | P6 | surfaced-not-relaxed |
| V-12 host-local connect source address | open (DELIVER step 9) | P4 | one D15 row |
| V-19 guest-prefix exposure | covered by P4-S19 (`prefix_landing`, authored; runs on native metal once new-steps 5, 9, 12 and 13 land) | P4 | |
| V-28 6.18 re-run | open, not a gate (KVER) | P6 | runs with DELIVER Tier-3 |
| contract-shaping (V-10/22/24/25/26/5c/13/16/17/18/20/21) | **all DONE** (P-37..P-48) | — | nothing blocked on them |

## 6-row boundary-scenario table (feature-delta §"Required boundary scenarios")

Rows 1 (available), 2 (typed failure in owner domain), 3 (unrelated unchanged),
4 (late success can't resurrect), 5 (disconnect/reconnect), 6 (feature disabled
= sim `transport: None`) are enumerated per gate inside the owning piece doc.
Row 6 for every gate is the planned attachment-cut cell P6-S12b (a
`SimDriver` allocation with `AllocationSpec.transport: None` through the
action shim: no lease, no CID claim, no `activate`, `alloc_status` equal to
the pre-feature `network: None` baseline). The startup-refusal projection of
G-V1 row 2 through `run_server` is the planned attachment-cut cell
"`run_server` boot refusal" under P4-S13.
Rows 2/4/5 for G-V2/G-V3/G-V5/G-V6/G-V8 need a seeded `overdrive-sim` invariant
through the production owner path; kernel effects (G-V1/G-V4/G-V6/G-V7/G-V8) need
native Tier-3 through `serve` + `deploy`.

## Retired / not-deleted

- **Re-created by the 2026-10-09 pass, updated 2026-10-10**:
  `distill/red-classification.md` (vsock model) and
  `distill/upstream-issues.md` (round 2 appended).
- **Retired in the attachment cut** (DG-2; coupled to old production-code
  removal, NOT deleted now): the OLD-model GREEN test modules cataloged in
  [`shared-test-infrastructure.md`](shared-test-infrastructure.md) §7.

//! P4 — intake listener lifecycle (seeded sim through the production owner).
//!
//! netns-density-295 vsock Attachment Replacement DESIGN
//! (`docs/feature/netns-density-295/feature-delta.md` § "Owner and provisioner
//! — `overdrive-control-plane::guest_network`": `activate` / `teardown`,
//! "One writer per allocation (M-7)", "Intake listener mirroring (D23)";
//! § "Driven port — guest-prefix steering" (`steer`, capacity); Gate G-V8;
//! ADR-0163 D23/U-6/M-7, ADR-0171 D8a-LOOKUP, ADR-0176 D8a-CAP,
//! ADR-0178 INTAKE-BIND). Quint oracle: `specs/quint/guest-flow-owner/intake.qnt`
//! (invariants 7, 8, 12; replayed by `vsock_intake_conformance.rs`).
//!
//! Lane: seeded `overdrive-sim` through the **production owner path** — the
//! owner's driving ports (`provision` / `activate` / `teardown` /
//! `quiesce_forwarding` / `restore_forwarding`) with `SimGuestPrefixSteering`
//! at the private steering-effect seam and a scripted `ListenState` / session
//! feed (the seeded lane's test-local ports, feature-delta § "Evidence-lane
//! matrix"). Example-based (Mandate 9/11 — layer-3 sad paths enumerated, never
//! PBT-generated); the one pure function (`guest_intake_host_port`) carries a
//! `proptest` (layer-1). Default unit lane — no real infrastructure.
//!
//! ## State-delta universe (Mandate 8, Rust realization)
//!
//! The Rust codebase has no `nwave_ai.state_delta` port; Mandate 8 is realized
//! as before/after snapshots of the owner's **port-exposed observable surface**
//! — never private owner fields:
//!   * `cid_state[cid]`            — Provisioned | Active | Retiring | Absent
//!   * `open_intake[cid]`          — set of declared ports with an open listener
//!   * `listener_bind[(cid,port)]` — the host address:port the listener bound
//!   * `steer_entries`             — set of (workload_addr, declared port) entries
//!   * `events`                    — emitted `guest_intake.bind_failed { .. }`
//! Anything in the universe not named in the expected delta MUST be unchanged
//! (fail-closed). See `capture_universe`.
//!
//! ## GATED — pending DELIVER roadmap reconciliation + owner/sim-steering seam
//!
//! The production surface this file calls does NOT exist yet
//! (`SharedGuestNetworkOwner`, `GuestNetworkProvisioner`, `ForwardingActivation`,
//! `GuestNetworkError`, `SimGuestPrefixSteering`, the seeded-lane event feed).
//! Per shared-test-infrastructure.md § 0 / § 3 and CLAUDE.md § "Implement to
//! the design — never invent API surface", none is invented here. DELIVER
//! creates the owner + `SimGuestPrefixSteering` + the seeded event feed at the
//! roadmap reconciliation + step 2/9 and only then **registers this module** in
//! `crates/overdrive-control-plane/tests/acceptance.rs` (`mod vsock_intake_listener;`).
//! Until registered Cargo never compiles this file; the authored bodies below
//! are the pre-authored RED scaffolds the crafter lifts.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4};

use overdrive_control_plane::guest_network::{
    ForwardingActivation, GuestNetworkError, SharedGuestNetworkOwner,
};
use overdrive_core::guest_transport::{GUEST_INTAKE_HOST_PORT_BASE, guest_intake_host_port};
use overdrive_core::id::AllocationId;
use overdrive_core::net::ListenPort;

// --- universe snapshot (Mandate 8) -------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct Universe {
    cid_state: BTreeMap<u32, &'static str>,
    open_intake: BTreeMap<u32, BTreeSet<u16>>,
    listener_bind: BTreeMap<(u32, u16), SocketAddrV4>,
    steer_entries: BTreeSet<(Ipv4Addr, u16)>,
    bind_failed_events: Vec<BindFailed>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct BindFailed {
    alloc: AllocationId,
    port: u16,
    retried: bool,
}

/// DELIVER-provided seeded-sim harness over the production owner + the sim
/// steering seam + the scripted event feed. Named so the scaffold reads; the
/// crafter supplies it (test-support surface) co-sequenced with the owner.
struct IntakeHarness;
impl IntakeHarness {
    fn capture_universe(&self) -> Universe {
        unreachable!("DELIVER harness: project the owner's observable surface")
    }
    fn owner(&self) -> &dyn SharedGuestNetworkOwner {
        unreachable!("DELIVER harness: the production owner under seeded sim")
    }
    /// Script the sim steering's next N `steer` calls to fail `KernelMemory`.
    fn script_steer_kernel_memory(&self, _attempts: u32) {}
    /// Script the sim steering's next `steer` to fail with a defect variant.
    fn script_steer_defect_map_update(&self) {}
    /// Feed a guest `ListenState` for a CID (the ports it reports listening).
    fn feed_listen_state(&self, _cid: u32, _listening: &[u16]) {}
    fn open_session(&self, _cid: u32) {}
    fn lose_session(&self, _cid: u32) {}
    fn advance_audit_cadence(&self) {}
    fn plan_for(
        &self,
        _cid: u32,
        _declared: &[u16],
    ) -> overdrive_control_plane::guest_network::GuestNetworkPlan {
        unreachable!("DELIVER harness: a GuestNetworkPlan for the CID")
    }
}
fn harness_with_prefix() -> IntakeHarness {
    unreachable!("DELIVER harness builder (seeded sim, SimGuestPrefixSteering seam)")
}

fn workload_addr(cid: u32) -> Ipv4Addr {
    // 100.95.x.y derived from the lease offset; DELIVER's pool assigns it.
    let off = cid & 0xffff;
    Ipv4Addr::new(100, 95, (off >> 8) as u8, (off & 0xff) as u8)
}

// --- P4-S6 / R5-5 (listen half), M-7, G-V3/G-V8: ListenState interleavings ----

#[tokio::test]
#[ignore = "pending DELIVER step 2/9 (owner intake seam + SimGuestPrefixSteering; reconciliation)"]
async fn a_listen_state_during_activation_teardown_or_quiescence_yields_the_invariant_state() {
    // The design's serving invariant (feature-delta § "One writer per allocation
    // (M-7)"):  serving(port) ⇔ Active ∧ no quiescence holder ∧ session Live ∧
    // last ListenState(port)=listening ∧ port ∈ listen_ports(). For each
    // interleaving the final listener/entry state MUST equal this function.
    let h = harness_with_prefix();
    let cid = 1u32;
    let declared = [8080u16, 8443];
    h.owner().provision(&h.plan_for(cid, &declared)).await.expect("provision");
    h.open_session(cid);
    h.feed_listen_state(cid, &[8080]); // only 8080 up during activation

    let before = h.capture_universe();
    let activated = h.owner().activate(&h.plan_for(cid, &declared)).await.expect("activate");
    assert!(matches!(activated, ForwardingActivation::Activated));
    let after = h.capture_universe();

    // Expected delta: cid -> Active; an open, steered listener for 8080 only
    // (8443 not reported listening); bound in the intake block; 8443 absent.
    assert_eq!(after.cid_state.get(&cid), Some(&"Active"));
    assert_eq!(after.open_intake.get(&cid), Some(&BTreeSet::from([8080u16])));
    assert_eq!(
        after.listener_bind.get(&(cid, 8080)),
        Some(&SocketAddrV4::new(workload_addr(cid), guest_intake_host_port(0).unwrap())),
    );
    assert!(after.steer_entries.contains(&(workload_addr(cid), 8080)));
    assert!(!after.steer_entries.contains(&(workload_addr(cid), 8443)));
    // Fail-closed: no other allocation's state changed.
    assert_eq!(
        before.cid_state.len() + 1,
        after.cid_state.len(),
        "only this CID entered the table"
    );

    // A late ListenState adding 8443 (serialized per-CID, M-7) adds exactly its
    // steered listener; nothing else moves.
    h.feed_listen_state(cid, &[8080, 8443]);
    let after2 = h.capture_universe();
    assert_eq!(after2.open_intake.get(&cid), Some(&BTreeSet::from([8080u16, 8443])));
    assert!(after2.steer_entries.contains(&(workload_addr(cid), 8443)));
    assert_eq!(
        after2.listener_bind.get(&(cid, 8080)),
        after.listener_bind.get(&(cid, 8080)),
        "8080 unchanged"
    );
}

// --- P4-S7 / R5-29, U-6, inv 8: all-or-nothing activation rollback ------------

#[tokio::test]
#[ignore = "pending DELIVER step 2/9 (owner intake seam + SimGuestPrefixSteering; reconciliation)"]
async fn activation_with_a_failing_steer_rolls_back_wholly_and_leaves_the_cid_provisioned() {
    // GIVEN three reported-listening ports where the SECOND steer fails (a
    // defect variant, not retried).
    let h = harness_with_prefix();
    let cid = 2u32;
    let declared = [9001u16, 9002, 9003];
    h.owner().provision(&h.plan_for(cid, &declared)).await.expect("provision");
    h.open_session(cid);
    h.feed_listen_state(cid, &declared);
    h.script_steer_defect_map_update(); // the second steer

    let before = h.capture_universe();
    // WHEN activate runs:
    let err =
        h.owner().activate(&h.plan_for(cid, &declared)).await.expect_err("activation returns Err");
    assert!(matches!(
        err,
        GuestNetworkError::Io { .. } | GuestNetworkError::GuestPrefixSteeringConverge { .. }
    ));
    let after = h.capture_universe();

    // THEN the CID stays Provisioned and NO listener or steering entry of the
    // CID remains (the rollback closes listeners and cannot fail — a close
    // removes its entry in the same kernel step).
    assert_eq!(after.cid_state.get(&cid), Some(&"Provisioned"), "CID stays Provisioned");
    assert_eq!(
        after.open_intake.get(&cid).map(BTreeSet::len).unwrap_or(0),
        0,
        "no listener of the CID remains"
    );
    assert!(
        after.steer_entries.iter().all(|(a, _)| *a != workload_addr(cid)),
        "no steering entry of the CID remains",
    );
    // Fail-closed: the universe is otherwise identical to before (no partial
    // listener left behind on any port).
    assert_eq!(before.steer_entries, after.steer_entries, "all-or-nothing: entry set unchanged");
}

// --- P4-S8 / R5-22, D8a-LOOKUP/D8a-CAP, G-V8: steer failure classes -----------

#[tokio::test]
#[ignore = "pending DELIVER step 2/9 (owner intake seam + SimGuestPrefixSteering; reconciliation)"]
async fn a_kernel_memory_steer_failure_is_retried_at_the_audit_cadence() {
    // GIVEN an Active CID whose port's steer fails `KernelMemory` for N attempts
    // then succeeds.
    let h = harness_with_prefix();
    let (cid, port) = (3u32, 7000u16);
    activate_single_port(&h, cid, port).await;
    h.script_steer_kernel_memory(2);
    h.feed_listen_state(cid, &[port]); // triggers bring-up

    // THEN while refused: no steering entry, the listener closed, and each
    // failed attempt emits bind_failed { retried: true }.
    let mid = h.capture_universe();
    assert!(!mid.steer_entries.contains(&(workload_addr(cid), port)), "refused while KernelMemory");
    assert_eq!(
        mid.open_intake.get(&cid).map(BTreeSet::len).unwrap_or(0),
        0,
        "listener closed between attempts"
    );
    assert!(
        mid.bind_failed_events.iter().any(|e| e.port == port && e.retried),
        "guest_intake.bind_failed {{ retried: true }} emitted",
    );
    // AND after the N+1th attempt (audit cadence) the port serves.
    h.advance_audit_cadence();
    h.advance_audit_cadence();
    let after = h.capture_universe();
    assert!(
        after.steer_entries.contains(&(workload_addr(cid), port)),
        "serves after retry succeeds"
    );
}

#[tokio::test]
#[ignore = "pending DELIVER step 2/9 (owner intake seam + SimGuestPrefixSteering; reconciliation)"]
async fn a_defect_steer_failure_is_not_retried_and_serves_again_only_after_the_next_listen_state() {
    let h = harness_with_prefix();
    let (cid, port) = (4u32, 7100u16);
    activate_single_port(&h, cid, port).await;
    h.script_steer_defect_map_update();
    h.feed_listen_state(cid, &[port]);

    // THEN the listener is closed, bind_failed { retried: false }, no retry.
    let after = h.capture_universe();
    assert!(
        after.bind_failed_events.iter().any(|e| e.port == port && !e.retried),
        "guest_intake.bind_failed {{ retried: false }} — an Overdrive defect, not retried",
    );
    assert!(
        !after.steer_entries.contains(&(workload_addr(cid), port)),
        "port refused, not retried"
    );
    h.advance_audit_cadence();
    assert_eq!(h.capture_universe(), after, "no retry at the audit cadence");

    // AND it serves again only on the next ListenState transition (re-want).
    h.feed_listen_state(cid, &[]); // off
    h.feed_listen_state(cid, &[port]); // on again -> re-want
    assert!(
        h.capture_universe().steer_entries.contains(&(workload_addr(cid), port)),
        "serves after re-want"
    );
}

#[tokio::test]
#[ignore = "pending DELIVER step 2/9 (owner intake seam + SimGuestPrefixSteering; reconciliation)"]
async fn teardown_returns_ok_without_waiting_on_any_steering_write() {
    // teardown closes listeners (each close removes its entry in the same kernel
    // step, so the removal cannot fail) and returns Ok; the lease releases.
    let h = harness_with_prefix();
    let (cid, port) = (5u32, 7200u16);
    activate_single_port(&h, cid, port).await;
    h.owner()
        .teardown(&h.plan_for(cid, &[port]))
        .await
        .expect("teardown Ok, no steering write to wait on");
    let after = h.capture_universe();
    assert_eq!(after.cid_state.get(&cid).copied().unwrap_or("Absent"), "Absent");
    assert!(
        after.steer_entries.iter().all(|(a, _)| *a != workload_addr(cid)),
        "no entry of the CID remains"
    );
}

async fn activate_single_port(h: &IntakeHarness, cid: u32, port: u16) {
    h.owner().provision(&h.plan_for(cid, &[port])).await.expect("provision");
    h.open_session(cid);
    h.feed_listen_state(cid, &[port]);
    h.owner().activate(&h.plan_for(cid, &[port])).await.expect("activate");
}

// --- P4-S9 / R5-42, INTAKE-BIND: intake bind in the fixed block ---------------

#[tokio::test]
#[ignore = "pending DELIVER step 2/9 (owner intake seam + SimGuestPrefixSteering; reconciliation)"]
async fn every_intake_listener_binds_the_workload_address_in_the_fixed_block() {
    // Two VMs declaring the SAME ports bind the same block ports on their own
    // addresses and both serve (never 0.0.0.0, never the declared port).
    let h = harness_with_prefix();
    let declared = [443u16, 80, 8080]; // ascending distinct -> 80, 443, 8080
    for cid in [6u32, 7] {
        activate_ports(&h, cid, &declared).await;
    }
    let u = h.capture_universe();
    // ascending declared ports: 80 (i=0), 443 (i=1), 8080 (i=2)
    for cid in [6u32, 7] {
        assert_eq!(
            u.listener_bind.get(&(cid, 80)),
            Some(&SocketAddrV4::new(workload_addr(cid), GUEST_INTAKE_HOST_PORT_BASE))
        );
        assert_eq!(
            u.listener_bind.get(&(cid, 443)),
            Some(&SocketAddrV4::new(workload_addr(cid), GUEST_INTAKE_HOST_PORT_BASE + 1))
        );
        assert_eq!(
            u.listener_bind.get(&(cid, 8080)),
            Some(&SocketAddrV4::new(workload_addr(cid), GUEST_INTAKE_HOST_PORT_BASE + 2))
        );
    }
    assert_ne!(
        workload_addr(6),
        workload_addr(7),
        "two VMs bind the same block ports on distinct addresses"
    );
}

async fn activate_ports(h: &IntakeHarness, cid: u32, declared: &[u16]) {
    h.owner().provision(&h.plan_for(cid, declared)).await.expect("provision");
    h.open_session(cid);
    h.feed_listen_state(cid, declared);
    h.owner().activate(&h.plan_for(cid, declared)).await.expect("activate");
}

/// P4-S9 pure-function property (layer-1 PBT): `guest_intake_host_port` is
/// injective over `0..GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX`, lands in the
/// block, and is `None` beyond.
mod intake_host_port_properties {
    use super::*;
    use overdrive_core::guest_transport::{
        GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX, guest_intake_host_ports,
    };
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn injective_within_block_none_beyond(a in 0u16..8192, b in 0u16..8192) {
            let (pa, pb) = (guest_intake_host_port(a), guest_intake_host_port(b));
            // in-block ⇔ index < MAX
            prop_assert_eq!(pa.is_some(), a < GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX);
            if let Some(p) = pa {
                prop_assert!(guest_intake_host_ports().contains(&p), "lands in the intake block");
            }
            // injective: equal ports ⇒ equal indices (both in-block)
            if let (Some(x), Some(y)) = (pa, pb) {
                prop_assert_eq!(x == y, a == b, "injective over the block");
            }
        }
    }
}

// --- P4-S10 / R5-13, V-20, INTAKE-TAG-MAP: intake-child identity --------------
//
// A leg-C transparent child with the same local address/port as an intake child
// is never installed by `sock_ops` (the clone-flagged socket-storage map stores
// only intake listeners). This is a kernel `sock_ops` fact (V-20 HOLDS, P-46);
// its production assertion is the P6 walking-skeleton Tier-3 cell. The
// seeded-sim counterpart below pins the OWNER contract that only an intake
// listener is ever tagged (`register_intake_listener`), never a leg-C child.

#[tokio::test]
#[ignore = "pending DELIVER step 2/9 (owner intake seam; intake-child tagging; reconciliation)"]
async fn only_intake_listeners_are_tagged_never_a_leg_c_child_on_the_same_address_port() {
    let h = harness_with_prefix();
    let (cid, port) = (8u32, 6443u16);
    activate_single_port(&h, cid, port).await;
    // The owner tags exactly its intake listeners. A leg-C transparent child on
    // the same (workload_addr, port) is installed by a different path and is
    // never in the clone-flagged intake-tag map. The sim records which fds the
    // owner tagged; the assertion is that the set is exactly the intake set.
    let tagged = tagged_intake_children(&h);
    assert_eq!(tagged.len(), 1, "exactly the one intake listener is tagged");
    assert!(
        tagged.iter().all(|a| a.ip() == &workload_addr(cid)),
        "no leg-C child is ever tagged, even sharing the address/port",
    );
}
fn tagged_intake_children(_h: &IntakeHarness) -> BTreeSet<SocketAddrV4> {
    unreachable!("DELIVER harness: the sim sock_ops tag map's recorded intake children")
}

// --- P4-S11 / G-V8 boundary rows (required boundary scenarios, rows 2-5) -------

#[tokio::test]
#[ignore = "pending DELIVER step 2/9 (owner intake seam + SimGuestPrefixSteering; reconciliation)"]
async fn g_v8_boundary_rows_row2_bind_failed_retried_row3_isolation_row4_late_row5_session_loss() {
    let h = harness_with_prefix();

    // row 2 — unavailable/timeout: a bind failure emits bind_failed { retried }.
    let (cid, port) = (9u32, 5000u16);
    activate_single_port(&h, cid, port).await;
    h.script_steer_kernel_memory(1);
    h.feed_listen_state(cid, &[port]);
    assert!(
        h.capture_universe().bind_failed_events.iter().any(|e| e.port == port && e.retried),
        "row2"
    );

    // row 3 — unrelated state unchanged: another allocation's ports/state do not move.
    let other = 10u32;
    activate_single_port(&h, other, 5001).await;
    let snap = h.capture_universe();
    h.feed_listen_state(cid, &[port]); // churn on `cid`
    let after = h.capture_universe();
    assert_eq!(
        after.open_intake.get(&other),
        snap.open_intake.get(&other),
        "row3: other alloc unchanged"
    );

    // row 4 — late success cannot resurrect: a ListenState after teardown binds nothing.
    h.owner().teardown(&h.plan_for(cid, &[port])).await.expect("teardown");
    h.feed_listen_state(cid, &[port]); // late
    assert_eq!(
        h.capture_universe().open_intake.get(&cid).map(BTreeSet::len).unwrap_or(0),
        0,
        "row4: binds nothing"
    );

    // row 5 — session loss closes every intake listener of the CID; reconnect re-binds.
    let (c2, p2) = (11u32, 5002u16);
    activate_single_port(&h, c2, p2).await;
    h.lose_session(c2);
    assert_eq!(
        h.capture_universe().open_intake.get(&c2).map(BTreeSet::len).unwrap_or(0),
        0,
        "row5: all closed on loss"
    );
    h.open_session(c2);
    h.feed_listen_state(c2, &[p2]); // guest re-sends full state
    assert!(
        h.capture_universe().steer_entries.contains(&(workload_addr(c2), p2)),
        "row5: re-binds on reconnect"
    );
}

// Suppress unused-import warnings in the (never-compiled) scaffold; DELIVER's
// bodies use every symbol. `ListenPort` is the declared-port newtype the plan
// builder consumes.
#[allow(dead_code)]
fn _vocabulary(_p: ListenPort) {}

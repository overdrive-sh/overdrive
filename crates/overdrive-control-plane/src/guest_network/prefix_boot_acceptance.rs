//! P4 — boot convergence order and `probe_startup` refusals (seeded sim,
//! source-local).
//!
//! netns-density-295 (`docs/feature/netns-density-295/feature-delta.md`
//! § "Composition — production entry points" boot steps 1 and 9, the
//! D8a-ROUTE / D8a-LOAD-SWAP ordering invariant; § "Owner and provisioner"
//! `probe_startup` / `converge_shared`; § DG-3; Gate G-V1; ADR-0152, 0171,
//! 0175, 0178). Scenarios: `distill/test-scenarios-p4-steering-intake-boot.md`
//! P4-S12, P4-S14, P4-S15; and `test-scenarios-p5-egress-unframe-hostinternal.md`
//! P5-S1 (the seeded boot-refusal leg) and P5-S2 (the `probe_startup` leg). Quint oracle for P4-S12:
//! `specs/quint/guest-flow-owner/prefix_boot.qnt` (replayed by
//! `prefix_boot_conformance`).
//!
//! Source-local because the owner's effect seam (`GuestFlowIo`) and its sim
//! are crate-private (QPROJ-1). Every test drives the production owner
//! `HostSharedGuestNetworkOwner::with_flow_io(ports, SimGuestFlowIo)` through
//! its driving ports (`probe_startup`, `converge_shared`) and asserts only
//! port-observable state: the returned `GuestNetworkError`, the sim
//! arbiter's node-global steering observation (link, program, route,
//! entries), the steering effects the owner issued, and the ordered effect
//! kinds it issued. Default lane (in-process sim; no real infrastructure).
//!
//! State-delta universe (P4-S12/S14/S15): `{boot outcome, steering link,
//! link program, prefix route, steering entries, effects issued}`; a refusal
//! at `probe_startup` leaves every element unchanged and issues no effect
//! beyond the reads and the forwarder's scratch probe.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::doc_markdown,
    reason = "test fixtures assert their preconditions; the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::sync::Arc;

use ipnet::Ipv4Net;
use overdrive_core::guest_transport::{
    GUEST_INTAKE_HOST_PORT_BASE, GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX, GUEST_NODE_GATEWAY,
    GUEST_NODE_PREFIX,
};
use overdrive_core::net::InterfaceIndex;
use overdrive_core::traits::mtls_resolve::MtlsResolution;
use overdrive_dataplane::allocators::VipRange;
use overdrive_dataplane::guest_flow::UnframeAttachError;
use overdrive_dataplane::guest_steering::GuestSteeringError;
use overdrive_sim::adapters::SimMtlsResolve;
use overdrive_sim::adapters::clock::SimClock;
use proptest::prelude::*;

use super::flow_io::sim::{
    SimEffectKind, SimFailure, SimGuestFlowIo, SimGuestNode, SimPrefixRoute, SimProgramVersion,
    SimSteeringEffect, SimSteeringFailure, SimSteeringLink, SimSteeringObservation,
    SimUnframeFailure,
};
use super::flow_io::{GuestFlowIo, InterfaceFacts};
use super::{
    GuestFlowOwnerPorts, GuestNetworkError, GuestNetworkOperation, HostSharedGuestNetworkOwner,
    IntakePortBlockConflict, InterfaceStacking, SharedGuestNetworkOwner, StackedLinkKind,
};

// ---------------------------------------------------------------------------
// Fixture: the production owner over the seam's sim
// ---------------------------------------------------------------------------

/// The owner's ports with the configured service VIP range `vip` and the
/// wildcard TCP ports `serve` binds; everything else as in production
/// (`GUEST_NODE_PREFIX` / `GUEST_NODE_GATEWAY`, DG-14), the time and
/// resolution ports simulated.
pub(super) fn owner_ports(vip: &[Ipv4Net], wildcard: &[u16]) -> GuestFlowOwnerPorts {
    GuestFlowOwnerPorts {
        clock: Arc::new(SimClock::new()),
        resolve: Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh)),
        mtls: crate::shared_network_test_ports::mtls_worker(),
        vip_range: VipRange::new(vip.to_vec(), BTreeSet::new()).expect("valid VIP range"),
        wildcard_tcp_ports: wildcard.iter().copied().collect(),
        node_prefix: GUEST_NODE_PREFIX,
        gateway: GUEST_NODE_GATEWAY,
    }
}

/// A VIP range disjoint from the guest prefix (the production default shape).
pub(super) fn disjoint_vip() -> Vec<Ipv4Net> {
    vec![net("10.96.0.0/16")]
}

fn net(cidr: &str) -> Ipv4Net {
    cidr.parse().expect("valid CIDR")
}

/// One `serve` process: the production owner over a seam of `node` whose
/// binary carries steering program `binary`.
struct Serve {
    io: Arc<SimGuestFlowIo>,
    owner: Arc<HostSharedGuestNetworkOwner>,
}

impl Serve {
    fn start(node: &Arc<SimGuestNode>, binary: &str, ports: GuestFlowOwnerPorts) -> Self {
        let io = SimGuestFlowIo::new(node, SimProgramVersion::new(binary));
        let seam: Arc<dyn GuestFlowIo> = Arc::clone(&io) as Arc<dyn GuestFlowIo>;
        let owner = Arc::new(HostSharedGuestNetworkOwner::with_flow_io(ports, seam));
        Self { io, owner }
    }

    /// Runs `converge_shared` to completion with every effect released, the
    /// `failure` (if any) scripted on the first effect of its kind.
    async fn converge(
        &self,
        failure: Option<(SimEffectKind, SimFailure)>,
    ) -> Result<(), GuestNetworkError> {
        self.io.auto_release_all();
        if let Some((kind, failure)) = failure {
            self.io.script_failure(kind, failure);
        }
        self.owner.converge_shared().await
    }

    /// `serve` ends (refusal, crash or shutdown): the process's objects go.
    fn exit(self) {
        drop(self.owner);
        self.io.exit_process();
    }
}

/// The issued effect kinds other than reads and the forwarder's scratch
/// probe — the boot effects.
fn boot_effects(io: &SimGuestFlowIo) -> Vec<SimEffectKind> {
    io.issued()
        .into_iter()
        .filter(|kind| {
            !matches!(
                kind,
                SimEffectKind::HostFacts
                    | SimEffectKind::StaleObjects
                    | SimEffectKind::ProbeForwarder
            )
        })
        .collect()
}

fn never_booted() -> SimSteeringObservation {
    SimSteeringObservation {
        link: SimSteeringLink::None,
        program: None,
        entries: BTreeMap::new(),
        route: SimPrefixRoute::Absent,
    }
}

fn converged(binary: &str) -> SimSteeringObservation {
    SimSteeringObservation {
        link: SimSteeringLink::Pinned,
        program: Some(SimProgramVersion::new(binary)),
        entries: BTreeMap::new(),
        route: SimPrefixRoute::Local,
    }
}

// ---------------------------------------------------------------------------
// P4-S12 / R5-24, G-V1, prefix_boot inv 6 — boot order and crash between steps
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 13, steering load and swap, shared local route)"]
async fn boot_runs_load_then_swap_then_route_and_a_verifier_reject_is_never_attached() {
    // A first boot on a node never booted: load → swap (first attach, pin) → route.
    let node = SimGuestNode::new();
    let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
    assert_eq!(serve.io.steering_observation(), never_booted());

    serve.converge(None).await.expect("boot converges");

    assert_eq!(
        serve.io.steering_effects(),
        vec![
            SimSteeringEffect::Load { version: SimProgramVersion::new("v1"), ok: true },
            SimSteeringEffect::Attach { adopted: false, ok: true },
            SimSteeringEffect::Pin { ok: true },
            SimSteeringEffect::Route { ok: true },
        ],
        "D8a-LOAD-SWAP: load, then swap, then the route",
    );
    assert_eq!(serve.io.steering_observation(), converged("v1"));
    serve.exit();

    // The kernel verifier rejects this binary's program on a node never booted.
    let node = SimGuestNode::new();
    let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
    let refused = serve
        .converge(Some((
            SimEffectKind::SteeringLoad,
            SimFailure::Steering(SimSteeringFailure::Load),
        )))
        .await
        .expect_err("a verifier rejection refuses startup");

    assert!(
        matches!(
            refused,
            GuestNetworkError::GuestPrefixSteeringConverge {
                source: GuestSteeringError::Load { .. }
            }
        ),
        "GuestPrefixSteeringConverge {{ Load }}, got {refused:?}",
    );
    assert_eq!(
        serve.io.steering_effects(),
        vec![SimSteeringEffect::Load { version: SimProgramVersion::new("v1"), ok: false }],
        "nothing after the failed load: no attach, no route write",
    );
    assert_eq!(serve.io.steering_observation(), never_booted(), "nothing attached, no route");
    serve.exit();
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 13, steering load and swap, shared local route)"]
async fn a_failed_or_interrupted_boot_step_leaves_the_earlier_program_and_route_local_only_over_a_pinned_link()
 {
    // An earlier boot of binary v1 converged the node, then serve stopped.
    let node = SimGuestNode::new();
    let earlier = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
    earlier.converge(None).await.expect("earlier boot converges");
    earlier.exit();
    let before = SimGuestFlowIo::new(&node, SimProgramVersion::new("v2")).steering_observation();
    assert_eq!(before, converged("v1"), "U-4: route and pinned steering outlive serve");

    // This boot (binary v2) fails the swap of the adopted link.
    let serve = Serve::start(&node, "v2", owner_ports(&disjoint_vip(), &[]));
    let refused = serve
        .converge(Some((
            SimEffectKind::SteeringAttach,
            SimFailure::Steering(SimSteeringFailure::Update),
        )))
        .await
        .expect_err("a failed swap refuses startup");
    assert!(
        matches!(
            refused,
            GuestNetworkError::GuestPrefixSteeringConverge {
                source: GuestSteeringError::Update { .. }
            }
        ),
        "GuestPrefixSteeringConverge {{ Update }}, got {refused:?}",
    );
    assert_eq!(
        serve.io.steering_effects(),
        vec![
            SimSteeringEffect::Load { version: SimProgramVersion::new("v2"), ok: true },
            SimSteeringEffect::Attach { adopted: true, ok: false },
        ],
        "an adopted link is updated, never detached or re-pinned; no route write after the failure",
    );
    assert_eq!(
        serve.io.steering_observation(),
        before,
        "K-L9: the earlier program, the route unchanged"
    );
    serve.exit();

    // A crash between a first attach and its pin, on a node never booted.
    let node = SimGuestNode::new();
    let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
    serve.io.auto_release_all();
    serve.io.hold(SimEffectKind::SteeringPin);
    let owner = Arc::clone(&serve.owner);
    let boot = tokio::spawn(async move { owner.converge_shared().await });
    serve.io.settled().await;
    assert!(serve.io.held_of(SimEffectKind::SteeringPin).is_some(), "parked on the pin");
    assert_eq!(serve.io.steering_observation().link, SimSteeringLink::Attached);
    boot.abort();
    let _crashed = boot.await;
    serve.exit();
    let after_crash =
        SimGuestFlowIo::new(&node, SimProgramVersion::new("v1")).steering_observation();
    assert_eq!(after_crash, never_booted(), "M-4: the unpinned link died; no route was written");
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 13, steering load and swap, shared local route)"]
async fn a_failed_route_write_refuses_startup_with_the_route_as_it_was() {
    let node = SimGuestNode::new();
    let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
    let refused = serve
        .converge(Some((SimEffectKind::PrefixRoute, SimFailure::Netlink)))
        .await
        .expect_err("a failed route write refuses startup");
    assert!(
        matches!(
            refused,
            GuestNetworkError::Netlink { operation: GuestNetworkOperation::LocalRouteConverge, .. }
        ),
        "Netlink {{ LocalRouteConverge }}, got {refused:?}",
    );
    let after = serve.io.steering_observation();
    assert_eq!(after.route, SimPrefixRoute::Absent, "the route as it was");
    assert_eq!(
        (after.link, after.program),
        (SimSteeringLink::Pinned, Some(SimProgramVersion::new("v1"))),
        "the route is absent over a pinned link: never local without steering",
    );
    serve.exit();
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 13, steering load and swap, shared local route)"]
async fn a_boot_crashed_at_any_steering_step_is_converged_by_the_next_boot() {
    for crash_at in [
        SimEffectKind::SteeringLoad,
        SimEffectKind::SteeringAttach,
        SimEffectKind::SteeringPin,
        SimEffectKind::PrefixRoute,
    ] {
        let node = SimGuestNode::new();
        let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
        serve.io.auto_release_all();
        serve.io.hold(crash_at);
        let owner = Arc::clone(&serve.owner);
        let boot = tokio::spawn(async move { owner.converge_shared().await });
        serve.io.settled().await;
        assert!(serve.io.held_of(crash_at).is_some(), "boot parked at {crash_at:?}");
        boot.abort();
        let _crashed = boot.await;
        serve.exit();

        let next = Serve::start(&node, "v2", owner_ports(&disjoint_vip(), &[]));
        next.converge(None)
            .await
            .unwrap_or_else(|e| panic!("next boot after a crash at {crash_at:?}: {e}"));
        assert_eq!(
            next.io.steering_observation(),
            converged("v2"),
            "the next boot converges after a crash at {crash_at:?}",
        );
        next.exit();
    }
}

// ---------------------------------------------------------------------------
// P4-S14 / R5-40, D8-PREFIX-VIP — VIP range overlap refused before any effect
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 13, probe_startup guest-prefix checks, D8-PREFIX-VIP)"]
async fn a_vip_range_overlapping_the_prefix_refuses_before_any_boot_effect() {
    for overlapping in ["100.95.255.0/24", "100.94.0.0/15"] {
        let node = SimGuestNode::new();
        let serve = Serve::start(&node, "v1", owner_ports(&[net(overlapping)], &[]));
        serve.io.auto_release_all();
        let refused = serve.owner.probe_startup().await.expect_err("probe_startup refuses");
        assert!(
            matches!(
                refused,
                GuestNetworkError::GuestPrefixOverlapsVipRange { prefix, vip_range }
                    if prefix == GUEST_NODE_PREFIX && vip_range == net(overlapping)
            ),
            "names the VIP range {overlapping}, got {refused:?}",
        );
        assert!(
            boot_effects(&serve.io).is_empty(),
            "no boot effect before the refusal ({overlapping})"
        );
        assert_eq!(serve.io.steering_observation(), never_booted());
        serve.exit();
    }
    for disjoint in ["100.96.0.0/16", "10.96.0.0/16"] {
        let node = SimGuestNode::new();
        let serve = Serve::start(&node, "v1", owner_ports(&[net(disjoint)], &[]));
        serve.io.auto_release_all();
        serve.owner.probe_startup().await.unwrap_or_else(|e| panic!("{disjoint} boots: {e}"));
        assert!(boot_effects(&serve.io).is_empty(), "probe_startup changes nothing");
        serve.exit();
    }
}

fn arb_v4net() -> impl Strategy<Value = Ipv4Net> {
    (any::<u32>(), 8_u8..=30).prop_map(|(bits, len)| {
        Ipv4Net::new(Ipv4Addr::from(bits), len).expect("prefix length in range").trunc()
    })
}

fn overlaps(a: Ipv4Net, b: Ipv4Net) -> bool {
    a.contains(&b.network()) || b.contains(&a.network())
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 13, probe_startup guest-prefix checks, D8-PREFIX-VIP)"]
fn probe_startup_refuses_a_vip_range_iff_one_of_its_ranges_meets_the_guest_prefix() {
    proptest!(|(ranges in proptest::collection::vec(arb_v4net(), 1..5))| {
        prop_assume!(VipRange::new(ranges.clone(), BTreeSet::new()).is_ok());
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        let node = SimGuestNode::new();
        let serve = Serve::start(&node, "v1", owner_ports(&ranges, &[]));
        serve.io.auto_release_all();
        let outcome = runtime.block_on(serve.owner.probe_startup());
        let expect_refusal = ranges.iter().any(|range| overlaps(*range, GUEST_NODE_PREFIX));
        prop_assert_eq!(
            matches!(outcome, Err(GuestNetworkError::GuestPrefixOverlapsVipRange { .. })),
            expect_refusal,
        );
        prop_assert!(boot_effects(&serve.io).is_empty());
        serve.exit();
    });
}

// ---------------------------------------------------------------------------
// P4-S15 / R5-42, R5-44, INTAKE-BIND — intake host-port block conflicts
// ---------------------------------------------------------------------------

const BLOCK_FIRST: u16 = GUEST_INTAKE_HOST_PORT_BASE;
const BLOCK_LAST: u16 = GUEST_INTAKE_HOST_PORT_BASE + GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX - 1;

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 9, intake host ports, INTAKE-BIND)"]
async fn the_intake_block_conflicting_with_the_ephemeral_range_or_a_wildcard_listener_refuses() {
    // ip_local_port_range reaching into the block (20000 60999).
    let node = SimGuestNode::new();
    node.update_host_facts(|facts| facts.ephemeral_ports = 20_000..=60_999);
    let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
    serve.io.auto_release_all();
    let refused = serve.owner.probe_startup().await.expect_err("refuses EphemeralRange");
    assert!(
        matches!(
            refused,
            GuestNetworkError::IntakePortBlockConflict {
                first: BLOCK_FIRST,
                last: BLOCK_LAST,
                conflict: IntakePortBlockConflict::EphemeralRange { low: 20_000, high: 60_999 },
            }
        ),
        "EphemeralRange before any boot effect, got {refused:?}",
    );
    assert!(boot_effects(&serve.io).is_empty(), "no boot effect before the refusal");
    serve.exit();

    // config.bind = 0.0.0.0:21000, a port of the block.
    let node = SimGuestNode::new();
    let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[8_443, 21_000]));
    serve.io.auto_release_all();
    let refused = serve.owner.probe_startup().await.expect_err("refuses HostListener");
    assert!(
        matches!(
            refused,
            GuestNetworkError::IntakePortBlockConflict {
                first: BLOCK_FIRST,
                last: BLOCK_LAST,
                conflict: IntakePortBlockConflict::HostListener { port: 21_000 },
            }
        ),
        "HostListener {{ port: 21000 }}, got {refused:?}",
    );
    assert!(boot_effects(&serve.io).is_empty(), "no boot effect before the refusal");
    serve.exit();

    // The default ephemeral range and a wildcard port outside the block boot.
    let node = SimGuestNode::new();
    let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[8_443]));
    serve.io.auto_release_all();
    serve.owner.probe_startup().await.expect("no conflict: probe_startup passes");
    serve.exit();
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 9, intake host ports, INTAKE-BIND)"]
fn probe_startup_refuses_iff_the_ephemeral_range_or_a_wildcard_port_meets_the_intake_block() {
    proptest!(|(low in 1_024_u16..=65_000, span in 0_u16..=40_000, wildcard in proptest::collection::btree_set(1_u16..=65_535, 0..4))| {
        let high = low.saturating_add(span);
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        let node = SimGuestNode::new();
        node.update_host_facts(|facts| facts.ephemeral_ports = low..=high);
        let ports: Vec<u16> = wildcard.iter().copied().collect();
        let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &ports));
        serve.io.auto_release_all();
        let outcome = runtime.block_on(serve.owner.probe_startup());
        let meets = (low <= BLOCK_LAST && high >= BLOCK_FIRST)
            || wildcard.iter().any(|port| (BLOCK_FIRST..=BLOCK_LAST).contains(port));
        prop_assert_eq!(
            matches!(outcome, Err(GuestNetworkError::IntakePortBlockConflict { .. })),
            meets,
        );
        prop_assert!(boot_effects(&serve.io).is_empty());
        serve.exit();
    });
}

// ---------------------------------------------------------------------------
// Owner placement (D24) and the fd budget — probe_startup refusals
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 11, owner cgroup placement under the connect4 attach point, D24)"]
async fn an_owner_outside_the_connect4_attach_path_refuses_before_any_boot_effect() {
    let node = SimGuestNode::new();
    node.update_host_facts(|facts| facts.owner_cgroup = "/sys/fs/cgroup/system.slice".into());
    let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
    serve.io.auto_release_all();
    let refused = serve.owner.probe_startup().await.expect_err("refuses OwnerCgroupPlacement");
    assert!(
        matches!(
            &refused,
            GuestNetworkError::OwnerCgroupPlacement { owner, .. }
                if owner.as_path() == std::path::Path::new("/sys/fs/cgroup/system.slice")
        ),
        "OwnerCgroupPlacement naming the owner's cgroup, got {refused:?}",
    );
    assert!(boot_effects(&serve.io).is_empty(), "no boot effect before the refusal");
    serve.exit();
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 7, owner flows: fd budget in probe_startup)"]
async fn an_rlimit_nofile_below_the_owner_budget_refuses_before_any_boot_effect() {
    let node = SimGuestNode::new();
    node.update_host_facts(|facts| facts.nofile_limit = 1_024);
    let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
    serve.io.auto_release_all();
    let refused = serve.owner.probe_startup().await.expect_err("refuses FdBudget");
    assert!(
        matches!(refused, GuestNetworkError::FdBudget { required, available: 1_024 } if required > 1_024),
        "FdBudget naming RLIMIT_NOFILE, got {refused:?}",
    );
    assert!(boot_effects(&serve.io).is_empty(), "no boot effect before the refusal");
    serve.exit();
}

// ---------------------------------------------------------------------------
// Node-interface fixtures (`InterfaceIndex`, DG-11)
// ---------------------------------------------------------------------------

fn ifindex(raw: u32) -> InterfaceIndex {
    InterfaceIndex::new(raw).expect("a non-zero interface index")
}

/// An independent root-namespace interface carrying `addresses`.
fn independent(addresses: &[&str]) -> InterfaceFacts {
    InterfaceFacts { addresses: addresses.iter().map(|cidr| net(cidr)).collect(), stacking: None }
}

/// The node's root-namespace interfaces, by index.
fn set_interfaces(node: &SimGuestNode, interfaces: Vec<(InterfaceIndex, InterfaceFacts)>) {
    node.update_host_facts(|facts| facts.interfaces = interfaces.into_iter().collect());
}

// ---------------------------------------------------------------------------
// P4-S14 / R5-11, D8-PREFIX-CFG — a node address inside the prefix refuses
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 13, probe_startup guest-prefix checks, D8-PREFIX-CFG)"]
async fn a_node_address_inside_the_prefix_refuses_before_any_boot_effect() {
    let eth0 = ifindex(2);
    let eth1 = ifindex(3);
    for inside in ["100.95.7.1/24", "100.95.255.254/32", "100.95.0.2/16"] {
        let node = SimGuestNode::new();
        set_interfaces(
            &node,
            vec![(eth0, independent(&["10.0.2.15/24"])), (eth1, independent(&[inside]))],
        );
        let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
        serve.io.auto_release_all();
        let before = serve.io.steering_observation();
        assert_eq!(before, never_booted(), "a node never booted");

        let refused = serve.owner.probe_startup().await.expect_err("probe_startup refuses");

        assert!(
            matches!(
                refused,
                GuestNetworkError::GuestPrefixOverlapsNodeAddress { prefix, interface, address }
                    if prefix == GUEST_NODE_PREFIX && interface == eth1 && address == net(inside)
            ),
            "names the interface and its address {inside}, got {refused:?}",
        );
        assert!(
            boot_effects(&serve.io).is_empty(),
            "no boot effect before the refusal ({inside}): no forwarder, listener, steering or route",
        );
        assert_eq!(
            serve.io.steering_observation(),
            before,
            "steering link, program, entries and route unchanged ({inside})",
        );
        serve.exit();
    }
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 13, probe_startup guest-prefix checks, D8-PREFIX-CFG)"]
async fn a_node_that_overlaps_the_prefix_by_address_and_by_vip_range_names_the_vip_range() {
    let node = SimGuestNode::new();
    set_interfaces(&node, vec![(ifindex(2), independent(&["100.95.7.1/24"]))]);
    let serve = Serve::start(&node, "v1", owner_ports(&[net("100.95.128.0/17")], &[]));
    serve.io.auto_release_all();

    let refused = serve.owner.probe_startup().await.expect_err("probe_startup refuses");

    assert!(
        matches!(
            refused,
            GuestNetworkError::GuestPrefixOverlapsVipRange { prefix, vip_range }
                if prefix == GUEST_NODE_PREFIX && vip_range == net("100.95.128.0/17")
        ),
        "D8-PREFIX-VIP is checked first: the VIP range is named, got {refused:?}",
    );
    assert!(boot_effects(&serve.io).is_empty(), "no boot effect before the refusal");
    assert_eq!(serve.io.steering_observation(), never_booted(), "steering and route unchanged");
    serve.exit();
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 13, probe_startup guest-prefix checks, D8-PREFIX-CFG)"]
async fn node_addresses_outside_the_prefix_pass_probe_startup_without_effect() {
    // Adjacent ranges on both sides and an unrelated private address.
    let node = SimGuestNode::new();
    set_interfaces(
        &node,
        vec![
            (ifindex(2), independent(&["10.0.2.15/24"])),
            (ifindex(3), independent(&["100.94.255.254/16", "100.96.0.1/16"])),
        ],
    );
    let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
    serve.io.auto_release_all();

    serve.owner.probe_startup().await.expect("no node address in the prefix: probe_startup passes");

    assert!(boot_effects(&serve.io).is_empty(), "probe_startup changes nothing");
    assert_eq!(serve.io.steering_observation(), never_booted(), "steering and route unchanged");
    serve.exit();
}

/// An interface address either inside the guest prefix (its address in
/// `100.95.0.0/16`, prefix length 16 or longer) or wholly disjoint from it —
/// never a shorter prefix that merely covers it.
fn arb_node_address() -> impl Strategy<Value = Ipv4Net> {
    prop_oneof![
        (any::<u16>(), 16_u8..=32).prop_map(|(host, len)| {
            let [hi, lo] = host.to_be_bytes();
            Ipv4Net::new(Ipv4Addr::new(100, 95, hi, lo), len).expect("prefix length in range")
        }),
        arb_v4net().prop_filter("disjoint from the guest prefix", |candidate| {
            !overlaps(*candidate, GUEST_NODE_PREFIX)
        }),
    ]
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 13, probe_startup guest-prefix checks, D8-PREFIX-CFG)"]
fn probe_startup_refuses_a_node_address_iff_one_lies_inside_the_guest_prefix() {
    proptest!(|(inventory in proptest::collection::vec(proptest::collection::btree_set(arb_node_address(), 0..3), 1..4))| {
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        let node = SimGuestNode::new();
        let interfaces: Vec<(InterfaceIndex, InterfaceFacts)> = inventory
            .iter()
            .zip(2_u32..)
            .map(|(addresses, raw)| {
                (ifindex(raw), InterfaceFacts { addresses: addresses.clone(), stacking: None })
            })
            .collect();
        set_interfaces(&node, interfaces);
        let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
        serve.io.auto_release_all();
        let outcome = runtime.block_on(serve.owner.probe_startup());
        let inside = inventory
            .iter()
            .flatten()
            .any(|address| GUEST_NODE_PREFIX.contains(&address.addr()));
        prop_assert_eq!(
            matches!(outcome, Err(GuestNetworkError::GuestPrefixOverlapsNodeAddress { .. })),
            inside,
        );
        prop_assert!(boot_effects(&serve.io).is_empty());
        prop_assert_eq!(serve.io.steering_observation(), never_booted());
        serve.exit();
    });
}

// ---------------------------------------------------------------------------
// P5-S2 / R5-38, IFACE-NO-STACK — a stacked root-namespace interface refuses
// ---------------------------------------------------------------------------

fn stacked(stacking: InterfaceStacking) -> InterfaceFacts {
    InterfaceFacts { addresses: BTreeSet::from([net("10.0.3.15/24")]), stacking: Some(stacking) }
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 4, probe_startup stacked-interface refusal, IFACE-NO-STACK)"]
async fn serve_refuses_to_boot_on_a_stacked_root_namespace_interface() {
    let lower = ifindex(2);
    let upper = ifindex(5);
    let mut cases = vec![InterfaceStacking::Enslaved { master: ifindex(4) }];
    cases.extend(
        [
            StackedLinkKind::Vlan,
            StackedLinkKind::Macvlan,
            StackedLinkKind::Macvtap,
            StackedLinkKind::Ipvlan,
            StackedLinkKind::Ipvtap,
        ]
        .into_iter()
        .map(|kind| InterfaceStacking::Upper { kind, lower }),
    );
    for stacking in cases {
        let node = SimGuestNode::new();
        set_interfaces(
            &node,
            vec![(lower, independent(&["10.0.2.15/24"])), (upper, stacked(stacking))],
        );
        let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
        serve.io.auto_release_all();

        let refused = serve.owner.probe_startup().await.expect_err("probe_startup refuses");

        assert!(
            matches!(
                refused,
                GuestNetworkError::StackedInterface { interface, stacking: named }
                    if interface == upper && named == stacking
            ),
            "StackedInterface naming the interface and {stacking:?}, got {refused:?}",
        );
        if let InterfaceStacking::Upper { kind, .. } = stacking {
            assert!(
                refused.to_string().contains(kind.as_str()),
                "the operator-facing message names the link kind {:?}: {refused}",
                kind.as_str(),
            );
        }
        assert!(
            boot_effects(&serve.io).is_empty(),
            "no boot effect before the refusal ({stacking:?}): no attach, route or listener",
        );
        assert_eq!(
            serve.io.steering_observation(),
            never_booted(),
            "steering and route unchanged ({stacking:?})",
        );
        serve.exit();
    }

    // Independent interfaces boot (a veth peer is independent: no master, no
    // stacking link kind).
    let node = SimGuestNode::new();
    set_interfaces(
        &node,
        vec![(lower, independent(&["10.0.2.15/24"])), (upper, independent(&["10.0.3.15/24"]))],
    );
    let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
    serve.io.auto_release_all();
    serve.owner.probe_startup().await.expect("independent interfaces: probe_startup passes");
    assert!(boot_effects(&serve.io).is_empty(), "probe_startup changes nothing");
    serve.exit();
}

fn arb_stacking() -> impl Strategy<Value = Option<InterfaceStacking>> {
    let kind = prop_oneof![
        Just(StackedLinkKind::Vlan),
        Just(StackedLinkKind::Macvlan),
        Just(StackedLinkKind::Macvtap),
        Just(StackedLinkKind::Ipvlan),
        Just(StackedLinkKind::Ipvtap),
    ];
    prop_oneof![
        2 => Just(None),
        1 => (1_u32..=64).prop_map(|master| Some(InterfaceStacking::Enslaved { master: ifindex(master) })),
        1 => (kind, 1_u32..=64)
            .prop_map(|(kind, lower)| Some(InterfaceStacking::Upper { kind, lower: ifindex(lower) })),
    ]
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 4, probe_startup stacked-interface refusal, IFACE-NO-STACK)"]
fn probe_startup_refuses_iff_some_root_namespace_interface_is_stacked() {
    proptest!(|(inventory in proptest::collection::vec(arb_stacking(), 1..6))| {
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        let node = SimGuestNode::new();
        let interfaces: Vec<(InterfaceIndex, InterfaceFacts)> = inventory
            .iter()
            .zip(2_u32..)
            .map(|(stacking, raw)| {
                (
                    ifindex(raw),
                    InterfaceFacts { addresses: BTreeSet::from([net("10.0.2.15/24")]), stacking: *stacking },
                )
            })
            .collect();
        set_interfaces(&node, interfaces);
        let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
        serve.io.auto_release_all();
        let outcome = runtime.block_on(serve.owner.probe_startup());
        prop_assert_eq!(
            matches!(outcome, Err(GuestNetworkError::StackedInterface { .. })),
            inventory.iter().any(Option::is_some),
        );
        prop_assert!(boot_effects(&serve.io).is_empty());
        serve.exit();
    });
}

// ---------------------------------------------------------------------------
// P5-S1 (sad) / R5-38, D5a-IFACE-BOOT — a failed boot unframe attach refuses
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 4, host forwarder: egress unframe attach at boot, D5a-IFACE-BOOT)"]
async fn a_failed_boot_unframe_attach_refuses_startup_and_writes_nothing_else() {
    let eth0 = ifindex(2);
    let veth = ifindex(3);
    for (failure, expect_kernel_memory) in [
        (SimUnframeFailure::KernelMemory, true),
        (SimUnframeFailure::Attach(libc::ERANGE), false),
        (SimUnframeFailure::Attach(libc::EEXIST), false),
        (SimUnframeFailure::Attach(libc::ENODEV), false),
    ] {
        let node = SimGuestNode::new();
        set_interfaces(
            &node,
            vec![(eth0, independent(&["10.0.2.15/24"])), (veth, independent(&["10.0.3.1/30"]))],
        );
        let serve = Serve::start(&node, "v1", owner_ports(&disjoint_vip(), &[]));
        serve.io.script_unframe_failure(veth, failure);
        let before = serve.io.steering_observation();
        assert_eq!(before, never_booted(), "a node never booted");

        let refused = serve.converge(None).await.expect_err("a failed unframe attach refuses");

        let names_the_interface = match &refused {
            GuestNetworkError::UnframeAttach {
                source: UnframeAttachError::KernelMemory { ifindex, .. },
            } => expect_kernel_memory && *ifindex == veth,
            GuestNetworkError::UnframeAttach {
                source: UnframeAttachError::Attach { ifindex, .. },
            } => !expect_kernel_memory && *ifindex == veth,
            _ => false,
        };
        assert!(
            names_the_interface,
            "UnframeAttach classified from {failure:?} naming the failed interface, got {refused:?}",
        );
        assert_eq!(
            boot_effects(&serve.io),
            vec![SimEffectKind::LoadForwarder],
            "nothing after the forwarder load ({failure:?}): no listener bound, no steering, no route",
        );
        assert_eq!(
            serve.io.steering_effects(),
            Vec::<SimSteeringEffect>::new(),
            "no steering load, attach, pin or route write ({failure:?})",
        );
        assert_eq!(
            serve.io.steering_observation(),
            before,
            "steering link, program, entries and route unchanged ({failure:?})",
        );
        serve.exit();
    }
}

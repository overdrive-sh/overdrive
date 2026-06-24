//! S-PORTSET — the inbound-capture port-set equals the advertise port-set
//! (DISTILL RED scaffold, GH #241, Tier-1 DST, PROPERTY — DELIVER obligation #1).
//!
//! `@us-portset @property`. For an N>=2-listener Service, the inbound-rule
//! port-set (`project_service_listen_ports(intent)` ->
//! `AllocationSpec.service_ports`) MUST EQUAL the advertise port-set (the bridge
//! reading `desired.listeners` ports). Same intent source, two code paths ->
//! latent drift risk; the AC asserts BYTE-SET EQUALITY (DELIVER obligation #1).
//!
//! Mandate 8 (Universe): `projection.service_ports_set` +
//! `advertise.listener_ports_set` with the invariant `projection == advertise`.
//! Mandate 9: Tier-1 `@property` -> PBT FULL. The crafter generates an arbitrary
//! non-empty set of `NonZeroU16` listener ports (N >= 2) and asserts set equality
//! across both read paths — the canonical "property over a domain-rich input
//! space" case the `@property` tag signals.
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-PORTSET.
//!
//! ## Non-triviality (asymmetry caution, roadmap review note)
//!
//! The two read paths are structurally asymmetric: `project_service_listen_ports`
//! collects `svc.listeners` (a `Vec`) while the bridge iterates `desired.listeners`
//! as a `(ServiceId, ProjectedListener)` map, then projects those listeners onto the
//! emitted `Backend.addr`. The property is sound ONLY because both bottom out in the
//! same `svc.listeners` source. This test drives a genuine N>=2-listener single
//! Service through BOTH paths:
//!   - capture path: `project_service_listen_ports(&WorkloadIntent::Service(svc))`;
//!   - advertise path: the bridge `desired.listeners` is built from the SAME
//!     `svc.listeners` (mirroring the runtime's `hydrate_bridge_desired_listeners`
//!     per-listener loop), then observed through `reconcile`'s emitted
//!     `Backend.addr` ports — NOT read straight off `desired.listeners`, so a
//!     shape mismatch between the projection and the advertised backend set cannot
//!     be masked.

#![allow(clippy::expect_used)]

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::{NonZeroU16, NonZeroU32};
use std::time::{Duration, Instant};

use proptest::prelude::*;

use overdrive_core::aggregate::{Exec, Listener, ServiceV1, WorkloadDriver, WorkloadIntent};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, WorkloadId};
use overdrive_core::reconcilers::Action;
use overdrive_core::reconcilers::backend_discovery_bridge::{
    BackendDiscoveryBridge, BackendDiscoveryBridgeState, BackendDiscoveryBridgeView,
    ProjectedListener,
};
use overdrive_core::reconcilers::workload_lifecycle::project_service_listen_ports;
use overdrive_core::reconcilers::{Reconciler, TickContext};
use overdrive_core::traits::driver::Resources;
use overdrive_core::wall_clock::UnixInstant;

const HOST_IPV4: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 5);
const MESH_WORKLOAD_ADDR: Ipv4Addr = Ipv4Addr::new(10, 99, 0, 6);
const SERVICE_VIP_ADDR: Ipv4Addr = Ipv4Addr::new(10, 96, 0, 1);

fn workload_id() -> WorkloadId {
    WorkloadId::new("portset-svc").expect("'portset-svc' is a valid WorkloadId")
}

fn node_id() -> NodeId {
    NodeId::new("node-1").expect("'node-1' is a valid NodeId")
}

fn service_vip() -> ServiceVip {
    ServiceVip::new(IpAddr::V4(SERVICE_VIP_ADDR)).expect("ServiceVip accepts IPv4")
}

fn tick(counter: u64) -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(counter)),
        tick: counter,
        deadline: Instant::now() + Duration::from_secs(1),
    }
}

/// Build a `ServiceV1` carrying exactly `ports` as TCP listeners, in the given
/// order. This is the single intent source both read paths bottom out in.
fn service_with_ports(ports: &[NonZeroU16]) -> ServiceV1 {
    ServiceV1 {
        id: workload_id(),
        replicas: NonZeroU32::new(1).expect("1 is non-zero"),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: WorkloadDriver::Exec(Exec { command: "/bin/svc".to_string(), args: Vec::new() }),
        listeners: ports.iter().map(|p| Listener { port: *p, protocol: Proto::Tcp }).collect(),
        startup_probes: Vec::new(),
        readiness_probes: Vec::new(),
        liveness_probes: Vec::new(),
    }
}

/// CAPTURE path (01-02): the inbound-rule port-set the nft-TPROXY rule keys on.
fn capture_port_set(svc: &ServiceV1) -> BTreeSet<NonZeroU16> {
    let intent = WorkloadIntent::Service(svc.clone());
    project_service_listen_ports(&intent).into_iter().collect()
}

/// ADVERTISE path (the bridge): build `desired.listeners` from the SAME
/// `svc.listeners` (mirroring `hydrate_bridge_desired_listeners`), drive
/// `reconcile` with one `Some(workload_addr)` Running alloc, and harvest the
/// listener ports off every emitted `Backend.addr`.
fn advertise_port_set(svc: &ServiceV1) -> BTreeSet<NonZeroU16> {
    let vip = service_vip();
    let mut state = BackendDiscoveryBridgeState::empty_for_workload(workload_id());
    for listener in &svc.listeners {
        let service_id = ServiceId::derive(&vip, listener.port, listener.protocol, "service-map");
        state.desired.listeners.insert(
            service_id,
            ProjectedListener { vip, port: listener.port, protocol: listener.protocol },
        );
    }
    let alloc = AllocationId::new("alloc-portset").expect("alloc id valid");
    state.actual.running.insert(alloc, Some(MESH_WORKLOAD_ADDR));

    let bridge = BackendDiscoveryBridge::new(HOST_IPV4, node_id());
    let (actions, _) =
        bridge.reconcile(&state, &state, &BackendDiscoveryBridgeView::default(), &tick(1));

    actions
        .iter()
        .filter_map(|a| match a {
            Action::WriteServiceBackendRow { row, .. } => Some(row),
            _ => None,
        })
        .flat_map(|row| row.backends.iter())
        .map(|b| port_of(&b.addr))
        .collect()
}

const fn port_of(addr: &SocketAddr) -> NonZeroU16 {
    NonZeroU16::new(addr.port()).expect("listener port is non-zero by construction")
}

/// Strategy: an arbitrary non-empty set of N>=2 DISTINCT `NonZeroU16` ports.
/// Distinct because a Service's listeners are keyed per `(vip, port, proto)`
/// `ServiceId` — duplicate ports collapse to one entry in both paths and would
/// weaken the byte-set claim. N>=2 per the asymmetry caution: a single-listener
/// Service cannot expose a per-listener shape mismatch.
fn distinct_port_set() -> impl Strategy<Value = Vec<NonZeroU16>> {
    prop::collection::btree_set(1u16..=65535, 2..=12)
        .prop_map(|s| s.into_iter().map(|p| NonZeroU16::new(p).expect("non-zero")).collect())
}

proptest! {
    /// S-PORTSET @property — byte-set equality across the two read paths for
    /// every N>=2-listener Service. No captured port missing from the
    /// advertised set; no advertised port missing from the captured set.
    #[test]
    fn every_captured_port_is_an_advertised_port_for_a_multi_listener_service(
        ports in distinct_port_set(),
    ) {
        let svc = service_with_ports(&ports);
        let capture = capture_port_set(&svc);
        let advertise = advertise_port_set(&svc);

        prop_assert_eq!(
            &capture,
            &advertise,
            "capture port-set (project_service_listen_ports) MUST byte-equal the advertise \
             port-set (bridge-emitted Backend.addr ports): no captured port missing from \
             advertised, no advertised port missing from captured (DELIVER obligation #1)"
        );
    }
}

// --- @example pins (canonical mesh + host rows, preserved for the reviewer) --

/// `@example`-pinned canonical case: a two-listener mesh Service driven through
/// both paths with `Some(10.99.0.6)`. Pins the property at the named example the
/// spec calls out, so a future generator change cannot silently drop coverage of
/// the canonical shape.
#[test]
fn portset_equality_example_pin_canonical_mesh_two_listeners() {
    let ports =
        vec![NonZeroU16::new(8080).expect("non-zero"), NonZeroU16::new(8443).expect("non-zero")];
    let svc = service_with_ports(&ports);
    assert_eq!(
        capture_port_set(&svc),
        advertise_port_set(&svc),
        "canonical two-listener mesh Service: capture == advertise port-set"
    );
}

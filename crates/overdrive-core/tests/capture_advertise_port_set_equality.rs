//! S-PORTSET structural complement: the capture projection and authoritative
//! ServiceLifecycle publication expose the same port set for every generated
//! multi-listener Service. Both paths start at the same Service intent; the
//! advertisement assertion observes emitted Backend addresses, not its input
//! map. Real owner hydration/dispatch is covered compositionally by BE02.

#![allow(clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::{NonZeroU16, NonZeroU32};
use std::time::{Duration, Instant};

use proptest::prelude::*;

use overdrive_core::aggregate::{Exec, Listener, ServiceV2, WorkloadDriver, WorkloadIntent};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, WorkloadId};
use overdrive_core::reconcilers::Action;
use overdrive_core::reconcilers::{Reconciler, TickContext};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::service_lifecycle::{
    ServiceAllocFact, ServiceDataplaneIdentity, ServiceLifecycleReconciler, ServiceLifecycleState,
    ServiceLifecycleView,
};
use overdrive_reconcilers::workload_lifecycle::project_service_listen_ports;

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

/// Build a `ServiceV2` carrying exactly `ports` as TCP listeners, in the given
/// order. This is the single intent source both read paths bottom out in.
fn service_with_ports(ports: &[NonZeroU16]) -> ServiceV2 {
    ServiceV2 {
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
fn capture_port_set(svc: &ServiceV2) -> BTreeSet<NonZeroU16> {
    let intent = WorkloadIntent::Service(svc.clone());
    project_service_listen_ports(&intent).into_iter().collect()
}

/// ADVERTISE path: structural reconcile input for the same intent's listeners.
/// Canonical allocation-IP hydration has its separate retained boundary test.
fn advertise_port_set(svc: &ServiceV2) -> BTreeSet<NonZeroU16> {
    let vip = service_vip();
    let mut state = ServiceLifecycleState::default();
    for listener in &svc.listeners {
        let service_id = ServiceId::derive(&vip, listener.port, listener.protocol, "service-map");
        state.service_dataplane.insert(
            service_id,
            ServiceDataplaneIdentity {
                vip,
                port: listener.port,
                protocol: listener.protocol,
                writer: node_id(),
            },
        );
    }
    let alloc = AllocationId::new("alloc-portset").expect("alloc id valid");
    state.allocs = BTreeMap::from([(
        alloc.clone(),
        ServiceAllocFact {
            alloc_id: alloc.clone(),
            state: AllocState::Running,
            started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(0))),
            exit_code: None,
            latest_startup_probe: None,
            latest_startup_probe_observed_at: None,
            max_attempts: 30,
            startup_deadline: Duration::from_secs(60),
            mechanic_summary: String::new(),
            inferred: false,
            startup_probes_empty: true,
            latest_readiness_probe: None,
            has_readiness_probe: false,
            readiness_success_threshold: 1,
            backend_spiffe: overdrive_core::SpiffeId::for_allocation(&svc.id, &alloc),
            backend_ip: MESH_WORKLOAD_ADDR,
            latest_liveness_probe: None,
            has_liveness_probe: false,
            liveness_failure_threshold: 3,
        },
    )]);
    let (actions, _) = ServiceLifecycleReconciler::new().reconcile(
        &state,
        &state,
        &ServiceLifecycleView::default(),
        &tick(1),
    );
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
    /// CONTRACT_SHAPE: pure-function.
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
             port-set (ServiceLifecycle-emitted Backend.addr ports): no captured port missing from \
             advertised, no advertised port missing from captured (DELIVER obligation #1)"
        );
    }
}

// --- @example pins (canonical mesh + host rows, preserved for the reviewer) --

/// `@example`-pinned canonical case: a two-listener mesh Service driven through
/// both paths with `Some(10.99.0.6)`. Pins the property at the named example the
/// spec calls out, so a future generator change cannot silently drop coverage of
/// the canonical shape.
/// CONTRACT_SHAPE: pure-function.
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

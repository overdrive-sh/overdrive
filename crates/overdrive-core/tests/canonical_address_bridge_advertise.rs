//! S-BRIDGE — `BackendDiscoveryBridge` advertises the canonical workload address
//! when present, the host address otherwise (DISTILL RED scaffold, GH #241,
//! Tier-1 DST / reconciler-logic, default-lane).
//!
//! D-B2 / `@us-B2`. The driving port is `BackendDiscoveryBridge::reconcile`
//! (the function signature IS the port — port-to-port at the domain layer).
//! Observable outcomes on the returned `(Vec<Action>, View)`:
//!
//!   - a Running alloc with `Some(workload_addr)` -> advertised
//!     `Backend.addr == workload_addr:listener_port`;
//!   - with `None` -> `host_ipv4:listener_port` (fallback UNCHANGED);
//!   - `ServiceBackendRow.vip` UNCHANGED in BOTH arms (the dialable-VIP path is
//!     #61 territory, orthogonal).
//!
//! The `None`-fallback arm is the error/edge coverage (host-netns workload).
//!
//! Mandate 8 (Universe — port-exposed names only): the reconcile-returned
//! actions' `backend_addr` + `service_vip` + the `View`'s advertised
//! fingerprint; NEVER the bridge's private fields.
//! Mandate 9: Tier-1 in-memory acceptance. PBT-eligible over
//! `{Some(addr) | None} x listener_port`, but shipped here as `@example`-pinned
//! `#[test]` arms (the canonical mesh row + host row) — no proptest in this
//! file. The capture/advertise port-set property coverage lives in S-PORTSET
//! (`capture_advertise_port_set_equality.rs`).
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-BRIDGE.

#![allow(clippy::expect_used)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;
use std::time::{Duration, Instant};

use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, WorkloadId};
use overdrive_core::reconcilers::Action;
use overdrive_core::reconcilers::backend_discovery_bridge::{
    BackendDiscoveryBridge, BackendDiscoveryBridgeState, BackendDiscoveryBridgeView,
    ProjectedListener,
};
use overdrive_core::reconcilers::{Reconciler, TickContext};
use overdrive_core::wall_clock::UnixInstant;

// --- fixtures (port-level constructors only; no private-field reach) ---------

const HOST_IPV4: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 5);
/// Canonical Path-A mesh `workload_addr` — the `slot.base + 2` second-usable
/// address of a `/30` (pinned example, GH #241 / D-BLOCKER2).
const MESH_WORKLOAD_ADDR: Ipv4Addr = Ipv4Addr::new(10, 99, 0, 6);
const LISTENER_PORT: u16 = 8080;
/// VIP must be UNCHANGED across both arms — this is the dialable-VIP slot
/// (#61 territory), orthogonal to the backend addr-source flip.
const SERVICE_VIP_ADDR: Ipv4Addr = Ipv4Addr::new(10, 96, 0, 1);

fn workload_id() -> WorkloadId {
    WorkloadId::new("mesh-payments").expect("'mesh-payments' is a valid WorkloadId")
}

fn node_id() -> NodeId {
    NodeId::new("node-1").expect("'node-1' is a valid NodeId")
}

fn alloc_id() -> AllocationId {
    AllocationId::new("alloc-a").expect("alloc id is valid")
}

fn service_id() -> ServiceId {
    ServiceId::new(1).expect("ServiceId accepts any u64")
}

fn service_vip() -> ServiceVip {
    ServiceVip::new(IpAddr::V4(SERVICE_VIP_ADDR)).expect("ServiceVip accepts IPv4")
}

fn projected_listener() -> ProjectedListener {
    ProjectedListener {
        vip: service_vip(),
        port: NonZeroU16::new(LISTENER_PORT).expect("port must be non-zero"),
        protocol: Proto::Tcp,
    }
}

fn tick(counter: u64) -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(counter)),
        tick: counter,
        deadline: Instant::now() + Duration::from_secs(1),
    }
}

/// Build a single-listener / single-Running-alloc state where the alloc's
/// per-alloc `workload_addr` slot carries `workload_addr`. The map value is
/// the addr-source the bridge selects: `Some(addr)` -> advertise `addr:port`;
/// `None` -> fall back to `host_ipv4:port`.
fn state_with(workload_addr: Option<Ipv4Addr>) -> BackendDiscoveryBridgeState {
    let mut state = BackendDiscoveryBridgeState::empty_for_workload(workload_id());
    state.desired.listeners.insert(service_id(), projected_listener());
    state.actual.running.insert(alloc_id(), workload_addr);
    state
}

/// Extract the single emitted `ServiceBackendRow` from the reconcile output —
/// the port-exposed observable Universe (`backend.addr`, `row.vip`).
fn written_row(
    actions: &[Action],
) -> &overdrive_core::traits::observation_store::ServiceBackendRow {
    let Some(Action::WriteServiceBackendRow { row, .. }) =
        actions.iter().find(|a| matches!(a, Action::WriteServiceBackendRow { .. }))
    else {
        panic!("expected exactly one WriteServiceBackendRow action, got {actions:?}");
    };
    row
}

// --- S-BRIDGE Some arm ------------------------------------------------------

/// `@example`-pinned canonical mesh row: `Some(10.99.0.6)` -> the emitted
/// `Backend.addr` MUST be `10.99.0.6:8080` (the canonical workload address,
/// NOT `host_ipv4`); the emitted service vip is UNCHANGED.
#[test]
fn bridge_advertises_canonical_workload_address_when_present() {
    let bridge = BackendDiscoveryBridge::new(HOST_IPV4, node_id());
    let state = state_with(Some(MESH_WORKLOAD_ADDR));

    let (actions, _) =
        bridge.reconcile(&state, &state, &BackendDiscoveryBridgeView::default(), &tick(1));

    let row = written_row(&actions);
    assert_eq!(row.backends.len(), 1, "single Running alloc yields exactly one backend");
    assert_eq!(
        row.backends[0].addr,
        SocketAddr::new(IpAddr::V4(MESH_WORKLOAD_ADDR), LISTENER_PORT),
        "Some(workload_addr) MUST advertise workload_addr:listener_port (D-B2 canonical address), \
         NOT host_ipv4"
    );
    assert_eq!(
        row.vip, SERVICE_VIP_ADDR,
        "ServiceBackendRow.vip UNCHANGED in the Some arm (dialable-VIP is #61 territory)"
    );
}

// --- S-BRIDGE None arm (error/edge: host-netns / non-Path-A alloc) ----------

/// `@example`-pinned host row: `None` -> the emitted `Backend.addr` MUST be
/// `host_ipv4:8080` (fallback behaviour UNCHANGED); the emitted service vip is
/// UNCHANGED.
#[test]
fn bridge_falls_back_to_host_address_for_host_netns_workload() {
    let bridge = BackendDiscoveryBridge::new(HOST_IPV4, node_id());
    let state = state_with(None);

    let (actions, _) =
        bridge.reconcile(&state, &state, &BackendDiscoveryBridgeView::default(), &tick(1));

    let row = written_row(&actions);
    assert_eq!(row.backends.len(), 1, "single Running alloc yields exactly one backend");
    assert_eq!(
        row.backends[0].addr,
        SocketAddr::new(IpAddr::V4(HOST_IPV4), LISTENER_PORT),
        "None (host-netns / non-Path-A alloc) MUST fall back to host_ipv4:listener_port \
         (fallback UNCHANGED)"
    );
    assert_eq!(
        row.vip, SERVICE_VIP_ADDR,
        "ServiceBackendRow.vip UNCHANGED in the None arm (dialable-VIP is #61 territory)"
    );
}

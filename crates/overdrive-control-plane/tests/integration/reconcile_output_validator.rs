//! Reconcile-output invariant validator — integration coverage for
//! `action_shim::validate::validate_reconcile_output`.
//!
//! These tests assert the validator's observable contract through its
//! public API as a downstream-crate consumer (the dispatch boundary
//! caller in `reconciler_runtime::run_convergence_tick`). The detailed
//! per-conflict-shape assertions live inline in `validate.rs` as
//! `#[cfg(test)] mod tests`; this file mirrors the scenarios the
//! task contract pins:
//!
//! 1. Happy path — distinct writes, no conflicts.
//! 2. XDP-vs-XDP conflict — the same `(vip, port, proto)` slot.
//!
//! Per `.claude/rules/testing.md` § "What stays in the default lane":
//! pure-function validator, no real I/O, runs <60s. Integration-test
//! placement is per task spec — these would equally be unit tests on
//! the validator module.
//!
//! Phase 16 D11 — runtime defense for the inter-Action conflict gap.

use std::net::{IpAddr, Ipv4Addr};

use overdrive_control_plane::action_shim::validate::{
    ReconcilerOutputViolation, WriteRoute, validate_reconcile_output,
};
use overdrive_core::id::{ContentHash, CorrelationKey, ServiceId, ServiceVip};
use overdrive_core::reconcilers::Action;

use overdrive_testing::placement;

placement!(host_pure);

fn correlation(purpose: &str) -> CorrelationKey {
    let hash = ContentHash::of(purpose.as_bytes());
    CorrelationKey::derive("service-map-hydrator/1", &hash, purpose)
}

fn service_id() -> ServiceId {
    ServiceId::new(1).expect("ServiceId")
}

const fn vip(o1: u8) -> Ipv4Addr {
    Ipv4Addr::new(10, 96, 0, o1)
}

fn service_vip_for(o1: u8) -> ServiceVip {
    ServiceVip::new(IpAddr::V4(vip(o1))).expect("ServiceVip")
}

fn update_service(o1: u8) -> Action {
    update_service_proto_port(o1, overdrive_core::dataplane::backend_key::Proto::Tcp, 8080)
}

fn update_service_proto_port(
    o1: u8,
    proto: overdrive_core::dataplane::backend_key::Proto,
    port: u16,
) -> Action {
    Action::DataplaneUpdateService {
        service_id: service_id(),
        vip: service_vip_for(o1),
        port: std::num::NonZeroU16::new(port).expect("non-zero"),
        proto,
        backends: vec![],
        correlation: correlation("update-service"),
    }
}

/// Happy path — a mix of write- and non-write-actions over distinct
/// VIP keys passes. The validator only conflicts when two writes
/// land on the SAME VIP, not when a reconciler emits many actions
/// across distinct services in one tick.
#[test]
fn happy_path_distinct_vips_and_noops_pass() {
    let actions =
        vec![Action::Noop, update_service(1), update_service(2), update_service(3), Action::Noop];
    assert!(
        validate_reconcile_output(&actions).is_ok(),
        "distinct VIPs + noops must validate cleanly"
    );
}

/// S-02-01 — two XDP `DataplaneUpdateService` writes for the SAME VIP
/// but DIFFERENT ports (tcp/8080 + tcp/8081) now return `Ok`. Before
/// 02-01 the XDP write-key was VIP-only, so these falsely conflicted
/// as `ConflictingServiceWrites`. The map key is now `(vip, port, proto)`
/// — distinct ports are distinct slots.
#[test]
fn xdp_same_vip_different_ports_pass() {
    use overdrive_core::dataplane::backend_key::Proto;
    let actions = vec![
        update_service_proto_port(1, Proto::Tcp, 8080),
        update_service_proto_port(1, Proto::Tcp, 8081),
    ];
    assert!(
        validate_reconcile_output(&actions).is_ok(),
        "same VIP, different ports must NOT conflict (distinct (vip,port,proto) slots)"
    );
}

/// S-02-01 — the DNS co-location case: two XDP writes for the SAME
/// `(vip, port)` but DIFFERENT proto (tcp/53 + udp/53) return `Ok`.
/// Distinct proto → distinct outer-map slot → no conflict.
#[test]
fn xdp_same_vip_port_different_proto_pass() {
    use overdrive_core::dataplane::backend_key::Proto;
    let actions = vec![
        update_service_proto_port(1, Proto::Tcp, 53),
        update_service_proto_port(1, Proto::Udp, 53),
    ];
    assert!(
        validate_reconcile_output(&actions).is_ok(),
        "same (vip,port), different proto must NOT conflict (DNS co-location)"
    );
}

/// S-02-01 — genuine duplicate-slot collisions are STILL caught. Two
/// XDP writes for IDENTICAL `(vip, port, proto)` (tcp/8080 twice)
/// remain a `ConflictingServiceWrites` error.
#[test]
fn xdp_identical_vip_port_proto_rejected() {
    use overdrive_core::dataplane::backend_key::Proto;
    let actions = vec![
        update_service_proto_port(5, Proto::Tcp, 8080),
        update_service_proto_port(5, Proto::Tcp, 8080),
    ];
    let err = validate_reconcile_output(&actions)
        .expect_err("identical (vip,port,proto) XDP writes must conflict");
    let ReconcilerOutputViolation::ConflictingServiceWrites {
        service_id: conflict_sid,
        vip: conflict_vip,
        vip_port,
        proto: conflict_proto,
        first_route,
        second_route,
    } = err
    else {
        panic!("expected a service-write conflict, not an allocation-reclaim conflict");
    };
    assert_eq!(conflict_sid, service_id());
    assert_eq!(conflict_vip, vip(5));
    assert_eq!(conflict_proto, Proto::Tcp);
    assert_eq!(vip_port, Some(8080), "XDP-vs-XDP conflict now reports the shared port");
    assert_eq!(first_route, WriteRoute::Xdp);
    assert_eq!(second_route, WriteRoute::Xdp);
}

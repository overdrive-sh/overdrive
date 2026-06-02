//! Reconcile-output invariant validator — integration coverage for
//! `action_shim::validate::validate_reconcile_output`.
//!
//! These tests assert the validator's observable contract through its
//! public API as a downstream-crate consumer (the dispatch boundary
//! caller in `reconciler_runtime::run_convergence_tick`). The detailed
//! per-conflict-shape assertions live inline in `validate.rs` as
//! `#[cfg(test)] mod tests`; this file mirrors the three scenarios the
//! task contract pins:
//!
//! 1. Happy path — distinct writes, no conflicts.
//! 2. Cgroup-vs-XDP conflict — same VIP touched by both routes.
//! 3. Register-vs-Deregister conflict — same `(vip, port)` slot.
//!
//! Per `.claude/rules/testing.md` § "What stays in the default lane":
//! pure-function validator, no real I/O, runs <60s. Integration-test
//! placement is per task spec — these would equally be unit tests on
//! the validator module.
//!
//! Phase 16 D11 — runtime defense for the inter-Action conflict gap.

use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};

use overdrive_control_plane::action_shim::validate::{
    ReconcilerOutputViolation, WriteRoute, validate_reconcile_output,
};
use overdrive_core::id::{ContentHash, CorrelationKey, ServiceId, ServiceVip};
use overdrive_core::reconcilers::Action;

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

fn register(v: Ipv4Addr, port: u16) -> Action {
    Action::RegisterLocalBackend {
        service_id: service_id(),
        vip: v,
        vip_port: port,
        backend: SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 5), 9090),
        correlation: correlation("register-local-backend"),
    }
}

fn deregister(v: Ipv4Addr, port: u16) -> Action {
    Action::DeregisterLocalBackend {
        service_id: service_id(),
        vip: v,
        vip_port: port,
        correlation: correlation("deregister-local-backend"),
    }
}

fn update_service(o1: u8) -> Action {
    Action::DataplaneUpdateService {
        service_id: service_id(),
        vip: service_vip_for(o1),
        port: std::num::NonZeroU16::new(8080).expect("non-zero"),
        proto: overdrive_core::dataplane::backend_key::Proto::Tcp,
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
    let actions = vec![
        Action::Noop,
        update_service(1),
        register(vip(2), 8080),
        register(vip(2), 9090), // same VIP, distinct port — ok
        deregister(vip(3), 7000),
        Action::Noop,
    ];
    assert!(
        validate_reconcile_output(&actions).is_ok(),
        "distinct VIPs + noops must validate cleanly"
    );
}

/// Cgroup-vs-XDP conflict — the canonical defect class. A buggy
/// reconciler emits BOTH `Action::DataplaneUpdateService` (XDP path
/// — `SERVICE_MAP`) AND `Action::RegisterLocalBackend` (cgroup path
/// — `LOCAL_BACKEND_MAP`) targeting the SAME VIP in one tick. The
/// dataplane post-state becomes non-deterministic: a packet to the
/// VIP could traverse the XDP path or the cgroup path depending on
/// which map was written second. The validator must reject this.
#[test]
fn cgroup_vs_xdp_conflict_on_same_vip_rejected() {
    let actions = vec![update_service(1), register(vip(1), 8080)];
    let err = validate_reconcile_output(&actions)
        .expect_err("XDP + cgroup writes on same VIP must conflict");
    let ReconcilerOutputViolation::ConflictingServiceWrites {
        vip: conflict_vip,
        vip_port,
        first_route,
        second_route,
    } = err;
    assert_eq!(conflict_vip, vip(1));
    assert_eq!(vip_port, None, "cross-route conflict is per-VIP, not per-(vip, port)");
    assert_eq!(first_route, WriteRoute::Xdp);
    assert_eq!(second_route, WriteRoute::Cgroup);
}

/// Register-vs-Deregister conflict — two cgroup-path writes to the
/// same `(vip, vip_port)` slot in one tick are a bug regardless of
/// the route match. The kernel-side `LOCAL_BACKEND_MAP` entry is
/// overwritten by whichever action the dispatcher applies second;
/// the reconciler's intent is undefined. The validator must reject.
#[test]
fn register_vs_deregister_on_same_key_rejected() {
    let actions = vec![register(vip(7), 5000), deregister(vip(7), 5000)];
    let err = validate_reconcile_output(&actions)
        .expect_err("register+deregister at same (vip, port) must conflict");
    let ReconcilerOutputViolation::ConflictingServiceWrites {
        vip: conflict_vip,
        vip_port,
        first_route,
        second_route,
    } = err;
    assert_eq!(conflict_vip, vip(7));
    assert_eq!(vip_port, Some(5000));
    assert_eq!(first_route, WriteRoute::Cgroup);
    assert_eq!(second_route, WriteRoute::Cgroup);
}

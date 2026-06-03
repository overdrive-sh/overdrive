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
    register_proto(v, port, overdrive_core::dataplane::backend_key::Proto::Tcp)
}

fn register_proto(
    v: Ipv4Addr,
    port: u16,
    proto: overdrive_core::dataplane::backend_key::Proto,
) -> Action {
    Action::RegisterLocalBackend {
        service_id: service_id(),
        vip: v,
        vip_port: port,
        proto,
        backend: SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 5), 9090),
        correlation: correlation("register-local-backend"),
    }
}

fn deregister(v: Ipv4Addr, port: u16) -> Action {
    Action::DeregisterLocalBackend {
        service_id: service_id(),
        vip: v,
        vip_port: port,
        proto: overdrive_core::dataplane::backend_key::Proto::Tcp,
        correlation: correlation("deregister-local-backend"),
    }
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

/// ADR-0053 § 4 dual-path — an XDP `SERVICE_MAP` write AND a cgroup
/// `LOCAL_BACKEND_MAP` write for the SAME VIP in one tick is the
/// BLESSED mixed local+remote dual-path, NOT a conflict (RCA
/// `fix-mixed-backend-dispatch-spin` Fix A1; ADR-0053 revision
/// 2026-06-03). The two routes are disjoint kernel maps consumed by
/// different hooks with no precedence race. The validator MUST accept
/// it. (This integration-side test was flipped from a cross-route
/// rejection to an acceptance alongside the 01-04 `service_id`
/// threading — its unit-side sibling was flipped in step 01-01; the
/// integration sibling was missed there and is corrected here per
/// deletion/test discipline.)
#[test]
fn cgroup_and_xdp_on_same_vip_accepted() {
    let actions = vec![update_service(1), register(vip(1), 8080)];
    assert!(
        validate_reconcile_output(&actions).is_ok(),
        "XDP + cgroup writes on same VIP are the ADR-0053 § 4 dual-path, not a conflict"
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
    } = err;
    assert_eq!(conflict_sid, service_id());
    assert_eq!(conflict_vip, vip(5));
    assert_eq!(conflict_proto, Proto::Tcp);
    assert_eq!(vip_port, Some(8080), "XDP-vs-XDP conflict now reports the shared port");
    assert_eq!(first_route, WriteRoute::Xdp);
    assert_eq!(second_route, WriteRoute::Xdp);
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
        service_id: conflict_sid,
        vip: conflict_vip,
        vip_port,
        proto: conflict_proto,
        first_route,
        second_route,
    } = err;
    assert_eq!(conflict_sid, service_id());
    assert_eq!(conflict_vip, vip(7));
    assert_eq!(conflict_proto, overdrive_core::dataplane::backend_key::Proto::Tcp);
    assert_eq!(vip_port, Some(5000));
    assert_eq!(first_route, WriteRoute::Cgroup);
    assert_eq!(second_route, WriteRoute::Cgroup);
}

/// S-02-02 — same-host DNS unlock: two cgroup `RegisterLocalBackend`
/// writes for the SAME `(vip, port)` but DISTINCT proto (tcp/53 +
/// udp/53) now return `Ok`. Step 02-02 widened the cgroup write-key
/// from `(vip, port)` to `(vip, port, proto)` mirroring the
/// `LOCAL_BACKEND_MAP` key — distinct proto → distinct slot → no
/// conflict, the same co-location the XDP path already permits.
#[test]
fn cgroup_same_vip_port_different_proto_pass() {
    use overdrive_core::dataplane::backend_key::Proto;
    let actions =
        vec![register_proto(vip(8), 53, Proto::Tcp), register_proto(vip(8), 53, Proto::Udp)];
    assert!(
        validate_reconcile_output(&actions).is_ok(),
        "same (vip,port) different proto on the cgroup path must NOT conflict (DNS co-location)"
    );
}

/// S-02-02 — genuine duplicate cgroup slot is STILL caught. Two
/// `RegisterLocalBackend` for IDENTICAL `(vip, port, proto)` (tcp/53
/// twice) remain a `ConflictingServiceWrites` error reporting the
/// shared port.
#[test]
fn cgroup_identical_vip_port_proto_rejected() {
    use overdrive_core::dataplane::backend_key::Proto;
    let actions =
        vec![register_proto(vip(9), 53, Proto::Tcp), register_proto(vip(9), 53, Proto::Tcp)];
    let err = validate_reconcile_output(&actions)
        .expect_err("identical (vip,port,proto) cgroup writes must conflict");
    let ReconcilerOutputViolation::ConflictingServiceWrites {
        service_id: conflict_sid,
        vip: conflict_vip,
        vip_port,
        proto: conflict_proto,
        first_route,
        second_route,
    } = err;
    assert_eq!(conflict_sid, service_id());
    assert_eq!(conflict_vip, vip(9));
    assert_eq!(conflict_proto, Proto::Tcp);
    assert_eq!(vip_port, Some(53), "cgroup-vs-cgroup conflict reports the shared port");
    assert_eq!(first_route, WriteRoute::Cgroup);
    assert_eq!(second_route, WriteRoute::Cgroup);
}

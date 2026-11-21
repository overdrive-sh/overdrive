//! S-GATE — `ServiceMapHydrator` gates mesh-subnet backends out of BOTH
//! load-balancer path, leaving the remote arm unchanged (GH #241, Tier-1 DST /
//! reconciler-logic, default-lane).
//!
//! D-GATE / D-GATE-PRED / `@us-GATE`. The driving port is
//! `ServiceMapHydrator::reconcile`. A two-way split:
//!
//!   - `addr.ip() ∈ WORKLOAD_SUBNET_BASE (10.99.0.0/16)` -> excluded from the
//!     `DataplaneUpdateService` payload (mesh -> empty-remote purge settles the
//!     service; nft-TPROXY owns delivery);
//!   - otherwise -> `DataplaneUpdateService` (REMOTE arm).
//!
//! The remote arm is the error/edge coverage — it proves the gate does NOT
//! over-fire (a mutant gating everything, or gating nothing, fails here).
//!
//! The hydrator's `actual` is the fingerprint the dataplane carries per
//! service (`BTreeMap<ServiceId, BackendSetFingerprint>`, absent when nothing
//! is programmed). A service whose slot already carries a different set is the
//! precondition for a purge: the fixtures below seed `actual` with the
//! fingerprint of a previously programmed remote set. A service with an empty
//! survivor set and nothing programmed is already in place and emits nothing.
//!
//! Mandate 8 (Universe): the reconcile-returned actions'
//! `dataplane_update_service_count` + the `View`'s attempted fingerprint;
//! NEVER the hydrator's private partition state.
//! Mandate 9: Tier-1 -> PBT-eligible over the two address classes;
//! `@example`-pin a representative addr per arm (10.99.0.6 mesh / 10.96.0.50
//! remote).
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-GATE.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant};

use ipnet::Ipv4Net;
use proptest::prelude::*;

use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::dataplane::fingerprint::{BackendSetFingerprint, fingerprint};
use overdrive_core::id::{ServiceId, ServiceVip, SpiffeId};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::{
    ServiceDesired, ServiceMapHydrator, ServiceMapHydratorState, ServiceMapHydratorView,
};

use overdrive_testing_macros::overdrive_test_default;

overdrive_test_default!(host_pure);

/// The canonical Path-A/mesh workload subnet — the SAME `10.99.0.0/16`
/// `WORKLOAD_SUBNET_BASE` the provisioner carves per-allocation `/30`s
/// from (one source, D-GATE-PRED). Core constructs the literal because
/// `WORKLOAD_SUBNET_BASE` lives in the `overdrive-control-plane` wiring
/// crate, which core MUST NOT depend on.
fn workload_subnet() -> Ipv4Net {
    Ipv4Net::new(Ipv4Addr::new(10, 99, 0, 0), 16).expect("valid /16")
}

fn make_service_id(n: u64) -> ServiceId {
    ServiceId::new(n).expect("valid ServiceId")
}

fn make_tick(now_secs: u64) -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(now_secs)),
        tick: now_secs,
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

/// The fingerprint the dataplane carries for a service that was programmed
/// earlier with one remote backend (`10.96.0.99:8080`), a set that no fixture
/// in this file desires. Seeding `actual` with it means "the slot is
/// programmed with something else", so a purge or a reprogram is due.
fn previously_programmed(vip: &ServiceVip) -> BackendSetFingerprint {
    let earlier = Backend {
        workload_identity: SpiffeId::new("spiffe://overdrive.local/workload/web/alloc/web-0")
            .expect("valid SpiffeId"),
        addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 96, 0, 99)), 8080),
        weight: 1,
        healthy: true,
    };
    fingerprint(vip, &[earlier])
}

/// `actual` for one service carrying the previously programmed fingerprint.
fn actual_previously_programmed(
    s_id: ServiceId,
    desired: &ServiceDesired,
) -> BTreeMap<ServiceId, BackendSetFingerprint> {
    BTreeMap::from([(s_id, previously_programmed(&desired.vip))])
}

/// Build a single-backend `ServiceDesired` whose only backend has the
/// given V4 address (port 8080). The VIP is a routable service VIP that
/// is itself NOT in the mesh subnet, so only the backend's class is
/// under test.
fn desired_with_backend(backend_ip: Ipv4Addr) -> ServiceDesired {
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("valid ServiceVip");
    let backends = vec![Backend {
        workload_identity: SpiffeId::new("spiffe://overdrive.local/workload/web/alloc/web-0")
            .expect("valid SpiffeId"),
        addr: SocketAddr::new(IpAddr::V4(backend_ip), 8080),
        weight: 1,
        healthy: true,
    }];
    let fp = fingerprint(&vip, &backends);
    ServiceDesired {
        vip,
        port: std::num::NonZeroU16::new(8080).expect("non-zero"),
        proto: Proto::Tcp,
        backends,
        fingerprint: fp,
    }
}

/// Drive `ServiceMapHydrator::reconcile` for a single service whose only
/// backend has `backend_ip` and whose slot was programmed earlier with a
/// different set ([`previously_programmed`]), returning the port-exposed observable
/// universe: `(dataplane_update_service_count, programmed_fingerprint,
/// emitted_remote_backends_len)`.
///
/// Under the convergence-model realignment every dispatching service
/// records the PROGRAMMABLE fingerprint (`fingerprint(vip,
/// remote_survivors)`) in `RetryMemory.last_attempted_fingerprint` — for an
/// all-mesh service that is `Some(fingerprint(vip, []))` (the
/// empty-set purge), for a remote-only / mixed service it is `Some` over the
/// surviving remote backends. `emitted_remote_backends_len` is the length of
/// the `DataplaneUpdateService.backends` vector (0 for an empty purge).
fn reconcile_universe(backend_ip: Ipv4Addr) -> (usize, Option<BackendSetFingerprint>, usize) {
    let r = ServiceMapHydrator::canonical(workload_subnet());
    let s_id = make_service_id(1);
    let svc = desired_with_backend(backend_ip);
    let actual = actual_previously_programmed(s_id, &svc);
    let mut desired = BTreeMap::new();
    desired.insert(s_id, svc);
    let state = ServiceMapHydratorState { desired, actual };
    let view = ServiceMapHydratorView::default();

    let (actions, next_view) = r.reconcile(&state, &state, &view, &make_tick(0));

    let dataplane_update_service_count =
        actions.iter().filter(|a| matches!(a, Action::DataplaneUpdateService { .. })).count();
    let programmed_fingerprint =
        next_view.retries.get(&s_id).and_then(|m| m.last_attempted_fingerprint);
    let emitted_remote_backends_len = actions
        .iter()
        .find_map(|a| match a {
            Action::DataplaneUpdateService { backends, .. } => Some(backends.len()),
            _ => None,
        })
        .unwrap_or(0);

    (dataplane_update_service_count, programmed_fingerprint, emitted_remote_backends_len)
}

/// S-GATE mesh arm — under the convergence-model realignment
/// (`fix-mesh-only-reconcile-loop`, convergence-model.md § 11.1) the
/// all-mesh contract INVERTS: a mesh backend is
/// EXCLUDED from the remote backend PAYLOAD, but the service DOES emit the
/// empty-remote purge that settles it. The mesh backend never leaks into
/// the emitted backends — the `DataplaneUpdateService` carries the EMPTY
/// set (the documented per-proto purge, traits/dataplane.rs:197-204).
/// `@example`-pinned at 10.99.0.6 (slot-1 `/30`). `dataplane_count == 1`
/// (CHANGED from 0 — the purge); emitted backends EMPTY; `programmed_fp ==
/// Some(fingerprint(vip, []))` (CHANGED from `None` — the service now
/// genuinely dispatches a purge over the empty programmable set).
#[test]
fn mesh_subnet_backend_settles_via_empty_remote_purge() {
    let mesh_backend = Ipv4Addr::new(10, 99, 0, 6);
    assert!(
        workload_subnet().contains(&mesh_backend),
        "fixture precondition: 10.99.0.6 must be inside the mesh subnet"
    );
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("valid ServiceVip");
    let empty_fp = fingerprint(&vip, &[]);

    let (dataplane_count, programmed_fp, remote_backends_len) = reconcile_universe(mesh_backend);

    assert_eq!(
        dataplane_count, 1,
        "mesh backend must emit exactly ONE DataplaneUpdateService — the empty-remote purge that settles the service"
    );
    assert_eq!(
        remote_backends_len, 0,
        "the mesh backend must NOT leak into the emitted payload — the purge carries the EMPTY set"
    );
    assert_eq!(
        programmed_fp,
        Some(empty_fp),
        "an all-mesh service records the programmable fingerprint over the EMPTY set"
    );
}

/// S-GATE remote arm (error/edge — gate must NOT over-fire) — a backend
/// whose `addr` is not within `WORKLOAD_SUBNET_BASE` still emits
/// `DataplaneUpdateService` (the REMOTE arm is UNCHANGED). Proves the gate
/// does not swallow the remote path. `@example`-pinned at 10.96.0.50 — a
/// routable cluster backend outside the mesh subnet.
#[test]
fn non_mesh_non_host_backend_still_drives_dataplane_service_update() {
    let remote_backend = Ipv4Addr::new(10, 96, 0, 50);
    assert!(
        !workload_subnet().contains(&remote_backend),
        "fixture precondition: 10.96.0.50 must NOT be inside the mesh subnet"
    );

    let (dataplane_count, programmed_fp, remote_backends_len) = reconcile_universe(remote_backend);

    assert_eq!(
        dataplane_count, 1,
        "a non-mesh non-host backend must still emit exactly one DataplaneUpdateService (REMOTE arm)"
    );
    assert_eq!(
        remote_backends_len, 1,
        "the remote-only happy path is UNCHANGED — the surviving remote backend is in the payload"
    );
    assert!(
        programmed_fp.is_some(),
        "a dispatched (remote) service records its attempted fingerprint in the View"
    );
}

/// Build a `ServiceDesired` carrying the given V4 backend addresses (each
/// at port 8080) on one VIP. Unlike [`desired_with_backend`], this packs
/// MULTIPLE backends into one service so per-backend filtering inside a
/// single mixed service is observable. The VIP (10.0.0.1) is itself NOT in
/// the mesh subnet, so only the backends' classes are under test.
fn desired_with_backends(backend_ips: &[Ipv4Addr]) -> ServiceDesired {
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("valid ServiceVip");
    let backends: Vec<Backend> = backend_ips
        .iter()
        .map(|ip| Backend {
            workload_identity: SpiffeId::new("spiffe://overdrive.local/workload/web/alloc/web-0")
                .expect("valid SpiffeId"),
            addr: SocketAddr::new(IpAddr::V4(*ip), 8080),
            weight: 1,
            healthy: true,
        })
        .collect();
    let fp = fingerprint(&vip, &backends);
    ServiceDesired {
        vip,
        port: std::num::NonZeroU16::new(8080).expect("non-zero"),
        proto: Proto::Tcp,
        backends,
        fingerprint: fp,
    }
}

/// D2 (mixed service, per-backend filtering) — the single test that
/// observes the EMITTED `backends` vector content, not just action counts.
/// One service, TWO backends spanning both address classes:
///
///   - `10.99.0.6:8080`   — mesh (∈ 10.99.0.0/16) -> EXCLUDED from the payload;
///   - `10.96.0.50:8080`  — remote (∉ subnet) -> survives into
///     `DataplaneUpdateService.backends`.
///
/// Pins per-backend filtering: the mesh backend must NOT leak into the
/// emitted remote vector, and no emitted action may reference the mesh
/// address. Every existing gate test is single-backend and asserts only
/// action COUNTS — this is the only test observing the surviving `backends`
/// vector.
#[test]
fn mixed_service_excludes_mesh_keeps_remote_backend() {
    let mesh = Ipv4Addr::new(10, 99, 0, 6);
    let remote = Ipv4Addr::new(10, 96, 0, 50);
    assert!(workload_subnet().contains(&mesh), "fixture precondition: 10.99.0.6 ∈ mesh subnet");
    assert!(!workload_subnet().contains(&remote), "fixture precondition: 10.96.0.50 ∉ mesh subnet");

    let r = ServiceMapHydrator::canonical(workload_subnet());
    let s_id = make_service_id(1);
    let mut desired = BTreeMap::new();
    desired.insert(s_id, desired_with_backends(&[mesh, remote]));
    let state = ServiceMapHydratorState { desired, actual: BTreeMap::new() };
    let view = ServiceMapHydratorView::default();

    let (actions, _next_view) = r.reconcile(&state, &state, &view, &make_tick(0));

    // Exactly one DataplaneUpdateService carrying EXACTLY the remote backend.
    let dataplane: Vec<&Action> =
        actions.iter().filter(|a| matches!(a, Action::DataplaneUpdateService { .. })).collect();
    assert_eq!(
        dataplane.len(),
        1,
        "exactly one DataplaneUpdateService for the surviving remote backend"
    );
    match dataplane[0] {
        Action::DataplaneUpdateService { backends, .. } => {
            assert_eq!(
                backends.len(),
                1,
                "the remote path must carry EXACTLY one backend (the mesh backend partitioned out)"
            );
            assert_eq!(
                backends[0].addr,
                SocketAddr::new(IpAddr::V4(remote), 8080),
                "the surviving remote backend must be 10.96.0.50:8080 — the mesh backend did NOT leak in"
            );
        }
        other => panic!("expected DataplaneUpdateService, got {other:?}"),
    }

    // The mesh backend leaks into NO emitted action's address surface.
    for action in &actions {
        if let Action::DataplaneUpdateService { backends, .. } = action {
            for b in backends {
                assert_ne!(
                    b.addr,
                    SocketAddr::new(IpAddr::V4(mesh), 8080),
                    "mesh backend 10.99.0.6 must NOT appear in any DataplaneUpdateService"
                );
            }
        }
    }
}

/// An all-mesh service SETTLES via the empty-remote purge (convergence-model.md
/// § 4 / § 11.1). A service whose slot still carries a previously programmed
/// remote set is not in place against an empty survivor set, so it dispatches
/// the purge; once the purge lands the dataplane carries nothing for it, which
/// is in place for an empty survivor set, and the hydrator goes quiet. Both
/// backends ∈ 10.99.0.0/16.
///
/// - **Tick 1** (`actual` carries an earlier remote set): emits exactly ONE
///   `DataplaneUpdateService { backends: [] }` (the per-proto purge) and
///   records `retries[s].last_attempted_fingerprint = Some(fp(vip, []))`.
/// - **Tick 2** (the purge applied, so `actual` has no entry; View fed
///   forward): emits ZERO actions and CLEARS `retries` — settled.
#[test]
fn all_mesh_service_settles_via_empty_remote_purge() {
    let m1 = Ipv4Addr::new(10, 99, 0, 6);
    let m2 = Ipv4Addr::new(10, 99, 0, 10);
    assert!(workload_subnet().contains(&m1), "fixture precondition: 10.99.0.6 ∈ mesh subnet");
    assert!(workload_subnet().contains(&m2), "fixture precondition: 10.99.0.10 ∈ mesh subnet");

    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("valid ServiceVip");
    let empty_fp = fingerprint(&vip, &[]);

    let r = ServiceMapHydrator::canonical(workload_subnet());
    let s_id = make_service_id(1);
    let svc = desired_with_backends(&[m1, m2]);
    let programmed_earlier = actual_previously_programmed(s_id, &svc);
    let mut desired = BTreeMap::new();
    desired.insert(s_id, svc);

    // Tick 1 — default View, the slot still carries an earlier remote set.
    let state = ServiceMapHydratorState { desired: desired.clone(), actual: programmed_earlier };
    let view0 = ServiceMapHydratorView::default();
    let (actions1, view1) = r.reconcile(&state, &state, &view0, &make_tick(0));
    let dataplane1: Vec<&Action> =
        actions1.iter().filter(|a| matches!(a, Action::DataplaneUpdateService { .. })).collect();
    assert_eq!(
        dataplane1.len(),
        1,
        "tick 1: an all-mesh service over a programmed slot emits exactly ONE DataplaneUpdateService (the empty purge)"
    );
    match dataplane1[0] {
        Action::DataplaneUpdateService { backends, .. } => {
            assert!(backends.is_empty(), "tick 1: the purge carries the EMPTY backend set");
        }
        other => panic!("expected DataplaneUpdateService, got {other:?}"),
    }
    assert_eq!(
        view1.retries.get(&s_id).and_then(|m| m.last_attempted_fingerprint),
        Some(empty_fp),
        "tick 1: an all-mesh service records the programmable fingerprint over the EMPTY set"
    );

    // Tick 2 — the purge applied: nothing is programmed for the service.
    let settled = ServiceMapHydratorState { desired, actual: BTreeMap::new() };
    let (actions2, view2) = r.reconcile(&settled, &settled, &view1, &make_tick(2));
    assert!(actions2.is_empty(), "tick 2: a settled all-mesh service emits ZERO actions");
    assert!(
        view2.retries.is_empty(),
        "tick 2: the in-place observation CLEARS retries once the slot is observed empty"
    );
}

/// An all-mesh service whose slot was never programmed is already in place
/// (an empty survivor set against no entry): it emits nothing and records no
/// retry memory, for a V4 and for a V6 VIP. This is the loop-free half of the
/// settle contract: the hydrator does not re-send a purge to an empty slot.
#[test]
fn all_mesh_service_with_nothing_programmed_emits_nothing() {
    let m1 = Ipv4Addr::new(10, 99, 0, 6);
    let m2 = Ipv4Addr::new(10, 99, 0, 10);
    let r = ServiceMapHydrator::canonical(workload_subnet());
    let s_id = make_service_id(1);
    let vip6 = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1);
    for (label, svc) in [
        ("V4 VIP", desired_with_backends(&[m1, m2])),
        ("V6 VIP", v6_vip_desired_with_backends(vip6, &[m1, m2])),
    ] {
        let state = ServiceMapHydratorState {
            desired: BTreeMap::from([(s_id, svc)]),
            actual: BTreeMap::new(),
        };
        let (actions, view) =
            r.reconcile(&state, &state, &ServiceMapHydratorView::default(), &make_tick(0));
        assert!(actions.is_empty(), "{label}: nothing to purge, nothing emitted");
        assert!(view.retries.is_empty(), "{label}: no dispatch, no retry memory");
    }
}

/// Build a `ServiceDesired` whose VIP is a **V6** `ServiceVip` carrying the
/// given V4 backend addresses (each at port 8080). `ServiceVip` wraps
/// `IpAddr` and accepts V6 at the type/parser level, so the V6 VIP arm of
/// `ServiceMapHydrator::reconcile` is reachable through the driving port —
/// only the IPv4-only `VipRange` allocator keeps it unreached in the current
/// production flow, with no compile-time guard. The backends are V4 so the
/// mesh gate (which keys on the BACKEND's address, not the VIP's family)
/// must still apply.
fn v6_vip_desired_with_backends(vip6: Ipv6Addr, backend_ips: &[Ipv4Addr]) -> ServiceDesired {
    let vip = ServiceVip::new(IpAddr::V6(vip6)).expect("valid V6 ServiceVip");
    let backends: Vec<Backend> = backend_ips
        .iter()
        .map(|ip| Backend {
            workload_identity: SpiffeId::new("spiffe://overdrive.local/workload/web/alloc/web-0")
                .expect("valid SpiffeId"),
            addr: SocketAddr::new(IpAddr::V4(*ip), 8080),
            weight: 1,
            healthy: true,
        })
        .collect();
    let fp = fingerprint(&vip, &backends);
    ServiceDesired {
        vip,
        port: std::num::NonZeroU16::new(8080).expect("non-zero"),
        proto: Proto::Tcp,
        backends,
        fingerprint: fp,
    }
}

/// Regression (latent defect) — the V6 VIP arm of
/// `ServiceMapHydrator::reconcile` previously branched on the VIP address
/// family BEFORE applying the `is_mesh_backend` gate: it emitted
/// `DataplaneUpdateService` with the FULL backend list and `continue`d,
/// bypassing the mesh filter the V4 path applies. The mesh-gate invariant
/// (ADR-0071: a mesh `workload_addr` backend's delivery is owned
/// EXCLUSIVELY by nft-TPROXY, NEVER the dataplane LB path) keys on the
/// BACKEND's address, not the VIP's family — so a V6 VIP carrying a V4
/// mesh backend silently leaked the mesh backend into the LB path
/// (split-brain delivery). `ServiceVip` accepts V6 at the type/parser
/// level, so the arm is reachable through the driving port.
///
/// A V6 VIP carrying a mesh backend (`10.99.0.6`) + a non-mesh remote
/// backend (`10.96.0.50`) must emit a `DataplaneUpdateService` whose
/// `backends` contains ONLY the remote backend — the mesh backend must
/// NOT appear.
#[test]
fn v6_vip_service_excludes_mesh_backend_from_dataplane_update() {
    let vip6 = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1);
    let mesh = Ipv4Addr::new(10, 99, 0, 6);
    let remote = Ipv4Addr::new(10, 96, 0, 50);
    assert!(workload_subnet().contains(&mesh), "fixture precondition: 10.99.0.6 ∈ mesh subnet");
    assert!(!workload_subnet().contains(&remote), "fixture precondition: 10.96.0.50 ∉ mesh subnet");

    let r = ServiceMapHydrator::canonical(workload_subnet());
    let s_id = make_service_id(1);
    let mut desired = BTreeMap::new();
    desired.insert(s_id, v6_vip_desired_with_backends(vip6, &[mesh, remote]));
    let state = ServiceMapHydratorState { desired, actual: BTreeMap::new() };
    let view = ServiceMapHydratorView::default();

    let (actions, _next_view) = r.reconcile(&state, &state, &view, &make_tick(0));

    // Exactly one DataplaneUpdateService carrying EXACTLY the remote backend.
    let dataplane: Vec<&Action> =
        actions.iter().filter(|a| matches!(a, Action::DataplaneUpdateService { .. })).collect();
    assert_eq!(
        dataplane.len(),
        1,
        "a V6 VIP service with a surviving remote backend emits exactly one DataplaneUpdateService"
    );
    match dataplane[0] {
        Action::DataplaneUpdateService { backends, .. } => {
            assert_eq!(
                backends.len(),
                1,
                "the V6 path must carry EXACTLY one backend (the mesh backend gated out)"
            );
            assert_eq!(
                backends[0].addr,
                SocketAddr::new(IpAddr::V4(remote), 8080),
                "the surviving backend must be the remote 10.96.0.50:8080"
            );
        }
        other => panic!("expected DataplaneUpdateService, got {other:?}"),
    }

    // The mesh backend must NOT appear in any emitted DataplaneUpdateService.
    for action in &actions {
        if let Action::DataplaneUpdateService { backends, .. } = action {
            for b in backends {
                assert_ne!(
                    b.addr,
                    SocketAddr::new(IpAddr::V4(mesh), 8080),
                    "mesh backend 10.99.0.6 must NOT leak into a V6 VIP DataplaneUpdateService"
                );
            }
        }
    }
}

/// An all-mesh V6 VIP service SETTLES via the empty-remote purge, mirroring the
/// V4 `all_mesh_service_settles_via_empty_remote_purge` contract (convergence-model.md
/// § 3.1 V6 note / § 11.1). The V6 arm emits the empty purge unconditionally on
/// dispatch while the slot still carries an earlier set, records the
/// programmable fingerprint over the empty set, and goes quiet once the slot
/// is observed empty.
#[test]
fn v6_vip_all_mesh_service_settles_via_empty_remote_purge() {
    let vip6 = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1);
    let m1 = Ipv4Addr::new(10, 99, 0, 6);
    let m2 = Ipv4Addr::new(10, 99, 0, 10);
    assert!(workload_subnet().contains(&m1), "fixture precondition: 10.99.0.6 ∈ mesh subnet");
    assert!(workload_subnet().contains(&m2), "fixture precondition: 10.99.0.10 ∈ mesh subnet");

    let vip = ServiceVip::new(IpAddr::V6(vip6)).expect("valid V6 ServiceVip");
    let empty_fp = fingerprint(&vip, &[]);

    let r = ServiceMapHydrator::canonical(workload_subnet());
    let s_id = make_service_id(1);
    let svc = v6_vip_desired_with_backends(vip6, &[m1, m2]);
    let programmed_earlier = actual_previously_programmed(s_id, &svc);
    let desired = BTreeMap::from([(s_id, svc)]);
    let view0 = ServiceMapHydratorView::default();

    // Tick 1 — one empty-remote purge over the still-programmed slot.
    let state = ServiceMapHydratorState { desired: desired.clone(), actual: programmed_earlier };
    let (actions1, view1) = r.reconcile(&state, &state, &view0, &make_tick(0));
    let dataplane1: Vec<&Action> =
        actions1.iter().filter(|a| matches!(a, Action::DataplaneUpdateService { .. })).collect();
    assert_eq!(
        dataplane1.len(),
        1,
        "tick 1: an all-mesh V6 service over a programmed slot emits exactly ONE DataplaneUpdateService (the empty purge)"
    );
    match dataplane1[0] {
        Action::DataplaneUpdateService { backends, .. } => {
            assert!(backends.is_empty(), "tick 1: the V6 purge carries the EMPTY backend set");
        }
        other => panic!("expected DataplaneUpdateService, got {other:?}"),
    }
    assert_eq!(
        view1.retries.get(&s_id).and_then(|m| m.last_attempted_fingerprint),
        Some(empty_fp),
        "tick 1: the V6 all-mesh service records the programmable fingerprint over the EMPTY set"
    );

    // Tick 2 — the purge applied, the slot is empty: settled, retries cleared.
    let settled = ServiceMapHydratorState { desired, actual: BTreeMap::new() };
    let (actions2, view2) = r.reconcile(&settled, &settled, &view1, &make_tick(2));
    assert!(actions2.is_empty(), "tick 2: a settled all-mesh V6 service emits ZERO actions");
    assert!(
        view2.retries.is_empty(),
        "tick 2: the in-place observation CLEARS retries once the slot is observed empty"
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        rng_seed: proptest::test_runner::RngSeed::Fixed(257_228),
        ..ProptestConfig::default()
    })]

    /// CONTRACT_SHAPE: pure-function.
    /// ADR-0101 D7: exact remote health action and full View complement.
    /// Pure inputs are not a reachability claim; BE10 seed257221 supplies that.
    #[test]
    fn local_health_selects_existing_action_preserving_remote_and_view(
        listener_port in 1u16..=u16::MAX,
        backend_port in 1u16..=u16::MAX,
    ) {
        use overdrive_core::id::{ContentHash, CorrelationKey};
        use overdrive_reconcilers::RetryMemory;

        let reconciler = ServiceMapHydrator::canonical(workload_subnet());
        let sid = make_service_id(42);
        let tick = make_tick(10);
        for address in [Ipv4Addr::new(10, 96, 0, 50), Ipv4Addr::new(10, 99, 0, 6)] {
            for proto in [Proto::Tcp, Proto::Udp] {
                for healthy in [true, false] {
                    let mut svc = desired_with_backend(address);
                    svc.port = std::num::NonZeroU16::new(listener_port).unwrap();
                    svc.proto = proto;
                    svc.backends[0].addr.set_port(backend_port);
                    svc.backends[0].healthy = healthy;
                    svc.fingerprint = fingerprint(&svc.vip, &svc.backends);
                    let remote = !workload_subnet().contains(&address);
                    let remote_backends = if remote { svc.backends.clone() } else { vec![] };
                    let target = format!("service-map-hydrator/{sid}");
                    let hash = ContentHash::of(svc.fingerprint.to_le_bytes().as_slice());
                    let expected_actions = vec![Action::DataplaneUpdateService {
                        service_id: sid, vip: svc.vip, port: svc.port, proto,
                        backends: remote_backends.clone(),
                        correlation: CorrelationKey::derive(&target, &hash, "update-service"),
                    }];
                    let expected_view = ServiceMapHydratorView {
                        retries: BTreeMap::from([(sid, RetryMemory {
                            attempts: 1,
                            last_failure_seen_at: tick.now_unix,
                            last_attempted_fingerprint: Some(fingerprint(&svc.vip, &remote_backends)),
                        })]),
                    };
                    // The slot carries an earlier set, so the survivor set (or the
                    // empty purge for a mesh backend) is not in place.
                    let actual = actual_previously_programmed(sid, &svc);
                    let state = ServiceMapHydratorState {
                        desired: BTreeMap::from([(sid, svc)]), actual,
                    };
                    let (actions, view) = reconciler.reconcile(
                        &state, &state, &ServiceMapHydratorView::default(), &tick,
                    );
                    prop_assert_eq!(actions, expected_actions, "seed=257228: exact ordered action universe");
                    prop_assert_eq!(&view, &expected_view, "seed=257228: complete View complement");
                    // Backoff pacing over a service that is still not in place, not
                    // effect acknowledgement or failed-effect retry.
                    let (repeat_actions, repeat_view) = reconciler.reconcile(&state, &state, &view, &tick);
                    prop_assert!(repeat_actions.is_empty());
                    prop_assert_eq!(repeat_view, view);
                }
            }
        }
    }
}

proptest! {
    /// PBT over the two address classes (convergence-model.md § 11.1):
    /// every single-backend service emits EXACTLY ONE `DataplaneUpdateService`
    /// (the remote/XDP path — populated for a remote backend, EMPTY purge for
    /// mesh). The OLD "mesh zeroes both paths" invariant is DELETED — it encoded the
    /// perpetual-loop bug. Mesh membership routes the backend out of the
    /// PAYLOAD (it never leaks in); the service still emits the empty purge
    /// that settles it. Strategy spans one representative per arm so shrinking
    /// always reports the minimal failing class.
    #[test]
    fn two_way_split_routes_each_address_class_to_exactly_one_disposition(
        backend_ip in prop_oneof![
            // mesh class — anywhere inside 10.99.0.0/16
            (0u8..=255, 0u8..=255).prop_map(|(c, d)| Ipv4Addr::new(10, 99, c, d)),
            // remote class — a routable address outside the mesh subnet
            (1u8..=95, 0u8..=255, 0u8..=255).prop_map(|(b, c, d)| Ipv4Addr::new(10, b, c, d)),
        ]
    ) {
        let (dataplane_count, programmed_fp, remote_backends_len) =
            reconcile_universe(backend_ip);

        let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))
            .expect("valid ServiceVip");
        let empty_fp = fingerprint(&vip, &[]);

        let is_mesh = workload_subnet().contains(&backend_ip);

        if is_mesh {
            // all-mesh single-backend service: empty-remote purge settles it.
            prop_assert_eq!(dataplane_count, 1, "mesh: one DataplaneUpdateService (empty purge)");
            prop_assert_eq!(remote_backends_len, 0, "mesh: empty purge payload");
            prop_assert_eq!(programmed_fp, Some(empty_fp), "mesh: programmed over the empty set");
        } else {
            // remote-only happy path — UNCHANGED: the surviving remote backend
            // is in the payload and the programmed fingerprint is over it.
            let remote_backend = Backend {
                workload_identity: SpiffeId::new("spiffe://overdrive.local/workload/web/alloc/web-0")
                    .expect("valid SpiffeId"),
                addr: SocketAddr::new(IpAddr::V4(backend_ip), 8080),
                weight: 1,
                healthy: true,
            };
            let remote_fp = fingerprint(&vip, std::slice::from_ref(&remote_backend));
            prop_assert_eq!(dataplane_count, 1, "remote: exactly one DataplaneUpdateService");
            prop_assert_eq!(remote_backends_len, 1, "remote: the surviving backend is in the payload");
            prop_assert_eq!(programmed_fp, Some(remote_fp), "remote: programmed over the remote survivor");
        }
    }
}

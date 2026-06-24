//! S-GATE — `ServiceMapHydrator` gates mesh-subnet backends out of BOTH
//! load-balancer paths, leaving the local and remote arms unchanged (DISTILL RED
//! scaffold, GH #241, Tier-1 DST / reconciler-logic, default-lane).
//!
//! D-GATE / D-GATE-PRED / `@us-GATE`. The driving port is
//! `ServiceMapHydrator::reconcile`. A three-way split applied BEFORE the existing
//! LOCAL/REMOTE partition:
//!
//!   - `addr.ip() ∈ WORKLOAD_SUBNET_BASE (10.99.0.0/16)` -> emits NEITHER
//!     `RegisterLocalBackend` NOR `DataplaneUpdateService` (mesh -> skip;
//!     nft-TPROXY owns delivery);
//!   - `addr == host_ipv4` -> `RegisterLocalBackend` (UNCHANGED LOCAL arm);
//!   - otherwise -> `DataplaneUpdateService` (UNCHANGED REMOTE arm).
//!
//! The two non-mesh arms are the error/edge coverage — they prove the gate does
//! NOT over-fire (a mutant gating everything, or gating nothing, fails here).
//!
//! Mandate 8 (Universe): the reconcile-returned actions'
//! `register_local_backend_count` + `dataplane_update_service_count` + the
//! `View`'s programmed fingerprint; NEVER the hydrator's private partition state.
//! Mandate 9: Tier-1 -> PBT-eligible over the three address classes;
//! `@example`-pin a representative addr per arm (10.99.0.6 mesh / `host_ipv4` local
//! / 10.96.0.50 remote).
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-GATE.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant};

use ipnet::Ipv4Net;
use proptest::prelude::*;

use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::dataplane::fingerprint::fingerprint;
use overdrive_core::id::{ServiceId, ServiceVip, SpiffeId};
use overdrive_core::reconcilers::{
    Action, Reconciler, ServiceDesired, ServiceMapHydrator, ServiceMapHydratorState,
    ServiceMapHydratorView, TickContext,
};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::wall_clock::UnixInstant;

/// The canonical Path-A/mesh workload subnet — the SAME `10.99.0.0/16`
/// `WORKLOAD_SUBNET_BASE` the provisioner carves per-allocation `/30`s
/// from (one source, D-GATE-PRED). Core constructs the literal because
/// `WORKLOAD_SUBNET_BASE` lives in the `overdrive-control-plane` wiring
/// crate, which core MUST NOT depend on.
fn workload_subnet() -> Ipv4Net {
    Ipv4Net::new(Ipv4Addr::new(10, 99, 0, 0), 16).expect("valid /16")
}

/// The configured host IPv4 — the LOCAL-arm classifier input. Distinct
/// from any mesh or remote address used below.
const fn host_ipv4() -> Ipv4Addr {
    Ipv4Addr::new(10, 0, 0, 9)
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

/// Build a single-backend `ServiceDesired` whose only backend has the
/// given V4 address (port 8080). The VIP is a routable service VIP that
/// is itself NOT in the mesh subnet, so only the backend's class is
/// under test.
fn desired_with_backend(backend_ip: Ipv4Addr) -> ServiceDesired {
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("valid ServiceVip");
    let backends = vec![Backend {
        alloc: SpiffeId::new("spiffe://overdrive.local/job/web/alloc/web-0")
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
/// backend has `backend_ip`, returning the port-exposed observable
/// universe: `(register_local_backend_count, dataplane_update_service_count,
/// programmed_fingerprint)`. The programmed fingerprint is read from the
/// returned `View` (`RetryMemory.last_attempted_fingerprint`) — `Some` iff
/// the hydrator counted this service as dispatched, `None` iff the service
/// was gated out of both paths.
fn reconcile_universe(backend_ip: Ipv4Addr) -> (usize, usize, Option<u64>) {
    let r = ServiceMapHydrator::canonical(host_ipv4(), workload_subnet());
    let s_id = make_service_id(1);
    let mut desired = BTreeMap::new();
    desired.insert(s_id, desired_with_backend(backend_ip));
    let state = ServiceMapHydratorState { desired, actual: BTreeMap::new() };
    let view = ServiceMapHydratorView::default();

    let (actions, next_view) = r.reconcile(&state, &state, &view, &make_tick(0));

    let register_local_backend_count =
        actions.iter().filter(|a| matches!(a, Action::RegisterLocalBackend { .. })).count();
    let dataplane_update_service_count =
        actions.iter().filter(|a| matches!(a, Action::DataplaneUpdateService { .. })).count();
    let programmed_fingerprint =
        next_view.retries.get(&s_id).and_then(|m| m.last_attempted_fingerprint);

    (register_local_backend_count, dataplane_update_service_count, programmed_fingerprint)
}

/// S-GATE mesh arm (happy) — a backend whose `addr.ip()` is within
/// `WORKLOAD_SUBNET_BASE` (10.99.0.0/16) emits NEITHER
/// `RegisterLocalBackend` NOR `DataplaneUpdateService` (mesh -> skip;
/// nft-TPROXY owns delivery). `@example`-pinned at 10.99.0.6 — the
/// canonical in-netns workload address (slot-1 `/30`). The retry-memory
/// fingerprint stays `None`: a fully-gated service is not counted as
/// dispatched.
#[test]
fn mesh_subnet_backend_programs_neither_local_nor_remote_lb_path() {
    let mesh_backend = Ipv4Addr::new(10, 99, 0, 6);
    assert!(
        workload_subnet().contains(&mesh_backend),
        "fixture precondition: 10.99.0.6 must be inside the mesh subnet"
    );

    let (register_count, dataplane_count, programmed_fp) = reconcile_universe(mesh_backend);

    assert_eq!(
        register_count, 0,
        "mesh backend must emit NO RegisterLocalBackend (nft-TPROXY owns delivery)"
    );
    assert_eq!(
        dataplane_count, 0,
        "mesh backend must emit NO DataplaneUpdateService (nft-TPROXY owns delivery)"
    );
    assert_eq!(
        programmed_fp, None,
        "a fully-gated mesh service is not counted as dispatched — no programmed fingerprint"
    );
}

/// S-GATE local arm (error/edge — gate must NOT over-fire) — a backend
/// whose `addr == host_ipv4` still emits `RegisterLocalBackend` (the
/// LOCAL arm is UNCHANGED). Proves the gate does not swallow the local
/// path. `@example`-pinned at `host_ipv4`.
#[test]
fn host_address_backend_still_registers_as_local_backend() {
    let local_backend = host_ipv4();
    assert!(
        !workload_subnet().contains(&local_backend),
        "fixture precondition: host_ipv4 must NOT be inside the mesh subnet"
    );

    let (register_count, dataplane_count, programmed_fp) = reconcile_universe(local_backend);

    assert_eq!(
        register_count, 1,
        "a host-address backend must still emit exactly one RegisterLocalBackend (LOCAL arm)"
    );
    assert_eq!(
        dataplane_count, 0,
        "a host-address backend is local — no DataplaneUpdateService for the remote path"
    );
    assert!(
        programmed_fp.is_some(),
        "a dispatched (local) service records its attempted fingerprint in the View"
    );
}

/// S-GATE remote arm (error/edge — gate must NOT over-fire) — a backend
/// whose `addr` is neither `host_ipv4` nor within `WORKLOAD_SUBNET_BASE`
/// still emits `DataplaneUpdateService` (the REMOTE arm is UNCHANGED).
/// Proves the gate does not swallow the remote path. `@example`-pinned at
/// 10.96.0.50 — a routable cluster backend outside both the host address
/// and the mesh subnet.
#[test]
fn non_mesh_non_host_backend_still_drives_dataplane_service_update() {
    let remote_backend = Ipv4Addr::new(10, 96, 0, 50);
    assert!(
        !workload_subnet().contains(&remote_backend),
        "fixture precondition: 10.96.0.50 must NOT be inside the mesh subnet"
    );
    assert_ne!(remote_backend, host_ipv4(), "fixture precondition: must not equal host_ipv4");

    let (register_count, dataplane_count, programmed_fp) = reconcile_universe(remote_backend);

    assert_eq!(register_count, 0, "a remote backend is not local — no RegisterLocalBackend");
    assert_eq!(
        dataplane_count, 1,
        "a non-mesh non-host backend must still emit exactly one DataplaneUpdateService (REMOTE arm)"
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
            alloc: SpiffeId::new("spiffe://overdrive.local/job/web/alloc/web-0")
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
/// One service, THREE backends spanning all three address classes against a
/// `host_ipv4 = 10.0.0.1` that is itself NOT in the mesh subnet:
///
///   - `10.99.0.6:8080`   — mesh (∈ 10.99.0.0/16) -> EXCLUDED from both paths;
///   - `10.96.0.50:8080`  — remote (≠ `host_ipv4`, ∉ subnet) -> survives into
///     `DataplaneUpdateService.backends`;
///   - `10.0.0.1:8080`    — local (== `host_ipv4`) -> `RegisterLocalBackend`.
///
/// Pins per-backend filtering: the mesh backend must NOT leak into the
/// emitted remote vector, the local backend must NOT leak into it either,
/// and no emitted action may reference the mesh address. Every existing
/// gate test is single-backend and asserts only action COUNTS — this is the
/// only test observing the surviving `backends` vector.
#[test]
fn mixed_service_excludes_mesh_keeps_remote_backend_and_registers_local() {
    let host = Ipv4Addr::new(10, 0, 0, 1);
    let mesh = Ipv4Addr::new(10, 99, 0, 6);
    let remote = Ipv4Addr::new(10, 96, 0, 50);
    // local == host, exercised below.
    assert!(workload_subnet().contains(&mesh), "fixture precondition: 10.99.0.6 ∈ mesh subnet");
    assert!(!workload_subnet().contains(&remote), "fixture precondition: 10.96.0.50 ∉ mesh subnet");
    assert!(!workload_subnet().contains(&host), "fixture precondition: host_ipv4 ∉ mesh subnet");

    let r = ServiceMapHydrator::canonical(host, workload_subnet());
    let s_id = make_service_id(1);
    let mut desired = BTreeMap::new();
    desired.insert(s_id, desired_with_backends(&[mesh, remote, host]));
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
                "the remote path must carry EXACTLY one backend (mesh + local partitioned out)"
            );
            assert_eq!(
                backends[0].addr,
                SocketAddr::new(IpAddr::V4(remote), 8080),
                "the surviving remote backend must be 10.96.0.50:8080 — the mesh backend did NOT leak in"
            );
        }
        other => panic!("expected DataplaneUpdateService, got {other:?}"),
    }

    // Exactly one RegisterLocalBackend for the host-address backend.
    let register: Vec<&Action> =
        actions.iter().filter(|a| matches!(a, Action::RegisterLocalBackend { .. })).collect();
    assert_eq!(register.len(), 1, "exactly one RegisterLocalBackend for the local (host) backend");
    match register[0] {
        Action::RegisterLocalBackend { backend, .. } => {
            assert_eq!(
                *backend,
                std::net::SocketAddrV4::new(host, 8080),
                "the local backend registered must be host_ipv4:8080 (10.0.0.1:8080)"
            );
        }
        other => panic!("expected RegisterLocalBackend, got {other:?}"),
    }

    // The mesh backend leaks into NO emitted action's address surface.
    for action in &actions {
        match action {
            Action::DataplaneUpdateService { backends, .. } => {
                for b in backends {
                    assert_ne!(
                        b.addr,
                        SocketAddr::new(IpAddr::V4(mesh), 8080),
                        "mesh backend 10.99.0.6 must NOT appear in any DataplaneUpdateService"
                    );
                }
            }
            Action::RegisterLocalBackend { backend, .. } => {
                assert_ne!(
                    backend.ip(),
                    &mesh,
                    "mesh backend 10.99.0.6 must NOT appear in any RegisterLocalBackend"
                );
            }
            _ => {}
        }
    }
}

/// D3 (≥2-tick all-mesh retry-guard) — an all-mesh service records NOTHING
/// in the View and does not bump the retry budget, across two consecutive
/// ticks. Without the all-mesh guard the service records a phantom
/// fingerprint and re-dispatches + fsyncs a View row ~once/sec forever
/// (the ratified-correct behavior the orchestrator's trace established).
/// Both backends ∈ 10.99.0.0/16; `host_ipv4 = 10.0.0.1`.
///
/// Tick 2 feeds `view1` forward with a later `tick.now_unix` to prove the
/// emptiness is stable across ticks — a phantom dispatch on tick 1 would
/// have populated `view1.retries` and the guard would never re-engage.
#[test]
fn all_mesh_service_emits_nothing_and_keeps_retries_empty_across_ticks() {
    let host = Ipv4Addr::new(10, 0, 0, 1);
    let m1 = Ipv4Addr::new(10, 99, 0, 6);
    let m2 = Ipv4Addr::new(10, 99, 0, 10);
    assert!(workload_subnet().contains(&m1), "fixture precondition: 10.99.0.6 ∈ mesh subnet");
    assert!(workload_subnet().contains(&m2), "fixture precondition: 10.99.0.10 ∈ mesh subnet");

    let r = ServiceMapHydrator::canonical(host, workload_subnet());
    let s_id = make_service_id(1);
    let mut desired = BTreeMap::new();
    desired.insert(s_id, desired_with_backends(&[m1, m2]));
    let state = ServiceMapHydratorState { desired, actual: BTreeMap::new() };

    // Tick 1 — default View, empty actual.
    let view0 = ServiceMapHydratorView::default();
    let (actions1, view1) = r.reconcile(&state, &state, &view0, &make_tick(0));
    assert!(actions1.is_empty(), "tick 1: an all-mesh service must emit NO actions");
    assert!(
        view1.retries.is_empty(),
        "tick 1: an all-mesh service must record NOTHING in the View (no phantom retry budget)"
    );

    // Tick 2 — feed view1 forward, later now_unix.
    let (actions2, view2) = r.reconcile(&state, &state, &view1, &make_tick(2));
    assert!(actions2.is_empty(), "tick 2: an all-mesh service must STILL emit NO actions");
    assert!(
        view2.retries.is_empty(),
        "tick 2: retries must STAY empty — no phantom dispatch re-engaged the backoff gate"
    );
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
            alloc: SpiffeId::new("spiffe://overdrive.local/job/web/alloc/web-0")
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
/// NOT appear. The V6 arm has no local path, so no `RegisterLocalBackend`
/// is emitted.
#[test]
fn v6_vip_service_excludes_mesh_backend_from_dataplane_update() {
    let host = Ipv4Addr::new(10, 0, 0, 1);
    let vip6 = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1);
    let mesh = Ipv4Addr::new(10, 99, 0, 6);
    let remote = Ipv4Addr::new(10, 96, 0, 50);
    assert!(workload_subnet().contains(&mesh), "fixture precondition: 10.99.0.6 ∈ mesh subnet");
    assert!(!workload_subnet().contains(&remote), "fixture precondition: 10.96.0.50 ∉ mesh subnet");

    let r = ServiceMapHydrator::canonical(host, workload_subnet());
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

    // The V6 arm has no local path — no RegisterLocalBackend is emitted.
    let register_count =
        actions.iter().filter(|a| matches!(a, Action::RegisterLocalBackend { .. })).count();
    assert_eq!(register_count, 0, "the V6 VIP arm has no local path — no RegisterLocalBackend");
}

/// Regression (latent defect, all-mesh half) — an all-mesh V6 VIP service
/// must emit NOTHING and leave `view.retries` empty, mirroring the V4
/// `all_mesh_service_emits_nothing_and_keeps_retries_empty_across_ticks`
/// guard. The pre-fix V6 arm bumped the retry budget unconditionally (it
/// emitted `DataplaneUpdateService` before gating), so an all-mesh V6
/// service recorded a phantom dispatch and would re-dispatch + fsync a
/// View row forever. Post-fix, with the gate hoisted above the VIP-family
/// switch, an all-mesh V6 service emits no action AND records nothing.
#[test]
fn v6_vip_all_mesh_service_emits_nothing_and_keeps_retries_empty() {
    let host = Ipv4Addr::new(10, 0, 0, 1);
    let vip6 = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1);
    let m1 = Ipv4Addr::new(10, 99, 0, 6);
    let m2 = Ipv4Addr::new(10, 99, 0, 10);
    assert!(workload_subnet().contains(&m1), "fixture precondition: 10.99.0.6 ∈ mesh subnet");
    assert!(workload_subnet().contains(&m2), "fixture precondition: 10.99.0.10 ∈ mesh subnet");

    let r = ServiceMapHydrator::canonical(host, workload_subnet());
    let s_id = make_service_id(1);
    let mut desired = BTreeMap::new();
    desired.insert(s_id, v6_vip_desired_with_backends(vip6, &[m1, m2]));
    let state = ServiceMapHydratorState { desired, actual: BTreeMap::new() };
    let view = ServiceMapHydratorView::default();

    let (actions, next_view) = r.reconcile(&state, &state, &view, &make_tick(0));

    assert!(actions.is_empty(), "an all-mesh V6 VIP service must emit NO actions");
    assert!(
        next_view.retries.is_empty(),
        "an all-mesh V6 VIP service must record NOTHING in the View (no phantom retry budget)"
    );
}

proptest! {
    /// PBT over the three address classes: for any backend address, the
    /// three-way subnet split routes it to exactly one disposition, and
    /// the two non-mesh arms never over-fire. The invariant is that mesh
    /// membership (and ONLY mesh membership) zeroes both LB paths;
    /// host-equality routes to LOCAL; everything else routes to REMOTE.
    /// Strategy spans one representative per arm so shrinking always
    /// reports the minimal failing class.
    #[test]
    fn three_way_split_routes_each_address_class_to_exactly_one_disposition(
        backend_ip in prop_oneof![
            // mesh class — anywhere inside 10.99.0.0/16
            (0u8..=255, 0u8..=255).prop_map(|(c, d)| Ipv4Addr::new(10, 99, c, d)),
            // local class — exactly host_ipv4
            Just(host_ipv4()),
            // remote class — a routable address outside both the host
            // address and the mesh subnet
            (1u8..=95, 0u8..=255, 0u8..=255).prop_map(|(b, c, d)| Ipv4Addr::new(10, b, c, d)),
        ]
    ) {
        let (register_count, dataplane_count, programmed_fp) = reconcile_universe(backend_ip);

        let is_mesh = workload_subnet().contains(&backend_ip);
        let is_local = backend_ip == host_ipv4();

        if is_mesh {
            prop_assert_eq!(register_count, 0, "mesh: no RegisterLocalBackend");
            prop_assert_eq!(dataplane_count, 0, "mesh: no DataplaneUpdateService");
            prop_assert_eq!(programmed_fp, None, "mesh: not counted as dispatched");
        } else if is_local {
            prop_assert_eq!(register_count, 1, "local: exactly one RegisterLocalBackend");
            prop_assert_eq!(dataplane_count, 0, "local: no DataplaneUpdateService");
            prop_assert!(programmed_fp.is_some(), "local: dispatched");
        } else {
            prop_assert_eq!(register_count, 0, "remote: no RegisterLocalBackend");
            prop_assert_eq!(dataplane_count, 1, "remote: exactly one DataplaneUpdateService");
            prop_assert!(programmed_fp.is_some(), "remote: dispatched");
        }
    }
}

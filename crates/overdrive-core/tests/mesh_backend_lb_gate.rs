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
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
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

//! `ServiceBackendKeysAreDisjoint` — ADR-0276.
//!
//! **Always invariant**: no `BackendKey` (backend ip, port, proto) appears in
//! the backend sets of two different Services' service-map entries. Two
//! Services never share a backend endpoint.
//!
//! The dataplane enforces this fail-closed at install time (R-59: a
//! reverse-NAT key another Service's VIP already owns is rejected before
//! anything is written). The invariant pins the observable outcome across a
//! sequence of installs, including a colliding install: every accepted state
//! has pairwise-disjoint backend key sets across the Services' entries, and a
//! collision is refused with the stored entries unchanged.
//!
//! Wired into the `Invariant` enum as additive variant
//! `ServiceBackendKeysAreDisjoint`.

// Every literal in this file is a hand-picked constant the author can prove
// parses; `expect` is documentation, not error suppression.
#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;

use overdrive_core::SpiffeId;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::backend_key::{BackendKey, Proto};
use overdrive_core::id::{NodeId, ServiceVip};
use overdrive_core::traits::dataplane::{Backend, Dataplane, DataplaneError};

use crate::adapters::dataplane::SimDataplane;
use crate::harness::{InvariantResult, InvariantStatus};

const NAME: &str = "service-backend-keys-are-disjoint";
const SERVICES: u8 = 4;
const BACKENDS_PER_SERVICE: u8 = 3;
const LISTENER_PORT: u16 = 8080;

/// Install `SERVICES` Services over disjoint workload-address ranges, then
/// attempt a colliding install.
///
/// The colliding Service's backend set takes an endpoint another Service
/// holds. After every step the backend key sets of the registered
/// Services' entries must be pairwise disjoint; the colliding install must be
/// refused and leave every entry as it was.
pub async fn evaluate_service_backend_keys_are_disjoint() -> InvariantResult {
    let dataplane = SimDataplane::new();
    let mut frontends: Vec<ServiceFrontend> = Vec::new();

    for service in 0..SERVICES {
        let frontend = frontend(service);
        if let Err(e) = dataplane.update_service(frontend, backends(service)).await {
            return fail(format!("install of service {service} failed: {e}"));
        }
        frontends.push(frontend);
        if let Some(violation) = shared_key(&dataplane, &frontends) {
            return fail(format!("after installing service {service}: {violation}"));
        }
    }

    // A fifth Service asks for the first Service's endpoint beside its own.
    let intruder = frontend(SERVICES);
    let mut stolen = backends(SERVICES);
    stolen.push(backends(0).remove(0));
    let before: Vec<Option<Vec<Backend>>> = entries(&dataplane, &frontends);
    match dataplane.update_service(intruder, stolen).await {
        Err(DataplaneError::ReverseNatConflict { .. }) => {}
        Err(other) => {
            return fail(format!("a shared endpoint must be a reverse-NAT conflict, got {other}"));
        }
        Ok(()) => {
            frontends.push(intruder);
            return shared_key(&dataplane, &frontends).map_or_else(
                || fail("a shared endpoint was accepted".to_owned()),
                |violation| fail(format!("a shared endpoint was accepted: {violation}")),
            );
        }
    }
    if entries(&dataplane, &frontends) != before {
        return fail("a refused install changed the stored service entries".to_owned());
    }
    if let Some(violation) = shared_key(&dataplane, &frontends) {
        return fail(format!("after the refused install: {violation}"));
    }

    pass()
}

fn entries(dataplane: &SimDataplane, frontends: &[ServiceFrontend]) -> Vec<Option<Vec<Backend>>> {
    frontends
        .iter()
        .map(|f| dataplane.service_backends_for(f.vip_v4(), f.port().get(), f.proto()))
        .collect()
}

/// The first `BackendKey` held by two different Services' entries, if any.
fn shared_key(dataplane: &SimDataplane, frontends: &[ServiceFrontend]) -> Option<String> {
    let mut holder: BTreeMap<BackendKey, Ipv4Addr> = BTreeMap::new();
    for f in frontends {
        let stored = dataplane.service_backends_for(f.vip_v4(), f.port().get(), f.proto())?;
        for backend in stored {
            let SocketAddr::V4(addr) = backend.addr else {
                continue;
            };
            let key = BackendKey::new(*addr.ip(), addr.port(), f.proto());
            if let Some(other) = holder.insert(key, f.vip_v4())
                && other != f.vip_v4()
            {
                return Some(format!(
                    "backend key {key} is in the entries of {other} and {}",
                    f.vip_v4()
                ));
            }
        }
    }
    None
}

fn frontend(service: u8) -> ServiceFrontend {
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 96, 0, service + 1)))
        .expect("valid IPv4 ServiceVip");
    ServiceFrontend::new(
        vip,
        const { NonZeroU16::new(LISTENER_PORT).expect("non-zero port") },
        Proto::Tcp,
    )
    .expect("IPv4 ServiceFrontend constructs")
}

/// Service `service`'s backends: its own slice of the guest address range.
fn backends(service: u8) -> Vec<Backend> {
    (0..BACKENDS_PER_SERVICE)
        .map(|index| Backend {
            alloc: SpiffeId::new(&format!(
                "spiffe://overdrive.local/workload/svc-{service}/alloc/a-{index}"
            ))
            .expect("valid SPIFFE ID"),
            addr: SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(100, 95, service, index + 2)),
                LISTENER_PORT,
            ),
            weight: 1,
            healthy: true,
        })
        .collect()
}

fn pass() -> InvariantResult {
    InvariantResult {
        name: NAME.to_owned(),
        status: InvariantStatus::Pass,
        tick: 1,
        host: cluster_host(),
        cause: None,
    }
}

fn fail(cause: String) -> InvariantResult {
    InvariantResult {
        name: NAME.to_owned(),
        status: InvariantStatus::Fail,
        tick: 1,
        host: cluster_host(),
        cause: Some(cause),
    }
}

fn cluster_host() -> String {
    NodeId::new("cluster").map_or_else(|_| "cluster".to_owned(), |id| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The invariant holds over the production `SimDataplane`.
    #[tokio::test]
    async fn the_service_backend_keys_invariant_passes_over_the_sim_dataplane() {
        let result = evaluate_service_backend_keys_are_disjoint().await;
        assert_eq!(result.status, InvariantStatus::Pass, "{:?}", result.cause);
        assert_eq!(result.name, NAME);
    }
}

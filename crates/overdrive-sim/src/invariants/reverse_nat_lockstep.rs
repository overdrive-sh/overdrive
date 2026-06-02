//! `ReverseNatLockstep` — Slice 05 (US-05; S-2.2-20).
//!
//! **Always invariant**: every forward-path `SimDataplane.services[vip]`
//! entry has a matching `reverse_nat[BackendKey::from(backend)]` entry
//! mapping back to the original VIP. Removing a backend purges both
//! the forward-path entry and the `REVERSE_NAT` entry in lockstep.
//! No observation of the dataplane shows a forward-path service
//! backend whose `REVERSE_NAT` entry is missing — and no orphan
//! `REVERSE_NAT` entry is left after backend removal.
//!
//! This is the DST mirror of the production `EbpfDataplane`'s
//! `REVERSE_NAT_MAP` lockstep contract: the userspace-side
//! `EbpfDataplane::update_service` writes `REVERSE_NAT_MAP` entries
//! for new backends BEFORE the `SERVICE_MAP` swap, and purges stale
//! entries AFTER. Per `.claude/rules/development.md`
//! § *Production code is not shaped by simulation*, the `SimDataplane`
//! mirrors this with a single mutex acquisition guarding both maps —
//! observers cannot witness a forward-path entry without a matching
//! reverse-NAT entry.
//!
//! Wired into the existing `Invariant` enum's exhaustive match at
//! `crates/overdrive-sim/src/invariants/mod.rs` as additive variant
//! `ReverseNatLockstep`.

// SPIFFE / SocketAddr literals in this file are structurally
// total — every input is a hand-picked constant the test author
// can prove parses. `expect` here is documentation, not error
// suppression in an unbounded code path.
#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroU16;
use std::sync::Arc;

use overdrive_core::SpiffeId;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::backend_key::{BackendKey, Proto};
use overdrive_core::id::{NodeId, ServiceVip};
use overdrive_core::traits::dataplane::{Backend, Dataplane};

use crate::adapters::dataplane::SimDataplane;
use crate::harness::{InvariantResult, InvariantStatus};

/// Drive the lockstep scenario and return an `InvariantResult` pinned
/// to the canonical kebab-case name.
///
/// # Scenario
///
/// 1. Build a `SimDataplane` with N services (each VIP) and M backends
///    per service.
/// 2. After each `update_service` call, walk every (service, backend,
///    proto) triple. Assert
///    `reverse_nat[BackendKey { ip, port, proto }] == service.vip`
///    for every supported proto (TCP, UDP).
/// 3. Remove a backend (call `update_service` with one less); assert
///    the corresponding `REVERSE_NAT` entries are purged.
/// 4. Add the backend back via `update_service`; assert the
///    `REVERSE_NAT` entries reappear with the matching VIP.
///
/// The lockstep guarantee comes from `SimDataplane`'s implementation:
/// `services` and `reverse_nat` live inside one `Mutex<ServiceState>`,
/// and `update_service` acquires the mutex once for the entire write
/// (purge prior reverse-NAT, install new reverse-NAT, replace
/// forward-path). Observers cannot witness a partial update.
pub async fn evaluate_reverse_nat_lockstep() -> InvariantResult {
    const NAME: &str = "reverse-nat-lockstep";
    const SERVICES: u32 = 4;
    const BACKENDS_PER_SERVICE: u32 = 3;

    let dataplane = Arc::new(SimDataplane::new());

    // Build N services × M backends. Distinct VIPs and distinct
    // backend addresses so the (vip, backend) cross-product yields
    // unique reverse-NAT keys. Each service declares ONE proto
    // (alternating tcp/udp) — the per-proto fan-out (ADR-0060 D4)
    // installs exactly the declared-proto key, never both.
    let mut layout: BTreeMap<Ipv4Addr, (Proto, Vec<Backend>)> = BTreeMap::new();
    for s in 0..SERVICES {
        let vip = Ipv4Addr::new(10, 0, 0, u8::try_from(s + 1).expect("s < 256"));
        let proto = if s % 2 == 0 { Proto::Tcp } else { Proto::Udp };
        let mut backends = Vec::with_capacity(BACKENDS_PER_SERVICE as usize);
        for b in 0..BACKENDS_PER_SERVICE {
            let backend_ip = Ipv4Addr::new(
                10,
                1,
                u8::try_from(s + 1).expect("s < 256"),
                u8::try_from(b + 1).expect("b < 256"),
            );
            backends.push(backend(s, b, backend_ip, 8080));
        }
        layout.insert(vip, (proto, backends));
    }

    // Install every service. After each install, assert lockstep
    // holds for the cumulative set: every backend across every
    // service has a REVERSE_NAT entry, under its declared proto only,
    // pointing to its VIP.
    for (vip, (proto, backends)) in &layout {
        if let Err(e) = dataplane.update_service(frontend(*vip, *proto), backends.clone()).await {
            return fail(NAME, format!("install update_service({vip}) failed: {e}"));
        }
    }
    if let Some(violation) = check_lockstep(&dataplane, &layout) {
        return fail(NAME, format!("after initial install: {violation}"));
    }

    // Remove one backend from the first service via update_service
    // with one less. The reverse-NAT entries for the removed backend
    // MUST be purged.
    let (first_vip, first_proto, first_backends) =
        layout.iter().next().map(|(v, (p, b))| (*v, *p, b.clone())).expect("SERVICES > 0");
    let removed_backend = first_backends.last().cloned().expect("BACKENDS_PER_SERVICE > 0");
    let mut shrunk = first_backends.clone();
    shrunk.pop();

    if let Err(e) = dataplane.update_service(frontend(first_vip, first_proto), shrunk.clone()).await
    {
        return fail(NAME, format!("shrink update_service({first_vip}) failed: {e}"));
    }

    // Update the layout to match the post-shrink reality.
    let mut after_shrink = layout.clone();
    after_shrink.insert(first_vip, (first_proto, shrunk.clone()));
    if let Some(violation) = check_lockstep(&dataplane, &after_shrink) {
        return fail(NAME, format!("after shrink: {violation}"));
    }

    // The removed backend's REVERSE_NAT entry (under the service's
    // declared proto) must be ABSENT.
    let removed_key = backend_key_for(&removed_backend, first_proto);
    if let Some(stale_vip) = dataplane.reverse_nat_lookup(removed_key) {
        return fail(
            NAME,
            format!(
                "removed backend {removed_key} still has REVERSE_NAT entry → \
                 {stale_vip}; expected purged"
            ),
        );
    }

    // Add the backend back via update_service. The REVERSE_NAT
    // entries must reappear with the correct VIP.
    if let Err(e) =
        dataplane.update_service(frontend(first_vip, first_proto), first_backends.clone()).await
    {
        return fail(NAME, format!("restore update_service({first_vip}) failed: {e}"));
    }
    if let Some(violation) = check_lockstep(&dataplane, &layout) {
        return fail(NAME, format!("after restore: {violation}"));
    }

    pass(NAME)
}

/// Build a `ServiceFrontend` for a VIP under a declared proto. Listener
/// port is fixed at 8080 (the lockstep does not vary port).
fn frontend(vip: Ipv4Addr, proto: Proto) -> ServiceFrontend {
    let service_vip = ServiceVip::new(IpAddr::V4(vip)).expect("valid IPv4 ServiceVip");
    ServiceFrontend::new(
        service_vip,
        const { NonZeroU16::new(8080).expect("non-zero port") },
        proto,
    )
    .expect("IPv4 ServiceFrontend constructs")
}

/// Walk every (service, backend, proto) triple in `layout` and assert
/// that the dataplane's `reverse_nat` map carries the expected entry,
/// AND that no orphan reverse-NAT entries exist (every entry maps
/// back to a backend in the live forward-path layout). Returns
/// `None` on success, `Some(reason)` on first violation.
fn check_lockstep(
    dataplane: &SimDataplane,
    layout: &BTreeMap<Ipv4Addr, (Proto, Vec<Backend>)>,
) -> Option<String> {
    // Build the expected reverse-NAT map from the layout — exactly one
    // key per backend, under the service's DECLARED proto (D4). A key
    // for any other proto is a phantom and fails the orphan check below.
    let mut expected: BTreeMap<BackendKey, Ipv4Addr> = BTreeMap::new();
    for (vip, (proto, backends)) in layout {
        for backend in backends {
            expected.insert(backend_key_for(backend, *proto), *vip);
        }
    }

    // Forward direction: every expected entry must exist.
    for (key, expected_vip) in &expected {
        match dataplane.reverse_nat_lookup(*key) {
            Some(actual_vip) if actual_vip == *expected_vip => {}
            Some(actual_vip) => {
                return Some(format!("reverse_nat[{key}] = {actual_vip}; expected {expected_vip}"));
            }
            None => {
                return Some(format!("reverse_nat[{key}] missing; expected {expected_vip}"));
            }
        }
    }

    // Reverse direction: no orphan entries — every entry in the
    // sim's reverse_nat map must correspond to a (backend, proto)
    // currently in the layout. This catches stale-entry leaks after
    // backend removal.
    for (key, vip) in dataplane.reverse_nat_entries() {
        if !expected.contains_key(&key) {
            return Some(format!(
                "orphan reverse_nat entry {key} → {vip}; not present in current layout"
            ));
        }
    }

    None
}

fn backend(service_idx: u32, backend_idx: u32, ip: Ipv4Addr, port: u16) -> Backend {
    Backend {
        alloc: SpiffeId::new(&format!(
            "spiffe://overdrive.local/job/svc-{service_idx}/alloc/b-{backend_idx}"
        ))
        .expect("valid SPIFFE ID"),
        addr: std::net::SocketAddr::new(std::net::IpAddr::V4(ip), port),
        weight: 1,
        healthy: true,
    }
}

/// Project a `Backend`'s IPv4 address + port onto a `BackendKey` for
/// the given `proto`. Mirrors the `reverse_nat_keys_for` helper in
/// `crate::adapters::dataplane`.
fn backend_key_for(backend: &Backend, proto: Proto) -> BackendKey {
    let ipv4 = match backend.addr.ip() {
        std::net::IpAddr::V4(v4) => v4,
        std::net::IpAddr::V6(_) => {
            unreachable!("evaluator builds IPv4 backends only");
        }
    };
    BackendKey::new(ipv4, backend.addr.port(), proto)
}

fn pass(name: &str) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Pass,
        tick: 1,
        host: cluster_host(),
        cause: None,
    }
}

fn fail(name: &str, cause: String) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Fail,
        tick: 1,
        host: cluster_host(),
        cause: Some(cause),
    }
}

fn cluster_host() -> String {
    NodeId::new("cluster").map_or_else(|_| "cluster".to_owned(), |id| id.to_string())
}

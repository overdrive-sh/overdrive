//! Tier-1 (DST / in-memory) regression: the Sim forward-path map keys on
//! the FULL frontend identity `(vip, port, proto)`, matching the host
//! `EbpfDataplane` `SERVICE_MAP` `ServiceKey { vip_host, port_host, proto }`
//! and the `ServiceFrontend` the `Dataplane` trait takes.
//!
//! Guards the sim-vs-host divergence surfaced while adjudicating
//! ADR-0060 D4: the sim previously keyed `services` on `(vip, proto)`,
//! dropping the listener port. Two listeners on one VIP differing only by
//! port but sharing proto (e.g. tcp/80 + tcp/443), installed by separate
//! per-listener `update_service` calls, collapsed into a single sim slot —
//! the second install silently evicted the first from BOTH the forward and
//! reverse-NAT maps — while the host kept them distinct. The existing
//! per-proto guards never caught it: they only exercise different-proto
//! co-location (tcp/80 + udp/5353), whose backend-port-bearing reverse-NAT
//! keys never collide.
//!
//! This is the symmetric sibling of the host unit test
//! `empty_backend_purge_is_scoped_to_frontend_not_vip_wide`
//! (`crates/overdrive-dataplane/src/lib.rs`).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;

use overdrive_core::SpiffeId;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::backend_key::{BackendKey, Proto};
use overdrive_core::id::ServiceVip;
use overdrive_core::traits::dataplane::{Backend, Dataplane};
use overdrive_sim::adapters::dataplane::SimDataplane;

const VIP: Ipv4Addr = Ipv4Addr::new(10, 96, 0, 10);
const BACKEND_IP: Ipv4Addr = Ipv4Addr::new(10, 1, 0, 1);

fn frontend(port: u16, proto: Proto) -> ServiceFrontend {
    let vip = ServiceVip::new(IpAddr::V4(VIP)).expect("valid IPv4 ServiceVip");
    ServiceFrontend::new(vip, NonZeroU16::new(port).expect("non-zero port"), proto)
        .expect("IPv4 ServiceFrontend constructs")
}

fn backend(port: u16) -> Backend {
    Backend {
        alloc: SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/b-0").expect("valid SPIFFE"),
        addr: SocketAddr::new(IpAddr::V4(BACKEND_IP), port),
        weight: 1,
        healthy: true,
    }
}

/// Two TCP listeners on one VIP differing only by port (tcp/80 + tcp/443),
/// installed by separate per-listener `update_service` calls, must be
/// INDEPENDENT entries — installing the second must not evict the first
/// from the reverse-NAT map. Under the old `(vip, proto)` forward key,
/// installing tcp/443 overwrote tcp/80's forward slot and purged its
/// reverse-NAT key.
#[tokio::test]
async fn same_proto_different_port_frontends_are_independent() {
    let dp = SimDataplane::new();

    dp.update_service(frontend(80, Proto::Tcp), vec![backend(80)]).await.unwrap();
    dp.update_service(frontend(443, Proto::Tcp), vec![backend(443)]).await.unwrap();

    assert_eq!(
        dp.reverse_nat_lookup(BackendKey::new(BACKEND_IP, 80, Proto::Tcp)),
        Some(VIP),
        "tcp/80's reverse-NAT key must survive installing the co-resident tcp/443 frontend"
    );
    assert_eq!(
        dp.reverse_nat_lookup(BackendKey::new(BACKEND_IP, 443, Proto::Tcp)),
        Some(VIP),
        "tcp/443's reverse-NAT key must be present"
    );

    // Forward map: each frontend's backend set is independently
    // addressable by its full `(vip, port, proto)` identity — the
    // surface that collapsed under the old `(vip, proto)` key.
    assert_eq!(
        dp.service_backends_for(VIP, 80, Proto::Tcp).map(|bs| bs.len()),
        Some(1),
        "tcp/80 forward slot must survive installing the co-resident tcp/443 frontend"
    );
    assert_eq!(
        dp.service_backends_for(VIP, 443, Proto::Tcp).map(|bs| bs.len()),
        Some(1),
        "tcp/443 forward slot must be present"
    );

    // Frontend-scoped purge: removing tcp/80 leaves the sibling tcp/443
    // intact across BOTH maps (the host's contract, ADR-0060 D4).
    dp.update_service(frontend(80, Proto::Tcp), vec![]).await.unwrap();
    assert!(
        dp.service_backends_for(VIP, 80, Proto::Tcp).is_none(),
        "tcp/80 forward slot must be purged"
    );
    assert_eq!(
        dp.service_backends_for(VIP, 443, Proto::Tcp).map(|bs| bs.len()),
        Some(1),
        "tcp/443 must survive a sibling frontend's empty-backend purge"
    );
    assert!(
        dp.reverse_nat_lookup(BackendKey::new(BACKEND_IP, 80, Proto::Tcp)).is_none(),
        "tcp/80's reverse-NAT key must be purged"
    );
    assert_eq!(
        dp.reverse_nat_lookup(BackendKey::new(BACKEND_IP, 443, Proto::Tcp)),
        Some(VIP),
        "tcp/443's reverse-NAT key must survive tcp/80's purge"
    );
}

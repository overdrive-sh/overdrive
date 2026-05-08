//! Regression test: `REVERSE_NAT_MAP` purge must not delete entries
//! still referenced by another active service.
//!
//! When two services share a backend address and one service removes
//! it, the reverse-NAT entry must survive for the other service.
//! Without the cross-service union check, the purge loop deletes the
//! entry solely because it left *this* service's backend set —
//! breaking the surviving service's egress path.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::Ipv4Addr;

use overdrive_core::SpiffeId;
use overdrive_core::dataplane::backend_key::{BackendKey, Proto};
use overdrive_core::traits::dataplane::{Backend, Dataplane};
use overdrive_sim::adapters::dataplane::SimDataplane;

fn spiffe(path: &str) -> SpiffeId {
    SpiffeId::new(&format!("spiffe://overdrive.local{path}")).expect("valid SPIFFE URI")
}

fn backend(ip: &str, port: u16) -> Backend {
    Backend {
        alloc: spiffe(&format!("/job/svc/alloc/{ip}-{port}")),
        addr: format!("{ip}:{port}").parse().expect("valid socket"),
        weight: 100,
        healthy: true,
    }
}

#[tokio::test]
async fn shared_backend_reverse_nat_survives_single_service_removal() {
    let dp = SimDataplane::new();
    let shared = backend("10.1.0.5", 9000);

    let vip_a = Ipv4Addr::new(10, 0, 0, 1);
    let vip_b = Ipv4Addr::new(10, 0, 0, 2);

    // Both services register with the same backend.
    dp.update_service(vip_a, vec![shared.clone()]).await.unwrap();
    dp.update_service(vip_b, vec![shared.clone()]).await.unwrap();

    let key_tcp = BackendKey::new("10.1.0.5".parse().unwrap(), 9000, Proto::Tcp);
    assert!(dp.reverse_nat_lookup(key_tcp).is_some(), "precondition: entry exists");

    // Service A removes the backend (empty update).
    dp.update_service(vip_a, vec![]).await.unwrap();

    // Service B still routes through this backend — entry MUST survive.
    assert_eq!(
        dp.reverse_nat_lookup(key_tcp),
        Some(vip_b),
        "reverse-NAT entry for shared backend was deleted when only one service removed it",
    );
}

#[tokio::test]
async fn shared_backend_reverse_nat_survives_backend_set_change() {
    let dp = SimDataplane::new();
    let shared = backend("10.1.0.5", 9000);
    let replacement = backend("10.2.0.1", 8080);

    let vip_a = Ipv4Addr::new(10, 0, 0, 1);
    let vip_b = Ipv4Addr::new(10, 0, 0, 2);

    dp.update_service(vip_a, vec![shared.clone()]).await.unwrap();
    dp.update_service(vip_b, vec![shared.clone()]).await.unwrap();

    let key_tcp = BackendKey::new("10.1.0.5".parse().unwrap(), 9000, Proto::Tcp);

    // Service A swaps to a different backend (non-empty update that
    // drops the shared backend from its set).
    dp.update_service(vip_a, vec![replacement]).await.unwrap();

    // Service B still routes through the shared backend.
    assert_eq!(
        dp.reverse_nat_lookup(key_tcp),
        Some(vip_b),
        "reverse-NAT entry for shared backend was deleted during non-empty update",
    );
}

#[tokio::test]
async fn unshared_backend_reverse_nat_deleted_on_removal() {
    let dp = SimDataplane::new();
    let only_a = backend("10.1.0.5", 9000);
    let only_b = backend("10.2.0.1", 8080);

    let vip_a = Ipv4Addr::new(10, 0, 0, 1);
    let vip_b = Ipv4Addr::new(10, 0, 0, 2);

    dp.update_service(vip_a, vec![only_a]).await.unwrap();
    dp.update_service(vip_b, vec![only_b]).await.unwrap();

    let key_a = BackendKey::new("10.1.0.5".parse().unwrap(), 9000, Proto::Tcp);

    // Service A removes its backend — no other service shares it.
    dp.update_service(vip_a, vec![]).await.unwrap();

    // The entry SHOULD be gone.
    assert_eq!(
        dp.reverse_nat_lookup(key_a),
        None,
        "reverse-NAT entry for unshared backend should be purged",
    );

    // Service B's entry is unaffected.
    let key_b = BackendKey::new("10.2.0.1".parse().unwrap(), 8080, Proto::Tcp);
    assert!(dp.reverse_nat_lookup(key_b).is_some(), "unrelated service's entry must survive");
}

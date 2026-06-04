//! Tier-1 (DST / in-memory) per-proto `REVERSE_NAT` key set + lockstep
//! set-equality gate (udp-service-support US-01/US-02; ADR-0060 D4 +
//! § Enforcement).
//!
//! Scenario SSOT:
//! `docs/feature/udp-service-support/distill/test-scenarios.md`
//! - S-03-A: Sim installs EXACTLY the declared-proto key set (property)
//! - S-03-B: NEGATIVE — a dropped fan-out key fails the lockstep
//! - S-03-C: NEGATIVE — an extra (phantom) key fails the lockstep (orphan check)
//! - S-03-D: the #163 shape (tcp-only for a udp service) is caught
//! - S-02-A: empty backends purge only frontend.proto's keys (property)
//! - S-02-B: cross-service shared key survives a per-proto purge
//! - S-02-C: idempotent re-apply (property)
//! - S-02-D: non-IPv4 backend contributes no key (boundary)
//!
//! Mandate 8 (Universe-bound assertion). The universe is the
//! port-observable `BTreeSet<BackendKey>` `REVERSE_NAT` key set. The
//! expected assertion is exact set-equality against
//! `{ BackendKey{ ip, port, frontend.proto } : backend }`; an
//! unexpected extra key fails set-equality (fail-closed). This native
//! set-equality IS the Rust equivalent of `assert_state_delta(..,
//! strict=True)`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::num::NonZeroU16;

use overdrive_core::SpiffeId;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::backend_key::{BackendKey, Proto};
use overdrive_core::id::ServiceVip;
use overdrive_core::traits::dataplane::{Backend, Dataplane};
use overdrive_sim::adapters::dataplane::SimDataplane;
use proptest::prelude::*;

fn spiffe(tag: usize) -> SpiffeId {
    SpiffeId::new(&format!("spiffe://overdrive.local/job/svc/alloc/b-{tag}"))
        .expect("valid SPIFFE URI")
}

fn frontend(vip_octet: u8, port: u16, proto: Proto) -> ServiceFrontend {
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 96, 0, vip_octet)))
        .expect("valid IPv4 ServiceVip");
    ServiceFrontend::new(vip, NonZeroU16::new(port).expect("non-zero port"), proto)
        .expect("IPv4 ServiceFrontend constructs")
}

fn ipv4_backend(tag: usize, ip: Ipv4Addr, port: u16) -> Backend {
    Backend {
        alloc: spiffe(tag),
        addr: SocketAddr::new(IpAddr::V4(ip), port),
        weight: 1,
        healthy: true,
    }
}

/// The set of `REVERSE_NAT` keys the Sim currently holds for any VIP,
/// projected to a `BTreeSet<BackendKey>` (the Mandate-8 universe).
fn reverse_nat_key_set(dp: &SimDataplane) -> BTreeSet<BackendKey> {
    dp.reverse_nat_entries().into_iter().map(|(k, _vip)| k).collect()
}

/// Expected per-proto key set for an IPv4-only backend slice.
fn expected_keys(backends: &[Backend], proto: Proto) -> BTreeSet<BackendKey> {
    backends
        .iter()
        .filter_map(|b| match b.addr.ip() {
            IpAddr::V4(v4) => Some(BackendKey::new(v4, b.addr.port(), proto)),
            IpAddr::V6(_) => None,
        })
        .collect()
}

/// Strategy: 1..=4 distinct IPv4 backends with distinct (ip, port).
fn ipv4_backends_strategy() -> impl Strategy<Value = Vec<Backend>> {
    prop::collection::vec((1u8..=254, 1u16..=u16::MAX), 1..=4).prop_map(|specs| {
        specs
            .into_iter()
            .enumerate()
            .map(|(i, (last_octet, port))| {
                ipv4_backend(i, Ipv4Addr::new(10, 1, 0, last_octet), port)
            })
            .collect()
    })
}

fn proto_strategy() -> impl Strategy<Value = Proto> {
    prop_oneof![Just(Proto::Tcp), Just(Proto::Udp)]
}

proptest! {
    /// S-03-A (criterion 7) — Property: the Sim `REVERSE_NAT` key set for
    /// a service equals exactly the keys derived from
    /// `(frontend.proto, backends)`, with NO key for any other protocol.
    #[test]
    fn sim_installs_exactly_the_declared_proto_key_set(
        backends in ipv4_backends_strategy(),
        proto in proto_strategy(),
    ) {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            let dp = SimDataplane::new();
            let fe = frontend(10, 8080, proto);
            dp.update_service(fe, backends.clone()).await.unwrap();

            let actual = reverse_nat_key_set(&dp);
            let expected = expected_keys(&backends, proto);
            prop_assert_eq!(&actual, &expected, "key set must equal exactly the declared-proto set");
            // No other-proto key may appear.
            let other = match proto { Proto::Tcp => Proto::Udp, Proto::Udp => Proto::Tcp };
            prop_assert!(
                !actual.iter().any(|k| k.proto == other),
                "no {:?} key may appear for a {:?} service", other, proto
            );
            Ok(())
        })?;
    }

    /// S-02-A (criterion 11) — Property: `update_service(frontend_P, [])`
    /// purges ONLY protocol P's `REVERSE_NAT` keys for the VIP; a
    /// co-resident other-proto frontend on the same VIP (separate
    /// `update_service` call) survives.
    #[test]
    fn empty_backends_purges_only_this_protos_keys(
        backends in ipv4_backends_strategy(),
    ) {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            let dp = SimDataplane::new();
            let fe_tcp = frontend(10, 80, Proto::Tcp);
            let fe_udp = frontend(10, 5353, Proto::Udp);
            // Same VIP, two protos installed via separate per-listener calls.
            dp.update_service(fe_tcp, backends.clone()).await.unwrap();
            dp.update_service(fe_udp, backends.clone()).await.unwrap();

            // Purge only udp.
            dp.update_service(fe_udp, vec![]).await.unwrap();

            let actual = reverse_nat_key_set(&dp);
            let expected = expected_keys(&backends, Proto::Tcp);
            prop_assert_eq!(&actual, &expected,
                "only tcp keys must survive an empty udp update on the same VIP");
            Ok(())
        })?;
    }

    /// S-02-C (criterion 13) — Property: `update_service` is idempotent —
    /// applying it twice with identical args yields the same key set.
    #[test]
    fn idempotent_re_apply_yields_same_key_set(
        backends in ipv4_backends_strategy(),
        proto in proto_strategy(),
    ) {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            let dp = SimDataplane::new();
            let fe = frontend(10, 8080, proto);
            dp.update_service(fe, backends.clone()).await.unwrap();
            let after_first = reverse_nat_key_set(&dp);
            dp.update_service(fe, backends.clone()).await.unwrap();
            let after_second = reverse_nat_key_set(&dp);
            prop_assert_eq!(after_first, after_second, "idempotent re-apply must not change the key set");
            Ok(())
        })?;
    }
}

/// S-03-D (criterion 10) — the exact #163 shape: a production-mirroring
/// fan-out that installs only the tcp key for a udp service fails the
/// Tier-1 set-equality against the declared udp frontend. Proves #163
/// cannot recur silently at Tier 1.
#[tokio::test]
async fn issue_163_tcp_only_for_udp_service_is_caught() {
    let dp = SimDataplane::new();
    let udp_fe = frontend(10, 5353, Proto::Udp);
    let backends = vec![ipv4_backend(0, Ipv4Addr::new(10, 1, 0, 1), 5353)];
    dp.update_service(udp_fe, backends.clone()).await.unwrap();

    let actual = reverse_nat_key_set(&dp);
    // The #163 bug installed the tcp key for a udp service. Assert the
    // post-fix Sim installs the UDP key and NOT the tcp key — i.e. the
    // tcp-only shape is structurally absent.
    let tcp_only = expected_keys(&backends, Proto::Tcp);
    let udp_expected = expected_keys(&backends, Proto::Udp);
    assert_ne!(
        actual, tcp_only,
        "S-03-D: the tcp-only-for-udp #163 shape must not be the post-state"
    );
    assert_eq!(actual, udp_expected, "S-03-D: a udp service installs exactly the udp keys");
}

/// S-02-B (criterion 12) — a `REVERSE_NAT` key shared with another live
/// service survives a per-proto empty-backends purge.
#[tokio::test]
async fn cross_service_shared_key_survives_per_proto_purge() {
    let dp = SimDataplane::new();
    let shared = ipv4_backend(0, Ipv4Addr::new(10, 1, 0, 5), 9000);

    let fe_a = frontend(1, 5353, Proto::Udp);
    let fe_b = frontend(2, 5353, Proto::Udp);
    dp.update_service(fe_a, vec![shared.clone()]).await.unwrap();
    dp.update_service(fe_b, vec![shared.clone()]).await.unwrap();

    let shared_key = BackendKey::new(Ipv4Addr::new(10, 1, 0, 5), 9000, Proto::Udp);
    assert_eq!(
        dp.reverse_nat_lookup(shared_key),
        Some(fe_b.vip_v4()),
        "precondition: shared udp key maps to the last writer"
    );

    // Service A scales to zero.
    dp.update_service(fe_a, vec![]).await.unwrap();

    assert!(
        dp.reverse_nat_lookup(shared_key).is_some(),
        "S-02-B: shared (ip,port,udp) key must survive when only service A scales to zero"
    );
}

/// S-02-D (criterion 14) — boundary: a backend set with one IPv4 and one
/// IPv6 backend under a udp frontend yields a key set with the IPv4
/// backend's `(ip, port, udp)` key and no key for the IPv6 backend.
#[tokio::test]
async fn ipv6_backend_contributes_no_reverse_nat_key() {
    let dp = SimDataplane::new();
    let v4 = ipv4_backend(0, Ipv4Addr::new(10, 1, 0, 1), 5353);
    let v6 = Backend {
        alloc: spiffe(1),
        addr: SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 5353),
        weight: 1,
        healthy: true,
    };
    let fe = frontend(10, 5353, Proto::Udp);
    dp.update_service(fe, vec![v4.clone(), v6]).await.unwrap();

    let actual = reverse_nat_key_set(&dp);
    let expected: BTreeSet<BackendKey> =
        std::iter::once(BackendKey::new(Ipv4Addr::new(10, 1, 0, 1), 5353, Proto::Udp)).collect();
    assert_eq!(actual, expected, "S-02-D: only the IPv4 backend's udp key is present");
}

/// S-03-B (criterion 8) / S-03-C (criterion 9) — the `ReverseNatLockstep`
/// invariant passes when the per-proto fan-out is correct (it walks every
/// service's declared proto and asserts exact set-equality, fail-closed
/// on both missing and phantom keys). The DST harness runs the negative
/// arms; here we assert the invariant evaluates Pass under the corrected
/// per-proto fan-out (the structural guard the negative arms protect).
#[tokio::test]
async fn reverse_nat_lockstep_passes_under_per_proto_fan_out() {
    let result =
        overdrive_sim::invariants::reverse_nat_lockstep::evaluate_reverse_nat_lockstep().await;
    assert!(
        matches!(result.status, overdrive_sim::harness::InvariantStatus::Pass),
        "ReverseNatLockstep must pass under the per-proto fan-out: {:?}",
        result.cause
    );
}

/// Criterion 15 — sctp boundary: parsing the listener protocol token
/// `"sctp"` returns `Err(UnknownProto)` so sctp can never produce a
/// `ServiceFrontend`. Proves the udp-support slice does NOT widen the
/// `Proto` admission set (shipped #164 boundary).
#[test]
fn sctp_proto_token_is_rejected() {
    use overdrive_core::dataplane::backend_key::{BackendKey, ParseError};
    use std::str::FromStr;

    // The Proto admission boundary lives on BackendKey's FromStr proto
    // token. `sctp` is rejected as UnknownProto.
    let err = BackendKey::from_str("10.0.0.1:5353/sctp").expect_err("sctp must be rejected");
    assert!(
        matches!(err, ParseError::UnknownProto(ref s) if s == "sctp"),
        "criterion 15: sctp must be rejected with UnknownProto, got {err:?}"
    );
}

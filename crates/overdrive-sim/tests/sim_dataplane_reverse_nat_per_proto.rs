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
//! - S-02-C: idempotent re-apply (property)
//! - S-02-D: a set with an IPv6 backend is refused, nothing written (boundary;
//!   D-20, reconciler-convergence-guarantees)
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
use overdrive_core::traits::dataplane::{Backend, Dataplane, DataplaneError};
use overdrive_sim::adapters::dataplane::SimDataplane;
use proptest::prelude::*;

use overdrive_testing::overdrive_test_default;

overdrive_test_default!(host_pure);

fn spiffe(tag: usize) -> SpiffeId {
    SpiffeId::new(&format!("spiffe://overdrive.local/workload/svc/alloc/b-{tag}"))
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
        workload_identity: spiffe(tag),
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

/// Expected per-proto key set for a backend slice. Every backend the Sim
/// accepts is IPv4: a set with an IPv6 backend is refused whole, so it never
/// reaches this oracle.
fn expected_keys(backends: &[Backend], proto: Proto) -> BTreeSet<BackendKey> {
    backends
        .iter()
        .map(|b| match b.addr.ip() {
            IpAddr::V4(v4) => BackendKey::new(v4, b.addr.port(), proto),
            IpAddr::V6(_) => unreachable!("an IPv6 backend set is refused, never installed"),
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

/// S-02-D (criterion 14) — boundary: `update_service` refuses a non-empty
/// backend set that contains an IPv6 backend with `LoadFailed`, before any
/// write. No `REVERSE_NAT` key is added (none for the IPv6 backend, none for
/// the IPv4 backend sent with it) and the service's prior state is unchanged
/// (D-20, ADR-0324; the host adapter already refuses).
///
/// Pending until DELIVER changes the Sim to refuse: today it accepts the set
/// and installs the IPv4 backend's key.
#[tokio::test]
async fn a_set_with_an_ipv6_backend_is_refused_and_leaves_the_service_unchanged() {
    let dp = SimDataplane::new();
    let fe = frontend(10, 5353, Proto::Udp);
    let prior = vec![ipv4_backend(0, Ipv4Addr::new(10, 1, 0, 1), 5353)];
    dp.update_service(fe, prior.clone()).await.unwrap();
    let keys_before = reverse_nat_key_set(&dp);
    let service_before = dp.service_backends_for(Ipv4Addr::new(10, 96, 0, 10), 5353, Proto::Udp);
    assert_eq!(keys_before, expected_keys(&prior, Proto::Udp), "the prior service is installed");

    // A different IPv4 backend travels with the IPv6 one, so a partial write
    // would show as a new key.
    let v4 = ipv4_backend(1, Ipv4Addr::new(10, 1, 0, 2), 5353);
    let v6 = Backend {
        workload_identity: spiffe(2),
        addr: SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 5353),
        weight: 1,
        healthy: true,
    };
    let outcome = dp.update_service(fe, vec![v4, v6]).await;

    assert!(
        matches!(outcome, Err(DataplaneError::LoadFailed(_))),
        "update_service accepted a set containing an IPv6 backend; it must refuse it with \
         LoadFailed, got {outcome:?}"
    );
    assert_eq!(
        reverse_nat_key_set(&dp),
        keys_before,
        "a refused update adds no REVERSE_NAT key and removes none"
    );
    assert_eq!(
        dp.service_backends_for(Ipv4Addr::new(10, 96, 0, 10), 5353, Proto::Udp),
        service_before,
        "a refused update leaves the service's backends unchanged"
    );
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

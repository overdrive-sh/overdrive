//! `ProgrammedServiceView::programmed_fingerprint` contract on `SimDataplane`
//! (D-20, ADR-0299; DISTILL obligation 2). One test per rustdoc clause of the
//! port. The host adapter is covered by the host/sim equivalence test in
//! `crates/overdrive-dataplane/tests/integration/programmed_service_view_equivalence.rs`.
//!
//! Layer: in-memory (the sim). The universe is the one port-exposed read,
//! `programmed_fingerprint(frontend)`, plus the `update_service` outcome. Each
//! test first asserts the nontrivial positive read, so an inert read
//! (`Ok(None)` for every frontend) fails at a named assertion instead of
//! passing vacuously.
//!
//! Pending until DELIVER implements the sim read: each test is
//! `#[should_panic(expected = ..)]` on the assertion it makes today.
//!
//! `CONTRACT_SHAPE`: pure-function (the read mutates nothing).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::num::NonZeroU16;

use overdrive_core::SpiffeId;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::dataplane::fingerprint::{BackendSetFingerprint, fingerprint};
use overdrive_core::id::ServiceVip;
use overdrive_core::traits::ProgrammedServiceView;
use overdrive_core::traits::dataplane::{Backend, Dataplane, DataplaneError};
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_testing::overdrive_test_default;
use proptest::prelude::*;

overdrive_test_default!(host_pure);

fn vip(octet: u8) -> ServiceVip {
    ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 96, 0, octet))).expect("IPv4 service VIP")
}

fn frontend(octet: u8, port: u16, proto: Proto) -> ServiceFrontend {
    ServiceFrontend::new(vip(octet), NonZeroU16::new(port).expect("non-zero port"), proto)
        .expect("IPv4 frontend")
}

fn backend(tag: usize, ip: IpAddr, weight: u16, healthy: bool) -> Backend {
    Backend {
        alloc: SpiffeId::new(&format!("spiffe://overdrive.local/workload/svc/alloc/b-{tag}"))
            .expect("SPIFFE id"),
        addr: SocketAddr::new(ip, 8080),
        weight,
        healthy,
    }
}

fn v4(tag: usize, last: u8) -> Backend {
    backend(tag, IpAddr::V4(Ipv4Addr::new(10, 1, 0, last)), 1, true)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().build().expect("tokio runtime")
}

/// Assert the read equals `expected`, naming `after` in the failure so each
/// clause's RED text is its own.
async fn assert_programmed(
    dp: &SimDataplane,
    frontend: &ServiceFrontend,
    expected: Option<BackendSetFingerprint>,
    after: &str,
) {
    let read = dp.programmed_fingerprint(frontend).await.expect("the read succeeds");
    assert_eq!(read, expected, "programmed_fingerprint after {after}");
}

/// Backend sets over the full domain the fingerprint covers: one to four IPv4
/// backends, any weight, either health flag, in a non-canonical order (the
/// fingerprint is computed over the slice in the order it is given, so an
/// implementation that sorts before fingerprinting must fail).
fn backend_sets() -> impl Strategy<Value = Vec<Backend>> {
    prop::collection::btree_set(1u8..=254, 1..=4)
        .prop_flat_map(|lasts| {
            let n = lasts.len();
            (
                Just(lasts.into_iter().collect::<Vec<_>>()).prop_shuffle(),
                prop::collection::vec((1u16..=1000, any::<bool>()), n),
            )
        })
        .prop_map(|(lasts, shape)| {
            lasts
                .into_iter()
                .zip(shape)
                .enumerate()
                .map(|(tag, (last, (weight, healthy)))| {
                    backend(tag, IpAddr::V4(Ipv4Addr::new(10, 1, 0, last)), weight, healthy)
                })
                .collect()
        })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, failure_persistence: None, ..ProptestConfig::default() })]

    /// Invariants 1 and 4: after a successful non-empty update the read equals
    /// the fingerprint of the applied slice, in the order it was given, whatever
    /// the weights, health flags and order; a second read with no write between
    /// is equal to the first.
    #[test]
    #[should_panic(expected = "programmed_fingerprint after a successful update_service")]
    fn read_after_update_equals_the_fingerprint_of_the_applied_backends(set in backend_sets()) {
        runtime().block_on(async {
            let dp = SimDataplane::new();
            let fe = frontend(10, 8080, Proto::Tcp);
            dp.update_service(fe, set.clone()).await.expect("update");
            assert_programmed(
                &dp,
                &fe,
                Some(fingerprint(&fe.vip(), &set)),
                "a successful update_service",
            )
            .await;
            assert_programmed(
                &dp,
                &fe,
                Some(fingerprint(&fe.vip(), &set)),
                "a second read with no write between",
            )
            .await;
        });
    }
}

/// Invariant 2: after `update_service(f, [])` the read is `None`.
#[test]
#[should_panic(expected = "programmed_fingerprint after the first apply (withdraw)")]
fn a_withdrawn_frontend_reads_none() {
    runtime().block_on(async {
        let dp = SimDataplane::new();
        let fe = frontend(10, 8080, Proto::Tcp);
        let set = vec![v4(0, 5)];
        dp.update_service(fe, set.clone()).await.expect("apply");
        assert_programmed(
            &dp,
            &fe,
            Some(fingerprint(&fe.vip(), &set)),
            "the first apply (withdraw)",
        )
        .await;
        dp.update_service(fe, vec![]).await.expect("withdraw");
        assert_programmed(&dp, &fe, None, "the withdrawal").await;
    });
}

/// Edge case: a frontend never programmed reads `Ok(None)`, not an error,
/// while a neighbour on another VIP is programmed.
#[test]
#[should_panic(expected = "programmed_fingerprint after programming the neighbour")]
fn a_never_programmed_frontend_reads_ok_none() {
    runtime().block_on(async {
        let dp = SimDataplane::new();
        let neighbour = frontend(10, 8080, Proto::Tcp);
        let set = vec![v4(0, 5)];
        dp.update_service(neighbour, set.clone()).await.expect("apply");
        assert_programmed(
            &dp,
            &neighbour,
            Some(fingerprint(&neighbour.vip(), &set)),
            "programming the neighbour",
        )
        .await;
        assert_programmed(&dp, &frontend(11, 8080, Proto::Tcp), None, "never programming it").await;
    });
}

/// Edge case: other ports and protocols on one VIP are independent keys; a
/// withdrawal of one leaves the others readable.
#[test]
#[should_panic(expected = "programmed_fingerprint after programming tcp/8080")]
fn ports_and_protocols_on_one_vip_are_independent_keys() {
    runtime().block_on(async {
        let dp = SimDataplane::new();
        let tcp_80 = frontend(10, 8080, Proto::Tcp);
        let udp_80 = frontend(10, 8080, Proto::Udp);
        let tcp_90 = frontend(10, 9090, Proto::Tcp);
        let (a, b, c) = (vec![v4(0, 5)], vec![v4(1, 6)], vec![v4(2, 7)]);
        dp.update_service(tcp_80, a.clone()).await.expect("tcp/8080");
        dp.update_service(udp_80, b.clone()).await.expect("udp/8080");
        dp.update_service(tcp_90, c.clone()).await.expect("tcp/9090");
        assert_programmed(
            &dp,
            &tcp_80,
            Some(fingerprint(&tcp_80.vip(), &a)),
            "programming tcp/8080",
        )
        .await;
        assert_programmed(&dp, &udp_80, Some(fingerprint(&udp_80.vip(), &b)), "udp/8080").await;
        assert_programmed(&dp, &tcp_90, Some(fingerprint(&tcp_90.vip(), &c)), "tcp/9090").await;
        dp.update_service(tcp_80, vec![]).await.expect("withdraw tcp/8080");
        assert_programmed(&dp, &tcp_80, None, "withdrawing tcp/8080").await;
        assert_programmed(
            &dp,
            &udp_80,
            Some(fingerprint(&udp_80.vip(), &b)),
            "withdrawing tcp/8080",
        )
        .await;
    });
}

/// Edge case: a failed `update_service` leaves the prior fingerprint in place.
#[test]
#[should_panic(expected = "programmed_fingerprint after the first apply (failed update)")]
fn a_failed_update_leaves_the_prior_read_unchanged() {
    runtime().block_on(async {
        let dp = SimDataplane::new();
        let fe = frontend(10, 8080, Proto::Tcp);
        let first = vec![v4(0, 5)];
        dp.update_service(fe, first.clone()).await.expect("first apply");
        let prior = Some(fingerprint(&fe.vip(), &first));
        assert_programmed(&dp, &fe, prior, "the first apply (failed update)").await;
        dp.script_update_service_failure(DataplaneError::LoadFailed("scripted".to_owned()));
        dp.update_service(fe, vec![v4(1, 6)]).await.expect_err("scripted failure");
        assert_programmed(&dp, &fe, prior, "the failed update").await;
    });
}

/// Edge case (D-20): a set with an IPv6 backend is refused before any write, so
/// the prior fingerprint stays in place.
#[test]
#[should_panic(expected = "programmed_fingerprint after the first apply (refused IPv6 set)")]
fn a_refused_ipv6_update_leaves_the_prior_read_unchanged() {
    runtime().block_on(async {
        let dp = SimDataplane::new();
        let fe = frontend(10, 8080, Proto::Tcp);
        let first = vec![v4(0, 5)];
        dp.update_service(fe, first.clone()).await.expect("first apply");
        let prior = Some(fingerprint(&fe.vip(), &first));
        assert_programmed(&dp, &fe, prior, "the first apply (refused IPv6 set)").await;
        let mixed = vec![v4(0, 5), backend(1, IpAddr::V6(Ipv6Addr::LOCALHOST), 1, true)];
        dp.update_service(fe, mixed).await.expect_err("an IPv6 backend set is refused");
        assert_programmed(&dp, &fe, prior, "the refused IPv6 update").await;
    });
}

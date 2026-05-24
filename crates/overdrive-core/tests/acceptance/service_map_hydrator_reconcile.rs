//! Acceptance tests for `ServiceMapHydrator::reconcile` per Slice 08
//! / step 08-02 (architecture.md § 8 + ADR-0042).
//!
//! Coverage of the decision tree:
//!
//! - Pending actual → dispatch
//! - Completed actual matching desired fingerprint → no dispatch +
//!   reset retries
//! - Completed actual on different fingerprint → dispatch
//! - Failed actual same fingerprint, backoff not elapsed → no dispatch
//! - Failed actual same fingerprint, backoff elapsed → dispatch
//! - Failed actual different fingerprint → dispatch (no backoff gate)
//! - GC: retry memory dropped for services no longer in desired
//! - Increments attempts on dispatch
//! - `BTreeMap` iteration order is deterministic across actions
//!
//! Lives in `tests/acceptance/` rather than `src/` because dst-lint
//! scans only `src/**/*.rs` and bans `Instant::now()` there even
//! under `#[cfg(test)]`. The test module needs an `Instant` snapshot
//! for `TickContext.now`; the `tests/` location keeps the dst-lint
//! gate happy without contorting the test fixture.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use overdrive_core::dataplane::fingerprint::fingerprint;
use overdrive_core::id::{ServiceId, ServiceVip, SpiffeId};
use overdrive_core::reconcilers::{
    Action, Reconciler, RetryMemory, ServiceDesired, ServiceMapHydrator, ServiceMapHydratorState,
    ServiceMapHydratorView, TickContext,
};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::ServiceHydrationStatus;
use overdrive_core::wall_clock::UnixInstant;

fn make_service_id(n: u64) -> ServiceId {
    ServiceId::new(n).expect("valid ServiceId")
}

fn make_vip() -> ServiceVip {
    ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("valid ServiceVip")
}

fn make_backend() -> Backend {
    Backend {
        alloc: SpiffeId::new("spiffe://overdrive.local/job/web/alloc/web-0")
            .expect("valid SpiffeId"),
        addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 1)), 8080),
        weight: 1,
        healthy: true,
    }
}

fn make_desired_svc() -> ServiceDesired {
    let vip = make_vip();
    let backends = vec![make_backend()];
    let fp = fingerprint(&vip, &backends);
    ServiceDesired { vip, backends, fingerprint: fp }
}

fn make_tick(now_secs: u64) -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(now_secs)),
        tick: now_secs,
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

#[test]
fn dispatch_when_actual_pending_and_desired_present() {
    let r = ServiceMapHydrator::canonical(std::net::Ipv4Addr::UNSPECIFIED);
    let s_id = make_service_id(1);
    let mut desired = BTreeMap::new();
    desired.insert(s_id, make_desired_svc());
    let state = ServiceMapHydratorState { desired, actual: BTreeMap::new() };
    let view = ServiceMapHydratorView::default();
    let tick = make_tick(0);

    let (actions, next_view) = r.reconcile(&state, &state, &view, &tick);

    assert_eq!(actions.len(), 1, "should emit exactly one DataplaneUpdateService");
    match &actions[0] {
        Action::DataplaneUpdateService { service_id, .. } => {
            assert_eq!(*service_id, s_id);
        }
        other => panic!("unexpected action: {other:?}"),
    }
    let retry = next_view.retries.get(&s_id).expect("retry memory should exist");
    assert_eq!(retry.attempts, 1, "attempts must increment on dispatch");
}

#[test]
fn no_dispatch_when_actual_completed_matches_desired_fingerprint() {
    let r = ServiceMapHydrator::canonical(std::net::Ipv4Addr::UNSPECIFIED);
    let s_id = make_service_id(1);
    let desired_svc = make_desired_svc();
    let fp = desired_svc.fingerprint;
    let mut desired = BTreeMap::new();
    desired.insert(s_id, desired_svc);
    let mut actual = BTreeMap::new();
    actual.insert(
        s_id,
        ServiceHydrationStatus::Completed {
            fingerprint: fp,
            applied_at: UnixInstant::from_unix_duration(Duration::from_secs(1)),
        },
    );
    let state = ServiceMapHydratorState { desired, actual };

    let mut view = ServiceMapHydratorView::default();
    view.retries.insert(s_id, RetryMemory { attempts: 3, ..Default::default() });

    let (actions, next_view) = r.reconcile(&state, &state, &view, &make_tick(0));

    assert!(actions.is_empty(), "converged hydrator must emit zero actions");
    assert!(
        !next_view.retries.contains_key(&s_id),
        "convergence resets retry memory for this service",
    );
}

#[test]
fn dispatch_when_actual_completed_on_different_fingerprint() {
    let r = ServiceMapHydrator::canonical(std::net::Ipv4Addr::UNSPECIFIED);
    let s_id = make_service_id(1);
    let mut desired = BTreeMap::new();
    desired.insert(s_id, make_desired_svc());
    let mut actual = BTreeMap::new();
    actual.insert(
        s_id,
        ServiceHydrationStatus::Completed {
            fingerprint: 0xDEAD_BEEF_DEAD_BEEF,
            applied_at: UnixInstant::from_unix_duration(Duration::from_secs(1)),
        },
    );
    let state = ServiceMapHydratorState { desired, actual };
    let view = ServiceMapHydratorView::default();

    let (actions, _) = r.reconcile(&state, &state, &view, &make_tick(0));
    assert_eq!(actions.len(), 1, "stale-fingerprint Completed → dispatch");
}

#[test]
fn no_dispatch_when_failed_same_fingerprint_within_backoff() {
    let r = ServiceMapHydrator::canonical(std::net::Ipv4Addr::UNSPECIFIED);
    let s_id = make_service_id(1);
    let desired_svc = make_desired_svc();
    let fp = desired_svc.fingerprint;
    let mut desired = BTreeMap::new();
    desired.insert(s_id, desired_svc);
    let mut actual = BTreeMap::new();
    actual.insert(
        s_id,
        ServiceHydrationStatus::Failed {
            fingerprint: fp,
            failed_at: UnixInstant::from_unix_duration(Duration::from_secs(0)),
            reason: "synthetic".into(),
        },
    );
    let state = ServiceMapHydratorState { desired, actual };

    let mut view = ServiceMapHydratorView::default();
    view.retries.insert(
        s_id,
        RetryMemory {
            attempts: 1,
            last_failure_seen_at: UnixInstant::from_unix_duration(Duration::from_secs(0)),
            last_attempted_fingerprint: Some(fp),
        },
    );

    let (actions, _) = r.reconcile(&state, &state, &view, &make_tick(0));
    assert!(actions.is_empty(), "Failed same-fingerprint within backoff window → no dispatch");
}

#[test]
fn dispatch_when_failed_same_fingerprint_after_backoff_elapsed() {
    let r = ServiceMapHydrator::canonical(std::net::Ipv4Addr::UNSPECIFIED);
    let s_id = make_service_id(1);
    let desired_svc = make_desired_svc();
    let fp = desired_svc.fingerprint;
    let mut desired = BTreeMap::new();
    desired.insert(s_id, desired_svc);
    let mut actual = BTreeMap::new();
    actual.insert(
        s_id,
        ServiceHydrationStatus::Failed {
            fingerprint: fp,
            failed_at: UnixInstant::from_unix_duration(Duration::from_secs(0)),
            reason: "synthetic".into(),
        },
    );
    let state = ServiceMapHydratorState { desired, actual };

    let mut view = ServiceMapHydratorView::default();
    view.retries.insert(
        s_id,
        RetryMemory {
            attempts: 1,
            last_failure_seen_at: UnixInstant::from_unix_duration(Duration::from_secs(0)),
            last_attempted_fingerprint: Some(fp),
        },
    );

    let (actions, _) = r.reconcile(&state, &state, &view, &make_tick(2));
    assert_eq!(actions.len(), 1, "Failed same-fingerprint past backoff → dispatch");
}

#[test]
fn dispatch_when_failed_different_fingerprint_ignores_backoff() {
    let r = ServiceMapHydrator::canonical(std::net::Ipv4Addr::UNSPECIFIED);
    let s_id = make_service_id(1);
    let mut desired = BTreeMap::new();
    desired.insert(s_id, make_desired_svc());
    let mut actual = BTreeMap::new();
    actual.insert(
        s_id,
        ServiceHydrationStatus::Failed {
            fingerprint: 0xCAFE_F00D_CAFE_F00D,
            failed_at: UnixInstant::from_unix_duration(Duration::from_secs(0)),
            reason: "synthetic".into(),
        },
    );
    let state = ServiceMapHydratorState { desired, actual };

    let mut view = ServiceMapHydratorView::default();
    view.retries.insert(
        s_id,
        RetryMemory {
            attempts: 1,
            last_failure_seen_at: UnixInstant::from_unix_duration(Duration::from_secs(0)),
            last_attempted_fingerprint: Some(0xCAFE_F00D_CAFE_F00D),
        },
    );

    let (actions, _) = r.reconcile(&state, &state, &view, &make_tick(0));
    assert_eq!(
        actions.len(),
        1,
        "fingerprint drift on Failed → dispatch immediately, ignoring backoff",
    );
}

#[test]
fn gc_drops_retry_memory_for_services_no_longer_in_desired() {
    let r = ServiceMapHydrator::canonical(std::net::Ipv4Addr::UNSPECIFIED);
    let alive_id = make_service_id(1);
    let dead_id = make_service_id(2);
    let mut desired = BTreeMap::new();
    desired.insert(alive_id, make_desired_svc());
    let state = ServiceMapHydratorState { desired, actual: BTreeMap::new() };

    let mut view = ServiceMapHydratorView::default();
    view.retries.insert(alive_id, RetryMemory { attempts: 1, ..Default::default() });
    view.retries.insert(dead_id, RetryMemory { attempts: 5, ..Default::default() });

    let (_, next_view) = r.reconcile(&state, &state, &view, &make_tick(0));
    assert!(
        !next_view.retries.contains_key(&dead_id),
        "GC sweep must drop retry memory for services no longer in desired",
    );
    assert!(
        next_view.retries.contains_key(&alive_id),
        "alive service retry memory must survive GC",
    );
}

#[test]
fn iteration_order_is_btreemap_deterministic() {
    let r = ServiceMapHydrator::canonical(std::net::Ipv4Addr::UNSPECIFIED);
    let s1 = make_service_id(1);
    let s2 = make_service_id(2);
    let mut desired = BTreeMap::new();
    desired.insert(s2, make_desired_svc());
    desired.insert(s1, make_desired_svc());
    let state = ServiceMapHydratorState { desired, actual: BTreeMap::new() };
    let view = ServiceMapHydratorView::default();

    let (actions, _) = r.reconcile(&state, &state, &view, &make_tick(0));
    assert_eq!(actions.len(), 2);
    let ids: Vec<ServiceId> = actions
        .iter()
        .map(|a| match a {
            Action::DataplaneUpdateService { service_id, .. } => *service_id,
            other => panic!("unexpected action: {other:?}"),
        })
        .collect();
    assert_eq!(ids, vec![s1, s2], "actions must be emitted in BTreeMap key order");
}

// ---- Phase 16 review D12: backend-address classifier ---------------

use overdrive_core::reconcilers::{BackendAddressRejection, classify_backend_address};

#[test]
fn classify_backend_address_accepts_routable_unicast() {
    // Sample of legitimate backend addresses across common
    // workload ranges: RFC 1918 private (10/8, 172.16/12,
    // 192.168/16) and globally routable.
    for ip in [
        Ipv4Addr::new(10, 0, 1, 1),
        Ipv4Addr::new(172, 16, 0, 5),
        Ipv4Addr::new(192, 168, 1, 100),
        Ipv4Addr::new(8, 8, 8, 8),
        Ipv4Addr::new(203, 0, 113, 42),
    ] {
        assert_eq!(
            classify_backend_address(ip),
            Ok(()),
            "address {ip} should be accepted as a backend",
        );
    }
}

#[test]
fn classify_backend_address_rejects_loopback() {
    for ip in [Ipv4Addr::LOCALHOST, Ipv4Addr::new(127, 1, 2, 3)] {
        assert_eq!(
            classify_backend_address(ip),
            Err(BackendAddressRejection::Loopback),
            "loopback {ip} must be rejected",
        );
    }
}

#[test]
fn classify_backend_address_rejects_link_local() {
    assert_eq!(
        classify_backend_address(Ipv4Addr::new(169, 254, 1, 1)),
        Err(BackendAddressRejection::LinkLocal),
    );
}

#[test]
fn classify_backend_address_rejects_multicast() {
    assert_eq!(
        classify_backend_address(Ipv4Addr::new(224, 0, 0, 1)),
        Err(BackendAddressRejection::Multicast),
    );
    assert_eq!(
        classify_backend_address(Ipv4Addr::new(239, 1, 2, 3)),
        Err(BackendAddressRejection::Multicast),
    );
}

#[test]
fn classify_backend_address_rejects_broadcast() {
    assert_eq!(
        classify_backend_address(Ipv4Addr::BROADCAST),
        Err(BackendAddressRejection::Broadcast),
    );
}

#[test]
fn classify_backend_address_rejects_reserved_zero_network() {
    // RFC 1122 "this host on this network" — 0.0.0.0/8.
    assert_eq!(
        classify_backend_address(Ipv4Addr::UNSPECIFIED),
        Err(BackendAddressRejection::Reserved),
    );
    assert_eq!(
        classify_backend_address(Ipv4Addr::new(0, 1, 2, 3)),
        Err(BackendAddressRejection::Reserved),
    );
}

#[test]
fn rejection_display_form_is_operator_actionable() {
    // The `tracing::warn!` carries the `Display` form on the
    // `reason` field; spot-check that each variant produces a
    // grep-friendly substring.
    assert!(
        BackendAddressRejection::Loopback.to_string().contains("127.0"),
        "loopback Display must name the RFC range",
    );
    assert!(BackendAddressRejection::LinkLocal.to_string().contains("169.254"),);
    assert!(BackendAddressRejection::Multicast.to_string().contains("224.0"));
    assert!(BackendAddressRejection::Broadcast.to_string().contains("255.255"));
    assert!(BackendAddressRejection::Reserved.to_string().contains("0.0.0"));
}

#[test]
fn hydrator_skips_register_local_backend_for_loopback() {
    // Hydrator wired so the classifier sees every backend as
    // "local" (`host_ipv4` matches the backend IP). A loopback
    // backend address must NOT produce a `RegisterLocalBackend`
    // action — D12's guard fires before emission.
    let host_ipv4 = Ipv4Addr::LOCALHOST;
    let r = ServiceMapHydrator::canonical(host_ipv4);
    let s_id = make_service_id(1);

    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("valid ServiceVip");
    let backend = Backend {
        alloc: SpiffeId::new("spiffe://overdrive.local/job/web/alloc/web-0")
            .expect("valid SpiffeId"),
        addr: SocketAddr::new(IpAddr::V4(host_ipv4), 8080),
        weight: 1,
        healthy: true,
    };
    let backends = vec![backend];
    let fp = fingerprint(&vip, &backends);

    let mut desired = BTreeMap::new();
    desired.insert(s_id, ServiceDesired { vip, backends, fingerprint: fp });
    let state = ServiceMapHydratorState { desired, actual: BTreeMap::new() };
    let view = ServiceMapHydratorView::default();
    let (actions, _) = r.reconcile(&state, &state, &view, &make_tick(0));

    let register_count =
        actions.iter().filter(|a| matches!(a, Action::RegisterLocalBackend { .. })).count();
    assert_eq!(
        register_count, 0,
        "loopback backend must be skipped by the D12 classifier guard; actions: {actions:?}",
    );
}

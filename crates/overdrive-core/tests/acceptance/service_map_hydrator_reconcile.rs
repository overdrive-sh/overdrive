//! Acceptance tests for `ServiceMapHydrator::reconcile` and
//! `ServiceMapHydrator::next_evaluation_at` (architecture.md § 8, ADR-0042,
//! ADR-0299, feature-delta D-3 and D-20).
//!
//! The hydrator's `actual` is the resource it manages: per service, the
//! fingerprint of the backend set the dataplane has programmed
//! (`BTreeMap<ServiceId, BackendSetFingerprint>`; a service with no entry has
//! nothing programmed). The executor records no outcome row, so the state
//! carries none.
//!
//! The diff is pure. A service is *in place* when its desired survivor set
//! (the backends that pass the mesh gate) has a fingerprint equal to its
//! `actual` entry, or when the survivor set is empty and `actual` has no entry.
//! A survivor set holding an IPv6 backend is never in place. A service that is
//! not in place dispatches unless its retry memory says the same set was
//! attempted and the backoff window has not elapsed.
//!
//! Coverage:
//!
//! - in place for an equal fingerprint, and for an empty survivor set with no
//!   programmed entry: no dispatch
//! - not in place for a missing, differing, weight-only, health-only, or
//!   removed-backend fingerprint, and for a still-programmed empty survivor
//!   set: dispatch
//! - an IPv6 survivor is never in place
//! - a dispatch inside the backoff window is suppressed and the retry deadline
//!   is retained by `next_evaluation_at`; a changed desired set dispatches at
//!   once
//! - an in-place observation removes the retry entry
//! - the decision is a function of desired survivors and the programmed
//!   fingerprint only (property)
//! - GC and `BTreeMap` iteration order
//!
//! Lives in `tests/acceptance/` rather than `src/` because dst-lint scans only
//! `src/**/*.rs` and bans `Instant::now()` there even under `#[cfg(test)]`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant};

use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::dataplane::fingerprint::{BackendSetFingerprint, fingerprint};
use overdrive_core::id::{ServiceId, ServiceVip, SpiffeId};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::{
    RetryMemory, ServiceDesired, ServiceMapHydrator, ServiceMapHydratorState,
    ServiceMapHydratorView, backoff_for_attempt,
};
use proptest::prelude::*;

/// The canonical Path-A/mesh workload subnet (`WORKLOAD_SUBNET_BASE`,
/// `10.99.0.0/16`). Backends outside it are remote survivors.
fn workload_subnet() -> ipnet::Ipv4Net {
    ipnet::Ipv4Net::new(Ipv4Addr::new(10, 99, 0, 0), 16).expect("valid /16")
}

fn make_service_id(n: u64) -> ServiceId {
    ServiceId::new(n).expect("valid ServiceId")
}

fn make_vip() -> ServiceVip {
    ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("valid ServiceVip")
}

const fn v4_addr(ip: Ipv4Addr) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(ip), 8080)
}

fn backend_at(addr: SocketAddr, weight: u16, healthy: bool) -> Backend {
    Backend {
        alloc: SpiffeId::new("spiffe://overdrive.local/workload/web/alloc/web-0")
            .expect("valid SpiffeId"),
        addr,
        weight,
        healthy,
    }
}

/// The one remote backend most tests use (outside the mesh subnet).
fn make_backend() -> Backend {
    backend_at(v4_addr(Ipv4Addr::new(10, 0, 1, 1)), 1, true)
}

/// A mesh backend (inside `10.99.0.0/16`): gated out of the survivor set.
fn mesh_backend() -> Backend {
    backend_at(v4_addr(Ipv4Addr::new(10, 99, 0, 6)), 1, true)
}

fn v6_backend() -> Backend {
    backend_at(
        SocketAddr::new(IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 7)), 8080),
        1,
        true,
    )
}

fn desired_of(backends: Vec<Backend>) -> ServiceDesired {
    let vip = make_vip();
    let fp = fingerprint(&vip, &backends);
    ServiceDesired {
        vip,
        port: std::num::NonZeroU16::new(8080).expect("non-zero"),
        proto: Proto::Tcp,
        backends,
        fingerprint: fp,
    }
}

fn make_desired_svc() -> ServiceDesired {
    desired_of(vec![make_backend()])
}

/// The fingerprint the dataplane carries once it has programmed `backends`
/// for the test VIP.
fn programmed(backends: &[Backend]) -> BackendSetFingerprint {
    fingerprint(&make_vip(), backends)
}

const fn secs(n: u64) -> UnixInstant {
    UnixInstant::from_unix_duration(Duration::from_secs(n))
}

fn make_tick(now_secs: u64) -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: secs(now_secs),
        tick: now_secs,
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

/// The runtime hydrates `desired` and `actual` as two separate states
/// (`hydrate_desired` fills only `desired`, `hydrate_actual` only `actual`).
/// Build them the same way so a test cannot pass by reading the wrong side.
const fn runtime_states(
    desired: BTreeMap<ServiceId, ServiceDesired>,
    actual: BTreeMap<ServiceId, BackendSetFingerprint>,
) -> (ServiceMapHydratorState, ServiceMapHydratorState) {
    (
        ServiceMapHydratorState { desired, actual: BTreeMap::new() },
        ServiceMapHydratorState { desired: BTreeMap::new(), actual },
    )
}

fn reconcile_once(
    desired: BTreeMap<ServiceId, ServiceDesired>,
    actual: BTreeMap<ServiceId, BackendSetFingerprint>,
    view: &ServiceMapHydratorView,
    tick: &TickContext,
) -> (Vec<Action>, ServiceMapHydratorView) {
    let r = ServiceMapHydrator::canonical(workload_subnet());
    let (d, a) = runtime_states(desired, actual);
    r.reconcile(&d, &a, view, tick)
}

fn next_wake(
    desired: BTreeMap<ServiceId, ServiceDesired>,
    actual: BTreeMap<ServiceId, BackendSetFingerprint>,
    next_view: &ServiceMapHydratorView,
    tick: &TickContext,
) -> Option<UnixInstant> {
    let r = ServiceMapHydrator::canonical(workload_subnet());
    let (d, a) = runtime_states(desired, actual);
    r.next_evaluation_at(&d, &a, next_view, tick)
}

fn update_payloads(actions: &[Action]) -> Vec<&Vec<Backend>> {
    actions
        .iter()
        .map(|a| match a {
            Action::DataplaneUpdateService { backends, .. } => backends,
            other => panic!("unexpected action: {other:?}"),
        })
        .collect()
}

// ---- in place: no dispatch -------------------------------------------

#[test]
fn dispatch_when_nothing_is_programmed_for_the_desired_set() {
    let s_id = make_service_id(1);
    let desired = BTreeMap::from([(s_id, make_desired_svc())]);

    let (actions, next_view) =
        reconcile_once(desired, BTreeMap::new(), &ServiceMapHydratorView::default(), &make_tick(0));

    assert_eq!(actions.len(), 1, "should emit exactly one DataplaneUpdateService");
    match &actions[0] {
        Action::DataplaneUpdateService { service_id, backends, .. } => {
            assert_eq!(*service_id, s_id);
            assert_eq!(backends, &vec![make_backend()], "the remote survivors are the payload");
        }
        other => panic!("unexpected action: {other:?}"),
    }
    let retry = next_view.retries.get(&s_id).expect("retry memory should exist");
    assert_eq!(retry.attempts, 1, "attempts must increment on dispatch");
    assert_eq!(
        retry.last_attempted_fingerprint,
        Some(programmed(&[make_backend()])),
        "the attempted fingerprint is the survivor set's"
    );
}

#[test]
fn no_dispatch_when_programmed_fingerprint_equals_the_desired_survivor_fingerprint() {
    let s_id = make_service_id(1);
    let desired = BTreeMap::from([(s_id, make_desired_svc())]);
    let actual = BTreeMap::from([(s_id, programmed(&[make_backend()]))]);

    let (actions, _) =
        reconcile_once(desired, actual, &ServiceMapHydratorView::default(), &make_tick(0));

    assert!(actions.is_empty(), "a service whose programmed fingerprint matches emits nothing");
}

#[test]
fn no_dispatch_when_the_programmed_set_matches_after_the_mesh_gate() {
    // Desired holds a remote and a mesh backend; the dataplane carries the
    // remote one alone, which is exactly the survivor set.
    let s_id = make_service_id(1);
    let desired = BTreeMap::from([(s_id, desired_of(vec![make_backend(), mesh_backend()]))]);
    let actual = BTreeMap::from([(s_id, programmed(&[make_backend()]))]);

    let (actions, _) =
        reconcile_once(desired, actual, &ServiceMapHydratorView::default(), &make_tick(0));

    assert!(actions.is_empty(), "the comparison is over the survivors, not the full desired set");
}

#[test]
fn no_dispatch_for_an_empty_survivor_set_with_nothing_programmed() {
    for (label, backends) in
        [("all-mesh backends", vec![mesh_backend()]), ("no backends at all", vec![])]
    {
        let s_id = make_service_id(1);
        let desired = BTreeMap::from([(s_id, desired_of(backends))]);

        let (actions, _) = reconcile_once(
            desired,
            BTreeMap::new(),
            &ServiceMapHydratorView::default(),
            &make_tick(0),
        );

        assert!(
            actions.is_empty(),
            "{label}: an empty survivor set with no programmed entry is a purge already in place"
        );
    }
}

// ---- not in place: dispatch ------------------------------------------

#[test]
fn dispatch_for_an_empty_survivor_set_whose_slot_is_still_programmed() {
    let vip = make_vip();
    for (label, entry) in [
        ("a previously programmed remote set", programmed(&[make_backend()])),
        ("a fingerprint over the empty set", fingerprint(&vip, &[])),
    ] {
        let s_id = make_service_id(1);
        let desired = BTreeMap::from([(s_id, desired_of(vec![mesh_backend()]))]);
        let actual = BTreeMap::from([(s_id, entry)]);

        let (actions, next_view) =
            reconcile_once(desired, actual, &ServiceMapHydratorView::default(), &make_tick(0));

        assert_eq!(actions.len(), 1, "{label}: an empty survivor set must purge the slot");
        assert_eq!(
            update_payloads(&actions),
            vec![&Vec::<Backend>::new()],
            "{label}: empty payload"
        );
        assert_eq!(
            next_view.retries.get(&s_id).and_then(|m| m.last_attempted_fingerprint),
            Some(fingerprint(&vip, &[])),
            "{label}: the purge records the fingerprint over the empty set"
        );
    }
}

#[test]
fn dispatch_when_the_programmed_fingerprint_differs_from_the_desired_set() {
    let remote = make_backend();
    let cases: Vec<(&str, BackendSetFingerprint)> = vec![
        ("an unrelated fingerprint", 0xDEAD_BEEF_DEAD_BEEF),
        ("a weight-only change", programmed(&[backend_at(remote.addr, 5, true)])),
        ("a health-only change", programmed(&[backend_at(remote.addr, 1, false)])),
        (
            "a removed backend",
            programmed(&[remote, backend_at(v4_addr(Ipv4Addr::new(10, 0, 1, 2)), 1, true)]),
        ),
    ];
    for (label, programmed_fp) in cases {
        let s_id = make_service_id(1);
        let desired = BTreeMap::from([(s_id, make_desired_svc())]);
        let actual = BTreeMap::from([(s_id, programmed_fp)]);

        let (actions, _) =
            reconcile_once(desired, actual, &ServiceMapHydratorView::default(), &make_tick(0));

        assert_eq!(actions.len(), 1, "{label}: a differing programmed fingerprint must dispatch");
    }
}

#[test]
fn dispatch_when_a_weight_or_health_change_is_the_only_difference_in_the_desired_set() {
    // The desired side changes while the dataplane still carries the old set.
    let addr = make_backend().addr;
    let old = vec![backend_at(addr, 1, true)];
    for (label, new) in
        [("weight", vec![backend_at(addr, 2, true)]), ("health", vec![backend_at(addr, 1, false)])]
    {
        let s_id = make_service_id(1);
        let desired = BTreeMap::from([(s_id, desired_of(new))]);
        let actual = BTreeMap::from([(s_id, programmed(&old))]);

        let (actions, _) =
            reconcile_once(desired, actual, &ServiceMapHydratorView::default(), &make_tick(0));

        assert_eq!(actions.len(), 1, "a {label}-only change must reprogram the service");
    }
}

#[test]
fn an_ipv6_survivor_is_never_in_place() {
    // Even when the dataplane reports exactly the fingerprint of the survivor
    // set, a set holding an IPv6 backend is one `update_service` refuses, so
    // the hydrator must not treat it as converged.
    let v6_only = vec![v6_backend()];
    let mixed = vec![make_backend(), v6_backend()];
    for (label, backends) in
        [("an IPv6 backend alone", v6_only), ("an IPv4 and an IPv6 backend", mixed)]
    {
        for entry in [None, Some(programmed(&backends))] {
            let s_id = make_service_id(1);
            let desired = BTreeMap::from([(s_id, desired_of(backends.clone()))]);
            let actual: BTreeMap<_, _> = entry.map(|fp| (s_id, fp)).into_iter().collect();

            let (actions, next_view) =
                reconcile_once(desired, actual, &ServiceMapHydratorView::default(), &make_tick(0));

            assert_eq!(
                actions.len(),
                1,
                "{label} (programmed entry: {entry:?}): never in place, so it dispatches"
            );
            assert_eq!(
                next_view.retries.get(&s_id).map(|m| m.attempts),
                Some(1),
                "{label}: the dispatch is charged to the retry budget"
            );
        }
    }
}

#[test]
fn an_ipv6_survivor_retries_on_its_backoff_rather_than_every_tick() {
    let s_id = make_service_id(1);
    let backends = vec![make_backend(), v6_backend()];
    let fp = programmed(&backends);
    let desired = BTreeMap::from([(s_id, desired_of(backends))]);
    let view = ServiceMapHydratorView {
        retries: BTreeMap::from([(
            s_id,
            RetryMemory {
                attempts: 1,
                last_failure_seen_at: secs(100),
                last_attempted_fingerprint: Some(fp),
            },
        )]),
    };

    let (inside, _) = reconcile_once(desired.clone(), BTreeMap::new(), &view, &make_tick(100));
    assert!(inside.is_empty(), "a refused IPv6 set inside its backoff window is not re-sent");

    let after = 100 + backoff_for_attempt(1).as_secs() + 1;
    let (past, _) = reconcile_once(desired, BTreeMap::new(), &view, &make_tick(after));
    assert_eq!(past.len(), 1, "once the window elapses the refused set is attempted again");
}

// ---- pacing: backoff window and retention ----------------------------

#[test]
fn dispatch_inside_the_backoff_window_is_suppressed_and_the_deadline_is_retained() {
    let s_id = make_service_id(1);
    let fp = programmed(&[make_backend()]);
    let desired = BTreeMap::from([(s_id, make_desired_svc())]);
    let retry = RetryMemory {
        attempts: 1,
        last_failure_seen_at: secs(100),
        last_attempted_fingerprint: Some(fp),
    };
    let view = ServiceMapHydratorView { retries: BTreeMap::from([(s_id, retry.clone())]) };
    let tick = make_tick(100);

    let (actions, next_view) = reconcile_once(desired.clone(), BTreeMap::new(), &view, &tick);

    assert!(actions.is_empty(), "same set, nothing programmed, inside the window: no re-dispatch");
    assert_eq!(
        next_view.retries.get(&s_id),
        Some(&retry),
        "a suppressed dispatch leaves the retry entry untouched"
    );
    assert_eq!(
        next_wake(desired, BTreeMap::new(), &next_view, &tick),
        Some(secs(100) + backoff_for_attempt(1)),
        "the key is retained until the backoff deadline last_failure_seen_at + backoff"
    );
}

#[test]
fn dispatch_resumes_once_the_backoff_window_has_elapsed() {
    let s_id = make_service_id(1);
    let fp = programmed(&[make_backend()]);
    let desired = BTreeMap::from([(s_id, make_desired_svc())]);
    let view = ServiceMapHydratorView {
        retries: BTreeMap::from([(
            s_id,
            RetryMemory {
                attempts: 1,
                last_failure_seen_at: secs(0),
                last_attempted_fingerprint: Some(fp),
            },
        )]),
    };
    let now = backoff_for_attempt(1).as_secs() + 1;

    let (actions, next_view) = reconcile_once(desired, BTreeMap::new(), &view, &make_tick(now));

    assert_eq!(actions.len(), 1, "past the window the unprogrammed set is attempted again");
    assert_eq!(next_view.retries.get(&s_id).map(|m| m.attempts), Some(2));
}

#[test]
fn a_changed_desired_set_dispatches_at_once_even_inside_a_backoff_window() {
    let old = programmed(&[backend_at(make_backend().addr, 9, true)]);
    for (label, actual) in [("nothing programmed", None), ("the old set programmed", Some(old))] {
        let s_id = make_service_id(1);
        let desired = BTreeMap::from([(s_id, make_desired_svc())]);
        let actual: BTreeMap<_, _> = actual.map(|fp| (s_id, fp)).into_iter().collect();
        let view = ServiceMapHydratorView {
            retries: BTreeMap::from([(
                s_id,
                RetryMemory {
                    attempts: 1,
                    last_failure_seen_at: secs(100),
                    last_attempted_fingerprint: Some(old),
                },
            )]),
        };

        let (actions, next_view) = reconcile_once(desired, actual, &view, &make_tick(100));

        assert_eq!(actions.len(), 1, "{label}: a changed desired set ignores the backoff window");
        let retry = next_view.retries.get(&s_id).expect("retry memory");
        assert_eq!(retry.attempts, 2, "{label}: the dispatch is charged");
        assert_eq!(
            retry.last_attempted_fingerprint,
            Some(programmed(&[make_backend()])),
            "{label}"
        );
        assert_eq!(retry.last_failure_seen_at, secs(100), "{label}: the window restarts at now");
    }
}

#[test]
fn next_evaluation_at_ranges_only_over_future_deadlines_of_services_not_in_place() {
    let b = backoff_for_attempt(1);
    let now = Duration::from_secs(1000);
    let at = |d: Duration| UnixInstant::from_unix_duration(d);
    let entry = |last: Duration| RetryMemory {
        attempts: 1,
        last_failure_seen_at: at(last),
        last_attempted_fingerprint: None,
    };
    let (s1, s2, s3, s4) =
        (make_service_id(1), make_service_id(2), make_service_id(3), make_service_id(4));
    let desired = BTreeMap::from([s1, s2, s3, s4].map(|id| (id, make_desired_svc())));
    // s4 is the only service in place.
    let actual = BTreeMap::from([(s4, programmed(&[make_backend()]))]);
    let next_view = ServiceMapHydratorView {
        retries: BTreeMap::from([
            (s1, entry(now)), // deadline now + b
            (s2, entry(now.saturating_sub(b).saturating_sub(Duration::from_secs(1)))), // deadline already past
            (s3, entry(now.saturating_sub(b / 2))), // deadline now + b/2: the earliest
            (s4, entry(now.saturating_sub(b) + Duration::from_millis(1))), // would be earliest, but in place
        ]),
    };

    assert_eq!(
        next_wake(desired, actual, &next_view, &make_tick(1000)),
        Some(at(now + b / 2)),
        "the earliest strictly-future deadline over services not in place"
    );
}

#[test]
fn next_evaluation_at_is_none_when_no_service_is_waiting_on_a_backoff() {
    let s_id = make_service_id(1);
    let desired = BTreeMap::from([(s_id, make_desired_svc())]);
    let tick = make_tick(100);

    // No retry entry at all.
    assert_eq!(
        next_wake(desired.clone(), BTreeMap::new(), &ServiceMapHydratorView::default(), &tick),
        None,
        "nothing is waiting"
    );

    // An entry exists but the service is in place.
    let view = ServiceMapHydratorView {
        retries: BTreeMap::from([(
            s_id,
            RetryMemory {
                attempts: 1,
                last_failure_seen_at: secs(100),
                last_attempted_fingerprint: None,
            },
        )]),
    };
    assert_eq!(
        next_wake(
            desired.clone(),
            BTreeMap::from([(s_id, programmed(&[make_backend()]))]),
            &view,
            &tick
        ),
        None,
        "an in-place service has no retry deadline"
    );

    // A deadline exactly at now is not strictly in the future.
    let at_boundary = make_tick(100 + backoff_for_attempt(1).as_secs());
    assert_eq!(
        next_wake(desired, BTreeMap::new(), &view, &at_boundary),
        None,
        "a deadline that is not strictly in the future is not retained"
    );
}

// ---- in-place observation clears the retry entry ---------------------

#[test]
fn an_in_place_observation_removes_the_retry_entry() {
    let (settled, all_mesh, waiting) = (make_service_id(1), make_service_id(2), make_service_id(3));
    let desired = BTreeMap::from([
        (settled, make_desired_svc()),
        (all_mesh, desired_of(vec![mesh_backend()])),
        (waiting, make_desired_svc()),
    ]);
    // `settled` carries its fingerprint, `all_mesh` has nothing programmed
    // (a purge in place), `waiting` has nothing programmed for a real set.
    let actual = BTreeMap::from([(settled, programmed(&[make_backend()]))]);
    let entry = RetryMemory {
        attempts: 3,
        last_failure_seen_at: secs(100),
        last_attempted_fingerprint: Some(programmed(&[make_backend()])),
    };
    let view = ServiceMapHydratorView {
        retries: BTreeMap::from([
            (settled, entry.clone()),
            (all_mesh, entry.clone()),
            (waiting, entry.clone()),
        ]),
    };

    let (actions, next_view) = reconcile_once(desired, actual, &view, &make_tick(100));

    assert!(
        actions.is_empty(),
        "the waiting service is inside its window; the others are in place"
    );
    assert!(!next_view.retries.contains_key(&settled), "equal fingerprint clears the entry");
    assert!(!next_view.retries.contains_key(&all_mesh), "an empty purge in place clears the entry");
    assert_eq!(
        next_view.retries.get(&waiting),
        Some(&entry),
        "a service that is not in place keeps its entry"
    );
}

// ---- the decision reads no receipt -----------------------------------

#[test]
fn a_map_lost_after_a_successful_update_is_reprogrammed_at_once() {
    // Tick 1: the dataplane carries the set, so the service is in place and
    // its retry entry is cleared. Tick 2: the maps were recreated (a restart
    // or reboot), so the dataplane reports nothing. Whatever receipt an
    // earlier update left behind, the hydrator dispatches again immediately.
    let s_id = make_service_id(1);
    let desired = BTreeMap::from([(s_id, make_desired_svc())]);
    let view0 = ServiceMapHydratorView {
        retries: BTreeMap::from([(
            s_id,
            RetryMemory {
                attempts: 2,
                last_failure_seen_at: secs(10),
                last_attempted_fingerprint: Some(programmed(&[make_backend()])),
            },
        )]),
    };

    let (actions1, view1) = reconcile_once(
        desired.clone(),
        BTreeMap::from([(s_id, programmed(&[make_backend()]))]),
        &view0,
        &make_tick(11),
    );
    assert!(actions1.is_empty());
    assert!(view1.retries.is_empty(), "in place: the retry entry is gone");

    let (actions2, _) = reconcile_once(desired, BTreeMap::new(), &view1, &make_tick(12));
    assert_eq!(actions2.len(), 1, "an empty dataplane is reprogrammed on the next evaluation");
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        rng_seed: proptest::test_runner::RngSeed::Fixed(351_020),
        ..ProptestConfig::default()
    })]

    /// CONTRACT_SHAPE: pure-function.
    /// The dispatch decision is a function of the desired survivor set and the
    /// programmed fingerprint alone. The state has no result-row field, and for
    /// every generated desired set and programmed entry the service dispatches
    /// exactly when it is not in place, whatever stale retry memory (attempts
    /// whose window has elapsed, aimed at the same set) the View carries.
    #[test]
    fn dispatch_is_a_function_of_desired_survivors_and_programmed_fingerprint_only(
        backend_specs in proptest::collection::vec(
            (any::<bool>(), 1u8..=250, 1u16..=4, any::<bool>()), 0..5,
        ),
        programmed_choice in 0u8..3,
        with_stale_retry in any::<bool>(),
    ) {
        let backends: Vec<Backend> = backend_specs
            .iter()
            .enumerate()
            .map(|(i, (mesh, host, weight, healthy))| {
                let ip = if *mesh {
                    Ipv4Addr::new(10, 99, 1, *host)
                } else {
                    Ipv4Addr::new(10, 1, u8::try_from(i).expect("at most 5 backends"), *host)
                };
                backend_at(v4_addr(ip), *weight, *healthy)
            })
            .collect();
        let survivors: Vec<Backend> = backends
            .iter()
            .filter(|b| match b.addr.ip() {
                IpAddr::V4(v4) => !workload_subnet().contains(&v4),
                IpAddr::V6(_) => true,
            })
            .cloned()
            .collect();
        let survivor_fp = programmed(&survivors);

        let s_id = make_service_id(1);
        let entry: Option<BackendSetFingerprint> = match programmed_choice {
            0 => None,
            1 => Some(survivor_fp),
            _ => Some(survivor_fp ^ 0x1),
        };
        // Independent restatement of the pinned rule.
        let in_place = if survivors.is_empty() { entry.is_none() } else { entry == Some(survivor_fp) };

        let view = if with_stale_retry {
            ServiceMapHydratorView {
                retries: BTreeMap::from([(s_id, RetryMemory {
                    attempts: 1,
                    last_failure_seen_at: secs(0),
                    last_attempted_fingerprint: Some(survivor_fp),
                })]),
            }
        } else {
            ServiceMapHydratorView::default()
        };

        let desired = BTreeMap::from([(s_id, desired_of(backends))]);
        let actual: BTreeMap<_, _> = entry.map(|fp| (s_id, fp)).into_iter().collect();
        let (actions, _) = reconcile_once(desired, actual, &view, &make_tick(1000));

        prop_assert_eq!(actions.len(), usize::from(!in_place), "seed=351020: dispatch iff not in place");
    }
}

// ---- housekeeping ----------------------------------------------------

#[test]
fn gc_drops_retry_memory_for_services_no_longer_in_desired() {
    let alive_id = make_service_id(1);
    let dead_id = make_service_id(2);
    let desired = BTreeMap::from([(alive_id, make_desired_svc())]);

    let mut view = ServiceMapHydratorView::default();
    view.retries.insert(alive_id, RetryMemory { attempts: 1, ..Default::default() });
    view.retries.insert(dead_id, RetryMemory { attempts: 5, ..Default::default() });

    let (_, next_view) = reconcile_once(desired, BTreeMap::new(), &view, &make_tick(0));
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
    let s1 = make_service_id(1);
    let s2 = make_service_id(2);
    let mut desired = BTreeMap::new();
    desired.insert(s2, make_desired_svc());
    desired.insert(s1, make_desired_svc());

    let (actions, _) =
        reconcile_once(desired, BTreeMap::new(), &ServiceMapHydratorView::default(), &make_tick(0));
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

use overdrive_reconcilers::{BackendAddressRejection, classify_backend_address};

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

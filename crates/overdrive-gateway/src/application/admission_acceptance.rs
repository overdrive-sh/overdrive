//! R-62 admission refusals: every ceiling of the admission budget and of a
//! connection lease refuses the one beyond it with its typed error, and a
//! refusal takes nothing that a later drop would have to give back.
//!
//! The budget-level bodies run in the default lane over the fixed first-slice
//! limits (`GatewayLimits::first_slice`, the only constructor): they take every
//! permit up to the ceiling, which is cheap and opens no socket. The bodies
//! that need an admitted connection lease need a promoted generation, so they
//! compose the real owner over the held-gate doubles and run under
//! `integration-tests` (they bind a loopback address, R-6).
#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use std::sync::atomic::Ordering;

use super::*;

fn budget() -> Arc<GatewayAdmissionBudget> {
    GatewayAdmissionBudget::new(Arc::new(GatewayLimits::first_slice()))
}

/// Take `ceiling` permits, all of which must be admitted.
fn fill<P>(ceiling: u32, take: impl Fn() -> Result<P, GatewayAdmissionError>) -> Vec<P> {
    (0..ceiling)
        .map(|n| {
            take().unwrap_or_else(|error| panic!("permit {n} of {ceiling} refused: {error:?}"))
        })
        .collect()
}

/// The public-connection ceiling admits exactly `max_public_connections`,
/// refuses the next with `ConnectionLimit` and takes nothing for it, admits
/// again once one permit is returned, and ends at zero.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn the_connection_beyond_the_public_connection_ceiling_is_refused_and_leaks_nothing() {
    let budget = budget();
    let ceiling = budget.limits.max_public_connections().get();
    let mut held = fill(ceiling, || budget.try_public_connection());
    assert_eq!(budget.public_connections.load(Ordering::SeqCst), ceiling);

    let refused = budget.try_public_connection();
    assert!(
        matches!(refused, Err(GatewayAdmissionError::ConnectionLimit)),
        "the connection beyond the ceiling is refused with ConnectionLimit: {:?}",
        refused.err()
    );
    assert_eq!(
        budget.public_connections.load(Ordering::SeqCst),
        ceiling,
        "a refusal takes no permit"
    );

    drop(held.pop());
    assert_eq!(budget.public_connections.load(Ordering::SeqCst), ceiling - 1);
    let again = budget.try_public_connection();
    assert!(again.is_ok(), "a returned permit admits one connection again");
    drop(again);
    drop(held);
    assert_eq!(budget.public_connections.load(Ordering::SeqCst), 0, "every permit is returned");
}

/// The in-flight connect ceiling refuses the connect beyond it with
/// `ConnectLimit`, takes nothing for the refusal, and ends at zero.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn the_connect_beyond_the_connect_ceiling_is_refused_and_leaks_nothing() {
    let budget = budget();
    let ceiling = budget.limits.max_inflight_connects().get();
    let mut held = fill(ceiling, || budget.try_connect());
    assert_eq!(budget.inflight_connects.load(Ordering::SeqCst), ceiling);

    let refused = budget.try_connect();
    assert!(
        matches!(refused, Err(GatewayAdmissionError::ConnectLimit)),
        "the connect beyond the ceiling is refused with ConnectLimit: {:?}",
        refused.err()
    );
    assert_eq!(
        budget.inflight_connects.load(Ordering::SeqCst),
        ceiling,
        "a refusal takes no permit"
    );

    drop(held.pop());
    let again = budget.try_connect();
    assert!(again.is_ok(), "a returned permit admits one connect again");
    drop(again);
    drop(held);
    assert_eq!(budget.inflight_connects.load(Ordering::SeqCst), 0, "every permit is returned");
}

/// The global in-flight request ceiling refuses the request beyond it with
/// `RequestLimit`, takes nothing for the refusal, and ends at zero.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn the_request_beyond_the_global_request_ceiling_is_refused_and_leaks_nothing() {
    let budget = budget();
    let ceiling = budget.limits.max_inflight_requests().get();
    let mut held = fill(ceiling, || budget.try_request());
    assert_eq!(budget.inflight_requests.load(Ordering::SeqCst), ceiling);

    let refused = budget.try_request();
    assert!(
        matches!(refused, Err(GatewayAdmissionError::RequestLimit)),
        "the request beyond the ceiling is refused with RequestLimit: {:?}",
        refused.err()
    );
    assert_eq!(
        budget.inflight_requests.load(Ordering::SeqCst),
        ceiling,
        "a refusal takes no permit"
    );

    drop(held.pop());
    let again = budget.try_request();
    assert!(again.is_ok(), "a returned permit admits one request again");
    drop(again);
    drop(held);
    assert_eq!(budget.inflight_requests.load(Ordering::SeqCst), 0, "every permit is returned");
}

/// The refusals of an admitted connection and of its requests, over the real
/// owner's promoted generation.
#[cfg(feature = "integration-tests")]
mod over_leases {
    use std::sync::atomic::AtomicU64;

    use super::super::held_probe_doubles::{
        Composition, ProbeOutcome, next_ordinal, route, wait_until,
    };
    use super::*;

    /// Second octets of this module's bind addresses, one per test (nextest
    /// runs each test in its own process; table in `held_probe_doubles`).
    const CONNECTION_CEILING_OCTET: u8 = 68;
    const IN_FLIGHT_REQUEST_OCTET: u8 = 69;
    const PER_CONNECTION_CEILING_OCTET: u8 = 70;
    const GLOBAL_REQUEST_CEILING_OCTET: u8 = 71;
    /// One ordinal per composition of the process, so no two share an address.
    static NEXT_ORDINAL: AtomicU64 = AtomicU64::new(0);

    /// A composition whose declared Route has been promoted: `admission`
    /// admits connections of its Current generation.
    async fn promoted_world(octet: u8) -> Composition {
        let world = Composition::start(octet, next_ordinal(&NEXT_ORDINAL)).await;
        world.routes.routes().apply(route("/")).await.expect("declare the Route");
        wait_until(&world.status, &world.probes, "the candidate's gate held", |_, probes| {
            probes.held_count() == 1
        })
        .await
        .expect("candidate held");
        let gate = world.probes.held_gates()[0];
        world.probes.release(gate, ProbeOutcome::Usable);
        wait_until(&world.status, &world.probes, "the candidate promoted", |status, _| {
            status.is_some_and(|row| row.current.is_some())
        })
        .await
        .expect("candidate promoted");
        world
    }

    fn now(world: &Composition) -> UnixInstant {
        UnixInstant::from_clock(world.clock.as_ref())
    }

    fn connect(world: &Composition) -> GatewayConnectionLease {
        world
            .admission
            .acquire_connection(now(world))
            .unwrap_or_else(|error| panic!("the connection was refused: {error:?}"))
    }

    fn retained_connections(lease: &GatewayConnectionLease) -> u32 {
        lease.snapshot.retention.connections.load(Ordering::SeqCst)
    }

    fn retained_requests(lease: &GatewayConnectionLease) -> u32 {
        lease.snapshot.retention.requests.load(Ordering::SeqCst)
    }

    /// `GatewayAdmission::acquire_connection` admits exactly
    /// `max_public_connections` connections, refuses the next with
    /// `ConnectionLimit` and holds nothing for it, in the budget or in the
    /// generation's retention; dropping every lease returns both to zero.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(start_paused = true)]
    async fn the_connection_beyond_the_ceiling_is_refused_by_admission_and_leaks_nothing() {
        let world = promoted_world(CONNECTION_CEILING_OCTET).await;
        let ceiling = world.admission.budget().limits.max_public_connections().get();
        let leases: Vec<GatewayConnectionLease> = (0..ceiling).map(|_| connect(&world)).collect();
        let retained = retained_connections(&leases[0]);
        assert_eq!(retained, ceiling, "every admitted connection is retained by the generation");

        let refused = world.admission.acquire_connection(now(&world));
        assert!(
            matches!(refused, Err(GatewayAdmissionError::ConnectionLimit)),
            "the connection beyond the ceiling is refused with ConnectionLimit: {:?}",
            refused.err()
        );
        let budget = world.admission.budget();
        assert_eq!(budget.public_connections.load(Ordering::SeqCst), ceiling);
        assert_eq!(retained_connections(&leases[0]), ceiling, "the refusal retained nothing");

        let probe = Arc::clone(&leases[0].snapshot);
        drop(leases);
        assert_eq!(budget.public_connections.load(Ordering::SeqCst), 0);
        assert_eq!(probe.retention.connections.load(Ordering::SeqCst), 0);
        drop(probe);
        world.stop().await;
    }

    /// While one request is in flight on a connection, a second `try_request`
    /// on the same lease is refused with `RequestLimit` (the delta's only
    /// request-shaped variant) and holds nothing; once the first request ends
    /// the connection serves the next.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(start_paused = true)]
    async fn a_second_request_on_a_connection_with_one_in_flight_is_refused() {
        let world = promoted_world(IN_FLIGHT_REQUEST_OCTET).await;
        let lease = connect(&world);
        let budget = world.admission.budget();

        let first = lease.try_request().unwrap_or_else(|error| panic!("first refused: {error:?}"));
        assert_eq!(budget.inflight_requests.load(Ordering::SeqCst), 1);
        assert_eq!(retained_requests(&lease), 1);

        let refused = lease.try_request();
        assert!(
            matches!(refused, Err(GatewayAdmissionError::RequestLimit)),
            "a second in-flight request is refused with RequestLimit: {:?}",
            refused.err()
        );
        assert_eq!(budget.inflight_requests.load(Ordering::SeqCst), 1, "the refusal took no slot");
        assert_eq!(retained_requests(&lease), 1, "the refusal retained nothing");
        assert_eq!(lease.requests_started.load(Ordering::SeqCst), 1, "the refusal counted nothing");

        drop(first);
        assert_eq!(budget.inflight_requests.load(Ordering::SeqCst), 0);
        assert_eq!(retained_requests(&lease), 0);
        let next = lease.try_request();
        assert!(next.is_ok(), "the connection serves the next request once the first ended");
        drop(next);
        drop(lease);
        assert_eq!(budget.public_connections.load(Ordering::SeqCst), 0);
        world.stop().await;
    }

    /// A connection serves exactly `max_requests_per_connection` requests, one
    /// at a time; the last is marked to close the connection after its
    /// response, the next is refused with `RequestLimit`, and the refusal holds
    /// no slot, no retention and no started-request count.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(start_paused = true)]
    async fn the_request_beyond_the_per_connection_ceiling_is_refused() {
        let world = promoted_world(PER_CONNECTION_CEILING_OCTET).await;
        let lease = connect(&world);
        let budget = world.admission.budget();
        let ceiling = lease.snapshot.budget.limits.max_requests_per_connection().get();

        for n in 1..=ceiling {
            let request = lease
                .try_request()
                .unwrap_or_else(|error| panic!("request {n} of {ceiling} refused: {error:?}"));
            assert_eq!(
                request.close_after_response(),
                n == ceiling,
                "only the last request of the connection closes it (request {n})"
            );
        }
        assert_eq!(lease.requests_started.load(Ordering::SeqCst), ceiling);
        assert_eq!(budget.inflight_requests.load(Ordering::SeqCst), 0, "every request ended");

        let refused = lease.try_request();
        assert!(
            matches!(refused, Err(GatewayAdmissionError::RequestLimit)),
            "the request beyond the per-connection ceiling is refused with RequestLimit: {:?}",
            refused.err()
        );
        assert_eq!(lease.requests_started.load(Ordering::SeqCst), ceiling);
        assert_eq!(budget.inflight_requests.load(Ordering::SeqCst), 0);
        assert_eq!(retained_requests(&lease), 0);
        assert!(!lease.request_active.load(Ordering::SeqCst), "the refusal left no request active");
        drop(lease);
        world.stop().await;
    }

    /// With the global in-flight request ceiling taken by other connections'
    /// requests, a request on this connection is refused with `RequestLimit`
    /// and gives back everything it had taken on the connection and the
    /// generation; once one global slot returns, the connection serves it.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(start_paused = true)]
    async fn a_request_beyond_the_global_request_ceiling_is_refused_and_leaks_nothing() {
        let world = promoted_world(GLOBAL_REQUEST_CEILING_OCTET).await;
        let lease = connect(&world);
        let budget = world.admission.budget();
        let ceiling = budget.limits.max_inflight_requests().get();
        let mut others = fill(ceiling, || budget.try_request());

        let refused = lease.try_request();
        assert!(
            matches!(refused, Err(GatewayAdmissionError::RequestLimit)),
            "a request beyond the global ceiling is refused with RequestLimit: {:?}",
            refused.err()
        );
        assert_eq!(budget.inflight_requests.load(Ordering::SeqCst), ceiling);
        assert_eq!(lease.requests_started.load(Ordering::SeqCst), 0, "the refusal counted nothing");
        assert!(!lease.request_active.load(Ordering::SeqCst), "the refusal left no request active");
        assert_eq!(retained_requests(&lease), 0, "the refusal retained nothing");

        drop(others.pop());
        let served = lease.try_request();
        assert!(served.is_ok(), "the connection serves a request once a global slot returned");
        drop(served);
        drop(others);
        assert_eq!(budget.inflight_requests.load(Ordering::SeqCst), 0);
        drop(lease);
        world.stop().await;
    }
}

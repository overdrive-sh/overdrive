//! The DNS responder's per-datagram `QueryObserver` (ADR-0307, DDD-12).
//!
//! Driving port: `DnsResponder::{new, probe, serve}` over a real UDP `:53`
//! socket, as the sibling `dns_responder_bind` scenarios drive it. Production
//! owner path: the responder's blocking serve loop. Evidence lane: integration
//! (guest `cgroup-default`; the bind needs root).
//!
//! Traces to: US-BENCH-07 AC (a) (resolver-internal timing is observable apart
//! from client timing) and DDD-12 ("behaviour with `NoQueryObserver` is
//! byte-identical to today's").

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "Tier-3 acceptance bodies; failures panic with informative messages"
)]

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use hickory_proto::op::{Message, MessageType, OpCode, Query};
use hickory_proto::rr::{Name, RecordType};
use hickory_proto::serialize::binary::BinEncodable;
use overdrive_control_plane::dns_responder::forwarder::Forwarder;
use overdrive_control_plane::dns_responder::frontend_addr_allocator::FrontendAddrAllocator;
use overdrive_control_plane::dns_responder::owned_ranges::OwnedAddressRanges;
use overdrive_control_plane::dns_responder::responder::DnsResponder;
use overdrive_control_plane::dns_responder::upstream_host::{
    HostUpstreamExchange, ResolvConfUpstreams,
};
use overdrive_control_plane::dns_responder::{NoQueryObserver, QueryObserver, QueryOutcome};
use overdrive_core::id::{MeshServiceName, NodeId, ServiceId, SpiffeId, WorkloadId};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationStore, ObservationWrite, ServiceBackendRow,
};
use overdrive_dataplane::allocators::VipRange;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_testing::overdrive_test;

/// Records every `observe` call, in arrival order.
#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<(QueryOutcome, Duration)>>,
}

impl QueryObserver for Recorder {
    fn observe(&self, outcome: QueryOutcome, service_time: Duration) {
        self.seen.lock().unwrap().push((outcome, service_time));
    }
}

/// A clock that advances by `STEP` on every `now()` read, so a duration measured on it is a
/// whole number of steps: a service time that is not a positive multiple of `STEP` was not
/// measured on this clock. (`SimClock` does not advance, so every duration on it is zero.)
struct SteppingClock {
    origin: Instant,
    reads: AtomicU32,
}

const STEP: Duration = Duration::from_millis(3);

impl SteppingClock {
    fn new() -> Self {
        Self { origin: Instant::now(), reads: AtomicU32::new(0) }
    }
}

#[async_trait]
impl Clock for SteppingClock {
    fn now(&self) -> Instant {
        self.origin + STEP * self.reads.fetch_add(1, Ordering::SeqCst)
    }
    fn unix_now(&self) -> Duration {
        Duration::from_secs(1_700_000_000)
    }
    async fn sleep(&self, _duration: Duration) {
        tokio::task::yield_now().await;
    }
}

fn query(name: &str, qtype: RecordType) -> Vec<u8> {
    let fqdn = Name::from_ascii(format!("{name}.svc.overdrive.local.")).expect("valid FQDN");
    let mut message = Message::new(0x4242, MessageType::Query, OpCode::Query);
    message.add_query(Query::query(fqdn, qtype));
    message.to_bytes().expect("encode query")
}

/// The four datagram shapes the responder distinguishes.
fn four_datagram_shapes() -> Vec<Vec<u8>> {
    vec![
        query("server", RecordType::A),
        query("server", RecordType::AAAA),
        query("ghost", RecordType::A),
        b"not a dns message".to_vec(),
    ]
}

/// Serve one responder over `datagrams` and return the replies in order
/// (`None` where the responder sent nothing within the wait).
async fn exchange(
    clock: Arc<dyn Clock>,
    observer: Arc<dyn QueryObserver>,
    datagrams: Vec<Vec<u8>>,
) -> Vec<Option<Vec<u8>>> {
    // SAFETY: getuid takes no arguments and cannot fail.
    assert_eq!(unsafe { libc::getuid() }, 0, "the :53 bind needs root; the guest runs as root");
    let store =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let frontend = FrontendAddrAllocator::new();
    frontend
        .assign(&MeshServiceName::new("server.svc.overdrive.local").expect("mesh name"))
        .expect("assign F");
    let backend = Backend {
        workload_identity: SpiffeId::for_workload(&WorkloadId::new("server").expect("workload id")),
        addr: std::net::SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 99, 0, 6), 8080)),
        weight: 1,
        healthy: true,
    };
    store
        .write(ObservationWrite::ServiceBackend(ServiceBackendRow {
            service_id: ServiceId::new(1).expect("service id"),
            vip: Ipv4Addr::new(10, 1, 0, 1),
            backends: vec![backend],
            updated_at: LogicalTimestamp {
                counter: 1,
                writer: NodeId::new("local").expect("node id"),
            },
        }))
        .await
        .expect("write backend row");
    let dns = Arc::new(DnsResponder::new(
        store as Arc<dyn ObservationStore>,
        clock,
        Ipv4Addr::new(100, 95, 0, 1),
        frontend,
        observer,
        Arc::new(Forwarder::new(
            Arc::new(ResolvConfUpstreams::from_path("/nonexistent/resolv.conf")),
            Arc::new(HostUpstreamExchange),
            Arc::new(SimClock::new()),
            OwnedAddressRanges::platform(&VipRange::default()),
            Ipv4Addr::new(100, 95, 0, 1),
            1,
        )),
    ));
    dns.probe().await.expect("probe binds :53");
    let serve = tokio::spawn(Arc::clone(&dns).serve());
    let replies = tokio::task::spawn_blocking(move || {
        std::thread::sleep(Duration::from_millis(200));
        let dst = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 2), 53);
        let client = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).expect("client");
        client.set_read_timeout(Some(Duration::from_millis(600))).expect("timeout");
        datagrams
            .iter()
            .map(|datagram| {
                client.send_to(datagram, dst).expect("send datagram");
                let mut buf = [0u8; 1500];
                client.recv_from(&mut buf).ok().map(|(n, _)| buf[..n].to_vec())
            })
            .collect::<Vec<_>>()
    })
    .await
    .expect("client task");
    dns.stop();
    tokio::time::timeout(Duration::from_secs(5), serve).await.expect("serve loop stops").ok();
    replies
}

/// The observer is told the outcome of every datagram: an answer, a name with no
/// record of the type, an unknown name, and a datagram that is not a query.
/// CONTRACT_SHAPE: bounded-change (the recorder's own buffer only).
/// Trace: DDD-12; US-BENCH-07 AC (a). Fault stimulus: a non-query datagram.
#[overdrive_test(guest = "cgroup-default", profile = "normal-gate")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_responder_reports_the_outcome_of_every_datagram_to_its_observer() {
    let recorder = Arc::new(Recorder::default());
    let replies = exchange(
        Arc::new(SimClock::new()),
        Arc::clone(&recorder) as Arc<dyn QueryObserver>,
        four_datagram_shapes(),
    )
    .await;
    assert_eq!(
        replies.iter().filter(|r| r.is_some()).count(),
        3,
        "three of four datagrams are answered"
    );
    // The observer is called on the serving thread before the reply send completes; allow it to land.
    std::thread::sleep(Duration::from_millis(200));
    let outcomes: Vec<QueryOutcome> = {
        let seen = recorder.seen.lock().unwrap();
        seen.iter().map(|(outcome, _)| *outcome).collect()
    };
    assert_eq!(
        outcomes,
        vec![
            QueryOutcome::Answered,
            QueryOutcome::NoData,
            QueryOutcome::NameError,
            QueryOutcome::Dropped
        ],
        "the observer must be told the outcome of every datagram, in arrival order"
    );
}

/// One service time is reported for each handled datagram, and it is a duration on the injected
/// clock: with a clock that advances by 3 ms per read, every reported time is a positive whole
/// number of 3 ms steps (the receive-to-send interval), never zero and never the client's wait.
/// CONTRACT_SHAPE: bounded-change (the recorder's own buffer only).
/// Trace: DDD-12 ("service_time runs from the receive to the send, on the injected `Clock`").
#[overdrive_test(guest = "cgroup-default", profile = "normal-gate")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_observed_service_time_is_measured_on_the_injected_clock() {
    let recorder = Arc::new(Recorder::default());
    exchange(
        Arc::new(SteppingClock::new()),
        Arc::clone(&recorder) as Arc<dyn QueryObserver>,
        four_datagram_shapes(),
    )
    .await;
    std::thread::sleep(Duration::from_millis(200));
    let seen = recorder.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 4, "every handled datagram reports a service time");
    assert!(
        seen.iter().all(|(_, t)| !t.is_zero() && t.as_nanos() % STEP.as_nanos() == 0),
        "each service time is a positive whole number of clock steps: {seen:?}"
    );
}

/// Replies are byte-for-byte identical whether the production `NoQueryObserver` or
/// a recording observer is installed: observing never changes what is answered.
/// CONTRACT_SHAPE: unbounded-preservation (the wire replies are the whole observable).
/// Trace: DDD-12 (byte-identical with `NoQueryObserver`).
#[overdrive_test(guest = "cgroup-default", profile = "normal-gate")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replies_are_byte_identical_with_the_production_observer_and_a_recording_one() {
    // One clock for both runs: the SOA serial is read from it, so the replies compare.
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let with_production =
        exchange(Arc::clone(&clock), Arc::new(NoQueryObserver), four_datagram_shapes()).await;
    let with_recorder =
        exchange(clock, Arc::new(Recorder::default()), four_datagram_shapes()).await;
    assert_eq!(with_production, with_recorder, "observing must not change a single reply byte");
    assert!(
        with_production.iter().take(3).all(Option::is_some),
        "the three query shapes are answered, so the comparison is not vacuous"
    );
    assert!(with_production[3].is_none(), "a non-query datagram is never answered");
}

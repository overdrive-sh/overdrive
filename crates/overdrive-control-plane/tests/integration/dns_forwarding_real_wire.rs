//! Guest public DNS resolution on the real wire (GH #382; ADR-0356 to ADR-0362).
//!
//! The PRODUCTION responder, built through `HostGuestDnsFactory::host()` and
//! driven over real UDP and TCP sockets inside a guest group, with the node's
//! upstream resolver supplied the way production supplies it: the host
//! resolver file `/etc/resolv.conf`, naming a scripted upstream
//! (`overdrive_testing::dns_upstream`) that this test also runs. The test
//! plays the guest's resolver: it sends queries to the guest gateway
//! (`GATEWAY`, a loopback alias that stands in for `100.95.0.1`; the gate
//! compares the datagram's destination with the configured gateway, so any
//! local address serves) and reads the replies.
//!
//! Driving ports: the responder's UDP and TCP port 53 (the guest's resolver
//! path) and `GuestDns::{probe, serve, stop}`. Driven ports: the host
//! resolver file and the upstream resolver on the wire. Nothing is injected
//! through a seam production does not have.
//!
//! Contract shapes: replies and upstream observations are `bounded-change`
//! (the only effects are the reply to the guest, one upstream exchange and the
//! episode warns); the guest-visible oracle is the reply, never an internal
//! counter. The pending scenarios (`should_panic(expected = ...)`) fail today
//! because the responder drops every non-mesh query; DELIVER removes the
//! marker one scenario at a time. Scenarios that characterise behaviour this
//! feature keeps (mesh answers, the gateway gate) are not pending.
//!
//! Lane: `integration-tests`, guest group `cgroup-default` (the responder binds
//! `:53` and the test rewrites the guest's own `/etc/resolv.conf`, so the body
//! runs in its own guest, never on the host).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::print_stderr,
    clippy::missing_const_for_fn,
    clippy::duration_suboptimal_units,
    reason = "Tier-3 acceptance body; failures panic with informative messages"
)]

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use hickory_proto::op::{Edns, Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::{Name, RData, RecordType};
use overdrive_control_plane::dns_responder::frontend_addr_allocator::FrontendAddrAllocator;
use overdrive_control_plane::dns_responder::owned_ranges::OwnedAddressRanges;
use overdrive_control_plane::dns_responder::responder::DnsResponderError;
use overdrive_control_plane::dns_responder::{
    GuestDns, GuestDnsDeps, GuestDnsFactory, HostGuestDnsFactory,
};
use overdrive_core::id::{MeshServiceName, NodeId, ServiceId, SpiffeId, WorkloadId};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationStore, ObservationWrite, ServiceBackendRow,
};
use overdrive_dataplane::allocators::VipRange;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_testing::dns_upstream::{
    Behaviour, HostResolverFile as ResolvConf, ScriptedDnsUpstream, Transport,
};
use overdrive_testing::overdrive_test;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

/// The guest gateway stand-in: a loopback alias the responder is told to treat
/// as the gateway. A datagram addressed here has `ipi_spec_dst == GATEWAY`.
const GATEWAY: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 10);
/// The scripted upstream, named by the host resolver file.
const UPSTREAM: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);
const GUEST_RESOLVER_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Event capture (the episode warns). Global: the forwarder logs from runtime
// threads, so a thread-local default would miss them.
// ---------------------------------------------------------------------------

static EVENTS: OnceLock<Arc<Mutex<Vec<String>>>> = OnceLock::new();

struct Capture(Arc<Mutex<Vec<String>>>);

impl<S: tracing::Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        if *meta.level() <= tracing::Level::WARN {
            self.0.lock().expect("events").push(meta.name().to_owned());
        }
    }
}

fn install_capture() -> Arc<Mutex<Vec<String>>> {
    Arc::clone(EVENTS.get_or_init(|| {
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(Capture(Arc::clone(&events)));
        let _ = tracing::subscriber::set_global_default(subscriber);
        events
    }))
}

fn warn_count(events: &Arc<Mutex<Vec<String>>>, name: &str) -> usize {
    events.lock().expect("events").iter().filter(|n| n.as_str() == name).count()
}

// ---------------------------------------------------------------------------
// The node: production responder + resolver file + scripted upstream
// ---------------------------------------------------------------------------

struct Node {
    dns: Arc<dyn GuestDns>,
    store: Arc<SimObservationStore>,
    frontend: FrontendAddrAllocator,
    resolv: ResolvConf,
    upstream: Option<ScriptedDnsUpstream>,
}

fn deps(
    store: &Arc<SimObservationStore>,
    frontend: &FrontendAddrAllocator,
    permits: usize,
) -> GuestDnsDeps {
    GuestDnsDeps {
        store: Arc::clone(store) as Arc<dyn ObservationStore>,
        clock: Arc::new(SimClock::new()) as Arc<dyn Clock>,
        gateway: GATEWAY,
        frontend: frontend.clone(),
        owned_ranges: OwnedAddressRanges::platform(&VipRange::default()),
        forward_permits: permits,
    }
}

impl Node {
    /// Boots the production responder (wildcard bind first), then the scripted
    /// upstream (its `SO_REUSEADDR` socket coexists with the wildcard one), and
    /// points the host resolver file at it.
    async fn boot(permits: usize) -> Self {
        let store =
            Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
        let frontend = FrontendAddrAllocator::new();
        let dns = HostGuestDnsFactory::host().responder(deps(&store, &frontend, permits));
        dns.probe().await.expect("probe binds the responder's sockets and List-seeds");
        tokio::spawn(Arc::clone(&dns).serve());
        let upstream = ScriptedDnsUpstream::start(UPSTREAM).expect("the scripted upstream binds");
        let resolv = ResolvConf::take();
        resolv.names(UPSTREAM);
        Self { dns, store, frontend, resolv, upstream: Some(upstream) }
    }

    fn upstream(&self) -> &ScriptedDnsUpstream {
        self.upstream.as_ref().expect("the upstream is running")
    }

    /// A mesh name with one backend, `healthy` or not; returns its stable frontend.
    async fn mesh_service(&self, workload: &str, healthy: bool) -> Ipv4Addr {
        let name = MeshServiceName::new(&format!("{workload}.svc.overdrive.local")).expect("name");
        let frontend = self.frontend.assign(&name).expect("assign a stable frontend");
        let backend = Backend {
            workload_identity: SpiffeId::for_workload(&WorkloadId::new(workload).expect("id")),
            addr: std::net::SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 99, 0, 6), 8080)),
            weight: 1,
            healthy,
        };
        self.store
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
            .expect("write the backend row");
        frontend
    }

    fn stop(&self) {
        self.dns.stop();
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        // A panicking scenario must still stop the blocking receive loop, or the
        // runtime teardown waits on it.
        self.dns.stop();
    }
}

// ---------------------------------------------------------------------------
// The guest's resolver: wire helpers
// ---------------------------------------------------------------------------

fn query(name: &str, qtype: RecordType, edns: bool) -> Vec<u8> {
    let mut m = Message::new(0x4242, MessageType::Query, OpCode::Query);
    m.metadata.recursion_desired = true;
    m.add_query(Query::query(Name::from_ascii(name).expect("name"), qtype));
    if edns {
        let mut e = Edns::new();
        e.set_max_payload(4096);
        m.set_edns(e);
    }
    m.to_vec().expect("encode")
}

fn query_labels(labels: &[&str]) -> Vec<u8> {
    let mut m = Message::new(0x4343, MessageType::Query, OpCode::Query);
    m.metadata.recursion_desired = true;
    let name = Name::from_labels(labels.iter().map(|l| l.as_bytes().to_vec())).expect("labels");
    m.add_query(Query::query(name, RecordType::A));
    m.to_vec().expect("encode")
}

/// One UDP exchange as a stub resolver does it: `None` when no reply arrives
/// within `wait`.
fn udp_lookup(dst: Ipv4Addr, wire: &[u8], wait: Duration) -> Option<(Message, Duration)> {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).expect("client socket");
    socket.set_read_timeout(Some(wait)).expect("timeout");
    let started = Instant::now();
    socket.send_to(wire, SocketAddrV4::new(dst, 53)).expect("send");
    let mut buf = [0u8; 4096];
    let (n, _) = socket.recv_from(&mut buf).ok()?;
    Some((Message::from_vec(&buf[..n]).expect("a decodable reply"), started.elapsed()))
}

fn gateway_lookup(wire: &[u8]) -> Option<(Message, Duration)> {
    udp_lookup(GATEWAY, wire, GUEST_RESOLVER_TIMEOUT)
}

/// One TCP exchange: `Ok(None)` when the node closed the connection without a
/// reply, `Err` when the connection was refused.
fn tcp_lookup(wire: &[u8]) -> std::io::Result<Option<Message>> {
    let mut stream = TcpStream::connect(SocketAddrV4::new(GATEWAY, 53))?;
    stream.set_read_timeout(Some(Duration::from_secs(6)))?;
    let len = u16::try_from(wire.len()).expect("small").to_be_bytes();
    let _ = stream.write_all(&len).and_then(|()| stream.write_all(wire));
    let mut prefix = [0u8; 2];
    if stream.read_exact(&mut prefix).is_err() {
        return Ok(None);
    }
    let mut body = vec![0u8; usize::from(u16::from_be_bytes(prefix))];
    stream.read_exact(&mut body)?;
    Ok(Some(Message::from_vec(&body).expect("a decodable reply")))
}

/// One TCP connection that sends `frame` as it is and reads: `Ok(true)` when the
/// node answered with bytes, `Ok(false)` when it closed the connection without a
/// reply (or reset it) before `within` elapsed, `Err` when the connection was
/// refused. A node that merely sits on the connection fails the oracle: the
/// close must come promptly, not by this side's read timeout.
fn tcp_raw(frame: &[u8], within: Duration) -> std::io::Result<bool> {
    let mut stream = TcpStream::connect(SocketAddrV4::new(GATEWAY, 53))?;
    stream.set_read_timeout(Some(within))?;
    let _ = stream.write_all(frame);
    let started = Instant::now();
    let mut byte = [0u8; 1];
    let replied = matches!(stream.read(&mut byte), Ok(n) if n > 0);
    assert!(
        replied || started.elapsed() < within,
        "the node must close a malformed connection, not hold it open: {:?}",
        started.elapsed()
    );
    Ok(replied)
}

fn a_records(m: &Message) -> Vec<Ipv4Addr> {
    m.answers
        .iter()
        .filter_map(|r| match &r.data {
            RData::A(a) => Some(a.0),
            _ => None,
        })
        .collect()
}

macro_rules! guest_test {
    ($(#[$meta:meta])* async fn $name:ident() $body:block) => {
        #[overdrive_test(guest = "cgroup-default", profile = "normal-gate")]
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        $(#[$meta])*
        async fn $name() $body
    };
}

// ===========================================================================
// US-GPD-1: a guest resolves a public name (AC-1, AC-3, K-GPD-1/2)
// ===========================================================================

guest_test! {
    async fn a_gateway_addressed_public_lookup_is_forwarded_and_answered() {
        let node = Node::boot(64).await;
        node.upstream().script("github.com", Behaviour::Answer(vec![Ipv4Addr::new(140, 82, 121, 4)]));
        let (reply, elapsed) = gateway_lookup(&query("github.com.", RecordType::A, false))
            .expect("a gateway-addressed public lookup must be answered");
        assert_eq!(a_records(&reply), vec![Ipv4Addr::new(140, 82, 121, 4)]);
        assert!(elapsed < Duration::from_millis(1000), "K-GPD-1: answered in {elapsed:?}");
        assert_eq!(node.upstream().seen_count("github.com"), 1, "one upstream exchange per lookup");
        node.stop();
    }
}

guest_test! {
    async fn a_nonexistent_public_name_gets_nxdomain_promptly() {
        let node = Node::boot(64).await;
        let (reply, elapsed) =
            gateway_lookup(&query("no-such-host-7f3a.example.com.", RecordType::A, false))
                .expect("a nonexistent public name must get a definite answer");
        assert_eq!(reply.metadata.response_code, ResponseCode::NXDomain, "the upstream's rcode relays");
        assert!(elapsed < Duration::from_millis(1000), "K-GPD-2: NXDOMAIN in {elapsed:?}");
        node.stop();
    }
}

guest_test! {
    /// A query addressed to a node address that is not the gateway is never
    /// forwarded (open-resolver gate, V-2): the node stays silent and the
    /// upstream sees nothing. Behaviour kept from today: not pending.
    async fn a_query_not_addressed_to_the_gateway_is_dropped_and_never_forwarded() {
        let node = Node::boot(64).await;
        node.upstream().script("github.com", Behaviour::Answer(vec![Ipv4Addr::new(140, 82, 121, 4)]));
        let reply = udp_lookup(Ipv4Addr::LOCALHOST, &query("github.com.", RecordType::A, false), Duration::from_millis(1500));
        assert!(reply.is_none(), "a non-gateway-addressed public query must get no reply: {reply:?}");
        assert!(node.upstream().seen().is_empty(), "nothing reached the upstream");
        node.stop();
    }
}

guest_test! {
    /// A gateway-addressed datagram that is not one IN-class QUERY with one
    /// question is dropped without a reply and never forwarded (DESIGN `WireReject`):
    /// a response (QR set), two questions, opcode STATUS, class other than IN.
    /// Behaviour kept from today: not pending; it guards the forwarder from
    /// answering or relaying what is not a query.
    async fn a_gateway_addressed_datagram_that_is_not_one_in_class_query_is_dropped_and_never_forwarded() {
        let node = Node::boot(64).await;
        node.upstream().script_default(Behaviour::Answer(vec![Ipv4Addr::new(140, 82, 121, 4)]));
        let base = query("github.com.", RecordType::A, false);
        let mut response = base.clone();
        response[2] |= 0x80; // QR: a response, not a query
        let mut two_questions = base.clone();
        two_questions[4..6].copy_from_slice(&2u16.to_be_bytes());
        two_questions.extend_from_slice(&base[12..]);
        let mut status = base.clone();
        status[2] = (status[2] & 0x87) | (2 << 3); // opcode STATUS
        let mut chaos = base.clone();
        let class_at = chaos.len() - 2;
        chaos[class_at..].copy_from_slice(&3u16.to_be_bytes()); // class CH
        for (what, wire) in [
            ("a response", response),
            ("two questions", two_questions),
            ("opcode STATUS", status),
            ("class CH", chaos),
        ] {
            let reply = udp_lookup(GATEWAY, &wire, Duration::from_millis(1000));
            assert!(reply.is_none(), "{what}: a datagram that is not one IN query gets no reply: {reply:?}");
        }
        assert!(node.upstream().seen().is_empty(), "none of them reached the upstream");
        node.stop();
    }
}

guest_test! {
    async fn a_tcp_connection_with_malformed_framing_is_closed_without_a_reply() {
        let node = Node::boot(64).await;
        node.upstream().script_default(Behaviour::Answer(vec![Ipv4Addr::new(140, 82, 121, 4)]));
        let wire = query("github.com.", RecordType::A, false);
        // A zero length frame.
        let replied = tcp_raw(&[0, 0], Duration::from_secs(2))
            .expect("the responder must serve TCP on the gateway");
        assert!(!replied, "a zero length frame is closed without a reply");
        // A frame longer than the 1500 byte limit shared with UDP.
        let mut oversize = 2000u16.to_be_bytes().to_vec();
        oversize.extend_from_slice(&wire);
        oversize.resize(2 + 2000, 0);
        let replied = tcp_raw(&oversize, Duration::from_secs(2)).expect("the listener accepts");
        assert!(!replied, "a frame over 1500 bytes is closed without a reply");
        assert!(node.upstream().seen().is_empty(), "neither frame reached the upstream");
        node.stop();
    }
}

// ===========================================================================
// Namespace boundary (GPD-2, AC-2, AC-5)
// ===========================================================================

guest_test! {
    async fn a_platform_zone_name_that_is_not_a_mesh_name_is_nxdomain_and_never_forwarded() {
        let node = Node::boot(64).await;
        node.upstream().script_default(Behaviour::Answer(vec![Ipv4Addr::new(1, 2, 3, 4)]));
        for wire in [
            query("foo.overdrive.local.", RecordType::A, false),
            query("overdrive.local.", RecordType::A, false),
            query("a.b.svc.overdrive.local.", RecordType::A, false),
            query("FOO.Overdrive.LOCAL.", RecordType::A, false),
            query_labels(&["a.svc", "overdrive", "local"]),
        ] {
            let (reply, _) = gateway_lookup(&wire)
                .expect("a platform-zone name that is not a mesh name must get NXDOMAIN, not silence");
            assert_eq!(reply.metadata.response_code, ResponseCode::NXDomain);
            assert!(a_records(&reply).is_empty());
        }
        assert!(node.upstream().seen().is_empty(), "no platform-zone name reaches an upstream");
        node.stop();
    }
}

guest_test! {
    async fn a_name_that_only_resembles_the_platform_zone_is_forwarded() {
        let node = Node::boot(64).await;
        node.upstream().script("evil-overdrive.local", Behaviour::Answer(vec![Ipv4Addr::new(5, 6, 7, 8)]));
        node.upstream().script("overdrive.local.example.com", Behaviour::Answer(vec![Ipv4Addr::new(5, 6, 7, 9)]));
        for name in ["evil-overdrive.local.", "overdrive.local.example.com."] {
            let (reply, _) = gateway_lookup(&query(name, RecordType::A, false))
                .expect("a name that only resembles the platform zone is an ordinary public name");
            assert_eq!(a_records(&reply).len(), 1, "{name}");
        }
        node.stop();
    }
}

guest_test! {
    /// Mesh names stay authoritative and local, and are never sent upstream
    /// (D-6). Behaviour kept from today: not pending.
    async fn mesh_names_still_resolve_locally_and_never_reach_the_upstream() {
        let node = Node::boot(64).await;
        let frontend = node.mesh_service("payments", true).await;
        node.upstream().script_default(Behaviour::Answer(vec![Ipv4Addr::new(9, 9, 9, 9)]));
        let (reply, _) = gateway_lookup(&query("payments.svc.overdrive.local.", RecordType::A, false))
            .expect("a healthy mesh name is answered by the node");
        assert_eq!(a_records(&reply), vec![frontend], "the stable frontend, as before this feature");
        let (reply, _) = gateway_lookup(&query("absent.svc.overdrive.local.", RecordType::A, false))
            .expect("an unknown mesh name is answered by the node");
        assert_eq!(reply.metadata.response_code, ResponseCode::NXDomain);
        // A mesh name whose only backend is unhealthy is withheld: NXDOMAIN, as
        // today, and still never sent upstream (AC-2).
        node.mesh_service("idle", false).await;
        let (reply, _) = gateway_lookup(&query("idle.svc.overdrive.local.", RecordType::A, false))
            .expect("a mesh name with no healthy backend is answered by the node");
        assert_eq!(reply.metadata.response_code, ResponseCode::NXDomain);
        assert!(a_records(&reply).is_empty(), "no address for a withheld mesh name");
        assert!(node.upstream().seen().is_empty(), "no mesh query reaches an upstream");
        node.stop();
    }
}

// ===========================================================================
// US-GPD-2: the owned-range guard on the real wire (AC-6, AC-7, AC-9)
// ===========================================================================

guest_test! {
    async fn an_upstream_answer_in_the_mesh_frontend_range_never_reaches_the_guest() {
        let node = Node::boot(64).await;
        node.upstream().script("build-cache.corp.example", Behaviour::Answer(vec![Ipv4Addr::new(10, 98, 0, 7)]));
        node.upstream().script("git.corp.example", Behaviour::Answer(vec![Ipv4Addr::new(10, 20, 4, 7)]));
        node.upstream().script(
            "github.com",
            Behaviour::Answer(vec![Ipv4Addr::new(140, 82, 121, 4), Ipv4Addr::new(10, 98, 0, 9)]),
        );
        let (blocked, _) = gateway_lookup(&query("build-cache.corp.example.", RecordType::A, true))
            .expect("an owned-range answer must still get a definite reply, never silence");
        assert!(a_records(&blocked).is_empty(), "the owned address is not delivered");
        let edns = blocked.edns.as_ref().expect("an EDNS query gets an OPT in the blocked reply");
        assert_eq!(edns.max_payload(), 1232);
        let (mixed, _) = gateway_lookup(&query("github.com.", RecordType::A, false)).expect("reply");
        assert!(a_records(&mixed).is_empty(), "a mixed set is emptied whole");
        let (benign, _) = gateway_lookup(&query("git.corp.example.", RecordType::A, false)).expect("reply");
        assert_eq!(a_records(&benign), vec![Ipv4Addr::new(10, 20, 4, 7)], "other private ranges pass");
        node.stop();
    }
}

// ===========================================================================
// US-GPD-3: upstream trouble (AC-10, AC-11, AC-12)
// ===========================================================================

guest_test! {
    async fn with_the_upstream_down_the_guest_gets_a_servfail_before_its_timeout_and_recovers() {
        let events = install_capture();
        let mut node = Node::boot(64).await;
        let frontend = node.mesh_service("payments", true).await;
        // The upstream goes away. The forward to its address lands on the
        // node itself, whose gateway gate drops it: the attempt times out.
        node.upstream = None;
        for _ in 0..3 {
            let (reply, elapsed) = gateway_lookup(&query("crates.io.", RecordType::A, false))
                .expect("with no upstream answering the guest must get a SERVFAIL, not silence");
            assert_eq!(reply.metadata.response_code, ResponseCode::ServFail);
            assert!(elapsed < GUEST_RESOLVER_TIMEOUT, "K-GPD-5: SERVFAIL in {elapsed:?}");
        }
        assert_eq!(warn_count(&events, "dns.forward.failed"), 1, "one episode warn for three failures");
        // Mesh answers during the outage (AC-11).
        let (mesh, _) = gateway_lookup(&query("payments.svc.overdrive.local.", RecordType::A, false))
            .expect("mesh answers during the outage");
        assert_eq!(a_records(&mesh), vec![frontend]);
        // The upstream returns: the next lookup succeeds with no restart (AC-12).
        let upstream = ScriptedDnsUpstream::start(UPSTREAM).expect("upstream returns");
        upstream.script("crates.io", Behaviour::Answer(vec![Ipv4Addr::new(52, 85, 1, 1)]));
        node.upstream = Some(upstream);
        let (reply, _) = gateway_lookup(&query("crates.io.", RecordType::A, false)).expect("reply");
        assert_eq!(a_records(&reply), vec![Ipv4Addr::new(52, 85, 1, 1)]);
        node.stop();
    }
}

guest_test! {
    async fn an_absent_resolver_file_is_a_servfail_not_a_boot_refusal_and_recovers_when_restored() {
        let node = Node::boot(64).await;
        node.upstream().script("crates.io", Behaviour::Answer(vec![Ipv4Addr::new(52, 85, 1, 1)]));
        node.resolv.remove();
        let (reply, elapsed) = gateway_lookup(&query("crates.io.", RecordType::A, false))
            .expect("an absent resolver file is a SERVFAIL, never silence");
        assert_eq!(reply.metadata.response_code, ResponseCode::ServFail);
        assert!(elapsed < Duration::from_millis(1000), "the failure is immediate: {elapsed:?}");
        node.resolv.names(UPSTREAM);
        let (reply, _) = gateway_lookup(&query("crates.io.", RecordType::A, false)).expect("reply");
        assert_eq!(a_records(&reply), vec![Ipv4Addr::new(52, 85, 1, 1)], "restored: no restart needed");
        node.stop();
    }
}

guest_test! {
    async fn a_resolver_file_naming_only_the_gateway_never_loops_the_forward_to_the_node() {
        let node = Node::boot(64).await;
        node.resolv.write(&format!("nameserver {GATEWAY}\n"));
        let (reply, _) = gateway_lookup(&query("crates.io.", RecordType::A, false))
            .expect("a resolver list naming only the gateway must be a SERVFAIL, not a loop");
        assert_eq!(reply.metadata.response_code, ResponseCode::ServFail);
        assert!(node.upstream().seen().is_empty());
        node.stop();
    }
}

// ===========================================================================
// TCP/53 (GPD-5, boundary scenario 2)
// ===========================================================================

guest_test! {
    async fn a_query_over_tcp_is_forwarded_over_tcp_and_answered_once_per_connection() {
        let node = Node::boot(64).await;
        node.upstream().script("github.com", Behaviour::Answer(vec![Ipv4Addr::new(140, 82, 121, 4)]));
        let reply = tcp_lookup(&query("github.com.", RecordType::A, false))
            .expect("the responder must serve TCP on the gateway")
            .expect("a framed reply");
        assert_eq!(a_records(&reply), vec![Ipv4Addr::new(140, 82, 121, 4)]);
        let seen = node.upstream().seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].transport, Transport::Tcp, "a TCP guest query is forwarded over TCP");
        node.stop();
    }
}

guest_test! {
    async fn the_probe_refuses_when_the_gateway_tcp_port_is_held() {
        let store =
            Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
        let frontend = FrontendAddrAllocator::new();
        let _held = TcpListener::bind(SocketAddrV4::new(GATEWAY, 53)).expect("hold the TCP port");
        let dns = HostGuestDnsFactory::host().responder(deps(&store, &frontend, 8));
        let Err(err) = dns.probe().await else {
            panic!("a held gateway TCP port must refuse the probe");
        };
        assert!(matches!(err, DnsResponderError::Bind { .. }), "the existing Bind refusal: {err:?}");
    }
}

// ===========================================================================
// Saturation and the shed (GPD-8, boundary scenario 4)
// ===========================================================================

guest_test! {
    async fn with_every_permit_held_a_further_query_is_shed_at_once_and_mesh_still_answers() {
        let events = install_capture();
        let node = Node::boot(2).await;
        let frontend = node.mesh_service("payments", true).await;
        node.upstream().script_default(Behaviour::Silent);
        // Two lookups hold the two permits while their upstream stays silent.
        let holders: Vec<_> = ["slow-a.example.", "slow-b.example."]
            .into_iter()
            .map(|name| {
                std::thread::spawn(move || gateway_lookup(&query(name, RecordType::A, false)))
            })
            .collect();
        std::thread::sleep(Duration::from_millis(400));

        let (shed, elapsed) = udp_lookup(GATEWAY, &query("third.example.", RecordType::A, false), Duration::from_secs(2))
            .expect("a UDP query beyond the permit bound must be shed with a SERVFAIL, not silence");
        assert_eq!(shed.metadata.response_code, ResponseCode::ServFail);
        assert!(elapsed < Duration::from_millis(500), "shed at once: {elapsed:?}");
        assert_eq!(node.upstream().seen_count("third.example"), 0, "a shed query opens no upstream exchange");

        let tcp = tcp_lookup(&query("fourth.example.", RecordType::A, false))
            .expect("the TCP listener accepts the connection");
        assert!(tcp.is_none(), "a TCP connection beyond the bound is closed unread");

        let (mesh, _) = gateway_lookup(&query("payments.svc.overdrive.local.", RecordType::A, false))
            .expect("mesh over UDP is unaffected by saturation");
        assert_eq!(a_records(&mesh), vec![frontend]);
        assert_eq!(warn_count(&events, "dns.forward.shed_started"), 1, "one start for the whole run");
        for holder in holders {
            let _ = holder.join();
        }
        node.stop();
    }
}

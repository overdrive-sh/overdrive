//! Family `dns-responder` (DDD-12): latency and throughput of the node-local DNS responder.
//!
//! The load is scheduled-arrival; the responder's own per-datagram service time is reported
//! beside the clients' view.
//!
//! One replicate creates a network namespace, constructs a `DnsResponder` directly with a
//! recording `QueryObserver` over a seeded name table, binds it inside the namespace and
//! drives it from client sockets inside the same namespace. Queries are sent on a fixed
//! schedule; each query's latency runs from its scheduled send time (so a responder that
//! falls behind is not forgiven), replies are matched by DNS id, and a reply that does not
//! arrive within the timeout, or is the wrong kind of answer, is a failure.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Barrier, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use hickory_proto::op::{Message, MessageType, OpCode, Query};
use hickory_proto::rr::{Name, RecordType};
use hickory_proto::serialize::binary::BinEncodable;
use overdrive_bench::{HostClock, HostInstant, Replicate, ReplicateFailure};
use overdrive_control_plane::dns_responder::frontend_addr_allocator::FrontendAddrAllocator;
use overdrive_control_plane::dns_responder::responder::DnsResponder;
use overdrive_control_plane::dns_responder::{QueryObserver, QueryOutcome};
use overdrive_core::id::{AllocationId, MeshServiceName, NodeId, ServiceId, SpiffeId, WorkloadId};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationStore, ObservationWrite, ServiceBackendRow,
};
use overdrive_host::SystemClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_testing::netns::NetNs;
use overdrive_testing_macros::overdrive_bench;

/// Queries sent one by one before the timed schedule, so the responder and the sockets are
/// warm when the first scheduled query arrives.
const WARMUP_QUERIES: u32 = 200;
/// The longest a reply may take, counted from the query's scheduled send time.
const REPLY_TIMEOUT: Duration = Duration::from_secs(1);
/// The delay between the clients being ready and the first scheduled query.
const START_OFFSET_NS: u64 = 100_000_000;
const NS_PER_SECOND: u64 = 1_000_000_000;
/// The address the queries are sent to inside the namespace; the responder's wildcard
/// socket receives it and source-pins the reply to it.
const DNS_DESTINATION: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 2), 53);
/// The resolvable names the unknown-name profile does not query.
const UNKNOWN_NAME_COUNT: u32 = 100;
const SERVICE_SUFFIX: &str = "svc.overdrive.local";
const CLIENT_LATENCY: &str = "client_latency_ns";
const CLIENT_THROUGHPUT: &str = "client_throughput_qps";
const RESOLVER_SERVICE: &str = "resolver_service_ns";
const ID_SPACE: u32 = 65_536;
const RECEIVE_POLL: Duration = Duration::from_millis(20);
const RESPONDER_STOP_BOUND: Duration = Duration::from_secs(5);

static NETNS_SEQUENCE: AtomicU32 = AtomicU32::new(0);

#[overdrive_bench(
    family = "dns-responder",
    substrate = native,
    arms = ["responder"],
    replicates = 30,
    profile(
        name = "a-record-1-name-1-client-2000qps",
        preparation = "scheduled-arrival load of 2000 qps for 5 seconds from 1 client socket; query shape A on a resolvable name; name table of 1 name"
    ),
    profile(
        name = "a-record-1000-names-4-clients-8000qps",
        preparation = "scheduled-arrival load of 8000 qps for 5 seconds from 4 client sockets; query shape A on resolvable names; name table of 1000 names"
    ),
    profile(
        name = "aaaa-no-data-100-names-1-client-2000qps",
        preparation = "scheduled-arrival load of 2000 qps for 5 seconds from 1 client socket; query shape AAAA on a resolvable name that has no AAAA record (no data); name table of 100 names"
    ),
    profile(
        name = "unknown-name-1-client-2000qps",
        preparation = "scheduled-arrival load of 2000 qps for 5 seconds from 1 client socket; query shape A on a name that is not in the name table (name error); name table of 100 other names"
    ),
    metric(name = "client_latency_ns", unit = ns, table = "client-observed", series),
    metric(name = "client_throughput_qps", unit = qps, table = "client-observed"),
    metric(name = "resolver_service_ns", unit = ns, table = "resolver-internal", series),
    clock(service = injected_monotonic),
    smoke(guest = "cgroup-default"),
)]
fn dns_query_latency(rep: &mut Replicate) -> Result<(), ReplicateFailure> {
    let shape = Shape::of(rep.profile())
        .ok_or_else(|| fail(&format!("profile `{}` has no shape", rep.profile())))?;
    note_shape(rep, shape);
    let sequence = NETNS_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    // Declared first, so it drops last: the responder and the clients are gone by then.
    let netns = NetNs::create(&format!("ovbdns-{}-{sequence}", std::process::id()))
        .map_err(|error| fail(&format!("creating the network namespace: {error}")))?;
    let recorder = Arc::new(Recorder::default());
    let responder = Responder::start(&netns.name, shape.seeded_names(), &recorder)?;
    let templates = Arc::new(shape.query_templates()?);
    let outcome = run_clients(&netns.name, shape, &templates, &recorder);
    responder.stop()?;
    let clients = outcome?;
    record_client_view(rep, shape, &clients)?;
    record_resolver_view(rep, &recorder)
}

fn fail(message: &str) -> ReplicateFailure {
    ReplicateFailure::fixture(message)
}

/// What a profile varies.
#[derive(Clone, Copy)]
struct Shape {
    kind: Kind,
    /// Resolvable names in the responder's name table.
    names: u32,
    clients: u32,
    qps: u32,
    seconds: u32,
}

/// The query shape and the answer it must get.
#[derive(Clone, Copy)]
enum Kind {
    /// A on a resolvable name: an address.
    Address,
    /// AAAA on a resolvable name: no data.
    NoData,
    /// A on a name the table does not hold: a name error.
    NameError,
}

impl Kind {
    const fn label(self) -> &'static str {
        match self {
            Self::Address => "A on a resolvable name",
            Self::NoData => "AAAA on a resolvable name (no data)",
            Self::NameError => "A on an unknown name (name error)",
        }
    }

    /// Whether `reply` is the answer this shape must get: the response code and the number
    /// of answer records in the DNS header.
    fn accepts(self, reply: &[u8]) -> bool {
        let (Some(flags), Some(count)) = (reply.get(3), reply.get(6..8)) else {
            return false;
        };
        let response_code = flags & 0x0f;
        let answers = u16::from_be_bytes([count[0], count[1]]);
        match self {
            Self::Address => response_code == 0 && answers >= 1,
            Self::NoData => response_code == 0 && answers == 0,
            Self::NameError => response_code == 3,
        }
    }
}

impl Shape {
    fn of(profile: &str) -> Option<Self> {
        let (kind, names, clients, qps) = match profile {
            "a-record-1-name-1-client-2000qps" => (Kind::Address, 1, 1, 2_000),
            "a-record-1000-names-4-clients-8000qps" => (Kind::Address, 1_000, 4, 8_000),
            "aaaa-no-data-100-names-1-client-2000qps" => (Kind::NoData, 100, 1, 2_000),
            "unknown-name-1-client-2000qps" => (Kind::NameError, UNKNOWN_NAME_COUNT, 1, 2_000),
            _ => return None,
        };
        Some(Self { kind, names, clients, qps, seconds: 5 })
    }

    /// The names the responder's table is seeded with.
    const fn seeded_names(self) -> u32 {
        self.names
    }

    const fn total_queries(self) -> u32 {
        self.qps * self.seconds
    }

    /// The gap between two scheduled queries.
    const fn period_ns(self) -> u64 {
        NS_PER_SECOND / self.qps as u64
    }

    /// One encoded query per name queried, with the DNS id left to the sender.
    fn query_templates(self) -> Result<Vec<Vec<u8>>, ReplicateFailure> {
        let record_type = match self.kind {
            Kind::NoData => RecordType::AAAA,
            Kind::Address | Kind::NameError => RecordType::A,
        };
        (0..self.names)
            .map(|index| {
                let label = match self.kind {
                    Kind::NameError => format!("missing-{index:05}"),
                    Kind::Address | Kind::NoData => workload_label(index),
                };
                encode_query(&label, record_type)
            })
            .collect()
    }
}

fn note_shape(rep: &mut Replicate, shape: Shape) {
    rep.note("arrival_model", "scheduled-arrival");
    rep.note("query_shape", shape.kind.label());
    rep.note("name_table_size", &shape.names.to_string());
    rep.note("client_sockets", &shape.clients.to_string());
    rep.note("rate_qps", &shape.qps.to_string());
    rep.note("duration_seconds", &shape.seconds.to_string());
    rep.note("reply_timeout_ms", &REPLY_TIMEOUT.as_millis().to_string());
}

fn workload_label(index: u32) -> String {
    format!("w{index:05}")
}

fn encode_query(label: &str, record_type: RecordType) -> Result<Vec<u8>, ReplicateFailure> {
    let name = Name::from_ascii(format!("{label}.{SERVICE_SUFFIX}."))
        .map_err(|error| fail(&format!("encoding the name `{label}`: {error}")))?;
    let mut message = Message::new(0, MessageType::Query, OpCode::Query);
    message.add_query(Query::query(name, record_type));
    message.to_bytes().map_err(|error| fail(&format!("encoding the query for `{label}`: {error}")))
}

/// Records the outcome and service time of every datagram the responder handles.
#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<(QueryOutcome, Duration)>>,
}

impl Recorder {
    fn take(&self) -> Vec<(QueryOutcome, Duration)> {
        std::mem::take(&mut *self.seen.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl QueryObserver for Recorder {
    fn observe(&self, outcome: QueryOutcome, service_time: Duration) {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner).push((outcome, service_time));
    }
}

/// Enters the network namespace `name` on the calling thread. Threads the caller spawns
/// afterwards start in it too, and sockets created on the thread belong to it.
fn enter_netns(name: &str) -> Result<(), String> {
    let handle = std::fs::File::open(format!("/var/run/netns/{name}"))
        .map_err(|error| format!("opening namespace {name}: {error}"))?;
    // SAFETY: `handle` is a live descriptor of a network namespace for the call's duration.
    let status = unsafe { libc::setns(handle.as_raw_fd(), libc::CLONE_NEWNET) };
    if status == 0 {
        Ok(())
    } else {
        Err(format!("entering namespace {name}: {}", std::io::Error::last_os_error()))
    }
}

/// The responder, serving on its own thread inside the namespace until stopped.
struct Responder {
    stop: Sender<()>,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl Responder {
    fn start(netns: &str, names: u32, recorder: &Arc<Recorder>) -> Result<Self, ReplicateFailure> {
        let (stop, stopped) = channel();
        let (ready_sender, ready) = channel();
        let netns = netns.to_owned();
        let observer = Arc::clone(recorder);
        let thread = std::thread::Builder::new()
            .name("dns-responder".to_owned())
            .spawn(move || serve_until_stopped(&netns, names, observer, &ready_sender, &stopped))
            .map_err(|error| fail(&format!("spawning the responder thread: {error}")))?;
        let responder = Self { stop, thread: Some(thread) };
        match ready.recv() {
            Ok(Ok(())) => Ok(responder),
            Ok(Err(reason)) => Err(fail(&reason)),
            Err(_) => Err(fail("the responder thread ended before it was ready")),
        }
    }

    /// Stops the responder and waits for its thread.
    fn stop(mut self) -> Result<(), ReplicateFailure> {
        self.finish().map_err(|reason| fail(&reason))
    }

    fn finish(&mut self) -> Result<(), String> {
        let Some(thread) = self.thread.take() else { return Ok(()) };
        // A thread that already ended has dropped its receiver; its result is read below.
        let _ = self.stop.send(());
        thread.join().map_err(|_| "the responder thread panicked".to_owned())?
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// The responder thread: seeds the name table, binds the responder inside the namespace,
/// reports readiness, serves until stopped.
fn serve_until_stopped(
    netns: &str,
    names: u32,
    observer: Arc<Recorder>,
    ready: &Sender<Result<(), String>>,
    stopped: &Receiver<()>,
) -> Result<(), String> {
    let started = start_responder(netns, names, observer);
    let (runtime, dns, serve) = match started {
        Ok(started) => started,
        Err(reason) => {
            let _ = ready.send(Err(reason.clone()));
            return Err(reason);
        }
    };
    let _ = ready.send(Ok(()));
    // The sender ends this wait by sending, or by being dropped.
    let _ = stopped.recv();
    dns.stop();
    let served =
        runtime.block_on(async { tokio::time::timeout(RESPONDER_STOP_BOUND, serve).await });
    runtime.shutdown_timeout(RESPONDER_STOP_BOUND);
    served.map(drop).map_err(|_| "the responder's serve loop did not stop in time".to_owned())
}

type Started = (tokio::runtime::Runtime, Arc<DnsResponder>, tokio::task::JoinHandle<()>);

fn start_responder(netns: &str, names: u32, observer: Arc<Recorder>) -> Result<Started, String> {
    enter_netns(netns)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| format!("building the responder runtime: {error}"))?;
    let (dns, serve) = runtime.block_on(async {
        let dns = build_responder(names, observer).await?;
        dns.probe().await.map_err(|error| format!("binding the responder: {error}"))?;
        let serve = tokio::spawn(Arc::clone(&dns).serve());
        Ok::<_, String>((dns, serve))
    })?;
    Ok((runtime, dns, serve))
}

/// A responder over a name table of `names` resolvable names, each with a bound frontend
/// address and one healthy backend, observed by `observer`.
async fn build_responder(names: u32, observer: Arc<Recorder>) -> Result<Arc<DnsResponder>, String> {
    let node = NodeId::new("local").map_err(|error| format!("node id: {error}"))?;
    let store = Arc::new(SimObservationStore::single_peer(node.clone(), 0));
    let frontend = FrontendAddrAllocator::new();
    for index in 0..names {
        let label = workload_label(index);
        let mesh_name = MeshServiceName::new(&format!("{label}.{SERVICE_SUFFIX}"))
            .map_err(|error| format!("mesh name `{label}`: {error}"))?;
        frontend.assign(&mesh_name).map_err(|error| format!("assigning `{label}`: {error}"))?;
        let workload =
            WorkloadId::new(&label).map_err(|error| format!("workload id `{label}`: {error}"))?;
        let alloc =
            AllocationId::new("alloc-1").map_err(|error| format!("allocation id: {error}"))?;
        let service_id = ServiceId::new(u64::from(index) + 1)
            .map_err(|error| format!("service id {index}: {error}"))?;
        let backend = Backend {
            alloc: SpiffeId::for_allocation(&workload, &alloc),
            addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 99, 0, 6), 8080)),
            weight: 1,
            healthy: true,
        };
        let row = ServiceBackendRow {
            service_id,
            vip: Ipv4Addr::new(10, 1, 0, 1),
            backends: vec![backend],
            updated_at: LogicalTimestamp { counter: 1, writer: node.clone() },
        };
        store
            .write(ObservationWrite::ServiceBackend(row))
            .await
            .map_err(|error| format!("writing the backend row for `{label}`: {error}"))?;
    }
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    Ok(Arc::new(DnsResponder::new(
        store as Arc<dyn ObservationStore>,
        clock,
        Ipv4Addr::new(100, 95, 0, 1),
        frontend,
        observer,
    )))
}

/// What one client socket measured.
#[derive(Default)]
struct ClientOutcome {
    /// Latency in nanoseconds of every query answered correctly within the timeout.
    latencies: Vec<f64>,
    /// Queries the client was due to send.
    due: u64,
    /// Nanoseconds from the client's start to its last correct reply.
    last_reply_ns: u64,
}

/// Runs every client socket through the schedule and returns their outcomes.
fn run_clients(
    netns: &str,
    shape: Shape,
    templates: &Arc<Vec<Vec<u8>>>,
    recorder: &Recorder,
) -> Result<Vec<ClientOutcome>, ReplicateFailure> {
    let ready = Arc::new(Barrier::new(shape.clients as usize + 1));
    let mut threads = Vec::with_capacity(shape.clients as usize);
    for index in 0..shape.clients {
        let netns = netns.to_owned();
        let templates = Arc::clone(templates);
        let ready = Arc::clone(&ready);
        let thread = std::thread::Builder::new()
            .name(format!("dns-client-{index}"))
            .spawn(move || run_client(&netns, index, shape, &templates, &ready))
            .map_err(|error| fail(&format!("spawning a client thread: {error}")))?;
        threads.push(thread);
    }
    // Every client is bound and warm: the warm-up datagrams are not part of the measurement.
    ready.wait();
    drop(recorder.take());
    let mut outcomes = Vec::with_capacity(threads.len());
    let mut failures = Vec::new();
    for thread in threads {
        match thread.join() {
            Ok(Ok(outcome)) => outcomes.push(outcome),
            Ok(Err(reason)) => failures.push(reason),
            Err(_) => failures.push("a client thread panicked".to_owned()),
        }
    }
    if failures.is_empty() { Ok(outcomes) } else { Err(fail(&failures.join("; "))) }
}

fn run_client(
    netns: &str,
    index: u32,
    shape: Shape,
    templates: &[Vec<u8>],
    ready: &Barrier,
) -> Result<ClientOutcome, String> {
    let prepared = prepare_client(netns, templates);
    // Every client reaches the barrier, so one failed setup cannot strand the others.
    ready.wait();
    let socket = prepared?;
    let base = HostClock.now();
    let sending_done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let receiver = scope.spawn(|| receive_replies(&socket, index, shape, base, &sending_done));
        let sent = send_schedule(&socket, index, shape, templates, base);
        sending_done.store(true, Ordering::Release);
        let outcome = receiver.join().map_err(|_| "a client receiver panicked".to_owned())?;
        sent.map(|()| outcome)
    })
}

/// Binds the client socket inside the namespace and warms the responder with sequential
/// queries.
fn prepare_client(netns: &str, templates: &[Vec<u8>]) -> Result<UdpSocket, String> {
    enter_netns(netns)?;
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| format!("binding the client socket: {error}"))?;
    socket
        .set_read_timeout(Some(REPLY_TIMEOUT))
        .map_err(|error| format!("setting the client read timeout: {error}"))?;
    let mut buffer = [0_u8; 1500];
    for warmup in 0..WARMUP_QUERIES {
        let template = &templates[warmup as usize % templates.len()];
        let mut query = template.clone();
        set_id(&mut query, 0xffff);
        socket
            .send_to(&query, DNS_DESTINATION)
            .map_err(|error| format!("sending a warm-up query: {error}"))?;
        socket
            .recv_from(&mut buffer)
            .map_err(|error| format!("no reply to warm-up query {warmup}: {error}"))?;
    }
    socket
        .set_read_timeout(Some(RECEIVE_POLL))
        .map_err(|error| format!("setting the client poll timeout: {error}"))?;
    Ok(socket)
}

fn set_id(query: &mut [u8], id: u16) {
    query[..2].copy_from_slice(&id.to_be_bytes());
}

/// The number of queries client `index` is due to send.
const fn queries_due(shape: Shape, index: u32) -> u32 {
    let total = shape.total_queries();
    if index >= total { 0 } else { (total - index).div_ceil(shape.clients) }
}

/// The scheduled send time of the `sequence`th query of client `index`, from the client's
/// start. Queries interleave across clients: query `n` of the schedule belongs to client
/// `n % clients`.
const fn scheduled_ns(shape: Shape, index: u32, sequence: u32) -> u64 {
    let position = sequence as u64 * shape.clients as u64 + index as u64;
    START_OFFSET_NS + position * shape.period_ns()
}

fn elapsed_ns(base: HostInstant) -> u64 {
    u64::try_from(HostClock.now().duration_since(base).as_nanos()).unwrap_or(u64::MAX)
}

fn send_schedule(
    socket: &UdpSocket,
    index: u32,
    shape: Shape,
    templates: &[Vec<u8>],
    base: HostInstant,
) -> Result<(), String> {
    let due = queries_due(shape, index);
    if due > ID_SPACE {
        return Err(format!("client {index} is due {due} queries, more than the DNS id space"));
    }
    let mut query = Vec::with_capacity(512);
    for sequence in 0..due {
        let target = scheduled_ns(shape, index, sequence);
        wait_until(base, target);
        let position = u64::from(sequence) * u64::from(shape.clients) + u64::from(index);
        let template = &templates[usize::try_from(position).unwrap_or_default() % templates.len()];
        query.clear();
        query.extend_from_slice(template);
        set_id(&mut query, u16::try_from(sequence).unwrap_or(u16::MAX));
        // A query that cannot be sent is never answered: the receiver counts it a timeout.
        let _ = socket.send_to(&query, DNS_DESTINATION);
    }
    Ok(())
}

/// Waits until `target` nanoseconds after `base`: sleeping while far, spinning when near.
fn wait_until(base: HostInstant, target: u64) {
    loop {
        let now = elapsed_ns(base);
        if now >= target {
            return;
        }
        let remaining = target - now;
        if remaining > 200_000 {
            std::thread::sleep(Duration::from_nanos(remaining - 100_000));
        } else {
            std::hint::spin_loop();
        }
    }
}

fn receive_replies(
    socket: &UdpSocket,
    index: u32,
    shape: Shape,
    base: HostInstant,
    sending_done: &AtomicBool,
) -> ClientOutcome {
    let due = queries_due(shape, index);
    let last_scheduled = if due == 0 { 0 } else { scheduled_ns(shape, index, due - 1) };
    let give_up_at = last_scheduled + u64::try_from(REPLY_TIMEOUT.as_nanos()).unwrap_or(u64::MAX);
    let mut answered = vec![false; due as usize];
    let mut accounted = 0_u32;
    let mut outcome = ClientOutcome { due: u64::from(due), ..ClientOutcome::default() };
    let mut buffer = [0_u8; 1500];
    loop {
        let finished = sending_done.load(Ordering::Acquire);
        if finished && (accounted == due || elapsed_ns(base) > give_up_at) {
            return outcome;
        }
        let Ok((length, _)) = socket.recv_from(&mut buffer) else { continue };
        let received_at = elapsed_ns(base);
        let reply = &buffer[..length];
        let Some(id) =
            reply.get(..2).map(|bytes| u32::from(u16::from_be_bytes([bytes[0], bytes[1]])))
        else {
            continue;
        };
        let Some(slot) = answered.get_mut(id as usize) else { continue };
        if *slot {
            continue;
        }
        *slot = true;
        accounted += 1;
        let scheduled = scheduled_ns(shape, index, id);
        let latency = received_at.saturating_sub(scheduled);
        let within_timeout = u128::from(latency) <= REPLY_TIMEOUT.as_nanos();
        if shape.kind.accepts(reply) && within_timeout {
            #[allow(
                clippy::cast_precision_loss,
                reason = "a latency in nanoseconds is far below 2^52"
            )]
            outcome.latencies.push(latency as f64);
            outcome.last_reply_ns = outcome.last_reply_ns.max(received_at);
        }
    }
}

/// Records what the clients observed: every latency, a failure for the queries that were
/// not answered correctly in time, and the answered rate.
fn record_client_view(
    rep: &mut Replicate,
    shape: Shape,
    clients: &[ClientOutcome],
) -> Result<(), ReplicateFailure> {
    let due: u64 = clients.iter().map(|client| client.due).sum();
    let mut latencies =
        Vec::with_capacity(clients.iter().map(|client| client.latencies.len()).sum());
    for client in clients {
        latencies.extend_from_slice(&client.latencies);
    }
    let answered = latencies.len() as u64;
    if answered == 0 {
        return Err(ReplicateFailure::measurement(&format!(
            "no query of {due} was answered correctly within {} ms of its scheduled send time",
            REPLY_TIMEOUT.as_millis()
        )));
    }
    rep.record_series(CLIENT_LATENCY, &latencies)?;
    if answered < due {
        rep.record_failure(
            CLIENT_LATENCY,
            &format!(
                "{} of {due} queries were not answered correctly within {} ms of their scheduled send time",
                due - answered,
                REPLY_TIMEOUT.as_millis()
            ),
        )?;
    }
    let last_reply_ns = clients.iter().map(|client| client.last_reply_ns).max().unwrap_or(0);
    let scheduled_window_ns = u64::from(shape.seconds) * NS_PER_SECOND;
    let window_ns = last_reply_ns.saturating_sub(START_OFFSET_NS).max(scheduled_window_ns);
    #[allow(
        clippy::cast_precision_loss,
        reason = "counts and nanosecond totals are far below 2^52"
    )]
    let throughput = answered as f64 / (window_ns as f64 / NS_PER_SECOND as f64);
    rep.record(CLIENT_THROUGHPUT, throughput)
}

/// Records the responder's own service time of every datagram it answered.
fn record_resolver_view(rep: &mut Replicate, recorder: &Recorder) -> Result<(), ReplicateFailure> {
    let seen = recorder.take();
    let dropped = seen.iter().filter(|(outcome, _)| *outcome == QueryOutcome::Dropped).count();
    if dropped > 0 {
        rep.note("responder_dropped_datagrams", &dropped.to_string());
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "a service time in nanoseconds is far below 2^52"
    )]
    let service: Vec<f64> = seen
        .iter()
        .filter(|(outcome, _)| *outcome != QueryOutcome::Dropped)
        .map(|(_, time)| time.as_nanos() as f64)
        .collect();
    if service.is_empty() {
        return rep.record_failure(RESOLVER_SERVICE, "the responder observed no handled datagram");
    }
    rep.record_series(RESOLVER_SERVICE, &service)
}

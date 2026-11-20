//! Family `mtls-path` (DDD-13): the cost of the host agent's mTLS hop against a plaintext
//! baseline, for traffic leaving a workload (`egress`) and traffic arriving at one (`ingress`).
//!
//! Both profiles drive the same closed-loop frame protocol over each arm. A frame is an
//! 8-byte big-endian length followed by that many bytes; the server discards the bytes and
//! answers with a fixed 64-byte reply. One replicate measures, in order: connection
//! establishment, handshake, first byte (100 sequential connections), tail latency (10,000
//! request and response round trips on one connection) and sustained throughput (one
//! 256 MiB frame on one connection).
//!
//! `egress` composes the node's worker: `MtlsInterceptWorker` over the real
//! `HostMtlsIntercept` and `HostMtlsEnforcement`, and a rustls mesh peer. The dialer is
//! plaintext (the identity-unaware workload); the mTLS is on the inter-agent hop to the peer.
//! `ingress` serves `Routed::Inbound` through `HostMtlsEnforcement` over a loopback leg C,
//! with the guest end an in-memory stream. The handshake is the duration of
//! `MtlsEnforcement::enforce`, taken by a timing decorator in front of the real enforcement;
//! the baseline arm has no enforcement, so the metric is absent there.
//!
//! Every thread the family spawns (the client that runs the measurement, the peer's accept
//! and session threads, the ingress accept thread) carries the `ovbm-` name prefix, so a
//! per-thread accounting of the run can set them apart from the agent's threads. The ingress
//! relays run on the runtime's own worker threads, where the agent's enforcement runs too.

#![allow(
    clippy::cast_precision_loss,
    reason = "counts and nanosecond totals of one replicate are far below 2^52"
)]

use std::fmt::{Debug, Display};
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_bench::{HostClock, HostInstant, Replicate, ReplicateFailure};
use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestDialError, GuestStream, GuestStreamDial, GuestStreamEnd, GuestStreamIo,
};
use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GUEST_FLOW_CONTROL_PORT, GUEST_FLOW_STREAM_PORT, GUEST_NODE_PREFIX, GuestCid,
    GuestFlowPorts, GuestTransportAssignment, LaunchToken,
};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
use overdrive_core::traits::driver::{AllocationSpec, DriverPayload, Resources, VmPayload};
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnection, InterceptedConnection, MtlsEnforcement, MtlsLimits, PumpLiveness, Routed,
};
use overdrive_core::traits::mtls_resolve::{
    MtlsResolution, MtlsResolve, MtlsResolveError, ResolvedBackend,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{AllocationId, CertSerial, SpiffeId};
use overdrive_dataplane::mtls::HostMtlsEnforcement;
use overdrive_host::SystemClock;
use overdrive_testing::cidr_lease::TestCidrLease;
use overdrive_testing_macros::overdrive_bench;
use overdrive_worker::mtls_intercept_port::HostMtlsIntercept;
use overdrive_worker::mtls_intercept_worker::{ForwardedOutbound, MtlsInterceptWorker};
use parking_lot::Mutex;
use rcgen::string::Ia5String;
use rcgen::{CertificateParams, Issuer, KeyPair, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use socket2::{Domain, SockAddr, Socket, Type};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::runtime::Runtime;

mod cost;

use cost::{Snapshot, host_ratios, open_sockets, pump_split, resident_bytes};

const BASELINE: &str = "plaintext-baseline";
const COMPOSED: &str = "composed-mtls";

const ESTABLISHMENT: &str = "connection_establishment_ns";
const HANDSHAKE: &str = "handshake_ns";
const FIRST_BYTE: &str = "first_byte_ns";
const THROUGHPUT: &str = "throughput_bytes_per_second";
const TAIL: &str = "tail_latency_ns";
const HOST_CPU_USER: &str = "host_cpu_user_ratio";
const HOST_CPU_SYSTEM: &str = "host_cpu_system_ratio";
const HOST_CPU_IRQ: &str = "host_cpu_irq_ratio";
const HOST_CPU_SOFTIRQ: &str = "host_cpu_softirq_ratio";
const MEMORY: &str = "memory_bytes";
const SOCKETS: &str = "socket_count";
const CONCURRENCY: &str = "connection_concurrency";
const FORWARD_CPU: &str = "forward_userspace_cpu_ratio";
const RETURN_CPU: &str = "return_splice_cpu_ratio";
const INGRESS_PUMP_REASON: &str = "asynchronous relay on shared runtime threads";
const BASELINE_PUMP_REASON: &str = "the plaintext baseline arm has no mTLS pump threads, so there is no forward or return work to split";
const HANDSHAKE_BASELINE_REASON: &str =
    "the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time";

/// Sequential connections before the measured ones, so the paths are warm.
const WARMUP_CONNECTIONS: u32 = 10;
/// Sequential connections, each yielding one connect, one first-byte and one handshake sample.
const CONNECTIONS: u32 = 100;
const WARMUP_ROUND_TRIPS: u32 = 200;
const ROUND_TRIPS: u32 = 10_000;
/// The bytes of the one bulk frame the throughput is taken over.
const BULK_BYTES: u64 = 256 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;
const HEADER_LEN: usize = 8;
const REQUEST_LEN: usize = 64;
const REPLY: [u8; 64] = [b'r'; 64];
const IO_TIMEOUT: Duration = Duration::from_secs(30);
const SETTLE_BOUND: Duration = Duration::from_secs(10);
const DUPLEX_BYTES: usize = 256 * 1024;
/// The guest CID of the intercepted allocation; the guest itself is never reached.
const GUEST_CID: u32 = GUEST_CID_BASE + 1;
/// The name prefix of every thread the family spawns.
const THREAD_PREFIX: &str = "ovbm-";

const PEER_SNI: &str = "peer.overdrive.local";
const SERVER_SNI: &str = "server.overdrive.local";
const CLIENT_SPIFFE: &str = "spiffe://overdrive.local/ns/default/sa/bench-client";
const SERVER_SPIFFE: &str = "spiffe://overdrive.local/ns/default/sa/bench-server";
const PEER_SPIFFE: &str = "spiffe://overdrive.local/ns/default/sa/bench-peer";
const EGRESS_ALLOC: &str = "bench-egress";
const INGRESS_ALLOC: &str = "bench-ingress";
const LEASE_OWNER: &str = "overdrive-bench-mtls-path-egress";
/// The service port the workload dialled; nothing listens on it, the registered backend is
/// what the flow reaches.
const DIALLED_SERVICE_PORT: u16 = 443;
/// A VM backend inside the node guest prefix, so the enforcement takes the guest path.
const GUEST_BACKEND: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, 7), 8080);

#[overdrive_bench(
    family = "mtls-path",
    substrate = native,
    arms = ["plaintext-baseline", "composed-mtls"],
    replicates = 30,
    profile(
        name = "egress",
        preparation = "closed-loop (each request waits for its response before the next is sent; no scheduled arrival) over the frame protocol, plaintext workload dialler to a mesh peer on loopback; 10 warm-up then 100 sequential connections (connect, first byte, handshake), then one connection of 200 warm-up and 10000 timed 64-byte round trips (tail latency) and one connection carrying one 256 MiB frame (throughput). Baseline arm plaintext-baseline: the same dialler, bound the same way, connects straight to a plaintext TCP peer with no intercept worker, no nft rules and no TLS. Arm composed-mtls: the dialler's flow is registered with the real intercept worker and captured on leg F, which runs mTLS over kTLS to a rustls mesh peer that requires a client certificate; peer tickets disabled; handshake is the enforce duration; resource rows are whole-host scope (the aggregate cpu line of /proc/stat over every CPU and every process, no cgroup figure; each ratio is the class's tick delta over the delta of all CPU time, idle included, user includes nice) sampled across the 256 MiB throughput window; irq and softirq are tick-sampled on this kernel (irq_time_accounting is off, so irq reads 0 and softirq is coarse); memory (VmRSS) and open sockets of the benchmark process are taken at the window's end; connection_concurrency is the peak number of client sessions the family held open (opened and not yet dropped) during that window; forward_userspace_cpu_ratio and return_splice_cpu_ratio are the mtls-out-fwd and mtls-out-ret threads' share of the CPU the process's threads outside the family's own ovbm- threads accrued in that window; the window carries the request one way, so the return pump is the light one; thread CPU is read in 10 ms ticks, so the split is a lower bound at the tick (record note pump_heavy_to_light_lower_bound is the heavy-to-light ratio) and a return_splice_cpu_ratio of 0 means below one tick, because the sustained window carries data forward only; the TLS mesh peer runs on the same host as the agent and may co-limit throughput and tail latency, so the numbers are not pure agent cost"
    ),
    profile(
        name = "ingress",
        preparation = "closed-loop (each request waits for its response before the next is sent; no scheduled arrival) over the frame protocol, client to the node's inbound path on loopback with an in-memory guest end; 10 warm-up then 100 sequential connections (connect, first byte, handshake), then one connection of 200 warm-up and 10000 timed 64-byte round trips (tail latency) and one connection carrying one 256 MiB frame (throughput). Baseline arm plaintext-baseline: a plaintext client to a loopback listener that bridges each connection to the same in-memory guest sink, with no TLS and no enforcement. Arm composed-mtls: a rustls client presenting a node-trusted certificate to the real HostMtlsEnforcement serving Routed::Inbound over loopback leg C; the guest transport is excluded, and first byte includes the client side of the TLS handshake; resource rows are whole-host scope (the aggregate cpu line of /proc/stat over every CPU and every process, no cgroup figure; each ratio is the class's tick delta over the delta of all CPU time, idle included, user includes nice) sampled across the 256 MiB throughput window; irq and softirq are tick-sampled on this kernel (irq_time_accounting is off, so irq reads 0 and softirq is coarse); memory (VmRSS) and open sockets of the benchmark process are taken at the window's end; connection_concurrency is the peak number of client sessions the family held open (opened and not yet dropped) during that window; forward_userspace_cpu_ratio and return_splice_cpu_ratio are absent because the relay is asynchronous on shared runtime threads; handshake_ns starts when the enforcement accepts the connection, so it includes the wait for the client's ClientHello (client-side TLS work); the TLS client runs on the same host as the agent and may co-limit throughput and tail latency, so the numbers are not pure agent cost"
    ),
    metric(name = "connection_establishment_ns", unit = ns, table = "connection-establishment", series),
    metric(name = "handshake_ns", unit = ns, table = "handshake", series),
    metric(name = "first_byte_ns", unit = ns, table = "first-byte", series),
    metric(name = "throughput_bytes_per_second", unit = bytes_per_second, table = "throughput"),
    metric(name = "tail_latency_ns", unit = ns, table = "tail-latency", series),
    metric(name = "host_cpu_user_ratio", unit = ratio, table = "host-cpu"),
    metric(name = "host_cpu_system_ratio", unit = ratio, table = "host-cpu"),
    metric(name = "host_cpu_irq_ratio", unit = ratio, table = "host-cpu"),
    metric(name = "host_cpu_softirq_ratio", unit = ratio, table = "host-cpu"),
    metric(name = "memory_bytes", unit = bytes, table = "memory"),
    metric(name = "socket_count", unit = count, table = "sockets"),
    metric(name = "connection_concurrency", unit = count, table = "sockets"),
    metric(name = "forward_userspace_cpu_ratio", unit = ratio, table = "pump-cpu"),
    metric(name = "return_splice_cpu_ratio", unit = ratio, table = "pump-cpu"),
    smoke(guest = "cgroup-default"),
)]
fn mtls_hop(rep: &mut Replicate) -> Result<(), ReplicateFailure> {
    let composed = rep.arm() == COMPOSED;
    if !composed && rep.arm() != BASELINE {
        return Err(fail(&format!("arm `{}` is not declared", rep.arm())));
    }
    note_shape(rep);
    // Idempotent: a second install returns `Err`, which only says one is already set.
    let _ = rustls::crypto::ring::default_provider().install_default();
    match rep.profile() {
        "egress" => {
            let path = EgressPath::start(composed)?;
            drive_on_client_thread(rep, &path, composed)?;
            path.finish()
        }
        "ingress" => {
            let path = IngressPath::start(composed)?;
            if let Err(error) = drive_on_client_thread(rep, &path, composed) {
                return Err(path.explain(error));
            }
            path.finish()
        }
        other => Err(fail(&format!("profile `{other}` is not declared"))),
    }
}

fn note_shape(rep: &mut Replicate) {
    rep.note("arrival_model", "closed-loop");
    rep.note("connections", &CONNECTIONS.to_string());
    rep.note("round_trips", &ROUND_TRIPS.to_string());
    rep.note("bulk_bytes", &BULK_BYTES.to_string());
    rep.note("request_bytes", &REQUEST_LEN.to_string());
    rep.note("reply_bytes", &REPLY.len().to_string());
    rep.note("fixture_thread_prefix", THREAD_PREFIX);
}

fn fail(message: &str) -> ReplicateFailure {
    ReplicateFailure::fixture(message)
}

fn broke(message: &str) -> ReplicateFailure {
    ReplicateFailure::measurement(message)
}

/// A fixture failure naming `what` and the display of its cause.
fn because<E: Display>(what: &'static str) -> impl FnOnce(E) -> ReplicateFailure {
    move |error| fail(&format!("{what}: {error}"))
}

/// A fixture failure naming `what` and the debug form of its cause.
fn because_debug<E: Debug>(what: &'static str) -> impl FnOnce(E) -> ReplicateFailure {
    move |error| fail(&format!("{what}: {error:?}"))
}

/// A measurement failure naming `what` and the display of its cause.
fn measured<E: Display>(what: &'static str) -> impl FnOnce(E) -> ReplicateFailure {
    move |error| broke(&format!("{what}: {error}"))
}

const fn nanos(duration: Duration) -> f64 {
    duration.as_nanos() as f64
}

// ============================================================================
// The measurement, identical for every path
// ============================================================================

/// What differs between the paths: how a client connection is opened, and which handshake
/// durations the enforcement recorded.
trait Path {
    fn open(&self) -> Result<Session, ReplicateFailure>;
    /// The count of client sessions this path has open.
    fn sessions(&self) -> &Arc<SessionCount>;
    /// Waits until the enforcement has recorded `count` durations since the last take. The
    /// pumps start inside `enforce`, so a client can see its reply (and close, which cancels
    /// the call) before the decorator has recorded the duration; the client waits here
    /// before it closes. A path with no enforcement returns at once.
    fn await_handshakes(&self, count: usize) -> Result<(), ReplicateFailure>;
    /// The enforce durations recorded since the last call; empty when the path has no
    /// enforcement.
    fn take_handshakes(&self) -> Vec<f64>;
}

/// Runs the measurement on a spawned, joined thread carrying the family prefix, so the
/// client's own work is as separable from the agent's as the peer's.
fn drive_on_client_thread<P: Path + Sync>(
    rep: &mut Replicate,
    path: &P,
    composed: bool,
) -> Result<(), ReplicateFailure> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name(format!("{THREAD_PREFIX}client"))
            .spawn_scoped(scope, || drive(rep, path, composed))
            .map_err(because("spawning the client thread"))?
            .join()
            .map_err(|_| fail("the client thread panicked"))?
    })
}

fn drive<P: Path>(rep: &mut Replicate, path: &P, composed: bool) -> Result<(), ReplicateFailure> {
    let frame = small_frame();
    for done in 1..=WARMUP_CONNECTIONS as usize {
        let mut session = path.open()?;
        session.small_exchange(&frame).map_err(measured("warm-up exchange"))?;
        path.await_handshakes(done)?;
        session.finish().map_err(measured("closing a warm-up connection"))?;
    }
    drop(path.take_handshakes());

    let mut connect = Vec::with_capacity(CONNECTIONS as usize);
    let mut first_byte = Vec::with_capacity(CONNECTIONS as usize);
    for done in 1..=CONNECTIONS as usize {
        let mut session = path.open()?;
        session.small_exchange(&frame).map_err(measured("first exchange of a connection"))?;
        let received = rep.clock().now();
        first_byte.push(nanos(received.duration_since(session.began)));
        connect.push(nanos(session.connect));
        path.await_handshakes(done)?;
        session.finish().map_err(measured("closing a measured connection"))?;
    }
    let handshakes = path.take_handshakes();
    rep.record_series(ESTABLISHMENT, &connect)?;
    rep.record_series(FIRST_BYTE, &first_byte)?;
    if composed {
        rep.record_series(HANDSHAKE, &handshakes)?;
    } else {
        rep.record_absent(HANDSHAKE, HANDSHAKE_BASELINE_REASON)?;
    }

    let mut session = path.open()?;
    for _ in 0..WARMUP_ROUND_TRIPS {
        session.small_exchange(&frame).map_err(measured("warm-up round trip"))?;
    }
    let mut round_trips = Vec::with_capacity(ROUND_TRIPS as usize);
    for _ in 0..ROUND_TRIPS {
        let sent = rep.clock().now();
        session.small_exchange(&frame).map_err(measured("round trip"))?;
        round_trips.push(nanos(rep.clock().now().duration_since(sent)));
    }
    session.finish().map_err(measured("closing the round-trip connection"))?;
    rep.record_series(TAIL, &round_trips)?;

    let mut session = path.open()?;
    // The enforcement buffers at most `max_prearm_bytes` of plaintext until its TLS leg is
    // armed, so the bulk frame follows one small exchange that proves the leg is up.
    session.small_exchange(&frame).map_err(measured("establishing the bulk connection"))?;
    let before =
        Snapshot::take().map_err(measured("sampling resource use before the bulk window"))?;
    path.sessions().begin_window();
    let started = rep.clock().now();
    session.bulk(BULK_BYTES).map_err(measured("bulk transfer"))?;
    let elapsed = rep.clock().now().duration_since(started);
    let after =
        Snapshot::take().map_err(measured("sampling resource use after the bulk window"))?;
    let resident = resident_bytes().map_err(measured("sampling resident memory"))?;
    let sockets = open_sockets().map_err(measured("counting open sockets"))?;
    let concurrency = path.sessions().peak_in_window();
    session.finish().map_err(measured("closing the bulk connection"))?;
    rep.record(THROUGHPUT, BULK_BYTES as f64 / elapsed.as_secs_f64())?;
    record_cost(rep, composed, &before, &after)?;
    rep.record(MEMORY, resident as f64)?;
    rep.record(SOCKETS, sockets as f64)?;
    rep.record(CONCURRENCY, concurrency as f64)
}

/// The whole-host CPU split of the sustained window, and the forward and return pump split
/// where the profile and arm have pumps to split.
fn record_cost(
    rep: &mut Replicate,
    composed: bool,
    before: &Snapshot,
    after: &Snapshot,
) -> Result<(), ReplicateFailure> {
    let host = host_ratios(before, after).map_err(measured("whole-host CPU of the window"))?;
    rep.record(HOST_CPU_USER, host.user)?;
    rep.record(HOST_CPU_SYSTEM, host.system)?;
    rep.record(HOST_CPU_IRQ, host.irq)?;
    rep.record(HOST_CPU_SOFTIRQ, host.softirq)?;
    let reason = match (rep.profile() == "egress", composed) {
        (true, true) => {
            let split = pump_split(before, after, THREAD_PREFIX)
                .map_err(measured("forward and return CPU of the window"))?;
            rep.note(
                "pump_heavy_to_light_lower_bound",
                &split.heavy_to_light_lower_bound.to_string(),
            );
            rep.record(FORWARD_CPU, split.forward)?;
            return rep.record(RETURN_CPU, split.ret);
        }
        (true, false) => BASELINE_PUMP_REASON,
        (false, _) => INGRESS_PUMP_REASON,
    };
    rep.record_absent(FORWARD_CPU, reason)?;
    rep.record_absent(RETURN_CPU, reason)
}

// ============================================================================
// The frame protocol
// ============================================================================

/// One 64-byte request frame: the length header and the payload.
fn small_frame() -> [u8; HEADER_LEN + REQUEST_LEN] {
    let mut frame = [b'q'; HEADER_LEN + REQUEST_LEN];
    frame[..HEADER_LEN].copy_from_slice(&(REQUEST_LEN as u64).to_be_bytes());
    frame
}

/// The server half, blocking: reads frames, discards the payloads, answers each with the
/// reply, ends on any end of stream or error (the client side observes what went wrong).
fn serve_frames<S: Read + Write>(stream: &mut S) {
    let mut scratch = vec![0_u8; CHUNK];
    loop {
        let mut header = [0_u8; HEADER_LEN];
        if stream.read_exact(&mut header).is_err() {
            return;
        }
        let mut left = u64::from_be_bytes(header);
        while left > 0 {
            let want = usize::try_from(left).unwrap_or(CHUNK).min(CHUNK);
            match stream.read(&mut scratch[..want]) {
                Ok(0) | Err(_) => return,
                Ok(read) => left -= read as u64,
            }
        }
        if stream.write_all(&REPLY).and_then(|()| stream.flush()).is_err() {
            return;
        }
    }
}

/// The server half over an in-memory stream.
async fn serve_frames_async(mut stream: DuplexStream) {
    let mut scratch = vec![0_u8; CHUNK];
    loop {
        let mut header = [0_u8; HEADER_LEN];
        if stream.read_exact(&mut header).await.is_err() {
            return;
        }
        let mut left = u64::from_be_bytes(header);
        while left > 0 {
            let want = usize::try_from(left).unwrap_or(CHUNK).min(CHUNK);
            match stream.read(&mut scratch[..want]).await {
                Ok(0) | Err(_) => return,
                Ok(read) => left -= read as u64,
            }
        }
        if stream.write_all(&REPLY).await.is_err() || stream.flush().await.is_err() {
            return;
        }
    }
}

// ============================================================================
// Client side
// ============================================================================

/// A client connection: plaintext, or a rustls client toward the inbound path.
enum Link {
    Plain(TcpStream),
    Tls(Box<TlsLink>),
}

impl Read for Link {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(tcp) => tcp.read(buf),
            Self::Tls(tls) => tls.read(buf),
        }
    }
}

impl Write for Link {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(tcp) => tcp.write(buf),
            Self::Tls(tls) => tls.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(tcp) => tcp.flush(),
            Self::Tls(tls) => tls.flush(),
        }
    }
}

/// Completes the client handshake before any timing that depends on it starts.
fn tls_handshake(
    config: &Arc<rustls::ClientConfig>,
    mut tcp: TcpStream,
) -> std::io::Result<TlsLink> {
    let sni = ServerName::try_from(SERVER_SNI.to_owned()).map_err(std::io::Error::other)?;
    let mut conn =
        rustls::ClientConnection::new(Arc::clone(config), sni).map_err(std::io::Error::other)?;
    conn.complete_io(&mut tcp)?;
    // The client's final flight must be on the wire before the timed exchange starts.
    while conn.wants_write() {
        conn.write_tls(&mut tcp)?;
    }
    Ok(rustls::StreamOwned::new(conn, tcp))
}

type TlsLink = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

/// The client's clean end: `close_notify`, then FIN.
fn close_tls(tls: &mut TlsLink) -> std::io::Result<()> {
    tls.conn.send_close_notify();
    while tls.conn.wants_write() {
        tls.conn.write_tls(&mut tls.sock)?;
    }
    tls.sock.shutdown(Shutdown::Write)
}

/// The client sessions a path has open: the live count, and the most that were open at once
/// since the window began.
#[derive(Default)]
struct SessionCount {
    live: AtomicU64,
    peak: AtomicU64,
}

impl SessionCount {
    /// Counts one more open session until the returned guard drops.
    fn opened(self: &Arc<Self>) -> LiveSession {
        let now = self.live.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        LiveSession(Arc::clone(self))
    }

    /// Starts a window: the peak is what is open now.
    fn begin_window(&self) {
        self.peak.store(self.live.load(Ordering::SeqCst), Ordering::SeqCst);
    }

    fn peak_in_window(&self) -> u64 {
        self.peak.load(Ordering::SeqCst)
    }
}

/// Holds one session in the live count; dropping it takes the session out.
struct LiveSession(Arc<SessionCount>);

impl Drop for LiveSession {
    fn drop(&mut self) {
        self.0.live.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One open client connection and what opening it cost.
struct Session {
    link: Link,
    /// Just before the client socket's connect.
    began: HostInstant,
    /// The client socket's connect, both arms.
    connect: Duration,
    /// Dropped after the link, so the flow closes before its registration is released.
    _registration: Option<ForwardedOutbound>,
    /// Keeps the session in the live count for as long as it is open.
    _live: LiveSession,
}

impl Session {
    fn small_exchange(&mut self, frame: &[u8]) -> std::io::Result<()> {
        self.link.write_all(frame)?;
        self.link.flush()?;
        self.read_reply()
    }

    fn bulk(&mut self, total: u64) -> std::io::Result<()> {
        self.link.write_all(&total.to_be_bytes())?;
        let chunk = vec![0x5a_u8; CHUNK];
        let mut left = total;
        while left > 0 {
            let size = usize::try_from(left).unwrap_or(CHUNK).min(CHUNK);
            self.link.write_all(&chunk[..size])?;
            left -= size as u64;
        }
        self.link.flush()?;
        self.read_reply()
    }

    fn read_reply(&mut self) -> std::io::Result<()> {
        let mut reply = [0_u8; REPLY.len()];
        self.link.read_exact(&mut reply)?;
        if reply == REPLY {
            Ok(())
        } else {
            Err(std::io::Error::new(ErrorKind::InvalidData, "the reply bytes differ"))
        }
    }

    fn finish(mut self) -> std::io::Result<()> {
        match &mut self.link {
            Link::Plain(_) => Ok(()),
            Link::Tls(tls) => close_tls(tls),
        }
    }
}

/// A client socket bound to loopback and configured, not yet connected: everything the
/// client does besides the connect happens before the timed window opens.
fn dialler() -> Result<(Socket, SocketAddrV4), ReplicateFailure> {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None)
        .map_err(because("creating the dialler socket"))?;
    socket.set_reuse_address(true).map_err(because("SO_REUSEADDR on the dialler socket"))?;
    socket.set_nodelay(true).map_err(because("setting TCP_NODELAY"))?;
    socket.set_read_timeout(Some(IO_TIMEOUT)).map_err(because("setting the read timeout"))?;
    socket.set_write_timeout(Some(IO_TIMEOUT)).map_err(because("setting the write timeout"))?;
    socket
        .bind(&SockAddr::from(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)))
        .map_err(because("binding the dialler socket"))?;
    let local = socket
        .local_addr()
        .map_err(because("the dialler address"))?
        .as_socket_ipv4()
        .ok_or_else(|| fail("an IPv4 bind reported a different family"))?;
    Ok((socket, local))
}

/// A connected client socket and the duration of its connect.
struct Dialled {
    tcp: TcpStream,
    /// Just before the connect.
    began: HostInstant,
    connect: Duration,
}

/// Connects `socket` to `target`; nothing but the connect sits between the two stamps.
fn dial(socket: Socket, target: SocketAddrV4) -> Result<Dialled, ReplicateFailure> {
    let began = HostClock.now();
    socket.connect(&SockAddr::from(target)).map_err(because("the dialler's connect"))?;
    let connected = HostClock.now();
    Ok(Dialled { tcp: TcpStream::from(socket), began, connect: connected.duration_since(began) })
}

// ============================================================================
// Test PKI and identity
// ============================================================================

struct Authority {
    params: CertificateParams,
    key: KeyPair,
    cert_pem: String,
    cert_der: Vec<u8>,
}

struct Leaf {
    cert_pem: String,
    key_pem: String,
    cert_der: CertificateDer<'static>,
    key_der: PrivateKeyDer<'static>,
    spiffe: &'static str,
}

impl Authority {
    fn root(name: &str) -> Result<Self, ReplicateFailure> {
        let mut params = CertificateParams::new(Vec::<String>::new())
            .map_err(because("root certificate parameters"))?;
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name.push(rcgen::DnType::CommonName, name);
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .map_err(because("generating the root key"))?;
        let cert = params.self_signed(&key).map_err(because("self-signing the root"))?;
        Ok(Self { cert_pem: cert.pem(), cert_der: cert.der().to_vec(), params, key })
    }

    fn intermediate(&self, name: &str) -> Result<Self, ReplicateFailure> {
        let mut params = CertificateParams::new(Vec::<String>::new())
            .map_err(because("intermediate certificate parameters"))?;
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
        params.distinguished_name.push(rcgen::DnType::CommonName, name);
        params.use_authority_key_identifier_extension = true;
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .map_err(because("generating the intermediate key"))?;
        let issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&key, &issuer).map_err(because("signing the intermediate"))?;
        Ok(Self { cert_pem: cert.pem(), cert_der: cert.der().to_vec(), params, key })
    }

    fn leaf(
        &self,
        spiffe: &'static str,
        dns_san: Option<&str>,
        client_auth: bool,
    ) -> Result<Leaf, ReplicateFailure> {
        let mut params = CertificateParams::new(Vec::<String>::new())
            .map_err(because("leaf certificate parameters"))?;
        let uri = Ia5String::try_from(spiffe).map_err(because("the SPIFFE URI"))?;
        let mut sans = vec![SanType::URI(uri)];
        if let Some(dns) = dns_san {
            sans.push(SanType::DnsName(Ia5String::try_from(dns).map_err(because("the DNS SAN"))?));
        }
        params.subject_alt_names = sans;
        params.distinguished_name.push(rcgen::DnType::CommonName, spiffe);
        params.use_authority_key_identifier_extension = true;
        params.extended_key_usages = if client_auth {
            vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth]
        } else {
            vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth]
        };
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .map_err(because("generating a leaf key"))?;
        let issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&key, &issuer).map_err(because("signing a leaf"))?;
        Ok(Leaf {
            cert_pem: cert.pem(),
            key_pem: key.serialize_pem(),
            cert_der: CertificateDer::from(cert.der().to_vec()),
            key_der: PrivateKeyDer::from(PrivatePkcs8KeyDer::from(key.serialize_der())),
            spiffe,
        })
    }
}

/// Root, intermediate and the three leaves the paths use.
struct Pki {
    root_pem: String,
    root_der: CertificateDer<'static>,
    intermediate_pem: String,
    intermediate_der: CertificateDer<'static>,
    client: Leaf,
    server: Leaf,
    peer: Leaf,
}

impl Pki {
    fn mint() -> Result<Self, ReplicateFailure> {
        let root = Authority::root("overdrive-bench-mtls-ROOT-CA")?;
        let intermediate = root.intermediate("overdrive-bench-mtls-INTERMEDIATE-CA")?;
        Ok(Self {
            client: intermediate.leaf(CLIENT_SPIFFE, None, true)?,
            server: intermediate.leaf(SERVER_SPIFFE, Some(SERVER_SNI), false)?,
            peer: intermediate.leaf(PEER_SPIFFE, Some(PEER_SNI), false)?,
            root_der: CertificateDer::from(root.cert_der),
            root_pem: root.cert_pem,
            intermediate_der: CertificateDer::from(intermediate.cert_der),
            intermediate_pem: intermediate.cert_pem,
        })
    }

    fn bundle(&self) -> TrustBundle {
        TrustBundle::new(
            CaCertPem::new(self.root_pem.clone()),
            Some(CaCertPem::new(self.intermediate_pem.clone())),
        )
    }

    fn roots(&self) -> Result<rustls::RootCertStore, ReplicateFailure> {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(self.root_der.clone()).map_err(because("trusting the test root"))?;
        Ok(roots)
    }

    /// The mesh peer's server configuration: its own chain, a required client certificate,
    /// and no TLS 1.3 session tickets (a ticket record breaks the agent's kTLS receive).
    fn peer_config(&self) -> Result<Arc<rustls::ServerConfig>, ReplicateFailure> {
        let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(self.roots()?))
            .build()
            .map_err(because("building the client verifier"))?;
        let mut config = rustls::ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                vec![self.peer.cert_der.clone(), self.intermediate_der.clone()],
                self.peer.key_der.clone_key(),
            )
            .map_err(because("building the peer server configuration"))?;
        config.send_tls13_tickets = 0;
        Ok(Arc::new(config))
    }

    /// The ingress client's configuration: the node-trusted client certificate.
    fn client_config(&self) -> Result<Arc<rustls::ClientConfig>, ReplicateFailure> {
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(self.roots()?)
            .with_client_auth_cert(
                vec![self.client.cert_der.clone(), self.intermediate_der.clone()],
                self.client.key_der.clone_key(),
            )
            .map_err(because("building the client configuration"))?;
        Ok(Arc::new(config))
    }

    fn svid(leaf: &Leaf) -> Result<SvidMaterial, ReplicateFailure> {
        let serial = CertSerial::new("0a0b0c0d").map_err(because("the certificate serial"))?;
        let spiffe: SpiffeId = leaf.spiffe.parse().map_err(because("the SPIFFE id"))?;
        let not_after = UnixInstant::from_unix_duration(Duration::from_secs(4_102_444_800));
        Ok(SvidMaterial::new(
            CaCertPem::new(leaf.cert_pem.clone()),
            CaCertDer::new(leaf.cert_der.as_ref().to_vec()),
            serial,
            spiffe,
            CaKeyPem::new(leaf.key_pem.clone()),
            not_after,
        ))
    }

    /// The agent's held identity: `leaf` for `alloc`, this PKI's bundle.
    fn held(&self, alloc: &AllocationId, leaf: &Leaf) -> Result<Arc<Held>, ReplicateFailure> {
        Ok(Arc::new(Held { alloc: alloc.clone(), svid: Self::svid(leaf)?, bundle: self.bundle() }))
    }
}

struct Held {
    alloc: AllocationId,
    svid: SvidMaterial,
    bundle: TrustBundle,
}

impl IdentityRead for Held {
    fn svid_for(&self, alloc: &AllocationId) -> Option<SvidMaterial> {
        (alloc == &self.alloc).then(|| self.svid.clone())
    }

    fn current_bundle(&self) -> Option<TrustBundle> {
        Some(self.bundle.clone())
    }
}

// ============================================================================
// The timing decorator
// ============================================================================

/// The real enforcement with the duration of every `enforce` recorded. Everything else is
/// passed through untouched.
struct TimedEnforcement {
    inner: HostMtlsEnforcement,
    handshakes: Mutex<Vec<f64>>,
}

impl TimedEnforcement {
    const fn new(inner: HostMtlsEnforcement) -> Self {
        Self { inner, handshakes: Mutex::new(Vec::new()) }
    }

    /// Waits, bounded, until `count` durations are recorded.
    fn await_count(&self, count: usize) -> Result<(), ReplicateFailure> {
        let until = Instant::now() + SETTLE_BOUND;
        while self.handshakes.lock().len() < count {
            if Instant::now() >= until {
                return Err(broke(&format!(
                    "`enforce` had not returned for {count} connections {} s after their replies",
                    SETTLE_BOUND.as_secs()
                )));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    fn take(&self) -> Vec<f64> {
        std::mem::take(&mut *self.handshakes.lock())
    }
}

#[async_trait]
impl MtlsEnforcement for TimedEnforcement {
    async fn probe(&self) -> overdrive_core::traits::mtls_enforcement::Result<()> {
        self.inner.probe().await
    }

    async fn enforce(
        &self,
        conn: InterceptedConnection,
    ) -> overdrive_core::traits::mtls_enforcement::Result<EnforcedConnection> {
        let began = HostClock.now();
        let enforced = self.inner.enforce(conn).await;
        let took = HostClock.now().duration_since(began);
        if enforced.is_ok() {
            self.handshakes.lock().push(nanos(took));
        }
        enforced
    }

    fn liveness(&self, handle: &EnforcedConnection) -> PumpLiveness {
        self.inner.liveness(handle)
    }

    async fn teardown(
        &self,
        handle: EnforcedConnection,
    ) -> overdrive_core::traits::mtls_enforcement::Result<()> {
        self.inner.teardown(handle).await
    }
}

// ============================================================================
// Accept loop and mesh peer
// ============================================================================

/// A blocking accept loop on its own thread; `stop` ends it with a connection to itself.
struct AcceptLoop {
    addr: SocketAddrV4,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl AcceptLoop {
    fn start(
        name: &str,
        mut on_accept: impl FnMut(TcpStream) + Send + 'static,
    ) -> Result<Self, ReplicateFailure> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .map_err(because("binding a loopback listener"))?;
        let addr = match listener.local_addr().map_err(because("the listener address"))? {
            SocketAddr::V4(addr) => addr,
            SocketAddr::V6(addr) => return Err(fail(&format!("an IPv4 bind reported {addr}"))),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                loop {
                    match listener.accept() {
                        Ok(_) if stopping.load(Ordering::SeqCst) => return,
                        Ok((tcp, _)) => on_accept(tcp),
                        Err(error) => {
                            tracing::warn!(%error, "the accept loop ended on an accept error");
                            return;
                        }
                    }
                }
            })
            .map_err(because("spawning an accept thread"))?;
        Ok(Self { addr, stop, thread: Some(thread) })
    }

    fn stop(&mut self) -> Result<(), ReplicateFailure> {
        let Some(thread) = self.thread.take() else { return Ok(()) };
        self.stop.store(true, Ordering::SeqCst);
        // Wakes the blocking accept; if the loop already ended the connect is refused.
        drop(TcpStream::connect(self.addr));
        thread.join().map_err(|_| fail("an accept thread panicked"))
    }
}

impl Drop for AcceptLoop {
    fn drop(&mut self) {
        // Reported by `stop` on the success path; here the replicate is already failing.
        drop(self.stop());
    }
}

/// The server a plain or TLS client connection ends at: sessions on their own threads.
struct Peer {
    addr: SocketAddrV4,
    accept: AcceptLoop,
    sessions: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl Peer {
    /// A rustls mesh peer when `tls` is given, a plaintext TCP peer otherwise.
    fn start(tls: Option<Arc<rustls::ServerConfig>>) -> Result<Self, ReplicateFailure> {
        let sessions: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::default();
        let held = Arc::clone(&sessions);
        let accept = AcceptLoop::start("ovbm-peer-accept", move |tcp| {
            let tls = tls.clone();
            let session = std::thread::Builder::new()
                .name("ovbm-peer-session".to_owned())
                .spawn(move || serve_peer_session(tcp, tls.as_ref()));
            match session {
                Ok(handle) => held.lock().push(handle),
                Err(error) => tracing::warn!(%error, "spawning a peer session thread failed"),
            }
        })?;
        Ok(Self { addr: accept.addr, accept, sessions })
    }

    fn stop(mut self) -> Result<(), ReplicateFailure> {
        self.accept.stop()?;
        let sessions = std::mem::take(&mut *self.sessions.lock());
        for session in sessions {
            session.join().map_err(|_| fail("a peer session thread panicked"))?;
        }
        Ok(())
    }
}

/// One peer session. A session that cannot be served just ends; its client sees the
/// failure and fails the replicate.
fn serve_peer_session(mut tcp: TcpStream, tls: Option<&Arc<rustls::ServerConfig>>) {
    if tcp.set_nodelay(true).is_err() || tcp.set_read_timeout(Some(IO_TIMEOUT)).is_err() {
        return;
    }
    let Some(config) = tls else {
        serve_frames(&mut tcp);
        return;
    };
    let Ok(mut conn) = rustls::ServerConnection::new(Arc::clone(config)) else { return };
    while conn.is_handshaking() {
        if conn.complete_io(&mut tcp).is_err() {
            return;
        }
    }
    serve_frames(&mut rustls::Stream::new(&mut conn, &mut tcp));
}

// ============================================================================
// Egress
// ============================================================================

/// Cross-process exclusion for the node-global `overdrive-mtls` kernel state, on the same
/// fixed path the worker's Tier-3 suites hold.
struct KernelStateLock {
    _fd: OwnedFd,
}

impl KernelStateLock {
    fn acquire() -> Result<Self, ReplicateFailure> {
        let path = c"/tmp/overdrive-mtls-kernel-state.lock";
        // SAFETY: `open` on a fixed NUL-terminated path; the descriptor is adopted below.
        let raw = unsafe { libc::open(path.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600) };
        if raw < 0 {
            return Err(fail(&format!(
                "opening the kernel-state lock file: {}",
                std::io::Error::last_os_error()
            )));
        }
        // SAFETY: `raw` is a freshly opened descriptor this function owns.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: `fd` is a live descriptor; `flock` blocks until the lock is held.
        if unsafe { libc::flock(fd.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(fail(&format!(
                "locking the kernel-state lock file: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(Self { _fd: fd })
    }
}

/// Scrubs the node-global `overdrive-mtls` state on every exit path. Declared after the
/// lock, so it drops while the lock is still held.
struct SharedInfraScrub;

impl SharedInfraScrub {
    fn run() {
        // Drains however many fwmark rules a prior run stacked.
        for _ in 0..64 {
            if !quiet("ip", &["rule", "del", "fwmark", "0x1", "lookup", "100"]) {
                break;
            }
        }
        // Best effort: each of these is absent on a clean node.
        quiet("ip", &["route", "del", "local", "0.0.0.0/0", "dev", "lo", "table", "100"]);
        quiet("nft", &["delete", "table", "ip", "overdrive-mtls"]);
        quiet("nft", &["delete", "table", "ip", "overdrive-mtls-guard"]);
    }
}

impl Drop for SharedInfraScrub {
    fn drop(&mut self) {
        Self::run();
    }
}

/// Runs a cleanup command; whether it succeeded.
fn quiet(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Answers `MeshUnreachable` for every address: a registered flow carries its own
/// resolution, so leg F never asks.
struct NoResolve;

#[async_trait]
impl MtlsResolve for NoResolve {
    async fn probe(&self) -> Result<(), MtlsResolveError> {
        Ok(())
    }

    async fn resolve(&self, _orig_dst: SocketAddrV4) -> Result<MtlsResolution, MtlsResolveError> {
        Ok(MtlsResolution::MeshUnreachable)
    }
}

/// The composed node of the egress arm. Field order is drop order: the worker first, the
/// runtime it runs on, the scrub next, the lease, the kernel lock last.
struct EgressNode {
    worker: Arc<MtlsInterceptWorker>,
    timed: Arc<TimedEnforcement>,
    alloc: AllocationId,
    original_destination: SocketAddrV4,
    leg_f: SocketAddrV4,
    runtime: Runtime,
    _scrub: SharedInfraScrub,
    _lease: TestCidrLease,
    _lock: KernelStateLock,
}

/// The node is present on the composed arm only; the baseline dials the peer directly.
struct EgressPath {
    node: Option<EgressNode>,
    peer: Peer,
    sessions: Arc<SessionCount>,
}

impl EgressPath {
    fn start(composed: bool) -> Result<Self, ReplicateFailure> {
        if !composed {
            return Ok(Self { node: None, peer: Peer::start(None)?, sessions: Arc::default() });
        }
        let lock = KernelStateLock::acquire()?;
        SharedInfraScrub::run();
        let scrub = SharedInfraScrub;
        let runtime = runtime()?;
        let lease = TestCidrLease::acquire(LEASE_OWNER).map_err(because_debug("the CIDR lease"))?;
        let pki = Pki::mint()?;
        let peer = Peer::start(Some(pki.peer_config()?))?;
        let alloc = AllocationId::new(EGRESS_ALLOC).map_err(because("the allocation id"))?;
        let timed = Arc::new(TimedEnforcement::new(HostMtlsEnforcement::new(
            pki.held(&alloc, &pki.client)?,
            MtlsLimits::default(),
            GUEST_NODE_PREFIX,
            Arc::new(PipeDial),
        )));
        runtime.block_on(timed.probe()).map_err(because("the enforcement probe"))?;
        let worker = Arc::new(MtlsInterceptWorker::new(
            Arc::clone(&timed) as Arc<dyn MtlsEnforcement>,
            Arc::new(NoResolve),
            Arc::new(SystemClock),
            Arc::new(HostMtlsIntercept::new()),
        ));
        let spec = intercept_spec(&alloc, &lease)?;
        runtime
            .block_on(async {
                worker.start_shared_owner().await.map_err(|error| format!("{error:?}"))?;
                worker.start_alloc(&spec).await.map_err(|error| format!("{error:?}"))
            })
            .map_err(because("making the allocation intercept-live"))?;
        let leg_f = worker.leg_f_addr().ok_or_else(|| fail("leg F is not published"))?;
        let original_destination = SocketAddrV4::new(lease.network(), DIALLED_SERVICE_PORT);
        let node = EgressNode {
            worker,
            timed,
            alloc,
            original_destination,
            leg_f,
            runtime,
            _scrub: scrub,
            _lease: lease,
            _lock: lock,
        };
        Ok(Self { node: Some(node), peer, sessions: Arc::default() })
    }

    fn finish(self) -> Result<(), ReplicateFailure> {
        let Self { node, peer, .. } = self;
        if let Some(node) = node {
            node.runtime
                .block_on(node.worker.shutdown_owner())
                .map_err(because_debug("shutting the node-shared owner down"))?;
        }
        peer.stop()
    }
}

/// The multi-thread runtime the composed egress node and the ingress relays run on.
fn runtime() -> Result<Runtime, ReplicateFailure> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .map_err(because("building the runtime"))
}

/// The allocation the intercept is live for, addressed from the lease.
fn intercept_spec(
    alloc: &AllocationId,
    lease: &TestCidrLease,
) -> Result<AllocationSpec, ReplicateFailure> {
    let cid = GuestCid::new(GUEST_CID).map_err(because_debug("the guest CID"))?;
    let launch = LaunchToken::from_bytes([0x5a; 16]).map_err(because_debug("the launch token"))?;
    let ports = GuestFlowPorts::new(GUEST_FLOW_STREAM_PORT, GUEST_FLOW_CONTROL_PORT)
        .map_err(because_debug("the guest flow ports"))?;
    let identity: SpiffeId = CLIENT_SPIFFE.parse().map_err(because("the allocation identity"))?;
    Ok(AllocationSpec {
        declared_listeners: Vec::new(),
        alloc: alloc.clone(),
        identity,
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/true".to_owned(),
            args: vec![],
            kernel: "/nonexistent/kernel".into(),
            rootfs: "/nonexistent/rootfs".into(),
            volume: None,
        }),
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        transport: Some(GuestTransportAssignment::new(
            lease.workload_addr(),
            lease.host_gateway(),
            cid,
            launch,
            ports,
        )),
    })
}

impl Path for EgressPath {
    fn open(&self) -> Result<Session, ReplicateFailure> {
        let (socket, local) = dialler()?;
        // The owner's role: register the flow before the dialler connects to leg F.
        let (target, registration) = match &self.node {
            Some(node) => {
                let backend = ResolvedBackend { addr: self.peer.addr, expected_svid: None };
                let registration = node
                    .worker
                    .register_forwarded_outbound(
                        &node.alloc,
                        local,
                        node.original_destination,
                        backend,
                    )
                    .map_err(because_debug("registering the forwarded flow"))?;
                (node.leg_f, Some(registration))
            }
            None => (self.peer.addr, None),
        };
        let Dialled { tcp, began, connect } = dial(socket, target)?;
        Ok(Session {
            link: Link::Plain(tcp),
            began,
            connect,
            _registration: registration,
            _live: self.sessions.opened(),
        })
    }

    fn sessions(&self) -> &Arc<SessionCount> {
        &self.sessions
    }

    fn await_handshakes(&self, count: usize) -> Result<(), ReplicateFailure> {
        self.node.as_ref().map_or(Ok(()), |node| node.timed.await_count(count))
    }

    fn take_handshakes(&self) -> Vec<f64> {
        self.node.as_ref().map(|node| node.timed.take()).unwrap_or_default()
    }
}

// ============================================================================
// Ingress
// ============================================================================

/// The guest end of an inbound flow: an in-memory stream to a frame server.
#[derive(Debug)]
struct PipeIo {
    pipe: DuplexStream,
    end: Option<GuestStreamEnd>,
    shut: bool,
}

#[async_trait]
impl GuestStreamIo for PipeIo {
    async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.pipe.read(buf).await?;
        if read == 0 {
            self.end = Some(GuestStreamEnd::HalfClose);
        }
        Ok(read)
    }

    async fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.shut {
            return Err(ErrorKind::BrokenPipe.into());
        }
        self.pipe.write(buf).await
    }

    async fn shutdown_write(&mut self) -> std::io::Result<()> {
        if !self.shut {
            self.pipe.shutdown().await?;
            self.shut = true;
        }
        Ok(())
    }

    fn end(&self) -> Option<GuestStreamEnd> {
        self.end
    }
}

/// Every dial pairs at once with a fresh frame server on the guest end.
struct PipeDial;

#[async_trait]
impl GuestStreamDial for PipeDial {
    async fn dial(
        &self,
        target: SocketAddrV4,
        _purpose: DialPurpose,
    ) -> Result<GuestStream, GuestDialError> {
        let (host, guest) = tokio::io::duplex(DUPLEX_BYTES);
        tokio::spawn(serve_frames_async(guest));
        Ok(GuestStream::new(target, Box::new(PipeIo { pipe: host, end: None, shut: false })))
    }
}

type Failures = Arc<Mutex<Vec<String>>>;

/// The inbound relay tasks, awaited by `finish`.
type Relays = Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>;

struct IngressPath {
    addr: SocketAddrV4,
    client_tls: Option<Arc<rustls::ClientConfig>>,
    timed: Option<Arc<TimedEnforcement>>,
    failures: Failures,
    relays: Relays,
    accept: AcceptLoop,
    sessions: Arc<SessionCount>,
    /// Kept alive for the relays; dropped last.
    runtime: Runtime,
}

impl IngressPath {
    fn start(composed: bool) -> Result<Self, ReplicateFailure> {
        let runtime = runtime()?;
        let failures: Failures = Arc::default();
        let relays: Relays = Arc::default();
        let handle = runtime.handle().clone();
        let (timed, client_tls, accept) = if composed {
            let pki = Pki::mint()?;
            let alloc = AllocationId::new(INGRESS_ALLOC).map_err(because("the allocation id"))?;
            let timed = Arc::new(TimedEnforcement::new(HostMtlsEnforcement::new(
                pki.held(&alloc, &pki.server)?,
                MtlsLimits::default(),
                GUEST_NODE_PREFIX,
                Arc::new(PipeDial),
            )));
            runtime.block_on(timed.probe()).map_err(because("the enforcement probe"))?;
            let (enforcement, failures, relays) =
                (Arc::clone(&timed), Arc::clone(&failures), Arc::clone(&relays));
            let accept = AcceptLoop::start("ovbm-ingress-accept", move |tcp| {
                let (enforcement, failures, alloc) =
                    (Arc::clone(&enforcement), Arc::clone(&failures), alloc.clone());
                let relay = handle.spawn(async move {
                    relay_composed(&enforcement, tcp, alloc, &failures).await;
                });
                relays.lock().push(relay);
            })?;
            (Some(timed), Some(pki.client_config()?), accept)
        } else {
            let (failures, relays) = (Arc::clone(&failures), Arc::clone(&relays));
            let accept = AcceptLoop::start("ovbm-ingress-accept", move |tcp| {
                let failures = Arc::clone(&failures);
                let relay = handle.spawn(async move {
                    relay_baseline(tcp, &failures).await;
                });
                relays.lock().push(relay);
            })?;
            (None, None, accept)
        };
        Ok(Self {
            addr: accept.addr,
            client_tls,
            timed,
            failures,
            relays,
            accept,
            sessions: Arc::default(),
            runtime,
        })
    }

    /// The measurement failure `error` together with what the inbound relays have reported
    /// so far: the server side usually names the cause of a client's reset.
    fn explain(&self, error: ReplicateFailure) -> ReplicateFailure {
        let failures = self.failures.lock().join("; ");
        if failures.is_empty() {
            return error;
        }
        broke(&format!("{error} (inbound relay failures: [{failures}])"))
    }

    /// Stops accepting, waits for the relays to settle and fails the replicate if any of
    /// them failed.
    fn finish(mut self) -> Result<(), ReplicateFailure> {
        self.accept.stop()?;
        let relays = std::mem::take(&mut *self.relays.lock());
        let settled = async {
            for relay in relays {
                relay.await.map_err(measured("an inbound relay task"))?;
            }
            Ok::<(), ReplicateFailure>(())
        };
        self.runtime
            .block_on(async { tokio::time::timeout(SETTLE_BOUND, settled).await })
            .map_err(|_| broke("an inbound relay did not end after its client closed"))??;
        let failures = std::mem::take(&mut *self.failures.lock());
        if failures.is_empty() {
            Ok(())
        } else {
            Err(broke(&format!("inbound relay failures: {}", failures.join("; "))))
        }
    }
}

fn report(failures: &Failures, message: String) {
    failures.lock().push(message);
}

async fn relay_composed(
    enforcement: &TimedEnforcement,
    tcp: TcpStream,
    alloc: AllocationId,
    failures: &Failures,
) {
    let conn = InterceptedConnection {
        leg: OwnedFd::from(tcp),
        routed: Routed::Inbound { orig_dst: GUEST_BACKEND },
        alloc,
        expected_peer: None,
    };
    match enforcement.enforce(conn).await {
        Ok(handle) => {
            // The relay ends by itself once both sides have closed; teardown before that
            // would reset the connection.
            while enforcement.liveness(&handle) != PumpLiveness::Gone {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            if let Err(error) = enforcement.teardown(handle).await {
                report(failures, format!("teardown: {error}"));
            }
        }
        Err(error) => report(failures, format!("enforce: {error}")),
    }
}

/// The baseline's inbound side: the accepted socket bridged to the same in-memory frame
/// server, with nothing between.
async fn relay_baseline(tcp: TcpStream, failures: &Failures) {
    if let Err(error) = tcp.set_nonblocking(true) {
        report(failures, format!("making an accepted socket non-blocking: {error}"));
        return;
    }
    let mut tcp = match tokio::net::TcpStream::from_std(tcp) {
        Ok(tcp) => tcp,
        Err(error) => {
            report(failures, format!("adopting an accepted socket: {error}"));
            return;
        }
    };
    let (mut host, guest) = tokio::io::duplex(DUPLEX_BYTES);
    tokio::spawn(serve_frames_async(guest));
    if let Err(error) = tokio::io::copy_bidirectional(&mut tcp, &mut host).await {
        report(failures, format!("bridging an accepted socket: {error}"));
    }
}

impl Path for IngressPath {
    fn open(&self) -> Result<Session, ReplicateFailure> {
        let (socket, _) = dialler()?;
        let Dialled { tcp, began, connect } = dial(socket, self.addr)?;
        let link = match &self.client_tls {
            Some(config) => Link::Tls(Box::new(
                tls_handshake(config, tcp).map_err(because("the client's TLS handshake"))?,
            )),
            None => Link::Plain(tcp),
        };
        Ok(Session { link, began, connect, _registration: None, _live: self.sessions.opened() })
    }

    fn sessions(&self) -> &Arc<SessionCount> {
        &self.sessions
    }

    fn await_handshakes(&self, count: usize) -> Result<(), ReplicateFailure> {
        self.timed.as_ref().map_or(Ok(()), |timed| timed.await_count(count))
    }

    fn take_handshakes(&self) -> Vec<f64> {
        self.timed.as_ref().map(|timed| timed.take()).unwrap_or_default()
    }
}

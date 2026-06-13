//! `HostMtlsEnforcement` — the production host adapter for the per-connection
//! transparent-mTLS enforcement port (ADR-0069, GH #26; OQ-2 home =
//! `overdrive-dataplane`).
//!
//! Implements [`MtlsEnforcement`](overdrive_core::traits::MtlsEnforcement) over
//! the spike-proven kernel primitives: rustls TLS 1.3 client/server handshakes
//! (consuming [`IdentityRead`](overdrive_core::traits::IdentityRead) for the held
//! SVID + trust bundle), kTLS arm (`setsockopt TCP_ULP/TLS_TX/TLS_RX`), and the
//! agent-light `splice(2)` pump in EVERY direction — forward (`legF → legB` into
//! leg B's kTLS-TX), return (`legB → legF`), deliver (`legC → legS`), response
//! (`legS → legC`). The forward path was a sockmap egress redirect; it is now a
//! splice pump symmetric to the others (the redirect `MSG_DONTWAIT`-stalled
//! ~10–15% of records — see
//! `docs/research/dataplane/sockmap-egress-redirect-into-ktls-tx-delivery-research.md`).
//!
//! Mechanism provenance (each method drives a spike-PROVEN syscall sequence):
//! - OUTBOUND lossless capture — `findings-userspace-relay.md` Unknown 1+2.
//! - splice into kTLS-TX (agent-light, forward + response) — `findings-splice-return.md`.
//! - splice out of kTLS-RX (agent-light, return + deliver) — `findings-splice-return.md`.
//! - INBOUND server-mTLS → kTLS-RX → splice-to-S — `findings-inbound-intercept.md`.
//!
//! `HostMtlsEnforcement::new` takes `Arc<dyn IdentityRead>` and [`MtlsLimits`] as
//! REQUIRED constructor parameters (`.claude/rules/development.md` § "Port-trait
//! dependencies"). #26 is a READER of `IdentityRead`, never an issuer (D-MTLS-9);
//! kTLS arms on the AGENT's leg, never the workload's socket; `expected_peer` is
//! `None` in v1 (authn-only).
//!
//! This module is raw `libc` syscall glue (`setsockopt` / `getsockname` /
//! `connect`), so the `size_of::<sockaddr_in>() as socklen_t` and
//! `Duration → time_t/suseconds_t` casts are FFI-width conversions on
//! compile-time-constant or already-bounded values — they cannot truncate. The
//! lints below are allowed module-wide with that rationale rather than peppering
//! every syscall site with a local `#[allow]`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "raw libc syscall glue: struct-size → socklen_t (compile-time constant, ≤ 56 bytes) and Duration → time_t/suseconds_t casts are FFI-width conversions on bounded values; cannot truncate or wrap"
)]

use std::io::Read;
use std::net::{SocketAddrV4, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::AllocationId;
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::SvidMaterial;
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnection, EnforcedConnectionId, InterceptedConnection, MtlsEnforcement,
    MtlsEnforcementError, MtlsLimits, ProbeSentinel, PumpLiveness, Result, Routed,
};
use overdrive_core::wall_clock::UnixInstant;
use parking_lot::Mutex;

mod inbound;
mod ktls;
mod outbound;
mod splice;
mod tls_config;

use splice::PumpHandle;

/// Per-connection adapter-private tracking state, keyed by
/// [`EnforcedConnectionId`]. Holds the owned legs (closed on teardown) + the
/// splice-pump handle. NOT exposed on [`EnforcedConnection`].
struct ConnState {
    /// The agent-owned legs to close on teardown. OUTBOUND: leg F + leg B.
    /// INBOUND: leg C + leg S.
    legs: Vec<OwnedFd>,
    /// The primary splice pump — the request-carrying direction `liveness`
    /// observes. OUTBOUND: the forward pump `splice(legF → legB)` (into leg B's
    /// kTLS-TX). INBOUND: the deliver pump `splice(legC → legS)`.
    pump: PumpHandle,
    /// Auxiliary splice pumps torn down with the connection but NOT observed by
    /// `liveness`. OUTBOUND carries the return pump `splice(legB → legF)` (the B→F
    /// response leg; leg B's kTLS-RX decrypts the peer's reply). INBOUND carries the
    /// response pump `splice(legS → legC)` (the S→C response leg — GAP 2 inbound
    /// half; leg C's kTLS-TX encrypts S's reply back to the client).
    aux_pumps: Vec<PumpHandle>,
}

/// The production host adapter for [`MtlsEnforcement`].
pub struct HostMtlsEnforcement {
    identity: Arc<dyn IdentityRead>,
    limits: MtlsLimits,
    next_counter: AtomicU64,
    /// Per-connection tracking, keyed by id. `liveness`/`teardown` look here.
    conns: Mutex<std::collections::BTreeMap<EnforcedConnectionId, ConnState>>,
}

impl HostMtlsEnforcement {
    /// Construct the adapter from its REQUIRED dependencies. `identity` is the
    /// shipped held-identity read port (#35) the proxy reads the SVID + trust
    /// bundle through (never an issuer); `limits` is the F7 resource contract.
    /// Both mandatory — no defaulting, no builder.
    #[must_use]
    pub fn new(identity: Arc<dyn IdentityRead>, limits: MtlsLimits) -> Self {
        Self {
            identity,
            limits,
            next_counter: AtomicU64::new(0),
            conns: Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    /// The construction-time resource bounds (F4/F7). Read-only; pinned at
    /// construction, not operator-tunable in v1.
    #[must_use]
    pub const fn limits(&self) -> &MtlsLimits {
        &self.limits
    }

    /// Mint the next stable correlation id for `alloc` (node-session-monotonic).
    fn next_id(&self, alloc: AllocationId) -> EnforcedConnectionId {
        let counter = self.next_counter.fetch_add(1, Ordering::Relaxed);
        EnforcedConnectionId::new(alloc, counter)
    }

    /// Read the held SVID for `alloc` (fail-closed: `None` ⇒ `AbsentSvid`).
    fn svid_or_fail(&self, alloc: &AllocationId) -> Result<SvidMaterial> {
        self.identity
            .svid_for(alloc)
            .ok_or_else(|| MtlsEnforcementError::AbsentSvid { alloc: alloc.clone() })
    }

    /// Register a steady-state-established connection in the tracking table and
    /// return its opaque handle.
    fn register(&self, id: EnforcedConnectionId, state: ConnState) -> EnforcedConnection {
        self.conns.lock().insert(id.clone(), state);
        EnforcedConnection::new(id)
    }
}

#[async_trait]
impl MtlsEnforcement for HostMtlsEnforcement {
    async fn probe(&self) -> Result<()> {
        // Earned-Trust: exercise the substrate the proxy relies on (kTLS arm +
        // agent-light forward splice) on a loopback sentinel and tear the sentinel
        // state down. Runs on a blocking task — the sentinel uses synchronous
        // rustls + raw setsockopt + splice.
        tokio::task::spawn_blocking(outbound::run_probe_sentinels).await.map_err(|e| {
            MtlsEnforcementError::Probe {
                which: ProbeSentinel::KtlsArmRoundTrip,
                message: format!("probe task panicked: {e}"),
            }
        })?
    }

    async fn enforce(&self, conn: InterceptedConnection) -> Result<EnforcedConnection> {
        match conn.routed {
            Routed::Outbound { .. } => self.enforce_outbound(conn).await,
            Routed::Inbound { .. } => self.enforce_inbound(conn).await,
        }
    }

    fn liveness(&self, handle: &EnforcedConnection) -> PumpLiveness {
        // Clone the shared pump state out and drop the map guard immediately —
        // never hold the lock past the read (no work happens under it).
        let pump = {
            let guard = self.conns.lock();
            match guard.get(handle.id()) {
                Some(state) => Arc::clone(&state.pump.state),
                None => return PumpLiveness::Gone, // torn down or never enforced
            }
        };
        if !pump.running.load(Ordering::SeqCst) {
            return PumpLiveness::Gone;
        }
        // Stalled iff a record is pending AND progress has not advanced for the
        // pump-stall deadline. A purely-idle connection (no pending record) is
        // Running, never Stalled.
        if pump.record_pending.load(Ordering::SeqCst) {
            let last = pump.last_progress_unix_nanos.load(Ordering::SeqCst);
            let stalled_for = now_unix_nanos().saturating_sub(last);
            let deadline_nanos =
                u64::try_from(self.limits.pump_stall_deadline.as_nanos()).unwrap_or(u64::MAX);
            if stalled_for >= deadline_nanos {
                return PumpLiveness::Stalled {
                    since: UnixInstant::from_unix_duration(Duration::from_nanos(last)),
                };
            }
        }
        PumpLiveness::Running
    }

    async fn teardown(&self, handle: EnforcedConnection) -> Result<()> {
        // Idempotent: tearing down an unknown/already-torn handle is Ok.
        let Some(mut state) = self.conns.lock().remove(handle.id()) else {
            return Ok(());
        };
        // Stop the pumps (joins their threads), then close the legs (OwnedFd drop).
        tokio::task::spawn_blocking(move || {
            state.pump.stop_and_join();
            for aux in &mut state.aux_pumps {
                aux.stop_and_join();
            }
            // legs drop here, closing the fds
            drop(state.legs);
        })
        .await
        .map_err(|e| MtlsEnforcementError::TeardownFailed {
            id: handle.id().clone(),
            source: std::io::Error::other(format!("teardown task panicked: {e}")),
        })?;
        Ok(())
    }
}

// ---- shared helpers used by the inbound/outbound flow modules ----

/// Drain pre-arm plaintext from `leg_fd` (bounded by `max_prearm_bytes`), with a
/// short read timeout so a non-speaking-first protocol returns an empty buffer
/// promptly. Returns the captured bytes; `BufferLimitExceeded` if the cap is hit.
fn drain_prearm(
    leg_fd: RawFd,
    max_prearm_bytes: usize,
    alloc: &AllocationId,
    settle: Duration,
) -> Result<Vec<u8>> {
    set_read_timeout(leg_fd, settle)?;
    let mut held = Vec::new();
    let mut buf = vec![0u8; 16384];
    // SAFETY: borrow the fd as a TcpStream WITHOUT taking ownership (forget at the
    // end so the leg fd is not closed).
    let stream = unsafe { TcpStream::from_raw_fd(leg_fd) };
    let result = (|| {
        loop {
            match (&stream).read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    held.extend_from_slice(&buf[..n]);
                    if held.len() > max_prearm_bytes {
                        return Err(MtlsEnforcementError::BufferLimitExceeded {
                            alloc: alloc.clone(),
                            max_prearm_bytes,
                        });
                    }
                    // One short read suffices for the walking skeleton's
                    // single-flight pre-arm; loop only while bytes keep arriving.
                    if n < buf.len() {
                        break;
                    }
                }
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    break;
                }
                Err(e) => return Err(MtlsEnforcementError::Io(e)),
            }
        }
        Ok(())
    })();
    std::mem::forget(stream);
    result?;
    Ok(held)
}

/// Drain every byte currently in `leg_fd`'s recv queue with non-blocking reads,
/// appending to `held` and bounding the total by `max_prearm_bytes`. Returns the
/// number of bytes drained on THIS pass (0 ⇒ the recv queue was empty). A single
/// pass: read until the first `EAGAIN`/`WouldBlock`, which means the queue has no
/// more readable bytes right now. The caller composes passes into the bounded
/// stable-empty loop in [`drain_and_flush_until_stable`].
fn drain_recv_queue_once(
    stream: &TcpStream,
    held: &mut Vec<u8>,
    buf: &mut [u8],
    max_prearm_bytes: usize,
    alloc: &AllocationId,
) -> Result<usize> {
    let mut drained = 0usize;
    loop {
        // `Read` is implemented for `&TcpStream`; read through a fresh `&mut &TcpStream`
        // so the borrow does not require an owned-mut binding.
        match (&mut &*stream).read(buf) {
            Ok(0) => break, // EOF — peer closed; nothing more to drain
            Ok(n) => {
                held.extend_from_slice(&buf[..n]);
                drained += n;
                if held.len() > max_prearm_bytes {
                    return Err(MtlsEnforcementError::BufferLimitExceeded {
                        alloc: alloc.clone(),
                        max_prearm_bytes,
                    });
                }
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break; // recv queue currently empty
            }
            Err(e) => return Err(MtlsEnforcementError::Io(e)),
        }
    }
    Ok(drained)
}

/// Drain leg F's recv queue to EMPTY right now (one bounded drain pass) and return
/// the captured bytes. Used as the final post-flip drain in `establish` (step 9, the
/// flip-moment guard): after `ARMED=1`, any byte that `SK_PASS`ed to leg F's own recv
/// queue in the window between the last capture read and the flip is read here and
/// flushed through leg B. After the `ARMED=1` store is visible, every leg-F skb whose
/// `sk_data_ready` fires sees `ARMED=1 → SK_REDIRECT`, so draining to `EAGAIN` leaves
/// no byte un-forwarded (research Finding 2/6). Non-blocking reads; stops at the first
/// `WouldBlock`/`EAGAIN` (queue currently empty). Bounded by `max_prearm_bytes`.
fn drain_recv_queue(
    leg_fd: RawFd,
    max_prearm_bytes: usize,
    alloc: &AllocationId,
) -> Result<Vec<u8>> {
    set_read_timeout(leg_fd, Duration::from_millis(1))?;
    let mut held = Vec::new();
    let mut buf = vec![0u8; 16384];
    // SAFETY: borrow the fd as a TcpStream WITHOUT taking ownership (forget at end).
    let stream = unsafe { TcpStream::from_raw_fd(leg_fd) };
    let result = drain_recv_queue_once(&stream, &mut held, &mut buf, max_prearm_bytes, alloc);
    std::mem::forget(stream);
    result?;
    Ok(held)
}

/// Drain every byte of already-decrypted plaintext rustls buffered during the
/// handshake, BEFORE `dangerous_extract_secrets` consumes the connection.
///
/// kTLS 0.5-RTT early-data correctness: the TLS writer finishes the handshake and
/// sends application_data immediately. A hand-rolled `read_tls`/`process_new_packets`
/// loop that stops at `!is_handshaking()` may have already pulled that early
/// application_data off the socket and decrypted it into rustls's internal plaintext
/// buffer (coalesced with / right after the peer's `Finished`). `read_seq` is then
/// advanced past those records — so the kTLS-RX arm (`rec_seq = read_seq`) correctly
/// resumes at the NEXT on-wire record, but the bytes rustls already decrypted live
/// ONLY in `conn.reader()` and would be lost. Draining them here and forwarding them
/// downstream ahead of the deliver/return pump makes the kTLS-RX leg lose no early
/// data. (The kernel-cork equivalent — `ktls::CorkStream` in the spikes — prevents
/// the over-read at the socket; this is the userspace-drain equivalent for the
/// hand-rolled synchronous rustls path, byte-for-byte correct because `read_seq`
/// already accounts for the consumed records.)
fn drain_early_plaintext(reader: &mut dyn Read) -> Vec<u8> {
    let mut early = Vec::new();
    let mut buf = [0u8; 16384];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => early.extend_from_slice(&buf[..n]),
            // `Reader` over rustls's in-memory plaintext buffer signals "no more
            // buffered plaintext right now" as `WouldBlock`; that is end-of-drain,
            // not an error (no more records have been decrypted yet).
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
    early
}

/// `setsockopt(SO_RCVTIMEO)` on a raw fd.
fn set_read_timeout(fd: RawFd, dur: Duration) -> Result<()> {
    let tv = libc::timeval {
        tv_sec: dur.as_secs() as libc::time_t,
        tv_usec: i64::from(dur.subsec_micros()) as libc::suseconds_t,
    };
    // SAFETY: SO_RCVTIMEO takes a `timeval`.
    let rc = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            std::ptr::from_ref(&tv).cast(),
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        return Err(MtlsEnforcementError::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Process-monotonic "now" in nanos for the pump progress metric.
fn now_unix_nanos() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO).as_nanos() as u64
}

/// Dial `peer` for the agent's own outbound leg B. The agent's dial is exempt
/// from the workload `cgroup_connect4` intercept by cgroup scoping (the program
/// attaches to the WORKLOAD subtree, not the agent's — F5), so no SO_MARK is
/// needed here; the agent process is not in the workload cgroup.
fn dial_leg(peer: SocketAddrV4, deadline: Duration) -> Result<TcpStream> {
    let stream =
        TcpStream::connect_timeout(&peer.into(), deadline).map_err(MtlsEnforcementError::Io)?;
    stream.set_nodelay(true).map_err(MtlsEnforcementError::Io)?;
    Ok(stream)
}

/// The `SO_MARK` the agent stamps on its INBOUND leg-S dial (F5 inbound
/// intercept-recursion exemption).
///
/// The nft-TPROXY `prerouting` rule intercepts the server's virtual address; the
/// agent's leg-S dial targets that same logical address the client aimed at, so
/// without this mark the SYN would be TPROXY'd back to the agent's leg-C listener,
/// recursing instead of reaching the server. The production nft-TPROXY rule
/// excludes this mark; the test harness mirrors it.
pub const MTLS_LEG_S_DIAL_MARK: u32 = 0x2;

/// Dial `peer` for the agent's INBOUND leg S (the server workload), stamping
/// [`MTLS_LEG_S_DIAL_MARK`] via `SO_MARK` so the nft-TPROXY prerouting rule skips
/// the agent's own dial (F5 intercept-recursion exemption — the inbound analogue
/// of the outbound leg-B cgroup-scoping exemption).
fn dial_leg_s(peer: SocketAddrV4, deadline: Duration) -> Result<TcpStream> {
    // Create the socket, set SO_MARK, THEN connect (the mark must be set before the
    // SYN so prerouting sees it on the outgoing packet).
    let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
    if sock < 0 {
        return Err(MtlsEnforcementError::Io(std::io::Error::last_os_error()));
    }
    let mark: u32 = MTLS_LEG_S_DIAL_MARK;
    // SAFETY: SO_MARK takes a u32 (needs CAP_NET_ADMIN — the agent has it).
    let rc = unsafe {
        libc::setsockopt(
            sock,
            libc::SOL_SOCKET,
            libc::SO_MARK,
            std::ptr::from_ref(&mark).cast(),
            std::mem::size_of::<u32>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        unsafe { libc::close(sock) };
        return Err(MtlsEnforcementError::Io(err));
    }
    // SAFETY: take ownership of the marked socket fd and connect via std.
    let stream = unsafe { std::os::fd::OwnedFd::from_raw_fd(sock) };
    let stream = TcpStream::from(stream);
    stream.set_nodelay(true).map_err(MtlsEnforcementError::Io)?;
    stream.connect_timeout_marked(peer, deadline)?;
    Ok(stream)
}

/// `connect_timeout` analogue for an already-created (marked) socket. `std`'s
/// `TcpStream::connect_timeout` creates its own socket, so we hand-roll a
/// non-blocking connect-with-deadline on the marked fd.
trait ConnectMarked {
    fn connect_timeout_marked(&self, peer: SocketAddrV4, deadline: Duration) -> Result<()>;
}

impl ConnectMarked for TcpStream {
    /// Connect the marked fd to `peer`, bounding the connect by `deadline`.
    ///
    /// `SO_RCVTIMEO` does NOT bound `connect(2)` — a broken DNAT/route would make a
    /// blocking `connect` hang until the kernel's TCP connect timeout (~127 s),
    /// wedging the `spawn_blocking` enforce task far past `handshake_deadline`. So
    /// the marked socket goes non-blocking, issues `connect` (expecting
    /// `EINPROGRESS`), waits for writability via `poll(POLLOUT)` with the remaining
    /// deadline, then reads `SO_ERROR` to learn the actual connect result. On a
    /// successful connect the fd is restored to BLOCKING mode (the downstream
    /// `splice` pumps + reads require blocking semantics). On deadline-exceed it
    /// returns `Io(TimedOut)` (fail-closed — `enforce`'s error path closes the owned
    /// legs; nothing is spliced to the server workload).
    fn connect_timeout_marked(&self, peer: SocketAddrV4, deadline: Duration) -> Result<()> {
        let fd = self.as_raw_fd();
        let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        sa.sin_family = libc::AF_INET as libc::sa_family_t;
        sa.sin_port = peer.port().to_be();
        sa.sin_addr.s_addr = u32::from_ne_bytes(peer.ip().octets());

        // Put the fd in non-blocking mode so `connect` returns immediately with
        // `EINPROGRESS` instead of blocking until the kernel's connect timeout.
        let prev_flags = get_fd_flags(fd)?;
        set_fd_flags(fd, prev_flags | libc::O_NONBLOCK)?;

        let connect_result = nonblocking_connect_with_deadline(fd, &sa, deadline);

        // Restore the prior (blocking) flags before returning, on every path — the
        // splice pumps and reads downstream require blocking semantics.
        let restore = set_fd_flags(fd, prev_flags);
        connect_result?;
        restore
    }
}

/// `fcntl(F_GETFL)` on a raw fd.
fn get_fd_flags(fd: RawFd) -> Result<libc::c_int> {
    // SAFETY: F_GETFL takes no extra argument on a valid fd.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags < 0 {
        return Err(MtlsEnforcementError::Io(std::io::Error::last_os_error()));
    }
    Ok(flags)
}

/// `fcntl(F_SETFL, flags)` on a raw fd.
fn set_fd_flags(fd: RawFd, flags: libc::c_int) -> Result<()> {
    // SAFETY: F_SETFL with an int flags argument on a valid fd.
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFL, flags) };
    if rc < 0 {
        return Err(MtlsEnforcementError::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Issue a non-blocking `connect` on `fd`, then `poll(POLLOUT)` with the remaining
/// `deadline` and read `SO_ERROR` to learn the connect outcome. The caller owns
/// setting/restoring `O_NONBLOCK`. Returns `Io(TimedOut)` if the deadline elapses
/// before the socket becomes writable, or the `SO_ERROR` errno if the connect failed.
fn nonblocking_connect_with_deadline(
    fd: RawFd,
    sa: &libc::sockaddr_in,
    deadline: Duration,
) -> Result<()> {
    // SAFETY: connect with a sockaddr_in on the marked fd; non-blocking so it returns
    // EINPROGRESS rather than blocking.
    let rc = unsafe {
        libc::connect(
            fd,
            std::ptr::from_ref(sa).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    if rc == 0 {
        return Ok(()); // connected immediately (loopback / already-resolved route)
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() != Some(libc::EINPROGRESS) {
        return Err(MtlsEnforcementError::Io(err)); // a real, immediate connect failure
    }

    // EINPROGRESS: wait for writability (connect completion) bounded by the deadline.
    let timeout_ms = i32::try_from(deadline.as_millis()).unwrap_or(i32::MAX);
    let mut pfd = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
    // SAFETY: poll a single owned pollfd.
    let pr = unsafe { libc::poll(std::ptr::from_mut(&mut pfd), 1, timeout_ms) };
    if pr < 0 {
        return Err(MtlsEnforcementError::Io(std::io::Error::last_os_error()));
    }
    if pr == 0 {
        // Deadline elapsed before the connect completed — fail-closed (a broken
        // DNAT/route must not pin the enforce task for ~127 s).
        return Err(MtlsEnforcementError::Io(std::io::Error::from(std::io::ErrorKind::TimedOut)));
    }

    // Writable: read SO_ERROR for the actual connect result (a non-zero value means
    // the connect failed asynchronously — ECONNREFUSED, EHOSTUNREACH, etc.).
    let mut so_error: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: SO_ERROR yields a c_int on the connected fd.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            std::ptr::from_mut(&mut so_error).cast(),
            std::ptr::from_mut(&mut len),
        )
    };
    if rc != 0 {
        return Err(MtlsEnforcementError::Io(std::io::Error::last_os_error()));
    }
    if so_error != 0 {
        return Err(MtlsEnforcementError::Io(std::io::Error::from_raw_os_error(so_error)));
    }
    Ok(())
}

impl HostMtlsEnforcement {
    /// OUTBOUND enforcement (`Direction::Outbound`).
    async fn enforce_outbound(&self, conn: InterceptedConnection) -> Result<EnforcedConnection> {
        let Routed::Outbound { peer } = conn.routed else {
            unreachable!("enforce_outbound dispatched on a non-Outbound routed fact");
        };
        let alloc = conn.alloc.clone();
        let svid = self.svid_or_fail(&alloc)?;
        let bundle = self.identity.current_bundle().ok_or(MtlsEnforcementError::AbsentBundle)?;
        let limits = self.limits;
        let leg_f = conn.leg;
        let id = self.next_id(alloc.clone());

        let established = tokio::task::spawn_blocking(move || {
            outbound::establish(leg_f, peer, &svid, &bundle, &alloc, limits)
        })
        .await
        .map_err(|e| {
            MtlsEnforcementError::Io(std::io::Error::other(format!("enforce task: {e}")))
        })??;

        Ok(self.register(id, established))
    }

    /// INBOUND enforcement (`Direction::Inbound`).
    async fn enforce_inbound(&self, conn: InterceptedConnection) -> Result<EnforcedConnection> {
        let Routed::Inbound { orig_dst } = conn.routed else {
            unreachable!("enforce_inbound dispatched on a non-Inbound routed fact");
        };
        let alloc = conn.alloc.clone();
        let svid = self.svid_or_fail(&alloc)?;
        let bundle = self.identity.current_bundle().ok_or(MtlsEnforcementError::AbsentBundle)?;
        let limits = self.limits;
        let leg_c = conn.leg;
        let id = self.next_id(alloc.clone());

        let established = tokio::task::spawn_blocking(move || {
            inbound::establish(leg_c, orig_dst, &svid, &bundle, &alloc, limits)
        })
        .await
        .map_err(|e| {
            MtlsEnforcementError::Io(std::io::Error::other(format!("enforce task: {e}")))
        })??;

        Ok(self.register(id, established))
    }
}

// The contract's `MtlsEnforcementError` is the adapter's error type — accessed
// via `overdrive_core::traits::mtls_enforcement::MtlsEnforcementError`. No
// adapter-local alias is invented here (the pinned contract names no
// `HostMtlsEnforcementError`).

/// Construct a `ConnState` with a primary pump + auxiliary pumps — both the
/// OUTBOUND and INBOUND sites. OUTBOUND: the primary forward pump
/// `splice(legF → legB)` plus the return pump `splice(legB → legF)` in `aux_pumps`.
/// INBOUND: the primary deliver pump `splice(legC → legS)` plus the response pump
/// `splice(legS → legC)` in `aux_pumps` (the GAP-2 S→C response leg).
const fn new_conn_state_bidi(
    legs: Vec<OwnedFd>,
    pump: PumpHandle,
    aux_pumps: Vec<PumpHandle>,
) -> ConnState {
    ConnState { legs, pump, aux_pumps }
}

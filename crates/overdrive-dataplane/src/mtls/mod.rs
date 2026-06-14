//! `HostMtlsEnforcement` — the production host adapter for the per-connection
//! transparent-mTLS enforcement port (ADR-0069, GH #26; OQ-2 home =
//! `overdrive-dataplane`).
//!
//! Implements [`MtlsEnforcement`](overdrive_core::traits::MtlsEnforcement) over
//! the spike-proven kernel primitives: rustls TLS 1.3 client/server handshakes
//! (consuming [`IdentityRead`](overdrive_core::traits::IdentityRead) for the held
//! SVID + trust bundle), kTLS arm (`setsockopt TCP_ULP/TLS_TX/TLS_RX`), and two
//! ASYMMETRIC agent-light pumps across the kTLS boundary (the two directions are
//! NOT the same primitive — the agent does no TLS crypto either way, but it copies
//! into a TX leg and zero-copies out of an RX leg):
//! - **Encrypt COPY pump** — forward (`legF → legB`) + response (`legS → legC`): a
//!   bounded userspace `read → write_all` COPY into a kTLS-TX leg; the kernel
//!   `tls_sw_sendmsg` encrypts each `write`.
//! - **Decrypt SPLICE pump** — return (`legB → legF`) + deliver (`legC → legS`): a
//!   zero-copy `splice` out of a kTLS-RX leg; `tls_sw_splice_read` decrypts each
//!   record on splice-out.
//!
//! The forward path was a sockmap egress redirect; it is now the `read → write_all`
//! COPY pump above — ASYMMETRIC to the splice (RX) directions, NOT symmetric to
//! them. The redirect `MSG_DONTWAIT`-stalled ~10–15% of records, and a `splice`
//! INTO a kTLS-TX socket loses records the same way (see
//! `docs/research/dataplane/sockmap-egress-redirect-into-ktls-tx-delivery-research.md`).
//!
//! Mechanism provenance (each method drives a spike-PROVEN syscall sequence):
//! - OUTBOUND lossless capture — `findings-userspace-relay.md` Unknown 1+2.
//! - write_all COPY into kTLS-TX (agent-light, forward + response) — `findings-splice-return.md`.
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
use overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::SvidMaterial;
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnection, EnforcedConnectionId, InterceptedConnection, MtlsEnforcement,
    MtlsEnforcementError, MtlsLimits, ProbeSentinel, PumpLiveness, Result, Routed,
};
use parking_lot::Mutex;

pub mod dataplane;
mod inbound;
mod ktls;
mod limits;
mod outbound;
mod splice;
mod supervision;
mod tls_config;

pub use dataplane::{MtlsCgroupLink, MtlsDataplane, MtlsDataplaneError};

use limits::InFlightLedger;
use splice::{PumpHandle, SelfTeardown};

/// Per-connection adapter-private tracking state, keyed by
/// [`EnforcedConnectionId`]. Holds the owned legs (closed on teardown) + the
/// splice-pump handle. NOT exposed on [`EnforcedConnection`].
struct ConnState {
    /// The agent-owned legs to close on teardown. OUTBOUND: leg F + leg B.
    /// INBOUND: leg C + leg S.
    legs: Vec<OwnedFd>,
    /// The primary pump — the request-carrying direction `liveness` observes.
    /// OUTBOUND: the forward encrypt pump (`read → write_all` COPY of leg F into
    /// leg B's kTLS-TX). INBOUND: the deliver pump (zero-copy `splice(legC → legS)`
    /// out of leg C's kTLS-RX).
    pump: PumpHandle,
    /// Auxiliary pumps torn down with the connection but NOT observed by
    /// `liveness`. OUTBOUND carries the return pump (zero-copy `splice(legB → legF)`
    /// out of leg B's kTLS-RX, which decrypts the peer's reply). INBOUND carries the
    /// response encrypt pump (`read → write_all` COPY of leg S into leg C's kTLS-TX
    /// — the S→C response leg, GAP 2 inbound half; leg C's kTLS-TX encrypts S's
    /// reply back to the client).
    aux_pumps: Vec<PumpHandle>,
}

/// The per-connection tracking table, keyed by id. `liveness`/`teardown` look here,
/// and the (B) self-teardown reaper drains an entry when its pump dies. `Arc`-shared
/// so the connection-level self-teardown trigger (installed into each pump's
/// `PumpState`) can drive the idempotent reclaim from a detached reaper thread
/// without holding `&self`.
type ConnTable = Arc<Mutex<std::collections::BTreeMap<EnforcedConnectionId, ConnState>>>;

/// The production host adapter for [`MtlsEnforcement`].
pub struct HostMtlsEnforcement {
    identity: Arc<dyn IdentityRead>,
    limits: MtlsLimits,
    next_counter: AtomicU64,
    /// Per-connection tracking, keyed by id. `liveness`/`teardown` look here.
    conns: ConnTable,
    /// Per-allocation in-flight (pre-arm) ceiling ledger (F4): a new intercept that
    /// would exceed `limits.max_inflight_per_alloc` for its alloc is refused
    /// fail-closed (`InFlightLimitExceeded`) — one workload cannot exhaust the agent
    /// by opening many stalled connections.
    inflight: InFlightLedger,
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
            conns: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
            inflight: InFlightLedger::new(),
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

    /// Register a steady-state-established connection in the tracking table, install
    /// the (B) D-MTLS-16 self-teardown trigger into its PRIMARY pump, and return its
    /// opaque handle.
    ///
    /// The trigger is installed ONLY into the primary (request-carrying,
    /// `liveness`-observed) pump — OUTBOUND forward / INBOUND deliver. It fires when
    /// that pump hits a transport-death terminal exit (a leg error / the (C)
    /// kernel-reaped `ETIMEDOUT`) that was NOT a deliberate `teardown`, running the
    /// same fail-closed reclaim `teardown` runs — close both legs, stop the sibling
    /// pump, drop the kTLS state — off a DETACHED reaper thread (so the calling pump
    /// never joins itself), re-homing the two F6 telemetry events from the retired
    /// central `MtlsSupervisor` to this per-connection path. The auxiliary
    /// (response-direction) pump does NOT carry the trigger: its finishing (a
    /// completed response, a clean EOF from the responder) is a half-close of the
    /// non-primary direction, not the connection's death — reclaiming on it would nuke
    /// a connection whose primary request path is still live (the regression that
    /// broke the established-connection Tier-3 tests).
    fn register(&self, id: EnforcedConnectionId, state: ConnState) -> EnforcedConnection {
        let trigger = self.self_teardown_trigger(id.clone());
        state.pump.state.install_self_teardown(trigger);
        self.conns.lock().insert(id.clone(), state);
        EnforcedConnection::new(id)
    }

    /// Build the (B) self-teardown trigger for connection `id`: emit
    /// `mtls.pump.stalled` for the pump that observed the terminal death, then spawn a
    /// detached reaper that idempotently reclaims the connection (drains it from the
    /// tracking table, stops both pumps, closes both legs) and emits
    /// `mtls.pump.teardown_on_stall`. Idempotent end-to-end: the first pump to die
    /// wins the `remove`; the sibling's later fire finds the entry already gone and is
    /// a harmless no-op.
    fn self_teardown_trigger(&self, id: EnforcedConnectionId) -> SelfTeardown {
        let conns = Arc::clone(&self.conns);
        Arc::new(move || {
            let alloc = id.alloc().clone();
            tracing::warn!(
                name: "mtls.pump.stalled",
                connection = %id,
                alloc = %alloc,
                "transparent-mTLS pump hit a transport-death exit (leg error / the (C) \
                 kernel-reaped ETIMEDOUT); self-tearing the connection down (F6/B). A clean \
                 EOF half-close does NOT reach here — only a connection death does."
            );
            let conns = Arc::clone(&conns);
            let id = id.clone();
            // Reclaim off a DETACHED reaper — `reclaim_connection` joins the pump
            // threads, and the calling thread IS one of those pumps; it must not join
            // itself.
            std::thread::spawn(move || {
                let torn = reclaim_connection(&conns, &id);
                tracing::warn!(
                    name: "mtls.pump.teardown_on_stall",
                    connection = %id,
                    alloc = %alloc,
                    reclaimed = torn,
                    "transparent-mTLS connection self-torn-down on terminal pump exit \
                     (F6/B per-connection reaction; the retired MtlsSupervisor's role)"
                );
            });
        })
    }
}

/// Idempotently reclaim a connection: drain its [`ConnState`] from the tracking
/// table, stop BOTH pumps (joining their threads), and close BOTH legs. Returns
/// `true` if this call performed the reclaim, `false` if the entry was already gone
/// (a racing `teardown` or the sibling pump's self-teardown won). Shared by the
/// deliberate `teardown` path and the (B) self-teardown reaper so both reclaim
/// identically.
fn reclaim_connection(
    conns: &Mutex<std::collections::BTreeMap<EnforcedConnectionId, ConnState>>,
    id: &EnforcedConnectionId,
) -> bool {
    let Some(mut state) = conns.lock().remove(id) else {
        return false; // already reclaimed by a racing teardown / sibling self-teardown
    };
    state.pump.stop_and_join();
    for aux in &mut state.aux_pumps {
        aux.stop_and_join();
    }
    // legs drop here, closing the fds
    drop(state.legs);
    true
}

#[async_trait]
impl MtlsEnforcement for HostMtlsEnforcement {
    async fn probe(&self) -> Result<()> {
        // Earned-Trust: exercise the substrate the proxy relies on (kTLS arm +
        // agent-light forward encrypt pump) on a loopback sentinel and tear the
        // sentinel state down. Runs on a blocking task — the sentinel uses
        // synchronous rustls + raw setsockopt + the `read → write_all` COPY pump.
        tokio::task::spawn_blocking(outbound::run_probe_sentinels).await.map_err(|e| {
            MtlsEnforcementError::Probe {
                which: ProbeSentinel::KtlsArmRoundTrip,
                message: format!("probe task panicked: {e}"),
            }
        })?
    }

    async fn enforce(&self, conn: InterceptedConnection) -> Result<EnforcedConnection> {
        // F4 in-flight ceiling: claim one per-alloc pre-arm slot BEFORE establishing.
        // The 129th concurrent pre-arm for an alloc (the one finding the count already
        // at `max_inflight_per_alloc`) is refused fail-closed — no leg is touched, no
        // cleartext. The guard releases the slot when this call returns (established or
        // failed), so the ceiling counts genuinely-concurrent pre-arms.
        let _slot = self
            .inflight
            .try_claim(&conn.alloc, self.limits.max_inflight_per_alloc)
            .ok_or_else(|| MtlsEnforcementError::InFlightLimitExceeded {
                alloc: conn.alloc.clone(),
                limit: self.limits.max_inflight_per_alloc,
            })?;
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
        // The F6 Stalled derivation is the pure `supervision::derive_liveness`
        // (extracted so the 30 s × record-pending boundary is unit/mutation-testable):
        // Gone if the pump exited, Running while moving OR idle-but-ready, Stalled iff
        // a record is pending AND progress has not advanced for `pump_stall_deadline`.
        supervision::derive_liveness(
            pump.running.load(Ordering::SeqCst),
            pump.record_pending.load(Ordering::SeqCst),
            pump.last_progress_unix_nanos.load(Ordering::SeqCst),
            now_unix_nanos(),
            self.limits.pump_stall_deadline,
        )
    }

    async fn teardown(&self, handle: EnforcedConnection) -> Result<()> {
        // Idempotent: tearing down an unknown/already-torn handle is Ok. Shares
        // `reclaim_connection` with the (B) self-teardown reaper — both stop the pumps
        // (joining their threads) and close the legs identically; a connection already
        // self-torn-down by its own dead pump is a harmless no-op here (the `remove`
        // returns `None`). Runs on a blocking task because `stop_and_join` joins the
        // pump threads.
        let conns = Arc::clone(&self.conns);
        let id = handle.id().clone();
        tokio::task::spawn_blocking(move || reclaim_connection(&conns, &id)).await.map_err(
            |e| MtlsEnforcementError::TeardownFailed {
                id: handle.id().clone(),
                source: std::io::Error::other(format!("teardown task panicked: {e}")),
            },
        )?;
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
                    if limits::prearm_exceeds(held.len(), max_prearm_bytes) {
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
                if limits::prearm_exceeds(held.len(), max_prearm_bytes) {
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

/// (C) D-MTLS-16 / ADR-0070: keepalive probe cadence for a steady-state leg. The
/// kernel sends a keepalive probe after `idle` of silence, then a probe every
/// `interval` until `count` consecutive probes go unacked, at which point the
/// socket fails `ETIMEDOUT`. These are an ADAPTER concern (ADR-0070 § "Tuning the
/// socket-option values is an adapter concern") — NOT operator-tunable in v1 and
/// deliberately NOT an `MtlsLimits` field (that struct is the F4/F7 contract,
/// pinned unchanged by D-MTLS-16). They detect a peer that has silently vanished
/// without sending a FIN/RST so the kernel's `TCP_USER_TIMEOUT` deadline (derived
/// from `pump_stall_deadline`) has a steady stream of unacked probes to time out
/// against on an otherwise-idle connection.
const KEEPALIVE_IDLE_SECS: libc::c_int = 10;
const KEEPALIVE_INTERVAL_SECS: libc::c_int = 5;
const KEEPALIVE_PROBE_COUNT: libc::c_int = 3;

/// (C) D-MTLS-16 / ADR-0070: arm the kernel's transport-death reaping on an
/// agent-owned steady-state leg, BEFORE the SD-2 pumps start. Sets
/// `TCP_USER_TIMEOUT` (the max time the kernel keeps retransmitting unacked data —
/// or unacked keepalive probes — before failing the socket `ETIMEDOUT`) to the
/// connection's no-progress deadline `pump_stall_deadline`, and enables TCP
/// keepalive so a peer that vanished without a FIN/RST still produces unacked
/// probes for `TCP_USER_TIMEOUT` to reap against. When the kernel reaps the leg the
/// (B) pump task observes the resulting `ETIMEDOUT`/error and self-tears-down — no
/// userspace tick, no central enumerator (the retired `MtlsSupervisor` shape).
///
/// `pump_stall_deadline` is the existing `MtlsLimits` field for the connection's
/// no-progress window; the `TCP_USER_TIMEOUT` value derives from it rather than
/// from a new field (ADR-0070 keeps `MtlsLimits` unchanged). The literal values are
/// an adapter concern; the acceptance test asserts the observable reaping, not the
/// numbers.
///
/// **Best-effort, NOT a correctness gate.** Transport-death reaping is a kernel
/// optimization on the TCP legs; a leg that does not support these `IPPROTO_TCP`
/// options (an `AF_UNIX` socketpair in tests, or any non-TCP leg) returns
/// `EOPNOTSUPP`/`ENOPROTOOPT` and is SKIPPED with a warning — the connection is NOT
/// refused. Aborting `enforce` on an unsupported transport-death option would
/// fail-closed a connection whose proxying is otherwise fine; the ADR scopes (C) as
/// an adapter tuning aid, not a precondition. Any OTHER errno (a genuine setsockopt
/// failure) still propagates fail-closed.
fn arm_transport_death_timeouts(fd: RawFd, pump_stall_deadline: Duration) -> Result<()> {
    // TCP_USER_TIMEOUT is milliseconds; saturate rather than wrap an over-long
    // deadline (a >49-day deadline is nonsensical here but must not silently flip).
    let user_timeout_ms =
        libc::c_uint::try_from(pump_stall_deadline.as_millis()).unwrap_or(libc::c_uint::MAX);
    set_best_effort_tcp_opt(
        fd,
        libc::IPPROTO_TCP,
        libc::TCP_USER_TIMEOUT,
        user_timeout_ms as libc::c_int,
    )?;
    set_best_effort_tcp_opt(fd, libc::SOL_SOCKET, libc::SO_KEEPALIVE, 1)?;
    set_best_effort_tcp_opt(fd, libc::IPPROTO_TCP, libc::TCP_KEEPIDLE, KEEPALIVE_IDLE_SECS)?;
    set_best_effort_tcp_opt(fd, libc::IPPROTO_TCP, libc::TCP_KEEPINTVL, KEEPALIVE_INTERVAL_SECS)?;
    set_best_effort_tcp_opt(fd, libc::IPPROTO_TCP, libc::TCP_KEEPCNT, KEEPALIVE_PROBE_COUNT)?;
    Ok(())
}

/// `setsockopt(level, name, &(value: c_int))` on a raw fd, TOLERATING a leg that does
/// not support the option (`EOPNOTSUPP`/`ENOPROTOOPT` ⇒ skip-with-warn, `Ok(())`).
/// Used for the (C) transport-death options, which are a best-effort kernel-reaping
/// aid (see [`arm_transport_death_timeouts`]) — not a correctness gate. Any other
/// errno propagates as a genuine `Io` failure.
fn set_best_effort_tcp_opt(
    fd: RawFd,
    level: libc::c_int,
    name: libc::c_int,
    value: libc::c_int,
) -> Result<()> {
    // SAFETY: each option named here takes a `c_int` argument.
    let rc = unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            std::ptr::from_ref(&value).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if rc == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        // The leg is not a TCP socket (AF_UNIX socketpair, etc.) — transport-death
        // reaping does not apply; skip this option, do NOT refuse the connection.
        Some(libc::EOPNOTSUPP | libc::ENOPROTOOPT) => {
            tracing::warn!(
                name: "mtls.transport_death.unsupported",
                level,
                option = name,
                "leg does not support a (C) transport-death socket option; skipping (best-effort)"
            );
            Ok(())
        }
        _ => Err(MtlsEnforcementError::Io(err)),
    }
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

/// Dial `peer` for the agent's INBOUND leg S (the server workload), stamping
/// [`MTLS_LEG_S_DIAL_MARK`](overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK)
/// via `SO_MARK` so the nft-TPROXY prerouting rule skips the agent's own dial
/// (F5 intercept-recursion exemption — the inbound analogue of the outbound
/// leg-B cgroup-scoping exemption).
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
/// OUTBOUND and INBOUND sites. OUTBOUND: the primary forward encrypt pump
/// (`read → write_all` COPY of leg F into leg B's kTLS-TX) plus the return pump
/// (zero-copy `splice(legB → legF)` out of leg B's kTLS-RX) in `aux_pumps`.
/// INBOUND: the primary deliver pump (zero-copy `splice(legC → legS)` out of leg
/// C's kTLS-RX) plus the response encrypt pump (`read → write_all` COPY of leg S
/// into leg C's kTLS-TX) in `aux_pumps` (the GAP-2 S→C response leg).
const fn new_conn_state_bidi(
    legs: Vec<OwnedFd>,
    pump: PumpHandle,
    aux_pumps: Vec<PumpHandle>,
) -> ConnState {
    ConnState { legs, pump, aux_pumps }
}

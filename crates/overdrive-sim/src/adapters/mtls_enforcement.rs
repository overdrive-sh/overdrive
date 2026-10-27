//! `SimMtlsEnforcement` — in-memory [`MtlsEnforcement`] double for DST
//! (transparent-mtls-host-socket, ADR-0069 F3; GH #26).
//!
//! The sim counterpart to the host `HostMtlsEnforcement` (`overdrive-dataplane`).
//! It models the handshake **OUTCOME** the contract defines — `Established`
//! (return an [`EnforcedConnection`]) vs the matching fail-closed
//! [`MtlsEnforcementError`] — driven entirely by the preloaded
//! [`SimIdentityRead`](super::SimIdentityRead) the consumer injects, in BOTH
//! directions (the observable contract is identical either way per the trait
//! docstring). It does **NO real crypto** — no rustls, no kTLS, no `splice` — and
//! reads the held SVID + bundle ONLY through the injected
//! [`IdentityRead`](overdrive_core::traits::IdentityRead) port (#26 is a READER,
//! never an issuer).
//!
//! # What it models, and what it does not
//!
//! The `mtls_enforcement_equivalence` DST harness drives BOTH this double and the
//! host adapter through the same call sequence and asserts the SAME observable
//! outcome (`.claude/rules/development.md` § "The DST equivalence test is the
//! structural guard"). The observable surface the contract pins and this double
//! mirrors:
//!
//! - **`enforce` outcome** starts with the preloaded identity read:
//!   - `svid_for(conn.alloc) == None` ⇒ `Err(AbsentSvid)` (the held-set
//!     fail-closed signal — `identity_read.rs` clause 3);
//!   - `current_bundle() == None` ⇒ `Err(AbsentBundle)` (no anchor to verify the
//!     peer against);
//!   - both present and an inbound target in the guest prefix ⇒ await the
//!     injected `GuestStreamDial` with `DialPurpose::Relay`; a dial refusal is
//!     returned as `GuestDial` and a paired stream is held by the simulated
//!     relay until its end or teardown.
//!   - other routes ⇒ `Ok(EnforcedConnection)` after the identity reads. The
//!     real TLS handshake and kTLS arm are NOT modelled (no crypto in the sim).
//! - **`liveness`** — `Running` while established, `Gone` after teardown (or for
//!   an unknown handle — the post-teardown observable, mirroring `Driver::status`
//!   returning `NotFound` after `stop`).
//! - **`teardown`** — closes the owned leg (drops the held [`OwnedFd`]) and is
//!   idempotent (tearing down an unknown/already-torn handle is `Ok(())`).
//! - **`probe`** — always `Ok(())`: the in-process sim substrate honours its
//!   contract by construction (no kernel kTLS arm to fail).
//!
//! `conn.leg` is satisfiable by any real fd the scenario owns (a
//! `UnixStream::pair()` end); the double takes ownership and drops it on teardown,
//! so the fd-reclaim invariant (no leg leak) is observable.
//!
//! # Determinism
//!
//! Per-connection ids draw from a monotonic counter (no entropy); the
//! per-connection table is a [`BTreeMap`] keyed by [`EnforcedConnectionId`] for
//! deterministic iteration (§ "Ordered-collection choice"). The double carries no
//! clock / entropy / crypto — the outcome is a deterministic function of the
//! preloaded identity state.
//!
//! # Dependency discipline
//!
//! `SimMtlsEnforcement::new` takes the `IdentityRead` port + [`MtlsLimits`] as
//! **required constructor parameters** — no builder, no production-binding default
//! (`.claude/rules/development.md` § "Port-trait dependencies"), mirroring
//! `HostMtlsEnforcement::new`.

use std::collections::BTreeMap;
use std::io;
use std::net::TcpStream as StdTcpStream;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use overdrive_core::AllocationId;
use overdrive_core::guest_transport::GUEST_NODE_PREFIX;
use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestStream, GuestStreamDial, GuestStreamEnd,
};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnection, EnforcedConnectionId, InterceptedConnection, MtlsEnforcement,
    MtlsEnforcementError, MtlsLimits, PumpLiveness, Result, Routed,
};
use overdrive_core::wall_clock::UnixInstant;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// A scripted fail-closed limit trip the DST equivalence harness arms for the NEXT
/// `enforce` of a given allocation.
///
/// The sim has no real pre-arm buffer / handshake timer to overflow, so the harness
/// models the F4/F7 limit trips as scripted preconditions — the OUTCOME the contract
/// pins (the cause-distinct error) is the observable surface both adapters must agree
/// on (the host adapter trips the SAME error from a REAL buffer overflow / handshake
/// deadline on the kernel; the equivalence is on the outcome, not the mechanism).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptedTrip {
    /// The pre-arm buffer cap was exceeded ⇒ `BufferLimitExceeded` (F4). The
    /// `max_prearm_bytes` carried in the error is the adapter's configured limit.
    BufferLimitExceeded,
    /// The handshake-and-arm exceeded the deadline ⇒ `HandshakeTimeout` (F4). The
    /// `deadline` carried in the error is the adapter's configured limit.
    HandshakeTimeout,
    /// The client's certificate does not chain to the held trust bundle ⇒
    /// `PeerVerificationFailed` (R5-54, LEGS-DIAL: refused before any leg-S
    /// dial). The host adapter reaches the same outcome from a real
    /// verification failure in the blocking handshake stage.
    PeerVerification,
}

/// Per-connection adapter-private tracking — the owned leg or relay the double
/// closes on teardown. Keyed by [`EnforcedConnectionId`], mirroring the host
/// adapter's `ConnState` while replacing its kernel pump with a Tokio relay.
struct SimConnState {
    /// The agent-owned leg handed over by `enforce` (`conn.leg`). Dropped on
    /// teardown — closing it makes the fd-reclaim invariant observable.
    _leg: Option<OwnedFd>,
    /// The simulated relay owns leg C and its `GuestStream` while active.
    relay: Option<JoinHandle<()>>,
    /// Stops the relay before teardown waits for it.
    relay_stop: Option<oneshot::Sender<()>>,
    /// F6: when set by [`SimMtlsEnforcement::mark_stalled`], `liveness` reports
    /// `Stalled { since }` for this connection (the harness drives the
    /// Running→Stalled transition the worker's supervisor reacts to). The host
    /// adapter derives this from the real pump's frozen progress metric; the sim
    /// scripts it, so the equivalence is on the `Stalled`-then-teardown OUTCOME.
    stalled_since: Option<UnixInstant>,
}

impl Drop for SimConnState {
    fn drop(&mut self) {
        if let Some(stop) = self.relay_stop.take() {
            let _ = stop.send(());
        }
    }
}

/// In-memory [`MtlsEnforcement`] double for DST.
///
/// Reads the held SVID + bundle through the injected `IdentityRead` port and maps
/// the preloaded identity state to the handshake outcome. `Send + Sync + 'static`
/// (held as `Arc<dyn MtlsEnforcement>`), matching the host adapter.
pub struct SimMtlsEnforcement {
    identity: Arc<dyn IdentityRead>,
    limits: MtlsLimits,
    next_counter: AtomicU64,
    /// Per-connection tracking, keyed by id — `liveness` / `teardown` look here.
    conns: Mutex<BTreeMap<EnforcedConnectionId, SimConnState>>,
    /// Per-alloc count of currently in-flight (pre-arm, not-yet-established)
    /// connections — the F4 in-flight ceiling the contract pins. Faithful to the
    /// host adapter's `InFlightLedger`: a new `enforce` whose count is already at
    /// `max_inflight_per_alloc` is refused fail-closed (`InFlightLimitExceeded`).
    /// In the sim, `enforce` completes synchronously, so the harness arms a
    /// `held_inflight` count via [`SimMtlsEnforcement::hold_inflight`] to model N
    /// concurrent pre-arms held open.
    held_inflight: Mutex<BTreeMap<AllocationId, u32>>,
    /// Per-alloc scripted limit trip armed for the NEXT `enforce` (consumed on use).
    scripted_trips: Mutex<BTreeMap<AllocationId, ScriptedTrip>>,
    /// The guest-flow owner's dial port (LEGS-DIAL, D-14): the inbound arm
    /// dials it for an `orig_dst` inside the guest prefix, after the client
    /// authenticated, and relays over the returned stream — the same contract
    /// as `HostMtlsEnforcement`.
    guest_dial: Arc<dyn GuestStreamDial>,
}

impl SimMtlsEnforcement {
    /// Construct the double from its REQUIRED dependencies — the held-identity read
    /// port (#35, never an issuer), the F7 resource contract, and the guest-flow
    /// owner's dial port leg S reaches a VM backend through (LEGS-DIAL). All
    /// mandatory: no defaulting, no builder (a consumer that forgets the port
    /// fails to compile, the discipline the trait surface exists to enforce).
    #[must_use]
    pub fn new(
        identity: Arc<dyn IdentityRead>,
        limits: MtlsLimits,
        guest_dial: Arc<dyn GuestStreamDial>,
    ) -> Self {
        Self {
            identity,
            limits,
            next_counter: AtomicU64::new(0),
            conns: Mutex::new(BTreeMap::new()),
            held_inflight: Mutex::new(BTreeMap::new()),
            scripted_trips: Mutex::new(BTreeMap::new()),
            guest_dial,
        }
    }

    /// The construction-time resource bounds (F4/F7). Read-only; pinned at
    /// construction, not operator-tunable in v1 (mirrors `HostMtlsEnforcement`).
    #[must_use]
    pub const fn limits(&self) -> &MtlsLimits {
        &self.limits
    }

    /// DST scripting (F4): model `count` concurrent in-flight (pre-arm) connections
    /// already held open for `alloc`, so the NEXT `enforce` for `alloc` sees the
    /// per-alloc in-flight ceiling at `count`. Used by the equivalence harness to
    /// drive the `max_inflight_per_alloc` trip: hold `max` open, then assert the next
    /// `enforce` is refused `InFlightLimitExceeded`.
    pub fn hold_inflight(&self, alloc: &AllocationId, count: u32) {
        self.held_inflight.lock().insert(alloc.clone(), count);
    }

    /// DST scripting (F4): arm a cause-distinct limit trip for the NEXT `enforce` of
    /// `alloc` (consumed on use). The sim has no real buffer/timer to overflow, so
    /// the harness scripts the OUTCOME the contract pins — the cause-distinct error
    /// the host adapter trips from a REAL overflow/deadline on the kernel.
    pub fn script_trip(&self, alloc: &AllocationId, trip: ScriptedTrip) {
        self.scripted_trips.lock().insert(alloc.clone(), trip);
    }

    /// DST scripting (F6): mark `handle`'s pump `Stalled { since }` so `liveness`
    /// reports the stall the worker's supervisor reacts to. Models the host adapter's
    /// real frozen-progress derivation; the equivalence is on the
    /// `Stalled`-then-teardown→`Gone` OUTCOME.
    pub fn mark_stalled(&self, handle: &EnforcedConnection, since: UnixInstant) {
        if let Some(state) = self.conns.lock().get_mut(handle.id()) {
            state.stalled_since = Some(since);
        }
    }
}

#[async_trait]
impl MtlsEnforcement for SimMtlsEnforcement {
    async fn probe(&self) -> Result<()> {
        // The in-process sim substrate honours its contract by construction — there
        // is no kernel kTLS arm to fail. (`probe` mutates no enforced connection and
        // leaks no sentinel state — there is none.)
        Ok(())
    }

    async fn enforce(&self, conn: InterceptedConnection) -> Result<EnforcedConnection> {
        // F4 in-flight ceiling: a new pre-arm whose per-alloc in-flight count is
        // already at `max_inflight_per_alloc` is refused fail-closed
        // (`InFlightLimitExceeded`) — faithful to the host adapter's `InFlightLedger`.
        // The harness arms the held count via `hold_inflight`; the gate is the SAME
        // `current >= max` boundary the host adapter enforces.
        let current_inflight = self.held_inflight.lock().get(&conn.alloc).copied().unwrap_or(0);
        if current_inflight >= self.limits.max_inflight_per_alloc {
            return Err(MtlsEnforcementError::InFlightLimitExceeded {
                alloc: conn.alloc,
                limit: self.limits.max_inflight_per_alloc,
            });
        }

        // F4 scripted limit trip (consumed on use): the harness arms a cause-distinct
        // overflow/deadline trip; the OUTCOME (the cause-distinct error) is the
        // observable surface both adapters must agree on.
        let scripted = self.scripted_trips.lock().remove(&conn.alloc);
        if let Some(trip) = scripted {
            return Err(match trip {
                ScriptedTrip::BufferLimitExceeded => MtlsEnforcementError::BufferLimitExceeded {
                    alloc: conn.alloc,
                    max_prearm_bytes: self.limits.max_prearm_bytes,
                },
                ScriptedTrip::HandshakeTimeout => MtlsEnforcementError::HandshakeTimeout {
                    alloc: conn.alloc,
                    deadline: self.limits.handshake_deadline,
                },
                ScriptedTrip::PeerVerification => MtlsEnforcementError::PeerVerificationFailed {
                    reason: "scripted: the client certificate does not chain to the bundle"
                        .to_owned(),
                },
            });
        }

        // The sim models only the authentication outcome, not TLS or kTLS. Read
        // the held SVID + bundle through the injected port before any leg-S dial.
        let svid = self
            .identity
            .svid_for(&conn.alloc)
            .ok_or_else(|| MtlsEnforcementError::AbsentSvid { alloc: conn.alloc.clone() })?;
        let _bundle = self.identity.current_bundle().ok_or(MtlsEnforcementError::AbsentBundle)?;
        // `svid` is read through the port to prove the held material is present and
        // owned-cloned (the host adapter presents it on the kTLS leg); the sim does
        // not perform the handshake, so it is not otherwise consumed.
        let _ = &svid;

        let allocation = conn.alloc;
        let (owned_leg, relay, relay_stop) = match conn.routed {
            Routed::Inbound { orig_dst } if GUEST_NODE_PREFIX.contains(orig_dst.ip()) => {
                let stream = match self.guest_dial.dial(orig_dst, DialPurpose::Relay).await {
                    Ok(stream) => stream,
                    Err(source) => {
                        reset_owned_leg(&conn.leg);
                        return Err(MtlsEnforcementError::GuestDial { source });
                    }
                };
                let leg = tcp_stream_from_leg(conn.leg).map_err(MtlsEnforcementError::Io)?;
                let (relay_stop, stop) = oneshot::channel();
                let relay = tokio::spawn(relay_guest_stream(leg, stream, stop));
                (None, Some(relay), Some(relay_stop))
            }
            _ => (Some(conn.leg), None, None),
        };

        // Steady-state-established: mint a stable per-connection id and retain
        // either the untouched leg or its paired guest relay until teardown.
        let counter = self.next_counter.fetch_add(1, Ordering::Relaxed);
        let id = EnforcedConnectionId::new(allocation, counter);
        self.conns.lock().insert(
            id.clone(),
            SimConnState { _leg: owned_leg, relay, relay_stop, stalled_since: None },
        );
        Ok(EnforcedConnection::new(id))
    }

    fn liveness(&self, handle: &EnforcedConnection) -> PumpLiveness {
        // Gone once torn down or for an unknown handle (the post-teardown observable);
        // Stalled if the harness scripted a stall on this connection (F6 — the worker
        // supervisor reacts to it); Running otherwise. The sim's stall is scripted
        // (`mark_stalled`) where the host adapter derives it from the real pump's
        // frozen progress metric — the equivalence is on the variant.
        {
            let conns = self.conns.lock();
            conns.get(handle.id()).map_or_else(
                || PumpLiveness::Gone,
                |state| {
                    state.stalled_since.map_or_else(
                        || {
                            if state.relay.as_ref().is_some_and(JoinHandle::is_finished) {
                                PumpLiveness::Gone
                            } else {
                                PumpLiveness::Running
                            }
                        },
                        |since| PumpLiveness::Stalled { since },
                    )
                },
            )
        }
    }

    async fn teardown(&self, handle: EnforcedConnection) -> Result<()> {
        // Idempotent: tearing down an unknown/already-torn handle is Ok. Stop
        // and join a simulated relay before its owned GuestStream is dropped.
        let Some(mut state) = self.conns.lock().remove(handle.id()) else { return Ok(()) };
        if let Some(stop) = state.relay_stop.take() {
            let _ = stop.send(());
        }
        if let Some(relay) = state.relay.take() {
            let _ = relay.await;
        }
        Ok(())
    }
}

fn tcp_stream_from_leg(leg: OwnedFd) -> io::Result<TcpStream> {
    let raw_fd = leg.into_raw_fd();
    // SAFETY: `InterceptedConnection` supplies a connected TCP leg C owned by
    // this `OwnedFd`; ownership transfers to the standard stream exactly once.
    let stream = unsafe { StdTcpStream::from_raw_fd(raw_fd) };
    stream.set_nonblocking(true)?;
    TcpStream::from_std(stream)
}

fn reset_owned_leg(leg: &OwnedFd) {
    let linger = libc::linger { l_onoff: 1, l_linger: 0 };
    // SAFETY: `leg` is the live accepted TCP descriptor; zero linger requests
    // the reset required when the owner dial refuses.
    let _ = unsafe {
        libc::setsockopt(
            leg.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            std::ptr::from_ref(&linger).cast(),
            libc::socklen_t::try_from(std::mem::size_of_val(&linger))
                .unwrap_or(libc::socklen_t::MAX),
        )
    };
}

enum RelayRead {
    Stopped,
    Leg(io::Result<usize>),
    Guest(io::Result<usize>),
}

async fn relay_guest_stream(
    mut leg: TcpStream,
    mut guest: GuestStream,
    mut stop: oneshot::Receiver<()>,
) {
    let mut leg_open = true;
    let mut guest_open = true;
    let mut leg_bytes = vec![0_u8; 16 * 1024];
    let mut guest_bytes = vec![0_u8; 16 * 1024];

    while leg_open || guest_open {
        let event = tokio::select! {
            biased;
            _ = &mut stop => RelayRead::Stopped,
            result = leg.read(&mut leg_bytes), if leg_open => RelayRead::Leg(result),
            result = guest.read(&mut guest_bytes), if guest_open => RelayRead::Guest(result),
        };

        match event {
            RelayRead::Stopped | RelayRead::Leg(Err(_)) => return,
            RelayRead::Leg(Ok(0)) => {
                leg_open = false;
                let result = tokio::select! {
                    biased;
                    _ = &mut stop => return,
                    result = guest.shutdown_write() => result,
                };
                if result.is_err() {
                    return;
                }
            }
            RelayRead::Leg(Ok(read)) => {
                if write_guest(&mut guest, &leg_bytes[..read], &mut stop).await.is_err() {
                    reset_tcp_stream(&leg);
                    return;
                }
            }
            RelayRead::Guest(Ok(0)) => match guest.end() {
                Some(GuestStreamEnd::HalfClose) => {
                    guest_open = false;
                    if leg.shutdown().await.is_err() {
                        return;
                    }
                }
                Some(GuestStreamEnd::Abort) | None => {
                    reset_tcp_stream(&leg);
                    return;
                }
            },
            RelayRead::Guest(Ok(read)) => {
                if write_leg(&mut leg, &guest_bytes[..read], &mut stop).await.is_err() {
                    reset_tcp_stream(&leg);
                    return;
                }
            }
            RelayRead::Guest(Err(_)) => {
                reset_tcp_stream(&leg);
                return;
            }
        }
    }
}

async fn write_guest(
    guest: &mut GuestStream,
    bytes: &[u8],
    stop: &mut oneshot::Receiver<()>,
) -> io::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        let result = tokio::select! {
            biased;
            _ = &mut *stop => return Err(io::Error::new(io::ErrorKind::Interrupted, "relay stopped")),
            result = guest.write(&bytes[written..]) => result,
        };
        match result {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(count) => written += count,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

async fn write_leg(
    leg: &mut TcpStream,
    bytes: &[u8],
    stop: &mut oneshot::Receiver<()>,
) -> io::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        let result = tokio::select! {
            biased;
            _ = &mut *stop => return Err(io::Error::new(io::ErrorKind::Interrupted, "relay stopped")),
            result = leg.write(&bytes[written..]) => result,
        };
        match result {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(count) => written += count,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn reset_tcp_stream(stream: &TcpStream) {
    let linger = libc::linger { l_onoff: 1, l_linger: 0 };
    // SAFETY: `stream` owns this live TCP leg; zero linger requests a reset.
    let _ = unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            std::ptr::from_ref(&linger).cast(),
            libc::socklen_t::try_from(std::mem::size_of_val(&linger))
                .unwrap_or(libc::socklen_t::MAX),
        )
    };
}

/// The simulation [`MtlsEnforcementFactory`](overdrive_core::traits::mtls_enforcement::MtlsEnforcementFactory).
///
/// NO-OVERRIDE-SEAMS, ADR-0219: every `enforcement` call returns a fresh
/// [`SimMtlsEnforcement`] over the boot's identity, limits and guest dial.
#[derive(Debug, Default)]
pub struct SimMtlsEnforcementFactory {
    _private: (),
}

impl SimMtlsEnforcementFactory {
    /// A factory with nothing scripted.
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }
}

impl overdrive_core::traits::mtls_enforcement::MtlsEnforcementFactory
    for SimMtlsEnforcementFactory
{
    fn enforcement(
        &self,
        deps: overdrive_core::traits::mtls_enforcement::MtlsEnforcementDeps,
    ) -> Arc<dyn MtlsEnforcement> {
        Arc::new(SimMtlsEnforcement::new(deps.identity, deps.limits, deps.guest_dial))
    }
}

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
//! - **`enforce` outcome** = a pure function of the preloaded identity read:
//!   - `svid_for(conn.alloc) == None` ⇒ `Err(AbsentSvid)` (the held-set
//!     fail-closed signal — `identity_read.rs` clause 3);
//!   - `current_bundle() == None` ⇒ `Err(AbsentBundle)` (no anchor to verify the
//!     peer against);
//!   - both present ⇒ `Ok(EnforcedConnection)` — steady-state-established. The
//!     real kTLS / handshake / pump mechanics the host adapter performs are NOT
//!     modelled (no crypto in the sim); the outcome IS the observable contract.
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
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use overdrive_core::AllocationId;
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnection, EnforcedConnectionId, InterceptedConnection, MtlsEnforcement,
    MtlsEnforcementError, MtlsLimits, PumpLiveness, Result,
};
use overdrive_core::wall_clock::UnixInstant;
use parking_lot::Mutex;

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
}

/// Per-connection adapter-private tracking — the owned leg the double closes on
/// teardown. Keyed by [`EnforcedConnectionId`], mirroring the host adapter's
/// `ConnState` (which additionally holds the real pump handles the sim has no
/// equivalent for).
struct SimConnState {
    /// The agent-owned leg handed over by `enforce` (`conn.leg`). Dropped on
    /// teardown — closing it makes the fd-reclaim invariant observable.
    _leg: OwnedFd,
    /// F6: when set by [`SimMtlsEnforcement::mark_stalled`], `liveness` reports
    /// `Stalled { since }` for this connection (the harness drives the
    /// Running→Stalled transition the worker's supervisor reacts to). The host
    /// adapter derives this from the real pump's frozen progress metric; the sim
    /// scripts it, so the equivalence is on the `Stalled`-then-teardown OUTCOME.
    stalled_since: Option<UnixInstant>,
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
}

impl SimMtlsEnforcement {
    /// Construct the double from its REQUIRED dependencies — the held-identity read
    /// port (#35, never an issuer) and the F7 resource contract. Both mandatory: no
    /// defaulting, no builder (a consumer that forgets the port fails to compile,
    /// the discipline the trait surface exists to enforce).
    #[must_use]
    pub fn new(identity: Arc<dyn IdentityRead>, limits: MtlsLimits) -> Self {
        Self {
            identity,
            limits,
            next_counter: AtomicU64::new(0),
            conns: Mutex::new(BTreeMap::new()),
            held_inflight: Mutex::new(BTreeMap::new()),
            scripted_trips: Mutex::new(BTreeMap::new()),
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
            });
        }

        // The handshake OUTCOME is a pure function of the preloaded identity read,
        // identical in both directions (the observable contract does not branch on
        // `conn.routed`). Read the held SVID + bundle through the injected port —
        // fail-closed on absence (no real crypto; the outcome IS the contract).
        let svid = self
            .identity
            .svid_for(&conn.alloc)
            .ok_or_else(|| MtlsEnforcementError::AbsentSvid { alloc: conn.alloc.clone() })?;
        let _bundle = self.identity.current_bundle().ok_or(MtlsEnforcementError::AbsentBundle)?;
        // `svid` is read through the port to prove the held material is present and
        // owned-cloned (the host adapter presents it on the kTLS leg); the sim does
        // not perform the handshake, so it is not otherwise consumed.
        let _ = &svid;

        // Steady-state-established: mint a stable per-connection id and take ownership
        // of the leg (closed on teardown — fd-reclaim invariant observable).
        let counter = self.next_counter.fetch_add(1, Ordering::Relaxed);
        let id = EnforcedConnectionId::new(conn.alloc, counter);
        self.conns.lock().insert(id.clone(), SimConnState { _leg: conn.leg, stalled_since: None });
        Ok(EnforcedConnection::new(id))
    }

    fn liveness(&self, handle: &EnforcedConnection) -> PumpLiveness {
        // Gone once torn down or for an unknown handle (the post-teardown observable);
        // Stalled if the harness scripted a stall on this connection (F6 — the worker
        // supervisor reacts to it); Running otherwise. The sim's stall is scripted
        // (`mark_stalled`) where the host adapter derives it from the real pump's
        // frozen progress metric — the equivalence is on the variant.
        let stalled = self.conns.lock().get(handle.id()).map(|state| state.stalled_since);
        match stalled {
            None => PumpLiveness::Gone,
            Some(None) => PumpLiveness::Running,
            Some(Some(since)) => PumpLiveness::Stalled { since },
        }
    }

    async fn teardown(&self, handle: EnforcedConnection) -> Result<()> {
        // Idempotent: tearing down an unknown/already-torn handle is Ok. Removing the
        // entry drops the owned leg (closes the fd) — no leg leak.
        self.conns.lock().remove(handle.id());
        Ok(())
    }
}

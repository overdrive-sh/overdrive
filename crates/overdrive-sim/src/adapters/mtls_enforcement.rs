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
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnection, EnforcedConnectionId, InterceptedConnection, MtlsEnforcement,
    MtlsEnforcementError, MtlsLimits, PumpLiveness, Result,
};
use parking_lot::Mutex;

/// Per-connection adapter-private tracking — the owned leg the double closes on
/// teardown. Keyed by [`EnforcedConnectionId`], mirroring the host adapter's
/// `ConnState` (which additionally holds the real pump handles the sim has no
/// equivalent for).
struct SimConnState {
    /// The agent-owned leg handed over by `enforce` (`conn.leg`). Dropped on
    /// teardown — closing it makes the fd-reclaim invariant observable.
    _leg: OwnedFd,
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
        }
    }

    /// The construction-time resource bounds (F4/F7). Read-only; pinned at
    /// construction, not operator-tunable in v1 (mirrors `HostMtlsEnforcement`).
    #[must_use]
    pub const fn limits(&self) -> &MtlsLimits {
        &self.limits
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
        self.conns.lock().insert(id.clone(), SimConnState { _leg: conn.leg });
        Ok(EnforcedConnection::new(id))
    }

    fn liveness(&self, handle: &EnforcedConnection) -> PumpLiveness {
        // Running while the connection is established; Gone once torn down or for an
        // unknown handle (the post-teardown observable). The sim models no stall —
        // there is no real pump whose progress could halt.
        if self.conns.lock().contains_key(handle.id()) {
            PumpLiveness::Running
        } else {
            PumpLiveness::Gone
        }
    }

    async fn teardown(&self, handle: EnforcedConnection) -> Result<()> {
        // Idempotent: tearing down an unknown/already-torn handle is Ok. Removing the
        // entry drops the owned leg (closes the fd) — no leg leak.
        self.conns.lock().remove(handle.id());
        Ok(())
    }
}

//! [`MtlsEnforcement`] — the per-connection transparent-mTLS enforcement port
//! (ADR-0069). The agent-light L4 proxy's driven contract, **bidirectional (F3)**:
//! bring a transparently-intercepted workload connection — OUTBOUND (the workload
//! is the client) OR INBOUND (the workload is the server) — to a
//! steady-state-established mTLS session, observe the agent-light pump's
//! liveness, and tear the connection down. `enforce` dispatches on
//! [`InterceptedConnection::routed`] (the [`Direction`]):
//! - **OUTBOUND**: lossless capture on leg F → rustls CLIENT handshake on leg B
//!   presenting the held SVID → arm kTLS-TX/RX → agent-light FORWARD pump
//!   (`legF → legB`, a bounded `read → write_all` COPY into leg B's kTLS-TX; the
//!   kernel `tls_sw_sendmsg` encrypts each `write`) + RETURN pump
//!   (`splice(legB → legF)`, a zero-copy `splice` out of leg B's kTLS-RX).
//! - **INBOUND**: TPROXY-intercept → `getsockname` orig-dst → rustls SERVER
//!   handshake on leg C (present the server SVID, REQUIRE+VERIFY the client SVID
//!   chains to the bundle via `WebPkiClientVerifier`) → arm kTLS-RX → dial the
//!   server workload (leg S) → DELIVER pump (`splice(legC → legS)`, a zero-copy
//!   `splice` out of leg C's kTLS-RX) + RESPONSE pump (`legS → legC`, a bounded
//!   `read → write_all` COPY into leg C's kTLS-TX); fail-closed on
//!   `nocert`/`wrongca` (`findings-inbound-intercept.md`).
//!
//! **Agent-light is per-direction, and the two directions are NOT symmetric.**
//! "Agent-light" means the agent does NO TLS crypto in userspace — the kernel
//! kTLS engine encrypts/decrypts; the agent only moves bytes between the
//! plaintext leg and the kTLS leg. The MECHANISM differs by direction:
//! - **DECRYPT / RX directions** (outbound RETURN `splice(legB→legF)`, inbound
//!   DELIVER `splice(legC→legS)`): a genuine **zero-copy `splice`** out of a
//!   plain kTLS-RX leg — `tls_sw_splice_read` decrypts each record on splice-out;
//!   the agent issues only `splice`/`ppoll`, ZERO per-byte userspace copy, ~1
//!   splice per TLS record (`findings-splice-return.md`, increment-h). The
//!   stronger **agent-idle / zero-per-byte-syscall** property holds HERE.
//! - **ENCRYPT / TX directions** (outbound FORWARD `legF→legB`, inbound RESPONSE
//!   `legS→legC`): a **bounded userspace `read → write_all` COPY** into a kTLS-TX
//!   leg — the kernel `tls_sw_sendmsg` encrypts each `write`, so the agent does
//!   ZERO crypto, but it DOES copy each record's plaintext through a userspace
//!   buffer and issues a `read`+`write` per record. **NOT zero-copy, NOT
//!   agent-idle, NOT zero-syscall.** A `splice` INTO a kTLS-TX socket is NOT
//!   used: `splice(pipe → ktls_tx)` consumes the bytes and reports success but
//!   the `tls_sw` splice/sendpage path does not reliably emit the record (the
//!   same `MSG_DONTWAIT`-backlog loss class the abandoned sockmap egress redirect
//!   suffered; trace-confirmed `n_out=55 errno=0` while the peer received 0
//!   bytes — `sockmap-egress-redirect-into-ktls-tx-delivery-research.md`). The
//!   blocking `write_all` is the proven kTLS-TX primitive
//!   (`crates/overdrive-dataplane/src/mtls/splice.rs`, the SSOT; SHIPPED +
//!   verified 20/20 at commit `bb6489ef`).
//!
//! Production wires `HostMtlsEnforcement` (over kTLS / `splice` /
//! `cgroup_connect4` / `nft`-TPROXY+`IP_TRANSPARENT`,
//! consuming [`IdentityRead`](crate::traits::IdentityRead)); simulation wires
//! `SimMtlsEnforcement` (in-memory observable-contract mirror). The
//! `mtls_enforcement_equivalence` DST harness drives both through the same call
//! sequence (both directions) and asserts identical observable state
//! (`.claude/rules/development.md` § "The DST equivalence test is the structural
//! guard").
//!
//! Consumes [`IdentityRead`](crate::traits::IdentityRead) (#35) as a REQUIRED
//! constructor parameter — #26 is a READER, never an issuer (D-MTLS-9). BOTH the
//! client AND the server workload hold NOTHING.
//!
//! **Scope (F1/F5 — authn + encryption, NOT authz; NO intended-peer pinning in
//! v1).** This port AUTHENTICATES the peer (**chain-to-trust-bundle** only — that
//! the peer is *some* valid cluster workload) and ENCRYPTS the wire (kTLS), in
//! BOTH directions. It does NOT AUTHORIZE the connection — allow/deny is the
//! BPF-LSM `socket_connect` hook
//! ([#27](https://github.com/overdrive-sh/overdrive/issues/27)) fed by compiled
//! `policy_verdicts` ([#38](https://github.com/overdrive-sh/overdrive/issues/38);
//! related [#49](https://github.com/overdrive-sh/overdrive/issues/49)), a SEPARATE
//! subsystem this port MUST NOT duplicate (no policy engine, no Regorus, no
//! `policy_verdicts` read here). It also does NOT pin the *intended* peer:
//! expected-destination identity pinning (`expected_peer` + `PeerIdentityMismatch`)
//! is the [#242](https://github.com/overdrive-sh/overdrive/issues/242) UPGRADE
//! (east-west SPIFFE-ID resolution supplies the expected peer); **v1 is authn-only**
//! (`expected_peer == None`). **A routing bug / VIP collision / malicious
//! in-cluster endpoint presenting a valid-but-unintended SVID is NOT prevented in
//! v1** — the honest v1 claim is "chain-to-bundle transport authn + encryption, no
//! intended-peer pinning." Docs/tests MUST NOT call the wrong-but-valid-peer case
//! "protected" until #242 lands (F5).
//!
//! Resource-bounded by [`MtlsLimits`] (F4/F7): bounded pre-arm buffer (256 KiB),
//! handshake deadline (5 s), per-allocation in-flight ceiling (128), pump-stall
//! deadline (30 s, F6) — all fail-closed, never queue-unbounded; CONCRETE v1
//! defaults the acceptance tests assert. Construction takes [`MtlsLimits`]
//! alongside [`IdentityRead`](crate::traits::IdentityRead).

use std::fmt::{self, Display, Formatter};
use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::id::IdParseError;
use crate::wall_clock::UnixInstant;
use crate::{AllocationId, SpiffeId};

/// Result alias used throughout the crate's mTLS-enforcement surface.
pub type Result<T, E = MtlsEnforcementError> = std::result::Result<T, E>;

/// Which half of the proxy this intercepted connection is (F3 — bidirectional).
/// Outbound = the workload is the CLIENT (its connect() was cgroup_connect4-
/// rewritten to the agent); Inbound = the workload is the SERVER (a connection
/// to its logical address was TPROXY-intercepted to the agent). `enforce`
/// dispatches on this; the observable contract is identical either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// The intercepted workload is the connection's CLIENT (outbound connect()).
    /// `cgroup_connect4` intercept → rustls CLIENT handshake on leg B →
    /// agent-light forward pump (`legF → legB`, a `read → write_all` copy into
    /// leg B's kTLS-TX) + return splice (`splice(legB → legF)` out of kTLS-RX).
    Outbound,
    /// The intercepted workload is the connection's SERVER (inbound accept()).
    /// TPROXY intercept → `getsockname` orig-dst → rustls SERVER handshake on
    /// leg C (present server SVID, verify client SVID) → splice-to-server.
    Inbound,
}

/// The direction-specific routing fact carried alongside the owned leg (F3).
#[derive(Debug, Clone, Copy)]
pub enum Routed {
    /// OUTBOUND: the real peer `SocketAddrV4` leg B must dial to originate the
    /// outbound mTLS session. V4 per the single-node Phase-1 scope (multi-node
    /// transparent mTLS is OUT of v1 scope — Phase 1 is single-node);
    /// `SocketAddrV4` matches the `Dataplane` local-backend call shape.
    Outbound { peer: std::net::SocketAddrV4 },
    /// INBOUND: the original destination the TPROXY listener recovered via
    /// `getsockname()` on the accepted leg C (`findings-inbound-intercept.md`
    /// §1 / Mechanics #1 — under TPROXY the orig-dst IS the accepted socket's
    /// local addr, NOT `SO_ORIGINAL_DST`). This is what selects the SERVER
    /// workload's `AllocationId` → its held SVID. The adapter dials the
    /// server workload's real plaintext socket (leg S) inside `enforce`.
    Inbound { orig_dst: std::net::SocketAddrV4 },
}

impl Routed {
    /// The [`Direction`] this routing fact selects — a projection of the
    /// `Routed` discriminant (not new public behavior; `Routed`/`Direction` are
    /// the pinned contract types and this is their derived view).
    #[must_use]
    pub const fn direction(self) -> Direction {
        match self {
            Self::Outbound { .. } => Direction::Outbound,
            Self::Inbound { .. } => Direction::Inbound,
        }
    }
}

/// One transparently-intercepted workload connection to enforce, in either
/// direction (F3).
///
/// OUTBOUND: produced after the `cgroup_connect4`-rewrite intercept lands the
/// workload on the agent's leg-F listener and the agent `accept()`s it
/// (`findings-userspace-relay.md` Unknown 1). The owned `leg` is leg F (the
/// workload-facing plaintext leg — the one outbound case where the intercepted
/// leg is plaintext); `routed` is `Outbound { peer }` (the real peer leg B
/// dials).
///
/// INBOUND: produced after the `nft` TPROXY + `IP_TRANSPARENT` intercept lands
/// a connection aimed at the server workload's logical address on the agent's
/// leg-C listener and the agent `accept()`s it (`findings-inbound-intercept.md`
/// §1). The owned `leg` is leg C (the CLIENT-facing TLS/kTLS leg — NOT
/// plaintext: for inbound the agent-owned kTLS leg IS the accepted intercepted
/// leg, and the plaintext leg S to the server workload is opened by the adapter
/// inside `enforce`). `routed` is `Inbound { orig_dst }` (the
/// `getsockname`-recovered original destination that selects the server SVID).
///
/// Workload-holds-nothing (D-MTLS-9): this descriptor never carries SVID
/// material — only the plaintext/kTLS leg fd and the routing facts. The proxy
/// reads the SVID through [`IdentityRead`](crate::traits::IdentityRead) inside
/// `enforce`. Both the client AND the server workload hold NOTHING.
#[derive(Debug)]
pub struct InterceptedConnection {
    /// The agent-owned leg the worker `accept()`ed for this intercepted
    /// connection, handed over by value (the port takes ownership, RAII close
    /// on teardown). OUTBOUND: leg F (workload-facing plaintext). INBOUND:
    /// leg C (client-facing — the kTLS leg the agent terminates TLS on).
    /// Owned, not borrowed: the port's lifecycle outlives the worker's call
    /// frame (the pump runs after `enforce` returns).
    pub leg: std::os::fd::OwnedFd,
    /// The direction discriminant + its direction-specific routing fact.
    pub routed: Routed,
    /// Whose held SVID to present. OUTBOUND: the CLIENT workload's SVID (the
    /// `IdentityRead::svid_for` key). INBOUND: the SERVER workload's SVID,
    /// selected by `Routed::Inbound { orig_dst }` → `AllocationId`. Either way
    /// `svid_for(alloc) == None` is the fail-closed signal (`enforce` returns
    /// `AbsentSvid`).
    pub alloc: AllocationId,
    /// OPTIONAL expected-destination SPIFFE identity (F1 / authn-vs-authz
    /// boundary). When `Some`, `enforce` SAN-matches the authenticated peer
    /// against it and returns `PeerIdentityMismatch` on a wrong-but-valid
    /// peer (one that chains to the trust bundle but is NOT the intended
    /// destination). **v1 leaves this `None` (authn-only)** in BOTH directions:
    /// #26 enforces chain-to-trust-bundle authentication only, and the
    /// expected-peer identity is supplied DOWNSTREAM by east-west SPIFFE-ID
    /// resolution ([#242](https://github.com/overdrive-sh/overdrive/issues/242),
    /// which "terminates in SPIFFE mTLS via sockops (#26)"). The field + the
    /// `PeerIdentityMismatch` variant are reserved now so the SAN-match wires
    /// the moment #242 supplies it — no contract change later. This is NOT
    /// authorization (allow/deny is #27's BPF-LSM `socket_connect` hook fed by
    /// #38's `policy_verdicts`); it is *identity pinning* of an
    /// already-authenticated peer. **v1 = chain-to-bundle transport authn +
    /// encryption, NO intended-peer pinning** (F5).
    pub expected_peer: Option<SpiffeId>,
}

/// Stable per-connection correlation id (the alloc + a monotonic per-connection
/// counter), for log/liveness correlation only. NOT a security identity — the
/// SVID identity is the allocation's, presented on the agent's kTLS leg.
///
/// Derived from `(AllocationId, u64)` — content-addressed within a node session,
/// no entropy. The canonical string form is `<alloc>#<counter>`; `FromStr`
/// round-trips it, so DST and telemetry can name a connection stably
/// (`.claude/rules/development.md` § "Newtype completeness").
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EnforcedConnectionId {
    alloc: AllocationId,
    counter: u64,
}

impl EnforcedConnectionId {
    /// Assemble a correlation id from the allocation whose connection is being
    /// enforced and a monotonic per-connection counter (no entropy — stable and
    /// content-addressed within a node session).
    #[must_use]
    pub const fn new(alloc: AllocationId, counter: u64) -> Self {
        Self { alloc, counter }
    }

    /// The allocation whose held SVID this connection presents.
    #[must_use]
    pub const fn alloc(&self) -> &AllocationId {
        &self.alloc
    }

    /// The monotonic per-connection counter within the node session.
    #[must_use]
    pub const fn counter(&self) -> u64 {
        self.counter
    }
}

impl Display for EnforcedConnectionId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.alloc, self.counter)
    }
}

impl FromStr for EnforcedConnectionId {
    type Err = EnforcedConnectionIdParseError;

    fn from_str(raw: &str) -> std::result::Result<Self, Self::Err> {
        let (alloc_part, counter_part) =
            raw.rsplit_once('#').ok_or(EnforcedConnectionIdParseError::MissingSeparator)?;
        let alloc = AllocationId::new(alloc_part)?;
        let counter = counter_part
            .parse::<u64>()
            .map_err(|_| EnforcedConnectionIdParseError::MalformedCounter)?;
        Ok(Self { alloc, counter })
    }
}

impl Serialize for EnforcedConnectionId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for EnforcedConnectionId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// Parse / validation failure for [`EnforcedConnectionId::from_str`]. The
/// failure-mode taxonomy is total: a missing `#` separator, an allocation part
/// that is not a valid [`AllocationId`], or a counter part that is not a `u64`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EnforcedConnectionIdParseError {
    /// The input had no `#` separator between the allocation and the counter.
    #[error("EnforcedConnectionId must contain a `#` separating alloc and counter")]
    MissingSeparator,
    /// The allocation component did not parse as a valid [`AllocationId`].
    #[error("EnforcedConnectionId allocation component is invalid: {0}")]
    InvalidAlloc(#[from] IdParseError),
    /// The counter component did not parse as a `u64`.
    #[error("EnforcedConnectionId counter component is not a u64")]
    MalformedCounter,
}

/// Opaque handle to a connection the proxy has brought to
/// steady-state-established. Returned by [`MtlsEnforcement::enforce`]; consumed
/// by [`MtlsEnforcement::teardown`]; correlated by [`MtlsEnforcement::liveness`].
/// The worker does NOT inspect the adapter-private tracking state — only
/// [`id`](EnforcedConnection::id) (a stable correlation key) is caller-readable,
/// mirroring `Driver`'s opaque `AllocationHandle`.
#[derive(Debug, Clone)]
pub struct EnforcedConnection {
    /// Stable per-connection correlation id (the alloc + a monotonic
    /// per-connection counter), for log/liveness correlation only. NOT a
    /// security identity — the SVID identity is `alloc`'s, presented on the
    /// agent's kTLS leg.
    id: EnforcedConnectionId,
    // adapter-private: leg-F/leg-B fds and the forward/return pump task handles.
    // Not exposed; the host adapter owns them.
}

impl EnforcedConnection {
    /// Assemble a handle wrapping its stable correlation id. The adapter
    /// constructs this once `enforce` has reached steady-state-established; the
    /// adapter-private tracking state (leg fds, pump task handles) lives
    /// in the adapter's own per-connection table keyed by this `id`.
    #[must_use]
    pub const fn new(id: EnforcedConnectionId) -> Self {
        Self { id }
    }

    /// The stable correlation id — the only caller-readable field (the
    /// `AllocationHandle` analogue; the worker does not inspect contents).
    #[must_use]
    pub const fn id(&self) -> &EnforcedConnectionId {
        &self.id
    }
}

/// Resource bounds for the lossless pre-arm capture (F4) + the F6 pump-stall
/// deadline. Construction-time, not per-connection: the adapter holds one
/// `MtlsLimits` and applies it to every `enforce`. Fail-closed on every limit
/// — never queue-unbounded, never degrade to cleartext. The F7 defaults are
/// CONCRETE (not "sensible defaults"): the acceptance tests assert these exact
/// values, not merely field existence.
#[derive(Debug, Clone, Copy)]
pub struct MtlsLimits {
    /// Max pre-arm plaintext bytes buffered per connection before kTLS arms.
    /// Exceeding it ⇒ `BufferLimitExceeded`: drop the buffer, reset the
    /// plaintext leg (leg F outbound / leg S not-yet-opened inbound), no
    /// cleartext egresses. **F7 default: 256 KiB (262_144).** Rationale: covers
    /// a request-first protocol's first flight (HTTP/2 headers, a gRPC request,
    /// a Postgres startup) while the handshake completes in single-digit ms,
    /// two orders of magnitude below what a stalled peer could otherwise pin.
    pub max_prearm_bytes: usize,
    /// Deadline for the handshake-and-arm (leg B outbound / leg C inbound).
    /// Exceeding it ⇒ `HandshakeTimeout`: the stalled peer cannot pin agent
    /// resources. **F7 default: 5 s.** Rationale: a same-node / east-west mTLS
    /// handshake completes in ms; 5 s distinguishes a dead/stalled peer from
    /// normal GC/scheduler variance without false-tripping.
    pub handshake_deadline: Duration,
    /// Max concurrent in-flight (pre-arm, not-yet-armed) connections per
    /// `AllocationId`. Over-limit ⇒ the new intercept is refused fail-closed
    /// (`InFlightLimitExceeded`), so one workload cannot exhaust the agent by
    /// opening many stalled connections. **F7 default: 128.** Rationale: a
    /// healthy workload arms each connection in ms, so 128 concurrent *pre-arm*
    /// connections is far above any legitimate burst yet caps the
    /// amplification one workload can inflict.
    pub max_inflight_per_alloc: u32,
    /// F6 — the no-progress window after which the liveness-observed primary pump
    /// (OUTBOUND forward `read → write_all` copy / INBOUND deliver `splice`) is
    /// `PumpLiveness::Stalled`: the bytes-moved counter has not advanced for this
    /// long WHILE a record is pending on the pump's source leg. The worker tears
    /// the connection down on `Stalled` (teardown + fail-closed reset). A purely
    /// idle connection (no pending record) is `Running`, never `Stalled`.
    /// **F7 default: 30 s.** Rationale: generous enough that no healthy bursty
    /// connection trips it, tight enough that a stranded pump is reclaimed
    /// promptly.
    pub pump_stall_deadline: Duration,
}

impl Default for MtlsLimits {
    /// The F7 v1 defaults — pinned, not operator-tunable in v1. The acceptance
    /// tests assert these exact values.
    fn default() -> Self {
        Self {
            max_prearm_bytes: 256 * 1024, // 256 KiB
            handshake_deadline: Duration::from_secs(5),
            max_inflight_per_alloc: 128,
            pump_stall_deadline: Duration::from_secs(30),
        }
    }
}

/// Liveness of a connection's agent-light primary pump (OUTBOUND forward
/// `read → write_all` copy / INBOUND deliver `splice`). F6 supervision is (C)+(B)
/// (ADR-0070 / D-MTLS-16): the kernel reaps transport-death via
/// `TCP_USER_TIMEOUT`/keepalive and the per-connection pump task self-tears-down on
/// its terminal exit; `liveness` is the observe surface, not a tick-driven query (see
/// [`MtlsEnforcement::liveness`] § "F6 supervision shape (C)+(B)").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpLiveness {
    /// The pump is moving records OR idle-but-ready (no record pending) — the
    /// path is live. A quiescent long-lived connection is `Running`, not `Stalled`.
    Running,
    /// The pump's bytes-moved progress metric has NOT advanced since `since`,
    /// for at least [`MtlsLimits::pump_stall_deadline`] (F7 default 30 s), WHILE a
    /// record is pending on the pump's source leg (a stranded/crashed pump — the
    /// path is broken; ADR-0069 § ATAM reliability sensitivity). RETAINED as the
    /// RESERVED predicate for the deferred per-connection progress-stall watchdog
    /// (#232); NOT driven by a central tick in v1 (ADR-0070 / D-MTLS-16).
    Stalled { since: UnixInstant },
    /// No live pump for this handle — torn down or never enforced (post-teardown
    /// observable; not an error).
    Gone,
}

/// Which probe sentinel failed (for the refuse-to-start diagnosis). The substrate
/// the proxy now relies on is "kTLS arm + agent-light forward encrypt round-trip"
/// — a single composed round-trip sentinel (the obsolete sockmap-egress-redirect
/// and arming-order sentinels were dropped with the sockmap mechanism).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeSentinel {
    /// kTLS arm on a loopback leg + the agent-light forward encrypt pump
    /// (`read → write_all` into the kTLS-TX leg) moving one record through it,
    /// reconstructed byte-exact by the peer's kTLS-RX (`findings.md` A; the
    /// shipped `crates/overdrive-dataplane/src/mtls/`).
    KtlsArmRoundTrip,
}

impl Display for ProbeSentinel {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::KtlsArmRoundTrip => "ktls-arm-forward-encrypt-round-trip",
        };
        f.write_str(s)
    }
}

/// Cause-distinct failure modes for the mTLS-enforcement surface (no catch-all
/// `Internal(String)` per `.claude/rules/development.md` § Errors). Each variant
/// names a distinct remediation. The `#[error("...")]` strings are the operator's
/// diagnostic surface.
#[derive(Debug, Error)]
pub enum MtlsEnforcementError {
    /// `IdentityRead::svid_for(&alloc) == None` — no held SVID for the
    /// allocation whose connection was intercepted. The fail-closed signal:
    /// the proxy refuses the handshake rather than presenting a stale/absent
    /// credential (constraint: "Reader of `IdentityRead`, never an issuer";
    /// `identity_read.rs` clause 3). Distinct from `AbsentBundle` so the
    /// operator sees WHICH side of the identity read was empty.
    #[error("no held SVID for allocation {alloc}; refusing handshake (fail-closed)")]
    AbsentSvid { alloc: AllocationId },

    /// `IdentityRead::current_bundle() == None` — no hydrated trust bundle to
    /// verify the peer against. Fail-closed: the proxy will not complete a
    /// handshake it cannot verify. Distinct from `AbsentSvid` (own identity) —
    /// this is the peer-verification anchor.
    #[error("no hydrated trust bundle; cannot verify peer (fail-closed)")]
    AbsentBundle,

    /// The leg-B rustls TLS 1.3 handshake aborted — a TLS alert, a wrong/expired
    /// presented SVID rejected by the peer, or a handshake timeout. No kTLS was
    /// armed; no cleartext egressed. Carries the rustls-side reason as a string
    /// (the rustls error is not a stable typed surface to embed).
    #[error("leg-B TLS handshake failed: {reason}")]
    HandshakeFailed { reason: String },

    /// The peer's presented certificate did not chain to `current_bundle()`'s
    /// anchor. Fail-closed: the connection is refused. This is the
    /// **authentication** failure (the peer is not a valid cluster workload).
    /// Distinct from `HandshakeFailed` so a peer *identity* rejection is not
    /// conflated with a transport/alert failure — the operator's remediation
    /// differs (wrong peer identity vs broken TLS).
    #[error("peer verification failed against the trust bundle: {reason}")]
    PeerVerificationFailed { reason: String },

    /// The authenticated peer chains to the trust bundle (it IS a valid cluster
    /// workload) but its SPIFFE-SAN does NOT match the expected destination
    /// `InterceptedConnection::expected_peer` — a wrong-but-valid peer. This is
    /// **expected-destination identity pinning** (F1), NOT authorization (that is
    /// #27's BPF-LSM `socket_connect` hook). Reserved now; **v1 never produces it
    /// because `expected_peer` is `None`** — it wires the moment east-west
    /// SPIFFE-ID resolution
    /// ([#242](https://github.com/overdrive-sh/overdrive/issues/242)) supplies the
    /// expected peer. Distinct from `PeerVerificationFailed` (authn) so the
    /// "valid peer, wrong destination" case is diagnosable on its own.
    #[error("peer identity mismatch: authenticated peer is not the expected destination: {reason}")]
    PeerIdentityMismatch { reason: String },

    /// The workload streamed more than `MtlsLimits::max_prearm_bytes` of pre-arm
    /// plaintext into leg F before kTLS armed on leg B (F4 — the DoS guard on the
    /// lossless capture buffer). Fail-closed: the buffer is dropped, leg F reset,
    /// NO cleartext egresses. Cause-distinct (NOT a generic `Io`) so the operator
    /// sees a resource-limit trip, not an I/O error.
    #[error(
        "pre-arm buffer limit exceeded for allocation {alloc}: capped at {max_prearm_bytes} bytes (fail-closed)"
    )]
    BufferLimitExceeded { alloc: AllocationId, max_prearm_bytes: usize },

    /// The leg-B handshake-and-arm did not complete within
    /// `MtlsLimits::handshake_deadline` (F4 — a stalled peer must not pin agent
    /// resources). Fail-closed: legs closed, no cleartext. Distinct from
    /// `HandshakeFailed` (an active TLS abort) — this is the *deadline* trip, a
    /// different remediation (slow/stalled peer vs broken TLS).
    #[error("leg-B handshake exceeded deadline {deadline:?} for allocation {alloc} (fail-closed)")]
    HandshakeTimeout { alloc: AllocationId, deadline: Duration },

    /// The per-allocation in-flight (pre-arm) connection ceiling
    /// `MtlsLimits::max_inflight_per_alloc` is already reached for this
    /// allocation (F4 — one workload cannot exhaust the agent by opening many
    /// stalled connections). Fail-closed: the new intercept is refused, no
    /// cleartext. Backpressure is *refuse*, never *queue-unbounded*.
    #[error(
        "in-flight connection limit {limit} reached for allocation {alloc}; refusing new intercept (fail-closed)"
    )]
    InFlightLimitExceeded { alloc: AllocationId, limit: u32 },

    /// `setsockopt(TCP_ULP "tls")` / `TLS_TX` / `TLS_RX` refused on the kTLS leg
    /// (leg B outbound / leg C inbound) after a successful handshake — the kTLS
    /// arm itself failed (kernel rejected the crypto_info, the ULP was already
    /// set, the kernel TLS module is absent, etc.). The extracted secrets were
    /// valid (handshake completed) but the kernel would not take them. Distinct
    /// from `HandshakeFailed` per `.claude/rules/development.md` § Errors — the
    /// failing layer is the kTLS install, not the handshake.
    #[error("kTLS arm refused by kernel: {source}")]
    KtlsArmFailed {
        #[source]
        source: std::io::Error,
    },

    /// The Earned-Trust `probe` sentinel round-trip failed — the kTLS-arm +
    /// agent-light forward-encrypt substrate (the `write_all`-into-kTLS-TX copy +
    /// the peer's kTLS-RX decrypt) did NOT round-trip clean on the loopback
    /// sentinel. The node MUST refuse to start
    /// (`health.startup.refused`); the proxy is not trustworthy. Mirrors
    /// `DataplaneError::LocalBackendProbe` / `ReverseLocalProbe`. `which` names
    /// the sentinel that failed so the refusal is diagnosable without
    /// `Display`-grepping.
    #[error("mTLS proxy probe round-trip failed [{which}]: {message}")]
    Probe { which: ProbeSentinel, message: String },

    /// Teardown could not fully reclaim a connection's resources — a leg close
    /// or pump stop errored. Surfaced (not swallowed)
    /// so a resource leak is observable; the equivalence harness asserts no leak
    /// on the `Ok` path.
    #[error("teardown of connection {id} failed: {source}")]
    TeardownFailed {
        id: EnforcedConnectionId,
        #[source]
        source: std::io::Error,
    },

    /// Underlying host I/O not covered by a more specific variant (leg-F
    /// `accept`/fd plumbing, `splice` pump setup). `#[from] std::io::Error`
    /// keeps `?` ergonomic at the host-adapter boundary, mirroring
    /// `DriverError::Io` / `DataplaneError::Io`. Specific, diagnosable failures
    /// get their own variant above; this is the genuine residual only.
    #[error("mTLS enforcement I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// The per-connection transparent-mTLS enforcement port (ADR-0069), bidirectional
/// (F3). `#[async_trait]` at the boundary (mirroring `Dataplane` / `Driver` —
/// async only where the contract genuinely awaits kernel I/O; the trait lives in
/// `overdrive-core` but `async_trait` is a declarative macro with no runtime, so
/// it stays off the `core` *I/O* surface exactly as `Dataplane` does today).
/// `Send + Sync + 'static` to be held as `Arc<dyn MtlsEnforcement>` and shared
/// across the worker's per-connection tasks.
#[async_trait]
pub trait MtlsEnforcement: Send + Sync + 'static {
    /// Earned-Trust probe (ADR-0069 § Enforcement; D-MTLS-11). Verify the proxy
    /// substrate honours its contract in the REAL environment BEFORE any
    /// connection is enforced. Composition-root invariant: wire → probe → use.
    ///
    /// # Preconditions
    /// None. Called once at node startup, after the adapter is constructed,
    /// before `enforce` is ever called.
    ///
    /// # Postconditions on `Ok(())`
    /// The substrate the proxy relies on has been exercised on a loopback
    /// sentinel and round-tripped clean: a sentinel rustls TLS 1.3 handshake on a
    /// loopback leg arms kTLS-TX/RX, the agent-light forward encrypt pump
    /// (`read → write_all` into the kTLS-TX leg) moves one sentinel record through
    /// it ENCRYPTED, and the sentinel peer's kTLS-RX reconstructs the exact
    /// sentinel plaintext via a single `tls_sw_splice_read` (`findings.md` A; the
    /// shipped `mtls/` code is the SSOT). The probe exercises the EXACT production
    /// forward primitive (the `write_all`-into-kTLS-TX copy, NOT a splice into TX
    /// — which loses records). After `Ok`, the proxy is declared usable; the node
    /// proceeds to serve.
    ///
    /// # Edge cases
    /// Any sentinel round-trip failure (kTLS arm refused, the forward encrypt pump
    /// produces cleartext or no bytes, the peer reconstructs the wrong plaintext)
    /// returns a typed `MtlsEnforcementError` and the node MUST refuse to start
    /// with a structured `health.startup.refused` event — it does NOT degrade to
    /// a cleartext path (fail-closed for confidentiality).
    ///
    /// # Observable invariants
    /// `probe` mutates no enforced connection (there are none yet) and leaks no
    /// sentinel state — the loopback legs are torn down before return regardless
    /// of outcome.
    async fn probe(&self) -> Result<()>;

    /// Bring `conn` to a steady-state-established mTLS session and return an
    /// opaque [`EnforcedConnection`] handle. Phases 1–2 of ADR-0069 + the
    /// steady-state install, as ONE atomic unit. **Dispatches on
    /// `conn.routed` (the `Direction`)** — outbound (workload = client) vs
    /// inbound (workload = server, F3).
    ///
    /// # Preconditions
    /// - `conn.leg` is an OWNED, ESTABLISHED socket the agent `accept()`ed for a
    ///   transparently-intercepted connection. OUTBOUND: leg F, the workload-facing
    ///   plaintext leg off the `cgroup_connect4`-rewrite intercept
    ///   (`findings-userspace-relay.md` Unknown 1). INBOUND: leg C, the
    ///   client-facing leg off the TPROXY/`IP_TRANSPARENT` intercept
    ///   (`findings-inbound-intercept.md` §1). The port takes ownership.
    /// - `conn.routed` matches the direction: `Outbound { peer }` carries the real
    ///   peer leg B dials; `Inbound { orig_dst }` carries the `getsockname`-
    ///   recovered original destination that selects the server SVID.
    /// - `conn.alloc` MAY be absent from the held set — see edge cases
    ///   (fail-closed). OUTBOUND: the client workload's alloc. INBOUND: the server
    ///   workload's alloc (selected by `orig_dst`). The caller does NOT pre-check
    ///   `svid_for`; `enforce` is the single fail-closed gate.
    ///
    /// # Postconditions on `Ok(EnforcedConnection)` — OUTBOUND
    /// After return, ALL of the following hold (the observable contract every
    /// adapter MUST satisfy — what the `mtls_enforcement_equivalence` harness and
    /// the Tier-3 wire tests check):
    /// - The pre-arm plaintext the workload wrote during the handshake window was
    ///   captured LOSSLESSLY and flushed to the peer as the first
    ///   `application_data` on leg B (no dropped pre-arm bytes; rec_seq starts at
    ///   0; `findings-userspace-relay.md` Unknown 2).
    /// - Leg B carries TLS 1.3 records (`0x17`) presenting `conn.alloc`'s held
    ///   SVID (read via `IdentityRead::svid_for(&conn.alloc)`); the peer was
    ///   **authenticated** against `IdentityRead::current_bundle()` (chains to
    ///   the trust bundle). Auth-session == data-session (the rustls handshake's
    ///   extracted secrets ARE the kTLS keys on leg B). NO cleartext appears on
    ///   the peer-facing wire (`tcpdump` oracle).
    /// - The forward steady state is AGENT-LIGHT (the agent does NO TLS crypto in
    ///   userspace), but it is a COPY, not a splice: the adapter's own task drives
    ///   a bounded `read(legF) → write_all(legB)` pump; leg B is kTLS-TX-armed, so
    ///   the kernel `tls_sw_sendmsg` encrypts each blocking `write` synchronously
    ///   (NOT the `MSG_DONTWAIT` sockmap-backlog path). The agent does ZERO
    ///   crypto, but it DOES copy each record's plaintext through a userspace
    ///   buffer and issues a `read`+`write` per record — **NOT zero-copy, NOT
    ///   agent-idle, NOT zero-syscall**. A `splice` INTO leg B's kTLS-TX is NOT
    ///   used: it consumes the bytes and reports success but does not reliably
    ///   emit the record (the same `MSG_DONTWAIT` loss class the abandoned sockmap
    ///   egress redirect suffered — `tls_sw_sendmsg` via the blocking `write_all`
    ///   is the proven primitive;
    ///   `docs/research/dataplane/sockmap-egress-redirect-into-ktls-tx-delivery-research.md`).
    ///   The asymmetry is load-bearing: the request-carrying OUTBOUND primary
    ///   (forward, liveness-observed) is a COPY pump, while the request-carrying
    ///   INBOUND primary (deliver) is a zero-copy SPLICE.
    /// - The return-splice pump is RUNNING (the adapter's own task drives a
    ///   zero-copy `splice(legB → pipe → legF)` out of a plain — NO psock —
    ///   kTLS-RX leg B; `tls_sw_splice_read` decrypts on splice-out; D-MTLS-5).
    ///   `liveness(&handle)` observes the FORWARD copy pump and reports `Running`.
    /// - **Leg B was dialed with the intercept-exemption bypass (F5).** The
    ///   agent's own outbound leg-B `connect()` is NOT re-intercepted by the
    ///   workload `cgroup_connect4` rewrite — via a narrowly-scoped `SO_MARK`
    ///   socket mark the program checks-and-skips, OR cgroup scoping (the program
    ///   attaches to the *workload* subtree, not the agent's — the existing
    ///   `cgroup_connect4_service` attach boundary). The bypass is agent-private:
    ///   a workload CANNOT replicate it to self-exempt from interception (proven
    ///   by the F5 Tier-3 obligations: leg B not re-intercepted AND workload
    ///   cannot self-exempt). Without this, the agent's dial would recurse
    ///   infinitely.
    ///
    /// # Postconditions on `Ok(EnforcedConnection)` — INBOUND (F3)
    /// After return, ALL of the following hold (grounded in
    /// `findings-inbound-intercept.md`; what the inbound Tier-3 tests check):
    /// - The original destination was recovered via `getsockname()` on leg C and
    ///   selected the server workload's `AllocationId` → its held SVID (§1).
    /// - Leg C carries TLS 1.3 records (`0x17`); the agent's rustls SERVER
    ///   handshake presented `conn.alloc`'s held server SVID (via
    ///   `IdentityRead::svid_for`) AND the client's presented SVID was
    ///   **REQUIRED + VERIFIED** to chain to `IdentityRead::current_bundle()` via
    ///   `WebPkiClientVerifier` (§2). Auth-session == data-session (the rustls
    ///   secrets ARE the kTLS-RX keys on leg C). NO cleartext of the request
    ///   appears on the client-facing wire (it carries `0x17` app_data; §3).
    /// - The server workload received the **byte-exact decrypted plaintext** on
    ///   leg S (the agent dialed the server workload's real plaintext socket and
    ///   spliced); the server workload holds NOTHING and is identity-unaware (§3).
    /// - The deliver pump (the request-carrying C→S direction, the INBOUND
    ///   primary) is RUNNING and is a genuine zero-copy SPLICE: the adapter's own
    ///   task drives `splice(legC → pipe → legS)` on a plain — NO psock — kTLS-RX
    ///   leg C; `tls_sw_splice_read` decrypts each record on splice-out (same
    ///   primitive as the outbound return). `liveness(&handle)` observes THIS pump
    ///   and reports `Running`.
    /// - The response pump (the S→C direction) is RUNNING and is a bounded
    ///   `read(legS) → write_all(legC)` COPY into leg C's kTLS-TX (the same
    ///   userspace-copy encrypt primitive as the outbound forward — the kernel
    ///   `tls_sw_sendmsg` encrypts each `write`; NOT a splice, NOT zero-copy). It
    ///   is auxiliary (torn down with the connection; not `liveness`-observed).
    /// - Server-config mechanics honoured: `NewSessionTicket` suppressed
    ///   (`send_tls13_tickets = 0`) and `peer_certificates()` read for the
    ///   fail-closed guard BEFORE `dangerous_extract_secrets` consumed the
    ///   connection (§ Mechanics #3/#6).
    ///
    /// # Postconditions — BOTH directions
    /// - **Authn, NOT authz; NO intended-peer pinning in v1 (F1/F5).** This
    ///   establishes the peer is *a valid cluster workload* (chains to the bundle),
    ///   NOT that the connection is *authorized* (allow/deny is #27's BPF-LSM
    ///   `socket_connect` hook fed by #38's `policy_verdicts`, a SEPARATE subsystem
    ///   the proxy MUST NOT duplicate) and NOT that the peer is the *intended*
    ///   destination. If `conn.expected_peer == Some(id)`, the authenticated peer's
    ///   SPIFFE-SAN is additionally matched against `id` (expected-destination
    ///   pinning); a mismatch is fail-closed (`PeerIdentityMismatch`). In **v1
    ///   `expected_peer` is `None`** (authn-only) — the expected-peer identity is
    ///   the #242 UPGRADE; this clause is a no-op until then. A
    ///   valid-but-unintended SVID is NOT rejected in v1.
    ///
    /// # Edge cases (all FAIL-CLOSED — no cleartext, connection refused) — both directions
    /// - `IdentityRead::svid_for(&conn.alloc) == None` ⇒ `Err(AbsentSvid)`; the
    ///   handshake is refused, `conn.leg` is closed, no bytes egress (OUTBOUND: no
    ///   client SVID; INBOUND: no server SVID for the selected `orig_dst`). (`None`
    ///   is the held-set fail-closed signal — `identity_read.rs` clause 3.)
    /// - `current_bundle() == None`, or the peer does not chain to it ⇒
    ///   `Err(PeerVerificationFailed)` / `Err(AbsentBundle)`; refused, leg closed.
    ///   INBOUND: this is the `nocert`/`wrongca` fail-closed path proven in
    ///   `findings-inbound-intercept.md` §4 — the client SVID is absent or does not
    ///   chain to the bundle; NO plaintext is spliced to the server workload.
    /// - `conn.expected_peer == Some(id)` and the authenticated peer's SPIFFE-SAN
    ///   does NOT match `id` (a wrong-but-valid peer — chains to the bundle but
    ///   is not the intended destination) ⇒ `Err(PeerIdentityMismatch)`; refused,
    ///   leg closed. **v1: unreachable while `expected_peer` is `None`** — the #242
    ///   UPGRADE (F1/F5). A valid-but-unintended SVID is NOT rejected in v1.
    /// - The peer/workload streamed more than `limits.max_prearm_bytes` of pre-arm
    ///   plaintext before kTLS armed ⇒ `Err(BufferLimitExceeded)`: the buffer is
    ///   dropped, the plaintext leg reset, no cleartext egresses (F4 / DoS guard).
    /// - The handshake-and-arm exceeded `limits.handshake_deadline` ⇒
    ///   `Err(HandshakeTimeout)`; refused, legs closed (F4 — leg B outbound / leg C
    ///   inbound).
    /// - The per-allocation in-flight ceiling `limits.max_inflight_per_alloc` is
    ///   already reached for `conn.alloc` ⇒ `Err(InFlightLimitExceeded)`: the new
    ///   intercept is refused, no cleartext (F4).
    /// - The rustls handshake aborts (wrong SVID, alert, timeout) ⇒
    ///   `Err(HandshakeFailed)`; refused, legs closed.
    /// - The kTLS arm refuses on the kTLS leg (`TCP_ULP`/`TLS_TX`/`TLS_RX`
    ///   rejected, kernel TLS module absent) ⇒ `Err(KtlsArmFailed)`; refused,
    ///   legs closed.
    /// On ANY error, the port owns the cleanup: every owned leg is closed (OUTBOUND:
    /// leg F + any opened leg B; INBOUND: leg C + any opened leg S), no pump or
    /// kTLS state leaks, and NO cleartext byte reached the wire
    /// (OUTBOUND: the peer wire; INBOUND: the server workload's leg S — nothing is
    /// spliced) — the confidentiality invariant the whole feature rests on.
    ///
    /// # Observable invariants
    /// `enforce` is NOT idempotent and NOT replayable — each call enforces ONE
    /// distinct connection (a fresh leg F). The returned `EnforcedConnection.id`
    /// is unique per call within a node session.
    async fn enforce(&self, conn: InterceptedConnection) -> Result<EnforcedConnection>;

    /// The current liveness of the agent-light pump for `handle` — the
    /// request-carrying primary direction. The two primaries are NOT the same
    /// primitive (the agent-light asymmetry):
    /// - **OUTBOUND**: the FORWARD pump (`legF → legB`), a bounded
    ///   `read → write_all` COPY into leg B's kTLS-TX (per-record `read`+`write`,
    ///   NOT a splice, NOT zero-copy — `splice` into kTLS-TX loses records).
    /// - **INBOUND**: the DELIVER pump (`splice(legC → legS)`), a zero-copy
    ///   `splice` out of leg C's kTLS-RX, ~1 `splice` per TLS record
    ///   (`findings-inbound-intercept.md` §5 / `findings-splice-return.md`).
    ///
    /// Both are agent-light (the agent does no TLS crypto — the kernel kTLS engine
    /// does). The opposite-direction pump (the OUTBOUND return `splice(legB → legF)`
    /// zero-copy decrypt / the INBOUND response `legS → legC` write_all copy) is
    /// torn down with the connection but not observed here. `liveness` observes
    /// the primary pump's shared bytes-moved progress counter; it does not drive
    /// the pump (the adapter's own task does — SD-2). The progress counter is the
    /// same shape for either primitive (bytes moved to the destination per
    /// `splice_out` / `write_all`), so the `Stalled` derivation below is identical
    /// across the copy and splice pumps.
    ///
    /// # Preconditions
    /// `handle` was returned by a prior `enforce` on THIS adapter and not yet
    /// `teardown`'d. A handle for an unknown/torn-down connection reports
    /// `Gone` (NOT an error — the post-teardown observable, mirroring
    /// `Driver::status` returning `NotFound` after `stop`).
    ///
    /// # Postconditions
    /// Returns `Running` while the pump is draining records OR is idle-but-ready
    /// (no record pending); `Stalled { since }` when the pump's bytes-spliced
    /// progress metric has NOT advanced for `MtlsLimits::pump_stall_deadline`
    /// (F7 default 30 s) WHILE a record is pending on the kTLS-RX leg (a
    /// crashed/stranded pump — the reliability sensitivity point ADR-0069 § ATAM
    /// names); or `Gone` after teardown / leg close. A purely-idle connection
    /// (no pending record) is `Running`, never `Stalled` (no false positives on
    /// quiescent long-lived connections).
    ///
    /// # F6 supervision shape (C)+(B) — who reacts, and to what (ADR-0070 / D-MTLS-16)
    /// v1 supervision is **(C)** kernel `TCP_USER_TIMEOUT`/keepalive on the agent's
    /// legs (the kernel reaps the transport-death class — peer gone, half-open,
    /// unacked past the deadline) **+ (B)** the per-connection pump task
    /// self-tearing-down fail-closed on its OWN terminal exit (EOF / error /
    /// `ETIMEDOUT`): **teardown + fail-closed reset** (close the legs, stop the pumps,
    /// reclaim the kTLS state). There is **no central point-query, no `supervise_tick`,
    /// no tick cadence** in v1 — the retired central `MtlsSupervisor` (shape (A)) is
    /// deleted (ADR-0070 supersedes the SD-4 point-query). The connection self-tears
    /// (a foreign process cannot resume a kTLS record sequence; no reconnect-in-place,
    /// no degrade to a userspace copy loop); request-retry protocols re-handshake on
    /// reconnect. Telemetry `mtls.pump.stalled` + `mtls.pump.teardown_on_stall` is
    /// emitted per connection on the (B) self-teardown path.
    ///
    /// `liveness` itself is **not driven by a tick** in v1. It is the **SD-2 observe
    /// surface** the equivalence harness re-queries for the post-teardown `Gone`
    /// no-leak assertion, and `Stalled` (derived from `pump_stall_deadline`) is the
    /// **reserved predicate** for the deferred kernel-invisible progress-stall watchdog
    /// (a per-connection watchdog, [#232]; NOT a central loop). The
    /// SD-4 point-query-vs-stream sub-decision is moot for v1 liveness (neither runs).
    ///
    /// # Observable invariants
    /// Read-only: `liveness` never mutates the pump or the connection. It is an
    /// observation surface (analogous to `Driver`'s exit-event observation), queried
    /// by an external observer / the reserved [#232] watchdog —
    /// not by a central worker tick in v1.
    ///
    /// [#232]: https://github.com/overdrive-sh/overdrive/issues/232
    fn liveness(&self, handle: &EnforcedConnection) -> PumpLiveness;

    /// Tear `handle` down: stop BOTH pumps (outbound: forward `legF → legB`
    /// write_all copy + return `splice(legB → legF)`; inbound: deliver
    /// `splice(legC → legS)` + response `legS → legC` write_all copy), drop the
    /// kTLS state, and close both legs (outbound: leg F + leg B; inbound: leg C +
    /// leg S). Phase 4 of ADR-0069.
    /// This is also the F6 fail-closed reclaim the (B) per-connection self-teardown
    /// runs (ADR-0070 / D-MTLS-16) when a pump hits a terminal exit (EOF / error /
    /// the (C) kernel-reaped `ETIMEDOUT`) — the same reclaim, triggered by the
    /// connection's own task rather than a central worker query.
    ///
    /// # Preconditions
    /// `handle` was returned by a prior `enforce`. Idempotent: tearing down an
    /// already-torn-down (or unknown) handle is `Ok(())`, NOT an error — mirrors
    /// `Driver::stop` / `deregister_local_backend` idempotency.
    ///
    /// # Postconditions on `Ok(())`
    /// Both legs are closed; both pump tasks have stopped; no
    /// kTLS state for this connection remains; `liveness(&handle)` returns
    /// `Gone`. The workload's connection is closed (the proxy owned both legs;
    /// no restart-survival in v1 — D-MTLS-2 / ADR-0069 Negative).
    ///
    /// # Observable invariants
    /// After `teardown`, no further bytes move for this connection in either
    /// direction; the per-connection resources are fully reclaimed (no fd/pump
    /// leak), which the equivalence harness asserts by re-querying `liveness`.
    async fn teardown(&self, handle: EnforcedConnection) -> Result<()>;
}

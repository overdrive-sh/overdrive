//! [`MtlsResolve`] — the per-connection enrollment-resolve driven port
//! (ADR-0071, the #242 anti-corruption boundary).
//!
//! Path A's transparent-mTLS enrollment model replaces the retired
//! per-destination redirect map (`MTLS_REDIRECT_DEST` /
//! `MtlsDataplane::{attach_alloc,program_redirect}` /
//! `program_declared_peer_redirect`) with a per-connection RESOLVE: on each
//! captured connection the agent recovers `orig_dst` (via `getsockname` on the
//! TPROXY-intercepted leg-F socket) and resolves it against the mesh's
//! `running` backend set into an [`MtlsResolution`] — a **3-variant sum type**,
//! NOT a binary `Option`. A binary `Option<ResolvedBackend>` cannot distinguish
//! "the dialed dst is genuinely not a mesh peer → pass-through in cleartext, by
//! design" (`NonMesh`) from "it should be a mesh peer but cannot be
//! resolved/reached/validated → fail-closed, NO cleartext" (`MeshUnreachable`);
//! collapsing both into `None` re-introduces the exact silent-cleartext
//! ambiguity the enrollment model exists to remove (CLAUDE.md § "Type-driven
//! design — sum types over sentinels / make invalid states unrepresentable").
//!
//! This port is the **#242 anti-corruption boundary**. THIS feature owns: the
//! port trait + the 3-variant [`MtlsResolution`] + the 2-field
//! [`ResolvedBackend`], a v1 host adapter (`ServiceBackendsResolve`) reading
//! `service_backends` via [`ObservationStore`](crate::traits::ObservationStore),
//! a sim adapter, and the fail-closed semantic + the Earned-Trust [`probe`]. The
//! expected-SVID join (`service_backends` × identity facts), the multi-backend
//! candidate-set + LB-pick policy, and the SAN-match wiring of `expected_peer`
//! are **#242** (so v1 returns `expected_svid = None`, authn-only, consistent
//! with ADR-0069). The VIP/DNS name → virt resolution upstream of `orig_dst`
//! (the responder daemon) is **#243**.
//!
//! Per `.claude/rules/development.md` § "Trait definitions specify behavior, not
//! just signature": the per-arm enforce/pass-through/fail-closed semantics
//! reproduced VERBATIM on [`MtlsResolution`] below ARE the port's rustdoc
//! contract — the `mtls_resolve_equivalence` DST harness (DELIVER) drives both
//! the host and sim adapters through the same call sequence and asserts every
//! arm.
//!
//! Production wires `ServiceBackendsResolve` (reads `service_backends` via
//! [`ObservationStore`](crate::traits::ObservationStore)); simulation wires
//! `SimMtlsResolve` (scriptable `orig_dst → MtlsResolution`). Pure trait +
//! `#[async_trait]` (a declarative macro, no runtime — off the `core` I/O
//! surface exactly as [`MtlsEnforcement`](crate::traits::MtlsEnforcement) and
//! `Dataplane`).
//!
//! [`probe`]: MtlsResolve::probe

use std::net::SocketAddrV4;

use async_trait::async_trait;
use thiserror::Error;

use crate::SpiffeId;

/// Result alias used throughout the crate's mTLS-resolve surface.
pub type Result<T, E = MtlsResolveError> = std::result::Result<T, E>;

/// Outcome of resolving a captured connection's original destination
/// (`orig_dst`, recovered via `getsockname` on the TPROXY-intercepted leg-F
/// socket) against the mesh's `running` backend set.
///
/// THREE arms, each with a DISTINCT enforce/pass-through/fail-closed semantic.
/// The worker's decision rule is pinned by the variant — a crafter MUST NOT
/// infer it from a sentinel. (Sharpens Q3 "fail-closed, not silent-cleartext"
/// into the type.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MtlsResolution {
    /// **mesh → ENFORCE.** `orig_dst` mapped to a `running` mesh backend.
    /// The worker sets `Routed::Outbound { peer: backend.addr }` (+
    /// `expected_peer` when #242's join supplies it) and calls `enforce`
    /// (mTLS to that backend). The only arm that drives a handshake.
    Mesh(ResolvedBackend),

    /// **non-mesh → PASS-THROUGH (cleartext, by design).** The dialed dst is
    /// genuinely NOT a mesh peer (no `running` mesh backend for `orig_dst`).
    /// Egress proceeds in cleartext — this is the classification arm, NOT an
    /// error and NOT a fail-closed. (e.g. a workload dialing an external
    /// address, or a non-meshed local port.)
    NonMesh,

    /// **unreachable-or-invalid mesh → FAIL-CLOSED (NO cleartext).** `orig_dst`
    /// SHOULD be a mesh peer but cannot be resolved/reached, or its identity
    /// cannot be validated. The worker REFUSES the connection — it does NOT
    /// fall back to cleartext. This is the footgun the enrollment model exists
    /// to remove: a should-be-mesh peer is never silently leaked in the clear.
    MeshUnreachable,
}

/// A single `running` mesh backend the captured connection resolves to.
/// Bounded to EXACTLY two fields — no more. Multi-backend candidate sets +
/// LB-pick are #242's concern, not this struct's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBackend {
    /// The concrete `running` backend address (v1 headless: the same
    /// `service_backends` addr DNS returned — one source, two readers).
    pub addr: SocketAddrV4,
    /// The peer's expected SPIFFE identity for SAN-pinning. **v1 = `None`**
    /// for every backend: the v1 `ServiceBackendsResolve` adapter is
    /// authn-only (chain-to-bundle) and does NOT join identity facts. The
    /// expected-SVID join is **#242**; filling it in here = boundary
    /// divergence across the anti-corruption boundary. The field exists so the
    /// SAN-pin wires the moment #242 supplies the join (Q4 / D-TME-8).
    pub expected_svid: Option<SpiffeId>,
}

/// Cause-distinct failure modes for the mTLS-resolve surface (no catch-all
/// `Internal(String)` per `.claude/rules/development.md` § Errors). Each variant
/// names a distinct remediation; the `#[error("...")]` strings are the
/// operator's diagnostic surface.
///
/// NOTE the asymmetry with [`MtlsResolution`]: a store-read failure AT RESOLVE
/// TIME, or a present-but-unreachable mesh backend, classifies INTO
/// [`MtlsResolution::MeshUnreachable`] (a fail-closed outcome the worker acts
/// on per-connection), NOT into an `Err` — `resolve` returns `Err` only for
/// the conditions below, which are NOT a per-connection classification.
#[derive(Debug, Error)]
pub enum MtlsResolveError {
    /// The `service_backends` observation surface could not be read at resolve
    /// time for a reason that is NOT a per-connection classification (the store
    /// handle is poisoned, the backing table is corrupt, an underlying
    /// subscription errored). Distinct from the per-connection
    /// [`MtlsResolution::MeshUnreachable`] outcome — this is a store-layer
    /// fault, not "this particular `orig_dst` should-be-mesh-but-can't." The
    /// adapter surfaces the underlying cause so the operator's remediation is
    /// store-specific, not a generic resolve error.
    #[error("service_backends store unreadable at resolve time: {reason}")]
    StoreUnreadable {
        /// The store-layer cause, named so the remediation is specific.
        reason: String,
    },

    /// The Earned-Trust [`probe`](MtlsResolve::probe) could not demonstrate the
    /// adapter can read the `service_backends` surface at node startup. The
    /// node MUST refuse to start with a structured `health.startup.refused`
    /// event — it does NOT proceed with a resolve adapter that silently returns
    /// empty (which would degrade to silent pass-through, the exact
    /// silent-cleartext footgun the enrollment model exists to remove). `reason`
    /// names the probe-time fault so the refusal is diagnosable without
    /// `Display`-grepping.
    #[error("mTLS-resolve probe failed; refusing to start (fail-closed): {reason}")]
    Probe {
        /// The probe-time cause that triggered the startup refusal.
        reason: String,
    },
}

/// The per-connection enrollment-resolve driven port (ADR-0071), the #242
/// anti-corruption boundary. `#[async_trait]` at the boundary (mirroring
/// [`MtlsEnforcement`](crate::traits::MtlsEnforcement) / `Dataplane` — async
/// only where the contract genuinely awaits store I/O; the trait lives in
/// `overdrive-core` but `async_trait` is a declarative macro with no runtime,
/// so it stays off the `core` *I/O* surface). `Send + Sync + 'static` to be
/// held as `Arc<dyn MtlsResolve>` and shared across the worker's per-connection
/// tasks.
#[async_trait]
pub trait MtlsResolve: Send + Sync + 'static {
    /// Earned-Trust probe (principle 12; ADR-0071 § "MtlsResolve port
    /// contract"). Verify the resolve adapter can read its backing
    /// `service_backends` surface in the REAL environment BEFORE any connection
    /// is resolved. Composition-root invariant: wire → probe → use.
    ///
    /// # Preconditions
    /// None. Called once at node startup, after the adapter is constructed,
    /// before [`resolve`](Self::resolve) is ever called.
    ///
    /// # Postconditions on `Ok(())`
    /// The adapter has demonstrated it can read the `ObservationStore`
    /// `service_backends` surface. After `Ok`, the resolve port is declared
    /// usable; the node proceeds to serve.
    ///
    /// # Edge cases
    /// An unreadable store at probe time returns [`MtlsResolveError::Probe`]
    /// and the node MUST refuse to start with a structured
    /// `health.startup.refused` event — it does NOT proceed and silently return
    /// an empty resolve (an empty resolve degrading to silent pass-through IS
    /// the silent-cleartext footgun the enrollment model exists to remove).
    ///
    /// # Observable invariants
    /// `probe` mutates no resolve state and leaks no probe-time state — it is a
    /// read-only liveness check on the backing store surface.
    async fn probe(&self) -> Result<()>;

    /// Resolve a captured connection's original destination `orig_dst` against
    /// the mesh's `running` backend set into an [`MtlsResolution`]. This is the
    /// per-connection enrollment-resolve the worker drives on every captured
    /// connection (the enrollment model's replacement for the retired
    /// per-destination redirect map).
    ///
    /// # Preconditions
    /// - `orig_dst` is the original destination recovered via `getsockname` on
    ///   the TPROXY-intercepted leg-F socket (v1 headless: the same
    ///   `service_backends` addr DNS returned — one source, two readers, so
    ///   `orig_dst` is byte-consistent with what the resolve port reads).
    /// - The adapter's backing `service_backends` surface is readable (the
    ///   Earned-Trust [`probe`](Self::probe) passed at startup).
    ///
    /// # Postconditions on `Ok(MtlsResolution)`
    /// The return classifies `orig_dst` into EXACTLY ONE of three arms, each
    /// with a DISTINCT enforce/pass-through/fail-closed semantic the worker acts
    /// on (the per-arm contract reproduced verbatim on [`MtlsResolution`]; the
    /// DST equivalence test exercises every arm):
    /// - **[`Mesh(ResolvedBackend)`](MtlsResolution::Mesh) → ENFORCE** — a
    ///   `running` mesh backend resolved. The worker sets `Routed::Outbound {
    ///   peer: backend.addr }` (+ `expected_peer` when #242 supplies it) and
    ///   calls `enforce` (mTLS to that backend). The only arm that drives a
    ///   handshake.
    /// - **[`NonMesh`](MtlsResolution::NonMesh) → PASS-THROUGH (cleartext, by
    ///   design)** — `orig_dst` is genuinely NOT a mesh peer (no `running` mesh
    ///   backend for it); egress proceeds in cleartext. The classification arm —
    ///   NOT an error, NOT a fail-closed.
    /// - **[`MeshUnreachable`](MtlsResolution::MeshUnreachable) → FAIL-CLOSED
    ///   (NO cleartext)** — `orig_dst` SHOULD be a mesh peer but cannot be
    ///   resolved/reached, or its identity cannot be validated. The worker
    ///   REFUSES the connection — it does NOT fall back to cleartext. This is
    ///   the silent-cleartext footgun the enrollment model exists to remove.
    ///
    /// The boundary between [`NonMesh`](MtlsResolution::NonMesh) ("not a mesh
    /// peer") and [`MeshUnreachable`](MtlsResolution::MeshUnreachable)
    /// ("should-be-mesh-but-can't") lives in the adapter, classified INTO the
    /// type, never inferred by the worker. v1's `ServiceBackendsResolve`
    /// distinguishes them: a `service_backends` lookup finding no `running` mesh
    /// backend for `orig_dst` is [`NonMesh`](MtlsResolution::NonMesh); a
    /// present-but-unreachable mesh backend (or absent required identity facts)
    /// is [`MeshUnreachable`](MtlsResolution::MeshUnreachable).
    ///
    /// # Edge cases
    /// - **v1 `expected_svid` is always `None`** in every
    ///   [`Mesh`](MtlsResolution::Mesh) arm: the v1 `ServiceBackendsResolve`
    ///   adapter is authn-only (chain-to-bundle) and does NOT join identity
    ///   facts. The expected-SVID join is #242 (filling it here is boundary
    ///   divergence across the anti-corruption boundary).
    /// - A store-read failure AT RESOLVE TIME that is NOT a per-connection
    ///   classification (poisoned handle, corrupt table) returns an `Err` of
    ///   [`MtlsResolveError::StoreUnreadable`] — it is NOT classified into
    ///   [`MeshUnreachable`](MtlsResolution::MeshUnreachable) (which is the
    ///   per-connection "should-be-mesh peer present but unreachable" outcome
    ///   the worker acts on per-connection).
    ///
    /// # Observable invariants
    /// `resolve` is read-only: it mutates no store state and is idempotent in
    /// observable state — two consecutive calls for the same `orig_dst` against
    /// an unchanged `running` backend set produce the same classification.
    async fn resolve(&self, orig_dst: SocketAddrV4) -> Result<MtlsResolution>;
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    /// TEST-LOCAL classification input — the three mutually-exclusive
    /// store-read outcomes the v1 `ServiceBackendsResolve` adapter observes for
    /// an `orig_dst` lookup. This is a private test model of the adapter's
    /// classification surface; it is NOT public API (the classification LOGIC
    /// lives in the 01-03 host adapter, exercised by the DST equivalence test).
    /// It exists only to drive the totality property over [`MtlsResolution`].
    #[derive(Debug, Clone, Copy)]
    enum ClassificationInput {
        /// A `running` mesh backend was found for `orig_dst`.
        AddrPresent,
        /// No `running` mesh backend was found for `orig_dst`.
        AddrAbsent,
        /// The store read for `orig_dst` failed in a per-connection
        /// should-be-mesh-but-can't way.
        StoreFail,
    }

    /// TEST-LOCAL model of the adapter's classification mapping. Maps each of
    /// the three store-read outcomes to EXACTLY ONE [`MtlsResolution`] arm —
    /// the totality the property below asserts. Private; never public surface.
    fn classify(input: ClassificationInput) -> MtlsResolution {
        match input {
            ClassificationInput::AddrPresent => MtlsResolution::Mesh(ResolvedBackend {
                addr: SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 8080),
                expected_svid: None,
            }),
            ClassificationInput::AddrAbsent => MtlsResolution::NonMesh,
            ClassificationInput::StoreFail => MtlsResolution::MeshUnreachable,
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]

        /// Classification is TOTAL: every (addr-present, addr-absent,
        /// store-fail) input maps to EXACTLY ONE of the three [`MtlsResolution`]
        /// arms — no panic, no fourth state, and the three inputs cover the
        /// three arms exhaustively (a structural defense against a fourth
        /// variant or a collapsed `Option`-shaped binary classification).
        ///
        /// Universe (observable): the [`MtlsResolution`] arm returned by the
        /// test-local `classify` model. The property is that the arm is a
        /// member of the closed 3-arm set for every input, and that the three
        /// distinct inputs map onto the three distinct arms.
        #[test]
        fn mtls_resolution_classification_is_total(selector in 0u8..3) {
            use proptest::prelude::*;

            let input = match selector {
                0 => ClassificationInput::AddrPresent,
                1 => ClassificationInput::AddrAbsent,
                _ => ClassificationInput::StoreFail,
            };

            // No panic, and the result is one of EXACTLY three arms — the
            // exhaustive match is itself the totality proof: adding a fourth
            // variant would fail to compile here.
            let resolution = classify(input);
            let arm_index = match resolution {
                MtlsResolution::Mesh(_) => 0u8,
                MtlsResolution::NonMesh => 1u8,
                MtlsResolution::MeshUnreachable => 2u8,
            };

            // Each distinct input selects its own distinct arm — the mapping is
            // a bijection over the three-element domain (no two inputs collapse
            // onto one arm, which is exactly what a binary `Option` return
            // could not preserve).
            prop_assert_eq!(arm_index, selector);
        }
    }
}

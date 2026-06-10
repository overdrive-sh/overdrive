//! CA issuance + audit binding — every workload SVID issuance writes an
//! `issued_certificates` audit row, bound so an audit-write failure refuses the
//! issuance (built-in-ca / GH #28, ADR-0063 D6).
//!
//! This is the focused issuance seam the workload-start path calls to mint a
//! workload SVID. It composes the [`Ca`] driving port (issue the leaf + draw the
//! issuer serial from the node intermediate) with the [`ObservationStore`]
//! driven port (record the issuance fact as a first-class
//! [`ObservationRow::IssuedCertificate`] row), and **binds the two**: the leaf
//! and its audit row are observable TOGETHER. If the audit row cannot be
//! written, the issuance fails — NO unaudited certificate ever escapes (KPI/AC
//! US-CA-05; ADR-0063 D6 "issuance is never silent").
//!
//! # State-layer hygiene (whitepaper §4, ADR-0063 D2/D6)
//!
//! The CA *material* (root key, intermediate keys) is **intent** (linearizable,
//! the [`crate::ca_boot`] path). The *record of what was issued* — the
//! `issued_certificates` row — is **observation** (gossiped when #36 lands;
//! single-node = local). This module writes ONLY the observation row, through
//! the `ObservationStore` port exactly like `alloc_status` / `node_health`; it
//! never touches the intent store. The `ObservationStore` IS the observation
//! boundary — there is no parallel audit table or inherent-method bypass
//! (ADR-0063 D6 "mirroring AllocStatusRow/NodeHealthRow").
//!
//! # Persist inputs, not derived state
//!
//! Per `.claude/rules/development.md` § "Persist inputs, not derived state",
//! the [`IssuedCertificateRow`] this module builds carries the audit *inputs*
//! observed at issuance time — `serial` / `spiffe_id` (from the faithful
//! [`SvidMaterial`] accessors), `issuer_serial` (from the node
//! [`IntermediateHandle`]), the validity window, `node_id`, and `issued_at`
//! (the [`Clock`] observation). No derived classification is persisted.
//!
//! # Re-issue without restart (US-CA-05 / S-05-05)
//!
//! [`issue_and_audit`] is an on-demand call on the RUNNING control plane —
//! calling it twice for the same [`SpiffeId`] mints a FRESH leaf each time
//! (distinct serial, new validity window) and writes a fresh audit row, with NO
//! restart. This is the mechanism the #40 rotation workflow will later drive on
//! a schedule; this module provides only the mechanism, not the trigger.

use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
use overdrive_core::ca::{SKEW_TOLERANCE, WORKLOAD_SVID_TTL};
use overdrive_core::id::IssuanceOrdinal;
use overdrive_core::traits::ca::{Ca, CaError, SvidMaterial, SvidRequest};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::{
    ObservationRow, ObservationStore, ObservationStoreError,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{NodeId, SpiffeId};

// The validity window is computed ONCE here from the injected [`Clock`] and the
// SAME two `overdrive_core::ca` constants the leaf is signed with —
// [`WORKLOAD_SVID_TTL`] (the validity width) and [`SKEW_TOLERANCE`] (the
// `not_before` back-off) — see imports above. There is no separate "audit TTL"
// constant.
//
// Under the ADR-0063 rev 2 amendment this window is the SINGLE source of truth:
// the SAME two `UnixInstant` values are threaded into the leaf (via the windowed
// [`SvidRequest`] passed to [`Ca::issue_svid`]) AND recorded on the
// `issued_certificates` audit row. So `svid.not_after() == row.not_after` by
// construction — not by two independent reconstructions that could drift
// (ADR-0067 rev 3 D8). The host adapter STAMPS this window onto the cert and
// reads no wall-clock of its own; the single clock read is
// `UnixInstant::from_clock(clock)` below, which is DST-controllable under
// `SimClock`. `not_after` is an OBSERVED FACT of the minted credential (window
// fixed at mint), not a recompute-from-policy deadline — it is correctly NOT
// reconstructed-on-read.

/// A CA-issuance failure — the issuance did NOT complete, and (critically) NO
/// unaudited certificate was handed out.
///
/// Distinct per failure mode (`.claude/rules/development.md` § "Distinct
/// failure modes get distinct error variants"): a signing failure from the
/// [`Ca`] port, versus an audit-write failure where the leaf was minted but its
/// audit row could not be persisted. In BOTH cases [`issue_and_audit`] returns
/// `Err` and the caller receives no [`SvidMaterial`] — the cert and its audit
/// row are observable together or not at all (ADR-0063 D6).
#[derive(Debug, thiserror::Error)]
pub enum CaIssuanceError {
    /// The [`Ca`] port failed to mint the leaf or its node intermediate. The
    /// typed [`CaError`] passes through so the caller keeps the structured
    /// signing/policy cause.
    #[error("certificate issuance failed: {source}")]
    Ca {
        /// The underlying CA failure.
        #[source]
        source: CaError,
    },

    /// The leaf was minted but its `issued_certificates` audit row could NOT be
    /// written. Per ADR-0063 D6 this refuses the issuance — the cert is dropped
    /// and this error surfaces rather than handing out an UNAUDITED certificate
    /// (no silent issuance). The structured [`ObservationStoreError`] carries
    /// the underlying store cause.
    #[error(
        "issuance audit row could not be written; refusing to issue an unaudited certificate: {source}"
    )]
    Audit {
        /// The underlying audit-store failure.
        #[source]
        source: ObservationStoreError,
    },
}

impl CaIssuanceError {
    fn ca(source: CaError) -> Self {
        Self::Ca { source }
    }

    fn audit(source: ObservationStoreError) -> Self {
        Self::Audit { source }
    }
}

/// Issue a workload SVID and record its `issued_certificates` audit row, bound
/// so an audit-write failure refuses the issuance.
///
/// # Behaviour (ADR-0063 D6, US-CA-05)
///
/// 1. Mint the node intermediate (`ca.issue_intermediate(node)`) to obtain the
///    `issuer_serial` — the chain link recorded on the audit row. Single-node
///    (Phase 2.6): one node → one intermediate, idempotently cached by the
///    adapter, so this does not re-mint on re-issue.
/// 2. Compute the validity window ONCE from the injected [`Clock`]
///    (`not_before = now − SKEW_TOLERANCE`, `not_after = not_before +
///    WORKLOAD_SVID_TTL`), then mint the leaf with a windowed
///    [`SvidRequest`] carrying those values — a fresh certificate each call
///    (distinct serial, new validity), the re-issue mechanism (S-05-05). The
///    clock is the single window SSOT: the workload identity is the
///    caller-supplied [`SpiffeId`] (ADR-0067 rev 3 spec item 5 / option b —
///    an un-windowed `SvidRequest` can't be built now that the window is
///    REQUIRED on the type, so the seam takes the identity directly and
///    builds its own windowed request internally).
/// 3. Build the [`IssuedCertificateRow`] from the FAITHFUL observed facts —
///    `serial` / `spiffe_id` from [`SvidMaterial`]'s per-call accessors,
///    `issuer_serial` from the [`IntermediateHandle`](overdrive_core::traits::ca::IntermediateHandle),
///    `issued_at` from the injected [`Clock`], and the SAME `not_before` /
///    `not_after` window threaded into the leaf — so `svid.not_after() ==
///    row.not_after` by construction (ADR-0063 rev 2 amendment).
/// 4. **Write the audit row through the [`ObservationStore`] port, then hand
///    back the leaf.** The row is written as a first-class
///    [`ObservationRow::IssuedCertificate`] via [`ObservationStore::write`] —
///    the SAME plumbing as `alloc_status` / `node_health` (ADR-0063 D6), so the
///    audit path is DST-testable through `SimObservationStore`. If the write
///    fails, return [`CaIssuanceError::Audit`] and DROP the leaf — no unaudited
///    cert escapes (no silent issuance).
///
/// # Preconditions
///
/// Two load-bearing invariants hold today and MUST be preserved — they make the
/// monotonic [`IssuanceOrdinal`] stamped on the audit row correct (feature-delta
/// § D1-AMEND-2):
///
/// 1. **Single-writer / serialized issuance.** The ordinal is derived as the
///    count of already-persisted `issued_certificates` rows read immediately
///    before the audit write. That read-then-write is a check-then-act shape
///    (`.claude/rules/development.md` § "Check-and-act must be atomic") and is
///    race-free ONLY because issuance is serialized through the single
///    action-shim executor (sequential per-action dispatch). `issue_and_audit`
///    MUST NOT be called concurrently for two issuances against the SAME
///    `observation` store — a concurrent caller would read the same count twice
///    and stamp duplicate ordinals (a TOCTOU).
/// 2. **Append-only audit log.** The `len()`-derived ordinal is strictly
///    monotonic ONLY because `ObservationStore::issued_certificate_rows` is
///    never deleted, overwritten, or compacted (exactly one row per distinct
///    serial). A future delete/GC path — e.g. Phase-5 revocation pruning
///    revoked certs — breaks ordinal uniqueness (a delete makes the next
///    `len()` smaller than a prior ordinal) and MUST re-source the ordinal then
///    (a persisted monotonic counter a delete cannot rewind, or equivalent).
///
/// # Errors
///
/// * [`CaIssuanceError::Ca`] — the leaf or intermediate could not be signed.
/// * [`CaIssuanceError::Audit`] — the leaf was minted but its audit row could
///   not be written (or the pre-write issuance-ordinal count read failed); the
///   issuance is refused and the cert dropped.
pub async fn issue_and_audit(
    ca: &dyn Ca,
    observation: &dyn ObservationStore,
    clock: &dyn Clock,
    node: &NodeId,
    spiffe_id: &SpiffeId,
) -> Result<SvidMaterial, CaIssuanceError> {
    // The node intermediate is the issuer of the leaf; its serial is the audit
    // row's `issuer_serial` (the chain link an auditor walks). Single-node: the
    // HOST adapter (`RcgenCa`) idempotently caches the intermediate, so re-issue
    // does not re-mint it. This is an adapter implementation detail, NOT a trait
    // guarantee — `Ca::issue_intermediate` does not promise caching, and `SimCa`
    // returns a fixture intermediate on every call (its serial is re-drawn from
    // the seeded `Entropy` port per the determinism contract).
    let intermediate = ca.issue_intermediate(node).map_err(CaIssuanceError::ca)?;

    // Global monotonic issuance ordinal — the issuance-order rank, read from the
    // durable audit log itself (the count of rows already persisted). Strictly
    // increasing across issuances and across restart (the count includes every
    // pre-restart row; the table is append-only — see § D1-AMEND-2 precondition),
    // DST-deterministic (a read on the same port the audit write uses; issuance is
    // serialized through the single action-shim executor, so the read-then-write is
    // race-free). This is the selection key the consumer-side "current cert"
    // projection maxes over — recency-correct even when `issued_at` ties under a
    // fixed SimClock. See feature-delta § D1-AMEND-2.
    let ordinal = IssuanceOrdinal::new(
        observation.issued_certificate_rows().await.map_err(CaIssuanceError::audit)?.len() as u64,
    );

    // Compute the validity window ONCE from the injected clock, BEFORE minting
    // (ADR-0063 rev 2 amendment / ADR-0067 rev 3 D8). `issued_at` is the clock
    // snapshot; `not_before` is backed off by `SKEW_TOLERANCE` so the freshly-
    // minted leaf verifies under a verifier whose clock is marginally behind;
    // `not_after = not_before + WORKLOAD_SVID_TTL` keeps the window width exactly
    // the leaf TTL. `saturating_sub` keeps the arithmetic panic-free; production
    // `issued_at` is a real Unix time far above the back-off, so the floor never
    // bites. This is the SINGLE window SSOT: the SAME two `UnixInstant` values
    // are threaded into the leaf (via the windowed `SvidRequest`) and recorded on
    // the audit row — `svid.not_after() == row.not_after` by construction, no
    // second clock read, DST-deterministic under `SimClock`.
    let issued_at = UnixInstant::from_clock(clock);
    let not_before = UnixInstant::from_unix_duration(
        issued_at.as_unix_duration().saturating_sub(SKEW_TOLERANCE),
    );
    let not_after = not_before + WORKLOAD_SVID_TTL;

    // Mint the leaf — a FRESH certificate each call (distinct serial, new
    // validity window). This is the re-issue mechanism (S-05-05): calling
    // `issue_and_audit` again for the same `SpiffeId` produces a distinct leaf.
    // The clock is the single window SSOT, so we build the windowed request
    // from the caller-supplied `spiffe_id` + the window just computed — the
    // executor never computes the window (ADR-0067 rev 3 PINNED SURFACE SPEC
    // item 5 / option b: the seam takes the identity directly).
    let windowed = SvidRequest::new(spiffe_id.clone(), not_before, not_after);
    let svid = ca.issue_svid(&windowed).map_err(CaIssuanceError::ca)?;

    let row = IssuedCertificateRow {
        serial: svid.serial().clone(),
        spiffe_id: svid.spiffe_id().clone(),
        issuer_serial: intermediate.serial().clone(),
        not_before,
        not_after,
        node_id: node.clone(),
        issued_at,
        issuance_ordinal: ordinal,
    };

    // Bind issuance + audit: write the audit row through the `ObservationStore`
    // port (as `ObservationRow::IssuedCertificate`, exactly like `alloc_status`
    // / `node_health`) BEFORE returning the cert. On failure, drop the leaf and
    // surface the error — the cert and its audit row are observable together or
    // not at all (ADR-0063 D6; no silent issuance).
    observation
        .write(ObservationRow::IssuedCertificate(row))
        .await
        .map_err(CaIssuanceError::audit)?;

    Ok(svid)
}

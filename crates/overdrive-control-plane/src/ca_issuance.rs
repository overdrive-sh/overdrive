//! CA issuance + audit binding — every workload SVID issuance writes an
//! `issued_certificates` audit row, bound so an audit-write failure refuses the
//! issuance (built-in-ca / GH #28, ADR-0063 D6).
//!
//! This is the focused issuance seam the workload-start path calls to mint a
//! workload SVID. It composes the [`Ca`] driving port (issue the leaf + draw the
//! issuer serial from the node intermediate) with an [`IssuedCertificateAudit`]
//! driven port (record the issuance fact), and **binds the two**: the leaf and
//! its audit row are observable TOGETHER. If the audit row cannot be written,
//! the issuance fails — NO unaudited certificate ever escapes (KPI/AC US-CA-05;
//! ADR-0063 D6 "issuance is never silent").
//!
//! # State-layer hygiene (whitepaper §4, ADR-0063 D2/D6)
//!
//! The CA *material* (root key, intermediate keys) is **intent** (linearizable,
//! the [`crate::ca_boot`] path). The *record of what was issued* — the
//! `issued_certificates` row — is **observation** (gossiped when #36 lands;
//! single-node = local). This module writes ONLY the observation row; it never
//! touches the intent store. The audit port is the observation boundary.
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

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use overdrive_core::NodeId;
use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
use overdrive_core::traits::ca::{Ca, CaError, SvidMaterial, SvidRequest};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStoreError;
use overdrive_core::wall_clock::UnixInstant;

/// Audit validity window the control plane records for a freshly-issued
/// workload SVID — ~1 hour, matching the workload-SVID TTL profile (research
/// Finding 6).
///
/// The leaf's exact `not_before`/`not_after` are an adapter-internal detail
/// ([`SvidMaterial`] exposes no validity accessors per ADR-0063 D1 / research
/// Finding 5 — the leaf key never crosses the trait boundary, and neither does
/// the window). The audit row therefore records the control plane's OBSERVED
/// issuance window (`issued_at` plus this TTL) as the audit input, per
/// "persist inputs, not derived state" — the window the platform issued for,
/// observed at the moment of issuance.
const WORKLOAD_SVID_AUDIT_TTL: Duration = Duration::from_secs(3600);

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

/// Driven port for the `issued_certificates` audit surface (ADR-0063 D6).
///
/// The observation boundary [`issue_and_audit`] writes through. Production wires
/// the real `LocalObservationStore` (the impl below); a test wires a double —
/// most usefully a failing double, to exercise the no-silent-issuance sad path
/// (S-05-04) by injecting an audit-write failure at the port boundary.
///
/// This is a driven port, so the test double lives at the hexagonal boundary
/// (`.claude/skills/nw-hexagonal-testing`) — never inside the issuance logic.
#[async_trait]
pub trait IssuedCertificateAudit: Send + Sync {
    /// Record one issuance fact. On `Ok(())`, the row is durable and readable
    /// back via [`Self::issued_certificate_rows`]. On `Err`, the audit write
    /// failed — [`issue_and_audit`] surfaces this as
    /// [`CaIssuanceError::Audit`] and the issued cert is dropped.
    async fn record(&self, row: IssuedCertificateRow) -> Result<(), ObservationStoreError>;

    /// Read back every recorded `issued_certificates` row. The operator-
    /// observable audit surface (graduates to verification expectation O05).
    async fn issued_certificate_rows(
        &self,
    ) -> Result<Vec<IssuedCertificateRow>, ObservationStoreError>;
}

/// Production [`IssuedCertificateAudit`] binding over the single-node
/// `LocalObservationStore` (ADR-0012, revised 2026-04-24).
///
/// Delegates to the store's inherent `issued_certificates` audit-table methods
/// — the `issued_certificates` row is OBSERVATION, persisted in the production
/// observation store, mirroring the `alloc_status` / `node_health` plumbing
/// (ADR-0063 D6). The `ObservationStore` *trait* is unchanged; the audit-table
/// methods are inherent on `LocalObservationStore` because this audit surface is
/// a typed-bytes table read back here, not through the closed `ObservationRow`
/// write enum.
#[async_trait]
impl IssuedCertificateAudit for overdrive_store_local::LocalObservationStore {
    async fn record(&self, row: IssuedCertificateRow) -> Result<(), ObservationStoreError> {
        self.write_issued_certificate(row).await
    }

    async fn issued_certificate_rows(
        &self,
    ) -> Result<Vec<IssuedCertificateRow>, ObservationStoreError> {
        Self::issued_certificate_rows(self).await
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
/// 2. Mint the leaf (`ca.issue_svid(request)`) — a fresh certificate each call
///    (distinct serial, new validity), the re-issue mechanism (S-05-05).
/// 3. Build the [`IssuedCertificateRow`] from the FAITHFUL observed facts —
///    `serial` / `spiffe_id` from [`SvidMaterial`]'s per-call accessors,
///    `issuer_serial` from the [`IntermediateHandle`](overdrive_core::traits::ca::IntermediateHandle),
///    `issued_at` from the injected [`Clock`], and the observed validity window.
/// 4. **Write the audit row, then hand back the leaf.** If the write fails,
///    return [`CaIssuanceError::Audit`] and DROP the leaf — no unaudited cert
///    escapes (no silent issuance).
///
/// # Errors
///
/// * [`CaIssuanceError::Ca`] — the leaf or intermediate could not be signed.
/// * [`CaIssuanceError::Audit`] — the leaf was minted but its audit row could
///   not be written; the issuance is refused and the cert dropped.
pub async fn issue_and_audit(
    ca: &dyn Ca,
    audit: &Arc<dyn IssuedCertificateAudit>,
    clock: &dyn Clock,
    node: &NodeId,
    request: &SvidRequest,
) -> Result<SvidMaterial, CaIssuanceError> {
    // The node intermediate is the issuer of the leaf; its serial is the audit
    // row's `issuer_serial` (the chain link an auditor walks). Single-node:
    // idempotently cached by the adapter, so re-issue does not re-mint it.
    let intermediate = ca.issue_intermediate(node).map_err(CaIssuanceError::ca)?;

    // Mint the leaf — a FRESH certificate each call (distinct serial, new
    // validity window). This is the re-issue mechanism (S-05-05): calling
    // `issue_and_audit` again for the same `SpiffeId` produces a distinct leaf.
    let svid = ca.issue_svid(request).map_err(CaIssuanceError::ca)?;

    // Observe the issuance facts. `issued_at` is the clock snapshot; the
    // validity window is the control plane's observed issuance window (the leaf
    // exposes no validity accessor, so the audit row records the window the
    // platform issued for — an input, not a derived classification).
    let issued_at = UnixInstant::from_clock(clock);
    let not_before = issued_at;
    let not_after = issued_at + WORKLOAD_SVID_AUDIT_TTL;

    let row = IssuedCertificateRow {
        serial: svid.serial().clone(),
        spiffe_id: svid.spiffe_id().clone(),
        issuer_serial: intermediate.serial().clone(),
        not_before,
        not_after,
        node_id: node.clone(),
        issued_at,
    };

    // Bind issuance + audit: write the audit row BEFORE returning the cert. On
    // failure, drop the leaf and surface the error — the cert and its audit row
    // are observable together or not at all (ADR-0063 D6; no silent issuance).
    audit.record(row).await.map_err(CaIssuanceError::audit)?;

    Ok(svid)
}

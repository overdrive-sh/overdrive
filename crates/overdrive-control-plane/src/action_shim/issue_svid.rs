//! Action-shim executor for `Action::IssueSvid` / `Action::DropSvid` per
//! ADR-0067 D3 — the ONE place workload-CA I/O happens in the convergence
//! loop (the ADR-0023 async shim boundary).
//!
//! `IssueSvid` composes the [`Ca`] driving port + the [`ObservationStore`]
//! driven port (via [`ca_issuance::issue_and_audit`], REUSED wholesale — it
//! mints the leaf, writes the `issued_certificates` audit row, and refuses
//! issuance on audit-write failure) with the in-process [`IdentityMgr`]
//! held-SVID store. On success it holds the returned [`SvidMaterial`] and
//! opportunistically refreshes the trust bundle (D6). `DropSvid` removes the
//! held entry so the node-held leaf key is no longer reachable (O2/K2).
//!
//! # Audit-before-hold (K4 fail-closed)
//!
//! If `issue_and_audit` returns an audit-write failure
//! ([`CaIssuanceError::Audit`]), the executor REPORTS the failure and
//! [`IdentityMgr`] does NOT hold — no unaudited [`SvidMaterial`] ever enters
//! the held map (ADR-0063 D6 "issuance is never silent").
//!
//! # `not_after` sourcing
//!
//! The executor does NOT compute or pass a validity window and NEVER reads a
//! clock to build one (that would re-create the drift the ADR-0063 rev-2
//! amendment closed). [`ca_issuance::issue_and_audit`] owns the single window
//! computation from its injected `clock`; the returned [`SvidMaterial`]
//! already carries the correct `not_after`. The executor passes only the
//! action's [`SpiffeId`](overdrive_core::id::SpiffeId) (ADR-0067 rev 3 PINNED
//! SURFACE SPEC item 5 / option b) — there is no throwaway window anywhere.

use overdrive_core::reconcilers::Action;
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStore;
use thiserror::Error;

use crate::ca_issuance::{self, CaIssuanceError};
use crate::identity_mgr::IdentityMgr;

/// Dispatch error for the `issue_svid` executor. Pass-through embedding via
/// `#[from]` per `.claude/rules/development.md` § Errors / pass-through.
#[derive(Debug, Error)]
pub enum IssueSvidDispatchError {
    /// `ca_issuance::issue_and_audit` failed — either the leaf could not be
    /// signed ([`CaIssuanceError::Ca`]) or the leaf was minted but its
    /// `issued_certificates` audit row could not be written
    /// ([`CaIssuanceError::Audit`]). In BOTH cases the executor holds NOTHING:
    /// no unaudited (or unsigned) [`SvidMaterial`] enters the held map
    /// (K4 fail-closed, ADR-0063 D6).
    #[error("issue_svid executor: CA issuance failed: {source}")]
    Issuance {
        /// The underlying typed issuance failure.
        #[from]
        source: CaIssuanceError,
    },
}

/// Dispatch one `Action::IssueSvid`. Calls [`ca_issuance::issue_and_audit`]
/// (which mints the leaf, writes the audit row, and binds the two), then holds
/// the returned material in [`IdentityMgr`] and opportunistically refreshes the
/// trust bundle.
///
/// **Audit-before-hold**: if issuance fails (signing OR audit-write), the
/// executor returns `Err` and holds NOTHING — no unaudited SVID escapes.
///
/// The executor sources only `spiffe_id` from the action and passes it to
/// `issue_and_audit`; the validity window is `issue_and_audit`'s sole concern
/// (computed once from its injected `clock`), so there is no throwaway window
/// here and the executor never reads a clock to build one.
///
/// # Errors
///
/// Returns [`IssueSvidDispatchError::Issuance`] when `issue_and_audit` fails —
/// either a [`Ca`] signing failure or an audit-write failure. On either, the
/// held map is left untouched.
///
/// # Panics
///
/// Panics if `action` is not [`Action::IssueSvid`]. The action shim's match arm
/// is the sole caller; passing the wrong variant is a programmer error.
pub async fn dispatch_issue(
    action: &Action,
    ca: &dyn Ca,
    observation: &dyn ObservationStore,
    clock: &dyn Clock,
    identity: &IdentityMgr,
) -> Result<(), IssueSvidDispatchError> {
    let Action::IssueSvid { alloc_id, spiffe_id, node_id, correlation: _ } = action else {
        panic!(
            "action_shim::issue_svid::dispatch_issue invoked with wrong Action \
             variant — caller is the action shim's match arm and is the sole \
             expected caller"
        );
    };

    // Mint the leaf + write the audit row + bind the two (ADR-0063 D6). The
    // window is `issue_and_audit`'s sole concern; we pass only the identity
    // (ADR-0067 rev 3 spec item 5 / option b). On an audit-write failure this
    // returns `CaIssuanceError::Audit` and NO SvidMaterial — so the hold below
    // never runs (K4 fail-closed). The `?` converts the typed `CaIssuanceError`
    // into `IssueSvidDispatchError::Issuance` via the `#[from]` embedding.
    let svid = ca_issuance::issue_and_audit(ca, observation, clock, node_id, spiffe_id).await?;

    // Hold the minted material (re-issue overwrites; ADR-0067 D2). The leaf key
    // stays inside IdentityMgr (K2).
    identity.hold(alloc_id.clone(), svid);

    // D6 bundle refresh — opportunistically install the current trust bundle.
    // A bundle-compose failure is non-fatal to the issuance that already
    // succeeded and was audited: the cert is held and observable, so we do NOT
    // unwind the hold on a bundle-refresh error. Surface it so a persistent
    // refresh failure is not silently swallowed.
    if let Ok(bundle) = ca.trust_bundle() {
        identity.set_bundle(bundle);
    }

    Ok(())
}

/// Dispatch one `Action::DropSvid`. Removes the held entry for `alloc_id` so the
/// node-held leaf private key is no longer reachable in the held set (O2/K2).
/// Idempotent: dropping an alloc that is not held is a no-op.
///
/// # Panics
///
/// Panics if `action` is not [`Action::DropSvid`]. As with [`dispatch_issue`],
/// the action shim's match arm is the sole caller.
pub fn dispatch_drop(action: &Action, identity: &IdentityMgr) {
    let Action::DropSvid { alloc_id, correlation: _ } = action else {
        panic!(
            "action_shim::issue_svid::dispatch_drop invoked with wrong Action \
             variant — caller is the action shim's match arm and is the sole \
             expected caller"
        );
    };
    // Remove the held entry so the node-held leaf private key is no longer
    // reachable (O2/K2). Idempotent — `drop_svid` is a no-op when not held.
    identity.drop_svid(alloc_id);
}

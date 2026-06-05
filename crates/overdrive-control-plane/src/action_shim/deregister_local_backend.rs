//! Action shim for `Action::DeregisterLocalBackend` per ADR-0053 § 3.
//!
//! Dispatch invokes [`Dataplane::deregister_local_backend`] for the
//! `(vip, vip_port, proto)` whose registration should be removed. The
//! single call performs the dual-removal (ADR-0053 rev 2026-06-05,
//! DDD-5a; caller-supplied backend, GH #211): both the
//! `LOCAL_BACKEND_MAP` forward entry AND its matching `REVERSE_LOCAL_MAP`
//! reverse entry are removed. The reverse key is built from the `backend`
//! carried on the action, NOT derived from a forward read-back — so the
//! removal is retry-safe (a retry after a partial failure still carries
//! the backend identity). The trait contract pins idempotence — removing
//! an entry that does not exist is `Ok(())`, not an error.
//!
//! No correlation-driven follow-up is required at the shim level
//! per the same rationale as
//! [`super::register_local_backend::dispatch`].

use overdrive_core::reconcilers::Action;
use overdrive_core::traits::dataplane::{Dataplane, DataplaneError};
use thiserror::Error;

/// Dispatch error for the local-backend deregistration shim.
/// Pass-through embedding via `#[from]` per
/// `.claude/rules/development.md` § Errors / pass-through.
#[derive(Debug, Error)]
pub enum DeregisterLocalBackendDispatchError {
    /// `Dataplane::deregister_local_backend` failed. KeyNotFound is
    /// NOT surfaced as an error per the ADR-0053 § 2 trait contract;
    /// the variant only fires when the underlying map delete
    /// genuinely fails (kernel-side `EINVAL` / corrupted FD).
    #[error("deregister_local_backend failed: {source}")]
    Dataplane {
        #[from]
        source: DataplaneError,
    },
}

/// Dispatch one `Action::DeregisterLocalBackend`. Calls
/// [`Dataplane::deregister_local_backend`] with the `(vip, vip_port,
/// backend, proto)` carried on the action.
///
/// # Errors
///
/// Returns [`DeregisterLocalBackendDispatchError::Dataplane`] when
/// the underlying adapter rejects the map delete. KeyNotFound is
/// idempotent per the trait contract and does not surface here.
///
/// # Panics
///
/// Panics if `action` is not [`Action::DeregisterLocalBackend`]. The
/// action shim's match arm is the sole caller; passing the wrong
/// variant is a programmer error.
pub async fn dispatch(
    action: &Action,
    dataplane: &dyn Dataplane,
) -> Result<(), DeregisterLocalBackendDispatchError> {
    let Action::DeregisterLocalBackend { vip, vip_port, backend, proto, .. } = action else {
        panic!(
            "action_shim::deregister_local_backend::dispatch invoked with \
             wrong Action variant — caller is the action shim's match \
             arm and is the sole expected caller"
        );
    };
    dataplane.deregister_local_backend(*vip, *vip_port, *backend, *proto).await?;
    Ok(())
}

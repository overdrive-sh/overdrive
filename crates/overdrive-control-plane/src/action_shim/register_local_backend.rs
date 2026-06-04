//! Action shim for `Action::RegisterLocalBackend` per ADR-0053 § 3.
//!
//! Dispatch invokes [`Dataplane::register_local_backend`] for the
//! local-side backend the hydrator's classifier (ADR-0053 § 4)
//! resolved as same-host. The cgroup_sock_addr program reads the
//! resulting `LOCAL_BACKEND_MAP` entry on every subsequent
//! `connect(vip:vip_port)` from a process inside the configured
//! cgroup, rewriting the destination to the backend's real address.
//!
//! No correlation-driven follow-up is required at the shim level —
//! the cgroup hook is not an HTTP call surface and produces no
//! observation row. The hydrator's next tick reads `desired` from
//! the bridge's `service_backends` rows; convergence is observable
//! via the read-back from the production handle in the
//! walking-skeleton test, not via an obs row.
//!
//! A `Dataplane::register_local_backend` failure surfaces as
//! [`RegisterLocalBackendDispatchError::Dataplane`] up to the
//! action-shim's match arm, which converts to
//! [`super::ShimError::RegisterLocalBackend`] for the per-arm
//! dispatch contract.

use overdrive_core::reconcilers::Action;
use overdrive_core::traits::dataplane::{Dataplane, DataplaneError};
use thiserror::Error;

/// Dispatch error for the local-backend registration shim.
/// Pass-through embedding via `#[from]` per
/// `.claude/rules/development.md` § Errors / pass-through.
#[derive(Debug, Error)]
pub enum RegisterLocalBackendDispatchError {
    /// `Dataplane::register_local_backend` failed. The cgroup hook
    /// could not install the entry; subsequent `connect(vip:port)`
    /// calls will NOT be rewritten until the next tick re-attempts.
    #[error("register_local_backend failed: {source}")]
    Dataplane {
        #[from]
        source: DataplaneError,
    },
}

/// Dispatch one `Action::RegisterLocalBackend`. Calls
/// [`Dataplane::register_local_backend`] with the `(vip, vip_port,
/// backend)` triple carried on the action.
///
/// # Errors
///
/// Returns [`RegisterLocalBackendDispatchError::Dataplane`] when
/// the underlying adapter rejects the map insert (typically a
/// kernel `EINVAL` / `ENOMEM` / `EPERM` mapped to
/// `DataplaneError::LocalBackendInsert`).
///
/// # Panics
///
/// Panics if `action` is not [`Action::RegisterLocalBackend`]. The
/// action shim's match arm is the sole caller; passing the wrong
/// variant is a programmer error. Follows the established precedent
/// across action-shim dispatch wrappers (see
/// [`super::write_service_backend_row::dispatch`] and
/// [`super::dataplane_update_service::dispatch`]).
pub async fn dispatch(
    action: &Action,
    dataplane: &dyn Dataplane,
) -> Result<(), RegisterLocalBackendDispatchError> {
    let Action::RegisterLocalBackend { vip, vip_port, backend, proto, .. } = action else {
        panic!(
            "action_shim::register_local_backend::dispatch invoked with \
             wrong Action variant — caller is the action shim's match \
             arm and is the sole expected caller"
        );
    };
    dataplane.register_local_backend(*vip, *vip_port, *backend, *proto).await?;
    Ok(())
}

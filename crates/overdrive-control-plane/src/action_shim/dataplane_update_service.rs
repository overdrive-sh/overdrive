//! Action shim for `Action::DataplaneUpdateService` per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md`
//! §§ 7, 9 + ADR-0042.
//!
//! Dispatch invokes [`Dataplane::update_service`]. The hydrator
//! reconciler learns whether the update landed by reading the
//! programmed service map back (ADR-0299), so dispatch records no
//! outcome row; a failed update is logged as one structured event.
//!
//! # Failure surface
//!
//! The failure surface is a log event, NOT a `TerminalCondition` per
//! architecture.md § 7 — service hydration cannot terminate an
//! allocation; mixing the channels would erode ADR-0037's "every
//! terminal claim has a single typed source" invariant. A
//! `Dataplane::update_service` `Err`, or an IPv6 VIP refused at the
//! operator-visible `ServiceFrontend::new` seam, emits
//! `dataplane.update_service.failed`. The hydrator's retry follows from
//! the programmed-map diff.

use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::fingerprint::fingerprint;
use overdrive_core::reconcilers::Action;
use overdrive_core::traits::dataplane::Dataplane;

/// Dispatch one `Action::DataplaneUpdateService`. Calls
/// [`Dataplane::update_service`] and emits one structured
/// `dataplane.update_service.failed` event when the update does not
/// land. See module docs for the failure-surface contract.
///
/// # Panics
///
/// Panics if `action` is not [`Action::DataplaneUpdateService`].
/// The action shim's match arm is the sole caller; passing the wrong
/// variant is a programmer error.
pub async fn dispatch(action: &Action, dataplane: &dyn Dataplane) {
    let Action::DataplaneUpdateService { service_id, vip, port, proto, backends, correlation: _ } =
        action
    else {
        panic!(
            "action_shim::dataplane_update_service::dispatch invoked \
             with wrong Action variant — caller is the action shim's \
             match arm and is the sole expected caller"
        );
    };

    let fp = fingerprint(vip, backends);

    // V4 validation lives here, at the operator-visible rejection site
    // (ADR-0060 D1a): `ServiceFrontend::new` rejects an IPv6 VIP before
    // the dataplane is ever called — NOT a late opaque `DataplaneError`
    // in an adapter.
    let frontend = match ServiceFrontend::new(*vip, *port, *proto) {
        Ok(frontend) => frontend,
        Err(refusal) => {
            tracing::warn!(
                name: "dataplane.update_service.failed",
                service_id = %service_id,
                fingerprint = fp,
                reason = %refusal,
            );
            return;
        }
    };

    match dataplane.update_service(frontend, backends.clone()).await {
        Ok(()) => {}
        Err(err) => {
            tracing::warn!(
                name: "dataplane.update_service.failed",
                service_id = %service_id,
                fingerprint = fp,
                reason = %err,
            );
        }
    }
}

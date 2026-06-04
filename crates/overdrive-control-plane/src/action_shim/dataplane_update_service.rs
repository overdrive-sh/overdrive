//! Action shim for `Action::DataplaneUpdateService` per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md`
//! §§ 7, 9, 12 + ADR-0042.
//!
//! Dispatch invokes [`Dataplane::update_service`] and writes the
//! outcome into the `service_hydration_results` ObservationStore
//! table. The hydrator reconciler (Slice 08-02) reads the row at the
//! next tick via `actual` and either advances on
//! `Completed { fingerprint == desired.fingerprint }` or, on
//! `Failed`, applies its retry-budget policy from the typed View.
//!
//! # Failure surface
//!
//! The failure surface is **observation, NOT a `TerminalCondition`**
//! per architecture.md § 7 — service hydration cannot terminate an
//! allocation; mixing the channels would erode ADR-0037's "every
//! terminal claim has a single typed source" invariant. A
//! `Dataplane::update_service` `Err` translates to a `Failed` row
//! whose `reason` is `Display::to_string(&err)`; the dispatch fn
//! itself returns `Ok(DispatchOutcome::Failed { ... })` to the
//! caller — only an `ObservationStoreError` causes the dispatch fn
//! to return `Err`.

use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::fingerprint::fingerprint;
use overdrive_core::id::{NodeId, ServiceVip};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::dataplane::Dataplane;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationRow, ObservationStore, ObservationStoreError,
    ServiceHydrationResultRow, ServiceHydrationStatus,
};
use thiserror::Error;

/// Outcome of a single `Action::DataplaneUpdateService` dispatch.
///
/// Returned to the action shim's match arm so it can record per-arm
/// observation. Both variants represent a successful obs-write; an
/// obs-write failure surfaces as
/// [`ServiceHydrationDispatchError::ObservationWrite`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// `Dataplane::update_service` returned `Ok(())`.
    Completed,
    /// `Dataplane::update_service` returned `Err(_)` and the action
    /// shim wrote a `Failed` row to the ObservationStore.
    Failed,
}

/// Dispatch error for the service-hydration shim. Pass-through
/// embedding via `#[from]` per `.claude/rules/development.md`
/// § Errors / pass-through embedding.
#[derive(Debug, Error)]
pub enum ServiceHydrationDispatchError {
    /// Writing the `service_hydration_results` row failed. Note:
    /// `Dataplane::update_service` errors do NOT surface as
    /// dispatch errors — they translate to a `Failed` row written
    /// to the ObservationStore (see [`DispatchOutcome::Failed`]).
    #[error("observation store write failed: {source}")]
    ObservationWrite {
        #[from]
        source: ObservationStoreError,
    },

    /// The `ServiceVip` carries an IPv6 address but the Phase 2.2
    /// dataplane is IPv4-only (architecture.md § 6, GH #155).
    #[error("IPv6 VIP {vip} not supported in Phase 2.2 dataplane (GH #155)")]
    Ipv6Unsupported {
        /// The offending VIP, for structured error reporting.
        vip: ServiceVip,
    },
}

/// Dispatch one `Action::DataplaneUpdateService`. Calls
/// [`Dataplane::update_service`], then writes a
/// `service_hydration_results` row whose status reflects the
/// outcome. See module docs for the failure-surface contract.
///
/// # Errors
///
/// Returns [`ServiceHydrationDispatchError::ObservationWrite`] only
/// when the ObservationStore itself rejects the write. A
/// `Dataplane::update_service` failure does NOT surface as `Err` —
/// it lands as a `Failed` observation row and the fn returns
/// `Ok(DispatchOutcome::Failed)`. An IPv6 VIP likewise lands as a
/// `Failed` row (Phase 2.2 is IPv4-only per architecture.md § 6,
/// GH #155).
///
/// # Panics
///
/// Panics if `action` is not [`Action::DataplaneUpdateService`].
/// The action shim's match arm is the sole caller; passing the wrong
/// variant is a programmer error.
pub async fn dispatch(
    action: &Action,
    dataplane: &dyn Dataplane,
    observation: &dyn ObservationStore,
    tick: &TickContext,
    writer: &NodeId,
) -> Result<DispatchOutcome, ServiceHydrationDispatchError> {
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
    // (ADR-0060 D1a): `ServiceFrontend::new` rejects an IPv6 VIP, which
    // we map to the existing operator-visible `Failed` row — NOT a late
    // opaque `DataplaneError` in an adapter.
    let Ok(frontend) = ServiceFrontend::new(*vip, *port, *proto) else {
        let reason = ServiceHydrationDispatchError::Ipv6Unsupported { vip: *vip }.to_string();
        let row = ServiceHydrationResultRow {
            service_id: *service_id,
            fingerprint: fp,
            status: ServiceHydrationStatus::Failed {
                fingerprint: fp,
                failed_at: tick.now_unix,
                reason,
            },
            updated_at: LogicalTimestamp {
                counter: tick.tick.saturating_add(1),
                writer: writer.clone(),
            },
        };
        observation.write(ObservationRow::ServiceHydration(row)).await?;
        return Ok(DispatchOutcome::Failed);
    };
    let dataplane_result = dataplane.update_service(frontend, backends.clone()).await;

    let (status, outcome) = match &dataplane_result {
        Ok(()) => (
            ServiceHydrationStatus::Completed { fingerprint: fp, applied_at: tick.now_unix },
            DispatchOutcome::Completed,
        ),
        Err(err) => (
            ServiceHydrationStatus::Failed {
                fingerprint: fp,
                failed_at: tick.now_unix,
                reason: err.to_string(),
            },
            DispatchOutcome::Failed,
        ),
    };

    let row = ServiceHydrationResultRow {
        service_id: *service_id,
        fingerprint: fp,
        status,
        updated_at: LogicalTimestamp {
            counter: tick.tick.saturating_add(1),
            writer: writer.clone(),
        },
    };
    observation.write(ObservationRow::ServiceHydration(row)).await?;
    Ok(outcome)
}

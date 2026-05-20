//! Action shim for `Action::WriteServiceBackendRow` per
//! `docs/feature/backend-discovery-bridge-service-reachability/
//! design/architecture.md` § 4.4.
//!
//! Dispatch writes the embedded [`ServiceBackendRow`] to the
//! ObservationStore via `ObservationRow::ServiceBackend(row)`. No
//! correlation-driven follow-up is needed at the shim level — the
//! bridge's next tick reads the row stream (transitively through the
//! runtime's hydrate path) and observes its own write via the dedup
//! fingerprint persisted in the bridge's
//! [`BackendDiscoveryBridgeView`].
//!
//! [`ServiceBackendRow`]: overdrive_core::traits::observation_store::ServiceBackendRow
//! [`BackendDiscoveryBridgeView`]:
//!     overdrive_core::reconciler::backend_discovery_bridge::BackendDiscoveryBridgeView

use overdrive_core::reconciler::Action;
use overdrive_core::traits::observation_store::{
    ObservationRow, ObservationStore, ObservationStoreError,
};

/// Dispatch one `Action::WriteServiceBackendRow`. Writes the
/// embedded row to the ObservationStore via
/// `ObservationRow::ServiceBackend(row.clone())`.
///
/// Per `.claude/rules/development.md` § Errors / pass-through
/// embedding: this fn does NOT re-wrap the typed
/// [`ObservationStoreError`] — it propagates the underlying error
/// via `?` and the caller's match arm decides whether to surface as
/// [`super::ShimError::Observation`].
///
/// # Errors
///
/// Returns the underlying [`ObservationStoreError`] when the
/// ObservationStore rejects the write itself. Per architecture.md
/// § 4.4 there is no other failure surface — the bridge's reconcile
/// loop observes its own write via the dedup fingerprint on the
/// next tick, so a write that succeeds at the obs-store boundary is
/// always sufficient (no follow-up correlation is required at the
/// shim).
///
/// # Panics
///
/// Panics if `action` is not [`Action::WriteServiceBackendRow`]. The
/// action shim's exhaustive match arm is the sole expected caller;
/// passing the wrong variant is a programmer error and follows the
/// established precedent across action-shim dispatch wrappers (see
/// [`super::dataplane_update_service::dispatch`]).
pub async fn dispatch(
    action: &Action,
    observation: &dyn ObservationStore,
) -> Result<(), ObservationStoreError> {
    let Action::WriteServiceBackendRow { row, correlation: _ } = action else {
        panic!(
            "action_shim::write_service_backend_row::dispatch invoked \
             with wrong Action variant — caller is the action shim's \
             match arm and is the sole expected caller"
        );
    };
    observation.write(ObservationRow::ServiceBackend(row.clone())).await
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test fixtures may panic on programmer error per project precedent in tests/"
)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::num::NonZeroU16;
    use std::sync::Arc;

    use overdrive_core::id::{ContentHash, CorrelationKey, NodeId, ServiceId, ServiceVip};
    use overdrive_core::reconciler::Action;
    use overdrive_core::traits::observation_store::{
        LogicalTimestamp, ObservationStore, ServiceBackendRow,
    };
    use overdrive_sim::adapters::observation_store::SimObservationStore;

    use super::dispatch;

    /// Build a canonical [`ServiceBackendRow`] for test fixtures.
    /// Inputs (not derived state) are pinned; the row's fields all
    /// come from constructor arguments.
    fn canonical_row(writer: &NodeId) -> ServiceBackendRow {
        let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("valid VIP");
        let port = NonZeroU16::new(8080).expect("non-zero port");
        let service_id = ServiceId::derive(&vip, port, "service-map");
        ServiceBackendRow {
            service_id,
            vip: Ipv4Addr::new(10, 0, 0, 1),
            backends: Vec::new(),
            updated_at: LogicalTimestamp { counter: 1, writer: writer.clone() },
        }
    }

    /// Build a canonical [`CorrelationKey`] for test fixtures via the
    /// public `derive(target, spec_hash, purpose)` constructor.
    fn canonical_correlation() -> CorrelationKey {
        let spec_hash = ContentHash::of(b"backend-discovery-bridge::test-spec");
        CorrelationKey::derive(
            "backend-discovery-bridge/test-workload",
            &spec_hash,
            "write-service-backend-row",
        )
    }

    /// `dispatch(Action::WriteServiceBackendRow)` writes one
    /// `ObservationRow::ServiceBackend(row)` to the
    /// ObservationStore — port-to-port assertion at the
    /// `service_backends_rows` accessor.
    #[tokio::test]
    async fn dispatch_writes_observation_row() {
        let writer = NodeId::new("test-writer").expect("valid node id");
        let obs: Arc<dyn ObservationStore> =
            Arc::new(SimObservationStore::single_peer(writer.clone(), 1));
        let row = canonical_row(&writer);
        let action = Action::WriteServiceBackendRow {
            row: row.clone(),
            correlation: canonical_correlation(),
        };

        dispatch(&action, obs.as_ref()).await.expect("dispatch must write the row");

        let stored = obs.service_backends_rows(&row.service_id).await.expect("read must succeed");
        assert_eq!(stored.len(), 1, "exactly one row must be persisted");
        assert_eq!(stored[0], row, "persisted row must equal the dispatched row");
    }

    /// `dispatch` panics when invoked with a non-matching Action
    /// variant. The sole expected caller is the action shim's
    /// exhaustive match arm; passing the wrong variant is a
    /// programmer error per the established precedent across
    /// action-shim wrappers.
    #[tokio::test]
    #[should_panic(
        expected = "action_shim::write_service_backend_row::dispatch invoked with wrong Action variant"
    )]
    async fn dispatch_panics_on_wrong_variant() {
        let writer = NodeId::new("test-writer").expect("valid node id");
        let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(writer, 2));
        let action = Action::Noop;
        let _ = dispatch(&action, obs.as_ref()).await;
    }
}

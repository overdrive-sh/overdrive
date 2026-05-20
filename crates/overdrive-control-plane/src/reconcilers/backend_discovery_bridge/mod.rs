//! `BackendDiscoveryBridge` reconciler — Phase 2.2
//! (`backend-discovery-bridge-service-reachability` step 01-02).
//!
//! Bridges Service intent + Running alloc observations onto the
//! `service_backends` ObservationStore table. Per architecture.md
//! § 4.1-4.2 the canonical reconciler types and trait impl live in
//! `overdrive-core::reconciler::backend_discovery_bridge` alongside
//! the `Reconciler` trait and its `WorkloadLifecycle` /
//! `ServiceMapHydrator` peers — [`overdrive_core::reconciler::AnyReconciler`]
//! holds the concrete type in its `BackendDiscoveryBridge` variant,
//! and `overdrive-core` cannot depend on `overdrive-control-plane`.
//!
//! This module exists as the architecture-mandated entry point
//! (`crates/overdrive-control-plane/src/reconcilers/
//! backend_discovery_bridge/`) per architecture.md § 4.1. It
//! re-exports the public surface for callers that previously
//! imported from this path; the reconciler's actual implementation
//! is in `overdrive_core::reconciler::backend_discovery_bridge`.
//!
//! Layout mirrors the sibling [`crate::reconcilers::service_map_hydrator`]
//! shape:
//!
//! - [`fingerprint`] — co-located pure decision fn re-exporting the
//!   canonical `(ServiceVip, &[Backend]) -> BackendSetFingerprint`
//!   hash from `overdrive-core::dataplane::fingerprint`.
//! - [`BackendDiscoveryBridge`] — concrete reconciler with mandatory
//!   `host_ipv4` + `writer_node_id` constructor parameters.

pub mod fingerprint;

pub use overdrive_core::reconciler::backend_discovery_bridge::{
    BackendDiscoveryBridge, BackendDiscoveryBridgeState, BackendDiscoveryBridgeView,
    ProjectedListener, RunningAllocSet, ServiceListenerSet,
};

/// `ReconcilerName` constant for this reconciler. Wired into the
/// runtime registry per ADR-0035 / ADR-0036 by DELIVER step 01-04.
pub const NAME: &str = "backend-discovery-bridge";

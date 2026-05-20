//! Pure decision fn — content-hash of a `(ServiceVip, &[Backend])`
//! pair for the bridge's dedup loop per architecture.md § 4.2.
//!
//! Re-exports the canonical hash from
//! `overdrive-core::dataplane::fingerprint`. The hash uses rkyv
//! archived bytes per `.claude/rules/development.md` § "Hashing
//! requires deterministic serialization" — rkyv 0.8 archives are
//! canonical by construction, so the byte feed into blake3 is
//! deterministic across processes, runs, and seeds.
//!
//! Co-located in this module (rather than the bridge module proper)
//! to mirror the sibling [`crate::reconcilers::service_map_hydrator`]
//! layout. The function itself lives in `overdrive-core` because it
//! is consumed by the reconciler's `reconcile` body — which lives in
//! `overdrive-core` per the `AnyReconciler::BackendDiscoveryBridge`
//! dispatch shape — and re-exporting from here keeps the
//! architecture-mandated `crates/overdrive-control-plane/src/
//! reconcilers/backend_discovery_bridge/fingerprint.rs` entry-point
//! present for module-path consumers without inducing a sim-control
//! dependency cycle.

pub use overdrive_core::dataplane::fingerprint::{BackendSetFingerprint, fingerprint};

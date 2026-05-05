//! Reconcilers shipped by the control plane.
//!
//! Phase 1 reconcilers (`noop-heartbeat`, `JobLifecycle`) live
//! inline in `reconciler_runtime.rs` per ADR-0013 / ADR-0035. Phase
//! 2.2 introduces the first reconciler whose body is non-trivial
//! enough to warrant a dedicated module: the
//! [`service_map_hydrator`].
//!
//! Future Phase 2+ reconcilers (POLICY_MAP / IDENTITY_MAP /
//! FS_POLICY_MAP / conntrack / sockops / kTLS) follow the same
//! shape per the `service_map_hydrator` reference implementation.
//!
//! **RED scaffold** — every body panics via `todo!()` until
//! DELIVER fills it per the carpaccio slice plan.

pub mod service_map_hydrator;

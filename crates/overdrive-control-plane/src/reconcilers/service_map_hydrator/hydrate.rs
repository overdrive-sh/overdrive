//! Async hydration arms for the `ServiceMapHydrator` reconciler
//! per ADR-0036 + architecture.md § 8 *Hydration shape*.
//!
//! The runtime owns hydration end-to-end; the reconciler author
//! only writes `reconcile`. Two free functions land in this module:
//!
//! - `hydrate_desired(target, state) -> ServiceMapHydratorState`
//!   reads `service_backends` rows for the target `ServiceId` and
//!   wraps `vip: Ipv4Addr` into `ServiceVip` at the read
//!   boundary. Computes the fingerprint via
//!   `overdrive_core::dataplane::fingerprint`.
//!
//! - `hydrate_actual(target, state) -> ServiceMapHydratorState`
//!   reads `service_hydration_results` rows for the target
//!   `ServiceId` and projects them into `ServiceHydrationStatus`.
//!
//! Both arms read **only the ObservationStore** — the hydrator
//! is purely an observation-driven reconciler. Neither arm
//! touches the IntentStore.
//!
//! Per architecture.md § 8 the bodies live as match arms inside
//! the runtime's existing `hydrate_desired` / `hydrate_actual`
//! free functions in `reconciler_runtime.rs`. This module hosts
//! the helper logic those arms call (`service_id_from_target`,
//! the row-to-state projection helpers).
//!
//! **RED scaffold** — bodies panic via `todo!()` until DELIVER
//! fills them per Slice 08.

#![allow(dead_code)]

pub const SCAFFOLD: bool = true;

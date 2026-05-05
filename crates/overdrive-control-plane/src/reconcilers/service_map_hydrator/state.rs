//! `ServiceMapHydratorState` — typed projection of intent +
//! observation for the hydrator reconciler per ADR-0021/0036 +
//! architecture.md § 8.
//!
//! Two parts merged by the runtime before `reconcile` is called:
//!
//! - `desired` — keyed by `ServiceId`, sourced from
//!   `service_backends` ObservationStore rows. Carries
//!   `(vip, backends, fingerprint)`.
//! - `actual` — keyed by `ServiceId`, sourced from the new
//!   `service_hydration_results` ObservationStore table.
//!   Carries `ServiceHydrationStatus` (Pending / Completed /
//!   Failed) so the reconciler observes the dataplane's
//!   confirmed state, not a next-action prediction (Drift 2).
//!
//! `BTreeMap` per `.claude/rules/development.md` § Ordered-
//! collection choice — deterministic iteration order is
//! load-bearing for the Maglev permutation generator.
//!
//! **RED scaffold** — type bodies are placeholders; actual fields
//! land per Slice 08.

// Imports deferred until DELIVER fills the State body. The
// canonical shape lives in
// `docs/feature/phase-2-xdp-service-map/design/architecture.md`
// § 8 *type State*; DELIVER transcribes it.

/// Hydrator state — split into `desired` and `actual` projections
/// merged by the runtime before `reconcile` per ADR-0036.
///
/// **RED scaffold** — empty placeholder; DELIVER fills the fields
/// per Slice 08 / S-2.2-26..30.
#[derive(Debug, Clone, Default)]
pub struct ServiceMapHydratorState {
    /// Reserved — the canonical `desired: BTreeMap<ServiceId,
    /// ServiceDesired>` and `actual: BTreeMap<ServiceId,
    /// ServiceHydrationStatus>` fields land per Slice 08.
    _scaffold_marker: (),
}

/// Desired backend set for a single service.
///
/// **RED scaffold** — empty placeholder; DELIVER fills the fields.
#[derive(Debug, Clone, Default)]
pub struct ServiceDesired {
    _scaffold_marker: (),
}

/// Confirmed hydration outcome — populated by the action shim
/// after `Dataplane::update_service` returns. Three variants:
/// `Pending`, `Completed { fingerprint, applied_at }`, `Failed
/// { fingerprint, failed_at, reason }` per architecture.md § 8.
///
/// **RED scaffold** — placeholder; DELIVER fills the variant
/// payloads per Slice 08 / S-2.2-26..30.
#[derive(Debug, Clone)]
pub enum ServiceHydrationStatus {
    /// No `service_hydration_results` row yet for this service —
    /// fresh state, never dispatched.
    Pending,
}

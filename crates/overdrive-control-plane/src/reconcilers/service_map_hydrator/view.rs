//! `ServiceMapHydratorView` — typed reconciler memory persisted by
//! the runtime via `RedbViewStore` per ADR-0035 + architecture.md
//! § 8.
//!
//! Persists *inputs* per `.claude/rules/development.md` § Persist
//! inputs, not derived state — `attempts` + `last_failure_seen_at`
//! + `last_attempted_fingerprint`. The next-attempt deadline is
//! recomputed every tick from these inputs + the live backoff
//! policy, never persisted.
//!
//! `BTreeMap` per § Ordered-collection choice.

// Imports deferred until DELIVER fills the View body — `ServiceId`,
// `UnixInstant`, and `BackendSetFingerprint` need their serde derives
// (and `Default` on `UnixInstant`) wired up in
// `overdrive-core` per the carpaccio plan. The scaffold ships an
// empty View; the canonical shape lives in
// `docs/feature/phase-2-xdp-service-map/design/architecture.md` § 8
// *type View* and DELIVER transcribes it.

use serde::{Deserialize, Serialize};

/// Hydrator-specific reconciler memory persisted by the runtime per
/// ADR-0035. The canonical shape ships per Slice 08 — see
/// architecture.md § 8 + this module's docstring for the locked
/// fields (`retries: BTreeMap<ServiceId, RetryMemory>` with
/// per-service `attempts`, `last_failure_seen_at`,
/// `last_attempted_fingerprint`).
///
/// **RED scaffold** — empty placeholder; DELIVER fills the fields
/// per Slice 08 / S-2.2-26..30.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServiceMapHydratorView {
    /// Reserved — the canonical `BTreeMap<ServiceId, RetryMemory>`
    /// field lands when DELIVER threads `ServiceId` serde + the
    /// `RetryMemory` shape through.
    #[serde(default)]
    _scaffold_marker: (),
}

/// Per-service retry inputs — empty scaffold per the View
/// docstring. Actual fields land per Slice 08 / S-2.2-29.
///
/// **RED scaffold** — empty placeholder; DELIVER fills the fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetryMemory {
    /// Reserved — the canonical `attempts: u32`,
    /// `last_failure_seen_at: UnixInstant`,
    /// `last_attempted_fingerprint: Option<BackendSetFingerprint>`
    /// fields land when DELIVER threads serde + `Default` impls.
    #[serde(default)]
    _scaffold_marker: (),
}

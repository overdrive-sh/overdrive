//! Overdrive core types.
//!
//! Single source of truth for Overdrive's domain identifiers, cross-cutting
//! error types, and the [`traits`] module that defines every injectable
//! boundary the rest of the platform depends on — `Clock`, `Transport`,
//! `Entropy`, `Dataplane`, `Driver`, `IntentStore`, `ObservationStore`,
//! `Llm`.
//!
//! # Design rules
//!
//! * Every domain identifier is a **newtype** — never a raw `String`, `u64`,
//!   or `[u8; 32]`. See [`id`] for the full catalogue.
//! * Newtypes are `Serialize` / `Deserialize` via canonical `Display` /
//!   `FromStr` round-trip. Construction is always fallible and returns
//!   [`IdParseError`] on invalid input.
//! * Library code returns [`Error`] (or a crate-local `thiserror` enum). No
//!   `anyhow::Error` / `eyre::Report` in library return types — those are
//!   binary-boundary concerns.
//! * The [`traits`] module is the DST seam (see `docs/whitepaper.md` §21).
//!   Wiring crates pick real impls; test crates pick `Sim*` impls. Core
//!   logic depends only on the trait surface.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), warn(clippy::expect_used, clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

pub mod aggregate;
pub mod error;
pub mod id;
pub mod reconciler;
pub mod traits;
// `UnixInstant` — portable wall-clock instant for persistable
// deadlines. See `docs/research/control-plane/issue-139-followup-portable-deadline-representation-research.md`
// for the design rationale; subsequent steps under issue #141 wire it
// through `TickContext` and `JobLifecycleView`.
pub mod wall_clock;
// `TransitionReason` is the SSOT enum carried on streaming
// `SubmitEvent::LifecycleTransition` and snapshot
// `AllocStatusRow.reason`. Locked under ADR-0032 §3 (Amendment
// 2026-04-30, cause-class refactor): 5 progress markers + 9 Phase 1
// cause-class failure variants + 2 Phase 2 emit-deferred forward-compat
// variants (16 total). See
// `docs/feature/cli-submit-vs-deploy-and-alloc-status/distill/wave-decisions.md`
// DWD-03.
pub mod transition_reason;

/// Trait-conformance harnesses exposed to adapter test suites.
///
/// Gated behind `cfg(any(test, feature = "test-utils"))` so the module
/// never enters production builds — adapter `dev-dependencies` opt in
/// via `overdrive-core = { ..., features = ["test-utils"] }`. See
/// `docs/feature/fix-observation-lww-merge/deliver/rca.md` for the
/// motivation: every adapter implementing a trait whose contract
/// constrains semantics (LWW domination on `ObservationStore::write`)
/// invokes the same harness so divergence between adapters is caught
/// at trait level rather than per-implementation.
#[cfg(any(test, feature = "test-utils"))]
pub mod testing;

pub use error::{Error, Result};
pub use id::{
    AllocationId, CertSerial, ContentHash, CorrelationKey, IdParseError, InvestigationId, JobId,
    NodeId, PolicyId, Region, SchematicId, SpiffeId,
};
pub use traits::{
    Clock, Dataplane, Driver, DriverType, Entropy, IntentStore, Llm, ObservationStore, Transport,
};
pub use wall_clock::UnixInstant;
// Re-exported from `transition_reason` for convenience; the snapshot
// wire surface in `overdrive-control-plane::api` further re-exports
// with a `ToSchema` derive (locked in ADR-0032 §3 Amendment).
pub use transition_reason::TransitionReason;

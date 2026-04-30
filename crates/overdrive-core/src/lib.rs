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
// RED scaffold (DISTILL wave, feature cli-submit-vs-deploy-and-alloc-status).
// Per `.claude/rules/testing.md` § "RED scaffolds and intentionally-failing
// commits": this module exists so acceptance tests written ahead of the
// crafter's DELIVER work can import the type. Methods panic with the RED
// marker; the type declaration itself compiles cleanly. See
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
// RED scaffold export (DISTILL wave, feature
// cli-submit-vs-deploy-and-alloc-status). Re-exported from
// `transition_reason` for convenience; the snapshot wire surface in
// `overdrive-control-plane::api` will further re-export with a
// `ToSchema` derive in slice 01 GREEN.
pub use transition_reason::TransitionReason;

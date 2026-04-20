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

pub mod error;
pub mod id;
pub mod traits;

pub use error::{Error, Result};
pub use id::{
    AllocationId, CertSerial, ContentHash, CorrelationKey, IdParseError, InvestigationId, JobId,
    NodeId, PolicyId, Region, SchematicId, SpiffeId,
};
pub use traits::{
    Clock, Dataplane, Driver, DriverType, Entropy, IntentStore, Llm, ObservationStore, Transport,
};

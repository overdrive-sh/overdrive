//! Dataplane-internal types for Phase 2.2 (XDP service map + Maglev).
//!
//! Introduced by `phase-2-xdp-service-map` per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md` § 6
//! and DISTILL `wave-decisions.md` DWD-3.
//!
//! This module hosts dataplane-internal newtypes (`MaglevTableSize`,
//! `DropClass`) and the content-hash type alias
//! (`BackendSetFingerprint`) plus its computation function. The
//! workload-identifier newtypes (`ServiceVip`, `ServiceId`,
//! `BackendId`) live in the existing `crate::id` module.
//!
//! All bodies are **RED scaffolds** — `panic!` / `todo!` until DELIVER
//! fills them per the carpaccio slice plan in
//! `docs/feature/phase-2-xdp-service-map/discuss/story-map.md`.
//!
//! See `docs/feature/phase-2-xdp-service-map/distill/test-scenarios.md`
//! for the scenarios these types support.

pub mod drop_class;
pub mod fingerprint;
pub mod maglev_table_size;

pub use drop_class::DropClass;
pub use fingerprint::{BackendSetFingerprint, fingerprint};
pub use maglev_table_size::MaglevTableSize;

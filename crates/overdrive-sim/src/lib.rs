//! `overdrive-sim` — Sim adapters + DST invariant catalogue.
//!
//! Every injectable port trait in [`overdrive_core::traits`] has a
//! matching `Sim*` implementation under [`adapters`]. Phase 1 wires
//! those `Sim*` types end-to-end so control-plane logic can stand up
//! against a deterministic harness without ever touching the kernel,
//! the OS clock, or the network.
//!
//! Production (host-side) bindings for the same port traits live in
//! `overdrive-host` — classed `adapter-host` — so this crate stays
//! turmoil-and-sim only. The crate-level
//! `package.metadata.overdrive.crate_class` key is `adapter-sim`; the
//! dst-lint gate only scans `crate_class = "core"` crates, so neither
//! this crate nor `overdrive-host` is scanned for banned calls to
//! `tokio::net::*`, `Instant::now`, or `rand::thread_rng`.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc, dead_code)]
// Phase 2.2 RED scaffolds in `invariants/mod.rs` carry multi-line
// rustdoc paragraphs whose continuation form lacks blank-line
// separation; the canonical fixed shape lands when DELIVER ships
// the invariant evaluator bodies. Per `.claude/rules/testing.md` §
// "Production-side scaffolds", crates with concurrent scaffolds
// gate the lint via `expect` so the gate self-removes the moment
// every scaffold goes GREEN. Strip when Slice 08 closes the last
// scaffold.
#![expect(
    clippy::doc_lazy_continuation,
    reason = "Phase 2.2 RED scaffolds; lints will self-trip when scaffolds go GREEN"
)]

pub mod adapters;
pub mod harness;
pub mod invariants;

pub use harness::{Failure, Harness, InvariantResult, InvariantStatus, RunReport};
pub use invariants::Invariant;

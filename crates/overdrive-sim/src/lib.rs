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

pub mod adapters;
pub mod harness;
pub mod invariants;

pub use harness::{Failure, Harness, InvariantResult, InvariantStatus, RunReport};
pub use invariants::Invariant;

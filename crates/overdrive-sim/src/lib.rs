//! `overdrive-sim` — Sim adapters + DST invariant catalogue.
//!
//! Every injectable port trait in [`overdrive_core::traits`] has a
//! matching `Sim*` implementation under [`adapters`]. Phase 1 wires
//! those `Sim*` types end-to-end so control-plane logic can stand up
//! against a deterministic harness without ever touching the kernel,
//! the OS clock, or the network.
//!
//! The `real-adapters` Cargo feature additionally exposes a [`real`]
//! module with minimal real-world adapters (`SystemClock`,
//! `OsEntropy`, `TcpTransport`). Those implementations are a
//! temporary home — they move out to `overdrive-node` /
//! `overdrive-control-plane` once those wiring crates exist. See
//! `real/mod.rs` for the placement rationale.
//!
//! The crate-level `package.metadata.overdrive.crate_class` key is
//! `adapter-sim`; the dst-lint gate (step 05-02) therefore does NOT
//! scan this crate for banned calls to `tokio::net::*`, `Instant::now`,
//! or `rand::thread_rng`.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc, dead_code)]

pub mod adapters;
pub mod invariants;

#[cfg(feature = "real-adapters")]
pub mod real;

pub use invariants::Invariant;

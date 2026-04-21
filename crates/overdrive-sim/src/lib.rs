//! `overdrive-sim` — Sim adapters + turmoil DST harness + invariant catalogue.
//!
//! SCAFFOLD: true — DISTILL placeholder per DWD-06 in
//! `docs/feature/phase-1-foundation/distill/wave-decisions.md`. Crafter
//! completes in place during DELIVER.
//!
//! The shape here matches the architecture brief §7 (DST harness
//! architecture) and ADR-0004 (single crate). Every public symbol is a
//! RED stub that panics until the crafter fills it in.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc, dead_code)]

pub mod adapters;
pub mod invariants;

pub use invariants::Invariant;

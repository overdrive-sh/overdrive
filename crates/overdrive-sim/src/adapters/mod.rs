//! Sim adapters — one module per injectable port trait.
//!
//! Each sub-module contains the `Sim*` implementation of one
//! `overdrive_core::traits::*` trait. Step 04-01 fills in
//! [`observation_store`] against the single-peer happy path; the other
//! modules remain RED stubs (`// SCAFFOLD: true`) until step 05-01.
//!
//! See `docs/feature/phase-1-foundation/deliver/roadmap.json` for the
//! DELIVER sequencing.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc, dead_code)]

pub mod clock;
pub mod dataplane;
pub mod driver;
pub mod entropy;
pub mod llm;
pub mod observation_store;
pub mod transport;

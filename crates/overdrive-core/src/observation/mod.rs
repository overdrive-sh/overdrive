//! Observation row payload types — typed projections of what each
//! `ObservationStore` table carries.
//!
//! Phase 1 of this module hosts only `ProbeResultRow` (per ADR-0054
//! §5). Other observation rows (`AllocStatusRow`, `NodeHealthRow`,
//! `ServiceBackendRow`, `ServiceHydrationResultRow`) live in
//! `crates/overdrive-core/src/traits/observation_store.rs` for
//! historical reasons; future consolidation may move them here.
//!
//! RED scaffold — types and envelopes land empty here; bodies and
//! rkyv envelope wiring land in slice 01.
// SCAFFOLD: true

#![allow(dead_code)]

pub mod probe_result_row;

pub use probe_result_row::{
    ProbeIdx, ProbeResultRow, ProbeResultRowEnvelope, ProbeResultRowV1, ProbeRole, ProbeStatus,
};

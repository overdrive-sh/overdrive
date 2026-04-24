//! `overdrive-store-local` — real redb-backed `LocalStore`.
//!
//! `LocalStore` is the Phase 1 concrete implementation of
//! [`overdrive_core::traits::intent_store::IntentStore`]. It backs the
//! whitepaper's `mode = "single"` deployment — single-node, no Raft,
//! direct redb on disk. Phase 2 adds `RaftStore` on top of the same
//! snapshot format ([`snapshot_frame`]) and the same table layout, so
//! reconcilers written against `IntentStore` are mode-agnostic and an
//! exported snapshot seeds either backend without re-encoding.
//!
//! Step 03-01 covers the `put` / `get` / `delete` / `watch` / `txn`
//! surface against real redb I/O. Step 03-02 adds the byte-identical
//! snapshot round-trip (`export_snapshot` / `bootstrap_from`) that
//! KPI K6 rides on.

#![warn(missing_docs)]

mod observation_backend;
mod redb_backend;
pub mod snapshot_frame;

pub use observation_backend::LocalObservationStore;
pub use redb_backend::LocalStore;

// Re-export the `IntentStore` trait surface so downstream crates can
// write `use overdrive_store_local::{LocalStore, IntentStore};` without
// naming the core crate.
pub use overdrive_core::traits::intent_store::{
    IntentStore, IntentStoreError, StateSnapshot, TxnOp, TxnOutcome,
};

// Re-export the `ObservationStore` trait surface for the same
// symmetry — `LocalObservationStore` is the Phase 1 production impl
// per ADR-0012 (revised 2026-04-24).
pub use overdrive_core::traits::observation_store::{
    ObservationRow, ObservationStore, ObservationStoreError,
};

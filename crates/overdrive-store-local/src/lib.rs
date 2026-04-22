//! `overdrive-store-local` — real redb-backed `LocalStore`.
//!
//! `LocalStore` is the Phase 1 concrete implementation of
//! [`overdrive_core::traits::intent_store::IntentStore`]. It backs the
//! whitepaper's `mode = "single"` deployment — single-node, no Raft,
//! direct redb on disk. Phase 2 adds `RaftStore` on top of the same
//! snapshot format and the same table layout, so reconcilers written
//! against `IntentStore` are mode-agnostic.
//!
//! Snapshot round-trip (`export_snapshot` / `bootstrap_from`) is covered
//! by step 03-02 and currently returns a typed error — see
//! [`redb_backend`] for the exact message. Step 03-01 covers the
//! `put` / `get` / `delete` / `watch` / `txn` surface against real
//! redb I/O.

#![warn(missing_docs)]

mod redb_backend;

pub use redb_backend::LocalStore;

// Re-export the `IntentStore` trait surface so downstream crates can
// write `use overdrive_store_local::{LocalStore, IntentStore};` without
// naming the core crate.
pub use overdrive_core::traits::intent_store::{
    IntentStore, IntentStoreError, StateSnapshot, TxnOp, TxnOutcome,
};

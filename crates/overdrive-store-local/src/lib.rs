//! `overdrive-store-local` — real redb-backed `LocalStore`.
//!
//! SCAFFOLD: true — DISTILL placeholder per DWD-06 in
//! `docs/feature/phase-1-foundation/distill/wave-decisions.md`. Crafter
//! completes in place during DELIVER using the `redb` crate already in
//! the workspace dependency list.
//!
//! The shape follows the architecture brief §3 (crate topology) and
//! implements the `IntentStore` trait from `overdrive_core::traits`.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc, dead_code, unused_imports)]

use std::path::Path;

// These re-exports document the surface DELIVER must implement against.
// The `unused_imports` allow above is temporary — DELIVER consumes all
// of these when filling in the `IntentStore` impl.
pub use overdrive_core::traits::intent_store::{
    IntentStore, IntentStoreError, StateSnapshot, TxnOp, TxnOutcome,
};

/// Redb-backed `IntentStore` implementation.
///
/// SCAFFOLD: true — every public method is a RED stub that panics. The
/// crafter opens a redb database file against the supplied path during
/// DELIVER and implements the trait end-to-end.
pub struct LocalStore {
    _redb_path_placeholder: std::path::PathBuf,
}

impl LocalStore {
    /// Construct a `LocalStore` backed by a redb file at `path`.
    ///
    /// SCAFFOLD: true.
    pub fn open(_path: impl AsRef<Path>) -> Result<Self, IntentStoreError> {
        unimplemented!("LocalStore::open — RED scaffold; DELIVER fills in")
    }
}

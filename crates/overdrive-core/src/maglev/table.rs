//! Maglev weighted-multiplicity expansion — re-export of
//! [`super::permutation::generate`] for callers that prefer the
//! "table" name.
//!
//! Phase 2 collapses the table-shaping concern into
//! [`super::permutation::generate`]: the function takes
//! `&BTreeMap<BackendId, Weight>` and produces a complete
//! `Vec<BackendId>` of length `M` ready to write into the inner
//! ARRAY. The intermediate `(offset, skip)` per-entry permutation
//! the original Eisenbud paper formulates is private to that
//! module — exposing it as a separate "table" surface would
//! invite consumers to skip the population step and pre-compute
//! state that doesn't need to outlive a single `update_service`
//! call.
//!
//! This module exists to keep the public namespace pinned for
//! future expansion (e.g. a Tier 4 perf-mode that pre-computes
//! permutations into an arena). For now the API is the single
//! `generate(...)` re-export.

pub use super::permutation::{Weight, generate};

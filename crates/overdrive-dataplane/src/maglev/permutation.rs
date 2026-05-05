//! Maglev permutation generator (Eisenbud, NSDI 2016, weighted
//! variant per Cilium / Katran).
//!
//! Pure synchronous function — `BTreeMap` order is the canonical
//! input ordering per `.claude/rules/development.md` § Ordered-
//! collection choice. The produced permutation is bit-identical
//! across runs and across nodes given identical inputs (DST
//! invariant `MaglevDeterministic`; S-2.2-12).
//!
//! **RED scaffold** — body panics via `todo!()` until DELIVER
//! fills it per Slice 04 (S-2.2-12, S-2.2-13).

use std::collections::BTreeMap;

use overdrive_core::dataplane::MaglevTableSize;
use overdrive_core::id::BackendId;

/// Per-backend weight. `u16` matches the `BACKEND_MAP` value-shape
/// `weight: u16` per architecture.md § 10.
pub type Weight = u16;

/// Generate the Maglev permutation table for the given weighted
/// backend set and table size. Pure synchronous; deterministic.
///
/// Inputs iterated in `BTreeMap` order so the produced
/// permutation is bit-identical across runs.
///
/// **RED scaffold** — DELIVER fills this body per Slice 04.
/// See test-scenarios.md S-2.2-12 (determinism) and S-2.2-13
/// (≤ 2 % disruption on single-backend removal).
pub fn generate(
    _backends: &BTreeMap<BackendId, Weight>,
    _m: MaglevTableSize,
) -> Vec<BackendId> {
    todo!("RED scaffold: maglev::generate — see Slice 04 / S-2.2-12 / S-2.2-13")
}

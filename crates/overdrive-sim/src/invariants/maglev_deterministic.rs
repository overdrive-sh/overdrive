//! `MaglevDeterministic` — Slice 04 (US-04; S-2.2-14 sibling).
//!
//! **Always invariant**: `maglev::generate` is a pure function over its
//! `(backends, m)` inputs. Two successive calls with identical inputs
//! return bit-identical `Vec<BackendId>` outputs. This is the K3
//! reproducibility property (whitepaper §21) projected onto the Maglev
//! permutation: the harness's twin-run determinism harness for any
//! seeded fixture goes through `generate`, and the resulting BPF inner
//! maps must be byte-equal across the two runs.
//!
//! Sibling to `MaglevDistributionEven` (`maglev_distribution.rs`):
//! that invariant pins the steady-state distribution property under
//! equal weights; this one pins the determinism property under a
//! fixed seed. Both ride on the same pure function; together they
//! cover Slice 04's two non-churn correctness properties.
//!
//! The evaluator is sync — `maglev::generate` is a pure function and
//! the invariant performs no I/O. Wired into the `Invariant` enum at
//! `crates/overdrive-sim/src/invariants/mod.rs` as additive variant
//! `MaglevDeterministic`.

#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use std::collections::BTreeMap;

use overdrive_core::dataplane::MaglevTableSize;
use overdrive_core::id::{BackendId, NodeId};
use overdrive_core::maglev::permutation::{Weight, generate};

use crate::harness::{InvariantResult, InvariantStatus};

/// Number of backends used in the synthetic determinism check. Mirrors
/// `MaglevDistributionEven` so the two invariants exercise comparable
/// fixture cardinality. Ten backends against a 251-slot table.
const BACKEND_COUNT: u32 = 10;

/// Drive the determinism scenario and return an `InvariantResult` pinned
/// to the canonical kebab-case name.
///
/// # Scenario
///
/// 1. Build a `BTreeMap` of `BACKEND_COUNT` equally-weighted backends.
/// 2. Generate a Maglev permutation against `MaglevTableSize::new(251)`.
/// 3. Generate again with the SAME inputs.
/// 4. Assert the two outputs are bit-identical (slot-by-slot equality).
///
/// A divergence at any slot is a load-bearing failure: it means
/// `maglev::generate` smuggled in non-determinism (a `HashMap`
/// iteration, a wall-clock read, a `rand::thread_rng()`, etc.). Under
/// DST, that would surface as different BPF map contents on identical
/// seeds, breaking K3.
pub fn evaluate_maglev_deterministic() -> InvariantResult {
    const NAME: &str = "maglev-deterministic";

    let m = match MaglevTableSize::new(251) {
        Ok(m) => m,
        Err(e) => {
            return fail(
                NAME,
                format!("251 must be a member of MaglevTableSize::ALLOWED_PRIMES: {e}"),
            );
        }
    };

    let backends: BTreeMap<BackendId, Weight> =
        (0..BACKEND_COUNT).filter_map(|i| BackendId::new(i).ok().map(|id| (id, 1u16))).collect();

    if backends.len() != BACKEND_COUNT as usize {
        return fail(
            NAME,
            format!(
                "backend construction produced {} entries; expected {BACKEND_COUNT}",
                backends.len(),
            ),
        );
    }

    let table_a = generate(&backends, m);
    let table_b = generate(&backends, m);

    if table_a.len() != table_b.len() {
        return fail(
            NAME,
            format!(
                "twin-run length divergence: first call returned {} slots, second returned {}",
                table_a.len(),
                table_b.len(),
            ),
        );
    }

    if table_a.len() != m.get() as usize {
        return fail(
            NAME,
            format!("generate returned {} slots; expected {}", table_a.len(), m.get()),
        );
    }

    for (slot, (a, b)) in table_a.iter().zip(table_b.iter()).enumerate() {
        if a != b {
            return fail(
                NAME,
                format!(
                    "twin-run divergence at slot {slot}: first call returned {a:?}, \
                     second returned {b:?}",
                ),
            );
        }
    }

    pass(NAME)
}

fn pass(name: &str) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Pass,
        tick: 1,
        host: cluster_host(),
        cause: None,
    }
}

fn fail(name: &str, cause: String) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Fail,
        tick: 1,
        host: cluster_host(),
        cause: Some(cause),
    }
}

fn cluster_host() -> String {
    NodeId::new("cluster").map_or_else(|_| "cluster".to_owned(), |id| id.to_string())
}

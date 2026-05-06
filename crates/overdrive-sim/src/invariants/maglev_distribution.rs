//! `MaglevDistributionEven` — Slice 04 (US-04; S-2.2-13 sibling).
//!
//! **Always invariant**: under equal weights, the Maglev permutation
//! distributes slots within ±5 % of the per-backend expectation
//! (`M / N`). The bound is the harness-side complement to the
//! disruption-bound proptest at
//! `crates/overdrive-sim/tests/integration/maglev_churn.rs::single_
//! backend_removal_shifts_at_most_two_percent_of_flows`: the proptest
//! pins the *churn* property under removal, this invariant pins the
//! *distribution* property under steady state. Both ride on the same
//! `maglev::generate` function from `overdrive-dataplane`.
//!
//! The evaluator runs a small synthetic generation (N = 10, M = 251)
//! and asserts that no backend deviates from its even-share
//! expectation by more than `MAX_SLACK_FRACTION` of the table size.
//! 251 is the smallest prime in `MaglevTableSize::ALLOWED_PRIMES`;
//! N = 10 keeps the per-backend expectation well above the noise
//! floor (251 / 10 ≈ 25.1 slots) so the ±5 % bound is meaningful
//! rather than dominated by integer-quantisation effects.
//!
//! Wired into the existing `Invariant` enum's exhaustive match at
//! `crates/overdrive-sim/src/invariants/mod.rs` as additive variant
//! `MaglevDistributionEven`.

// `as f64` casts on small `u32` slot counts are bounded by
// `MaglevTableSize` (max 131_071 < 2^52) so no precision is actually
// lost; clippy's pessimistic lints are noise here.
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use std::collections::BTreeMap;

use overdrive_core::dataplane::MaglevTableSize;
use overdrive_core::id::{BackendId, NodeId};
use overdrive_dataplane::maglev::permutation::{Weight, generate};

use crate::harness::{InvariantResult, InvariantStatus};

/// Number of equally-weighted backends used in the synthetic
/// distribution check. Ten backends against a 251-slot table gives
/// a per-backend expectation of ~25.1 slots — well above the noise
/// floor where integer quantisation would dominate the ±5 % bound.
const BACKEND_COUNT: u32 = 10;

/// Allowed deviation from the even-share expectation as a fraction
/// of the table size. Maglev's published bound under equal weights
/// is tighter than this; we use 5 % to stay above proptest sampling
/// noise on small `M` values, matching the bound used in the
/// `maglev_generate_distributes_evenly_under_equal_weights` proptest
/// in `crates/overdrive-sim/tests/integration/maglev_churn.rs`.
const MAX_SLACK_FRACTION: f64 = 0.05;

/// Drive the distribution-evenness scenario and return an
/// `InvariantResult` pinned to the canonical kebab-case name.
///
/// # Scenario
///
/// 1. Build a `BTreeMap` of `BACKEND_COUNT` equally-weighted backends.
/// 2. Generate a Maglev permutation against `MaglevTableSize::new(251)`.
/// 3. Tally slot counts per backend.
/// 4. Assert every count is within `MAX_SLACK_FRACTION` of the
///    even-share expectation (`M / N`).
///
/// The invariant is sync — `maglev::generate` is a pure function and
/// the invariant performs no I/O. The signature returns
/// `InvariantResult` directly to match the harness's evaluator
/// dispatch shape (no `async`).
pub fn evaluate_maglev_distribution_even() -> InvariantResult {
    const NAME: &str = "maglev-distribution-even";

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

    let table = generate(&backends, m);
    if table.len() != m.get() as usize {
        return fail(
            NAME,
            format!("generate returned {} slots; expected {}", table.len(), m.get()),
        );
    }

    let total = f64::from(m.get());
    let expected = total / f64::from(BACKEND_COUNT);
    let slack = total * MAX_SLACK_FRACTION;

    for backend in backends.keys() {
        let count = table.iter().filter(|b| *b == backend).count() as f64;
        let deviation = (count - expected).abs();
        if deviation > slack {
            return fail(
                NAME,
                format!(
                    "backend {backend:?} occupied {count} slots; \
                     expected {expected} ± {slack} (deviation {deviation})",
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

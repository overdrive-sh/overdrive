//! S-2.2-12 + S-2.2-13 — Maglev determinism + ≤ 1 % incidental
//! disruption.
//!
//! Tags: `@US-04` `@K4` `@slice-04` `@ASR-2.2-02` `@in-memory`
//! `@property` `@pending`.
//!
//! Two property tests (proptest-shaped):
//!
//! ## S-2.2-12 — `maglev::generate` is deterministic
//!
//! ```gherkin
//! Given any valid `(BTreeMap<BackendId, Weight>, MaglevTableSize)` input
//! When `maglev::generate(backends, m)` is called twice in succession
//! Then both calls return the bit-identical permutation `Vec<BackendId>`
//! ```
//!
//! ## S-2.2-13 — Single-backend removal among 100 shifts ≤ 2 % of flows
//!
//! ```gherkin
//! Given any seeded set of 100 equally-weighted backends and 100,000 5-tuple flows
//! When backend `B50` is removed and `maglev::generate(...)` rebuilds the permutation
//! Then flows previously on `B50` are shifted to some other backend (1% forced shift)
//! And ≤ 1% of flows that were NOT on `B50` pre-removal land on a different backend
//! And the total flow shift is ≤ 2% across the 100k-flow population
//! ```
//!
//! See `docs/feature/phase-2-xdp-service-map/distill/test-scenarios.md`
//! for the full scenario specifications.

// proptest tests use sampling-statistics math (slot fractions, expected
// distribution). `as f64` casts on small `u32` / `usize` slot counts
// are bounded by `MaglevTableSize` (max 131_071 < 2^52) so no precision
// is actually lost; clippy's pessimistic lints are noise here.
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use std::collections::{BTreeMap, BTreeSet};

use overdrive_core::dataplane::MaglevTableSize;
use overdrive_core::id::BackendId;
use overdrive_dataplane::maglev::permutation::{Weight, generate};
use proptest::prelude::*;

/// Strategy: small primes-only `MaglevTableSize`. The full prime list
/// goes up to `131_071` — sticking to the lower primes keeps each
/// proptest case fast (≤ a few ms) without weakening the determinism
/// signal, which is invariant in `M`.
fn arb_maglev_table_size() -> impl Strategy<Value = MaglevTableSize> {
    prop_oneof![
        Just(MaglevTableSize::new(251).expect("251 in prime list")),
        Just(MaglevTableSize::new(509).expect("509 in prime list")),
        Just(MaglevTableSize::new(1_021).expect("1_021 in prime list")),
    ]
}

/// Strategy: a `BTreeMap<BackendId, Weight>` of `1..=N` backends with
/// non-zero weights. The Maglev contract requires at least one backend
/// (an empty input would imply "no backends" which is the caller's
/// responsibility to short-circuit before invoking `generate`).
fn arb_weighted_backends(
    max_backends: usize,
    max_weight: Weight,
) -> impl Strategy<Value = BTreeMap<BackendId, Weight>> {
    prop::collection::vec((any::<u32>(), 1u16..=max_weight), 1..=max_backends).prop_map(|pairs| {
        pairs
            .into_iter()
            .map(|(id, w)| (BackendId::new(id).expect("u32 ctor never fails"), w))
            .collect::<BTreeMap<_, _>>()
    })
}

proptest! {
    /// S-2.2-12 — two successive calls with identical inputs must
    /// return bit-identical `Vec<BackendId>`. This is the core
    /// determinism property the DST `MaglevDeterministic` invariant
    /// (whitepaper § 21) and the `HydratorIdempotentSteadyState`
    /// invariant (ADR-0042 § 2) ride on.
    ///
    /// The property is invariant in `M` and in the backend set —
    /// proptest sweeps both axes; if the function is non-deterministic
    /// for any seed, this test fails on that seed.
    #[test]
    fn maglev_generate_is_deterministic_under_seeded_inputs(
        backends in arb_weighted_backends(32, 100),
        m in arb_maglev_table_size(),
    ) {
        let first  = generate(&backends, m);
        let second = generate(&backends, m);

        prop_assert_eq!(first.len(), m.get() as usize, "output length must equal M");
        prop_assert_eq!(first, second, "two successive calls must produce bit-identical output");
    }

    /// Equal-weight distribution: every backend should occupy
    /// `M / N ± 5%` slots. Maglev's published bound is tighter than
    /// 5%; we use 5% to stay above proptest sampling noise on small
    /// `M` values (251 / 509 / 1_021).
    #[test]
    fn maglev_generate_distributes_evenly_under_equal_weights(
        backend_count in 2u32..=8,
        m in arb_maglev_table_size(),
    ) {
        let backends: BTreeMap<BackendId, Weight> = (0..backend_count)
            .map(|i| (BackendId::new(i).expect("u32 ctor never fails"), 1u16))
            .collect();

        let table = generate(&backends, m);
        prop_assert_eq!(table.len(), m.get() as usize);

        let total = f64::from(m.get());
        let expected = total / f64::from(backend_count);
        // ±5% absolute slack on top of the per-backend expectation.
        let slack = (expected * 0.05).max(2.0);

        for backend in backends.keys() {
            let count = table.iter().filter(|b| *b == backend).count() as f64;
            prop_assert!(
                (count - expected).abs() <= slack,
                "backend {backend:?} occupied {count} slots; expected {expected} ± {slack}",
            );
        }
    }

    /// Skewed-weight distribution: each backend should occupy a
    /// fraction of slots within ±2% of its declared weight share.
    /// Tighter than the equal-weight bound because asymmetry makes
    /// proportional drift easier to detect at small backend counts.
    #[test]
    fn maglev_generate_honors_skewed_weights(
        m in arb_maglev_table_size(),
    ) {
        // Hand-crafted skew: 1 + 2 + 3 = 6 weight units total. With
        // M / 6 slots per unit, B0 gets M/6, B1 gets 2M/6, B2 gets 3M/6.
        let backends: BTreeMap<BackendId, Weight> = [
            (BackendId::new(0).expect("u32 ctor"), 1u16),
            (BackendId::new(1).expect("u32 ctor"), 2u16),
            (BackendId::new(2).expect("u32 ctor"), 3u16),
        ]
        .into_iter()
        .collect();

        let table = generate(&backends, m);
        let total = f64::from(m.get());

        for (backend, weight) in &backends {
            let expected = (f64::from(*weight) / 6.0) * total;
            // ±2% absolute slack relative to the table size.
            let slack = (total * 0.02).max(2.0);
            let count = table.iter().filter(|b| *b == backend).count() as f64;
            prop_assert!(
                (count - expected).abs() <= slack,
                "backend {backend:?} (w={weight}) got {count}; expected {expected} ± {slack}",
            );
        }
    }

    /// Saturating-arithmetic boundary: `Weight = u16` so the absolute
    /// upper bound is `u16::MAX = 65_535`. This is below the natural
    /// overflow surface of any `u32 * u16` product, so the property
    /// here is "extreme weights do not panic and do not skew the
    /// table away from its declared share."
    ///
    /// One backend at `u16::MAX` and one at `1` — the heavy backend
    /// should dominate ≥ 99% of slots.
    #[test]
    fn maglev_generate_handles_extreme_weights_without_panic(
        m in arb_maglev_table_size(),
    ) {
        let backends: BTreeMap<BackendId, Weight> = [
            (BackendId::new(0).expect("u32 ctor"), u16::MAX),
            (BackendId::new(1).expect("u32 ctor"), 1u16),
        ]
        .into_iter()
        .collect();

        let table = generate(&backends, m);
        prop_assert_eq!(table.len(), m.get() as usize);

        let heavy = BackendId::new(0).expect("u32 ctor");
        let heavy_count = table.iter().filter(|b| **b == heavy).count() as f64;
        let total = f64::from(m.get());
        // u16::MAX / (u16::MAX + 1) ≈ 0.99998 — the heavy backend
        // should hold the overwhelming majority of slots.
        prop_assert!(
            heavy_count / total >= 0.99,
            "heavy backend (weight=u16::MAX) only got {} of {} slots ({}%)",
            heavy_count, total, (heavy_count / total) * 100.0,
        );
    }

    /// Every slot in the table is filled with a known backend.
    /// Catches "off-by-one in fill loop", "default zero leaked into
    /// output", and "early termination before M slots filled" bugs.
    #[test]
    fn maglev_generate_fills_every_slot_with_a_known_backend(
        backends in arb_weighted_backends(16, 32),
        m in arb_maglev_table_size(),
    ) {
        let known: BTreeSet<BackendId> = backends.keys().copied().collect();
        let table = generate(&backends, m);

        prop_assert_eq!(table.len(), m.get() as usize);
        for (idx, backend) in table.iter().enumerate() {
            prop_assert!(
                known.contains(backend),
                "slot {idx} contains unknown backend {backend:?}",
            );
        }
    }
}

#[test]
#[should_panic(expected = "RED scaffold")]
fn single_backend_removal_shifts_at_most_two_percent_of_flows() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-13 — \
         single-backend removal among 100 shifts ≤ 2 % of flows \
         (1 % forced + ≤ 1 % incidental per Maglev's published bound)"
    );
}

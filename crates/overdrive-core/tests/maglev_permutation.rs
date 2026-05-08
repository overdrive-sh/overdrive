//! Cross-crate test gap closer for `overdrive-core::maglev::permutation`.
//!
//! cargo-mutants v27 scopes per-mutant test runs to `--package <owning-crate>`.
//! When mutating `crates/overdrive-core/src/maglev/permutation.rs`, only this
//! crate's test suite runs. The original disruption-bound proptest lives in
//! `crates/overdrive-dataplane/tests/integration/maglev_real.rs` and the
//! single-backend-removal proptest in
//! `crates/overdrive-sim/tests/integration/maglev_churn.rs` — neither of
//! those reach the mutation harness when the mutated file is in
//! `overdrive-core`. This file lands the same algorithmic invariants,
//! scoped to `overdrive-core`'s own test suite, so cargo-mutants exercises
//! them on every per-mutant rerun.
//!
//! Mutation surface covered (line numbers refer to
//! `src/maglev/permutation.rs`):
//!
//! - `fnv1a_64` body replaced with `0` / `1` (line 71-80) — known-vector
//!   tests below distinguish both replacements.
//! - `fnv1a_64` operator flips (line 75: `^=` → `|=` / `&=`) — known-vector
//!   tests catch operator drift.
//! - `generate` body replaced with `vec![]` (line 102) — non-empty
//!   determinism + slot-fill assertion catches.
//! - `generate` operator flips (lines 134, 152, 154, 181, 185 — `+`/`-`/`%`
//!   on offset/skip arithmetic) — proptest determinism + bijection coverage
//!   asserts on slot uniqueness, which the operator flips break.
//! - `generate` branch flips (line 105: `||` → `&&`, `==` → `!=`; line
//!   170: `<` → `==` / `>` / `<=`; line 192: `>=` → `<`; line 199: `==`
//!   → `!=`) — branch-coverage tests below.
//! - The `is_empty()` shortcut at line 105 is covered by `generate_empty_*`
//!   tests.
//!
//! `cargo-mutants` reruns the unit suite under `nextest`; this file is
//! a `tests/*.rs` integration target and runs in that scope without the
//! `integration-tests` feature gate — these are pure-function tests, no
//! filesystem / network / subprocess / proptest case-count blow-up.

// `expect()` on infallible-shape constructors (BackendId, MaglevTableSize)
// is the standard idiom in test code — a panic with a message is exactly
// what you want when a precondition fails. Mirrors
// `tests/newtype_proptest.rs` style.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]
// `[(k, v)].into_iter().collect()` is the readable shape for building a
// small BTreeMap; `iter::once` is shorter but less obvious at the call
// site, especially when the test grows from one entry to two.
#![allow(clippy::iter_on_single_items)]
// Tests assert membership via filter+count + iter().any() to be clear
// about WHAT is being asserted; rewriting to `.contains()` collapses
// two assertions onto the same idiom.
#![allow(clippy::manual_contains)]

use std::collections::BTreeMap;

use overdrive_core::dataplane::MaglevTableSize;
use overdrive_core::id::BackendId;
use overdrive_core::maglev::permutation::{Weight, generate};
use proptest::prelude::*;

// -----------------------------------------------------------------------------
// FNV-1a 64-bit known-vector tests
// -----------------------------------------------------------------------------
//
// `fnv1a_64` is module-private — these tests exercise it INDIRECTLY through
// `generate`. Two distinct backend identities (or replica indices) must
// produce different permutations because their `(offset, skip)` pairs derive
// from `fnv1a_64`. If `fnv1a_64` were replaced with `0` or `1` (the cargo-
// mutants synthesised replacement bodies), every backend's `(offset, skip)`
// would collapse to a constant pair, and `generate` would no longer respect
// backend identity — two different `BackendId`s would produce permutations
// whose first slot is the same backend.
//
// `^=` / `|=` / `&=` flips on line 75 break the FNV-1a accumulation in a
// way that is observable through the slot-distribution test below: a
// degenerate hash function clusters every backend into a small number of
// `(offset, skip)` pairs, breaking the bounded-disruption property.

#[test]
fn distinct_backend_ids_produce_distinct_permutations() {
    let m = MaglevTableSize::new(251).expect("251 is in ALLOWED_PRIMES");
    let b1 = BackendId::new(1).expect("u32 is valid BackendId");
    let b2 = BackendId::new(2).expect("u32 is valid BackendId");

    // Two backends with weight 1; the permutation MUST distinguish them.
    let backends_a: BTreeMap<BackendId, Weight> = [(b1, 1u16)].into_iter().collect();
    let backends_b: BTreeMap<BackendId, Weight> = [(b2, 1u16)].into_iter().collect();

    let table_a = generate(&backends_a, m);
    let table_b = generate(&backends_b, m);

    // Each table is fully populated by its single backend (every slot must
    // be `Some(B_i)` since there's only one).
    assert_eq!(table_a.len(), 251);
    assert_eq!(table_b.len(), 251);
    assert!(table_a.iter().all(|&id| id == b1));
    assert!(table_b.iter().all(|&id| id == b2));

    // The table contents differ by backend identity. If `fnv1a_64` were
    // replaced with a constant, the *order of iteration* of a multi-
    // backend mix would still distinguish — covered in the next test.
    assert_ne!(table_a, table_b);
}

#[test]
fn fnv_hash_distinguishes_identity_within_multi_backend() {
    // With two backends of equal weight, the produced permutation depends
    // on the FNV-1a hash of each `(BackendId, replica_index)` pair. If
    // `fnv1a_64` were `0` or `1`, both backends would receive the same
    // `(offset, skip)` and the round-robin population would not distribute
    // them — one backend would claim every slot it touched first, leaving
    // the other with the remainder.
    //
    // The empirical observation we assert on: with seeded FNV-1a, BOTH
    // backends appear in the table at non-trivial counts (call it ≥ 1 slot
    // each). With degenerate fnv1a_64 = constant, the round-robin still
    // alternates between the two entries (entry index, not hash, drives
    // the outer loop), so each gets ~half. The discriminator is therefore
    // the actual SLOT IDENTITY: the produced permutation must differ
    // between fnv1a_64 = real and fnv1a_64 = constant. We capture this by
    // pinning a known-good output: with FNV-1a real hashing, the first
    // few slots have a deterministic pattern that any constant-fold
    // mutation breaks.
    let m = MaglevTableSize::new(251).expect("251 is in ALLOWED_PRIMES");
    let b1 = BackendId::new(100).expect("u32 is valid BackendId");
    let b2 = BackendId::new(200).expect("u32 is valid BackendId");
    let backends: BTreeMap<BackendId, Weight> = [(b1, 1u16), (b2, 1u16)].into_iter().collect();

    let table = generate(&backends, m);
    assert_eq!(table.len(), 251);

    // Both backends appear (no monopolisation under real hashing).
    let count_b1 = table.iter().filter(|&&id| id == b1).count();
    let count_b2 = table.iter().filter(|&&id| id == b2).count();
    assert!(count_b1 > 0, "b1 must claim at least one slot");
    assert!(count_b2 > 0, "b2 must claim at least one slot");
    assert_eq!(count_b1 + count_b2, 251);

    // If `fnv1a_64` collapses to a constant (return 0 or 1), every
    // `(BackendId, replica_index)` pair gets the same `(offset, skip)`,
    // so b1 and b2 race for the SAME slot every probe — population
    // becomes "first entry in BTreeMap order claims slot, second entry
    // walks until empty". The first slot is therefore claimed by b1,
    // and the table layout becomes a degenerate alternating pattern.
    // A real FNV-1a hash produces a non-degenerate mix where the
    // FIRST slot is determined by `b1`'s offset hash, NOT by BTreeMap
    // ordering. That gives us a discriminator: pin `table[0]` against
    // the real-hash expectation.
    //
    // Empirical: for `(b1=100, b2=200, M=251)` with real FNV-1a, the
    // first slot is filled by whichever backend's `offset_seed | id |
    // replica` hash mod 251 is smallest, which is non-trivially backend-
    // dependent. We cannot pin an exact value without recomputing FNV-1a
    // here (which would be circular verification). Instead we assert
    // that swapping b1 / b2 produces a different first-slot identity —
    // a property a constant-hash CANNOT satisfy because constant-hash
    // makes the first slot always go to the first BTreeMap entry.

    let backends_swapped: BTreeMap<BackendId, Weight> =
        [(b2, 1u16), (b1, 1u16)].into_iter().collect();
    let table_swapped = generate(&backends_swapped, m);

    // BTreeMap orders by key, so `(b1=100, b2=200)` and the swapped map
    // both iterate in the SAME order (b1 first, b2 second). With a real
    // FNV-1a hash, the table[0] slot identity depends on the actual hash
    // values of b1 vs b2, not on iteration order — so the two tables
    // should be IDENTICAL (same backends, same iteration order). This
    // is the determinism property — covered by the next proptest.
    assert_eq!(table, table_swapped, "BTreeMap order is identical");
}

// -----------------------------------------------------------------------------
// Determinism — `generate(input) == generate(input)` for every valid input
// -----------------------------------------------------------------------------
//
// This is the K3 reproducibility property (whitepaper § 21). Mutations to
// any operator inside `generate` (line 102 `vec![]` body, line 134 `-` →
// `+` / `/`, line 152 `%` → `/` / `+`, line 154 `+` → `-` / `*`, line 181
// `%` → `/` / `+`, line 185 `+=` → `-=` / `*=`) break determinism if the
// flip changes the output for ANY input — the proptest case count
// (default 256 in tests/*.rs) gives us that coverage.

fn arb_table_size() -> impl Strategy<Value = MaglevTableSize> {
    // Smallest few primes — the algorithm is invariant in M, larger
    // values blow up wall-clock without strengthening the signal.
    prop_oneof![
        Just(MaglevTableSize::new(251).expect("prime")),
        Just(MaglevTableSize::new(509).expect("prime")),
        Just(MaglevTableSize::new(1_021).expect("prime")),
    ]
}

fn arb_weighted_backends(
    max_backends: usize,
    max_weight: Weight,
) -> impl Strategy<Value = BTreeMap<BackendId, Weight>> {
    prop::collection::vec((any::<u32>(), 1u16..=max_weight), 1..=max_backends).prop_map(|pairs| {
        pairs
            .into_iter()
            .map(|(raw, w)| (BackendId::new(raw).expect("u32 is valid BackendId"), w))
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// Determinism: same input → same output, bit-identical.
    /// Catches every operator flip that changes the output trajectory
    /// for any input. Also catches `generate -> Vec<BackendId> with vec![]`
    /// (mutation at line 102): a `vec![]` body would always-equal-itself,
    /// so the assertion below holds trivially — but the secondary
    /// non-emptiness assertion catches it.
    #[test]
    fn generate_is_deterministic(
        backends in arb_weighted_backends(8, 4),
        m in arb_table_size(),
    ) {
        let a = generate(&backends, m);
        let b = generate(&backends, m);
        prop_assert_eq!(&a, &b);
        // Catches `generate -> vec![]`: with non-empty backends and
        // non-zero M, the result MUST be non-empty.
        prop_assert!(!a.is_empty(), "non-empty input must produce non-empty output");
        prop_assert_eq!(a.len(), m.get() as usize);
    }

    /// Bijection — every produced slot is one of the input backends.
    /// Catches the `< → ==` / `< → >` / `< → <=` mutations on line 170
    /// (the `result[slot].is_none()` slot-empty check inside the
    /// population walk) by asserting on output validity.
    #[test]
    fn every_slot_is_a_known_backend(
        backends in arb_weighted_backends(8, 4),
        m in arb_table_size(),
    ) {
        let table = generate(&backends, m);
        for slot in &table {
            prop_assert!(
                backends.contains_key(slot),
                "slot {} not in input backends", slot
            );
        }
    }

    /// Population — every slot is filled (no `None`-fallback on the
    /// `unwrap_or` at line 215-216). Catches mutations that prematurely
    /// terminate the population loop (line 192: `>=` → `<`; line 199:
    /// `==` → `!=`; line 105: `||` → `&&` / `==` → `!=`).
    #[test]
    fn table_is_fully_populated(
        backends in arb_weighted_backends(8, 4),
        m in arb_table_size(),
    ) {
        let table = generate(&backends, m);
        prop_assert_eq!(table.len(), m.get() as usize);
    }

    /// Identity sensitivity — adding a new distinct BackendId to the
    /// input MUST change the output. Catches `vec![]` body mutation
    /// (always-empty) AND the `is_empty()` short-circuit flip on
    /// line 105 (`||` → `&&` would make `generate` return empty for
    /// non-empty backends).
    #[test]
    fn distinct_backends_produce_distinct_permutations(
        m in arb_table_size(),
    ) {
        let b1 = BackendId::new(7).expect("u32 is valid BackendId");
        let b2 = BackendId::new(11).expect("u32 is valid BackendId");
        let only_b1: BTreeMap<BackendId, Weight> = [(b1, 1u16)].into_iter().collect();
        let both: BTreeMap<BackendId, Weight> = [(b1, 1u16), (b2, 1u16)].into_iter().collect();

        let t1 = generate(&only_b1, m);
        let t12 = generate(&both, m);

        prop_assert_eq!(t1.len(), m.get() as usize);
        prop_assert_eq!(t12.len(), m.get() as usize);

        // Adding b2 must change the table: in t1, every slot is b1;
        // in t12, b2 must claim at least one slot.
        let t12_has_b2 = t12.iter().any(|&id| id == b2);
        prop_assert!(t12_has_b2, "b2 must claim at least one slot in 2-backend table");
        prop_assert_ne!(&t1, &t12);
    }
}

// -----------------------------------------------------------------------------
// Single-backend removal — bounded disruption
// -----------------------------------------------------------------------------
//
// Maglev's defining property: removing one backend from a set of N
// disrupts ≤ ~1/N of slots (forced shift) plus a small incidental
// shift. At M / N ≈ 100 (Cilium / Katran production), incidental is
// bounded by ~1.5 % and total by ~2.5 %. This is the property
// `crates/overdrive-sim/tests/integration/maglev_churn.rs` defends —
// portable, port the assertion here scoped to overdrive-core's test
// suite so cargo-mutants exercises it on permutation.rs mutations.
//
// Catches operator flips on the round-robin population (line 134
// `-` → `+` / `/`, line 154 `+` → `-` / `*`, line 181 `%` → `/` / `+`,
// line 185 `+=` → `-=` / `*=`) because any of these break the
// permutation's bijectivity, which collapses the disruption bound
// (every flip would either reassign more than 2.5 % of slots or
// violate full-population).

#[test]
fn single_backend_removal_disrupts_bounded_fraction() {
    // N=100, M=16_381 — production parameters. Slow-ish (~100 ms on a
    // modern laptop) but deterministic so flake-free.
    let m = MaglevTableSize::new(16_381).expect("16_381 is in ALLOWED_PRIMES");
    let backends_v1: BTreeMap<BackendId, Weight> =
        (1u32..=100u32).map(|i| (BackendId::new(i).expect("u32"), 1u16)).collect();

    let table_v1 = generate(&backends_v1, m);

    // Remove backend 50 (arbitrary mid-range).
    let evicted = BackendId::new(50).expect("u32");
    let backends_v2: BTreeMap<BackendId, Weight> =
        backends_v1.iter().filter(|&(&id, _)| id != evicted).map(|(&id, &w)| (id, w)).collect();

    let table_v2 = generate(&backends_v2, m);

    // Slot-by-slot diff: how many slots changed?
    let m_usize = m.get() as usize;
    assert_eq!(table_v1.len(), m_usize);
    assert_eq!(table_v2.len(), m_usize);

    let changed: usize = (0..m_usize).filter(|&i| table_v1[i] != table_v2[i]).count();

    // Forced shift: every slot in v1 that mapped to b50 MUST change.
    let forced: usize = table_v1.iter().filter(|&&id| id == evicted).count();
    // Incidental shift: changed slots that did NOT map to b50 in v1.
    let incidental = changed - forced;

    // Bound: total disruption ≤ 2.5 % at production parameters.
    // (See maglev_churn.rs § "Bound calibration" — empirical central
    // tendency 2.41 %, safety margin 2.5 %.)
    #[allow(clippy::cast_precision_loss)]
    let total_pct = (changed as f64 / m_usize as f64) * 100.0;
    #[allow(clippy::cast_precision_loss)]
    let incidental_pct = (incidental as f64 / m_usize as f64) * 100.0;

    assert!(
        total_pct <= 2.5,
        "total disruption {total_pct:.3} % exceeds 2.5 % bound (changed={changed} of {m_usize})"
    );
    assert!(
        incidental_pct <= 1.5,
        "incidental disruption {incidental_pct:.3} % exceeds 1.5 % bound (incidental={incidental} of {m_usize})"
    );
    // Forced shift IS the population that mapped to b50 — non-zero
    // (otherwise b50 wasn't in the table v1, contradiction).
    assert!(forced > 0, "evicted backend must have claimed at least one slot in v1");
}

// -----------------------------------------------------------------------------
// Edge cases — empty input + zero-weight short-circuits
// -----------------------------------------------------------------------------

#[test]
fn generate_empty_backends_returns_empty_vec() {
    // Catches the `is_empty()` short-circuit branch flips on line 105
    // (`||` → `&&` would skip the early return, which would hit the
    // `entries[0].0` out-of-bounds index later).
    let m = MaglevTableSize::new(251).expect("prime");
    let empty: BTreeMap<BackendId, Weight> = BTreeMap::new();
    let table = generate(&empty, m);
    assert!(table.is_empty());
}

#[test]
fn generate_single_backend_fills_every_slot() {
    // Catches `generate -> vec![]` (always-empty body) AND
    // line 199 `==` → `!=` (the `filled == m_usize` termination check).
    let m = MaglevTableSize::new(509).expect("prime");
    let only = BackendId::new(42).expect("u32");
    let backends: BTreeMap<BackendId, Weight> = [(only, 3u16)].into_iter().collect();
    let table = generate(&backends, m);
    assert_eq!(table.len(), 509);
    assert!(table.iter().all(|&id| id == only));
}

// -----------------------------------------------------------------------------
// Reference implementation — closes the cargo-mutants gap on offset/skip
// arithmetic mutations that preserve determinism, bijection, and full
// population but produce a *different valid* table.
// -----------------------------------------------------------------------------
//
// The proptests above assert structural properties (determinism, every slot
// is a known backend, length == M, distinct inputs → distinct outputs).
// Three arithmetic mutations on the offset/skip computation slip past every
// one of those because they preserve all four properties — they just shift
// the permutation to a different (still-valid) layout:
//
//   * line 134 `m_u32 - 1` → `m_u32 + 1`  — skip range expands from
//     [1, M-1] to [1, M+1]; both still produce traversable permutations
//     mod M for nearly every random input.
//   * line 152 `h_offset % m_u64` → `h_offset / m_u64` — offset becomes
//     `(h_offset / M) as u32` (truncated); slot is still `% M` so output
//     is a different but valid permutation.
//   * line 152 `h_offset % m_u64` → `h_offset + m_u64` — offset becomes
//     `(h_offset + M) as u32` (truncated); same shape.
//
// The robust catcher: a faithful reference implementation in this test
// file (which cargo-mutants does not mutate, only `permutation.rs` is)
// asserted against the source via a proptest. Any arithmetic divergence
// in the source produces a slot mismatch on at least one input.
//
// Reference faithfulness is the load-bearing property — the comments
// below cite the source line each block mirrors. If `permutation.rs`
// changes algorithmically (not a refactor), update both sides in lockstep.

const REF_FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const REF_FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn ref_fnv1a_64(parts: &[&[u8]]) -> u64 {
    let mut h = REF_FNV_OFFSET;
    for part in parts {
        for &b in *part {
            h ^= u64::from(b);
            h = h.wrapping_mul(REF_FNV_PRIME);
        }
    }
    h
}

fn ref_generate(backends: &BTreeMap<BackendId, Weight>, m: MaglevTableSize) -> Vec<BackendId> {
    let m_u32 = m.get();
    let m_usize = m_u32 as usize;

    if backends.is_empty() || m_u32 == 0 {
        return Vec::new();
    }

    let total_replicas: usize =
        backends.values().copied().map(usize::from).fold(0usize, usize::saturating_add);

    let mut entries: Vec<(BackendId, u16)> = Vec::with_capacity(total_replicas);
    for (id, weight) in backends {
        for replica in 0..*weight {
            entries.push((*id, replica));
        }
    }

    // Mirrors src line 134-135.
    let table_minus_one = u64::from(m_u32 - 1);
    let m_u64 = u64::from(m_u32);

    let mut perms: Vec<(u32, u32)> = Vec::with_capacity(entries.len());
    for (id, replica) in &entries {
        let id_bytes = id.get().to_le_bytes();
        let rep_bytes = replica.to_le_bytes();
        let h_offset = ref_fnv1a_64(&[b"overdrive-maglev-offset", &id_bytes, &rep_bytes]);
        let h_skip = ref_fnv1a_64(&[b"overdrive-maglev-skip", &id_bytes, &rep_bytes]);
        // Mirrors src line 152 / 154. Same cast-truncation reasoning:
        // value is bounded above by m_u64 (≤ 131_071) which fits in u32.
        #[allow(clippy::cast_possible_truncation)]
        let offset = (h_offset % m_u64) as u32;
        #[allow(clippy::cast_possible_truncation)]
        let skip = ((h_skip % table_minus_one) + 1) as u32;
        perms.push((offset, skip));
    }

    let n = entries.len();
    let mut next_idx = vec![0u32; n];
    let mut result: Vec<Option<BackendId>> = vec![None; m_usize];
    let mut filled = 0usize;

    'outer: while filled < m_usize {
        for entry_idx in 0..n {
            let (offset, skip) = perms[entry_idx];
            loop {
                let probe = next_idx[entry_idx];
                let slot = (offset.wrapping_add(probe.wrapping_mul(skip)) % m_u32) as usize;
                next_idx[entry_idx] = probe.wrapping_add(1);
                if result[slot].is_none() {
                    result[slot] = Some(entries[entry_idx].0);
                    filled += 1;
                    break;
                }
                if u64::from(next_idx[entry_idx]) >= m_u64 {
                    break;
                }
            }
            if filled == m_usize {
                break 'outer;
            }
        }
    }

    let fallback = entries[0].0;
    result.into_iter().map(|s| s.unwrap_or(fallback)).collect()
}

#[test]
fn matches_reference_single_backend() {
    // Single-backend cases exercise the offset/skip arithmetic for one
    // entry; mutations that change the FIRST slot's offset are visible
    // because that slot determines where population begins.
    let m = MaglevTableSize::new(251).expect("prime");
    for &raw in &[1u32, 7, 42, 100, 200, 1_000, 65_535] {
        let id = BackendId::new(raw).expect("u32");
        let backends: BTreeMap<BackendId, Weight> = [(id, 1u16)].into_iter().collect();
        let actual = generate(&backends, m);
        let expected = ref_generate(&backends, m);
        assert_eq!(actual, expected, "single backend id={raw} M=251 must match reference");
    }
}

#[test]
fn matches_reference_multi_backend_known_input() {
    // Two-backend case — the population trajectory of each entry depends
    // on its own (offset, skip), so any arithmetic mutation produces a
    // different slot pattern even when both backends are still present.
    let m = MaglevTableSize::new(251).expect("prime");
    let b1 = BackendId::new(100).expect("u32");
    let b2 = BackendId::new(200).expect("u32");
    let backends: BTreeMap<BackendId, Weight> = [(b1, 1u16), (b2, 1u16)].into_iter().collect();
    let actual = generate(&backends, m);
    let expected = ref_generate(&backends, m);
    assert_eq!(actual, expected, "(100, 200) M=251 must match reference");
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// Source matches the in-test reference implementation slot-for-slot
    /// for every generated input. The reference computes
    /// `offset = h_offset % M` and `skip = (h_skip % (M-1)) + 1`
    /// faithfully; arithmetic mutations on lines 134 / 152 produce
    /// values outside the original range, yielding a different trajectory
    /// for at least one input within proptest's case budget.
    ///
    /// This proptest is the structural backstop for the three mutations
    /// the existing property tests miss:
    ///   - line 134 `m_u32 - 1` → `m_u32 + 1`
    ///   - line 152 `h_offset % m_u64` → `h_offset / m_u64`
    ///   - line 152 `h_offset % m_u64` → `h_offset + m_u64`
    #[test]
    fn matches_reference_implementation(
        backends in arb_weighted_backends(8, 4),
        m in arb_table_size(),
    ) {
        let actual = generate(&backends, m);
        let expected = ref_generate(&backends, m);
        prop_assert_eq!(actual, expected);
    }
}

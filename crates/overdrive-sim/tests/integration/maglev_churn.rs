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

#[test]
#[ignore = "RED scaffold S-2.2-12 — DELIVER fills the body per Slice 04"]
fn maglev_generate_is_deterministic_under_seeded_inputs() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-12 — \
         maglev::generate is deterministic across calls with \
         identical (BTreeMap<BackendId, Weight>, MaglevTableSize) inputs"
    );
}

#[test]
#[ignore = "RED scaffold S-2.2-13 — DELIVER fills the body per Slice 04"]
fn single_backend_removal_shifts_at_most_two_percent_of_flows() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-13 — \
         single-backend removal among 100 shifts ≤ 2 % of flows \
         (1 % forced + ≤ 1 % incidental per Maglev's published bound)"
    );
}

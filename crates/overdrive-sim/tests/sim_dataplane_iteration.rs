//! Deterministic iteration over `SimDataplane.services` — DST seed
//! reproducibility prerequisite per `.claude/rules/development.md`
//! § Ordered-collection choice.
//!
//! `core` and control-plane hot paths default to `BTreeMap` for keyed
//! maps whose iteration order is observed (drain, snapshot, JSON
//! output, invariant evaluation). `HashMap`'s `RandomState` is a
//! per-process random-seeded source of nondeterminism; two seeded DST
//! runs produce divergent dispatch orderings the moment ≥ 2 distinct
//! keys are held. That violates the K3 *seed → bit-identical
//! trajectory* property documented in whitepaper § 21.
//!
//! `SimDataplane.services` is observed by harness invariants — every
//! reconciliation that emits an `update_service` Action lands in this
//! map, and DST assertions read it back via `service_backends` (and
//! later, via map-iteration callsites in the slice-08 hydrator).
//! Migrating it from `HashMap` to `BTreeMap` makes the iteration
//! order a function of the keys (`Ord` on `Ipv4Addr`), not of the
//! process's hash seed — every seed produces the bit-identical
//! sequence.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::redundant_clone)]
#![allow(clippy::cast_possible_truncation)]

use std::net::Ipv4Addr;

use overdrive_core::SpiffeId;
use overdrive_core::traits::dataplane::{Backend, Dataplane};
use overdrive_sim::adapters::dataplane::SimDataplane;
use proptest::prelude::*;

// -----------------------------------------------------------------------------
// Helpers — minimal scaffolding for building Backend values; the
// proptest below cares about the *order* the dataplane stores VIPs
// in, not the backend payload.
// -----------------------------------------------------------------------------

fn spiffe(path: &str) -> SpiffeId {
    SpiffeId::new(&format!("spiffe://overdrive.local{path}")).expect("valid SPIFFE URI")
}

fn dummy_backend() -> Backend {
    Backend {
        alloc: spiffe("/job/payments/alloc/a1b2c3"),
        addr: "127.0.0.1:8080".parse().expect("valid socket"),
        weight: 100,
        healthy: true,
    }
}

// -----------------------------------------------------------------------------
// Property — for any non-empty set of distinct VIPs, the order in
// which `service_backends_keys` enumerates them is a function of
// `Ord` on `Ipv4Addr`, not of insertion order. This is the load-
// bearing property the `BTreeMap` migration delivers.
// -----------------------------------------------------------------------------

fn distinct_vips() -> impl Strategy<Value = Vec<Ipv4Addr>> {
    // 2..=8 distinct addresses keeps shrinking fast and stays within
    // the proptest default case budget. Single-element sets are not
    // useful (any iteration order is "deterministic"); the property
    // bites the moment there are ≥ 2 keys.
    prop::collection::vec(any::<u32>(), 2..=8)
        .prop_map(|raws| raws.into_iter().map(Ipv4Addr::from).collect::<Vec<_>>())
        .prop_filter("VIPs must be distinct", |vs| {
            let mut sorted = vs.clone();
            sorted.sort();
            sorted.dedup();
            sorted.len() == vs.len()
        })
}

proptest! {
    /// For any pair of `(insertion_order_a, insertion_order_b)` over
    /// the same VIP set, the iteration order returned by
    /// `service_backends_keys` is identical — and equal to the
    /// `Ord`-sorted form of the input. This pins the §K3 reproducibility
    /// property at the SimDataplane boundary.
    #[test]
    fn service_iteration_is_deterministic_across_insertion_orders(
        vips in distinct_vips(),
        permutation_seed in any::<u64>(),
    ) {
        // Fixed insertion order: sorted ascending.
        let mut order_a = vips.clone();
        order_a.sort();

        // Permuted insertion order: deterministically shuffle by
        // `permutation_seed`. Using `rotate_right` over a hash of the
        // seed keeps the permutation deterministic across proptest
        // runs without pulling in `rand`.
        let mut order_b = vips.clone();
        let rotation = (permutation_seed % (vips.len() as u64)) as usize;
        order_b.rotate_right(rotation);

        let dataplane_a = SimDataplane::new();
        let dataplane_b = SimDataplane::new();
        let backend = dummy_backend();

        // Use a tokio runtime for the async `update_service` calls.
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("tokio runtime");

        for v in &order_a {
            rt.block_on(dataplane_a.update_service(*v, vec![backend.clone()]))
                .expect("update_service succeeds");
        }
        for v in &order_b {
            rt.block_on(dataplane_b.update_service(*v, vec![backend.clone()]))
                .expect("update_service succeeds");
        }

        let keys_a = dataplane_a.service_vip_keys();
        let keys_b = dataplane_b.service_vip_keys();

        // Both insertion orders produce identical iteration order.
        prop_assert_eq!(&keys_a, &keys_b);

        // And that iteration order is the `Ord`-sorted VIP set —
        // i.e. a function of the keys, never of insertion history.
        let mut expected = vips.clone();
        expected.sort();
        prop_assert_eq!(&keys_a, &expected);
    }
}

// -----------------------------------------------------------------------------
// Smoke test — pinned single seed equivalent of the property above,
// catches a regression under plain `cargo nextest` without proptest
// having to hit the broken case.
// -----------------------------------------------------------------------------

#[test]
fn service_iteration_matches_btreemap_order_for_three_vips() {
    let dp = SimDataplane::new();
    let b = dummy_backend();
    let v1 = Ipv4Addr::new(10, 0, 0, 3);
    let v2 = Ipv4Addr::new(10, 0, 0, 1);
    let v3 = Ipv4Addr::new(10, 0, 0, 2);

    let rt = tokio::runtime::Builder::new_current_thread().build().expect("tokio runtime");

    // Insert in non-sorted order.
    rt.block_on(dp.update_service(v1, vec![b.clone()])).expect("update");
    rt.block_on(dp.update_service(v2, vec![b.clone()])).expect("update");
    rt.block_on(dp.update_service(v3, vec![b])).expect("update");

    let keys = dp.service_vip_keys();
    assert_eq!(keys, vec![v2, v3, v1], "BTreeMap order = ascending Ipv4Addr");
}

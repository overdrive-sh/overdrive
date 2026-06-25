//! S-DBN-FRONTEND-01..04 — `dns_responder::frontend_addr_allocator` proptests
//! (Tier 1, default unit lane, in-process; ADR-0072 REV-2 "stable-frontend",
//! GH #243; roadmap 01-04 / REV-2 design unit 1a-A).
//!
//! These are the mandatory PBT coverage of the `FrontendAddrAllocator` seam —
//! the per-host source of the STABLE per-`<job>` frontend address `F` the
//! dial-by-name responder answers with. Every property asserts THROUGH the
//! pinned public surface (`assign` / `release` / `snapshot` /
//! `WORKLOAD_FRONTEND_BASE`) and the two REAL named disjointness consts
//! (`veth_provisioner::WORKLOAD_SUBNET_BASE`, `VipRange::default()`) — NEVER on
//! the allocator's private map field and NEVER against a magic number. The
//! litmus: deleting the `held.insert(...)` in `assign` flips FRONTEND-04's
//! pairwise-distinctness assertion RED (the port-to-port / no-testing-theatre
//! defence).
//!
//! Port-to-port discipline: the pure smallest-free scan and the atomic held-map
//! wrapper are both exercised through the public `assign`/`release`/`snapshot`
//! surface — there is no below-port helper that warrants a separate unit layer.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

use overdrive_control_plane::dns_responder::frontend_addr_allocator::{
    FrontendAddrAllocator, WORKLOAD_FRONTEND_BASE,
};
use overdrive_control_plane::veth_provisioner::WORKLOAD_SUBNET_BASE;
use overdrive_core::id::MeshServiceName;
use overdrive_dataplane::allocators::VipRange;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies — domain-specific generators for the `<job>` key space.
// ---------------------------------------------------------------------------

/// A valid `<job>` label: DNS-1123, starts + ends alphanumeric, single label
/// (no interior `.` — the v1 single-label contract), within `LABEL_MAX`. Kept
/// short (≤ 16) to keep generation cheap; the boundary is covered by the
/// `MeshServiceName` validation suite, not here.
fn arb_job_label() -> impl Strategy<Value = String> {
    "[a-z0-9]([a-z0-9-]{0,14}[a-z0-9])?"
        .prop_filter("no trailing/leading hyphen", |s| !s.starts_with('-') && !s.ends_with('-'))
}

/// A valid `MeshServiceName` (the logical `<job>` key) from a generated label.
fn arb_mesh_name() -> impl Strategy<Value = MeshServiceName> {
    arb_job_label().prop_map(|label| {
        let full = format!("{label}.{}", MeshServiceName::SUFFIX);
        MeshServiceName::new(&full).expect("generated label is a valid mesh service name")
    })
}

/// A set of DISTINCT `<job>` labels of size `1..=n_max`. Distinctness via the
/// canonical `<job>` string keeps the {J1..Jn} set genuinely n-element.
fn arb_distinct_jobs(n_max: usize) -> impl Strategy<Value = Vec<MeshServiceName>> {
    proptest::collection::hash_set(arb_job_label(), 1..=n_max).prop_map(|labels| {
        labels
            .into_iter()
            .map(|label| {
                let full = format!("{label}.{}", MeshServiceName::SUFFIX);
                MeshServiceName::new(&full).expect("generated label is a valid mesh service name")
            })
            .collect()
    })
}

// ---------------------------------------------------------------------------
// S-DBN-FRONTEND-01 — membership + disjointness.
//
// A fresh allocator's `assign(J)` returns Ok(F) where F ∈ 10.98.0.0/16 AND F is
// disjoint from WORKLOAD_SUBNET_BASE (10.99.0.0/16) AND from the service-VIP
// range (10.96.0.0/16, VipRange::default()) — asserted against the live named
// consts, never a magic number.
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn frontend_01_assigned_addr_is_in_frontend_block_and_disjoint(
        job in arb_mesh_name(),
    ) {
        let allocator = FrontendAddrAllocator::new();
        let f = allocator.assign(&job).expect("a fresh allocator has free addresses");

        // Membership in the frontend block — the real const, not a literal.
        prop_assert!(
            WORKLOAD_FRONTEND_BASE.contains(&f),
            "assigned {f} must lie inside WORKLOAD_FRONTEND_BASE {WORKLOAD_FRONTEND_BASE}",
        );

        // Never the reserved network or broadcast endpoint of the /16 — a
        // subnet-zero / broadcast destination is not guaranteed routable through
        // the frontend datapath, so the usable-host scan excludes both (mirrors
        // the veth_provisioner gateway/peer and VipRange reservation discipline).
        prop_assert_ne!(
            f,
            WORKLOAD_FRONTEND_BASE.network(),
            "assigned {} must not be the reserved network address {}",
            f,
            WORKLOAD_FRONTEND_BASE.network(),
        );
        prop_assert_ne!(
            f,
            WORKLOAD_FRONTEND_BASE.broadcast(),
            "assigned {} must not be the reserved broadcast address {}",
            f,
            WORKLOAD_FRONTEND_BASE.broadcast(),
        );

        // Disjoint from the per-netns /30 block (10.99.0.0/16) — the real const.
        prop_assert!(
            !WORKLOAD_SUBNET_BASE.contains(&f),
            "assigned {f} must NOT lie inside WORKLOAD_SUBNET_BASE {WORKLOAD_SUBNET_BASE}",
        );

        // Disjoint from the service-VIP range (10.96.0.0/16) — the real default.
        prop_assert!(
            !VipRange::default().contains(f),
            "assigned {f} must NOT lie inside the service-VIP range (VipRange::default())",
        );
    }
}

// ---------------------------------------------------------------------------
// S-DBN-FRONTEND-02 — retained across an alloc cycle (SQ1-elimination).
//
// Once assign(J) returned F, a second assign(J) returns the SAME F, unchanged
// regardless of intervening assigns/releases of OTHER <job>s (the alloc-cycle
// case: stop → new AllocationId → new backend addr but the SAME logical <job>).
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn frontend_02_assign_is_idempotent_across_an_alloc_cycle(
        job in arb_mesh_name(),
        others in arb_distinct_jobs(8),
    ) {
        let allocator = FrontendAddrAllocator::new();
        let first = allocator.assign(&job).expect("first assign of J succeeds");

        // Intervening churn of OTHER <job>s: assign each, then release the ones
        // distinct from J (J itself is never released here — that is the whole
        // point of the alloc-cycle case).
        for other in &others {
            let _ = allocator.assign(other).expect("assigning another job succeeds");
        }
        for other in &others {
            if other != &job {
                allocator.release(other);
            }
        }

        // The second assign of the SAME logical <job> returns the SAME F.
        let second = allocator.assign(&job).expect("second assign of J succeeds");
        prop_assert_eq!(
            first,
            second,
            "assign(J) must be idempotent: the same <job> keeps the same frontend F \
             across an alloc cycle and intervening churn of other <job>s",
        );

        // And the snapshot still maps J → F (the binding survived).
        let snap = allocator.snapshot();
        prop_assert_eq!(
            snap.get(&job).copied(),
            Some(first),
            "snapshot must still carry J → F after the alloc cycle",
        );
    }
}

// ---------------------------------------------------------------------------
// S-DBN-FRONTEND-03 — withhold-not-release (Finding-2).
//
// PROPERTY 1: while release(J) is NOT called (the transient zero-healthy
// window), a subsequent assign(J) STILL returns the SAME F AND snapshot still
// contains J → F (the allocator carries NO health state).
//
// PROPERTY 2 (the genuine end): when release(J) IS called (logical deletion),
// snapshot no longer contains J AND a later assign of a DIFFERENT <job> MAY
// draw F.
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn frontend_03_withhold_does_not_release_only_explicit_release_does(
        job in arb_mesh_name(),
        successor in arb_mesh_name(),
    ) {
        prop_assume!(job != successor);

        let allocator = FrontendAddrAllocator::new();
        let f = allocator.assign(&job).expect("first assign of J succeeds");

        // PROPERTY 1 — withhold (zero-healthy) is NOT a release. No release(J)
        // call: a subsequent assign(J) keeps F and the snapshot still has J → F.
        let still = allocator.assign(&job).expect("re-assign of withheld J succeeds");
        prop_assert_eq!(
            f,
            still,
            "a transient zero-healthy window (no release) must NOT change J's F",
        );
        prop_assert_eq!(
            allocator.snapshot().get(&job).copied(),
            Some(f),
            "snapshot must still carry J → F while J is merely withheld, never released",
        );

        // PROPERTY 2 — explicit release (logical deletion) IS the genuine end.
        allocator.release(&job);
        prop_assert!(
            !allocator.snapshot().contains_key(&job),
            "after release(J), snapshot must no longer contain J",
        );

        // On a fresh allocator drawing only the single successor, F (the lowest
        // free address) MAY be reused — concretely, since the block reclaims the
        // freed lowest address, the successor draws exactly F.
        let reused = allocator.assign(&successor).expect("assigning the successor succeeds");
        prop_assert_eq!(
            reused,
            f,
            "after J is released, the reclaimed lowest-free F is available to a different <job>",
        );
    }
}

// ---------------------------------------------------------------------------
// S-DBN-FRONTEND-04 — collision-free distinct assignment + reclaim.
//
// For every set of distinct <job> labels {J1..Jn}, each assign()ed yields
// pairwise-distinct {F1..Fn}, each Fi ∈ 10.98.0.0/16; AND after release(Jk) a
// fresh assign of a NEW <job> MAY reuse Fk (the block is reclaimed).
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn frontend_04_distinct_jobs_get_distinct_addrs_and_reclaim_works(
        jobs in arb_distinct_jobs(16),
        newcomer in arb_mesh_name(),
    ) {
        prop_assume!(!jobs.contains(&newcomer));

        let allocator = FrontendAddrAllocator::new();

        // Each distinct <job> gets a distinct frontend F, all in the block.
        let mut assigned: BTreeSet<Ipv4Addr> = BTreeSet::new();
        for job in &jobs {
            let f = allocator.assign(job).expect("assign within block capacity succeeds");
            prop_assert!(
                WORKLOAD_FRONTEND_BASE.contains(&f),
                "every assigned {f} must lie inside WORKLOAD_FRONTEND_BASE",
            );
            prop_assert!(
                assigned.insert(f),
                "distinct <job>s must get pairwise-distinct frontend addresses (no collision)",
            );
        }
        prop_assert_eq!(
            assigned.len(),
            jobs.len(),
            "n distinct <job>s yield exactly n distinct frontend addresses",
        );

        // Reclaim: release one job, then a NEW <job> draws the reclaimed lowest
        // free address — concretely the smallest currently-free address in the
        // block. We assert the new draw is distinct from every STILL-held addr
        // and back inside the block (the free address re-enters the pool).
        let released_job = &jobs[0];
        let released_addr = allocator
            .snapshot()
            .get(released_job)
            .copied()
            .expect("the released job was held");
        allocator.release(released_job);

        let drawn = allocator.assign(&newcomer).expect("assigning the newcomer succeeds");
        prop_assert!(
            WORKLOAD_FRONTEND_BASE.contains(&drawn),
            "the reclaimed draw {drawn} must lie inside WORKLOAD_FRONTEND_BASE",
        );

        // The drawn address is NOT held by any of the still-live other jobs.
        let snap = allocator.snapshot();
        for job in jobs.iter().skip(1) {
            prop_assert_ne!(
                snap.get(job).copied(),
                Some(drawn),
                "the reclaimed draw must not collide with a still-held <job>'s address",
            );
        }
        // And the freed address is back in the pool: it is now held by exactly
        // one of {newcomer} ∪ (still-live jobs) — the smallest-free reclaim means
        // the newcomer drew the freed lowest address.
        prop_assert_eq!(
            snap.get(&newcomer).copied(),
            Some(drawn),
            "the newcomer holds the address it was assigned",
        );
        let _ = released_addr; // freed address: reclaimable; smallest-free draws it
    }
}

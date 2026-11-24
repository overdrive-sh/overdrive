//! Pool acceptance — P1-S2, P1-S9, R5-55 (a) and (c) and the G-V0 rows
//! (`AdmissionCapReached`, `LeaseRetiring`, `PoolExhausted`), re-derived for
//! CID-NOCLAIM (netns-density-295; ADR-0156, ADR-0177, ADR-0132/0133). R5-55
//! (a): every lease's CID is `GUEST_CID_BASE + offset`, unique among held
//! leases, next-fit unchanged (proptest over operation sequences); (c): the
//! lease outlives its retirement until `release` (the caller releases only
//! after the VMM exited — `quint_cid_lease`). Lands with DELIVER step 15
//! (pool next-fit with the launch token from `Entropy`).
//!
//! Contract: `docs/feature/netns-density-295/feature-delta.md`
//! § *Core vocabulary* (`GuestAddressPool::new`, `assign`, `release`, the CID
//! derivation), § *Owner and provisioner* → "Conformance observation (owner
//! accessors)" (`lease_observation`, QPROJ-2) and Gate G-V0.
//!
//! The lease-state model every body is compared against:
//! `absent -> Admitted -> Retiring -> absent`. Within one `serve` process the
//! lease alone gives CID uniqueness: every lease's CID is
//! `GUEST_CID_BASE + offset` of its own workload address, the walk is
//! next-fit from an in-memory cursor (M-9), and each new lease draws its
//! launch token from the injected `Entropy` port in the same lock acquisition
//! (ATTR-TOKEN). An Admitted replay returns the plan the first `assign`
//! returned; a Retiring replay is refused `LeaseRetiring`; both states count
//! against the fixed cap until `release`; a typed refusal changes nothing.
//! The pool persists nothing: a pool built after a `serve` restart reports no
//! lease (NO-SURVIVE).
//!
//! Driving port: the pool's own operations (`assign`, `retire`, `release`).
//! Observation: the pinned `lease_observation()` (QPROJ-2) and the
//! production `GuestAttachmentView` read (`observe`); no body reads or writes
//! the pool's private state. Layer 1-2 (pure, in-memory): property-based where
//! the input space is unbounded.
//!
//! Release only after the VMM has exited is the CALLER's ordering
//! (release-last); its enforcement is the `cid_lease` conformance driver's
//! `ReleaseWhileTheVmmRunsIsRefused` and `StopExitsTheVmmThenReleaseIsLast`
//! (`quint_cid_lease`), not a pool body.

#![allow(
    clippy::single_match_else,
    clippy::doc_markdown,
    clippy::expect_used,
    reason = "source-local lease-state tables: each expect() names the fixture precondition it establishes; CONTRACT_SHAPE lines are machine-read"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::sync::{Arc, LazyLock};

use ipnet::Ipv4Net;
use overdrive_core::guest_network::{GuestAttachmentOccupancy, MAX_GUEST_NETWORK_ATTACHMENTS};
use overdrive_core::guest_transport::{GUEST_CID_BASE, GuestCid, LaunchToken};
use overdrive_core::id::AllocationId;
use overdrive_core::net::ListenPort;
use overdrive_core::traits::{GuestAttachmentLease, GuestAttachmentObservation};
use overdrive_sim::adapters::entropy::SimEntropy;
use proptest::prelude::*;

use crate::shared_network_test_ports::test_owner_flow_ports;

use super::{
    GuestAddressPool, GuestLeaseObservation, GuestLeaseView, GuestNetworkError, GuestNetworkPlan,
};

use overdrive_testing::placement;

placement!(host_pure);

/// The fixed admission cap as a lease count.
const CAP: usize = MAX_GUEST_NETWORK_ATTACHMENTS as usize;

/// Allocation-id key space of the generated operation sequences. Small, so
/// sequences replay, retire and release the same allocations often.
const KEYS: u16 = 48;

/// The node gateway every fixture pool is built with (first host address).
const GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);

/// Ids for the at-cap body: `CAP` leases plus one refused newcomer.
static CAP_IDS: LazyLock<Vec<AllocationId>> = LazyLock::new(|| {
    (0..=CAP)
        .map(|index| AllocationId::new(&format!("nd295-cap-{index:05}")).expect("allocation id"))
        .collect()
});

fn pool_over(prefix: &str, seed: u64) -> GuestAddressPool {
    GuestAddressPool::new(
        prefix.parse().expect("node prefix"),
        GATEWAY,
        Arc::new(SimEntropy::new(seed)),
    )
}

fn pool() -> GuestAddressPool {
    pool_over("100.95.0.0/16", 0x295)
}

fn alloc(key: u16) -> AllocationId {
    AllocationId::new(&format!("nd295-{key:04x}")).expect("generated allocation id")
}

/// The address offset of `addr` inside `prefix`.
fn offset_of(prefix: Ipv4Net, addr: Ipv4Addr) -> u32 {
    u32::from(addr) - u32::from(prefix.network())
}

/// The usable lease offsets of `prefix`: every host address except the
/// gateway (the first host address), in address order.
fn usable_offsets(prefix: Ipv4Net) -> Vec<u32> {
    prefix.hosts().map(|host| offset_of(prefix, host)).filter(|offset| *offset != 1).collect()
}

/// G-V0 for one plan: the CID is `GUEST_CID_BASE + offset` of the plan's own
/// workload address, the gateway is the node's, and the launch token is the
/// lease's.
fn assert_plan_identity(prefix: Ipv4Net, plan: &GuestNetworkPlan, view: &GuestLeaseView) {
    let assignment = plan.assignment();
    let offset = offset_of(prefix, assignment.workload_addr());
    assert_eq!(assignment.cid().get(), GUEST_CID_BASE + offset, "CID = GUEST_CID_BASE + offset");
    assert_eq!(assignment.gateway(), GATEWAY, "the node's gateway");
    assert_eq!(view.cid(), assignment.cid(), "the observation names the plan's CID");
    assert_eq!(
        view.launch(),
        Some(assignment.launch()),
        "the observation names the plan's launch token"
    );
}

/// One generated pool operation.
#[derive(Debug, Clone, Copy)]
enum LeaseOp {
    Assign(u16),
    Retire(u16),
    Release(u16),
}

fn lease_op() -> impl Strategy<Value = LeaseOp> {
    prop_oneof![
        3 => (0..KEYS).prop_map(LeaseOp::Assign),
        2 => (0..KEYS).prop_map(LeaseOp::Retire),
        1 => (0..KEYS).prop_map(LeaseOp::Release),
    ]
}

/// The independent lease and next-fit model (M-9): each held allocation's
/// lease, the plan its first `assign` returned, and the cursor as an index
/// into the usable offsets.
#[derive(Debug, Clone)]
struct LeaseModel {
    usable: Vec<u32>,
    cursor: usize,
    leases: BTreeMap<AllocationId, (GuestNetworkPlan, GuestAttachmentLease)>,
}

impl LeaseModel {
    fn new(prefix: Ipv4Net) -> Self {
        Self { usable: usable_offsets(prefix), cursor: 0, leases: BTreeMap::new() }
    }

    fn held_offsets(&self, prefix: Ipv4Net) -> BTreeSet<u32> {
        self.leases
            .values()
            .map(|(plan, _)| offset_of(prefix, plan.assignment().workload_addr()))
            .collect()
    }

    /// The offset next-fit takes from the cursor, and the cursor after it.
    fn next_fit(&self, prefix: Ipv4Net) -> Option<(u32, usize)> {
        let held = self.held_offsets(prefix);
        (0..self.usable.len())
            .map(|step| (self.cursor + step) % self.usable.len())
            .find(|index| !held.contains(&self.usable[*index]))
            .map(|index| (self.usable[index], (index + 1) % self.usable.len()))
    }

    fn observation(&self, requested: &[AllocationId]) -> GuestAttachmentObservation {
        let held = u32::try_from(self.leases.len()).expect("small model");
        let retiring = u32::try_from(
            self.leases
                .values()
                .filter(|(_, lease)| *lease == GuestAttachmentLease::Retiring)
                .count(),
        )
        .expect("small model");
        GuestAttachmentObservation {
            occupancy: GuestAttachmentOccupancy { held, retiring },
            leases: requested
                .iter()
                .filter_map(|alloc| {
                    self.leases.get(alloc).map(|(_, lease)| (alloc.clone(), *lease))
                })
                .collect(),
        }
    }
}

/// Drive `op` through the pool's own operations and compare its outcome and
/// the pinned lease observation with the model's.
fn apply_and_compare(
    prefix: Ipv4Net,
    pool: &GuestAddressPool,
    model: &mut LeaseModel,
    op: LeaseOp,
) -> std::result::Result<(), TestCaseError> {
    let before = pool.lease_observation();
    match op {
        LeaseOp::Assign(key) => {
            let alloc = alloc(key);
            let outcome = pool.assign(alloc.clone(), &[], test_owner_flow_ports());
            match model.leases.get(&alloc) {
                Some((first, GuestAttachmentLease::Admitted)) => {
                    prop_assert!(
                        matches!(&outcome, Ok(plan) if plan == first),
                        "an Admitted replay of {key} returned {outcome:?}"
                    );
                    prop_assert_eq!(pool.lease_observation(), before, "a replay changes nothing");
                }
                Some((_, GuestAttachmentLease::Retiring)) => {
                    prop_assert!(
                        matches!(&outcome, Err(GuestNetworkError::LeaseRetiring { alloc: refused }) if *refused == alloc),
                        "a Retiring replay of {key} returned {outcome:?}"
                    );
                    prop_assert_eq!(pool.lease_observation(), before, "a refusal changes nothing");
                }
                None => match model.next_fit(prefix) {
                    Some((offset, cursor)) => {
                        let plan = outcome.expect("a free offset admits the lease");
                        prop_assert_eq!(plan.alloc(), &alloc);
                        prop_assert_eq!(
                            offset_of(prefix, plan.assignment().workload_addr()),
                            offset,
                            "next-fit from the cursor (M-9)"
                        );
                        prop_assert_eq!(plan.assignment().cid().get(), GUEST_CID_BASE + offset);
                        model.cursor = cursor;
                        model.leases.insert(alloc, (plan, GuestAttachmentLease::Admitted));
                    }
                    None => {
                        prop_assert!(
                            matches!(&outcome, Err(GuestNetworkError::PoolExhausted { .. })),
                            "no free offset is PoolExhausted, got {outcome:?}"
                        );
                        prop_assert_eq!(
                            pool.lease_observation(),
                            before,
                            "a refusal changes nothing"
                        );
                    }
                },
            }
        }
        LeaseOp::Retire(key) => {
            let alloc = alloc(key);
            let expected = match model.leases.get_mut(&alloc) {
                Some((_, lease @ GuestAttachmentLease::Admitted)) => {
                    *lease = GuestAttachmentLease::Retiring;
                    true
                }
                Some((_, GuestAttachmentLease::Retiring)) | None => false,
            };
            prop_assert_eq!(pool.retire(&alloc), expected, "retire({}) transition", key);
        }
        LeaseOp::Release(key) => {
            model.leases.remove(&alloc(key));
            pool.release(&alloc(key));
            // A release of an absent lease is a no-op.
            pool.release(&alloc(key));
        }
    }
    Ok(())
}

/// The pinned observation of the model's leases (cursor excluded).
fn model_leases(
    model: &LeaseModel,
) -> BTreeMap<AllocationId, (GuestCid, GuestAttachmentLease, LaunchToken)> {
    model
        .leases
        .iter()
        .map(|(alloc, (plan, lease))| {
            (alloc.clone(), (plan.assignment().cid(), *lease, plan.assignment().launch()))
        })
        .collect()
}

fn observed_leases(
    observation: &GuestLeaseObservation,
) -> BTreeMap<AllocationId, (GuestCid, GuestAttachmentLease, LaunchToken)> {
    observation
        .leases
        .iter()
        .map(|(alloc, view)| {
            let launch =
                view.launch().expect("a pool lease assign made is launched, never Reserved");
            (alloc.clone(), (view.cid(), view.state(), launch))
        })
        .collect()
}

proptest! {
    /// G-V0, M-9, CID-NOCLAIM — over any sequence of the pool's own
    /// operations on a small prefix (so the cursor wraps and offsets are
    /// reused), every new lease takes the first free offset at or after the
    /// cursor, its CID is `GUEST_CID_BASE + offset`, no two held leases share
    /// a CID, and the pinned observation holds exactly the model's Admitted
    /// and Retiring leases with their CIDs and launch tokens.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn next_fit_leases_carry_unique_offset_derived_cids_over_any_operation_sequence(
        operations in prop::collection::vec(lease_op(), 0..160),
    ) {
        let prefix: Ipv4Net = "100.95.0.0/27".parse().expect("node prefix");
        let pool = pool_over("100.95.0.0/27", 0x295);
        let mut model = LeaseModel::new(prefix);
        for op in operations {
            apply_and_compare(prefix, &pool, &mut model, op)?;
            let observation = pool.lease_observation();
            prop_assert_eq!(observed_leases(&observation), model_leases(&model));
            let cids = observation.leases.values().map(GuestLeaseView::cid).collect::<BTreeSet<_>>();
            prop_assert_eq!(cids.len(), observation.leases.len(), "CidUnique");
        }
    }

    /// ATTR-TOKEN — the launch token is drawn from the injected entropy port
    /// with the lease: two pools over the same seed hand out the same tokens
    /// for the same assignment sequence; a re-admitted allocation (after
    /// release) draws a new token; no token is all zero.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn launch_tokens_come_from_the_injected_entropy_and_a_new_lease_draws_a_new_token(
        seed in any::<u64>(),
        count in 1_u16..12,
    ) {
        let first = pool_over("100.95.0.0/24", seed);
        let second = pool_over("100.95.0.0/24", seed);
        for key in 0..count {
            let a = first.assign(alloc(key), &[], test_owner_flow_ports()).expect("below capacity");
            let b = second.assign(alloc(key), &[], test_owner_flow_ports()).expect("below capacity");
            prop_assert_eq!(a.assignment().launch(), b.assignment().launch(), "same seed, same draw");
            let launch = a.assignment().launch();
            prop_assert_ne!(launch.as_bytes(), &[0_u8; 16], "never all zero");
        }

        let key = 0;
        let before = first.assign(alloc(key), &[], test_owner_flow_ports()).expect("Admitted replay");
        prop_assert!(first.retire(&alloc(key)));
        first.release(&alloc(key));
        let after = first.assign(alloc(key), &[], test_owner_flow_ports()).expect("a released id takes the normal path");
        prop_assert_ne!(before.assignment().launch(), after.assignment().launch(), "a new lease, a new token");
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-04 — Lease replay, retirement and release change only the named
    /// allocation.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Retirement happens once and never returns a lease to Admitted: a
    /// repeated `retire` and a refused replay leave it Retiring with its CID
    /// and launch token (until `release`), it still counts in `held`, and only
    /// `release` frees it, after which a fresh `assign` of the same allocation
    /// admits it again.
    #[test]
    fn retirement_is_monotonic_and_a_retiring_lease_keeps_its_cid_and_token_until_release(
        admitted in 1_u16..24,
        retire_mask in any::<u32>(),
        repeats in 1_usize..4,
    ) {
        let pool = pool();
        let held = (0..admitted).map(alloc).collect::<Vec<_>>();
        for alloc in &held {
            pool.assign(alloc.clone(), &[], test_owner_flow_ports()).expect("below-cap assignment");
        }
        let admitted_view = pool.lease_observation();
        let retiring = held
            .iter()
            .enumerate()
            .filter(|(index, _)| retire_mask & (1 << index) != 0)
            .map(|(_, alloc)| alloc.clone())
            .collect::<BTreeSet<_>>();

        for alloc in &retiring {
            prop_assert!(pool.retire(alloc), "Admitted -> Retiring reports the transition");
            for _ in 0..repeats {
                prop_assert!(!pool.retire(alloc), "a Retiring lease never transitions again");
            }
        }
        prop_assert!(!pool.retire(&alloc(KEYS + 1)), "an absent lease has no transition");

        let before = pool.lease_observation();
        for alloc in &retiring {
            let refusal = pool.assign(alloc.clone(), &[], test_owner_flow_ports());
            prop_assert!(
                matches!(&refusal, Err(GuestNetworkError::LeaseRetiring { alloc: refused }) if refused == alloc),
                "a Retiring replay is refused as retiring, got {refusal:?}"
            );
        }
        prop_assert_eq!(&pool.lease_observation(), &before, "refused replays change nothing");
        for (alloc, view) in &before.leases {
            let original = &admitted_view.leases[alloc];
            prop_assert_eq!(view.cid(), original.cid(), "retirement keeps the CID");
            prop_assert_eq!(view.launch(), original.launch(), "retirement keeps the token");
            let expected = if retiring.contains(alloc) {
                GuestAttachmentLease::Retiring
            } else {
                GuestAttachmentLease::Admitted
            };
            prop_assert_eq!(view.state(), expected);
        }
        prop_assert_eq!(
            pool.observe(&held).occupancy,
            GuestAttachmentOccupancy {
                held: u32::from(admitted),
                retiring: u32::try_from(retiring.len()).expect("small set"),
            }
        );

        if let Some(released) = retiring.iter().next().cloned() {
            pool.release(&released);
            let after_release = pool.lease_observation();
            prop_assert!(!after_release.leases.contains_key(&released), "release frees the lease");
            prop_assert_eq!(after_release.leases.len(), usize::from(admitted) - 1);
            pool.assign(released.clone(), &[], test_owner_flow_ports()).expect("a released id takes the normal path");
            prop_assert_eq!(
                pool.lease_observation().leases.get(&released).map(GuestLeaseView::state),
                Some(GuestAttachmentLease::Admitted)
            );
        }
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-05A — Admission refuses at the cap over held leases, one pool
    /// per server.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Held leases count in both states. At 16,383 held, with any mix of
    /// Retiring among them, the next allocation is admitted; at 16,384 held a
    /// new allocation is refused with `AdmissionCapReached { held, retiring,
    /// cap: 16_384 }` and the pinned observation is unchanged, while an
    /// Admitted replay still returns its plan and a Retiring replay is refused
    /// as retiring.
    #[test]
    fn admission_refuses_at_the_cap_over_held_leases_for_every_retiring_mix(
        retiring in prop_oneof![Just(0_usize), Just(CAP - 1), 1_usize..CAP - 1],
        offset in 0_usize..CAP - 1,
        retire_the_last_admitted in any::<bool>(),
    ) {
        let ids = &*CAP_IDS;
        let pool = pool();
        let below_cap = CAP - 1;
        for id in &ids[..below_cap] {
            pool.assign(id.clone(), &[], test_owner_flow_ports()).expect("below-cap assignment");
        }
        let retired = (0..retiring).map(|step| (offset + step) % below_cap).collect::<BTreeSet<_>>();
        for index in &retired {
            prop_assert!(pool.retire(&ids[*index]));
        }

        let last = pool.assign(ids[below_cap].clone(), &[], test_owner_flow_ports()).expect("16,383 held admits one more");
        prop_assert_eq!(last.alloc(), &ids[below_cap]);
        let mut retiring_at_cap = retiring;
        if retire_the_last_admitted {
            prop_assert!(pool.retire(&ids[below_cap]));
            retiring_at_cap += 1;
        }

        let before = pool.lease_observation();
        prop_assert_eq!(before.leases.len(), CAP);
        let refusal = pool.assign(ids[CAP].clone(), &[], test_owner_flow_ports());
        prop_assert!(
            matches!(
                refusal,
                Err(GuestNetworkError::AdmissionCapReached { held, retiring, cap })
                    if held == MAX_GUEST_NETWORK_ATTACHMENTS
                        && usize::try_from(retiring).expect("u32 fits") == retiring_at_cap
                        && cap == 16_384
            ),
            "at 16,384 held a new allocation is refused with its counts, got {refusal:?}"
        );
        prop_assert_eq!(&pool.lease_observation(), &before, "the cap refusal changes nothing");

        let admitted_replay = (0..CAP)
            .find(|index| !(retired.contains(index) || (*index == below_cap && retire_the_last_admitted)));
        if let Some(index) = admitted_replay {
            let replay = pool.assign(ids[index].clone(), &[], test_owner_flow_ports()).expect("an Admitted replay at the cap");
            prop_assert_eq!(replay.assignment().cid(), before.leases[&ids[index]].cid(), "equal replay");
            prop_assert_eq!(Some(replay.assignment().launch()), before.leases[&ids[index]].launch());
        }
        prop_assert_eq!(pool.lease_observation(), before, "no refusal or replay changes the leases");
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-05B — Placement reads the node's held attachments, not the
    /// workload's rows.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Over any lease map the pool's own operations produce and any request
    /// set, `observe` returns exactly the whole map's held and retiring counts
    /// and the lease of each requested allocation that holds one, and it
    /// changes nothing: the pinned observation is equal before and after and a
    /// second read is equal.
    #[test]
    fn observe_reads_occupancy_and_requested_leases_in_one_snapshot_and_changes_nothing(
        operations in prop::collection::vec(lease_op(), 0..128),
        requested in prop::collection::vec(0_u16..KEYS + 16, 0..24),
    ) {
        let prefix: Ipv4Net = "100.95.0.0/16".parse().expect("node prefix");
        let pool = pool();
        let mut model = LeaseModel::new(prefix);
        for op in operations {
            apply_and_compare(prefix, &pool, &mut model, op)?;
        }
        let requested = requested.into_iter().map(alloc).collect::<Vec<_>>();

        let before = pool.lease_observation();
        let observed = pool.observe(&requested);
        prop_assert_eq!(&observed, &model.observation(&requested));
        prop_assert_eq!(pool.lease_observation(), before, "observe is read-only");
        prop_assert_eq!(pool.observe(&requested), observed, "a repeated read is equal");
    }
}

/// P1-S2 walking cell — the first `assign` of a fresh pool admits the lease
/// on the first usable offset: its CID is `GUEST_CID_BASE + offset`, its plan
/// carries the declared listen ports and the node gateway, and the pinned
/// observation gains exactly that lease while the cursor moves past it. A
/// repeated observation with no operation between is equal.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn the_first_assign_admits_the_first_usable_offset_and_the_observation_gains_exactly_that_lease() {
    let prefix: Ipv4Net = "100.95.0.0/16".parse().expect("node prefix");
    let pool = pool();
    let fresh = pool.lease_observation();
    assert!(fresh.leases.is_empty(), "an empty pool holds no lease");
    assert_eq!(pool.lease_observation(), fresh, "a repeated read is equal");

    let ports = [ListenPort::new(8080).expect("port"), ListenPort::new(9090).expect("port")];
    let plan = pool.assign(alloc(1), &ports, test_owner_flow_ports()).expect("a fresh pool admits");
    let after = pool.lease_observation();

    let first_usable = usable_offsets(prefix)[0];
    assert_eq!(offset_of(prefix, plan.assignment().workload_addr()), first_usable);
    assert_eq!(plan.listen_ports(), &ports[..], "the plan carries the declared TCP ports");
    assert_eq!(after.leases.len(), 1, "exactly one lease was added");
    let view = &after.leases[&alloc(1)];
    assert_eq!(view.state(), GuestAttachmentLease::Admitted);
    assert_plan_identity(prefix, &plan, view);
    assert_ne!(after.next_fit, fresh.next_fit, "the cursor moved past the taken offset");
    assert_eq!(
        after.next_fit.get(),
        GUEST_CID_BASE + first_usable + 1,
        "next-fit continues after it"
    );
}

/// P1-S9 — a typed refusal changes nothing: on a prefix with one usable
/// offset the second allocation is refused `PoolExhausted { held: 1,
/// capacity: 1 }` below the fixed admission cap, and the pinned observation
/// (leases and cursor) is equal before and after.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
fn pool_exhaustion_below_the_cap_is_typed_and_changes_nothing() {
    let pool = pool_over("100.95.0.0/30", 0x295);
    pool.assign(
        AllocationId::new("nd295-below-cap-first").expect("allocation id"),
        &[],
        test_owner_flow_ports(),
    )
    .expect("the sole usable offset is initially free");
    let before = pool.lease_observation();

    let error = pool
        .assign(
            AllocationId::new("nd295-below-cap-overflow").expect("allocation id"),
            &[],
            test_owner_flow_ports(),
        )
        .expect_err("the small prefix is exhausted below the fixed admission cap");
    assert!(
        matches!(error, GuestNetworkError::PoolExhausted { held: 1, capacity: 1 }),
        "{error:?}"
    );
    assert_eq!(pool.lease_observation(), before, "typed exhaustion does not mutate or reuse state");
}

/// NO-SURVIVE — the pool persists nothing: a pool built after a `serve`
/// restart reports no lease of the previous process and the initial cursor,
/// equal to a never-used pool's observation, and its first `assign` takes the
/// first usable offset again (whatever the previous process's cursor was).
/// The previous process's VMMs are ended by boot reclamation before this
/// `assign` (`vm_reclamation_boot` cells; `quint_cid_lease`).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_pool_built_after_a_serve_restart_reports_no_lease_of_the_previous_process() {
    let prefix: Ipv4Net = "100.95.0.0/16".parse().expect("node prefix");
    let previous = pool();
    for key in 0..5 {
        previous.assign(alloc(key), &[], test_owner_flow_ports()).expect("below capacity");
    }
    assert_eq!(previous.lease_observation().leases.len(), 5);
    drop(previous);

    let restarted = pool();
    let never_used = pool_over("100.95.0.0/16", 7);
    let observation = restarted.lease_observation();
    assert!(observation.leases.is_empty(), "no lease of the previous process");
    assert_eq!(observation.next_fit, never_used.lease_observation().next_fit, "the initial cursor");
    let plan =
        restarted.assign(alloc(9), &[], test_owner_flow_ports()).expect("assign after restart");
    assert_eq!(offset_of(prefix, plan.assignment().workload_addr()), usable_offsets(prefix)[0]);
}

/// Whether two prefixes share any address.
fn prefixes_overlap(a: Ipv4Net, b: Ipv4Net) -> bool {
    a.contains(&b.network()) || b.contains(&a.network())
}

proptest! {
    /// ADR-0276 — over any sequence of assignments and releases, no two
    /// allocations hold the same workload address at once, every leased
    /// address lies inside the node guest prefix, is never the gateway, and
    /// never falls inside the mesh workload range (`WORKLOAD_SUBNET_BASE`).
    /// The small prefix makes the next-fit cursor wrap, so released addresses
    /// are reused and the "while held" qualifier is exercised.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn no_workload_address_is_leased_twice_while_held_and_none_is_a_mesh_address(
        small_prefix in any::<bool>(),
        operations in prop::collection::vec(lease_op(), 0..200),
    ) {
        let prefix_text = if small_prefix { "100.95.0.0/27" } else { "100.95.0.0/16" };
        let prefix: Ipv4Net = prefix_text.parse().expect("node prefix");
        let pool = pool_over(prefix_text, 0x295);
        let mut held: BTreeMap<AllocationId, Ipv4Addr> = BTreeMap::new();
        for op in operations {
            match op {
                LeaseOp::Assign(key) => {
                    let alloc = alloc(key);
                    if let Ok(plan) = pool.assign(alloc.clone(), &[], test_owner_flow_ports()) {
                        let addr = plan.assignment().workload_addr();
                        prop_assert!(prefix.contains(&addr), "{addr} is inside {prefix}");
                        prop_assert_ne!(addr, GATEWAY, "the gateway is never leased");
                        prop_assert!(
                            !crate::veth_provisioner::WORKLOAD_SUBNET_BASE.contains(&addr),
                            "{addr} is a mesh workload address"
                        );
                        match held.get(&alloc) {
                            Some(first) => prop_assert_eq!(*first, addr, "a replay keeps its address"),
                            None => {
                                prop_assert!(
                                    !held.values().any(|other| *other == addr),
                                    "{addr} is already held by another allocation"
                                );
                                held.insert(alloc, addr);
                            }
                        }
                    }
                }
                LeaseOp::Retire(key) => {
                    pool.retire(&alloc(key));
                }
                LeaseOp::Release(key) => {
                    pool.release(&alloc(key));
                    held.remove(&alloc(key));
                }
            }
            let distinct = held.values().collect::<BTreeSet<_>>();
            prop_assert_eq!(distinct.len(), held.len(), "held addresses are pairwise distinct");
        }
    }
}

/// ADR-0276 — the guest prefix the pool leases from and the mesh workload range
/// are disjoint, so no allocation address can be both a guest and a mesh
/// address.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_guest_prefix_and_the_mesh_workload_range_are_disjoint() {
    let guest: Ipv4Net = "100.95.0.0/16".parse().expect("guest prefix");
    assert_eq!(overdrive_core::guest_transport::GUEST_NODE_PREFIX, guest);
    let mesh = crate::veth_provisioner::WORKLOAD_SUBNET_BASE;
    assert_eq!(mesh, "10.99.0.0/16".parse::<Ipv4Net>().expect("mesh range"));
    assert!(!prefixes_overlap(guest, mesh), "{guest} and {mesh} overlap");
}

// ---------------------------------------------------------------------------
// A stuck VM's reservation (ADR-0350, feature delta "Boot proceeds past a stuck
// VM", Pool API). `reserve_leftover` inserts a Retiring lease with no plan at
// the offset a leftover row's `workload_addr` names. Layer 1-2, in-process.
// Driving port: `reserve_leftover`, `assign`, `release`; observation:
// `lease_observation`, `observe`, `snapshot`.
// ---------------------------------------------------------------------------

use crate::action_shim::reclamation::CorruptLeftoverRow;

/// Four-offset prefix: usable offsets 2..=6 (gateway at 1), so a wrap is reached
/// in a few assigns.
const SMALL_PREFIX: &str = "100.95.0.0/29";

fn address_at(prefix: Ipv4Net, offset: u32) -> Ipv4Addr {
    Ipv4Addr::from(u32::from(prefix.network()) + offset)
}

fn leftover_id(index: usize) -> AllocationId {
    AllocationId::new(&format!("nd295-stuck-{index:05}")).expect("allocation id")
}

/// ADR-0350, QPROJ-2. A reservation is one Retiring lease that counts toward
/// the cap: with every other slot reserved, a new allocation is refused with the
/// held and retiring counts the reservations make.
/// CONTRACT_SHAPE: bounded-change. Universe: the pool's occupancy and leases.
#[test]
fn reservations_count_toward_the_cap_and_refuse_the_next_assign_at_it() {
    let prefix: Ipv4Net = "100.95.0.0/16".parse().expect("node prefix");
    let pool = pool();
    let offsets = usable_offsets(prefix);
    for (index, offset) in offsets.iter().take(CAP - 1).enumerate() {
        pool.reserve_leftover(&leftover_id(index), address_at(prefix, *offset))
            .expect("a free leased offset reserves");
    }
    let below = pool.observe(&[]);
    assert_eq!(below.occupancy.held, MAX_GUEST_NETWORK_ATTACHMENTS - 1, "each reservation is held");
    assert_eq!(
        below.occupancy.retiring,
        MAX_GUEST_NETWORK_ATTACHMENTS - 1,
        "each reservation is Retiring"
    );
    pool.assign(alloc(1), &[], test_owner_flow_ports())
        .expect("one slot below the cap still admits a launch");

    let before = pool.lease_observation();
    let refusal = pool.assign(alloc(2), &[], test_owner_flow_ports());
    assert!(
        matches!(
            refusal,
            Err(GuestNetworkError::AdmissionCapReached { held, retiring, cap })
                if held == MAX_GUEST_NETWORK_ATTACHMENTS
                    && retiring == MAX_GUEST_NETWORK_ATTACHMENTS - 1
                    && cap == MAX_GUEST_NETWORK_ATTACHMENTS
        ),
        "reservations count toward the cap: {refusal:?}"
    );
    assert_eq!(pool.lease_observation(), before, "the refusal changes nothing");
}

/// ADR-0350. `assign` for the reserved allocation returns the existing
/// `LeaseRetiring` and changes nothing; the reservation is not a plan.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
fn assign_for_a_reserved_allocation_is_refused_as_retiring_and_changes_nothing() {
    let prefix: Ipv4Net = "100.95.0.0/16".parse().expect("node prefix");
    let pool = pool();
    let stuck = leftover_id(0);
    pool.reserve_leftover(&stuck, address_at(prefix, 9)).expect("reserve");
    let before = pool.lease_observation();

    let refusal = pool.assign(stuck.clone(), &[], test_owner_flow_ports());

    assert!(
        matches!(&refusal, Err(GuestNetworkError::LeaseRetiring { alloc }) if *alloc == stuck),
        "a reserved allocation's assign is LeaseRetiring: {refusal:?}"
    );
    assert_eq!(pool.lease_observation(), before, "the refusal changes nothing");
}

/// ADR-0350, QPROJ-2. `observe` reports the reservation as a Retiring lease
/// and `lease_observation` as `Reserved`: the CID the offset derives and no
/// launch token; `snapshot` (the plans) omits it. A repeat for the same
/// allocation and address changes nothing.
/// CONTRACT_SHAPE: bounded-change. Universe: leases, snapshot, cursor.
#[test]
fn a_reservation_is_a_retiring_reserved_lease_with_the_offsets_cid_and_no_plan() {
    let prefix: Ipv4Net = "100.95.0.0/16".parse().expect("node prefix");
    let pool = pool();
    let stuck = leftover_id(0);
    let fresh_cursor = pool.lease_observation().next_fit;

    pool.reserve_leftover(&stuck, address_at(prefix, 9)).expect("reserve");
    let launched = pool.assign(alloc(1), &[], test_owner_flow_ports()).expect("a launch admits");

    let observation = pool.lease_observation();
    assert_eq!(
        observation.leases[&stuck],
        GuestLeaseView::Reserved {
            cid: GuestCid::new(GUEST_CID_BASE + 9).expect("an in-range CID")
        },
        "the reservation is Reserved at GUEST_CID_BASE + offset"
    );
    assert_eq!(observation.leases[&stuck].state(), GuestAttachmentLease::Retiring);
    assert_eq!(observation.leases[&stuck].launch(), None, "a reservation has no launch token");
    assert_eq!(
        pool.observe(std::slice::from_ref(&stuck)).leases.get(&stuck),
        Some(&GuestAttachmentLease::Retiring),
        "the attachment view reports it Retiring"
    );
    let plans = pool.snapshot();
    assert_eq!(
        plans.keys().cloned().collect::<Vec<_>>(),
        vec![alloc(1)],
        "snapshot lists only the launch's plan: the reservation has none"
    );
    assert_eq!(plans[&alloc(1)].assignment().cid(), launched.assignment().cid());
    assert_ne!(
        observation.next_fit, fresh_cursor,
        "the launch moved the cursor; the reservation did not"
    );

    pool.reserve_leftover(&stuck, address_at(prefix, 9)).expect("a repeat is idempotent");
    assert_eq!(pool.lease_observation(), observation, "a repeat reservation changes nothing");
}

/// ADR-0350. A reserved offset is never handed to a launch, including after
/// next-fit wraps past the end of the prefix, and it stays reserved: with the
/// other four offsets taken the pool is exhausted, and a released launch's
/// offset is the one the next assign takes.
/// CONTRACT_SHAPE: bounded-change. Universe: assigned CIDs, the reservation.
#[test]
fn a_reserved_offset_survives_a_next_fit_wrap_and_is_never_assigned() {
    let prefix: Ipv4Net = SMALL_PREFIX.parse().expect("small prefix");
    let pool = pool_over(SMALL_PREFIX, 0x295);
    let usable = usable_offsets(prefix);
    assert_eq!(usable, vec![2, 3, 4, 5, 6], "precondition: five usable offsets");
    let stuck = leftover_id(0);
    pool.reserve_leftover(&stuck, address_at(prefix, 4)).expect("reserve offset 4");
    let reserved_cid = GuestCid::new(GUEST_CID_BASE + 4).expect("an in-range CID");

    let mut offsets = Vec::new();
    for key in 1..=4 {
        let plan = pool.assign(alloc(key), &[], test_owner_flow_ports()).expect("a free offset");
        assert_ne!(plan.assignment().cid(), reserved_cid, "the reserved CID is never handed out");
        offsets.push(offset_of(prefix, plan.assignment().workload_addr()));
    }
    assert_eq!(offsets, vec![2, 3, 5, 6], "next-fit skips the reserved offset");
    let exhausted = pool.assign(alloc(5), &[], test_owner_flow_ports());
    assert!(
        matches!(exhausted, Err(GuestNetworkError::PoolExhausted { .. })),
        "a full prefix of leases and one reservation is exhausted: {exhausted:?}"
    );

    // The cursor is past offset 6: releasing offset 5 and assigning wraps.
    pool.release(&alloc(3));
    let wrapped = pool.assign(alloc(6), &[], test_owner_flow_ports()).expect("the released offset");
    assert_eq!(offset_of(prefix, wrapped.assignment().workload_addr()), 5);
    assert_eq!(pool.lease_observation().leases[&stuck].cid(), reserved_cid, "still reserved");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// ADR-0350. Over any sequence of assigns and releases, a reserved offset's
    /// CID is never in a launch's plan and the reservation is still there at
    /// the end: only an explicit release of the reserved allocation frees it.
    /// CONTRACT_SHAPE: unbounded-preservation.
    #[test]
    fn no_assign_sequence_ever_hands_out_a_reserved_cid(
        reserved_index in 0_usize..14,
        operations in prop::collection::vec(lease_op(), 0..96),
    ) {
        let small: Ipv4Net = "100.95.0.0/28".parse().expect("small prefix");
        let pool = pool_over("100.95.0.0/28", 0x295);
        let usable = usable_offsets(small);
        let reserved_offset = usable[reserved_index % usable.len()];
        let stuck = leftover_id(0);
        pool.reserve_leftover(&stuck, address_at(small, reserved_offset)).expect("reserve");
        let reserved_cid = GuestCid::new(GUEST_CID_BASE + reserved_offset).expect("in range");
        for op in operations {
            match op {
                LeaseOp::Assign(key) => {
                    if let Ok(plan) = pool.assign(alloc(key), &[], test_owner_flow_ports()) {
                        prop_assert_ne!(plan.assignment().cid(), reserved_cid);
                    }
                }
                LeaseOp::Retire(key) => {
                    let _ = pool.retire(&alloc(key));
                }
                LeaseOp::Release(key) => pool.release(&alloc(key)),
            }
        }
        prop_assert_eq!(pool.lease_observation().leases[&stuck].cid(), reserved_cid);
        prop_assert!(pool.snapshot().values().all(|plan| plan.assignment().cid() != reserved_cid));
    }
}

/// ADR-0350, ADR-0156 release-last. `release` of the reservation returns its
/// offset: the occupancy returns to empty and the allocation can be assigned.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn releasing_a_reservation_returns_its_offset_to_the_pool() {
    let prefix: Ipv4Net = "100.95.0.0/16".parse().expect("node prefix");
    let pool = pool();
    let stuck = leftover_id(0);
    pool.reserve_leftover(&stuck, address_at(prefix, 9)).expect("reserve");

    pool.release(&stuck);

    let observed = pool.observe(std::slice::from_ref(&stuck));
    assert_eq!((observed.occupancy.held, observed.occupancy.retiring), (0, 0));
    assert!(observed.leases.is_empty(), "no lease remains");
    pool.assign(stuck, &[], test_owner_flow_ports()).expect("the allocation is assignable again");
}

/// ADR-0350 corrupt rows. An address no pool offset can hold is typed
/// `AddressOutsidePrefix` naming the allocation and the address, never a panic,
/// and the pool is unchanged. The cases are the address outside the prefix, the
/// gateway, and the prefix's network and broadcast addresses.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
fn a_reservation_outside_the_leased_range_is_a_typed_error_naming_the_allocation() {
    let prefix: Ipv4Net = "100.95.0.0/16".parse().expect("node prefix");
    let pool = pool();
    let before = pool.lease_observation();
    for (index, address) in [
        Ipv4Addr::new(10, 0, 0, 5),
        Ipv4Addr::new(100, 96, 0, 5),
        GATEWAY,
        prefix.network(),
        prefix.broadcast(),
    ]
    .into_iter()
    .enumerate()
    {
        let stuck = leftover_id(index);
        let error = pool
            .reserve_leftover(&stuck, address)
            .expect_err("an address the pool does not lease is corrupt");
        assert!(
            matches!(
                &error,
                CorruptLeftoverRow::AddressOutsidePrefix { allocation, address: named }
                    if *allocation == stuck && *named == address
            ),
            "{address}: {error:?}"
        );
        assert!(error.to_string().contains(stuck.as_str()), "the message names the allocation");
        assert_eq!(pool.lease_observation(), before, "a corrupt row reserves nothing");
    }
}

/// ADR-0350 corrupt rows. An offset another allocation already holds (an
/// Admitted launch's, or an earlier reservation's) and the same allocation at a
/// second offset are typed `OffsetAlreadyHeld`, naming the allocation and the
/// holder; the pool keeps its earlier lease.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
fn a_reservation_on_a_held_offset_is_a_typed_error_naming_both_allocations() {
    let prefix: Ipv4Net = "100.95.0.0/16".parse().expect("node prefix");
    let pool = pool();
    let launched = pool.assign(alloc(1), &[], test_owner_flow_ports()).expect("a launch admits");
    let launched_addr = launched.assignment().workload_addr();
    let first = leftover_id(1);
    let second = leftover_id(2);
    pool.reserve_leftover(&first, address_at(prefix, 9)).expect("reserve");
    let before = pool.lease_observation();

    let on_launch = pool.reserve_leftover(&second, launched_addr).expect_err("held by a launch");
    assert!(
        matches!(
            &on_launch,
            CorruptLeftoverRow::OffsetAlreadyHeld { allocation, held_by, .. }
                if *allocation == second && *held_by == alloc(1)
        ),
        "{on_launch:?}"
    );
    let on_reservation =
        pool.reserve_leftover(&second, address_at(prefix, 9)).expect_err("held by a reservation");
    assert!(
        matches!(
            &on_reservation,
            CorruptLeftoverRow::OffsetAlreadyHeld { allocation, held_by, .. }
                if *allocation == second && *held_by == first
        ),
        "{on_reservation:?}"
    );
    let moved = pool.reserve_leftover(&first, address_at(prefix, 10)).expect_err("a second offset");
    assert!(
        matches!(
            &moved,
            CorruptLeftoverRow::OffsetAlreadyHeld { allocation, held_by, .. }
                if *allocation == first && *held_by == first
        ),
        "{moved:?}"
    );
    for error in [&on_launch, &on_reservation, &moved] {
        assert!(
            error.to_string().contains(second.as_str())
                || error.to_string().contains(first.as_str())
        );
    }
    assert_eq!(pool.lease_observation(), before, "a corrupt row changes nothing");
}

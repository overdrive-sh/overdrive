//! The convergence property for `svid-lifecycle` (US-05 S1, S2, S3, S5).
//!
//! From an arbitrary starting `(Running allocations, held SVIDs, audit rows,
//! View)` and up to one dropped issuance (the audit write that precedes the
//! hold failing once), every Running allocation holds an SVID and no SVID is
//! held for an allocation that is not Running, within [`ROUNDS`] rounds; then
//! further rounds change nothing. The View generator includes `View::default()`
//! and seeded retry memory, which is pacing only: it must never keep an SVID
//! from being issued once its backoff has elapsed.
//!
//! The world also generates the crash-partial state the held leg of
//! `managed_targets` exists for (D-1a-i): an SVID held for an allocation of a
//! workload that has no allocation row at all, because the `StartAllocation`
//! dispatch holds the identity before `driver.start` and the row write that
//! follows failed. Only an evaluation of that workload can emit its `DropSvid`,
//! so the pass must reach a workload it can enumerate only through the held set.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use overdrive_core::SpiffeId;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::AllocationId;
use overdrive_core::reconcilers::ReconcilerName;
use overdrive_core::traits::observation_store::{AllocState, ObservationStoreError};
use overdrive_reconcilers::svid_lifecycle::{IssueRetry, SvidLifecycleView};
use overdrive_testing::placement;
use proptest::prelude::*;

use super::{
    Composed, ROUNDS, STABLE_ROUNDS, name_of, property_config, runtime, target_of, workload_id,
};

placement!(host_pure);

const WORKLOAD: &str = "payments";
const ALLOCS: [&str; 3] = ["payments-0", "payments-1", "payments-2"];
/// The workload whose SVID is held with no allocation row anywhere.
const ORPHAN_WORKLOAD: &str = "orphaned";
const ORPHAN_ALLOC: &str = "orphaned-0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    /// Issued and held.
    Held,
    /// Issued once (an audit row exists), the hold lost: the restart marker.
    IssuedThenLost,
    /// Never issued.
    Never,
}

#[derive(Debug, Clone, Copy)]
struct AllocWorld {
    /// The allocation row is Running (otherwise Terminated).
    running: bool,
    hold: Hold,
    /// Seeded retry memory for this allocation (attempts, seen just now).
    retry: Option<(u32, bool)>,
}

#[derive(Debug, Clone)]
struct World {
    allocs: Vec<AllocWorld>,
    /// The next issuance's audit write fails once.
    dropped_issue: bool,
    /// An SVID is held for `orphaned-0`, a workload with no allocation row
    /// (crash-partial: held, then the row write failed).
    orphan_hold: bool,
}

fn world() -> impl Strategy<Value = World> {
    (
        prop::collection::vec(
            (
                any::<bool>(),
                prop_oneof![Just(Hold::Held), Just(Hold::IssuedThenLost), Just(Hold::Never)],
                prop::option::of((0u32..=3, any::<bool>())),
            )
                .prop_map(|(running, hold, retry)| AllocWorld { running, hold, retry }),
            1..=ALLOCS.len(),
        ),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(allocs, dropped_issue, orphan_hold)| World {
            allocs,
            dropped_issue,
            orphan_hold,
        })
}

struct Fixture {
    node: Composed,
    name: ReconcilerName,
}

#[cfg(test)]
async fn fixture(world: &World) -> Fixture {
    let node = Composed::new(vec![crate::svid_lifecycle()]).await;
    let target = target_of("workload", WORKLOAD);
    let mut retry = BTreeMap::new();
    for (alloc_world, alloc) in world.allocs.iter().zip(ALLOCS) {
        let state = if alloc_world.running { AllocState::Running } else { AllocState::Terminated };
        node.write_alloc_row(WORKLOAD, alloc, state, WorkloadKind::Job, None).await;
        let alloc_id = AllocationId::new(alloc).expect("alloc id");
        let spiffe = SpiffeId::for_allocation(&workload_id(WORKLOAD), &alloc_id);
        if alloc_world.hold != Hold::Never {
            let material = crate::ca_issuance::issue_and_audit(
                node.state.ca.as_ref(),
                node.obs.as_ref(),
                node.clock.as_ref(),
                &node.state.node_id,
                &spiffe,
                overdrive_core::ca::issued_certificate_row::IssuedFor::Allocation(alloc_id.clone()),
            )
            .await
            .expect("pre-issue");
            if alloc_world.hold == Hold::Held {
                node.state.identity.hold(alloc_id.clone(), material);
            }
        }
        if let Some((attempts, seen_recently)) = alloc_world.retry {
            let seen = if seen_recently {
                UnixInstant::from_clock(&*node.clock)
            } else {
                UnixInstant::from_unix_duration(Duration::ZERO)
            };
            retry.insert(alloc_id, IssueRetry { attempts, last_failure_seen_at: seen });
        }
    }
    if world.orphan_hold {
        let alloc_id = AllocationId::new(ORPHAN_ALLOC).expect("alloc id");
        let spiffe = SpiffeId::for_allocation(&workload_id(ORPHAN_WORKLOAD), &alloc_id);
        let material = crate::ca_issuance::issue_and_audit(
            node.state.ca.as_ref(),
            node.obs.as_ref(),
            node.clock.as_ref(),
            &node.state.node_id,
            &spiffe,
            overdrive_core::ca::issued_certificate_row::IssuedFor::Allocation(alloc_id.clone()),
        )
        .await
        .expect("pre-issue the orphan");
        node.state.identity.hold(alloc_id, material);
    }
    if !retry.is_empty() {
        node.state.runtime.seed_svid_lifecycle_view_for_test(&target, SvidLifecycleView { retry });
    }
    if world.dropped_issue {
        node.obs.inject_write_failure(ObservationStoreError::Unreachable {
            peer: "dropped by the property's fault injection".to_owned(),
        });
    }
    Fixture { node, name: name_of("svid-lifecycle") }
}

impl Fixture {
    /// `actual == desired`: the held set equals the Running set.
    async fn converged(&self) -> bool {
        let running: BTreeSet<String> = self
            .node
            .alloc_rows()
            .await
            .into_iter()
            .filter(|(_, state)| *state == AllocState::Running)
            .map(|(alloc, _)| alloc)
            .collect();
        let held: BTreeSet<String> =
            self.node.state.identity.held_snapshot().keys().map(ToString::to_string).collect();
        running == held
    }

    fn held(&self) -> String {
        format!("{:?}", self.node.state.identity.held_snapshot())
    }
}

/// Outcome anchor: OUT-RCG-SVID-CONVERGES-FROM-ARBITRARY-STATE
/// US-05 S1, S2, S3, S5 for `svid-lifecycle`. Pending until the resync pass
/// exists: a Running allocation with no held SVID (a restart, a dropped
/// issuance) and a held SVID for an allocation that stopped are only
/// re-evaluated by the pass.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn svid_lifecycle_converges_the_held_set_to_the_running_set_from_any_state_then_stays_converged() {
    let runtime = runtime();
    proptest!(property_config(48, 351_006), |(world in world())| {
        runtime.block_on(async {
            let fixture = fixture(&world).await;

            let mut round = 0usize;
            while !fixture.converged().await && round < ROUNDS {
                round += 1;
                // A dropped issuance returns the shim's `IssueSvid` error: expected under the fault.
                fixture.node.round(&fixture.name, world.dropped_issue).await;
            }
            assert!(
                fixture.converged().await,
                "svid-lifecycle stalled with actual != desired after {ROUNDS} rounds \
                 (View/actual skew or a dropped issuance): world {world:?}, held {}, rows {:?}",
                fixture.held(),
                fixture.node.alloc_rows().await
            );

            let settled = fixture.held();
            for _ in 0..STABLE_ROUNDS {
                fixture.node.round(&fixture.name, false).await;
                prop_assert_eq!(
                    &fixture.held(),
                    &settled,
                    "a converged svid-lifecycle emitted an action that changed the held set"
                );
            }
            Ok(())
        })?;
    });
}

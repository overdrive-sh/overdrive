//! The convergence property for `workload-lifecycle` (US-05 S1, S2, S3, S5).
//!
//! From an arbitrary starting `(intent, allocation row, View)` and up to one
//! dropped dataplane-side effect (the driver rejecting the first start), the
//! workload reaches the desired replica count of Running allocations within
//! [`ROUNDS`] rounds (or none, after withdrawal), and further rounds change no
//! allocation row. The View generator includes `View::default()` (the
//! forgotten View) and seeded retry memory (attempts, a failure seen just now
//! or long ago) over the current allocation.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::AllocationId;
use overdrive_core::reconcilers::ReconcilerName;
use overdrive_core::traits::observation_store::AllocState;
use overdrive_reconcilers::workload_lifecycle::WorkloadLifecycleView;
use overdrive_testing::placement;
use proptest::prelude::*;

use super::{Composed, ROUNDS, STABLE_ROUNDS, name_of, property_config, runtime, target_of};

placement!(host_pure);

const WORKLOAD: &str = "payments";
const ALLOC: &str = "alloc-payments-0";

#[derive(Debug, Clone, Copy)]
struct World {
    /// The Job's intent is present (otherwise withdrawn).
    intent: bool,
    /// The allocation row the node left.
    row: Row,
    /// The reconciler's View for the workload (`None` is `View::default()`).
    view: Option<ViewSeed>,
    /// The driver rejects the first start it is asked to make.
    first_start_rejected: bool,
}

#[derive(Debug, Clone, Copy)]
enum Row {
    Absent,
    Running,
    /// Failed with no terminal claim: a workload failure the reconciler restarts.
    Failed,
}

#[derive(Debug, Clone, Copy)]
struct ViewSeed {
    /// Failures counted against the current allocation (below the restart ceiling).
    attempts: u32,
    /// The last failure was seen just now (inside the backoff) rather than long ago.
    seen_recently: bool,
}

fn world() -> impl Strategy<Value = World> {
    (
        any::<bool>(),
        prop_oneof![Just(Row::Absent), Just(Row::Running), Just(Row::Failed)],
        prop::option::of((0u32..=3, any::<bool>())),
        any::<bool>(),
    )
        .prop_map(|(intent, row, view, first_start_rejected)| World {
            intent,
            row,
            view: view.map(|(attempts, seen_recently)| ViewSeed { attempts, seen_recently }),
            first_start_rejected,
        })
}

struct Fixture {
    node: Composed,
    name: ReconcilerName,
}

#[cfg(test)]
async fn fixture(world: &World) -> Fixture {
    let node = Composed::new(vec![crate::workload_lifecycle()]).await;
    if world.intent {
        node.put_service(WORKLOAD).await;
    }
    match world.row {
        Row::Absent => {}
        Row::Running => {
            node.write_alloc_row(
                WORKLOAD,
                ALLOC,
                AllocState::Running,
                WorkloadKind::Service,
                Some(Ipv4Addr::new(100, 95, 0, 2)),
            )
            .await;
        }
        Row::Failed => {
            node.write_alloc_row(WORKLOAD, ALLOC, AllocState::Failed, WorkloadKind::Service, None)
                .await;
        }
    }
    let target = target_of("workload", WORKLOAD);
    if let Some(seed) = world.view {
        let alloc = AllocationId::new(ALLOC).expect("alloc id");
        let seen = if seed.seen_recently {
            UnixInstant::from_clock(&*node.clock)
        } else {
            UnixInstant::from_unix_duration(Duration::ZERO)
        };
        node.state.runtime.seed_workload_lifecycle_view_for_test(
            &target,
            WorkloadLifecycleView {
                restart_counts: BTreeMap::from([(alloc.clone(), seed.attempts)]),
                last_failure_seen_at: BTreeMap::from([(alloc, seen)]),
                ..Default::default()
            },
        );
    }
    if world.first_start_rejected {
        node.driver.script_next_start_rejection(Some("dropped by the property's fault injection"));
    }
    Fixture { node, name: name_of("workload-lifecycle") }
}

impl Fixture {
    /// `actual == desired` through the allocation rows the node observes: one
    /// Running allocation while the intent stands, none once it is withdrawn.
    async fn converged(&self, world: &World) -> bool {
        let running = self
            .node
            .alloc_rows()
            .await
            .iter()
            .filter(|(_, state)| *state == AllocState::Running)
            .count();
        running == usize::from(world.intent)
    }
}

/// Outcome anchor: OUT-RCG-WORKLOAD-CONVERGES-FROM-ARBITRARY-STATE
/// US-05 S1, S2, S3, S5 for `workload-lifecycle`. Pending until the resync pass
/// exists: a world whose first evaluation the edge never delivered (a failed
/// row with no wake, a missing allocation) is only re-evaluated by the pass.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn the_workload_converges_from_any_state_despite_a_dropped_start_then_stays_converged() {
    let runtime = runtime();
    proptest!(property_config(48, 351_005), |(world in world())| {
        runtime.block_on(async {
            let fixture = fixture(&world).await;

            let mut round = 0usize;
            while !fixture.converged(&world).await && round < ROUNDS {
                round += 1;
                // A rejected start lands as a Failed row, not a tick error: no error is expected.
                fixture.node.round(&fixture.name, false).await;
            }
            assert!(
                fixture.converged(&world).await,
                "workload-lifecycle stalled with actual != desired after {ROUNDS} rounds \
                 (View/actual skew or a dropped start): world {world:?}, rows {:?}",
                fixture.node.alloc_rows().await
            );

            let settled = fixture.node.alloc_rows().await;
            for _ in 0..STABLE_ROUNDS {
                fixture.node.round(&fixture.name, false).await;
                prop_assert_eq!(
                    &fixture.node.alloc_rows().await,
                    &settled,
                    "a converged workload emitted an action that changed an allocation row"
                );
            }
            Ok(())
        })?;
    });
}

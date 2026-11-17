//! The convergence property for `vm-reclamation` (US-05 S1, S2, S5).
//!
//! `vm-reclamation` is host-backed: it hydrates `actual` live from the host, so
//! the pass is its only trigger. From an arbitrary host (stranded scopes, run
//! directories and clones for allocations that have a Terminated row or no row)
//! the reclaimable artifacts are gone within [`ROUNDS`] rounds, and further
//! rounds change nothing. The reconciler's View is field-less, so there is no
//! View skew to generate (`VmReclamationView` carries nothing the reconciler
//! consults). The dropped-action stimulus is the host refusing a reclaim: up to
//! two `kill_scope` and two `discard_artifacts` calls fail (the
//! `SimVmHostState` scripted-failure seams) before one succeeds. A reconciler
//! that stops re-emitting after its first attempt leaves the artifact in place.

use std::io;
use std::path::PathBuf;

use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::AllocationId;
use overdrive_core::reconcilers::ReconcilerName;
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::traits::vm_host_state::{VmHostObservation, VmHostState};
use overdrive_testing::placement;
use proptest::prelude::*;

use super::{Composed, ROUNDS, STABLE_ROUNDS, name_of, property_config, runtime};

placement!(host_pure);

const WORKLOAD: &str = "vmjob";
const ALLOCS: [&str; 3] = ["vmjob-0", "vmjob-1", "vmjob-2"];
const MAX_DROPPED_RECLAIMS: usize = 2;

#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "four independent host surfaces and a row flag: each combination is a distinct world"
)]
struct AllocWorld {
    /// The allocation's row is Terminated (otherwise it has no row).
    terminal_row: bool,
    scope: bool,
    run_dir: bool,
    clone: bool,
}

impl AllocWorld {
    /// The surfaces the reconciler's table removes: every surface of a
    /// Terminated allocation, and the VM-exclusive surfaces (run directory and
    /// clone) of an allocation with no entry. A scope alone with no entry is
    /// shared with exec allocations and left alone.
    fn must_be_gone(self, id: &AllocationId, host: &VmHostObservation) -> bool {
        let artifacts_gone = !host.run_dirs.contains(id) && !host.clones.contains_key(id);
        if self.terminal_row {
            artifacts_gone && !host.scopes.contains_key(id)
        } else {
            artifacts_gone
        }
    }
}

/// One arbitrary starting host plus the reclaims the host refuses.
#[derive(Debug, Clone)]
struct World {
    allocs: Vec<AllocWorld>,
    /// `kill_scope` calls that fail before one succeeds.
    dropped_kills: usize,
    /// `discard_artifacts` calls that fail before one succeeds.
    dropped_discards: usize,
}

impl World {
    fn faults_armed(&self) -> bool {
        self.dropped_kills + self.dropped_discards > 0
    }
}

fn world() -> impl Strategy<Value = World> {
    (
        prop::collection::vec(
            (any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>())
                .prop_filter("an allocation is on the host", |(_, scope, run_dir, clone)| {
                    *scope || *run_dir || *clone
                })
                .prop_map(|(terminal_row, scope, run_dir, clone)| AllocWorld {
                    terminal_row,
                    scope,
                    run_dir,
                    clone,
                }),
            1..=ALLOCS.len(),
        ),
        0usize..=MAX_DROPPED_RECLAIMS,
        0usize..=MAX_DROPPED_RECLAIMS,
    )
        .prop_map(|(allocs, dropped_kills, dropped_discards)| World {
            allocs,
            dropped_kills,
            dropped_discards,
        })
}

struct Fixture {
    node: Composed,
    name: ReconcilerName,
}

#[cfg(test)]
async fn fixture(world: &World) -> Fixture {
    let node = Composed::new(vec![crate::vm_reclamation()]).await;
    // The workload is VM-driven, so its allocations are the reconciler's.
    node.put_job(WORKLOAD).await;
    for (alloc_world, alloc) in world.allocs.iter().zip(ALLOCS) {
        let id = AllocationId::new(alloc).expect("alloc id");
        if alloc_world.terminal_row {
            node.write_alloc_row(WORKLOAD, alloc, AllocState::Terminated, WorkloadKind::Job, None)
                .await;
        }
        if alloc_world.scope {
            node.vm_host.set_scope(id.clone(), std::collections::BTreeSet::new());
        }
        if alloc_world.run_dir {
            node.vm_host.set_run_dir(id.clone());
        }
        if alloc_world.clone {
            node.vm_host.set_clone(id, PathBuf::from(format!("/clones/{alloc}")));
        }
    }
    for _ in 0..world.dropped_kills {
        node.vm_host.script_kill_scope_failure(io::ErrorKind::PermissionDenied);
    }
    for _ in 0..world.dropped_discards {
        node.vm_host.script_discard_artifacts_failure(io::ErrorKind::PermissionDenied);
    }
    Fixture { node, name: name_of("vm-reclamation") }
}

impl Fixture {
    /// `actual == desired`: no reclaimable artifact remains on the host.
    async fn converged(&self, world: &World) -> bool {
        let host = self.node.vm_host.observe().await.expect("observe host");
        world.allocs.iter().zip(ALLOCS).all(|(alloc_world, alloc)| {
            let id = AllocationId::new(alloc).expect("alloc id");
            alloc_world.must_be_gone(&id, &host)
        })
    }

    async fn host(&self) -> String {
        format!("{:?}", self.node.vm_host.observe().await.expect("observe host"))
    }
}

/// Outcome anchor: OUT-RCG-VM-RECLAIM-CONVERGES-FROM-ARBITRARY-HOST
/// US-05 S1, S2, S3, S5 for `vm-reclamation`. Pending until the resync pass
/// exists: the reconciler is host-backed, so nothing but the pass evaluates it,
/// and a reclaim the host refused is retried only by a later pass.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "vm-reclamation stalled with actual != desired after 8 rounds")]
fn vm_reclamation_removes_every_reclaimable_artifact_from_any_host_then_stays_converged() {
    let runtime = runtime();
    proptest!(property_config(48, 351_008), |(world in world())| {
        runtime.block_on(async {
            let fixture = fixture(&world).await;

            let mut round = 0usize;
            while !fixture.converged(&world).await && round < ROUNDS {
                round += 1;
                // A refused reclaim returns the reclamation executor's error: expected under the fault.
                fixture.node.round(&fixture.name, world.faults_armed()).await;
            }
            assert!(
                fixture.converged(&world).await,
                "vm-reclamation stalled with actual != desired after {ROUNDS} rounds: \
                 world {world:?}, host {}",
                fixture.host().await
            );

            let settled = fixture.host().await;
            for _ in 0..STABLE_ROUNDS {
                fixture.node.round(&fixture.name, false).await;
                prop_assert_eq!(
                    &fixture.host().await,
                    &settled,
                    "a converged vm-reclamation emitted an action that changed the host"
                );
            }
            Ok(())
        })?;
    });
}

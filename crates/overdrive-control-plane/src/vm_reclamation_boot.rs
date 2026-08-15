//! `vm_reclamation_boot::converge` — the boot-epoch drive for
//! `VmReclamation` (ADR-0083 §D7, `brief.md` §105a.6, GH #42).
//!
//! Runs ONCE, in `run_server`, IMMEDIATELY BEFORE
//! `veth_provisioner::adopt_on_restart_recovery` — so any `rmdir` this
//! drive issues (via `VmHostState::kill_scope`'s settle postcondition)
//! has succeeded or returned `NotFound` before `adopt_on_restart_recovery`
//! reads the same cgroup tree via `alloc_scope_pids` (S-VM-23). It is
//! placed OUTSIDE the `state.mtls_worker.is_some()` gate — VM allocations
//! exist whether or not mTLS is composed.
//!
//! NOT gated on `Vmm` / `DriverRegistry::Vm` composition (S-VM-30):
//! `state.vm_host_state` is composed unconditionally
//! (`AppState::vm_host_state`'s own doc comment), so a node that
//! uninstalled `cloud-hypervisor` still observes and still reclaims —
//! `state.drivers.get(DriverType::Vm) == None` reads as
//! `SupervisionSet::Observed(BTreeSet::new())`, a KNOWN fact about the
//! world (no supervision handle exists) that AUTHORISES reclamation
//! rather than blocking it (brief.md §105a.3's composition table).
//!
//! Bar-2 converge-on-boot (`.claude/rules/reconcilers.md`): observe ->
//! `plan_reclamation` (the SAME pure diff the steady-state tick uses,
//! unchanged) -> execute. This is NOT a second implementation of the
//! diff — brief.md §105a.6: "One observation function, one pure diff,
//! one executor pair." The boot drive calls the executor directly
//! rather than routing through `action_shim::dispatch_single`, which
//! takes fifteen parameters (`driver`, `dataplane`, `ca`, `identity`,
//! `mtls_worker`, ...) none of which reclamation touches.
//!
//! `desired.allocations` (the intent-side two-surface join) is populated
//! by [`crate::reconciler_runtime::hydrate_vm_reclamation_desired`] —
//! the SAME function `reconciler_runtime::hydrate_desired`'s
//! `VmReclamation` arm calls for the steady-state tick (step 02-03;
//! brief.md §105a.6: "one observation function, one pure diff, one
//! executor pair" extends to the desired-side join). `Action::
//! ReclaimAllocation` is therefore reachable from this drive: a
//! first-seen or reboot-surviving VM allocation whose row is
//! non-terminal and whose claim is unsupervised (`Observed(∅)` at a
//! fresh boot, per the composition table above — a brand-new
//! `VmDriver`/`DriverRegistry` has not tracked anything yet) authors a
//! Platform Reclamation ending exactly as it would at a steady-state
//! tick — same input, same `plan_reclamation`, same executor.

use std::collections::{BTreeMap, BTreeSet};

use overdrive_core::reconcilers::{Action, VmReclamationState, plan_reclamation};
use overdrive_core::reconcilers::vm_reclamation::SupervisionSet;
use overdrive_core::traits::driver::DriverType;

use crate::AppState;
use crate::action_shim::reclamation::{
    ReclamationError, execute_discard_stranded_artifacts, execute_reclaim_allocation,
};
use crate::reconciler_runtime::hydrate_vm_reclamation_desired;

/// Run one boot-epoch `VmReclamation` pass against `state`. Idempotent —
/// safe to call on every boot regardless of whether any VM allocation
/// ever existed on this node (every surface then reads empty and
/// `plan_reclamation` returns `Vec::new()`).
///
/// # Errors
///
/// Returns [`ConvergeError::Desired`] when the desired-side two-surface
/// join fails to read the IntentStore or ObservationStore,
/// [`ConvergeError::Host`] on a genuine (non-benign-absence)
/// `VmHostState::observe` substrate failure, or
/// [`ConvergeError::Reclamation`] when an executor fails. Any of the
/// three refuses the whole boot (mirroring `adopt_on_restart_recovery`'s
/// own fail-closed posture) — the same cgroup tree
/// `adopt_on_restart_recovery` reads next must not be read out from
/// under a still-in-flight `rmdir`.
pub async fn converge(state: &AppState) -> Result<(), ConvergeError> {
    // brief.md §105a.2's pinned order, extended to the desired side by
    // §105a.6: rows (this drive's own `hydrate_vm_reclamation_desired`
    // call) FIRST, THEN `observe()`, supervision LAST. The
    // kill-authorising input is the freshest thing the pass reads; the
    // desired-side intent join is the stalest — exactly the SAME
    // hydration path and the SAME ordering argument the steady-state
    // tick uses (`reconciler_runtime::run_convergence_tick` calls
    // `hydrate_desired` before `hydrate_actual` on every tick).
    let allocations = hydrate_vm_reclamation_desired(state)
        .await
        .map_err(|e| ConvergeError::Desired(Box::new(e)))?;
    let desired = VmReclamationState { allocations, ..VmReclamationState::default() };

    let host = state.vm_host_state.observe().await.map_err(ConvergeError::Host)?;

    // Composition table (brief.md §105a.3): no `Vm` registry entry ⇒
    // `Observed(∅)` — authorising, not a missing observation (S-VM-30).
    let supervision = state.drivers.get(DriverType::Vm).map_or_else(
        || SupervisionSet::Observed(BTreeSet::new()),
        |driver| {
            driver
                .live_allocations()
                .map_or(SupervisionSet::Unavailable, |ids| {
                    SupervisionSet::Observed(ids.into_iter().collect())
                })
        },
    );

    let actual = VmReclamationState { allocations: BTreeMap::new(), host, supervision };

    for action in plan_reclamation(&desired, &actual) {
        match action {
            Action::DiscardStrandedArtifacts { alloc_id } => {
                execute_discard_stranded_artifacts(&alloc_id, state.vm_host_state.as_ref())
                    .await
                    .map_err(ConvergeError::Reclamation)?;
            }
            Action::ReclaimAllocation { alloc_id } => {
                execute_reclaim_allocation(
                    &alloc_id,
                    state.vm_host_state.as_ref(),
                    state.obs.as_ref(),
                    state.clock.as_ref(),
                    &state.node_id,
                    state.runtime.broker_mutex(),
                )
                .await
                .map_err(ConvergeError::Reclamation)?;
            }
            // `plan_reclamation` (this reconciler's own pure diff) never
            // emits any other `Action` variant — the wildcard exists
            // only because `Action` is the workspace-shared action enum;
            // no production `Vec<Action>` reaching this loop can
            // populate it.
            _ => {}
        }
    }
    Ok(())
}

/// Failure surface for [`converge`].
#[derive(Debug, thiserror::Error)]
pub enum ConvergeError {
    /// The desired-side two-surface join
    /// ([`hydrate_vm_reclamation_desired`]) failed to read the
    /// IntentStore or ObservationStore. Boxed: `ConvergenceError` embeds
    /// `ControlPlaneError` (its `ViewPersist` variant), which itself
    /// embeds `ConvergeError` (`VmReclamationBoot`) — an unboxed payload
    /// here closes that cycle into infinite-size recursion (E0072).
    #[error("desired-side hydration failed: {0}")]
    Desired(Box<crate::reconciler_runtime::ConvergenceError>),
    /// `VmHostState::observe` failed at the substrate level (a
    /// non-benign-absence I/O error).
    #[error("VmHostState observe failed: {0}")]
    Host(std::io::Error),
    /// A reclamation executor failed.
    #[error("reclamation executor failed: {0}")]
    Reclamation(#[from] ReclamationError),
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use overdrive_core::id::NodeId;
    use overdrive_core::traits::driver::{Driver, DriverType};
    use overdrive_core::traits::intent_store::IntentStore;
    use overdrive_core::traits::observation_store::ObservationStore;
    use overdrive_core::traits::vm_host_state::VmHostState;
    use overdrive_sim::adapters::vm_host_state::SimVmHostState;
    use overdrive_store_local::LocalIntentStore;
    use tempfile::TempDir;

    use super::*;
    use crate::reconciler_runtime::ReconcilerRuntime;

    /// Mirrors `build_app_state` in
    /// `tests/acceptance/runtime_registers_noop_heartbeat.rs` — the
    /// established `AppState::new` fixture shape for this crate's own
    /// unit tests — with `state.vm_host_state` overridden to the
    /// caller-supplied adapter afterward (the convenience constructor
    /// default-composes `NoopVmHostState`, which every test here needs
    /// to replace with a seeded `SimVmHostState`).
    fn app_state(
        tmp: &TempDir,
        vm_host_state: Arc<dyn overdrive_core::traits::vm_host_state::VmHostState>,
    ) -> AppState {
        let runtime =
            ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
        let obs: Arc<dyn ObservationStore> =
            Arc::new(overdrive_sim::adapters::observation_store::SimObservationStore::single_peer(
                NodeId::new("local").expect("NodeId"),
                0,
            ));
        let driver: Arc<dyn Driver> =
            Arc::new(overdrive_sim::adapters::driver::SimDriver::new(DriverType::Exec));
        let allocator = crate::test_default_allocator(
            Arc::clone(&store) as Arc<dyn overdrive_core::traits::intent_store::IntentStore>
        );
        let mut state = AppState::new(
            store,
            store_path,
            obs,
            Arc::new(runtime),
            driver,
            Arc::new(overdrive_sim::adapters::clock::SimClock::new()),
            Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
            Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
                overdrive_sim::adapters::entropy::SimEntropy::new(0),
            ))),
            Arc::new(crate::identity_mgr::IdentityMgr::new(None)),
            NodeId::new("writer-1").expect("valid NodeId"),
            allocator,
            crate::test_empty_listener_facts(),
            std::net::Ipv4Addr::LOCALHOST,
        );
        state.vm_host_state = vm_host_state;
        state
    }

    #[tokio::test]
    async fn converge_over_empty_host_state_is_a_noop() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let state = app_state(&tmp, Arc::new(host));
        converge(&state).await.expect("empty host state converges cleanly");
    }

    #[tokio::test]
    async fn converge_discards_a_run_dir_orphan_with_no_supervision_and_no_row() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let alloc = overdrive_core::AllocationId::new("vm-boot-orphan-0").expect("valid id");
        // The fixture's driver is `SimDriver::new(DriverType::Exec)` — no
        // `Vm` registry entry at all, so supervision reads
        // Observed(∅) -> reclamation_authorised(alloc) is true.
        host.set_run_dir(alloc.clone());
        let state = app_state(&tmp, Arc::new(host.clone()));

        converge(&state).await.expect("converge succeeds");

        let observed = host.observe().await.expect("observe succeeds");
        assert!(
            !observed.run_dirs.contains(&alloc),
            "the run-dir orphan must be discarded by the boot drive; still present: {observed:?}"
        );
    }

    /// Step 02-03 completion — the desired-side join makes
    /// `Action::ReclaimAllocation` reachable from THIS drive (previously
    /// only `DiscardStrandedArtifacts` was reachable; see this module's
    /// own docs). A non-terminal `AllocStatusRow` owned by a Vm-driven
    /// `Job` intent, with a live cgroup scope and no `Vm` registry entry
    /// (`SimDriver::new(DriverType::Exec)` — the same fixture the
    /// `converge_discards_a_run_dir_orphan_...` test above uses),
    /// authorises reclamation: `supervision` reads `Observed(∅)`, the
    /// join finds `desired.allocations[alloc] = VmAllocFacts { terminal:
    /// false, .. }`, and `plan_reclamation`'s row 1 fires
    /// `ReclaimAllocation` — writing `Terminated` /
    /// `StoppedBy::PlatformReclaimed`, the disposition ONLY that
    /// executor ever writes (never `DiscardStrandedArtifacts`, which
    /// authors no row at all).
    #[tokio::test]
    async fn converge_reclaims_a_non_terminal_unsupervised_vm_driven_allocation() {
        use overdrive_core::TransitionReason;
        use overdrive_core::aggregate::{IntentKey, Job, Vm, WorkloadDriver, WorkloadIntent, WorkloadKind};
        use overdrive_core::id::{AllocationId, WorkloadId};
        use overdrive_core::traits::driver::Resources;
        use overdrive_core::traits::observation_store::{
            AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow,
        };
        use overdrive_core::transition_reason::StoppedBy;

        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let alloc = AllocationId::new("vm-boot-desired-0").expect("valid id");
        // A live cgroup scope is the minimal fixture that brings this
        // alloc_id into `plan_reclamation`'s `host_ids` walk (any of the
        // three `VmHostObservation` surfaces suffices).
        host.set_scope(alloc.clone(), std::collections::BTreeSet::from([4242]));
        let state = app_state(&tmp, Arc::new(host));

        // Seed a Vm-driven Job intent — the desired-side surface
        // `hydrate_vm_reclamation_desired` now scans via `scan_prefix`.
        let workload_id = WorkloadId::new("vm-boot-desired-workload").expect("valid id");
        let job = Job {
            id: workload_id.clone(),
            replicas: std::num::NonZeroU32::new(1).expect("nonzero"),
            resources: Resources { cpu_milli: 500, memory_bytes: 134_217_728 },
            driver: WorkloadDriver::Vm(Vm {
                command: "/sbin/init".to_owned(),
                args: Vec::new(),
                kernel: "/boot/vmlinux".to_owned(),
                rootfs: "/boot/rootfs.ext4".to_owned(),
            }),
        };
        let intent = WorkloadIntent::Job(job);
        let key = IntentKey::for_workload(&workload_id);
        let archived = intent.archive_for_store().expect("rkyv archive");
        state.store.put(key.as_bytes(), archived.as_ref()).await.expect("put intent");

        // Seed a NON-TERMINAL AllocStatusRow for `alloc` under
        // `workload_id` — the observation-side surface of the join.
        let n = NodeId::new("writer-1").expect("valid NodeId");
        let row = AllocStatusRow {
            alloc_id: alloc.clone(),
            workload_id: workload_id.clone(),
            node_id: n.clone(),
            state: AllocState::Running,
            updated_at: LogicalTimestamp { counter: 1, writer: n.clone() },
            reason: Some(TransitionReason::Started),
            detail: None,
            terminal: None,
            stderr_tail: None,
            kind: WorkloadKind::Job,
            listeners: Vec::new(),
            started_at: None,
            workload_addr: None,
            last_terminated: None,
            restart_count: 0,
        };
        state.obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("seed row");

        converge(&state).await.expect("converge succeeds");

        let after = state
            .obs
            .alloc_status_row(&alloc)
            .await
            .expect("read succeeds")
            .expect("row exists after converge");
        assert_eq!(after.state, AllocState::Terminated);
        assert!(
            matches!(after.reason, Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed })),
            "a non-terminal, unsupervised, Vm-driven allocation reached via the desired-side \
             join must be reclaimed via Action::ReclaimAllocation, got {:?}",
            after.reason
        );
    }
}

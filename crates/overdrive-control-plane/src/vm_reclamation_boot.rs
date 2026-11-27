//! `vm_reclamation_boot::converge` — the boot-epoch drive for
//! `VmReclamation` (ADR-0083 §D7, `brief.md` §105a.6, GH #42).
//!
//! Runs ONCE, in `run_server`, before any later boot step reads the cgroup
//! tree — so any `rmdir` this drive issues (via `VmHostState::kill_scope`'s
//! settle postcondition) has succeeded or returned `NotFound` first
//! (S-VM-23). It is
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
use std::io::ErrorKind;

use overdrive_core::id::AllocationId;
use overdrive_core::reconcilers::Action;
use overdrive_core::traits::driver::DriverType;
use overdrive_core::traits::observation_store::{AllocState, LogicalTimestamp, TransitionSource};
use overdrive_core::transition_reason::TransitionReason;
use overdrive_reconcilers::vm_reclamation::SupervisionSet;
use overdrive_reconcilers::{VmReclamationState, plan_reclamation};

use crate::AppState;
use crate::action_shim::abandoned_launch_row;
use crate::action_shim::reclamation::{
    ReclamationError, execute_discard_stranded_artifacts, execute_reclaim_allocation_at_boot,
};
use crate::reconciler_runtime::build_hydration_context;

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
/// three refuses the whole boot (fail-closed) — the cgroup tree later boot
/// steps read must not be read out from under a still-in-flight `rmdir`.
#[expect(
    clippy::significant_drop_tightening,
    reason = "the allocator/listener_facts MutexGuards are lent into the HydrationContext \
              borrow-bundle and must outlive the desired-side join .await"
)]
pub async fn converge(state: &AppState) -> Result<(), ConvergeError> {
    // ADR-0355 W4: the first step closes every launch the previous control
    // plane left unfinished, before anything is observed or killed, so the
    // executors below meet a terminal row and never write a second reason for
    // the same fact.
    close_lost_launches(state).await?;

    // brief.md §105a.2's pinned order, extended to the desired side by
    // §105a.6: rows (this drive's own `hydrate_vm_reclamation_desired`
    // call) FIRST, THEN `observe()`, supervision LAST. The
    // kill-authorising input is the freshest thing the pass reads; the
    // desired-side intent join is the stalest — exactly the SAME
    // hydration path and the SAME ordering argument the steady-state
    // tick uses (`reconciler_runtime::run_convergence_tick` calls
    // `hydrate_desired` before `hydrate_actual` on every tick).
    // Post-ADR-0086 S3: the desired-side join moved onto the `VmReclamation`
    // impl in `overdrive-reconcilers` and reads through a `HydrationContext`.
    // Build the same per-tick borrow-bundle the steady-state loop uses and call
    // the SAME shared join (brief.md §105a.6 "one observation function").
    // The scoped block is the minimal hydration window: the guards are lent into
    // the `HydrationContext` and must outlive the desired-side join .await
    // (suppressed at the fn level — rust-1.95.0's `significant_drop_tightening`
    // cannot see the borrow-bundle that forces the guards to be held).
    let allocations = {
        let allocator = state.allocator.lock().await;
        let listener_facts = state.listener_facts.lock().await;
        let ctx = build_hydration_context(state, &allocator, &listener_facts);
        overdrive_reconcilers::vm_reclamation::hydrate_vm_reclamation_desired(&ctx)
            .await
            .map_err(|e| ConvergeError::Desired(Box::new(e.into())))?
    };
    let desired = VmReclamationState { allocations, ..VmReclamationState::default() };

    let host = state.vm_host_state.observe().await.map_err(ConvergeError::Host)?;

    // Composition table (brief.md §105a.3): no `Vm` registry entry ⇒
    // `Observed(∅)` — authorising, not a missing observation (S-VM-30).
    let supervision = state.drivers.get(DriverType::Vm).map_or_else(
        || SupervisionSet::Observed(BTreeSet::new()),
        |driver| {
            driver.live_allocations().map_or(SupervisionSet::Unavailable, |ids| {
                SupervisionSet::Observed(ids.into_iter().collect())
            })
        },
    );

    let actual = VmReclamationState { allocations: BTreeMap::new(), host, supervision };

    // ADR-0350: every executor runs to completion together, so boot waits one
    // settle bound however many VMs are stuck. Results keep the planner's order.
    let executions = plan_reclamation(&desired, &actual).into_iter().filter_map(|action| {
        match action {
            Action::DiscardStrandedArtifacts { alloc_id } => Some(Execution::Discard(alloc_id)),
            Action::ReclaimAllocation { alloc_id } => Some(Execution::Reclaim(alloc_id)),
            // `plan_reclamation` (this reconciler's own pure diff) never emits
            // any other `Action` variant — the wildcard exists only because
            // `Action` is the workspace-shared action enum.
            _ => None,
        }
    });
    let outcomes = futures::future::join_all(executions.map(|execution| async move {
        let result = execution.run(state).await;
        (execution.into_alloc_id(), result)
    }))
    .await;

    // Every busy scope's offset is reserved before any error is returned: one
    // scope's failure never drops another's reservation. The boot refuses with
    // the first non-busy executor error in action order, else with the first
    // reservation failure.
    let mut first_executor_error: Option<ReclamationError> = None;
    let mut first_reservation_error: Option<ReclamationError> = None;
    for (alloc_id, result) in outcomes {
        match result {
            Ok(()) => {}
            // Both executors can meet a busy scope: the discard executor kills a
            // scope with no row too, and a busy scope never refuses the boot.
            Err(ReclamationError::Host(error)) if error.kind() == ErrorKind::ResourceBusy => {
                if let (Err(error), None) =
                    (reserve_busy_scope(state, &alloc_id).await, &first_reservation_error)
                {
                    first_reservation_error = Some(error);
                }
            }
            Err(other) => {
                first_executor_error.get_or_insert(other);
            }
        }
    }
    first_executor_error
        .or(first_reservation_error)
        .map_or(Ok(()), |error| Err(ConvergeError::Reclamation(error)))
}

/// One reclamation executor the planner asked for.
enum Execution {
    Discard(AllocationId),
    Reclaim(AllocationId),
}

impl Execution {
    async fn run(&self, state: &AppState) -> Result<(), ReclamationError> {
        match self {
            Self::Discard(alloc_id) => {
                execute_discard_stranded_artifacts(
                    alloc_id,
                    &state.drivers,
                    state.vm_host_state.as_ref(),
                )
                .await
            }
            Self::Reclaim(alloc_id) => {
                execute_reclaim_allocation_at_boot(
                    alloc_id,
                    &state.drivers,
                    state.vm_host_state.as_ref(),
                    state.obs.as_ref(),
                    state.clock.as_ref(),
                    &state.node_id,
                    &state.runtime,
                )
                .await
            }
        }
    }

    fn into_alloc_id(self) -> AllocationId {
        match self {
            Self::Discard(alloc_id) | Self::Reclaim(alloc_id) => alloc_id,
        }
    }
}

/// Reserve the guest address the row of a busy (stuck) VM recorded, so no
/// launch is handed its CID while the VMM still holds its device (ADR-0350).
/// A row with no address, or no row, reserves nothing: the address is on record
/// before the VMM exists, so such a VM holds no CID.
async fn reserve_busy_scope(
    state: &AppState,
    alloc_id: &AllocationId,
) -> Result<(), ReclamationError> {
    let Some(row) = state.obs.alloc_status_row(alloc_id).await? else {
        return Ok(());
    };
    let Some(address) = row.workload_addr else {
        return Ok(());
    };
    state.guest_pool.reserve_leftover(alloc_id, address)?;
    Ok(())
}

/// End every `Pending` row of this node `Terminated` /
/// `LaunchLostWithControlPlane` with the row's address carried (ADR-0355 W4).
///
/// A boot process owns no launch from before the restart, and boot
/// reclamation completes before the convergence loop dispatches anything, so
/// every `Pending` row of this node at this point belongs to a dead launch,
/// whether or not its VM left a scope, a run directory or a clone. The step is
/// idempotent (the row it writes is terminal), reserves nothing and kills
/// nothing. Another node's `Pending` row is not this boot's launch.
async fn close_lost_launches(state: &AppState) -> Result<(), ReclamationError> {
    let rows = state.obs.alloc_status_rows().await?;
    for pending in
        rows.iter().filter(|row| row.state == AllocState::Pending && row.node_id == state.node_id)
    {
        // The closing row dominates the `Pending` row through its prior; no tick
        // floor is needed, and a wall-clock value is not a Lamport counter.
        let updated_at =
            LogicalTimestamp::dominating(0, state.node_id.clone(), Some(&pending.updated_at));
        let closed =
            abandoned_launch_row(pending, TransitionReason::LaunchLostWithControlPlane, updated_at);
        state.obs.write_alloc_lifecycle(closed, TransitionSource::Reconciler).await?;
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
    use overdrive_core::byte_size::ByteSize;
    use overdrive_core::cpu_quantity::CpuQuantity;
    use std::sync::Arc;

    use overdrive_core::id::NodeId;
    use overdrive_core::traits::driver::{Driver, DriverType};
    use overdrive_core::traits::observation_store::ObservationStore;
    use overdrive_core::traits::vm_host_state::VmHostState;
    use overdrive_sim::adapters::vm_host_state::SimVmHostState;
    use overdrive_store_local::LocalIntentStore;
    use tempfile::TempDir;

    use super::*;
    use crate::reconciler_runtime::ReconcilerRuntime;
    use overdrive_testing::placement;

    placement!(host_pure);

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
            Arc::new(overdrive_sim::adapters::driver::SimDriver::new(DriverType::Unikernel));
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
            crate::shared_network_test_ports::mtls_worker(),
            crate::shared_network_test_ports::shared_guest_network_owner(),
            crate::shared_network_test_ports::guest_network_exec_gate(),
            crate::shared_network_test_ports::guest_pool(),
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
        // The fixture composes a non-VM driver, so there is no `Vm`
        // registry entry and supervision reads
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

    /// CONTRACT_SHAPE: pure-function.
    #[allow(
        clippy::too_many_lines,
        reason = "one pure state-transition scenario keeps claim, executor postcondition, and lifecycle redrive joined"
    )]
    #[test]
    fn same_boot_epoch_claims_each_unsupervised_allocation_once() {
        use std::num::NonZeroU32;
        use std::time::{Duration, Instant};

        use overdrive_core::aggregate::{
            Job, Node, NodeSpecInput, Vm, WorkloadDriver, WorkloadKind,
        };
        use overdrive_core::id::{AllocationId, WorkloadId};
        use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
        use overdrive_core::traits::driver::Resources;
        use overdrive_core::traits::observation_store::{
            AllocState, AllocStatusRow, LogicalTimestamp,
        };
        use overdrive_core::traits::vm_host_state::{ScopeFacts, VmHostObservation};
        use overdrive_core::transition_reason::{StoppedBy, TransitionReason};
        use overdrive_core::wall_clock::UnixInstant;
        use overdrive_reconcilers::vm_reclamation::{SupervisionSet, VmAllocFacts};
        use overdrive_reconcilers::{
            WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView,
        };

        let alloc = AllocationId::new("vm-boot-desired-0").expect("valid allocation");
        let workload_id = WorkloadId::new("vm-boot-desired-workload").expect("valid workload");
        let node = Node::new(NodeSpecInput {
            id: "local".to_owned(),
            region: "test".to_owned(),
            cpu: CpuQuantity::from_millis(1_000),
            memory: ByteSize::new(1 << 30),
        })
        .expect("valid node");
        let job = Job {
            id: workload_id.clone(),
            replicas: NonZeroU32::new(1).expect("nonzero"),
            resources: Resources {
                cpu: CpuQuantity::from_millis(500),
                memory: ByteSize::new(134_217_728),
            },
            driver: WorkloadDriver::Vm(Vm {
                volume: None,
                command: "/sbin/init".to_owned(),
                args: Vec::new(),
                kernel: "/boot/vmlinux".to_owned(),
                rootfs: "/boot/rootfs.ext4".to_owned(),
            }),
        };

        let reclaim_desired = VmReclamationState {
            allocations: BTreeMap::from([(
                alloc.clone(),
                VmAllocFacts { workload_id: workload_id.clone(), terminal: false },
            )]),
            ..VmReclamationState::default()
        };
        let reclaim_actual = VmReclamationState {
            allocations: BTreeMap::new(),
            host: VmHostObservation {
                scopes: BTreeMap::from([(alloc.clone(), ScopeFacts::default())]),
                ..VmHostObservation::default()
            },
            supervision: SupervisionSet::Observed(BTreeSet::new()),
        };
        assert_eq!(
            plan_reclamation(&reclaim_desired, &reclaim_actual),
            vec![Action::ReclaimAllocation { alloc_id: alloc.clone() }],
        );

        // The production executor's state delta is a reclaimed row plus an
        // empty host observation. Feeding that exact postcondition back into
        // the same pure planner cannot mint a second reclamation claim.
        let reclaimed_row = AllocStatusRow {
            alloc_id: alloc.clone(),
            workload_id: workload_id.clone(),
            node_id: node.id.clone(),
            state: AllocState::Terminated,
            updated_at: LogicalTimestamp { counter: 1, writer: node.id.clone() },
            reason: Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed }),
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
        let claimed_desired = VmReclamationState {
            allocations: BTreeMap::from([(
                alloc.clone(),
                VmAllocFacts { workload_id: workload_id.clone(), terminal: true },
            )]),
            ..VmReclamationState::default()
        };
        let claimed_actual = VmReclamationState {
            supervision: SupervisionSet::Observed(BTreeSet::new()),
            ..VmReclamationState::default()
        };
        assert!(plan_reclamation(&claimed_desired, &claimed_actual).is_empty());

        // The standing intent consumes that one platform claim exactly once.
        // Each lifecycle evaluation reserves a distinct successor; re-driving
        // the unchanged predecessor advances above the prior reservation.
        let lifecycle_desired = WorkloadLifecycleState {
            current_startup_probe: None,
            workload_id,
            job: Some(job),
            desired_to_stop: false,
            generation: 0,
            nodes: BTreeMap::from([(node.id.clone(), node)]),
            allocations: BTreeMap::new(),
            workload_kind: WorkloadKind::Job,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
            declared_listeners: Vec::new(),
            guest_attachments: overdrive_core::traits::GuestAttachmentObservation {
                occupancy: overdrive_core::guest_network::GuestAttachmentOccupancy {
                    held: 0,
                    retiring: 0,
                },
                leases: std::collections::BTreeMap::new(),
            },
        };
        let lifecycle_actual = WorkloadLifecycleState {
            current_startup_probe: None,
            job: None,
            nodes: BTreeMap::new(),
            allocations: BTreeMap::from([(alloc.clone(), reclaimed_row)]),
            ..lifecycle_desired.clone()
        };
        let view = WorkloadLifecycleView::default();
        let now = Instant::now();
        let tick = TickContext {
            now,
            now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_100)),
            tick: 1,
            deadline: now + Duration::from_secs(1),
        };
        let lifecycle = WorkloadLifecycle::canonical();
        let (first_actions, next_view) =
            lifecycle.reconcile(&lifecycle_desired, &lifecycle_actual, &view, &tick);
        let successor_one =
            AllocationId::new("alloc-vm-boot-desired-workload-1").expect("valid successor");
        assert_eq!(
            first_actions
                .iter()
                .filter(|action| matches!(
                    action,
                    Action::RestartAllocation { alloc_id, spec, .. }
                        if alloc_id == &alloc && spec.alloc == successor_one
                ))
                .count(),
            1,
        );
        assert_eq!(next_view.restart_counts.get(&successor_one), Some(&0));
        let (repeated_actions, repeated_view) =
            lifecycle.reconcile(&lifecycle_desired, &lifecycle_actual, &next_view, &tick);
        let successor_two =
            AllocationId::new("alloc-vm-boot-desired-workload-2").expect("valid successor");
        assert_eq!(
            repeated_actions
                .iter()
                .filter(|action| matches!(
                    action,
                    Action::RestartAllocation { alloc_id, spec, .. }
                        if alloc_id == &alloc && spec.alloc == successor_two
                ))
                .count(),
            1,
        );
        assert_eq!(repeated_view.restart_counts.get(&successor_one), Some(&0));
        assert_eq!(repeated_view.restart_counts.get(&successor_two), Some(&0));
    }

    /// Step 02-03 completion — the desired-side join makes
    /// `Action::ReclaimAllocation` reachable from THIS drive (previously
    /// only `DiscardStrandedArtifacts` was reachable; see this module's
    /// own docs). A non-terminal `AllocStatusRow` owned by a Vm-driven
    /// `Job` intent, with a live cgroup scope and no `Vm` registry entry
    /// (a non-VM simulated driver — the same fixture the
    /// `converge_discards_a_run_dir_orphan_...` test above uses),
    /// authorises reclamation: `supervision` reads `Observed(∅)`, the
    /// join finds `desired.allocations[alloc] = VmAllocFacts { terminal:
    /// false, .. }`, and `plan_reclamation`'s row 1 fires
    /// `ReclaimAllocation` — writing `Terminated` /
    /// `StoppedBy::PlatformReclaimed`, the disposition ONLY that
    /// executor ever writes (never `DiscardStrandedArtifacts`, which
    /// authors no row at all).
    /// CONTRACT_SHAPE: bounded-change.
    #[allow(
        clippy::too_many_lines,
        reason = "one boot-to-lifecycle scenario keeps the reclaim and fresh-successor redrive evidence joined"
    )]
    #[tokio::test]
    async fn boot_reclamation_executor_persists_one_platform_claim_and_one_lifecycle_redrive() {
        use overdrive_core::TransitionReason;
        use overdrive_core::aggregate::{
            IntentKey, Job, Vm, WorkloadDriver, WorkloadIntent, WorkloadKind,
        };
        use overdrive_core::id::{AllocationId, WorkloadId};
        use overdrive_core::reconcilers::{Action, TargetResource, TickContext};
        use overdrive_core::traits::driver::Resources;
        use overdrive_core::traits::observation_store::{
            AllocState, AllocStatusRow, LogicalTimestamp,
        };
        use overdrive_core::transition_reason::StoppedBy;
        use overdrive_core::wall_clock::UnixInstant;
        use overdrive_reconcilers::{
            AnyReconciler, AnyReconcilerView, WorkloadLifecycle, WorkloadLifecycleView,
        };

        use crate::reconciler_runtime::{hydrate_actual_for_test, hydrate_desired_for_test};

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
            resources: Resources {
                cpu: CpuQuantity::from_millis(500),
                memory: ByteSize::new(134_217_728),
            },
            driver: WorkloadDriver::Vm(Vm {
                volume: None,
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
        state
            .obs
            .write_alloc_lifecycle(
                row,
                overdrive_core::traits::observation_store::TransitionSource::Reconciler,
            )
            .await
            .expect("seed row");

        converge(&state).await.expect("converge succeeds");

        let after = state
            .obs
            .alloc_status_row(&alloc)
            .await
            .expect("read succeeds")
            .expect("row exists after converge");
        assert_eq!(after.state, AllocState::Terminated);
        assert!(
            matches!(
                after.reason,
                Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed })
            ),
            "a non-terminal, unsupervised, Vm-driven allocation reached via the desired-side \
             join must be reclaimed via Action::ReclaimAllocation, got {:?}",
            after.reason
        );

        // Join the boot claim to the standing-intent lifecycle boundary. The
        // first evaluation reserves one fresh successor; re-evaluating the
        // identical reclaimed row with the returned private view advances to
        // a second fresh reservation rather than reusing an issued ID.
        let lifecycle = AnyReconciler::WorkloadLifecycle(WorkloadLifecycle::canonical());
        let target =
            TargetResource::new(&format!("workload/{workload_id}")).expect("valid workload target");
        let lifecycle_desired = hydrate_desired_for_test(&lifecycle, &target, &state)
            .await
            .expect("hydrate standing intent");
        let lifecycle_actual = hydrate_actual_for_test(&lifecycle, &target, &state)
            .await
            .expect("hydrate reclaimed allocation");
        let view = AnyReconcilerView::WorkloadLifecycle(WorkloadLifecycleView::default());
        let now = std::time::Instant::now();
        let tick = TickContext {
            now,
            now_unix: UnixInstant::from_unix_duration(std::time::Duration::from_secs(
                1_700_000_100,
            )),
            tick: 1,
            deadline: now + std::time::Duration::from_secs(1),
        };
        let (first_actions, next_view) =
            lifecycle.reconcile(&lifecycle_desired, &lifecycle_actual, &view, &tick);
        let successor_one =
            AllocationId::new("alloc-vm-boot-desired-workload-1").expect("valid successor");
        let first_redrives = first_actions
            .iter()
            .filter(|action| {
                matches!(
                    action,
                    Action::RestartAllocation { alloc_id, spec, .. }
                        if alloc_id == &alloc && spec.alloc == successor_one
                )
            })
            .count();
        assert_eq!(
            first_redrives, 1,
            "one Platform Reclamation claim emits exactly one fresh-successor lifecycle re-drive: {first_actions:#?}"
        );
        let (repeated_actions, repeated_view) =
            lifecycle.reconcile(&lifecycle_desired, &lifecycle_actual, &next_view, &tick);
        let successor_two =
            AllocationId::new("alloc-vm-boot-desired-workload-2").expect("valid successor");
        assert_eq!(
            repeated_actions
                .iter()
                .filter(|action| matches!(
                    action,
                    Action::RestartAllocation { alloc_id, spec, .. }
                        if alloc_id == &alloc && spec.alloc == successor_two
                ))
                .count(),
            1,
            "same boot re-drive must advance above the issued successor: {repeated_actions:#?}"
        );
        let AnyReconcilerView::WorkloadLifecycle(repeated_view) = repeated_view else {
            panic!("workload lifecycle returned the wrong View variant")
        };
        assert_eq!(repeated_view.restart_counts.get(&successor_one), Some(&0));
        assert_eq!(repeated_view.restart_counts.get(&successor_two), Some(&0));

        converge(&state).await.expect("a repeated same-boot convergence is a no-op");
        let repeated = state
            .obs
            .alloc_status_row(&alloc)
            .await
            .expect("read succeeds")
            .expect("row remains present");
        assert_eq!(
            repeated, after,
            "the terminal reclamation row is the claim fence: a same-boot retry authors no duplicate"
        );
    }
}

/// Test support (DISTILL): the host a `serve` composition runs on, simulated
/// over one [`overdrive_sim::SimVmm`] that outlives every composition — the
/// shape the boot-reclamation cells below and the `cid_lease` conformance
/// driver (`guest_network::quint_cid_lease`) share.
///
/// - [`SimVmmHost`] is a test-local [`VmHostState`](overdrive_core::traits::vm_host_state::VmHostState)
///   over the `SimVmm`: every simulated VMM process the host knows of has a
///   cgroup scope and a run directory (a VM-exclusive surface, so an
///   unsupervised one is reclaimable); `kill_scope` ends the scope's VMM
///   processes through [`overdrive_sim::SimVmm::kill`], the SIGKILL a real
///   `cgroup.kill` delivers. Built `gated`, it holds each kill until the test
///   releases it ([`SimVmmHost::grant_kill`]) — the stepping the conformance
///   driver needs to replay one `Reclaim(o)` at a time.
/// - [`RecordingVmm`] is a test-local `Vmm` decorator that tells the host
///   which allocation each created VMM process belongs to (the scope the VM
///   driver places it in).
/// - [`compose`] builds one `serve` composition over them: a fresh
///   `GuestAddressPool`, a `VmDriver` (beacon intake bound over a stepped
///   `SimBeaconIo`), and the `AppState` boot reclamation reads.
///
/// None of this authors a production decision: the pool leases, the VM driver
/// launches and stops, and `converge` decides what to reclaim.
#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::redundant_pub_crate,
    reason = "test fixtures assert their preconditions; the fixture items are shared with sibling test modules"
)]
pub(crate) mod sim_vm_host {
    use overdrive_core::byte_size::ByteSize;
    use overdrive_core::cpu_quantity::CpuQuantity;
    use std::collections::{BTreeMap, BTreeSet};
    use std::net::Ipv4Addr;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use ipnet::Ipv4Net;
    use overdrive_core::SpiffeId;
    use overdrive_core::cgroup::CgroupPath;
    use overdrive_core::guest_transport::GuestCid;
    use overdrive_core::id::{AllocationId, NodeId};
    use overdrive_core::traits::driver::{
        AllocationSpec, Driver, DriverPayload, Resources, VmPayload,
    };
    use overdrive_core::traits::intent_store::IntentStore;
    use overdrive_core::traits::observation_store::ObservationStore;
    use overdrive_core::traits::vm_host_state::{
        ScopeFacts, VmHostObservation, VmHostState, VmHostStateProbeError,
    };
    use overdrive_core::traits::vmm::{
        Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmProbeError,
    };
    use overdrive_core::vm::beacon::BeaconIo;
    use overdrive_core::vm::config::{
        Gid, HostArch, KERNEL_MAGIC_WINDOW, VmConfig, VmConfinement, VmmIdentity,
    };
    use overdrive_sim::adapters::beacon_io::SimBeaconIo;
    use overdrive_sim::adapters::clock::SimClock;
    use overdrive_sim::adapters::entropy::SimEntropy;
    use overdrive_sim::adapters::observation_store::SimObservationStore;
    use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
    use overdrive_sim::{SimCgroupAccounting, SimCgroupFs, SimVmm};
    use overdrive_store_local::LocalIntentStore;
    use overdrive_worker::probe_runner::ProbeRunner;
    use overdrive_worker::{BeaconIntake, VmDriver, VmHostLayout};
    use parking_lot::Mutex;
    use tokio::sync::Notify;

    use crate::AppState;
    use crate::guest_network::{GuestAddressPool, GuestNetworkPlan};
    use crate::reconciler_runtime::ReconcilerRuntime;

    #[derive(Debug, Default)]
    struct HostFacts {
        /// Every VM the host knows of: its scope's VMM pids, until the scope
        /// is killed and its run directory discarded.
        vms: BTreeMap<AllocationId, BTreeSet<u32>>,
        /// Kills of these pids are held until granted.
        gated: bool,
        /// Pids whose kill `kill_scope` is waiting for.
        requested: BTreeSet<u32>,
        /// Pids whose kill the test released.
        granted: BTreeSet<u32>,
        /// Allocations whose VMM was killed and not reaped: the scope stays
        /// until a `kill_scope` finds every process reaped.
        lingering: BTreeSet<AllocationId>,
        /// Allocations whose run directory was discarded. A run directory is a
        /// separate surface from the scope: discarding it never removes the
        /// scope, and an unreaped process keeps the scope.
        run_dirs_discarded: BTreeSet<AllocationId>,
        /// Pids whose VMM cannot be reaped: the kill `kill_scope` delivers
        /// ends them `KilledNotReaped` (the bound waited on the VMM's clock)
        /// and the scope stays busy until a scripted reap.
        stuck: BTreeSet<u32>,
    }

    /// The simulated host's VM surfaces over one `SimVmm`.
    pub(crate) struct SimVmmHost {
        vmm: SimVmm,
        facts: Mutex<HostFacts>,
        wake: Notify,
    }

    impl SimVmmHost {
        pub(crate) fn new(vmm: SimVmm, gated: bool) -> Arc<Self> {
            Arc::new(Self {
                vmm,
                facts: Mutex::new(HostFacts { gated, ..HostFacts::default() }),
                wake: Notify::new(),
            })
        }

        /// `pid`'s VMM was killed and not reaped: its allocation's scope stays.
        fn mark_lingering(&self, pid: u32) {
            let mut facts = self.facts.lock();
            let owner = facts
                .vms
                .iter()
                .find(|(_, pids)| pids.contains(&pid))
                .map(|(alloc, _)| alloc.clone());
            if let Some(alloc) = owner {
                facts.lingering.insert(alloc);
            }
        }

        fn record(&self, alloc: AllocationId, pid: u32) {
            let mut facts = self.facts.lock();
            facts.run_dirs_discarded.remove(&alloc);
            facts.vms.entry(alloc).or_default().insert(pid);
        }

        /// The pids whose kill a `kill_scope` call is waiting for (gated).
        #[cfg(all(test, feature = "integration-tests"))]
        pub(crate) fn kills_requested(&self) -> BTreeSet<u32> {
            self.facts.lock().requested.clone()
        }

        /// The VMM `pid` cannot be reaped: the kill `kill_scope` delivers to
        /// it leaves its scope busy (`ResourceBusy`) until `SimVmm::exit_process`
        /// reaps it. `false` when `pid` is not a live process.
        #[cfg(all(test, feature = "integration-tests"))]
        pub(crate) fn make_unreapable(&self, pid: u32) -> bool {
            if !self.vmm.make_process_unreapable(pid) {
                return false;
            }
            self.facts.lock().stuck.insert(pid);
            true
        }

        /// Release the held kill of `pid` (gated).
        #[cfg(all(test, feature = "integration-tests"))]
        pub(crate) fn grant_kill(&self, pid: u32) {
            self.facts.lock().granted.insert(pid);
            self.wake.notify_waiters();
        }

        /// Wake a waiting `kill_scope` to re-check its processes (after a
        /// VMM exited by itself).
        #[cfg(all(test, feature = "integration-tests"))]
        pub(crate) fn notify(&self) {
            self.wake.notify_waiters();
        }

        /// Forget every outstanding kill request and grant: the `serve`
        /// process whose boot reclamation made them died with them.
        #[cfg(all(test, feature = "integration-tests"))]
        pub(crate) fn clear_requests(&self) {
            let mut facts = self.facts.lock();
            facts.requested.clear();
            facts.granted.clear();
        }

        /// The VMM pids the host placed in `alloc`'s scope (the relabelling
        /// of a production allocation to its VMM processes).
        #[cfg(all(test, feature = "integration-tests"))]
        pub(crate) fn pids_of(&self, alloc: &AllocationId) -> BTreeSet<u32> {
            self.facts.lock().vms.get(alloc).cloned().unwrap_or_default()
        }

        fn scope_alloc(&self, scope: &CgroupPath) -> Option<AllocationId> {
            self.facts
                .lock()
                .vms
                .keys()
                .find(|alloc| CgroupPath::for_alloc(alloc) == *scope)
                .cloned()
        }
    }

    #[async_trait]
    impl VmHostState for SimVmmHost {
        fn kind(&self) -> &'static str {
            "fixture-sim-vmm-host"
        }

        async fn probe(&self) -> Result<(), VmHostStateProbeError> {
            Ok(())
        }

        async fn observe(&self) -> std::io::Result<VmHostObservation> {
            let facts = self.facts.lock();
            let mut observation = VmHostObservation::default();
            for (alloc, pids) in &facts.vms {
                // `cgroup.procs` lists every member task: a running VMM and a
                // killed one that is not reaped (a stuck task stays a member).
                let members: BTreeSet<u32> = pids
                    .iter()
                    .copied()
                    .filter(|pid| self.vmm.is_live(*pid) || self.vmm.is_unreaped(*pid))
                    .collect();
                if !members.is_empty() || facts.lingering.contains(alloc) {
                    observation.scopes.insert(alloc.clone(), ScopeFacts { pids: members });
                }
                if !facts.run_dirs_discarded.contains(alloc) {
                    observation.run_dirs.insert(alloc.clone());
                }
            }
            drop(facts);
            Ok(observation)
        }

        async fn kill_scope(&self, scope: &CgroupPath) -> std::io::Result<()> {
            let Some(alloc) = self.scope_alloc(scope) else { return Ok(()) };
            loop {
                let notified = self.wake.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let ready = {
                    let mut facts = self.facts.lock();
                    let pids = facts.vms.get(&alloc).cloned().unwrap_or_default();
                    let live: Vec<u32> =
                        pids.into_iter().filter(|pid| self.vmm.is_live(*pid)).collect();
                    let held: Vec<u32> = live
                        .iter()
                        .copied()
                        .filter(|pid| facts.gated && !facts.granted.contains(pid))
                        .collect();
                    if held.is_empty() {
                        for pid in &live {
                            facts.requested.remove(pid);
                        }
                        Some(live)
                    } else {
                        facts.requested.extend(held);
                        None
                    }
                };
                if let Some(live) = ready {
                    for pid in live {
                        let stuck = self.facts.lock().stuck.contains(&pid);
                        if stuck {
                            // SIGKILL is delivered; the process is never
                            // reaped: `terminate` waits the reap bound on the
                            // VMM's clock and reports `KilledNotReaped`.
                            let control =
                                VmControl { pid, api_socket: PathBuf::from("/sim/stuck/api.sock") };
                            let _ = self.vmm.terminate(&control, Duration::ZERO).await;
                            self.mark_lingering(pid);
                        } else {
                            self.vmm.kill(pid);
                        }
                    }
                    let mut facts = self.facts.lock();
                    let unreaped = facts
                        .vms
                        .get(&alloc)
                        .is_some_and(|pids| pids.iter().any(|pid| self.vmm.is_unreaped(*pid)));
                    if unreaped {
                        return Err(std::io::ErrorKind::ResourceBusy.into());
                    }
                    facts.lingering.remove(&alloc);
                    drop(facts);
                    return Ok(());
                }
                notified.await;
            }
        }

        async fn discard_artifacts(&self, alloc: &AllocationId) -> std::io::Result<()> {
            // Removes the run directory only; the scope of an unreaped process
            // persists (the kernel removes a scope only once it is empty).
            self.facts.lock().run_dirs_discarded.insert(alloc.clone());
            Ok(())
        }
    }

    /// `Vmm` decorator: records each created VMM process under its
    /// allocation on the host, then forwards unchanged.
    pub(crate) struct RecordingVmm {
        pub(crate) inner: SimVmm,
        pub(crate) host: Arc<SimVmmHost>,
    }

    #[async_trait]
    impl Vmm for RecordingVmm {
        fn kind(&self) -> &'static str {
            self.inner.kind()
        }

        async fn probe(&self) -> Result<(), VmmProbeError> {
            self.inner.probe().await
        }

        async fn create(&self, config: &VmConfig, cid: GuestCid) -> VmmResult<VmProcess> {
            let process = self.inner.create(config, cid).await?;
            self.host.record(config.alloc.clone(), process.control.pid);
            Ok(process)
        }

        async fn terminate(
            &self,
            control: &VmControl,
            grace: Duration,
        ) -> VmmResult<VmTermination> {
            let outcome = self.inner.terminate(control, grace).await;
            if matches!(outcome, Ok(VmTermination::KilledNotReaped)) {
                self.host.mark_lingering(control.pid);
            }
            outcome
        }
    }

    /// One `serve` composition: pool, VM driver, its beacon listener, and the
    /// `AppState` boot reclamation reads.
    pub(crate) struct VmComposition {
        pub(crate) state: AppState,
        pub(crate) driver: VmDriver,
        pub(crate) pool: Arc<GuestAddressPool>,
        pub(crate) root: PathBuf,
    }

    /// The node gateway of every fixture pool.
    pub(crate) const GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);

    fn stage_artifacts(root: &Path) {
        std::fs::create_dir_all(root).expect("composition root");
        std::fs::write(root.join("master.img"), b"deterministic-fixture-rootfs-bytes")
            .expect("write synthetic master rootfs");
        let mut header = vec![0_u8; KERNEL_MAGIC_WINDOW];
        header[..4].copy_from_slice(b"\x7fELF");
        std::fs::write(root.join("vmlinuz"), &header).expect("stage the synthetic kernel image");
    }

    /// Build one `serve` composition under `root` over the host's `vmm`.
    pub(crate) async fn compose(
        root: &Path,
        vmm: &SimVmm,
        host: &Arc<SimVmmHost>,
        prefix: Ipv4Net,
        seed: u64,
    ) -> VmComposition {
        stage_artifacts(root);
        let clock = SimClock::new();
        let beacon = Arc::new(SimBeaconIo::stepped(vmm.clone()));
        let intake = BeaconIntake::bind(
            Arc::clone(&beacon) as Arc<dyn BeaconIo>,
            overdrive_core::guest_transport::VsockListenPort::KernelAssigned,
        )
        .await
        .expect("the sim beacon listener binds");
        let wiring =
            overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(clock.clone()));
        assert!(wiring.supervisor().open_after_boot(), "fixture EXEC gate opens");
        let guest_listener_board = Arc::new(overdrive_worker::GuestListenerBoard::new());
        let probe_runner = Arc::new(ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::new(clock.clone()),
            Arc::new(SimObservationStore::single_peer(
                NodeId::new("cid-lease").expect("valid node ID"),
                0,
            )) as Arc<dyn ObservationStore>,
            Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
            Arc::clone(&guest_listener_board)
                as Arc<dyn overdrive_core::traits::GuestListenerReports>,
        ));
        let driver = VmDriver::new(
            Arc::new(RecordingVmm { inner: vmm.clone(), host: Arc::clone(host) }),
            Arc::new(clock.clone()),
            Arc::new(SimCgroupFs::new()),
            Arc::new(SimCgroupAccounting::new()),
            probe_runner,
            wiring.gate(),
            VmHostLayout {
                cgroup_root: root.join("cgroup"),
                run_dir_root: root.join("run"),
                clone_index_dir: root.join("clone-index"),
                clone_staging_dir: root.join("clone-staging"),
                arch: HostArch::X86_64,
                confinement: VmConfinement::confined(
                    VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
                    1024,
                ),
            },
            intake,
            guest_listener_board,
            None,
            std::sync::Arc::new(overdrive_sim::SimVmHostState::new()),
        );
        let pool =
            Arc::new(GuestAddressPool::new(prefix, GATEWAY, Arc::new(SimEntropy::new(seed))));

        let runtime =
            ReconcilerRuntime::new_with_redb_view_store_for_test(root).expect("runtime::new");
        let store_path = root.join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
        let obs: Arc<dyn ObservationStore> =
            Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("NodeId"), 0));
        let allocator = crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
        let mut state = AppState::new(
            store,
            store_path,
            obs,
            Arc::new(runtime),
            Arc::new(driver.clone()) as Arc<dyn Driver>,
            Arc::new(clock.clone()),
            Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
            Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(SimEntropy::new(0)))),
            Arc::new(crate::identity_mgr::IdentityMgr::new(None)),
            NodeId::new("writer-1").expect("valid NodeId"),
            allocator,
            crate::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            crate::shared_network_test_ports::mtls_worker(),
            crate::shared_network_test_ports::shared_guest_network_owner(),
            crate::shared_network_test_ports::guest_network_exec_gate(),
            Arc::clone(&pool),
        );
        state.vm_host_state = Arc::clone(host) as Arc<dyn VmHostState>;
        VmComposition { state, driver, pool, root: root.to_path_buf() }
    }

    /// The VM spec of one lease: the plan's transport (leased CID and launch
    /// token) over the artifacts staged under the composition's `root`.
    pub(crate) fn vm_spec(root: &Path, plan: &GuestNetworkPlan) -> AllocationSpec {
        AllocationSpec {
            declared_listeners: Vec::new(),
            alloc: plan.alloc().clone(),
            identity: SpiffeId::new("spiffe://overdrive.local/workload/cid-lease")
                .expect("valid spiffe id"),
            driver: DriverPayload::Vm(VmPayload {
                volume: None,
                command: "/sbin/init".to_owned(),
                args: vec![],
                kernel: root.join("vmlinuz"),
                rootfs: root.join("master.img"),
            }),
            resources: Resources {
                cpu: CpuQuantity::from_millis(100),
                memory: ByteSize::new(128 * 1024 * 1024),
            },
            probe_descriptors: Vec::new(),
            transport: Some(plan.assignment().clone()),
        }
    }
}

/// Boot reclamation before any `assign` (NO-SURVIVE, CID-NOCLAIM; Gate G-V0,
/// V-RECLAIM). One `SimVmm` outlives the compositions; a dropped composition
/// is a crashed `serve` whose VMMs survive as orphans holding their devices.
/// Observed at the driven ports: `SimVmm::vsock_device_holders` (QPROJ-7
/// arbiter; Tier-3 pair `overdrive-host` `vsock_vmm_device`, V-30, V-RECLAIM)
/// and the pool's `lease_observation` (QPROJ-2).
#[cfg(test)]
#[allow(clippy::expect_used, reason = "test fixtures assert their preconditions")]
mod boot_reclamation_before_assign {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use ipnet::Ipv4Net;
    use overdrive_core::id::AllocationId;
    use overdrive_core::traits::driver::Driver;
    use overdrive_sim::SimVmm;
    use tempfile::TempDir;

    use super::converge;
    use super::sim_vm_host::{SimVmmHost, VmComposition, compose, vm_spec};
    use overdrive_testing::placement;

    placement!(host_pure);

    const PREFIX: &str = "100.95.0.0/29";

    fn alloc(name: &str) -> AllocationId {
        AllocationId::new(name).expect("valid allocation id")
    }

    /// Lease `name` and launch its VM; returns once the VMM opened its
    /// device or its start ended: the VMM's pid (0 when nothing holds the
    /// CID), whether the launch holds its device with its start still
    /// pending (no guest READY in these cells), and the start task.
    async fn lease_and_launch(
        composition: &VmComposition,
        vmm: &SimVmm,
        name: &str,
    ) -> (u32, bool, tokio::task::JoinHandle<()>) {
        let plan = composition
            .pool
            .assign(alloc(name), &[], crate::shared_network_test_ports::test_owner_flow_ports())
            .expect("a free offset");
        let cid = plan.assignment().cid();
        let spec = vm_spec(&composition.root, &plan);
        let driver = composition.driver.clone();
        let start = tokio::spawn(async move {
            let _ =
                driver.start(&spec, &overdrive_sim::adapters::SimLaunchSupersession::new()).await;
        });
        for _ in 0..10_000 {
            if vmm.vsock_device_holders().contains_key(&cid) || start.is_finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let held = vmm.vsock_device_holders().get(&cid).copied();
        (held.unwrap_or(0), !start.is_finished() && held.is_some(), start)
    }

    /// A `serve` crash leaves its VMM running and holding its CID's device;
    /// the restarted process's boot reclamation kills it before the pool's
    /// first `assign`, so that `assign` — which takes the same first usable
    /// offset, hence the same CID — launches a VMM that opens the device.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn boot_reclamation_ends_the_previous_processs_vmm_before_the_first_assign() {
        let tmp = TempDir::new().expect("tmpdir");
        let prefix: Ipv4Net = PREFIX.parse().expect("prefix");
        let vmm = SimVmm::new(std::sync::Arc::new(overdrive_sim::adapters::clock::SimClock::new()));
        let host = SimVmmHost::new(vmm.clone(), false);

        let crashed = compose(&tmp.path().join("serve-1"), &vmm, &host, prefix, 1).await;
        let (orphan, launched, start) = lease_and_launch(&crashed, &vmm, "orphan-0").await;
        assert!(launched, "precondition: the first process's VMM holds its device");
        let orphan_cid = crashed.pool.lease_observation().leases[&alloc("orphan-0")].cid();
        // `serve` crashes: its tasks and composition die; its VMM does not.
        start.abort();
        drop(crashed);
        assert_eq!(
            vmm.vsock_device_holders(),
            BTreeMap::from([(orphan_cid, orphan)]),
            "the crashed process's VMM survives as an orphan holding its device"
        );

        let restarted = compose(&tmp.path().join("serve-2"), &vmm, &host, prefix, 2).await;
        assert!(
            restarted.pool.lease_observation().leases.is_empty(),
            "no lease survives (NO-SURVIVE)"
        );
        converge(&restarted.state).await.expect("boot reclamation converges");
        assert!(
            vmm.vsock_device_holders().is_empty(),
            "the orphan was killed and its device released"
        );
        assert!(!vmm.is_live(orphan));

        let (fresh, launched, _start) = lease_and_launch(&restarted, &vmm, "fresh-0").await;
        let fresh_cid = restarted.pool.lease_observation().leases[&alloc("fresh-0")].cid();
        assert_eq!(fresh_cid, orphan_cid, "the first assign takes the reclaimed CID again");
        assert!(launched, "the new VMM opened the reclaimed CID's device: no clash");
        assert_eq!(vmm.vsock_device_holders(), BTreeMap::from([(fresh_cid, fresh)]));
    }

    /// R5-55 (d), D-UP-19 — With two VMs running, a `serve` crash leaves both
    /// as orphans holding their devices; the restarted process's one boot
    /// reclamation (one `converge` call) kills both before the first
    /// `assign`, and the new leases launch on the reclaimed CIDs without a
    /// clash.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn one_boot_reclamation_kills_every_orphan_before_the_first_assign() {
        let tmp = TempDir::new().expect("tmpdir");
        let prefix: Ipv4Net = PREFIX.parse().expect("prefix");
        let vmm = SimVmm::new(std::sync::Arc::new(overdrive_sim::adapters::clock::SimClock::new()));
        let host = SimVmmHost::new(vmm.clone(), false);

        let crashed = compose(&tmp.path().join("serve-1"), &vmm, &host, prefix, 1).await;
        let (first, launched_first, start_first) =
            lease_and_launch(&crashed, &vmm, "orphan-0").await;
        let (second, launched_second, start_second) =
            lease_and_launch(&crashed, &vmm, "orphan-1").await;
        assert!(launched_first && launched_second, "precondition: both VMMs hold their devices");
        let orphans = crashed.pool.lease_observation();
        let cids =
            [orphans.leases[&alloc("orphan-0")].cid(), orphans.leases[&alloc("orphan-1")].cid()];
        start_first.abort();
        start_second.abort();
        drop(crashed);
        assert_eq!(
            vmm.vsock_device_holders(),
            BTreeMap::from([(cids[0], first), (cids[1], second)]),
            "both VMMs survive the crash as orphans"
        );

        let restarted = compose(&tmp.path().join("serve-2"), &vmm, &host, prefix, 2).await;
        converge(&restarted.state).await.expect("one boot reclamation converges");
        assert!(!vmm.is_live(first) && !vmm.is_live(second), "every orphan was killed");
        assert!(vmm.vsock_device_holders().is_empty(), "every orphan device was released");

        let (fresh_first, launched_first, _a) = lease_and_launch(&restarted, &vmm, "fresh-0").await;
        let (fresh_second, launched_second, _b) =
            lease_and_launch(&restarted, &vmm, "fresh-1").await;
        let fresh = restarted.pool.lease_observation();
        assert_eq!(
            [fresh.leases[&alloc("fresh-0")].cid(), fresh.leases[&alloc("fresh-1")].cid()],
            cids,
            "the first assigns take the reclaimed CIDs again (next-fit from a fresh cursor)"
        );
        assert!(launched_first && launched_second, "no launch clashes on a reclaimed CID");
        assert_eq!(
            vmm.vsock_device_holders(),
            BTreeMap::from([(cids[0], fresh_first), (cids[1], fresh_second)])
        );
    }

    /// The counterexample G-V0 rules out: an `assign` before boot reclamation
    /// hands the orphan's CID to a new lease, and that launch's VMM exits at
    /// device setup (the existing pre-READY VMM-exit start failure) while the
    /// orphan keeps its device.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn a_launch_on_an_orphans_cid_before_reclamation_fails_at_device_setup() {
        let tmp = TempDir::new().expect("tmpdir");
        let prefix: Ipv4Net = PREFIX.parse().expect("prefix");
        let vmm = SimVmm::new(std::sync::Arc::new(overdrive_sim::adapters::clock::SimClock::new()));
        let host = SimVmmHost::new(vmm.clone(), false);

        let crashed = compose(&tmp.path().join("serve-1"), &vmm, &host, prefix, 1).await;
        let (orphan, _, start) = lease_and_launch(&crashed, &vmm, "orphan-0").await;
        let orphan_cid = crashed.pool.lease_observation().leases[&alloc("orphan-0")].cid();
        start.abort();
        drop(crashed);

        let restarted = compose(&tmp.path().join("serve-2"), &vmm, &host, prefix, 2).await;
        let plan = restarted
            .pool
            .assign(
                alloc("early-0"),
                &[],
                crate::shared_network_test_ports::test_owner_flow_ports(),
            )
            .expect("a free offset");
        assert_eq!(plan.assignment().cid(), orphan_cid, "precondition: the orphan's CID");
        let spec = vm_spec(&restarted.root, &plan);
        let outcome = tokio::time::timeout(
            Duration::from_secs(10),
            restarted.driver.start(&spec, &overdrive_sim::adapters::SimLaunchSupersession::new()),
        )
        .await
        .expect("the start ends");
        assert!(outcome.is_err(), "the VMM exits before READY: a start failure, got {outcome:?}");
        assert_eq!(
            vmm.vsock_device_holders(),
            BTreeMap::from([(orphan_cid, orphan)]),
            "the orphan keeps its device; the refused launch held nothing"
        );
    }

    /// Boot reclamation ends only the VMMs the new process does not
    /// supervise: a VMM the restarted process launched itself keeps running
    /// with its device.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn boot_reclamation_leaves_a_supervised_vmm_running() {
        let tmp = TempDir::new().expect("tmpdir");
        let prefix: Ipv4Net = PREFIX.parse().expect("prefix");
        let vmm = SimVmm::new(std::sync::Arc::new(overdrive_sim::adapters::clock::SimClock::new()));
        let host = SimVmmHost::new(vmm.clone(), false);

        let composition = compose(&tmp.path().join("serve-1"), &vmm, &host, prefix, 1).await;
        let (supervised, launched, _start) =
            lease_and_launch(&composition, &vmm, "supervised-0").await;
        assert!(launched, "precondition: the VMM holds its device");
        let before = vmm.vsock_device_holders();

        converge(&composition.state).await.expect("boot reclamation converges");
        assert!(vmm.is_live(supervised), "a supervised VMM is not reclaimed");
        assert_eq!(vmm.vsock_device_holders(), before, "its device is untouched");
        assert!(
            composition
                .driver
                .live_allocations()
                .is_some_and(|live| live.contains(&alloc("supervised-0"))),
            "the driver still supervises it"
        );
    }
}

/// Boot reclamation ends a VMM that was still booting when `serve` died,
/// before any launch for its workload, and its volume claim drains so the
/// successor's launch attaches the volume without a `VolumeUnavailable { Held }`
/// row and with nothing charged (M7; feature delta § Launch path, Gate G-2).
///
/// Two boots of the production composition, `run_server_with_obs_and_driver`
/// (the boot, its `VmReclamation` boot-epoch drive, the convergence loop, the
/// public HTTPS API), over the same data directory. One `SimVmm`, one
/// `SimVolumeBacking`, one `SimObservationStore`, one `SimVmmHost` and one
/// `SimClock` outlive both. The workload is deployed through the public API
/// (`POST /v1/workloads`, the declaration with a `[vm.volume]`), so the
/// convergence loop, the action shim and the `VmDriver` launch it; the launch
/// parks in the guest-boot wait (no guest READY is delivered) with its VMM
/// holding the claim. `ServerHandle::kill_for_test` is the process death of a
/// crashed `serve`: its VMM survives as an orphan. The second boot runs the
/// boot reclamation as its own boot step; nothing in the test calls
/// `converge`.
///
/// Test support models the orphan's claim: the recording `Vmm` links the
/// workload's `SimVolumeBacking` claim to the created process's liveness
/// (`hold_claim_while_live`), so killing the VMM releases its claim.
///
/// Oracle, at driven ports only: `SimVmm::is_live` (the orphan's pid), what
/// the recording `Vmm` was asked to create and, at each create, the claim the
/// pool reported and whether the orphan was still live,
/// `SimVolumeBacking::claim_state`, and the observation store's rows.
///
/// PENDING: the production spec parser refuses `[vm.volume]`, so the deploy is
/// refused and each test stops at "a workload declaring a volume must be
/// accepted"; its reclamation assertions are first exercised when the spec
/// surface, the launch path and `SimVolumeBacking` land.
///
/// Tier 1, in-process over `Sim*` adapters. `host_pure`.
#[cfg(all(test, feature = "integration-tests"))]
#[allow(
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::panic,
    reason = "test fixtures assert their preconditions"
)]
mod mid_boot_volume_reclamation {
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use overdrive_core::UnixInstant;
    use overdrive_core::aggregate::WorkloadKind;
    use overdrive_core::guest_network::GuestNetworkExecWiring;
    use overdrive_core::guest_transport::GuestCid;
    use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
    use overdrive_core::testing::volume::volume_size_gib;
    use overdrive_core::traits::clock::Clock;
    use overdrive_core::traits::driver::{
        AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType,
        ExitEvent, LaunchSupersession, Resources,
    };
    use overdrive_core::traits::observation_store::{
        AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, TransitionSource,
    };
    use overdrive_core::traits::vm_host_state::VmHostState;
    use overdrive_core::traits::vmm::{
        Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmProbeError,
    };
    use overdrive_core::traits::volume_backing::{ClaimState, VolumeBacking, VolumePoolProbe};
    use overdrive_core::transition_reason::TransitionReason;
    use overdrive_core::vm::beacon::BeaconIo;
    use overdrive_core::vm::config::{
        Gid, HostArch, KERNEL_MAGIC_WINDOW, VmConfig, VmConfinement, VmmIdentity,
    };
    use overdrive_sim::adapters::beacon_io::SimBeaconIo;
    use overdrive_sim::adapters::observation_store::SimObservationStore;
    use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
    use overdrive_sim::{
        SimCgroupAccounting, SimCgroupFs, SimVmm, SimVolumeBacking, SimVolumePoolProbe,
    };
    use overdrive_testing::placement;
    use overdrive_worker::probe_runner::ProbeRunner;
    use overdrive_worker::{BeaconIntake, VmDriver, VmHostLayout};
    use parking_lot::Mutex;
    use tempfile::TempDir;

    use super::sim_vm_host::{RecordingVmm, SimVmmHost};
    use crate::shared_network_test_ports::composed::ComposedNode;
    use crate::shared_network_test_ports::{TestGuestDnsFactory, shared_guest_network_owner};
    use crate::{ServerHandle, run_server_with_obs_and_driver};

    placement!(host_pure);

    /// The workload `quint_node_volume::declaration_wire` declares.
    const WORKLOAD: &str = "vol-w";
    /// The convergence tick cadence the injected clock is advanced by.
    const TICK: Duration = Duration::from_millis(100);
    /// Upper bound on injected ticks a wait may take (30 s of logical time).
    const MAX_TICKS: usize = 300;

    fn workload() -> WorkloadId {
        WorkloadId::new(WORKLOAD).expect("valid workload id")
    }

    /// What the recording `Vmm` saw of one `create`.
    #[derive(Clone, Debug)]
    struct CreateRecord {
        alloc: AllocationId,
        volume: bool,
        /// The pool's claim for the workload when the create was asked for
        /// (volume creates only).
        claim_before: Option<ClaimState>,
        /// Whether the orphan watched by this `Vmm` was still live then.
        orphan_live: bool,
    }

    /// `Vmm` decorator over the host-recording double: records every `create`
    /// and, for a create whose config carries a volume, links the workload's
    /// claim to the created process's liveness (Cloud Hypervisor's exclusive
    /// open lasts as long as the process).
    struct ClaimingVmm {
        inner: RecordingVmm,
        volumes: SimVolumeBacking,
        /// The orphan to watch, 0 for none.
        orphan: AtomicU32,
        creates: Mutex<Vec<CreateRecord>>,
    }

    #[async_trait]
    impl Vmm for ClaimingVmm {
        fn kind(&self) -> &'static str {
            self.inner.kind()
        }

        async fn probe(&self) -> Result<(), VmmProbeError> {
            self.inner.probe().await
        }

        async fn create(&self, config: &VmConfig, cid: GuestCid) -> VmmResult<VmProcess> {
            let volume = config.volume.is_some();
            let claim_before =
                if volume { self.volumes.claim_state(&workload()).await.ok() } else { None };
            let orphan = self.orphan.load(Ordering::SeqCst);
            self.creates.lock().push(CreateRecord {
                alloc: config.alloc.clone(),
                volume,
                claim_before,
                orphan_live: orphan != 0 && self.inner.inner.is_live(orphan),
            });
            let process = self.inner.create(config, cid).await?;
            if volume {
                let vmm = self.inner.inner.clone();
                let pid = process.control.pid;
                self.volumes.hold_claim_while_live(&workload(), move || vmm.is_live(pid));
            }
            Ok(process)
        }

        async fn terminate(
            &self,
            control: &VmControl,
            grace: Duration,
        ) -> VmmResult<VmTermination> {
            self.inner.terminate(control, grace).await
        }
    }

    /// What the recording driver saw of one `Driver::start`.
    #[derive(Clone, Debug)]
    struct StartRecord {
        alloc: AllocationId,
        /// Whether the orphan watched by this driver was still live when the start was
        /// called.
        orphan_live: bool,
    }

    /// `Driver` decorator over the production `VmDriver`: records every `start` at the driven
    /// port, with the orphan's liveness at that instant, so a launch minted before boot
    /// reclamation ended the orphan is observable even when it has not reached `Vmm::create`.
    struct StartRecordingDriver {
        inner: VmDriver,
        sim_vmm: SimVmm,
        /// The orphan to watch, 0 for none.
        orphan: u32,
        starts: Mutex<Vec<StartRecord>>,
    }

    #[async_trait]
    impl Driver for StartRecordingDriver {
        fn r#type(&self) -> DriverType {
            self.inner.r#type()
        }

        async fn start(
            &self,
            spec: &AllocationSpec,
            supersession: &dyn LaunchSupersession,
        ) -> Result<AllocationHandle, DriverError> {
            self.starts.lock().push(StartRecord {
                alloc: spec.alloc.clone(),
                orphan_live: self.orphan != 0 && self.sim_vmm.is_live(self.orphan),
            });
            self.inner.start(spec, supersession).await
        }

        async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
            self.inner.release_for_exit_emission(handle).await;
        }

        async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
            self.inner.stop(handle).await
        }

        async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
            self.inner.status(handle).await
        }

        async fn resize(
            &self,
            handle: &AllocationHandle,
            resources: Resources,
        ) -> Result<(), DriverError> {
            self.inner.resize(handle, resources).await
        }

        fn take_exit_receiver(&self) -> Option<tokio::sync::mpsc::Receiver<ExitEvent>> {
            self.inner.take_exit_receiver()
        }

        fn on_alloc_running(&self, spec: &AllocationSpec) {
            self.inner.on_alloc_running(spec);
        }

        fn on_alloc_terminal(&self, alloc_id: &AllocationId) {
            self.inner.on_alloc_terminal(alloc_id);
        }

        fn on_alloc_stable(&self, alloc_id: &AllocationId) {
            self.inner.on_alloc_stable(alloc_id);
        }

        fn live_allocations(&self) -> Option<Vec<AllocationId>> {
            self.inner.live_allocations()
        }

        fn try_begin_reclamation(&self, alloc: &AllocationId) -> bool {
            self.inner.try_begin_reclamation(alloc)
        }

        fn release_supervision(&self, alloc: &AllocationId) {
            self.inner.release_supervision(alloc);
        }
    }

    fn stage_artifacts(root: &Path) {
        std::fs::create_dir_all(root).expect("artifact root");
        std::fs::write(root.join("master.img"), b"deterministic-fixture-rootfs-bytes")
            .expect("write synthetic master rootfs");
        let mut header = vec![0_u8; KERNEL_MAGIC_WINDOW];
        header[..4].copy_from_slice(b"\x7fELF");
        std::fs::write(root.join("vmlinuz"), &header).expect("stage the synthetic kernel image");
    }

    /// The node: what outlives a `serve` process.
    struct Node {
        composed: ComposedNode,
        sim_vmm: SimVmm,
        host: Arc<SimVmmHost>,
        volumes: SimVolumeBacking,
        obs: Arc<SimObservationStore>,
        artifacts: TempDir,
    }

    /// One `serve` process: the production boot over the node's roots.
    struct Process {
        server: ServerHandle,
        vmm: Arc<ClaimingVmm>,
        driver: Arc<StartRecordingDriver>,
        client: reqwest::Client,
        base: String,
    }

    fn client_trusting(ca_pem: &str) -> reqwest::Client {
        let cert = reqwest::Certificate::from_pem(ca_pem.as_bytes()).expect("parse CA PEM");
        reqwest::Client::builder()
            .add_root_certificate(cert)
            .https_only(true)
            .use_rustls_tls()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("build reqwest client")
    }

    fn read_ca_from_trust_triple(operator_config_dir: &Path) -> String {
        use base64::Engine as _;
        use base64::engine::general_purpose::STANDARD as BASE64;

        let config_path = operator_config_dir.join(".overdrive").join("config");
        let text = std::fs::read_to_string(&config_path).unwrap_or_else(|error| {
            panic!("read trust triple at {}: {error}", config_path.display())
        });
        let doc: toml::Value = toml::from_str(&text).expect("parse trust triple TOML");
        let ca_b64 = doc
            .get("contexts")
            .and_then(toml::Value::as_array)
            .and_then(|contexts| {
                contexts.iter().find(|context| {
                    context.get("name").and_then(toml::Value::as_str) == Some("local")
                })
            })
            .and_then(|context| context.get("ca"))
            .and_then(toml::Value::as_str)
            .expect("[[contexts]] with name=\"local\" must carry a ca field");
        String::from_utf8(BASE64.decode(ca_b64).expect("base64 decode ca"))
            .expect("ca PEM is UTF-8")
    }

    impl Node {
        fn new() -> Self {
            let artifacts = TempDir::new().expect("tmpdir");
            stage_artifacts(artifacts.path());
            let sim_vmm =
                SimVmm::new(std::sync::Arc::new(overdrive_sim::adapters::clock::SimClock::new()));
            Self {
                composed: ComposedNode::fresh(),
                host: SimVmmHost::new(sim_vmm.clone(), false),
                sim_vmm,
                volumes: SimVolumeBacking::with_capacity(volume_size_gib(100)),
                obs: Arc::new(SimObservationStore::single_peer(
                    NodeId::new("local").expect("valid node ID"),
                    0,
                )),
                artifacts,
            }
        }

        /// Boot one `serve` process through `run_server_with_obs_and_driver`:
        /// the VM driver over the recording `Vmm` and the node's pool, the
        /// node's observation store, simulated host and clock. `orphan` is the
        /// pid the recording `Vmm` watches (0 for none).
        async fn boot(&self, seed: u64, orphan: u32) -> Process {
            let clock = Arc::clone(&self.composed.clock);
            let config = self.composed.config();
            let mut boot = self
                .composed
                .boot_adapters(&config, Arc::new(TestGuestDnsFactory::default()))
                .expect("the node's stores open over its data directory");
            boot.clock = Arc::clone(&clock) as Arc<dyn Clock>;
            boot.observation = Arc::clone(&self.obs) as Arc<dyn ObservationStore>;
            boot.vm_host_state = Arc::clone(&self.host) as Arc<dyn VmHostState>;

            let wiring = GuestNetworkExecWiring::new(Arc::clone(&clock) as Arc<dyn Clock>);
            let root = self.artifacts.path().join(format!("serve-{seed}"));
            tokio::fs::create_dir_all(&root).await.expect("per-process root");
            let beacon = Arc::new(SimBeaconIo::stepped(self.sim_vmm.clone()));
            let intake = BeaconIntake::bind(
                Arc::clone(&beacon) as Arc<dyn BeaconIo>,
                overdrive_core::guest_transport::VsockListenPort::KernelAssigned,
            )
            .await
            .expect("the sim beacon listener binds");
            let guest_listener_board = Arc::new(overdrive_worker::GuestListenerBoard::new());
            let probe_runner = Arc::new(ProbeRunner::new(
                Arc::new(SimTcpProber::new()),
                Arc::new(SimHttpProber::new()),
                Arc::clone(&clock) as Arc<dyn Clock>,
                Arc::clone(&self.obs) as Arc<dyn ObservationStore>,
                Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
                Arc::clone(&guest_listener_board)
                    as Arc<dyn overdrive_core::traits::GuestListenerReports>,
            ));
            let probed: Arc<dyn VolumeBacking> = SimVolumePoolProbe::composed(self.volumes.clone())
                .probe()
                .await
                .expect("the composed pool probes")
                .expect("a node with a composed pool yields a backing");
            let vmm = Arc::new(ClaimingVmm {
                inner: RecordingVmm { inner: self.sim_vmm.clone(), host: Arc::clone(&self.host) },
                volumes: self.volumes.clone(),
                orphan: AtomicU32::new(orphan),
                creates: Mutex::new(Vec::new()),
            });
            let vm_driver = VmDriver::new(
                Arc::clone(&vmm) as Arc<dyn Vmm>,
                Arc::clone(&clock) as Arc<dyn Clock>,
                Arc::new(SimCgroupFs::new()),
                Arc::new(SimCgroupAccounting::new()),
                probe_runner,
                wiring.gate(),
                VmHostLayout {
                    cgroup_root: root.join("cgroup"),
                    run_dir_root: root.join("run"),
                    clone_index_dir: root.join("clone-index"),
                    clone_staging_dir: root.join("clone-staging"),
                    arch: HostArch::X86_64,
                    confinement: VmConfinement::confined(
                        VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
                        1024,
                    ),
                },
                intake,
                guest_listener_board,
                Some(probed),
                std::sync::Arc::new(overdrive_sim::SimVmHostState::new()),
            );
            let driver = Arc::new(StartRecordingDriver {
                inner: vm_driver,
                sim_vmm: self.sim_vmm.clone(),
                orphan,
                starts: Mutex::new(Vec::new()),
            });
            let operator_config_dir = config.operator_config_dir.clone();
            let server = run_server_with_obs_and_driver(
                config,
                boot,
                Arc::clone(&driver) as Arc<dyn Driver>,
                shared_guest_network_owner(),
                wiring,
            )
            .await
            .expect("the production boot, boot reclamation included, completes");
            let bound = server.local_addr().await.expect("server bound address");
            let client = client_trusting(&read_ca_from_trust_triple(&operator_config_dir));
            Process {
                server,
                vmm,
                driver,
                client,
                base: format!("https://localhost:{}", bound.port()),
            }
        }

        /// Deploy the volume workload through the public API.
        async fn deploy(&self, process: &Process) {
            let root = self.artifacts.path().to_str().expect("utf-8 artifact root");
            let response = process
                .client
                .post(format!("{}/v1/workloads", process.base))
                .json(&crate::quint_node_volume::declaration_wire(root))
                .send()
                .await
                .expect("submit reaches the public API");
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            assert!(
                status.is_success(),
                "a workload declaring a volume must be accepted: {status} {body}"
            );
        }

        /// Advance the injected clock until `cond` holds.
        async fn tick_until(&self, what: &str, mut cond: impl FnMut() -> bool) {
            for _ in 0..MAX_TICKS {
                if cond() {
                    return;
                }
                self.composed.clock.tick(TICK);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("{what} within {MAX_TICKS} ticks");
        }

        /// Drive `work` to completion while the injected clock keeps advancing, so
        /// no clock-bounded wait inside it parks forever.
        async fn ticking<F: std::future::Future>(&self, work: F) -> F::Output {
            let mut work = std::pin::pin!(work);
            loop {
                tokio::select! {
                    biased;
                    output = &mut work => return output,
                    () = tokio::time::sleep(Duration::from_millis(5)) => self.composed.clock.tick(TICK),
                }
            }
        }
    }

    /// The `Running` row a launch that completed leaves behind, in the shape
    /// the action shim writes.
    #[cfg(test)]
    fn running_row(
        alloc_id: &AllocationId,
        workload_addr: Option<std::net::Ipv4Addr>,
        pending: &LogicalTimestamp,
    ) -> AllocStatusRow {
        let node = NodeId::new("local").expect("valid NodeId");
        AllocStatusRow {
            alloc_id: alloc_id.clone(),
            workload_id: workload(),
            node_id: node.clone(),
            state: AllocState::Running,
            // Dominates the launch's stored `Pending` row, as the shim's Running write does.
            updated_at: LogicalTimestamp::dominating(0, node, Some(pending)),
            reason: Some(TransitionReason::Started),
            detail: None,
            terminal: None,
            stderr_tail: None,
            kind: WorkloadKind::Service,
            listeners: Vec::new(),
            started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
            workload_addr,
            last_terminated: None,
            restart_count: 0,
        }
    }

    /// What the crash leaves in the durable observation store.
    #[derive(Clone, Copy)]
    enum Residue {
        /// The launch was parked in boot: its `Pending` row (the guest address
        /// on record before `Vmm::create`, ADR-0353) is the only row, and the
        /// boot ends it as lost with the control plane (ADR-0355 W4).
        NoRow,
        /// The allocation's `Running` row survives (the boot reclamation
        /// reclaims the allocation).
        RunningRow,
    }

    async fn scenario(residue: Residue) {
        let node = Node::new();

        // The first process deploys the workload; its launch parks in the guest-boot wait.
        let first = node.boot(1, 0).await;
        node.deploy(&first).await;
        node.tick_until("the volume launch creates its VMM", || {
            !first.vmm.creates.lock().is_empty()
        })
        .await;
        let orphan_alloc = first.vmm.creates.lock()[0].alloc.clone();
        let orphan_pid = {
            let pids = node.host.pids_of(&orphan_alloc);
            assert_eq!(pids.len(), 1, "the launch created one VMM: {pids:?}");
            *pids.iter().next().expect("one pid")
        };
        assert!(
            first.vmm.creates.lock()[0].volume,
            "the launch must attach the declared volume to its VMM"
        );
        assert!(
            matches!(node.volumes.claim_state(&workload()).await, Ok(ClaimState::Held)),
            "the parked launch's VMM must hold the workload's volume claim"
        );
        let pending_rows = node.obs.alloc_status_rows().await.expect("rows");
        let pending = pending_rows
            .iter()
            .find(|row| row.alloc_id == orphan_alloc && row.state == AllocState::Pending)
            .expect("the parked launch's Pending row is on record before its VMM exists");
        let recorded_addr = pending.workload_addr;
        assert!(recorded_addr.is_some(), "the Pending row carries the launch's guest address");

        // `serve` dies: its tasks and composition die; its VMM does not.
        let Process { server, .. } = first;
        let _residue = node.ticking(server.kill_for_test()).await;
        assert!(node.sim_vmm.is_live(orphan_pid), "the orphan survives the crash");
        if matches!(residue, Residue::RunningRow) {
            node.obs
                .write_alloc_lifecycle(
                    running_row(&orphan_alloc, recorded_addr, &pending.updated_at),
                    TransitionSource::Reconciler,
                )
                .await
                .expect("the completed launch's Running row");
        }

        // The next process: boot reclamation is its own boot step.
        let second = node.boot(2, orphan_pid).await;
        assert!(
            !node.sim_vmm.is_live(orphan_pid),
            "the orphan's VMM is not live once boot returns"
        );
        node.tick_until("the successor's launch creates its VMM", || {
            !second.vmm.creates.lock().is_empty()
        })
        .await;

        let starts = second.driver.starts.lock().clone();
        assert_eq!(starts.len(), 1, "one launch reached the driver's start: {starts:?}");
        assert_ne!(starts[0].alloc, orphan_alloc, "the launch is a new allocation: {starts:?}");
        assert!(
            !starts[0].orphan_live,
            "no launch was minted before boot reclamation ended the orphan: {starts:?}"
        );
        let creates = second.vmm.creates.lock().clone();
        assert_eq!(creates.len(), 1, "one launch reached the VMM port: {creates:?}");
        let successor = &creates[0];
        assert_ne!(successor.alloc, orphan_alloc, "the successor is a new allocation");
        assert!(successor.volume, "the successor's launch must attach the volume");
        assert!(
            !successor.orphan_live,
            "no launch reached the VMM port before the orphan was ended: {creates:?}"
        );
        assert!(
            matches!(successor.claim_before, Some(ClaimState::Free)),
            "the orphan's claim must have drained before the successor attached: {creates:?}"
        );
        let rows = node.obs.alloc_status_rows().await.expect("rows readable");
        assert!(
            rows.iter().all(|row| row.state != AllocState::Failed
                && !matches!(row.reason, Some(TransitionReason::VmVolumeUnavailable { .. }))),
            "no VolumeUnavailable Held row and nothing charged: {rows:?}"
        );
        let orphan_row = rows
            .iter()
            .find(|row| row.alloc_id == orphan_alloc)
            .expect("the orphan allocation keeps a terminal row");
        assert_eq!(orphan_row.state, AllocState::Terminated, "the orphan's row is terminal");
        assert_eq!(
            orphan_row.workload_addr, recorded_addr,
            "the terminal row carries the recorded address"
        );
        let text = orphan_row
            .reason
            .as_ref()
            .map_or_else(|| "<no reason>".to_owned(), TransitionReason::human_readable);
        match residue {
            Residue::NoRow => assert!(
                text.contains("launch abandoned: lost with the control plane"),
                "a Pending row ends as lost with the control plane, got `{text}`"
            ),
            Residue::RunningRow => assert!(
                matches!(
                    orphan_row.reason,
                    Some(TransitionReason::Stopped {
                        by: overdrive_core::transition_reason::StoppedBy::PlatformReclaimed
                    })
                ),
                "a Running row ends as platform-reclaimed, got `{text}`"
            ),
        }

        let Process { server, .. } = second;
        node.ticking(server.shutdown(Duration::from_secs(2)))
            .await
            .expect("the second process shuts down within its drain deadline");
    }

    /// M7 (the named cell; the crash leaves the launch's `Pending` row). The
    /// shim writes that row before `Vmm::create` (ADR-0353), so boot reclaims
    /// the mid-boot VMM and closes the row (ADR-0355 W4).
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn boot_reclamation_ends_a_mid_boot_vmm_before_any_launch_for_its_workload() {
        scenario(Residue::NoRow).await;
    }

    /// M7 (c): the same when the crash leaves a `Running` row, so the
    /// reclamation is a `ReclaimAllocation`.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn boot_reclamation_ends_a_mid_boot_vmm_when_the_crash_left_a_running_row() {
        scenario(Residue::RunningRow).await;
    }
}

/// Boot reclamation ends every launch the previous control plane left
/// unfinished (RB-4, RB-7, RB-11; ADR-0350, ADR-0354, ADR-0355 W4).
///
/// # Scenario family
///
/// A `serve` that crashed leaves rows. A `Pending` row is a launch that had
/// not yet run; it is closed first, by the first step of `converge`, as
/// `Terminated` / `LaunchLostWithControlPlane` with the row's address carried,
/// whether or not its VM left host artifacts. A `Running` row whose scope is
/// busy (the kill is delivered, the VMM is not gone) is ended as
/// platform-reclaimed at once and its guest-address offset is reserved; boot
/// continues. A non-busy substrate refusal still refuses the boot.
///
/// # Owner path, fault stimulus, oracle
///
/// Production `converge` over `SimVmHostState` and the production observation
/// store seam (`SimObservationStore`). Faults enter at the driven port only:
/// the host double's scopes and its scripted `kill_scope` failure. No row is
/// seeded that production does not author: the `Pending` row is the row the
/// shim writes before `Vmm::create` (ADR-0353), the `Running` row is the shim's.
/// Oracle: the allocation's row, the host observation, the pool's `observe`.
///
/// # Tier and placement
///
/// Tier 1 layer 2, `host_pure`. Pending tests keep their full body under
/// `#[should_panic(expected = ...)]` naming the assertion that fails today.
#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "acceptance bodies fail at once on a broken precondition, naming it"
)]
mod launches_lost_with_the_control_plane {
    use std::collections::BTreeSet;
    use std::net::Ipv4Addr;
    use std::num::NonZeroU32;
    use std::sync::Arc;

    use overdrive_core::aggregate::{
        IntentKey, Job, Vm, WorkloadDriver, WorkloadIntent, WorkloadKind,
    };
    use overdrive_core::byte_size::ByteSize;
    use overdrive_core::cpu_quantity::CpuQuantity;
    use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
    use overdrive_core::traits::driver::{Driver, DriverType, Resources};
    use overdrive_core::traits::guest_attachment_view::GuestAttachmentLease;
    use overdrive_core::traits::intent_store::IntentStore;
    use overdrive_core::traits::observation_store::{
        AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, TransitionSource,
    };
    use overdrive_core::traits::vm_host_state::VmHostState;
    use overdrive_core::transition_reason::{StoppedBy, TransitionReason};
    use overdrive_sim::adapters::vm_host_state::SimVmHostState;
    use overdrive_store_local::LocalIntentStore;
    use overdrive_testing::placement;
    use tempfile::TempDir;

    use super::{AppState, ConvergeError, converge};
    use crate::action_shim::reclamation::ReclamationError;
    use crate::reconciler_runtime::ReconcilerRuntime;

    placement!(host_pure);

    /// The node the composition runs as; a row owned by it is this boot's.
    const THIS_NODE: &str = "writer-1";
    const WORKLOAD: &str = "lost-launch-workload";

    fn node(name: &str) -> NodeId {
        NodeId::new(name).expect("valid node id")
    }

    fn alloc(name: &str) -> AllocationId {
        AllocationId::new(name).expect("valid allocation id")
    }

    /// One `AppState` over `host`, composing a non-VM driver (so supervision
    /// reads `Observed(empty)` and every non-terminal allocation is
    /// unsupervised) and the production guest-address pool.
    fn app_state(tmp: &TempDir, host: &SimVmHostState) -> AppState {
        let runtime =
            ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("intent store"));
        let obs: Arc<dyn ObservationStore> =
            Arc::new(overdrive_sim::adapters::observation_store::SimObservationStore::single_peer(
                node("local"),
                0,
            ));
        let driver: Arc<dyn Driver> =
            Arc::new(overdrive_sim::adapters::driver::SimDriver::new(DriverType::Unikernel));
        let allocator = crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
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
            node(THIS_NODE),
            allocator,
            crate::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            crate::shared_network_test_ports::mtls_worker(),
            crate::shared_network_test_ports::shared_guest_network_owner(),
            crate::shared_network_test_ports::guest_network_exec_gate(),
            crate::shared_network_test_ports::guest_pool(),
        );
        state.vm_host_state = Arc::new(host.clone()) as Arc<dyn VmHostState>;
        state
    }

    /// The Vm-driven Job intent the desired-side join reads.
    async fn put_vm_job(state: &AppState) {
        let id = WorkloadId::new(WORKLOAD).expect("workload id");
        let job = Job {
            id: id.clone(),
            replicas: NonZeroU32::new(1).expect("nonzero"),
            resources: Resources {
                cpu: CpuQuantity::from_millis(500),
                memory: ByteSize::new(134_217_728),
            },
            driver: WorkloadDriver::Vm(Vm {
                volume: None,
                command: "/sbin/init".to_owned(),
                args: Vec::new(),
                kernel: "/boot/vmlinux".to_owned(),
                rootfs: "/boot/rootfs.ext4".to_owned(),
            }),
        };
        let archived = WorkloadIntent::Job(job).archive_for_store().expect("archive");
        state
            .store
            .put(IntentKey::for_workload(&id).as_bytes(), archived.as_ref())
            .await
            .expect("put intent");
    }

    /// A row of `name` as the shim authors it: `Pending` (before `Vmm::create`,
    /// ADR-0353) or `Running` (after the guest's readiness).
    async fn write_row(
        state: &AppState,
        name: &str,
        owner: &str,
        row_state: AllocState,
        address: Ipv4Addr,
    ) {
        let row = AllocStatusRow {
            alloc_id: alloc(name),
            workload_id: WorkloadId::new(WORKLOAD).expect("workload id"),
            node_id: node(owner),
            state: row_state,
            updated_at: LogicalTimestamp { counter: 1, writer: node(owner) },
            reason: (row_state == AllocState::Running).then_some(TransitionReason::Started),
            detail: None,
            terminal: None,
            stderr_tail: None,
            kind: WorkloadKind::Job,
            listeners: Vec::new(),
            started_at: None,
            workload_addr: Some(address),
            last_terminated: None,
            restart_count: 0,
        };
        state
            .obs
            .write_alloc_lifecycle(row, TransitionSource::Reconciler)
            .await
            .expect("write the row");
    }

    async fn row(state: &AppState, name: &str) -> AllocStatusRow {
        state
            .obs
            .alloc_status_row(&alloc(name))
            .await
            .expect("row readable")
            .expect("the allocation has a row")
    }

    fn reason_text(row: &AllocStatusRow) -> String {
        row.reason
            .as_ref()
            .map_or_else(|| "<no reason>".to_owned(), TransitionReason::human_readable)
    }

    /// The row a launch lost with the control plane must end as (ADR-0355 W4).
    fn assert_lost_with_the_control_plane(row: &AllocStatusRow, address: Ipv4Addr) {
        assert_eq!(
            row.state,
            AllocState::Terminated,
            "a launch lost with the control plane ends Terminated, got {:?} ({})",
            row.state,
            reason_text(row)
        );
        assert!(
            reason_text(row).contains("launch abandoned: lost with the control plane"),
            "the row names why: `launch abandoned: lost with the control plane`, got `{}`",
            reason_text(row)
        );
        assert!(
            !matches!(
                row.reason,
                Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed })
            ),
            "the executors never rewrite the reason of a row the closing step ended"
        );
        assert_eq!(row.terminal, None, "the workload's intent still stands");
        assert_eq!(row.workload_addr, Some(address), "the Pending row's address is carried");
    }

    const ADDRESS: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 9);

    /// A `Pending` row whose launch left no host artifact ends `Terminated` /
    /// `LaunchLostWithControlPlane`, address carried.
    /// CONTRACT_SHAPE: bounded-change. Universe: the node's rows.
    #[tokio::test]
    async fn a_pending_row_with_no_host_artifact_ends_as_lost_with_the_control_plane() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let state = app_state(&tmp, &host);
        write_row(&state, "lost-bare-0", THIS_NODE, AllocState::Pending, ADDRESS).await;

        converge(&state).await.expect("boot converges");

        assert_lost_with_the_control_plane(&row(&state, "lost-bare-0").await, ADDRESS);
    }

    /// A `Pending` row whose VM left a settled scope, a run directory and a
    /// clone ends as lost with the control plane, not as
    /// `Stopped { PlatformReclaimed }`; the executors discard the artifacts and
    /// leave the reason alone.
    #[tokio::test]
    async fn a_pending_row_whose_vm_left_a_settled_scope_ends_as_lost_with_the_control_plane() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let id = alloc("lost-settled-0");
        host.set_scope(id.clone(), BTreeSet::from([4242]));
        host.set_run_dir(id.clone());
        host.set_clone(id.clone(), tmp.path().join("clone.img"));
        let state = app_state(&tmp, &host);
        put_vm_job(&state).await;
        write_row(&state, "lost-settled-0", THIS_NODE, AllocState::Pending, ADDRESS).await;

        converge(&state).await.expect("boot converges");

        assert_lost_with_the_control_plane(&row(&state, "lost-settled-0").await, ADDRESS);
        assert!(host.artifacts_absent(&id), "the executors discard the launch's artifacts");
    }

    /// A second boot over the closed rows writes nothing: the closing step is
    /// idempotent.
    #[tokio::test]
    async fn a_second_boot_over_the_closed_rows_writes_nothing() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let state = app_state(&tmp, &host);
        write_row(&state, "lost-twice-0", THIS_NODE, AllocState::Pending, ADDRESS).await;
        converge(&state).await.expect("first boot converges");
        let after_first = row(&state, "lost-twice-0").await;
        assert_lost_with_the_control_plane(&after_first, ADDRESS);

        converge(&state).await.expect("second boot converges");

        assert_eq!(row(&state, "lost-twice-0").await, after_first, "a second boot writes nothing");
    }

    /// GUARD (RB-11). Another node's `Pending` row is not this boot's launch:
    /// the closing step leaves it alone, today and after the change.
    #[tokio::test]
    async fn guard_another_nodes_pending_row_is_left_alone() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let state = app_state(&tmp, &host);
        write_row(&state, "other-node-0", "other-node", AllocState::Pending, ADDRESS).await;

        converge(&state).await.expect("boot converges");

        assert_eq!(
            row(&state, "other-node-0").await.state,
            AllocState::Pending,
            "a Pending row of another node is not closed by this node's boot"
        );
    }

    /// PENDING (RB-4, RB-7, ADR-0350, ADR-0354). A `Running` row whose scope
    /// is busy (the kill is delivered, the scope stays) never refuses the boot:
    /// `converge` returns `Ok`, the row is ended as platform-reclaimed carrying
    /// the address, the scope stays for `VmReclamation`, and the guest-address
    /// offset the row names is held (a Retiring lease) so no launch can be
    /// handed that CID. Today the busy error refuses the boot.
    ///
    /// CONTRACT_SHAPE: bounded-change. Universe: the allocation's row, the
    /// host's scopes, the pool's occupancy and lease of the allocation.
    #[tokio::test]
    async fn a_busy_scope_boots_ends_the_running_row_and_reserves_its_address() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let id = alloc("busy-running-0");
        host.set_scope(id.clone(), BTreeSet::from([4242]));
        host.script_kill_scope_failure(std::io::ErrorKind::ResourceBusy);
        let state = app_state(&tmp, &host);
        put_vm_job(&state).await;
        write_row(&state, "busy-running-0", THIS_NODE, AllocState::Running, ADDRESS).await;
        let before = state.guest_pool.observe(std::slice::from_ref(&id));
        assert!(before.leases.is_empty(), "precondition: no lease is held at boot");

        let result = converge(&state).await;

        assert!(result.is_ok(), "a busy scope never refuses the boot: {result:?}");
        let ended = row(&state, "busy-running-0").await;
        assert_eq!(ended.state, AllocState::Terminated, "the stuck VM's row is terminal at once");
        assert!(
            matches!(
                ended.reason,
                Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed })
            ),
            "the busy Running row is ended as platform-reclaimed: {:?}",
            ended.reason
        );
        assert_eq!(ended.workload_addr, Some(ADDRESS), "the terminal row carries the address");
        assert!(host.has_scope(&id), "the scope stays for VmReclamation");
        let after = state.guest_pool.observe(std::slice::from_ref(&id));
        assert_eq!(
            after.leases.get(&id),
            Some(&GuestAttachmentLease::Retiring),
            "the stuck VM's offset is reserved as a Retiring lease"
        );
        assert_eq!(after.occupancy.held, 1, "the reservation counts toward the attachment cap");
        assert_eq!(after.occupancy.retiring, 1, "and is Retiring");
    }

    /// PENDING (RB-11, RB-4). A `Pending` row whose VM left a busy scope ends as
    /// lost with the control plane (not platform-reclaimed) and its offset is
    /// reserved from the closed row's carried address.
    #[tokio::test]
    async fn a_busy_scope_of_a_pending_row_ends_as_lost_and_reserves_the_carried_address() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let id = alloc("busy-pending-0");
        host.set_scope(id.clone(), BTreeSet::from([4242]));
        host.script_kill_scope_failure(std::io::ErrorKind::ResourceBusy);
        let state = app_state(&tmp, &host);
        put_vm_job(&state).await;
        write_row(&state, "busy-pending-0", THIS_NODE, AllocState::Pending, ADDRESS).await;

        let result = converge(&state).await;

        assert!(result.is_ok(), "a busy scope never refuses the boot: {result:?}");
        assert_lost_with_the_control_plane(&row(&state, "busy-pending-0").await, ADDRESS);
        let observed = state.guest_pool.observe(std::slice::from_ref(&id));
        assert_eq!(
            observed.leases.get(&id),
            Some(&GuestAttachmentLease::Retiring),
            "the offset named by the closed row is reserved"
        );
    }

    /// The row the reclamation executor writes for a `Running` row whose scope
    /// settled carries the prior row's address (it was `None`).
    #[tokio::test]
    async fn a_platform_reclaimed_row_carries_the_prior_rows_address() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let id = alloc("reclaimed-running-0");
        host.set_scope(id.clone(), BTreeSet::from([4242]));
        let state = app_state(&tmp, &host);
        put_vm_job(&state).await;
        write_row(&state, "reclaimed-running-0", THIS_NODE, AllocState::Running, ADDRESS).await;

        converge(&state).await.expect("boot converges");

        let ended = row(&state, "reclaimed-running-0").await;
        assert!(
            matches!(
                ended.reason,
                Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed })
            ),
            "precondition: the Running row is ended as platform-reclaimed: {:?}",
            ended.reason
        );
        assert_eq!(
            ended.workload_addr,
            Some(ADDRESS),
            "the platform-reclaimed row carries the prior row's workload_addr"
        );
    }

    /// GUARD (ADR-0350 item 5). A non-busy substrate refusal still refuses the
    /// boot with `ConvergeError::Reclamation`: it is not a stuck VM.
    #[tokio::test]
    async fn guard_a_non_busy_executor_error_still_refuses_the_boot() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let id = alloc("refused-running-0");
        host.set_scope(id.clone(), BTreeSet::from([4242]));
        host.script_kill_scope_failure(std::io::ErrorKind::PermissionDenied);
        let state = app_state(&tmp, &host);
        put_vm_job(&state).await;
        write_row(&state, "refused-running-0", THIS_NODE, AllocState::Running, ADDRESS).await;

        let result = converge(&state).await;

        assert!(
            matches!(result, Err(ConvergeError::Reclamation(ReclamationError::Host(_)))),
            "a substrate refusal refuses the boot: {result:?}"
        );
    }
}

/// Boot proceeds past a stuck VM (ADR-0350, ADR-0354; feature delta "Boot
/// proceeds past a stuck VM", "A busy boot scope writes its terminal row", "A
/// scope kill's settle wait is bounded" item 5; ADR-0348 and ADR-0349's
/// `VmReclamation` clauses).
///
/// A stuck VM is a killed VMM that is not reaped: its scope stays populated,
/// `SimVmHostState::hold_scope_while` keeps `kill_scope` answering
/// `ResourceBusy`, and `SimVmm`'s unreaped process keeps its vsock device and
/// its CID. Owner path: `converge` (the boot drive, run as `run_server` runs
/// it), the production `VmReclamation` tick (`run_convergence_tick`), the
/// action shim's `ReclaimAllocationNetwork` dispatch and the production
/// `GuestAddressPool`. Fault stimulus: a held scope, a scripted non-busy
/// refusal, a corrupt row address. Oracle: the allocation's row, the host's
/// observation, the pool's `lease_observation` / `observe`, `SimVmm`'s vsock
/// holders and the injected clock.
///
/// Tier 1 layer 2, `host_pure`. A GUARD passes today; a PENDING test keeps its
/// whole body under `#[should_panic(expected = ...)]` naming the assertion
/// that fails today.
#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::similar_names,
    reason = "acceptance bodies fail at once on a broken precondition, naming it"
)]
mod stuck_vm_boot {
    use std::collections::BTreeSet;
    use std::net::Ipv4Addr;
    use std::num::NonZeroU32;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use async_trait::async_trait;
    use ipnet::Ipv4Net;
    use overdrive_core::aggregate::{
        IntentKey, Job, Vm, WorkloadDriver, WorkloadIntent, WorkloadKind,
    };
    use overdrive_core::byte_size::ByteSize;
    use overdrive_core::cgroup::CgroupPath;
    use overdrive_core::cpu_quantity::CpuQuantity;
    use overdrive_core::guest_transport::{GUEST_CID_BASE, GuestCid};
    use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
    use overdrive_core::reconcilers::{Action, ReconcilerName, TargetResource, TickContext};
    use overdrive_core::traits::driver::{Driver, DriverType, Resources};
    use overdrive_core::traits::guest_attachment_view::GuestAttachmentLease;
    use overdrive_core::traits::intent_store::IntentStore;
    use overdrive_core::traits::observation_store::{
        AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, TransitionSource,
    };
    use overdrive_core::traits::vm_host_state::{
        KILL_SCOPE_SETTLE_BOUND, VmHostObservation, VmHostState, VmHostStateProbeError,
    };
    use overdrive_core::traits::vmm::{VMM_REAP_BOUND, VmControl, VmTermination, Vmm};
    use overdrive_core::transition_reason::{StoppedBy, TransitionReason};
    use overdrive_core::wall_clock::UnixInstant;
    use overdrive_sim::SimVmm;
    use overdrive_sim::adapters::clock::SimClock;
    use overdrive_sim::adapters::vm_host_state::SimVmHostState;
    use overdrive_store_local::LocalIntentStore;
    use overdrive_testing::placement;
    use tempfile::TempDir;

    use super::sim_vm_host::{SimVmmHost, VmComposition, compose};
    use super::{AppState, ConvergeError, converge};
    use crate::action_shim::reclamation::{CorruptLeftoverRow, ReclamationError};
    use crate::guest_network::{
        ForwardingActivation, GuestLeaseView, GuestNetworkPlan, GuestNetworkProvisioner,
        Result as GuestNetworkResult,
    };
    use crate::reconciler_runtime::ReconcilerRuntime;

    placement!(host_pure);

    /// The node the composition runs as; a row owned by it is this boot's.
    const THIS_NODE: &str = "writer-1";
    const WORKLOAD: &str = "stuck-vm-workload";
    /// An address of the guest prefix (offset 9).
    const ADDRESS: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 9);
    const OFFSET_OF_ADDRESS: u32 = 9;

    fn node(name: &str) -> NodeId {
        NodeId::new(name).expect("valid node id")
    }

    fn alloc(name: &str) -> AllocationId {
        AllocationId::new(name).expect("valid allocation id")
    }

    // -- the host double ----------------------------------------------------

    /// `SimVmHostState` that counts its `kill_scope` calls and, with a clock,
    /// waits `KILL_SCOPE_SETTLE_BOUND` on it before answering: the timing the
    /// real adapter has and the sim leaves out. The scope hold and the scripted
    /// failures are the sim's own.
    #[derive(Clone)]
    struct RecordingHost {
        inner: SimVmHostState,
        wait_on: Option<SimClock>,
        kill_calls: Arc<AtomicUsize>,
    }

    impl RecordingHost {
        fn new(inner: SimVmHostState, wait_on: Option<SimClock>) -> Self {
            Self { inner, wait_on, kill_calls: Arc::new(AtomicUsize::new(0)) }
        }

        fn kill_calls(&self) -> usize {
            self.kill_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl VmHostState for RecordingHost {
        fn kind(&self) -> &'static str {
            "recording-sim-vm-host-state"
        }

        async fn probe(&self) -> Result<(), VmHostStateProbeError> {
            self.inner.probe().await
        }

        async fn observe(&self) -> std::io::Result<VmHostObservation> {
            self.inner.observe().await
        }

        async fn kill_scope(&self, scope: &CgroupPath) -> std::io::Result<()> {
            self.kill_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(clock) = &self.wait_on {
                use overdrive_core::traits::clock::Clock as _;
                clock.sleep(KILL_SCOPE_SETTLE_BOUND).await;
            }
            self.inner.kill_scope(scope).await
        }

        async fn discard_artifacts(&self, alloc: &AllocationId) -> std::io::Result<()> {
            self.inner.discard_artifacts(alloc).await
        }
    }

    /// Seed `id`'s scope and hold it while the returned flag is set.
    fn hold(host: &SimVmHostState, id: &AllocationId) -> Arc<AtomicBool> {
        host.set_scope(id.clone(), BTreeSet::from([4242]));
        let held = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&held);
        host.hold_scope_while(id, move || flag.load(Ordering::SeqCst));
        held
    }

    /// The guest network the shim is lent: it records its teardowns.
    #[derive(Default)]
    struct RecordingProvisioner {
        teardowns: AtomicUsize,
    }

    #[async_trait]
    impl GuestNetworkProvisioner for RecordingProvisioner {
        async fn provision(&self, _plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
            Ok(())
        }

        async fn activate(
            &self,
            _plan: &GuestNetworkPlan,
        ) -> GuestNetworkResult<ForwardingActivation> {
            Ok(ForwardingActivation::Activated)
        }

        async fn teardown(&self, _plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
            self.teardowns.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    // -- the composition ----------------------------------------------------

    /// One `AppState` over `host`: the production guest-address pool, the
    /// `vm-reclamation` reconciler registered (so the production tick runs),
    /// a non-VM driver (supervision reads `Observed(empty)`, every non-terminal
    /// allocation is unsupervised) and `clock` as the injected clock.
    async fn app_state(tmp: &TempDir, host: Arc<dyn VmHostState>, clock: &SimClock) -> AppState {
        let mut runtime =
            ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
        runtime.register(crate::vm_reclamation()).await.expect("register vm-reclamation");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("intent store"));
        let obs: Arc<dyn ObservationStore> =
            Arc::new(overdrive_sim::adapters::observation_store::SimObservationStore::single_peer(
                node("local"),
                0,
            ));
        let driver: Arc<dyn Driver> =
            Arc::new(overdrive_sim::adapters::driver::SimDriver::new(DriverType::Unikernel));
        let allocator = crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
        let mut state = AppState::new(
            store,
            store_path,
            obs,
            Arc::new(runtime),
            driver,
            Arc::new(clock.clone()),
            Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
            Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
                overdrive_sim::adapters::entropy::SimEntropy::new(0),
            ))),
            Arc::new(crate::identity_mgr::IdentityMgr::new(None)),
            node(THIS_NODE),
            allocator,
            crate::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            crate::shared_network_test_ports::mtls_worker(),
            crate::shared_network_test_ports::shared_guest_network_owner(),
            crate::shared_network_test_ports::guest_network_exec_gate(),
            crate::shared_network_test_ports::guest_pool(),
        );
        state.vm_host_state = host;
        state
    }

    /// The Vm-driven Job intent the desired-side join reads.
    async fn put_vm_job(state: &AppState) {
        let id = WorkloadId::new(WORKLOAD).expect("workload id");
        let job = Job {
            id: id.clone(),
            replicas: NonZeroU32::new(1).expect("nonzero"),
            resources: Resources {
                cpu: CpuQuantity::from_millis(500),
                memory: ByteSize::new(134_217_728),
            },
            driver: WorkloadDriver::Vm(Vm {
                volume: None,
                command: "/sbin/init".to_owned(),
                args: Vec::new(),
                kernel: "/boot/vmlinux".to_owned(),
                rootfs: "/boot/rootfs.ext4".to_owned(),
            }),
        };
        let archived = WorkloadIntent::Job(job).archive_for_store().expect("archive");
        state
            .store
            .put(IntentKey::for_workload(&id).as_bytes(), archived.as_ref())
            .await
            .expect("put intent");
    }

    /// A row of `name` as the shim authors it, with the address it recorded.
    async fn write_row(
        state: &AppState,
        name: &str,
        row_state: AllocState,
        reason: Option<TransitionReason>,
        address: Option<Ipv4Addr>,
    ) {
        let row = AllocStatusRow {
            alloc_id: alloc(name),
            workload_id: WorkloadId::new(WORKLOAD).expect("workload id"),
            node_id: node(THIS_NODE),
            state: row_state,
            updated_at: LogicalTimestamp { counter: 1, writer: node(THIS_NODE) },
            reason,
            detail: None,
            terminal: None,
            stderr_tail: None,
            kind: WorkloadKind::Job,
            listeners: Vec::new(),
            started_at: None,
            workload_addr: address,
            last_terminated: None,
            restart_count: 0,
        };
        state
            .obs
            .write_alloc_lifecycle(row, TransitionSource::Reconciler)
            .await
            .expect("write the row");
    }

    async fn write_running(state: &AppState, name: &str, address: Option<Ipv4Addr>) {
        write_row(state, name, AllocState::Running, Some(TransitionReason::Started), address).await;
    }

    async fn write_operator_stopped(state: &AppState, name: &str, address: Option<Ipv4Addr>) {
        let reason = Some(TransitionReason::Stopped { by: StoppedBy::Operator });
        write_row(state, name, AllocState::Terminated, reason, address).await;
    }

    async fn row(state: &AppState, name: &str) -> AllocStatusRow {
        state
            .obs
            .alloc_status_row(&alloc(name))
            .await
            .expect("row readable")
            .expect("the allocation has a row")
    }

    fn assert_platform_reclaimed(row: &AllocStatusRow, address: Option<Ipv4Addr>) {
        assert_eq!(row.state, AllocState::Terminated, "the stuck VM's row is terminal at once");
        assert!(
            matches!(
                row.reason,
                Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed })
            ),
            "the busy Running row is ended as platform-reclaimed: {:?}",
            row.reason
        );
        assert_eq!(row.workload_addr, address, "the terminal row carries the address");
    }

    /// One production `VmReclamation` tick over the node target.
    async fn vm_reclamation_tick(
        state: &AppState,
        tick_n: u64,
    ) -> Result<(), crate::reconciler_runtime::ConvergenceError> {
        let name = ReconcilerName::new("vm-reclamation").expect("reconciler name");
        let target = TargetResource::new(&format!("node/{THIS_NODE}")).expect("node target");
        let now = Instant::now();
        crate::reconciler_runtime::run_convergence_tick(
            state,
            &name,
            &target,
            now,
            tick_n,
            now + Duration::from_secs(5),
        )
        .await
    }

    fn tick_context() -> TickContext {
        let now = Instant::now();
        TickContext {
            now,
            now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_100)),
            tick: 1,
            deadline: now + Duration::from_secs(1),
        }
    }

    /// One `ReclaimAllocationNetwork` attempt through the production shim.
    async fn reclaim_network_attempt(
        state: &AppState,
        provisioner: &RecordingProvisioner,
        id: &AllocationId,
    ) -> Result<(), crate::action_shim::ShimError> {
        let tick = tick_context();
        let attempt = crate::action_shim::dispatch_with_guest_network_provisioner_for_test(
            vec![Action::ReclaimAllocationNetwork { alloc_id: id.clone() }],
            state,
            &tick,
            provisioner,
        );
        tokio::time::timeout(Duration::from_secs(5), attempt)
            .await
            .expect("a reclaim attempt over a held scope returns promptly")
    }

    // -- a stuck VMM --------------------------------------------------------

    /// What a crashed `serve` left behind: a VMM that was killed and is not
    /// reaped, holding its vsock device, with its scope held.
    struct StuckVm {
        vmm: SimVmm,
        vmm_host: Arc<SimVmmHost>,
        host: SimVmHostState,
        alloc: AllocationId,
        pid: u32,
        address: Ipv4Addr,
        cid: GuestCid,
    }

    /// Lease `name` and launch its VM; returns once the VMM opened its device
    /// or its start ended: the VMM's pid (0 when nothing holds the CID),
    /// whether the launch holds its device with its start still pending, and
    /// the start task.
    async fn lease_and_launch(
        composition: &VmComposition,
        vmm: &SimVmm,
        name: &str,
    ) -> (u32, bool, tokio::task::JoinHandle<()>) {
        let plan = composition
            .pool
            .assign(alloc(name), &[], crate::shared_network_test_ports::test_owner_flow_ports())
            .expect("a free offset");
        let cid = plan.assignment().cid();
        let spec = super::sim_vm_host::vm_spec(&composition.root, &plan);
        let driver = composition.driver.clone();
        let start = tokio::spawn(async move {
            let _ =
                driver.start(&spec, &overdrive_sim::adapters::SimLaunchSupersession::new()).await;
        });
        for _ in 0..10_000 {
            if vmm.vsock_device_holders().contains_key(&cid) || start.is_finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let held = vmm.vsock_device_holders().get(&cid).copied();
        (held.unwrap_or(0), !start.is_finished() && held.is_some(), start)
    }

    /// Launch the first lease of a `prefix` pool as an unreapable VMM, crash
    /// `serve`, and kill the VMM: it ends `KilledNotReaped`, still holding its
    /// device. The returned host reports its scope and holds it while the VMM
    /// is unreaped (the one shared fact, `SimVmm::is_unreaped`).
    async fn leave_a_stuck_vm(root: &Path, prefix: Ipv4Net) -> StuckVm {
        let clock = SimClock::new();
        let vmm = SimVmm::new(Arc::new(clock.clone()));
        let vmm_host = SimVmmHost::new(vmm.clone(), false);
        let crashed = compose(&root.join("serve-1"), &vmm, &vmm_host, prefix, 1).await;
        vmm.inject_unreapable_process();
        let (pid, launched, start) = lease_and_launch(&crashed, &vmm, "stuck-0").await;
        assert!(launched, "precondition: the stuck VM's VMM holds its device");
        let plan = crashed.pool.snapshot().remove(&alloc("stuck-0")).expect("the lease's plan");
        let (address, cid) = (plan.assignment().workload_addr(), plan.assignment().cid());
        // `serve` crashes; its VMM does not.
        start.abort();
        drop(crashed);

        let killer = {
            let vmm = vmm.clone();
            tokio::spawn(async move {
                let control = VmControl { pid, api_socket: PathBuf::from("/sim/stuck/api.sock") };
                vmm.terminate(&control, Duration::ZERO).await
            })
        };
        for _ in 0..1_000 {
            if killer.is_finished() {
                break;
            }
            clock.tick(VMM_REAP_BOUND);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let outcome = killer.await.expect("the kill task joins").expect("terminate answers");
        assert!(
            matches!(outcome, VmTermination::KilledNotReaped),
            "precondition: the VMM is killed and not reaped: {outcome:?}"
        );
        assert!(vmm.is_unreaped(pid), "precondition: the process is unreaped");
        assert_eq!(vmm.vsock_device_holders().get(&cid), Some(&pid), "the device is still held");

        let id = alloc("stuck-0");
        let host = SimVmHostState::new();
        host.set_scope(id.clone(), BTreeSet::from([pid]));
        host.set_run_dir(id.clone());
        let reaped = vmm.clone();
        host.hold_scope_while(&id, move || reaped.is_unreaped(pid));
        StuckVm { vmm, vmm_host, host, alloc: id, pid, address, cid }
    }

    // -----------------------------------------------------------------
    // ADR-0350: boot reserves what is still busy and proceeds
    // -----------------------------------------------------------------

    /// PENDING (ADR-0350, QPROJ-2). The reservation a busy scope makes is
    /// reported `Reserved`: the CID its offset derives, no launch token, no plan.
    /// CONTRACT_SHAPE: bounded-change. Universe: leases, snapshot.
    #[tokio::test]
    async fn a_busy_scope_is_reserved_as_a_retiring_reservation_with_no_launch_token() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let id = alloc("busy-reserved-0");
        hold(&host, &id);
        let state = app_state(&tmp, Arc::new(host.clone()), &SimClock::new()).await;
        put_vm_job(&state).await;
        write_running(&state, "busy-reserved-0", Some(ADDRESS)).await;

        let result = converge(&state).await;

        assert!(result.is_ok(), "a busy scope never refuses the boot: {result:?}");
        let observation = state.guest_pool.lease_observation();
        assert_eq!(
            observation.leases.get(&id),
            Some(&GuestLeaseView::Reserved {
                cid: GuestCid::new(GUEST_CID_BASE + OFFSET_OF_ADDRESS).expect("in range")
            }),
            "the busy VM's offset is reserved, with no launch token"
        );
        assert!(state.guest_pool.snapshot().is_empty(), "a reservation carries no plan");
    }

    /// PENDING (ADR-0350 item 4). A busy scope whose row records no address
    /// cannot hold a CID (the address is on record before the VMM exists):
    /// the boot proceeds, the row is terminal, nothing is reserved.
    /// CONTRACT_SHAPE: bounded-change. Universe: the row, the pool's leases.
    #[tokio::test]
    async fn a_busy_scope_whose_row_records_no_address_reserves_nothing() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let id = alloc("busy-no-address-0");
        hold(&host, &id);
        let state = app_state(&tmp, Arc::new(host.clone()), &SimClock::new()).await;
        put_vm_job(&state).await;
        write_running(&state, "busy-no-address-0", None).await;

        let result = converge(&state).await;

        assert!(result.is_ok(), "a busy scope never refuses the boot: {result:?}");
        assert_platform_reclaimed(&row(&state, "busy-no-address-0").await, None);
        let observed = state.guest_pool.observe(std::slice::from_ref(&id));
        assert!(observed.leases.is_empty(), "no address, no reservation");
        assert_eq!(observed.occupancy.held, 0);
    }

    /// PENDING (ADR-0350 item 4, plan_reclamation row 5). A busy scope with no
    /// row is a stranded artifact: the boot proceeds, the artifacts stay for
    /// `VmReclamation`, nothing is reserved.
    /// CONTRACT_SHAPE: bounded-change. Universe: the pool's leases, the host.
    #[tokio::test]
    async fn a_busy_scope_with_no_row_reserves_nothing_and_does_not_refuse_the_boot() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let id = alloc("busy-no-row-0");
        hold(&host, &id);
        host.set_run_dir(id.clone());
        let state = app_state(&tmp, Arc::new(host.clone()), &SimClock::new()).await;

        let result = converge(&state).await;

        assert!(result.is_ok(), "a busy scope never refuses the boot: {result:?}");
        assert!(host.has_scope(&id), "the scope stays for VmReclamation");
        assert!(!host.artifacts_absent(&id), "the run directory stays for VmReclamation");
        assert!(
            state.guest_pool.observe(&[id]).leases.is_empty(),
            "a row-less scope reserves nothing"
        );
    }

    /// PENDING (ADR-0350 corrupt rows). A busy scope whose row names an address
    /// outside the guest prefix refuses the boot with the typed corrupt-row
    /// error naming the allocation and the address; nothing panics.
    /// CONTRACT_SHAPE: bounded-change. Universe: the result, the pool's leases.
    #[tokio::test]
    async fn a_row_address_outside_the_guest_prefix_refuses_the_boot_naming_the_allocation() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let id = alloc("corrupt-outside-0");
        hold(&host, &id);
        let state = app_state(&tmp, Arc::new(host.clone()), &SimClock::new()).await;
        put_vm_job(&state).await;
        let outside = Ipv4Addr::new(10, 1, 2, 3);
        write_running(&state, "corrupt-outside-0", Some(outside)).await;

        let result = converge(&state).await;

        assert!(
            matches!(
                &result,
                Err(ConvergeError::Reclamation(ReclamationError::CorruptLeftoverRow(
                    CorruptLeftoverRow::AddressOutsidePrefix { allocation, address }
                ))) if *allocation == id && *address == outside
            ),
            "a corrupt leftover row refuses the boot naming the allocation: {result:?}"
        );
        let message = result.expect_err("refused").to_string();
        assert!(message.contains(id.as_str()), "the message names the allocation: {message}");
        assert!(
            state.guest_pool.observe(&[id]).leases.is_empty(),
            "a corrupt row reserves nothing"
        );
    }

    /// PENDING (ADR-0350 corrupt rows, ordering). Two busy rows naming one
    /// address refuse the boot naming the second allocation and its holder; the
    /// first allocation's reservation stands (every executor ran, every valid
    /// reservation was made before the refusal).
    /// CONTRACT_SHAPE: bounded-change. Universe: the result, both rows, the pool.
    #[tokio::test]
    async fn two_busy_rows_naming_one_address_refuse_the_boot_and_keep_the_first_reservation() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let (first, second) = (alloc("dup-a-0"), alloc("dup-b-0"));
        hold(&host, &first);
        hold(&host, &second);
        let state = app_state(&tmp, Arc::new(host.clone()), &SimClock::new()).await;
        put_vm_job(&state).await;
        write_running(&state, "dup-a-0", Some(ADDRESS)).await;
        write_running(&state, "dup-b-0", Some(ADDRESS)).await;

        let result = converge(&state).await;

        assert!(
            matches!(
                &result,
                Err(ConvergeError::Reclamation(ReclamationError::CorruptLeftoverRow(
                    CorruptLeftoverRow::OffsetAlreadyHeld { allocation, held_by, address }
                ))) if *allocation == second && *held_by == first && *address == ADDRESS
            ),
            "a corrupt leftover row refuses the boot naming the allocation: {result:?}"
        );
        let observed = state.guest_pool.observe(&[first.clone(), second.clone()]);
        assert_eq!(observed.leases.get(&first), Some(&GuestAttachmentLease::Retiring));
        assert_eq!(observed.leases.get(&second), None, "the corrupt row reserved nothing");
        assert_platform_reclaimed(&row(&state, "dup-a-0").await, Some(ADDRESS));
        assert_platform_reclaimed(&row(&state, "dup-b-0").await, Some(ADDRESS));
    }

    /// PENDING (ADR-0350 item 3, "boot waits one bound"). Several stuck scopes
    /// cost one bound of boot delay: every executor's wait starts before any
    /// ends, so one advance of the injected clock by `KILL_SCOPE_SETTLE_BOUND`
    /// completes the boot, and every stuck VM's offset is reserved.
    /// CONTRACT_SHAPE: bounded-change. Universe: the clock, rows, the pool.
    #[tokio::test]
    async fn several_stuck_scopes_cost_one_bound_of_boot_delay_and_are_all_reserved() {
        const STUCK: usize = 3;
        let tmp = TempDir::new().expect("tmpdir");
        let clock = SimClock::new();
        let inner = SimVmHostState::new();
        let host = RecordingHost::new(inner.clone(), Some(clock.clone()));
        let ids: Vec<AllocationId> =
            (0..STUCK).map(|i| alloc(&format!("stuck-many-{i}"))).collect();
        for id in &ids {
            hold(&inner, id);
        }
        let state = app_state(&tmp, Arc::new(host.clone()), &clock).await;
        put_vm_job(&state).await;
        for (index, id) in ids.iter().enumerate() {
            let offset = u8::try_from(index).expect("small");
            write_running(&state, id.as_str(), Some(Ipv4Addr::new(100, 95, 0, 9 + offset))).await;
        }

        let converge_future = converge(&state);
        tokio::pin!(converge_future);
        let mut result = None;
        let mut advanced = false;
        for _ in 0..3_000 {
            tokio::select! {
                finished = &mut converge_future => {
                    result = Some(finished);
                    break;
                }
                () = tokio::time::sleep(Duration::from_millis(1)) => {}
            }
            if !advanced && host.kill_calls() == STUCK {
                clock.tick(KILL_SCOPE_SETTLE_BOUND);
                advanced = true;
            }
        }

        assert!(
            advanced,
            "every stuck scope's wait starts together, so boot waits one bound and not one per \
             scope (kill_scope called {} of {STUCK} times)",
            host.kill_calls()
        );
        let result = result.expect("the boot returns after one bound of the injected clock");
        assert!(result.is_ok(), "a busy scope never refuses the boot: {result:?}");
        let observed = state.guest_pool.observe(&ids);
        assert_eq!(observed.occupancy.held, 3, "every stuck VM's offset is reserved");
        assert!(
            ids.iter().all(|id| observed.leases.get(id) == Some(&GuestAttachmentLease::Retiring))
        );
    }

    /// One non-busy refusal beside a stuck VM: the refusal is the boot's
    /// error and the stuck VM's offset is reserved all the same, whichever
    /// scope the planner reaches first (`failing` and `stuck` fix the order).
    async fn refusal_beside_a_stuck_vm(failing: &str, stuck: &str) {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        hold(&host, &alloc(stuck));
        host.set_scope(alloc(failing), BTreeSet::from([4343]));
        // A held scope answers busy before the queue; the one un-held kill
        // takes the scripted refusal.
        host.script_kill_scope_failure(std::io::ErrorKind::PermissionDenied);
        let state = app_state(&tmp, Arc::new(host.clone()), &SimClock::new()).await;
        put_vm_job(&state).await;
        write_running(&state, failing, Some(Ipv4Addr::new(100, 95, 0, 20))).await;
        write_running(&state, stuck, Some(ADDRESS)).await;

        let result = converge(&state).await;

        assert!(
            matches!(&result, Err(ConvergeError::Reclamation(ReclamationError::Host(error)))
                if error.kind() == std::io::ErrorKind::PermissionDenied),
            "the refusal is the first non-busy executor error: {result:?}"
        );
        let observed = state.guest_pool.observe(&[alloc(stuck), alloc(failing)]);
        assert_eq!(
            observed.leases.get(&alloc(stuck)),
            Some(&GuestAttachmentLease::Retiring),
            "one scope's failure never drops another scope's reservation"
        );
        assert_eq!(observed.leases.get(&alloc(failing)), None, "the refused VM reserved nothing");
        assert_platform_reclaimed(&row(&state, stuck).await, Some(ADDRESS));
    }

    /// PENDING (ADR-0350 item 5). The refusal comes from the scope the planner
    /// reaches first; the stuck VM behind it is still killed, closed and reserved.
    #[tokio::test]
    async fn a_refusal_before_a_stuck_vm_never_drops_its_reservation() {
        refusal_beside_a_stuck_vm("fail-a-0", "stuck-b-0").await;
    }

    /// PENDING (ADR-0350 item 5). The stuck VM comes first: the boot is still
    /// refused for the later scope's non-busy error, with the reservation made.
    #[tokio::test]
    async fn a_refusal_after_a_stuck_vm_still_refuses_the_boot_with_its_reservation_made() {
        refusal_beside_a_stuck_vm("fail-b-0", "busy-a-0").await;
    }

    /// PENDING (ADR-0350, G-V0, QPROJ-7). A stuck VMM keeps its vsock device:
    /// the restarted boot proceeds, reserves its offset, and a launch sweep
    /// across the whole offset range (a next-fit wrap included) never receives
    /// its CID; every launch opens its own device (no clash) and the stuck
    /// VMM's holder entry is unchanged. `SimVmm::vsock_device_holders` is the
    /// arbiter.
    /// CONTRACT_SHAPE: bounded-change. Universe: pool leases, vsock holders.
    #[tokio::test]
    async fn a_launch_sweep_across_the_whole_offset_range_never_receives_the_stuck_vmms_cid() {
        let tmp = TempDir::new().expect("tmpdir");
        let prefix: Ipv4Net = "100.95.0.0/29".parse().expect("prefix");
        let stuck = leave_a_stuck_vm(tmp.path(), prefix).await;
        let mut restarted =
            compose(&tmp.path().join("serve-2"), &stuck.vmm, &stuck.vmm_host, prefix, 2).await;
        restarted.state.vm_host_state = Arc::new(stuck.host.clone());
        put_vm_job(&restarted.state).await;
        write_running(&restarted.state, stuck.alloc.as_str(), Some(stuck.address)).await;

        let result = converge(&restarted.state).await;

        assert!(result.is_ok(), "a busy scope never refuses the boot: {result:?}");
        assert_eq!(
            restarted.pool.lease_observation().leases.get(&stuck.alloc).map(GuestLeaseView::cid),
            Some(stuck.cid),
            "the stuck VMM's CID is reserved before the first assign"
        );
        let mut swept = Vec::new();
        for index in 0..4 {
            let (pid, launched, _start) =
                lease_and_launch(&restarted, &stuck.vmm, &format!("sweep-{index}")).await;
            let cid =
                restarted.pool.lease_observation().leases[&alloc(&format!("sweep-{index}"))].cid();
            assert_ne!(cid, stuck.cid, "a launch was handed the stuck VMM's CID");
            assert!(launched, "the launch opened its own device: no clash with the stuck VMM");
            assert_eq!(stuck.vmm.vsock_device_holders().get(&cid), Some(&pid));
            swept.push(cid);
        }
        let exhausted = restarted.pool.assign(
            alloc("sweep-extra"),
            &[],
            crate::shared_network_test_ports::test_owner_flow_ports(),
        );
        assert!(exhausted.is_err(), "every offset but the stuck VMM's is leased: {exhausted:?}");
        // Wrap: the cursor is past the last offset; a released offset is the
        // only one next-fit may take, never the reserved one.
        restarted.pool.release(&alloc("sweep-1"));
        let wrapped = restarted
            .pool
            .assign(
                alloc("sweep-wrap"),
                &[],
                crate::shared_network_test_ports::test_owner_flow_ports(),
            )
            .expect("the released offset");
        assert_eq!(wrapped.assignment().cid(), swept[1], "the wrap takes the released offset");
        assert_eq!(
            stuck.vmm.vsock_device_holders().get(&stuck.cid),
            Some(&stuck.pid),
            "the stuck VMM still holds its device"
        );
    }

    // -----------------------------------------------------------------
    // ADR-0350: the one release site over a reservation
    // -----------------------------------------------------------------

    /// PENDING (ADR-0350, feature delta "The late release"). Repeated
    /// `ReclaimAllocationNetwork` attempts over a reservation, with no driver
    /// entry for the allocation (a restarted process), return `Ok` promptly,
    /// keep the reservation while the scope is held and tear nothing down
    /// (a reservation has no plan).
    /// CONTRACT_SHAPE: bounded-change. Universe: pool leases, teardowns.
    #[tokio::test]
    async fn repeated_reclaim_attempts_over_a_reservation_return_ok_and_keep_it_while_the_scope_is_held()
     {
        let tmp = TempDir::new().expect("tmpdir");
        let stuck = leave_a_stuck_vm(tmp.path(), "100.95.0.0/29".parse().expect("prefix")).await;
        let state = app_state(&tmp, Arc::new(stuck.host.clone()), &SimClock::new()).await;
        put_vm_job(&state).await;
        write_running(&state, stuck.alloc.as_str(), Some(stuck.address)).await;
        let result = converge(&state).await;
        assert!(result.is_ok(), "a busy scope never refuses the boot: {result:?}");
        assert!(
            state.alloc_drivers.lock().get(&stuck.alloc).is_none(),
            "precondition: a restarted process has no driver entry for the allocation"
        );
        let provisioner = RecordingProvisioner::default();

        for attempt in 0..3 {
            reclaim_network_attempt(&state, &provisioner, &stuck.alloc)
                .await
                .unwrap_or_else(|error| panic!("attempt {attempt} over a reservation: {error}"));
            let observed = state.guest_pool.observe(std::slice::from_ref(&stuck.alloc));
            assert_eq!(
                observed.leases.get(&stuck.alloc),
                Some(&GuestAttachmentLease::Retiring),
                "the reservation is kept while the scope is held (attempt {attempt})"
            );
        }
        assert_eq!(
            provisioner.teardowns.load(Ordering::SeqCst),
            0,
            "a reservation has no teardown"
        );
    }

    /// PENDING (ADR-0350, ADR-0348 (b)). After the scripted reap, a
    /// `VmReclamation` pass removes the scope and one more reclaim attempt
    /// releases the reservation: the offset is assignable again, and no teardown
    /// was recorded for it.
    /// CONTRACT_SHAPE: bounded-change. Universe: pool leases, host scopes, teardowns.
    #[tokio::test]
    async fn a_reservation_is_released_by_a_reclaim_attempt_after_the_reap_and_a_reclamation_pass()
    {
        let tmp = TempDir::new().expect("tmpdir");
        let stuck = leave_a_stuck_vm(tmp.path(), "100.95.0.0/29".parse().expect("prefix")).await;
        let state = app_state(&tmp, Arc::new(stuck.host.clone()), &SimClock::new()).await;
        put_vm_job(&state).await;
        write_running(&state, stuck.alloc.as_str(), Some(stuck.address)).await;
        let result = converge(&state).await;
        assert!(result.is_ok(), "a busy scope never refuses the boot: {result:?}");
        let provisioner = RecordingProvisioner::default();
        reclaim_network_attempt(&state, &provisioner, &stuck.alloc).await.expect("held attempt");
        assert!(vm_reclamation_tick(&state, 1).await.is_err(), "the pass meets the held scope");

        assert!(stuck.vmm.exit_process(stuck.pid, None, Some(9)), "the scripted reap");
        vm_reclamation_tick(&state, 2).await.expect("the pass removes the reaped scope");
        assert!(!stuck.host.has_scope(&stuck.alloc), "the scope is gone after the reap");
        reclaim_network_attempt(&state, &provisioner, &stuck.alloc).await.expect("late attempt");

        let observed = state.guest_pool.observe(std::slice::from_ref(&stuck.alloc));
        assert!(
            observed.leases.is_empty(),
            "the reservation is released once the scope is absent: {:?}",
            observed.leases
        );
        assert_eq!(observed.occupancy.held, 0);
        assert!(
            state
                .guest_pool
                .assign(
                    stuck.alloc.clone(),
                    &[],
                    crate::shared_network_test_ports::test_owner_flow_ports()
                )
                .is_ok(),
            "the allocation's offset is assignable again"
        );
        assert_eq!(
            provisioner.teardowns.load(Ordering::SeqCst),
            0,
            "a reservation has no teardown"
        );
    }

    // -----------------------------------------------------------------
    // ADR-0354: a busy boot executor step writes the row, not the discard
    // -----------------------------------------------------------------

    /// PENDING (ADR-0354, S-RB-13). A busy boot step ends the `Running` row as
    /// platform-reclaimed carrying the address and leaves the scope, run
    /// directory and clone; once the hold clears the next `VmReclamation` pass
    /// discards them and writes no row.
    /// CONTRACT_SHAPE: bounded-change. Universe: the row, the host's surfaces.
    #[tokio::test]
    async fn a_busy_boot_step_writes_the_row_and_the_next_pass_discards_the_artifacts() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let id = alloc("busy-boot-0");
        let held = hold(&host, &id);
        host.set_run_dir(id.clone());
        host.set_clone(id.clone(), tmp.path().join("clone.img"));
        let state = app_state(&tmp, Arc::new(host.clone()), &SimClock::new()).await;
        put_vm_job(&state).await;
        write_running(&state, "busy-boot-0", Some(ADDRESS)).await;

        let result = converge(&state).await;

        assert!(result.is_ok(), "a busy scope never refuses the boot: {result:?}");
        let after_boot = row(&state, "busy-boot-0").await;
        assert_platform_reclaimed(&after_boot, Some(ADDRESS));
        let observed = host.observe().await.expect("observe");
        assert!(
            observed.scopes.contains_key(&id)
                && observed.run_dirs.contains(&id)
                && observed.clones.contains_key(&id),
            "a busy boot step discards no artifact: {observed:?}"
        );

        held.store(false, Ordering::SeqCst);
        vm_reclamation_tick(&state, 1).await.expect("the pass after the hold clears");
        assert!(!host.has_scope(&id), "the pass removes the scope");
        assert!(host.artifacts_absent(&id), "the pass discards the run directory and clone");
        assert_eq!(row(&state, "busy-boot-0").await, after_boot, "the pass writes no row");
    }

    /// PENDING (ADR-0354 consequences). After boot ended the row, a
    /// steady-state tick that meets the same busy scope leaves the row
    /// unchanged and still discards nothing.
    /// CONTRACT_SHAPE: unbounded-preservation. Universe: the row, host surfaces.
    #[tokio::test]
    async fn the_steady_state_tick_that_meets_the_busy_scope_leaves_the_boot_written_row_unchanged()
    {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let id = alloc("busy-then-tick-0");
        hold(&host, &id);
        host.set_run_dir(id.clone());
        let state = app_state(&tmp, Arc::new(host.clone()), &SimClock::new()).await;
        put_vm_job(&state).await;
        write_running(&state, "busy-then-tick-0", Some(ADDRESS)).await;
        let result = converge(&state).await;
        assert!(result.is_ok(), "a busy scope never refuses the boot: {result:?}");
        let after_boot = row(&state, "busy-then-tick-0").await;

        let tick = vm_reclamation_tick(&state, 1).await;

        assert!(tick.is_err(), "the tick meets the busy scope: {tick:?}");
        assert_eq!(row(&state, "busy-then-tick-0").await, after_boot, "the tick writes no row");
        assert!(host.has_scope(&id) && !host.artifacts_absent(&id), "the tick discards nothing");
    }

    // -----------------------------------------------------------------
    // ADR-0349 / ADR-0354: the steady-state VmReclamation tick over a held scope
    // -----------------------------------------------------------------

    /// GUARD (ADR-0349 item 6, ADR-0354). A steady-state tick that meets a busy
    /// scope of a `Running` row writes no row, discards nothing and leaves the
    /// scope: only the boot step is exempt from "busy means no row".
    /// CONTRACT_SHAPE: unbounded-preservation. Universe: the row, host surfaces.
    #[tokio::test]
    async fn guard_a_tick_over_a_busy_scope_of_a_running_row_writes_no_row_and_discards_nothing() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let id = alloc("tick-busy-running-0");
        hold(&host, &id);
        host.set_run_dir(id.clone());
        let state = app_state(&tmp, Arc::new(host.clone()), &SimClock::new()).await;
        put_vm_job(&state).await;
        write_running(&state, "tick-busy-running-0", Some(ADDRESS)).await;
        let before = row(&state, "tick-busy-running-0").await;

        let tick = vm_reclamation_tick(&state, 1).await;

        assert!(tick.is_err(), "the busy scope is the tick's error: {tick:?}");
        assert_eq!(row(&state, "tick-busy-running-0").await, before, "no row is written");
        assert!(host.has_scope(&id), "the scope stays");
        assert!(!host.artifacts_absent(&id), "no artifact is discarded");
    }

    /// GUARD (ADR-0349 item 6). A tick over a held scope of a terminal
    /// allocation discards nothing, writes no row and still runs the tick's
    /// other actions; the next tick, after the hold clears, removes the scope
    /// and its artifacts and writes no row.
    /// CONTRACT_SHAPE: bounded-change. Universe: both rows, host surfaces.
    #[tokio::test]
    async fn guard_a_tick_over_a_held_scope_runs_the_other_actions_and_the_next_tick_removes_it() {
        let tmp = TempDir::new().expect("tmpdir");
        let host = SimVmHostState::new();
        let (busy, free) = (alloc("tick-a-busy-0"), alloc("tick-b-free-0"));
        let held = hold(&host, &busy);
        host.set_run_dir(busy.clone());
        host.set_scope(free.clone(), BTreeSet::from([4343]));
        let state = app_state(&tmp, Arc::new(host.clone()), &SimClock::new()).await;
        put_vm_job(&state).await;
        write_operator_stopped(&state, "tick-a-busy-0", Some(ADDRESS)).await;
        write_running(&state, "tick-b-free-0", Some(Ipv4Addr::new(100, 95, 0, 20))).await;
        let busy_before = row(&state, "tick-a-busy-0").await;

        let tick = vm_reclamation_tick(&state, 1).await;

        assert!(tick.is_err(), "the held scope is the tick's error: {tick:?}");
        assert_eq!(
            row(&state, "tick-a-busy-0").await,
            busy_before,
            "the busy scope's row is unchanged"
        );
        assert!(host.has_scope(&busy) && !host.artifacts_absent(&busy), "nothing was discarded");
        let free_row = row(&state, "tick-b-free-0").await;
        assert_eq!(free_row.state, AllocState::Terminated, "the other action ran");
        assert!(
            matches!(
                free_row.reason,
                Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed })
            ),
            "the other action ended its row as platform-reclaimed: {:?}",
            free_row.reason
        );

        held.store(false, Ordering::SeqCst);
        vm_reclamation_tick(&state, 2).await.expect("the next tick after the hold clears");
        assert!(!host.has_scope(&busy), "the next tick removes the scope");
        assert!(host.artifacts_absent(&busy), "and its artifacts");
        assert_eq!(row(&state, "tick-a-busy-0").await, busy_before, "and writes no row");
    }

    /// GUARD (ADR-0348 test (b) shape). A pass over a terminal allocation whose
    /// scope cannot be removed emits again on the next pass (one more
    /// `kill_scope`, still busy) and removes the scope after the VMM is
    /// reaped (`SimVmm::exit_process`, the one fact the hold reads).
    /// CONTRACT_SHAPE: bounded-change. Universe: kill calls, host surfaces.
    #[tokio::test]
    async fn guard_a_pass_over_a_terminal_allocation_with_an_unremovable_scope_emits_again_until_the_reap()
     {
        let tmp = TempDir::new().expect("tmpdir");
        let stuck = leave_a_stuck_vm(tmp.path(), "100.95.0.0/29".parse().expect("prefix")).await;
        let host = RecordingHost::new(stuck.host.clone(), None);
        let state = app_state(&tmp, Arc::new(host.clone()), &SimClock::new()).await;
        put_vm_job(&state).await;
        write_operator_stopped(&state, stuck.alloc.as_str(), Some(stuck.address)).await;

        assert!(
            vm_reclamation_tick(&state, 1).await.is_err(),
            "the first pass meets the busy scope"
        );
        assert!(vm_reclamation_tick(&state, 2).await.is_err(), "the next pass emits again");
        assert_eq!(host.kill_calls(), 2, "each pass tried the removal once");
        assert!(stuck.host.has_scope(&stuck.alloc), "the scope is kept until the reap");

        assert!(stuck.vmm.exit_process(stuck.pid, None, Some(9)), "the scripted reap");
        vm_reclamation_tick(&state, 3).await.expect("the pass after the reap");
        assert!(!stuck.host.has_scope(&stuck.alloc), "the pass after the reap removes the scope");
        assert_eq!(host.kill_calls(), 3);
    }
}

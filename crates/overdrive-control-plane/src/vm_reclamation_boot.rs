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

use overdrive_core::reconcilers::Action;
use overdrive_core::traits::driver::DriverType;
use overdrive_reconcilers::vm_reclamation::SupervisionSet;
use overdrive_reconcilers::{VmReclamationState, plan_reclamation};

use crate::AppState;
use crate::action_shim::reclamation::{
    ReclamationError, execute_discard_stranded_artifacts, execute_reclaim_allocation,
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
/// three refuses the whole boot (mirroring `adopt_on_restart_recovery`'s
/// own fail-closed posture) — the same cgroup tree
/// `adopt_on_restart_recovery` reads next must not be read out from
/// under a still-in-flight `rmdir`.
#[expect(
    clippy::significant_drop_tightening,
    reason = "the allocator/listener_facts MutexGuards are lent into the HydrationContext \
              borrow-bundle and must outlive the desired-side join .await"
)]
pub async fn converge(state: &AppState) -> Result<(), ConvergeError> {
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

    for action in plan_reclamation(&desired, &actual) {
        match action {
            Action::DiscardStrandedArtifacts { alloc_id } => {
                execute_discard_stranded_artifacts(
                    &alloc_id,
                    &state.drivers,
                    state.vm_host_state.as_ref(),
                )
                .await
                .map_err(ConvergeError::Reclamation)?;
            }
            Action::ReclaimAllocation { alloc_id } => {
                execute_reclaim_allocation(
                    &alloc_id,
                    &state.drivers,
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
            cpu_milli: 1_000,
            memory_bytes: 1 << 30,
        })
        .expect("valid node");
        let job = Job {
            id: workload_id.clone(),
            replicas: NonZeroU32::new(1).expect("nonzero"),
            resources: Resources { cpu_milli: 500, memory_bytes: 134_217_728 },
            driver: WorkloadDriver::Vm(Vm {
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
            workload_id,
            job: Some(job),
            desired_to_stop: false,
            generation: 0,
            nodes: BTreeMap::from([(node.id.clone(), node)]),
            allocations: BTreeMap::new(),
            workload_kind: WorkloadKind::Job,
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
            service_ports: Vec::new(),
            guest_attachments: overdrive_core::traits::GuestAttachmentObservation {
                occupancy: overdrive_core::guest_network::GuestAttachmentOccupancy {
                    held: 0,
                    retiring: 0,
                },
                leases: std::collections::BTreeMap::new(),
            },
        };
        let lifecycle_actual = WorkloadLifecycleState {
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

        fn record(&self, alloc: AllocationId, pid: u32) {
            self.facts.lock().vms.entry(alloc).or_default().insert(pid);
        }

        /// The pids whose kill a `kill_scope` call is waiting for (gated).
        pub(crate) fn kills_requested(&self) -> BTreeSet<u32> {
            self.facts.lock().requested.clone()
        }

        /// Release the held kill of `pid` (gated).
        pub(crate) fn grant_kill(&self, pid: u32) {
            self.facts.lock().granted.insert(pid);
            self.wake.notify_waiters();
        }

        /// Wake a waiting `kill_scope` to re-check its processes (after a
        /// VMM exited by itself).
        pub(crate) fn notify(&self) {
            self.wake.notify_waiters();
        }

        /// Forget every outstanding kill request and grant: the `serve`
        /// process whose boot reclamation made them died with them.
        pub(crate) fn clear_requests(&self) {
            let mut facts = self.facts.lock();
            facts.requested.clear();
            facts.granted.clear();
        }

        /// The VMM pids the host placed in `alloc`'s scope (the relabelling
        /// of a production allocation to its VMM processes).
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
                let live = pids.iter().copied().filter(|pid| self.vmm.is_live(*pid)).collect();
                observation.scopes.insert(alloc.clone(), ScopeFacts { pids: live });
                observation.run_dirs.insert(alloc.clone());
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
                        self.vmm.kill(pid);
                    }
                    return Ok(());
                }
                notified.await;
            }
        }

        async fn discard_artifacts(&self, alloc: &AllocationId) -> std::io::Result<()> {
            self.facts.lock().vms.remove(alloc);
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
            self.inner.terminate(control, grace).await
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
        let probe_runner = Arc::new(ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::new(clock.clone()),
            Arc::new(SimObservationStore::single_peer(
                NodeId::new("cid-lease").expect("valid node ID"),
                0,
            )) as Arc<dyn ObservationStore>,
            Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
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
            alloc: plan.alloc().clone(),
            identity: SpiffeId::new("spiffe://overdrive.local/workload/cid-lease/alloc/x")
                .expect("valid spiffe id"),
            driver: DriverPayload::Vm(VmPayload {
                command: "/sbin/init".to_owned(),
                args: vec![],
                kernel: root.join("vmlinuz"),
                rootfs: root.join("master.img"),
            }),
            resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
            probe_descriptors: Vec::new(),
            transport: Some(plan.assignment().clone()),
            service_ports: Vec::new(),
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
            let _ = driver.start(&spec).await;
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
        let vmm = SimVmm::new();
        let host = SimVmmHost::new(vmm.clone(), false);

        let crashed = compose(&tmp.path().join("serve-1"), &vmm, &host, prefix, 1).await;
        let (orphan, launched, start) = lease_and_launch(&crashed, &vmm, "orphan-0").await;
        assert!(launched, "precondition: the first process's VMM holds its device");
        let orphan_cid = crashed.pool.lease_observation().leases[&alloc("orphan-0")].cid;
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
        let fresh_cid = restarted.pool.lease_observation().leases[&alloc("fresh-0")].cid;
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
        let vmm = SimVmm::new();
        let host = SimVmmHost::new(vmm.clone(), false);

        let crashed = compose(&tmp.path().join("serve-1"), &vmm, &host, prefix, 1).await;
        let (first, launched_first, start_first) =
            lease_and_launch(&crashed, &vmm, "orphan-0").await;
        let (second, launched_second, start_second) =
            lease_and_launch(&crashed, &vmm, "orphan-1").await;
        assert!(launched_first && launched_second, "precondition: both VMMs hold their devices");
        let orphans = crashed.pool.lease_observation();
        let cids = [orphans.leases[&alloc("orphan-0")].cid, orphans.leases[&alloc("orphan-1")].cid];
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
            [fresh.leases[&alloc("fresh-0")].cid, fresh.leases[&alloc("fresh-1")].cid],
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
        let vmm = SimVmm::new();
        let host = SimVmmHost::new(vmm.clone(), false);

        let crashed = compose(&tmp.path().join("serve-1"), &vmm, &host, prefix, 1).await;
        let (orphan, _, start) = lease_and_launch(&crashed, &vmm, "orphan-0").await;
        let orphan_cid = crashed.pool.lease_observation().leases[&alloc("orphan-0")].cid;
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
        let outcome = tokio::time::timeout(Duration::from_secs(10), restarted.driver.start(&spec))
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
        let vmm = SimVmm::new();
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

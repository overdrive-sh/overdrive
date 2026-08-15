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
//! `desired.allocations` (the intent-side two-surface join) is left
//! EMPTY here, mirroring `reconciler_runtime::hydrate_desired`'s own
//! still-skeletal `VmReclamation` arm (unchanged by this step; a later
//! step's obligation alongside `execute_reclaim_allocation` — see
//! `action_shim::reclamation`'s module docs). `Action::ReclaimAllocation`
//! is therefore NOT executed by this drive in this step: with desired
//! empty, `plan_reclamation` can only route through the "no entry"
//! branch of the decision table (brief.md §105a.4 rows 4-6), never rows
//! 1-3, so `ReclaimAllocation` cannot be emitted against a
//! `VmReclamationState` this function builds. Every action this drive's
//! `plan_reclamation` call CAN produce today is
//! `Action::DiscardStrandedArtifacts`, which IS fully executed.

use std::collections::{BTreeMap, BTreeSet};

use overdrive_core::reconcilers::{Action, VmReclamationState, plan_reclamation};
use overdrive_core::reconcilers::vm_reclamation::SupervisionSet;
use overdrive_core::traits::driver::DriverType;

use crate::AppState;
use crate::action_shim::reclamation::{ReclamationError, execute_discard_stranded_artifacts};

/// Run one boot-epoch `VmReclamation` pass against `state`. Idempotent —
/// safe to call on every boot regardless of whether any VM allocation
/// ever existed on this node (every surface then reads empty and
/// `plan_reclamation` returns `Vec::new()`).
///
/// # Errors
///
/// Returns [`ConvergeError::Host`] on a genuine (non-benign-absence)
/// `VmHostState::observe` substrate failure, or
/// [`ConvergeError::Reclamation`] when an executor fails. Either refuses
/// the whole boot (mirroring `adopt_on_restart_recovery`'s own
/// fail-closed posture) — the same cgroup tree `adopt_on_restart_recovery`
/// reads next must not be read out from under a still-in-flight `rmdir`.
pub async fn converge(state: &AppState) -> Result<(), ConvergeError> {
    // brief.md §105a.2's pinned order: `observe()` FIRST, supervision
    // LAST. At the boot epoch this ordering has no observable
    // consequence on its own (nothing races the boot drive), but it is
    // the SAME hydration path the steady-state tick uses — brief.md
    // §105a.6: "one observation function," not a second one that happens
    // to differ in ordering.
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

    let desired = VmReclamationState::default();
    let actual = VmReclamationState { allocations: BTreeMap::new(), host, supervision };

    for action in plan_reclamation(&desired, &actual) {
        if let Action::DiscardStrandedArtifacts { alloc_id } = action {
            execute_discard_stranded_artifacts(&alloc_id, state.vm_host_state.as_ref())
                .await
                .map_err(ConvergeError::Reclamation)?;
        }
        // Action::ReclaimAllocation — see this module's own docs for why
        // it is unreachable from this function's own `desired` (always
        // empty) and, independent of that, why its executor does not
        // exist yet in this step's scope.
    }
    Ok(())
}

/// Failure surface for [`converge`].
#[derive(Debug, thiserror::Error)]
pub enum ConvergeError {
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
}

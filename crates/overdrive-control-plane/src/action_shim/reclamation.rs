//! Executor(s) for the `VmReclamation` reconciler's `Action` variants
//! (ADR-0083 §D7, `brief.md` §105a.5, GH #42).
//!
//! # Scope of this step (02-02)
//!
//! Only [`execute_discard_stranded_artifacts`] is implemented here. It
//! authors NO ending — `kill_scope` -> `discard_artifacts`, nothing else
//! — and needs no new vocabulary, so it is fully self-contained.
//!
//! `execute_reclaim_allocation` (the `Action::ReclaimAllocation`
//! executor, brief.md §105a.5) is **deliberately NOT implemented in this
//! step**. Its terminal-row write needs `StoppedBy::PlatformReclaimed`,
//! which per `overdrive_core::transition_reason`'s own module docs "has
//! not landed... [and] lands with `execute_reclaim_allocation`... not
//! with this reconciler skeleton [step 02-01]" — bundled with an
//! ADR-0081/ADR-0078 amendment touching `overdrive-core` files outside
//! this step's scope. The scenarios that exercise it (S-VM-79 "a
//! write-time terminality guard", S-VM-80 "P2 over VmReclamation",
//! S-VM-81 "the fourth evaluation") are explicitly routed to
//! `overdrive-control-plane`, NOT this step, in
//! `distill/test-scenarios.md`'s Driving Ports table (DWD-16) — and are
//! absent from this step's own `scenario_name` list. The action shim's
//! `dispatch_single` RED scaffold for `Action::ReclaimAllocation` /
//! `Action::DiscardStrandedArtifacts` (`action_shim/mod.rs`) is
//! therefore left untouched for the `ReclaimAllocation` arm; a later
//! step wires both the executor and its `dispatch_single` arm together.
//!
//! [`crate::vm_reclamation_boot::converge`] (this step) calls
//! [`execute_discard_stranded_artifacts`] directly for every
//! `Action::DiscardStrandedArtifacts` `plan_reclamation` emits, and
//! leaves `Action::ReclaimAllocation` unexecuted (see that module's own
//! docs).

use overdrive_core::AllocationId;
use overdrive_core::traits::vm_host_state::VmHostState;

/// Errors from the reclamation executor(s).
#[derive(Debug, thiserror::Error)]
pub enum ReclamationError {
    /// `VmHostState::kill_scope` or `VmHostState::discard_artifacts`
    /// failed at the substrate level (a non-benign-absence I/O error —
    /// both methods are idempotent on an already-absent resource per
    /// their own port-trait contract).
    #[error("VmHostState operation failed: {0}")]
    Host(#[from] std::io::Error),
}

/// `Action::DiscardStrandedArtifacts` executor (brief.md §105a.5).
/// Authors NO ending: kills the scope (if any) and removes the run
/// directory + rootfs clone (if any), and nothing else — no row write,
/// no evaluation submitted. This is the operational form of DD-5's
/// "declared delta empty over the observation universe": there is no
/// code path from this function to a row, so `after == before` on the
/// observation store is not an assertion a caller must remember to make.
///
/// Still kills a live VMM scope when one exists — a *terminal*
/// allocation whose VMM survived a failed stop is precisely SD-1's
/// unstoppable orphan, and killing it authors no ending because that
/// allocation's ending is already authored.
///
/// # Errors
///
/// Returns [`ReclamationError::Host`] on a genuine (non-benign-absence)
/// substrate failure from either `kill_scope` or `discard_artifacts`.
pub async fn execute_discard_stranded_artifacts(
    alloc_id: &AllocationId,
    host: &dyn VmHostState,
) -> Result<(), ReclamationError> {
    let scope = overdrive_core::cgroup::CgroupPath::for_alloc(alloc_id);
    host.kill_scope(&scope).await?;
    host.discard_artifacts(alloc_id).await?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::collections::BTreeSet;

    use overdrive_sim::adapters::vm_host_state::SimVmHostState;

    use super::*;

    fn alloc(id: &str) -> AllocationId {
        AllocationId::new(id).expect("valid AllocationId")
    }

    #[tokio::test]
    async fn discards_scope_run_dir_and_clone_and_authors_nothing() {
        let host = SimVmHostState::new();
        let a = alloc("vm-discard-0");
        host.set_scope(a.clone(), BTreeSet::from([4242]));
        host.set_run_dir(a.clone());
        host.set_clone(a.clone(), std::path::PathBuf::from("/staging/vm-discard-0.img"));

        execute_discard_stranded_artifacts(&a, &host).await.expect("executor succeeds");

        let observed = host.observe().await.expect("observe succeeds");
        assert!(
            !observed.scopes.contains_key(&a),
            "kill_scope must remove the scope; still present: {observed:?}"
        );
        assert!(
            !observed.run_dirs.contains(&a),
            "discard_artifacts must remove the run dir; still present: {observed:?}"
        );
        assert!(
            !observed.clones.contains_key(&a),
            "discard_artifacts must remove the clone; still present: {observed:?}"
        );
    }

    #[tokio::test]
    async fn is_idempotent_on_an_already_absent_allocation() {
        let host = SimVmHostState::new();
        let a = alloc("vm-discard-absent");
        // Never seeded on any surface — must still be Ok per the port's
        // own "idempotent on absence" contract.
        execute_discard_stranded_artifacts(&a, &host)
            .await
            .expect("absent allocation is a no-op success, not an error");
    }
}

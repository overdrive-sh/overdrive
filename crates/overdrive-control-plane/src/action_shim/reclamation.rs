//! Executor(s) for the `VmReclamation` reconciler's `Action` variants
//! (ADR-0083 §D7, `brief.md` §105a.5, GH #42).
//!
//! # Scope
//!
//! Both executors are implemented here. [`execute_discard_stranded_artifacts`]
//! (step 02-02) authors NO ending — `kill_scope` -> `discard_artifacts`,
//! nothing else. [`execute_reclaim_allocation`] (step 02-03) authors a
//! Platform Reclamation ending: a write-time terminality guard (gating
//! the WHOLE executor, not just the row write — iteration-2 review
//! NEW-1) -> `kill_scope` -> `discard_artifacts` -> write the terminal
//! row (`StoppedBy::PlatformReclaimed`) -> submit the four evaluations
//! the exit observer submits per exit (`worker/exit_observer.rs:234`,
//! `:254`, `:295`, `:318-320` — `workload_lifecycle`,
//! `backend_discovery_bridge`, `service_lifecycle`, and the fourth,
//! `svid_lifecycle`, whose omission leaves the node holding the dead
//! allocation's leaf private key; ADR-0083 §D7).
//!
//! [`crate::vm_reclamation_boot::converge`] calls both executors
//! directly on the `Vec<Action>` `plan_reclamation` returns. Step 02-03
//! fills the boot drive's own `desired` via
//! [`crate::reconciler_runtime::hydrate_vm_reclamation_desired`] — the
//! SAME join the steady-state tick's `hydrate_desired` arm calls — so
//! `Action::ReclaimAllocation` (this module's [`execute_reclaim_allocation`])
//! is reachable from BOTH drives, not only the steady-state tick. The
//! steady-state tick reaches both executors via
//! `action_shim::dispatch_single`.

use overdrive_core::AllocationId;
use overdrive_core::eval_broker::{Evaluation, EvaluationBroker};
use overdrive_core::id::NodeId;
use overdrive_core::reconcilers::TargetResource;
use overdrive_core::traits::clock::Clock;
use overdrive_core::TransitionReason;
use overdrive_core::traits::observation_store::{
    AllocState, LogicalTimestamp, ObservationRow, ObservationStore, ObservationStoreError,
};
use overdrive_core::traits::vm_host_state::VmHostState;
use overdrive_core::transition_reason::StoppedBy;

use super::build_alloc_status_row;

/// Errors from the reclamation executor(s).
#[derive(Debug, thiserror::Error)]
pub enum ReclamationError {
    /// `VmHostState::kill_scope` or `VmHostState::discard_artifacts`
    /// failed at the substrate level (a non-benign-absence I/O error —
    /// both methods are idempotent on an already-absent resource per
    /// their own port-trait contract).
    #[error("VmHostState operation failed: {0}")]
    Host(#[from] std::io::Error),
    /// The prior-row re-read (the write-time terminality guard) or the
    /// terminal-row write itself failed at the observation-store level.
    /// NOT emitted for a refused race (brief.md §105a.5) — that path
    /// returns `Ok(())` by design; this variant is a genuine substrate
    /// failure.
    #[error("ObservationStore operation failed: {0}")]
    Observation(#[from] ObservationStoreError),
}

/// Canonical `ReconcilerName`s for the four evaluations
/// [`execute_reclaim_allocation`] submits, sourced from each
/// reconciler's trait const — the same `refactor-reconciler-static-name`
/// pattern `worker/exit_observer.rs`'s own four name helpers use, so
/// there is exactly one place to change per reconciler if a canonical
/// name ever moves.
mod evaluation_targets {
    use overdrive_core::reconcilers::backend_discovery_bridge::BackendDiscoveryBridge;
    use overdrive_core::reconcilers::svid_lifecycle::SvidLifecycle;
    use overdrive_core::reconcilers::{Reconciler, ReconcilerName, WorkloadLifecycle};
    use overdrive_core::service_lifecycle::ServiceLifecycleReconciler;

    #[allow(clippy::expect_used)]
    pub(super) fn workload_lifecycle() -> ReconcilerName {
        ReconcilerName::new(<WorkloadLifecycle as Reconciler>::NAME)
            .expect("WorkloadLifecycle::NAME is a valid ReconcilerName by construction")
    }

    #[allow(clippy::expect_used)]
    pub(super) fn backend_discovery_bridge() -> ReconcilerName {
        ReconcilerName::new(<BackendDiscoveryBridge as Reconciler>::NAME)
            .expect("BackendDiscoveryBridge::NAME is a valid ReconcilerName by construction")
    }

    #[allow(clippy::expect_used)]
    pub(super) fn service_lifecycle() -> ReconcilerName {
        ReconcilerName::new(<ServiceLifecycleReconciler as Reconciler>::NAME)
            .expect("ServiceLifecycleReconciler::NAME is a valid ReconcilerName by construction")
    }

    #[allow(clippy::expect_used)]
    pub(super) fn svid_lifecycle() -> ReconcilerName {
        ReconcilerName::new(<SvidLifecycle as Reconciler>::NAME)
            .expect("SvidLifecycle::NAME is a valid ReconcilerName by construction")
    }
}

/// `Action::ReclaimAllocation` executor (brief.md §105a.5, ADR-0083 §D7).
///
/// Re-reads the row `plan_reclamation` observed at diff time — a GUARD
/// over the WHOLE executor, not merely a `workload_id` lookup
/// (iteration-2 review NEW-1; Hera's DD-1(b.i) consequence 3):
///
/// ```text
/// Some(row) if !row.state.is_terminal()  -> AUTHORISED  — proceed
/// Some(row)  // is_terminal()            -> REFUSED     — do NOTHING; Ok(())
/// None                                   -> REFUSED     — do NOTHING; Ok(())
/// ```
///
/// A refusal is a total no-op — no kill, no discard, no row write — and
/// returns `Ok(())`; it is NOT a degradation to disposal (that would
/// smuggle back the one-command-two-behaviours shape DD-5's two-variant
/// split refuses). A structured `vm.reclamation.refused` event carries
/// `alloc_id` and the observed state. The next tick re-observes and, for
/// a genuinely terminal row, correctly re-decides
/// `DiscardStrandedArtifacts` on its own account.
///
/// On the AUTHORISED branch: `kill_scope` -> `discard_artifacts` -> write
/// the terminal row (`state: Terminated`, `reason: Stopped { by:
/// PlatformReclaimed }`, `terminal: None`) through the existing LWW
/// merge, so a re-run is a same-value write -> submit the four
/// evaluations the exit observer submits per exit.
///
/// `workload_id` is resolved by re-reading the row (the SAME guard read)
/// — the `alloc_id`-only payload carries no `workload_id` (DD-5).
///
/// # Errors
///
/// [`ReclamationError::Host`] on a genuine `VmHostState` substrate
/// failure; [`ReclamationError::Observation`] on a genuine
/// `ObservationStore` substrate failure (the prior-row read or the
/// terminal-row write). Never returned for a refused race — that path
/// is `Ok(())`.
pub async fn execute_reclaim_allocation(
    alloc_id: &AllocationId,
    host: &dyn VmHostState,
    obs: &dyn ObservationStore,
    clock: &dyn Clock,
    writer_node: &NodeId,
    broker: &parking_lot::Mutex<EvaluationBroker>,
) -> Result<(), ReclamationError> {
    // The write-time terminality guard — a precondition of the whole
    // executor. A tick decides at t while its executor runs at t + ε; an
    // ending authored inside that gap is an ending, and DD-1(b)'s refusal
    // to overwrite an authored ending binds this path exactly as it binds
    // the disposal path.
    let Some(prior_row) = obs.alloc_status_row(alloc_id).await? else {
        tracing::info!(
            name: "vm.reclamation.refused",
            alloc_id = %alloc_id,
            "execute_reclaim_allocation refused: no row observed for this allocation \
             (an unrelated ending-authoring path may have raced ahead); total no-op"
        );
        return Ok(());
    };
    if prior_row.state.is_terminal() {
        tracing::info!(
            name: "vm.reclamation.refused",
            alloc_id = %alloc_id,
            observed_state = ?prior_row.state,
            "execute_reclaim_allocation refused: row is already terminal (an unrelated \
             ending-authoring path won the race before this re-read resolved); total no-op, \
             never a degradation to DiscardStrandedArtifacts"
        );
        return Ok(());
    }

    // AUTHORISED — kill the scope, discard the run-dir/clone artifacts.
    let scope = overdrive_core::cgroup::CgroupPath::for_alloc(alloc_id);
    host.kill_scope(&scope).await?;
    host.discard_artifacts(alloc_id).await?;

    // Write the terminal row through the existing LWW merge. `tick_floor`
    // has no `TickContext` to source from here (the executor's pinned
    // surface carries `clock`, not `tick`) — mirrors the documented
    // "outside a convergence loop" shape (`LogicalTimestamp::dominating`'s
    // own docs name the exit observer as the precedent for this exact
    // case), deriving a monotonic floor from wall-clock seconds rather
    // than a hardcoded `0` so the floor is non-trivial even before `prior`
    // is consulted.
    let tick_floor = clock.unix_now().as_secs();
    let updated_at = LogicalTimestamp::dominating(
        tick_floor,
        writer_node.clone(),
        Some(&prior_row.updated_at),
    );
    let row = build_alloc_status_row(
        alloc_id.clone(),
        prior_row.workload_id.clone(),
        prior_row.node_id.clone(),
        AllocState::Terminated,
        updated_at,
        Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed }),
        None,
        None,
        None,
        prior_row.kind,
        prior_row.started_at,
        None,
        Some(&prior_row),
    );
    obs.write(ObservationRow::AllocStatus(Box::new(row.clone()))).await?;

    // The four evaluations the exit observer submits per exit
    // (`worker/exit_observer.rs:234`, `:254`, `:295`, `:318-320`).
    // Reclamation deliberately bypasses the exit observer (no
    // `ExitEvent`, no watcher — this executor authors its row directly),
    // so it is responsible for the same fan-out. All four are
    // unconditional, mirroring the observer's own unconditional shape —
    // a spurious enqueue costs exactly one empty reconcile.
    if let Ok(target) = TargetResource::new(&format!("workload/{}", row.workload_id)) {
        let mut guard = broker.lock();
        guard.submit(Evaluation {
            reconciler: evaluation_targets::workload_lifecycle(),
            target: target.clone(),
        });
        guard.submit(Evaluation {
            reconciler: evaluation_targets::backend_discovery_bridge(),
            target: target.clone(),
        });
        guard.submit(Evaluation {
            reconciler: evaluation_targets::service_lifecycle(),
            target: target.clone(),
        });
        guard.submit(Evaluation {
            reconciler: evaluation_targets::svid_lifecycle(),
            target,
        });
    }

    Ok(())
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
    use std::time::Duration;

    use overdrive_core::UnixInstant;
    use overdrive_core::aggregate::WorkloadKind;
    use overdrive_core::id::WorkloadId;
    use overdrive_core::traits::observation_store::AllocStatusRow;
    use overdrive_sim::adapters::clock::SimClock;
    use overdrive_sim::adapters::observation_store::SimObservationStore;
    use overdrive_sim::adapters::vm_host_state::SimVmHostState;
    use tracing_subscriber::layer::SubscriberExt as _;

    use super::*;

    // -----------------------------------------------------------------
    // D2 (02-03 review) — a minimal in-memory tracing-event capture.
    // This crate carries no `tracing-test` dependency; a custom
    // `tracing_subscriber::Layer` recording each event's
    // `metadata().name()` (the `name: "..."` macro override — e.g.
    // "vm.reclamation.refused" — confirmed against the `tracing` 0.1.44
    // `event!` macro expansion to set `Metadata::name` directly) plus its
    // fields is enough to assert S-VM-79's event clause without adding a
    // new dependency.
    // -----------------------------------------------------------------

    #[derive(Debug, Clone)]
    struct CapturedEvent {
        name: String,
        fields: std::collections::BTreeMap<String, String>,
    }

    #[derive(Clone, Default)]
    struct EventCapture(std::sync::Arc<std::sync::Mutex<Vec<CapturedEvent>>>);

    impl EventCapture {
        fn events(&self) -> Vec<CapturedEvent> {
            self.0.lock().expect("capture lock").clone()
        }
    }

    struct FieldRecorder<'a>(&'a mut std::collections::BTreeMap<String, String>);

    impl tracing::field::Visit for FieldRecorder<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for EventCapture {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut fields = std::collections::BTreeMap::new();
            event.record(&mut FieldRecorder(&mut fields));
            self.0
                .lock()
                .expect("capture lock")
                .push(CapturedEvent { name: event.metadata().name().to_owned(), fields });
        }
    }

    fn alloc(id: &str) -> AllocationId {
        AllocationId::new(id).expect("valid AllocationId")
    }

    fn workload(id: &str) -> WorkloadId {
        WorkloadId::new(id).expect("valid WorkloadId")
    }

    fn node(id: &str) -> NodeId {
        NodeId::new(id).expect("valid NodeId")
    }

    /// A `Running` `AllocStatusRow` for `alloc`/`workload`/`node` — the
    /// non-terminal fixture the AUTHORISED-branch tests seed as the prior
    /// row `execute_reclaim_allocation`'s guard must let through.
    fn running_row(alloc: &AllocationId, workload: &WorkloadId, node: &NodeId) -> AllocStatusRow {
        AllocStatusRow {
            alloc_id: alloc.clone(),
            workload_id: workload.clone(),
            node_id: node.clone(),
            state: AllocState::Running,
            updated_at: LogicalTimestamp { counter: 7, writer: node.clone() },
            reason: Some(TransitionReason::Started),
            detail: None,
            terminal: None,
            stderr_tail: None,
            kind: WorkloadKind::Job,
            listeners: Vec::new(),
            started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1000))),
            workload_addr: None,
            last_terminated: None,
            restart_count: 0,
        }
    }

    /// A `Terminated` `AllocStatusRow` — the fixture the write-time
    /// terminality guard's refusal tests seed as the prior row. Distinct
    /// `StoppedBy::Operator` disposition so a refusal is unambiguously
    /// distinguishable from a `PlatformReclaimed` write in the
    /// byte-unchanged assertion.
    fn terminated_row(alloc: &AllocationId, workload: &WorkloadId, node: &NodeId) -> AllocStatusRow {
        AllocStatusRow {
            alloc_id: alloc.clone(),
            workload_id: workload.clone(),
            node_id: node.clone(),
            state: AllocState::Terminated,
            updated_at: LogicalTimestamp { counter: 9, writer: node.clone() },
            reason: Some(TransitionReason::Stopped { by: StoppedBy::Operator }),
            detail: None,
            terminal: None,
            stderr_tail: None,
            kind: WorkloadKind::Job,
            listeners: Vec::new(),
            started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1000))),
            workload_addr: None,
            last_terminated: None,
            restart_count: 0,
        }
    }

    fn seeded_host(a: &AllocationId) -> SimVmHostState {
        let host = SimVmHostState::new();
        host.set_scope(a.clone(), BTreeSet::from([4242]));
        host.set_run_dir(a.clone());
        host.set_clone(a.clone(), std::path::PathBuf::from("/staging/vm-reclaim.img"));
        host
    }

    // -----------------------------------------------------------------
    // execute_reclaim_allocation — AUTHORISED branch (S-VM-81 content:
    // kill -> discard -> write terminal row -> submit the four
    // evaluations, mirroring the exit observer's own fan-out).
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn execute_reclaim_allocation_authorised_kills_discards_writes_and_submits_four_evaluations()
     {
        let a = alloc("vm-reclaim-0");
        let w = workload("vm-reclaim-workload");
        let n = node("vm-reclaim-node");
        let host = seeded_host(&a);
        let obs = SimObservationStore::single_peer(n.clone(), 0);
        let prior = running_row(&a, &w, &n);
        obs.write(ObservationRow::AllocStatus(Box::new(prior.clone())))
            .await
            .expect("seed the prior Running row");
        let clock = SimClock::new();
        let broker = parking_lot::Mutex::new(EvaluationBroker::new());

        execute_reclaim_allocation(&a, &host, &obs, &clock, &n, &broker)
            .await
            .expect("authorised reclaim succeeds");

        // Host artifacts gone.
        let observed = host.observe().await.expect("observe succeeds");
        assert!(!observed.scopes.contains_key(&a), "kill_scope must remove the scope");
        assert!(!observed.run_dirs.contains(&a), "discard_artifacts must remove the run dir");
        assert!(!observed.clones.contains_key(&a), "discard_artifacts must remove the clone");

        // Terminal row written with StoppedBy::PlatformReclaimed.
        let row = obs
            .alloc_status_row(&a)
            .await
            .expect("read succeeds")
            .expect("a row exists after the reclaim write");
        assert_eq!(row.state, AllocState::Terminated);
        assert!(
            matches!(
                row.reason,
                Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed })
            ),
            "expected Stopped {{ by: PlatformReclaimed }}, got {:?}",
            row.reason
        );
        assert_eq!(row.terminal, None, "brief.md §105 pins terminal: None on this write");
        assert!(
            overdrive_core::transition_reason::is_platform_reclaimed(&row),
            "the written row must classify as a Platform Reclamation"
        );

        // D5 (02-03 review) — complement-equality: fields OUTSIDE the
        // declared delta (state, reason, terminal, updated_at) must be
        // carried forward from the prior row byte-for-byte, not
        // silently dropped or defaulted at the `build_alloc_status_row`
        // call site. `workload_id` / `node_id` / `kind` / `started_at`
        // are the fields whose fixture value is non-trivial enough
        // (a real workload id, a real node id, `WorkloadKind::Job`, a
        // real `Some(UnixInstant)`) that a mutation dropping the
        // carry-forward would be caught here.
        assert_eq!(
            row.workload_id, prior.workload_id,
            "workload_id must be carried forward from the prior row"
        );
        assert_eq!(row.node_id, prior.node_id, "node_id must be carried forward from the prior row");
        assert_eq!(row.kind, prior.kind, "kind must be carried forward from the prior row");
        assert_eq!(
            row.started_at, prior.started_at,
            "started_at must be carried forward from the prior row"
        );

        // The four evaluations, all targeting workload/<id>.
        let pending = broker.lock().drain_pending();
        let expected_target = TargetResource::new(&format!("workload/{w}")).expect("valid target");
        let mut names: Vec<String> =
            pending.iter().map(|e| e.reconciler.as_str().to_owned()).collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "backend-discovery-bridge".to_owned(),
                "service-lifecycle".to_owned(),
                "svid-lifecycle".to_owned(),
                "workload-lifecycle".to_owned(),
            ],
            "execute_reclaim_allocation must submit exactly the four evaluations the exit \
             observer submits per exit (workload_lifecycle, backend_discovery_bridge, \
             service_lifecycle, svid_lifecycle — the fourth is load-bearing per ADR-0083 §D7: \
             its omission leaves the node holding the dead allocation's leaf private key)"
        );
        assert!(
            pending.iter().all(|e| e.target == expected_target),
            "every evaluation must target the reclaimed allocation's own workload, got {pending:?}"
        );
    }

    // -----------------------------------------------------------------
    // execute_reclaim_allocation — the write-time terminality guard
    // (S-VM-79 content). A refusal is a TOTAL no-op: no kill, no
    // discard, no row write, Ok(()) — never a degradation to
    // DiscardStrandedArtifacts.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn execute_reclaim_allocation_refuses_total_no_op_when_row_already_terminal() {
        let a = alloc("vm-reclaim-refuse-0");
        let w = workload("vm-reclaim-refuse-workload");
        let n = node("vm-reclaim-refuse-node");
        let host = seeded_host(&a);
        let obs = SimObservationStore::single_peer(n.clone(), 0);
        let seeded = terminated_row(&a, &w, &n);
        obs.write(ObservationRow::AllocStatus(Box::new(seeded.clone())))
            .await
            .expect("seed the prior Terminated row");
        let clock = SimClock::new();
        let broker = parking_lot::Mutex::new(EvaluationBroker::new());

        // D2 (02-03 review) — capture tracing events for the duration of
        // the call under test so the `vm.reclamation.refused` event
        // (S-VM-79's event clause) can be asserted on directly, rather
        // than assumed from the code path taken.
        let capture = EventCapture::default();
        let subscriber = tracing_subscriber::Registry::default().with(capture.clone());
        let guard = tracing::subscriber::set_default(subscriber);

        execute_reclaim_allocation(&a, &host, &obs, &clock, &n, &broker)
            .await
            .expect("a refusal is Ok(()), never an error");

        drop(guard);
        let refused = capture
            .events()
            .into_iter()
            .find(|e| e.name == "vm.reclamation.refused")
            .expect("the vm.reclamation.refused event must fire for a terminal-row refusal");
        assert_eq!(
            refused.fields.get("alloc_id").map(String::as_str),
            Some(a.as_str()),
            "the event must carry the refused allocation's id, got {:?}",
            refused.fields
        );
        assert!(
            refused.fields.get("observed_state").is_some_and(|s| s.contains("Terminated")),
            "the event must carry the observed (terminal) state, got {:?}",
            refused.fields
        );

        // No kill, no discard: every host artifact is exactly as seeded.
        let observed = host.observe().await.expect("observe succeeds");
        assert!(observed.scopes.contains_key(&a), "a refusal must NOT kill the scope");
        assert!(observed.run_dirs.contains(&a), "a refusal must NOT discard the run dir");
        assert!(observed.clones.contains_key(&a), "a refusal must NOT discard the clone");

        // No row write: byte-unchanged (every field, not just `state`).
        let row = obs
            .alloc_status_row(&a)
            .await
            .expect("read succeeds")
            .expect("the seeded row still exists");
        assert_eq!(row, seeded, "a refused reclaim must leave the row byte-unchanged");

        // No evaluation submitted.
        assert!(
            broker.lock().drain_pending().is_empty(),
            "a refusal must submit no evaluation at all"
        );
    }

    #[tokio::test]
    async fn execute_reclaim_allocation_refuses_total_no_op_when_row_absent() {
        let a = alloc("vm-reclaim-refuse-absent-0");
        let n = node("vm-reclaim-refuse-absent-node");
        // Nothing seeded on any host surface, no row written.
        let host = SimVmHostState::new();
        let obs = SimObservationStore::single_peer(n.clone(), 0);
        let clock = SimClock::new();
        let broker = parking_lot::Mutex::new(EvaluationBroker::new());

        execute_reclaim_allocation(&a, &host, &obs, &clock, &n, &broker)
            .await
            .expect("a refusal is Ok(()), never an error");

        assert!(
            obs.alloc_status_row(&a).await.expect("read succeeds").is_none(),
            "no row must be written for an allocation with no prior row"
        );
        assert!(
            broker.lock().drain_pending().is_empty(),
            "a refusal must submit no evaluation at all"
        );
    }

    /// S-VM-79 manual mutation-kill proof (per `nw-mutation-test` /
    /// `.claude/rules/testing.md` § "Mandatory targets" — the write-time
    /// guard is a mandatory mutation target). Flipping the guard's
    /// polarity (`is_terminal()` -> `!is_terminal()`, i.e. AUTHORISED and
    /// REFUSED swap) must turn
    /// `execute_reclaim_allocation_refuses_total_no_op_when_row_already_terminal`
    /// red. This test asserts the SAME property the manual flip exercises
    /// — that a terminal row and a non-terminal row are NOT
    /// interchangeable through this executor — as a standing regression
    /// guard alongside the manual proof recorded in the step's commit
    /// history.
    #[tokio::test]
    async fn execute_reclaim_allocation_terminal_and_non_terminal_rows_are_not_interchangeable() {
        let n = node("vm-reclaim-guard-node");
        let w = workload("vm-reclaim-guard-workload");

        let running_alloc = alloc("vm-reclaim-guard-running");
        let running_host = seeded_host(&running_alloc);
        let running_obs = SimObservationStore::single_peer(n.clone(), 0);
        running_obs
            .write(ObservationRow::AllocStatus(Box::new(running_row(&running_alloc, &w, &n))))
            .await
            .expect("seed Running row");
        let clock = SimClock::new();
        let running_broker = parking_lot::Mutex::new(EvaluationBroker::new());
        execute_reclaim_allocation(&running_alloc, &running_host, &running_obs, &clock, &n, &running_broker)
            .await
            .expect("ok");
        assert_eq!(
            running_broker.lock().drain_pending().len(),
            4,
            "a non-terminal row is AUTHORISED and must submit evaluations"
        );

        let terminal_alloc = alloc("vm-reclaim-guard-terminal");
        let terminal_host = seeded_host(&terminal_alloc);
        let terminal_obs = SimObservationStore::single_peer(n.clone(), 0);
        terminal_obs
            .write(ObservationRow::AllocStatus(Box::new(terminated_row(&terminal_alloc, &w, &n))))
            .await
            .expect("seed Terminated row");
        let terminal_broker = parking_lot::Mutex::new(EvaluationBroker::new());
        execute_reclaim_allocation(
            &terminal_alloc,
            &terminal_host,
            &terminal_obs,
            &clock,
            &n,
            &terminal_broker,
        )
        .await
        .expect("ok");
        assert_eq!(
            terminal_broker.lock().drain_pending().len(),
            0,
            "a terminal row is REFUSED and must submit nothing"
        );
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

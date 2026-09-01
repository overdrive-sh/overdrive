//! Acceptance — dial-by-name-responder step 02-02 review-resolution D1.
//!
//! Pins the `workload_addr` forward-carry branch in the `action_shim`
//! `FinalizeFailed` arm (`crates/overdrive-control-plane/src/action_shim/mod.rs`
//! `~:1089`):
//!
//! ```ignore
//! if is_stable { prior_workload_addr } else { None }
//! ```
//!
//! This is the load-bearing fix of step 02-02 — a `Stable` (still-Running)
//! terminal MUST keep the alloc's per-instance backend address so the
//! `BackendDiscoveryBridge` advertises a reachable addr instead of silently
//! reverting to its `host_ipv4` fallback (the dial-by-name walking-skeleton
//! backend-drop; GH #248). A genuine terminal (`Failed` / `Completed` /
//! `BackoffExhausted`) is a dead alloc, not a live backend, so it drops to
//! `None`.
//!
//! # Why this test exists (the mutation gap)
//!
//! The behaviour was defended ONLY indirectly at Tier-3 (the `is_root()`-gated,
//! Lima-only, `integration-tests`-gated S-DBN-WS walking skeleton). Every
//! pre-existing default-lane prior-row fixture carries `workload_addr: None`,
//! so both arms of `if is_stable { prior_workload_addr } else { None }` collapse
//! to `None` and ALL FOUR branch mutants survive: swap-arms, always-`None`,
//! always-`prior`, and the `matches!` `==`→`!=` on the `Stable` discriminant.
//! This test seeds a `workload_addr: Some(addr)` prior row so the two arms
//! diverge and every mutant flips it RED — independent of the Lima environment.
//!
//! # PORT-TO-PORT litmus
//!
//! Drives the production driving port `action_shim::dispatch` and asserts on the
//! driven-port boundary (the `AllocStatusRow` written to the
//! `SimObservationStore`) — never on internal state. Mutating the forward-carry
//! branch in any of the four ways above turns this RED.
//!
//! Shape mirrors `release_service_vip_dispatch.rs` (the sibling ungated
//! default-lane action-shim dispatch acceptance test): real `dispatch`, sim
//! adapters for every orthogonal port, `mtls_worker: None` + a fresh
//! `NetSlotAllocator` (the genuine-terminal arm's teardown is a clean no-op when
//! the worker is absent — see `teardown_and_release_netns`). No root, no Lima,
//! no `integration-tests` feature: runs under bare `cargo nextest`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use proptest::prelude::*;

use overdrive_control_plane::action_shim::dispatch;
use overdrive_control_plane::veth_provisioner::NetSlotAllocator;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, Resources,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore,
};
use overdrive_core::transition_reason::{ProbeWitness, TerminalCondition, TransitionReason};
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

/// Inert driver — the `FinalizeFailed` arm never calls the driver (it reads the
/// prior obs row and writes a successor row), so every method is unreachable
/// under this test.
struct InertDriver;

#[async_trait::async_trait]
impl Driver for InertDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Exec
    }

    async fn start(&self, _spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        Err(DriverError::StartRejected {
            failure: overdrive_core::traits::driver::DriverStartFailure {
                class: overdrive_core::traits::driver::DriverStartClass::Unclassified {
                    driver: DriverType::Exec,
                },
                detail: "InertDriver: start() not expected on FinalizeFailed dispatch".to_owned(),
            },
        })
    }

    async fn stop(&self, _handle: &AllocationHandle) -> Result<(), DriverError> {
        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        Err(DriverError::NotFound { alloc: handle.alloc.clone() })
    }

    async fn resize(
        &self,
        _handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        Ok(())
    }
}

/// The opt-out `Stable` witness the `ServiceLifecycleReconciler` emits for an
/// empty-startup-probes Service — mirrors the real emission shape so the
/// dispatched terminal matches production.
fn stable_terminal() -> TerminalCondition {
    TerminalCondition::Stable {
        settled_in_ms: 0,
        witness: ProbeWitness {
            probe_idx: 0,
            role: "startup".to_owned(),
            mechanic_summary: "none (opted out)".to_owned(),
            inferred: false,
        },
    }
}

/// Seed a `Running` prior `AllocStatusRow` carrying `workload_addr: Some(addr)`
/// — the precondition the forward-carry branch reads. `counter: 0` so the
/// `FinalizeFailed` write (counter `tick.tick + 1` = 1, same writer) strictly
/// dominates under LWW and the successor row is the one the assertions read.
async fn seed_running_row_with_addr(
    obs: &dyn ObservationStore,
    alloc: &AllocationId,
    workload: &WorkloadId,
    node: &NodeId,
    addr: Ipv4Addr,
) {
    let row = AllocStatusRow {
        alloc_id: alloc.clone(),
        workload_id: workload.clone(),
        node_id: node.clone(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 0, writer: node.clone() },
        reason: Some(TransitionReason::Started),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: Some(addr),
        last_terminated: None,
        restart_count: 0,
    };
    obs.write_alloc_lifecycle(
        row,
        overdrive_core::traits::observation_store::TransitionSource::Reconciler,
    )
    .await
    .expect("seed prior Running alloc row carrying workload_addr");
}

/// Drive ONE `FinalizeFailed { terminal }` through the production
/// `action_shim::dispatch` against a `Running` prior row that owns
/// `workload_addr: Some(seed_addr)`, and return the successor row's
/// `(state, workload_addr)` — the two port-exposed slots the forward-carry
/// branch governs.
async fn finalize_and_read_successor(
    terminal: TerminalCondition,
    seed_addr: Ipv4Addr,
) -> (AllocState, Option<Ipv4Addr>) {
    let tmp = TempDir::new().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let store: Arc<dyn IntentStore> =
        Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    let alloc = AllocationId::new("ffwc-alloc").expect("valid alloc id");
    let workload = WorkloadId::new("ffwc-svc").expect("valid workload id");
    let node = NodeId::new("node-001").expect("valid node id");

    seed_running_row_with_addr(obs.as_ref(), &alloc, &workload, &node, seed_addr).await;

    // ---- Orthogonal ports the FinalizeFailed arm does not exercise: sim
    // shapes. `mtls_worker: None` + a fresh NetSlotAllocator → the genuine-
    // terminal arm's `teardown_and_release_netns` is a clean no-op (it returns
    // Ok immediately when the worker is absent), so the test stays default-lane
    // (no netns, no root, no Lima). Mirrors release_service_vip_dispatch.rs.
    let dataplane: Arc<dyn overdrive_core::traits::dataplane::Dataplane> =
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new());
    let driver: Arc<dyn Driver> = Arc::new(InertDriver);
    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut r = overdrive_core::traits::driver::DriverRegistry::new();
        r.insert(Arc::clone(&driver));
        Arc::new(r)
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    let (lifecycle_tx, _lifecycle_rx) = tokio::sync::broadcast::channel(16);
    let writer_node = NodeId::new("writer-1").expect("NodeId");
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        Arc::clone(&store),
    )));
    let net_slot_allocator = NetSlotAllocator::new();
    let test_broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());

    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    dispatch(
        vec![Action::FinalizeFailed { alloc_id: alloc.clone(), terminal: Some(terminal) }],
        drivers.as_ref(),
        &alloc_drivers,
        obs.as_ref(),
        dataplane.as_ref(),
        &overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        )),
        &overdrive_sim::adapters::clock::SimClock::new(),
        &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
        &lifecycle_tx,
        &tick,
        &writer_node,
        Arc::clone(&allocator),
        &test_broker,
        None,
        // No mTLS worker — the genuine-terminal teardown seam is a no-op.
        None,
        &net_slot_allocator,
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
    )
    .await
    .expect("FinalizeFailed dispatch must succeed (records a successor row, never an Err)");

    let rows = obs.alloc_status_rows().await.expect("read alloc rows");
    let successor = rows
        .into_iter()
        .filter(|r| r.alloc_id == alloc)
        .max_by_key(|r| r.updated_at.counter)
        .expect("a successor AllocStatusRow must exist after FinalizeFailed");
    (successor.state, successor.workload_addr)
}

proptest! {
    // Default-lane property test. PROPTEST_CASES (1024 in CI) explores the IPv4
    // address space; the invariant holds for every seeded address. `deadline:
    // None` because each case boots a tempdir-backed LocalIntentStore + sim
    // dispatch (~ms), well under the default-lane budget for the case count.
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// PROPERTY (Stable preserves): for ANY IPv4 `addr`, a `FinalizeFailed
    /// { Stable }` against a `Running` prior row owning `workload_addr:
    /// Some(addr)` writes a successor row that STAYS `Running` AND KEEPS
    /// `Some(addr)`.
    ///
    /// Kills: always-`None` (would drop addr), swap-arms (would drop addr),
    /// and `matches!` `==`→`!=` (would treat Stable as genuine → state Failed
    /// AND addr None).
    #[test]
    fn finalize_failed_stable_keeps_the_running_alloc_workload_addr(
        a in any::<u8>(), b in any::<u8>(), c in any::<u8>(), d in any::<u8>(),
    ) {
        let addr = Ipv4Addr::new(a, b, c, d);
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let (state, carried) = rt.block_on(finalize_and_read_successor(stable_terminal(), addr));
        prop_assert_eq!(
            state,
            AllocState::Running,
            "a Stable FinalizeFailed is a success claim — the row must stay Running",
        );
        prop_assert_eq!(
            carried,
            Some(addr),
            "GH #248: a Stable FinalizeFailed must FORWARD-CARRY the prior row's \
             workload_addr (a live backend keeps its per-instance address), got {:?}",
            carried,
        );
    }

    /// PROPERTY (genuine terminal drops `workload_addr`; row `state` follows
    /// the terminal claim's own success/failure identity): for ANY IPv4
    /// `addr` AND ANY non-`Stable` terminal (`Failed` / `Completed` /
    /// `BackoffExhausted`), a `FinalizeFailed` against the SAME `Running`
    /// `Some(addr)` prior row drops `workload_addr` to `None` — a dead alloc
    /// is not a live backend, whether the terminal is success or failure.
    /// The row's `state` mirrors the terminal's own variant (per
    /// `TerminalCondition::Completed` / `::Failed`'s docs: "branch on the
    /// variant, never on exit_code != 0"): `Completed` (a Job-kind clean
    /// exit — the success case) lands `Terminated`; `Failed` /
    /// `BackoffExhausted` land `Failed`.
    ///
    /// **Corrected** (microvm-driver-cloud-hypervisor step 01-08 review
    /// remediation, S-VM-01): this property previously asserted
    /// `state == AllocState::Failed` uniformly, including for `Completed`.
    /// That was wrong — `TerminalCondition::Completed`'s own doc names exit
    /// code 0 "the canonical success", and the production
    /// `action_shim::dispatch` `FinalizeFailed` arm forcing `Completed` to
    /// `AllocState::Failed` (while forward-carrying the prior row's
    /// `reason: Stopped { by: Process }`, written by
    /// `exit_observer::classify()`'s `CleanExit` branch) produced a
    /// `Failed` + `Stopped { by: Process }` row `classify()` itself never
    /// constructs — the exact S-VM-01 walking-skeleton finding (a VM guest
    /// that exited 0 was reported Failed to the operator). This was already
    /// the ONLY place asserting that pairing: `streaming.rs`'s
    /// `workload_event_from_terminal` independently projects
    /// `Completed -> JobSubmitEvent::Succeeded`, `streaming_submit.rs`
    /// hand-builds `Terminated` + `Completed` fixtures, and
    /// `job_kind_streaming.rs`'s S-02-01/S-02-04 assert `Succeeded` on the
    /// streaming path for a zero-exit Job — this property's `state`
    /// assertion had not caught up.
    ///
    /// Kills: always-`prior` (would keep `Some(addr)` on a genuine terminal),
    /// swap-arms (would keep addr on the genuine arm), and a regression of
    /// the `Completed -> Terminated` mapping back to unconditional `Failed`.
    #[test]
    fn finalize_failed_genuine_terminal_drops_workload_addr(
        a in any::<u8>(), b in any::<u8>(), c in any::<u8>(), d in any::<u8>(),
        terminal in prop_oneof![
            any::<i32>().prop_map(|code| TerminalCondition::Failed { exit_code: Some(code) }),
            Just(TerminalCondition::Failed { exit_code: None }),
            any::<i32>().prop_map(|code| TerminalCondition::Completed { exit_code: code }),
            any::<u32>().prop_map(|attempts| TerminalCondition::BackoffExhausted { attempts }),
        ],
    ) {
        let addr = Ipv4Addr::new(a, b, c, d);
        let expected_state = if matches!(terminal, TerminalCondition::Completed { .. }) {
            AllocState::Terminated
        } else {
            AllocState::Failed
        };
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let (state, carried) = rt.block_on(finalize_and_read_successor(terminal, addr));
        prop_assert_eq!(
            state,
            expected_state,
            "Completed (success) must land Terminated; Failed/BackoffExhausted (genuine \
             failure) must land Failed",
        );
        prop_assert_eq!(
            carried,
            None,
            "a genuine terminal is a dead alloc — workload_addr must drop to None, got {:?}",
            carried,
        );
    }
}

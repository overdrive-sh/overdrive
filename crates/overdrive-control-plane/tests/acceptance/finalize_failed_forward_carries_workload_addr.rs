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
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
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
            reason: "InertDriver: start() not expected on FinalizeFailed dispatch".to_owned(),
            driver: DriverType::Exec,
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
    };
    obs.write(ObservationRow::AllocStatus(Box::new(row)))
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
        driver.as_ref(),
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

    /// PROPERTY (genuine terminal drops): for ANY IPv4 `addr` AND ANY genuine
    /// terminal (`Failed` / `Completed` / `BackoffExhausted`), a `FinalizeFailed`
    /// against the SAME `Running` `Some(addr)` prior row writes a successor row
    /// that lands `Failed` AND drops `workload_addr` to `None` — a dead alloc is
    /// not a live backend.
    ///
    /// Kills: always-`prior` (would keep `Some(addr)` on a genuine terminal) and
    /// swap-arms (would keep addr on the genuine arm).
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
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let (state, carried) = rt.block_on(finalize_and_read_successor(terminal, addr));
        prop_assert_eq!(
            state,
            AllocState::Failed,
            "a genuine FinalizeFailed terminal must land the row Failed",
        );
        prop_assert_eq!(
            carried,
            None,
            "a genuine terminal is a dead alloc — workload_addr must drop to None, got {:?}",
            carried,
        );
    }
}

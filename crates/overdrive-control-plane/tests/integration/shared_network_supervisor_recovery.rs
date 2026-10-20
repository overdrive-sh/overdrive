//! GH #295 S-ND295-29B (re-derived for QHOLD) — the composed server's runtime
//! shared-network supervisor on a healthy node, through its required ports.
//!
//! # What moved, and why
//!
//! The recovery cells of this proof (one recovery per damaged component,
//! counted quiescence, the bounded fail-stop, the restart) need the owner to
//! hand the supervisor a `QuiescenceHold`. A hold is constructible only by
//! the production owner (QHOLD, ADR-0169), so the out-of-crate
//! `SimSharedGuestNetworkOwner` this file composes cannot take one (DISTILL
//! blocker: no pinned way for a sim outside the owner crate to obtain a
//! hold). Those cells are source-local in `overdrive-control-plane` over the
//! production owner on `SimGuestFlowIo` — `shared_network_recovery_acceptance`
//! (P6-S1..S6, C2) and the Quint `quiescence` conformance driver
//! (`quint_quiescence`, C1). The TAP-era cells (bridge, TCX link, endpoint and
//! counter maps, bpffs pins, the bridge guard, per-TAP quiescence) were deleted
//! with that mechanism.
//!
//! What stays here is the supervisor's healthy-node contract, which needs no
//! hold: on a healthy node it audits the shared owner once per one-second
//! period (ADR-0124, D-295-R13), never enters recovery, never quiesces,
//! repairs or requests shutdown, and the EXEC gate stays Open.
//!
//! # Driving port and injected ports
//!
//! `run_server_with_obs_and_driver(config, boot, driver, shared_guest_network,
//! guest_network_exec, vm_cgroups)` (`boot` from `node_adapters::boot_adapters`,
//! GH #295 NO-OVERRIDE-SEAMS) with a `SimDriver`, the
//! `SimSharedGuestNetworkOwner` (its call log is the audit oracle),
//! `SimMtlsIntercept`, `SimGuestDnsFactory` and a `SimClock`. The test keeps
//! the EXEC gate and supervisor of the wiring it hands to the boot and reads
//! them directly; the clock advances in fixed steps.
//!
//! # Oracle
//!
//! ADR-0124's cadence, measured on the injected clock: the shared owner's
//! `audit_shared` once per 1 s period; no `ForwardingQuiesce`,
//! `ForwardingRestore` or `ForwardingActivate` call; no recovery in
//! `recovery_progress()`; the gate's `claim_release()` resolves at once at
//! every step; no `ServeShutdownRequest`.

#![allow(
    clippy::unchecked_time_subtraction,
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt as _;
use overdrive_control_plane::guest_network::GuestNetworkOperation;
use overdrive_control_plane::{ServerHandle, run_server_with_obs_and_driver};
use overdrive_core::guest_network::{
    GuestNetworkExecGate, GuestNetworkExecSupervisor, GuestNetworkExecWiring,
};
use overdrive_core::id::NodeId;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::SimCgroupFs;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use tempfile::TempDir;

/// ADR-0124's audit period.
const AUDIT_PERIOD: Duration = Duration::from_secs(1);
/// The test's clock step.
const STEP: Duration = Duration::from_millis(10);
/// The observed window: ten audit periods.
const WINDOW: Duration = Duration::from_secs(10);

struct Node {
    clock: Arc<SimClock>,
    owner: Arc<SimSharedGuestNetworkOwner>,
    gate: Arc<GuestNetworkExecGate>,
    supervisor: Arc<GuestNetworkExecSupervisor>,
    handle: ServerHandle,
    _tmp: TempDir,
}

impl Node {
    async fn boot() -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let data_dir = tmp.path().join("data");
        let operator_config_dir = tmp.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        std::fs::create_dir_all(&operator_config_dir).expect("create config dir");
        let clock = Arc::new(SimClock::new());
        let config = super::node_adapters::test_config(data_dir, operator_config_dir);
        let mut boot = super::node_adapters::boot_adapters(&config);
        boot.clock = clock.clone();
        let owner = Arc::new(SimSharedGuestNetworkOwner::default());
        let wiring = GuestNetworkExecWiring::new(clock.clone());
        let gate = wiring.gate();
        let supervisor = wiring.supervisor();
        let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
            NodeId::new("nd295-supervisor-healthy").expect("node id"),
            0,
        ));
        let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
        let handle = run_server_with_obs_and_driver(
            config,
            overdrive_control_plane::BootAdapters { observation: obs, ..boot },
            driver,
            Arc::clone(&owner) as _,
            wiring,
            overdrive_worker::cgroup_manager::CgroupManager::new(
                std::path::PathBuf::from("/sys/fs/cgroup"),
                Arc::new(SimCgroupFs::new()),
            ),
        )
        .await
        .expect("a healthy node boots");
        Self { clock, owner, gate, supervisor, handle, _tmp: tmp }
    }

    /// One clock step, then let every ready task run.
    async fn step(&self) {
        self.clock.tick(STEP);
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }

    fn audits(&self) -> usize {
        self.owner
            .calls()
            .iter()
            .filter(|call| **call == GuestNetworkOperation::SharedAudit)
            .count()
    }

    fn admission_open(&self) -> bool {
        self.gate.claim_release().now_or_never().flatten().is_some()
    }
}

/// S-ND295-29B (C0) — A healthy owner keeps admission open: no recovery, no
/// quiescence, no restore, no activation, no shutdown request.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER attachment-cut step (09-01 rework)"]
async fn a_healthy_owner_keeps_admission_open_without_recovery_effects() {
    let mut node = Node::boot().await;
    let boot_calls = node.owner.calls().len();
    let mut elapsed = Duration::ZERO;
    while elapsed < WINDOW {
        node.step().await;
        elapsed += STEP;
        assert!(node.supervisor.recovery_progress().is_empty(), "no recovery at {elapsed:?}");
        assert!(node.admission_open(), "EXEC admission open at {elapsed:?}");
        assert!(
            node.handle.shutdown_requested().now_or_never().is_none(),
            "no shutdown request at {elapsed:?}"
        );
    }
    let calls = node.owner.calls()[boot_calls..].to_vec();
    for forbidden in [
        GuestNetworkOperation::ForwardingQuiesce,
        GuestNetworkOperation::ForwardingRestore,
        GuestNetworkOperation::ForwarderOpen,
    ] {
        assert!(
            !calls.contains(&forbidden),
            "after boot a healthy owner receives no {forbidden:?} (no repair): {calls:?}"
        );
    }
    node.handle.shutdown(Duration::from_secs(1)).await.expect("graceful shutdown");
}

/// S-ND295-29B (C1) — On a healthy node the supervisor audits the shared
/// owner once per one-second period.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER attachment-cut step (09-01 rework)"]
async fn a_healthy_node_audits_the_shared_owner_every_second() {
    let node = Node::boot().await;
    let at_boot = node.audits();
    let mut audit_times = Vec::new();
    let mut seen = at_boot;
    let mut elapsed = Duration::ZERO;
    while elapsed < WINDOW {
        node.step().await;
        elapsed += STEP;
        let now = node.audits();
        for _ in seen..now {
            audit_times.push(elapsed);
        }
        seen = now;
    }
    let periods = usize::try_from(WINDOW.as_millis() / AUDIT_PERIOD.as_millis()).expect("periods");
    assert!(
        audit_times.len() == periods || audit_times.len() == periods + 1,
        "one audit per {AUDIT_PERIOD:?} over {WINDOW:?}: {audit_times:?}"
    );
    for pair in audit_times.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            gap + STEP >= AUDIT_PERIOD && gap <= AUDIT_PERIOD + STEP,
            "consecutive audits one period apart (within one step): {audit_times:?}"
        );
    }
    node.handle.shutdown(Duration::from_secs(1)).await.expect("graceful shutdown");
}

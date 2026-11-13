//! One shared VM-typed [`Driver`] and one shared network provisioner stub for the
//! control-plane acceptance scenarios of feature
//! `service-allocation-readiness-and-ending`.
//!
//! [`HookedDriver`] delegates process start, stop, status and the exit gate to a
//! `SimDriver`, forwards the probe-supervision hooks (`on_alloc_running`,
//! `on_alloc_stable`, `on_alloc_terminal`) to a real `ProbeRunner` as `VmDriver`
//! does, records the order of the lifecycle calls, owns its exit channel (so a
//! scenario can deliver a guest exit, including OOM facts, through the production
//! exit observer) and can model a slow boot, an exit raised while a stop is being
//! dispatched, and a guest that is already dead when the gate releases.

#![allow(
    dead_code,
    clippy::expect_used,
    reason = "a test composition asserts its own preconditions; each scenario file uses a subset"
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::guest_transport::GuestTransportAssignment;
use overdrive_core::id::AllocationId;
use overdrive_core::observation::ProbeRole;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, ExitEvent,
    Resources,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_worker::probe_runner::ProbeRunner;
use parking_lot::Mutex;
use tokio::sync::mpsc;

/// One recorded call on the driver's lifecycle surface, in call order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// `Driver::start`.
    Start(AllocationId),
    /// `Driver::stop`.
    Stop(AllocationId),
    /// `Driver::on_alloc_running`.
    Running(AllocationId),
    /// `Driver::release_for_exit_emission`, the Running-confirmed gate send.
    Release(AllocationId),
    /// `Driver::on_alloc_terminal`.
    Terminal(AllocationId),
}

/// The node's driver for the composed scenarios.
pub struct HookedDriver {
    sim: Arc<SimDriver>,
    runner: Arc<ProbeRunner>,
    clock: Arc<SimClock>,
    calls: Mutex<Vec<Call>>,
    exit_tx: mpsc::Sender<ExitEvent>,
    exit_rx: Mutex<Option<mpsc::Receiver<ExitEvent>>>,
    /// Supplied to `on_alloc_running` when the spec carries no transport.
    default_transport: Mutex<Option<GuestTransportAssignment>>,
    /// How long `start` takes on the injected clock.
    boot_delay: Mutex<Duration>,
    /// An exit the guest produces while the next `stop` is being dispatched.
    crash_during_stop: Mutex<Option<ExitEvent>>,
    /// Allocations whose guest is already dead when the gate releases: the
    /// exit is sent the instant the gate send happens, then the given number of
    /// yields follow.
    exits_at_release: Mutex<BTreeMap<AllocationId, u32>>,
}

impl HookedDriver {
    pub fn new(sim: Arc<SimDriver>, runner: Arc<ProbeRunner>, clock: Arc<SimClock>) -> Self {
        let (exit_tx, exit_rx) = mpsc::channel(32);
        Self {
            sim,
            runner,
            clock,
            calls: Mutex::new(Vec::new()),
            exit_tx,
            exit_rx: Mutex::new(Some(exit_rx)),
            default_transport: Mutex::new(None),
            boot_delay: Mutex::new(Duration::ZERO),
            crash_during_stop: Mutex::new(None),
            exits_at_release: Mutex::new(BTreeMap::new()),
        }
    }

    /// The transport `on_alloc_running` supplies to a spec that carries none.
    #[must_use]
    pub fn with_default_transport(self, transport: GuestTransportAssignment) -> Self {
        *self.default_transport.lock() = Some(transport);
        self
    }

    pub fn set_boot_delay(&self, boot: Duration) {
        *self.boot_delay.lock() = boot;
    }

    /// The guest produces `event` while the next `stop` is being dispatched.
    pub fn exit_during_next_stop(&self, event: ExitEvent) {
        *self.crash_during_stop.lock() = Some(event);
    }

    /// The guest of `alloc` is already dead when the gate releases.
    pub fn exit_at_release(&self, alloc: &AllocationId, yields: u32) {
        self.exits_at_release.lock().insert(alloc.clone(), yields);
    }

    /// The guest ends on its own: `event` reaches the production exit observer.
    pub async fn deliver_exit(&self, event: ExitEvent) {
        self.exit_tx.send(event).await.expect("the observer holds the exit receiver");
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().clone()
    }

    pub fn index_of(&self, call: &Call) -> Option<usize> {
        self.calls().iter().position(|c| c == call)
    }

    pub fn terminal_calls(&self) -> Vec<AllocationId> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Terminal(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    pub fn terminal_hook_calls(&self) -> usize {
        self.calls().iter().filter(|c| matches!(c, Call::Terminal(_))).count()
    }

    pub fn stop_calls_for(&self, alloc: &AllocationId) -> usize {
        self.calls().iter().filter(|c| **c == Call::Stop(alloc.clone())).count()
    }
}

async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

#[async_trait]
impl Driver for HookedDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Vm
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.calls.lock().push(Call::Start(spec.alloc.clone()));
        let delay = *self.boot_delay.lock();
        if !delay.is_zero() {
            self.clock.sleep(delay).await;
        }
        self.sim.start(spec).await
    }

    async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        self.calls.lock().push(Call::Release(handle.alloc.clone()));
        self.sim.release_for_exit_emission(handle).await;
        let yields = self.exits_at_release.lock().remove(&handle.alloc);
        if let Some(yields) = yields {
            let event = ExitEvent {
                alloc: handle.alloc.clone(),
                kind: overdrive_core::traits::driver::ExitKind::Crashed {
                    exit_code: Some(1),
                    signal: None,
                },
                intentional_stop: false,
                stderr_tail: None,
                oom: None,
            };
            self.exit_tx.send(event).await.expect("the observer's receiver is open");
            for _ in 0..yields {
                tokio::task::yield_now().await;
            }
        }
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.calls.lock().push(Call::Stop(handle.alloc.clone()));
        let racing = self.crash_during_stop.lock().take();
        if let Some(event) = racing {
            self.exit_tx.send(event).await.expect("the observer holds the exit receiver");
            settle().await;
        }
        self.sim.stop(handle).await
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        self.sim.status(handle).await
    }

    async fn resize(
        &self,
        handle: &AllocationHandle,
        resources: Resources,
    ) -> Result<(), DriverError> {
        self.sim.resize(handle, resources).await
    }

    fn take_exit_receiver(&self) -> Option<mpsc::Receiver<ExitEvent>> {
        self.exit_rx.lock().take()
    }

    fn on_alloc_running(&self, spec: &AllocationSpec) {
        self.calls.lock().push(Call::Running(spec.alloc.clone()));
        let mut spec = spec.clone();
        if spec.transport.is_none() {
            spec.transport.clone_from(&self.default_transport.lock());
        }
        let _token = self.runner.start_alloc(&spec);
    }

    fn on_alloc_terminal(&self, alloc_id: &AllocationId) {
        self.calls.lock().push(Call::Terminal(alloc_id.clone()));
        self.runner.stop_alloc(alloc_id);
    }

    fn on_alloc_stable(&self, alloc_id: &AllocationId) {
        self.runner.stop_role(alloc_id, ProbeRole::Startup);
        self.runner.start_liveness(alloc_id);
    }

    fn live_allocations(&self) -> Option<Vec<AllocationId>> {
        self.sim.live_allocations()
    }

    fn try_begin_reclamation(&self, alloc: &AllocationId) -> bool {
        self.sim.try_begin_reclamation(alloc)
    }

    fn release_supervision(&self, alloc: &AllocationId) {
        self.sim.release_supervision(alloc);
    }
}

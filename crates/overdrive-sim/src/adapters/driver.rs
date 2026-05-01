//! `SimDriver` — in-memory [`Driver`] implementation for DST.
//!
//! Each `SimDriver` is configured with a fixed [`DriverType`] and owns
//! an allocation table that tracks lifecycle state (`Running`,
//! `Terminated`, `Failed`). Failure modes are injected via builder
//! methods — `fail_on_start_with(reason)` — so scheduler and
//! reconciler tests can exercise "driver rejected start" behaviour
//! without spawning a real VMM.
//!
//! # Exit-event injection
//!
//! Per `fix-exec-driver-exit-watcher` Step 01-02, `SimDriver` mirrors
//! the production `ExecDriver`'s `ExitEvent` surface so the
//! `exit_observer` subsystem can be exercised under DST. Tests call
//! [`SimDriver::inject_exit_after`] to schedule a delayed `ExitEvent`
//! emission; the `Driver::take_exit_receiver` impl returns the
//! `mpsc::Receiver` half of the same channel.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use overdrive_core::id::AllocationId;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, ExitEvent,
    ExitKind, Resources,
};

use crate::adapters::clock::SimClock;

/// Capacity of the per-driver `ExitEvent` channel. Identical to
/// `ExecDriver`'s constant — sized for burst load.
const EXIT_CHANNEL_CAPACITY: usize = 256;

/// In-memory driver. Construct via [`SimDriver::new`], optionally
/// chain `.fail_on_start_with(reason)` to reject every subsequent
/// `start` call with a [`DriverError::StartRejected`].
pub struct SimDriver {
    r#type: DriverType,
    allocations: Mutex<HashMap<AllocationId, AllocationState>>,
    failure_mode: Mutex<Option<FailureMode>>,
    /// Per-alloc `intentional_stop` flags — mirrors `ExecDriver`'s
    /// `LiveAllocation::Running { intentional_stop, .. }` field.
    /// Set by [`Driver::stop`] BEFORE returning, read by the
    /// scheduled exit-event task before sending.
    intentional_stops: Mutex<HashMap<AllocationId, Arc<AtomicBool>>>,
    exit_tx: mpsc::Sender<ExitEvent>,
    exit_rx: Mutex<Option<mpsc::Receiver<ExitEvent>>>,
    /// Injected `Clock` used by [`SimDriver::inject_exit_after`] for
    /// the simulated wall-clock delay before emitting an `ExitEvent`.
    /// Defaults to a fresh [`SimClock`] when constructed via
    /// [`SimDriver::new`]; tests that need to share a clock with the
    /// observer subsystem (so observer counters are derived from the
    /// same logical-time source the harness drives) construct the
    /// driver via [`SimDriver::with_clock`]. Per
    /// `fix-exec-driver-exit-watcher` Step 01-02 RCA §Bug 1.
    ///
    /// `tokio::time::sleep` is forbidden here per whitepaper §21:
    /// every nondeterminism source in `core`-class logic crates must
    /// flow through an injected trait, and the matching test binding
    /// is the [`SimClock`] under DST.
    clock: Arc<dyn Clock>,
}

/// Configured failure mode for the driver. Stored behind a mutex so
/// tests can mutate it after construction (e.g. "fail the next start,
/// then succeed").
#[derive(Debug, Clone)]
enum FailureMode {
    StartRejected { reason: String },
}

impl SimDriver {
    /// Construct a `SimDriver` that reports `r#type` from
    /// [`Driver::type`] and holds no allocations.
    ///
    /// The injected [`Clock`] defaults to a fresh [`SimClock`].
    /// Tests that need to share a logical-time source with the
    /// observer subsystem (so observer counters are derived from the
    /// same clock the harness drives) construct via
    /// [`SimDriver::with_clock`].
    #[must_use]
    pub fn new(r#type: DriverType) -> Self {
        Self::with_clock(r#type, Arc::new(SimClock::new()))
    }

    /// Construct a `SimDriver` with a caller-supplied [`Clock`]. Used
    /// by integration tests that share the clock with the
    /// `exit_observer` subsystem so the observer's counter and the
    /// driver's emit-delay are derived from the same logical-time
    /// source. Per `fix-exec-driver-exit-watcher` Step 01-02 RCA
    /// §Bug 1.
    #[must_use]
    pub fn with_clock(r#type: DriverType, clock: Arc<dyn Clock>) -> Self {
        let (exit_tx, exit_rx) = mpsc::channel(EXIT_CHANNEL_CAPACITY);
        Self {
            r#type,
            allocations: Mutex::new(HashMap::new()),
            failure_mode: Mutex::new(None),
            intentional_stops: Mutex::new(HashMap::new()),
            exit_tx,
            exit_rx: Mutex::new(Some(exit_rx)),
            clock,
        }
    }

    /// Configure this driver to reject every subsequent `start` call
    /// with the given reason.
    #[must_use]
    pub fn fail_on_start_with(self, reason: String) -> Self {
        *self.failure_mode.lock() = Some(FailureMode::StartRejected { reason });
        self
    }

    /// DST hook — schedule an `ExitEvent` to be emitted on the
    /// driver's channel after `after` real-time wall-clock duration.
    ///
    /// `after = Duration::ZERO` emits immediately (still on a spawned
    /// task so the call site does not block). The `intentional_stop`
    /// flag the event carries is read at emission time, NOT at
    /// scheduling time — so a test that calls
    /// `driver.stop(handle)` before the scheduled emission fires will
    /// see `intentional_stop = true` on the event, exactly as the
    /// production watcher would.
    ///
    /// Mirrors `ExecDriver`'s spawn-watcher path: the production
    /// driver's per-alloc tokio task awaits `child.wait()` then
    /// sends; the sim's task awaits `tokio::time::sleep(after)` then
    /// sends. Same observation surface.
    pub fn inject_exit_after(&self, alloc: &AllocationId, after: Duration, kind: ExitKind) {
        let alloc = alloc.clone();
        let exit_tx = self.exit_tx.clone();
        let clock = self.clock.clone();
        // Get-or-insert the per-alloc intentional_stop flag — if the
        // alloc was never started via `Driver::start` (rare in tests
        // but defensible), construct a fresh `false` flag so the
        // emitted event still carries a valid bool.
        let intentional_stop = self
            .intentional_stops
            .lock()
            .entry(alloc.clone())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone();
        tokio::spawn(async move {
            // Per RCA §Bug 1: route the simulated emit-delay through
            // the injected `Clock` so DST runs deterministic —
            // `SimClock::sleep` advances logical time in place and
            // yields once cooperatively before returning.
            // `tokio::time::sleep` would block on a real wall-clock
            // timer, breaking the test harness's single-threaded
            // ordering invariants and stalling the observer task
            // indefinitely under SimClock.
            //
            // Even when `after.is_zero()`, the spawn task must yield
            // at least once so a peer task awaiting on the mpsc
            // receiver actually gets scheduled — without this, under
            // a single-threaded `#[tokio::test]` runtime the test
            // thread stays on-CPU through the convergence loop and
            // the observer never receives the event.
            clock.sleep(after).await;
            let intentional = intentional_stop.load(Ordering::SeqCst);
            // When `intentional_stop` is true, the event collapses to
            // `CleanExit` so the observer writes Terminated (mirrors
            // the production `classify_exit` mapping in
            // `crates/overdrive-worker/src/driver.rs`).
            let final_kind = if intentional { ExitKind::CleanExit } else { kind };
            let event = ExitEvent { alloc, kind: final_kind, intentional_stop: intentional };
            let _ = exit_tx.send(event).await;
        });
    }
}

#[async_trait]
impl Driver for SimDriver {
    fn r#type(&self) -> DriverType {
        self.r#type
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        let failure = self.failure_mode.lock().clone();
        if let Some(FailureMode::StartRejected { reason }) = failure {
            return Err(DriverError::StartRejected { driver: self.r#type, reason });
        }

        self.allocations.lock().insert(spec.alloc.clone(), AllocationState::Running);
        // Mint a fresh `intentional_stop` flag for this alloc so the
        // scheduled exit-event task can observe operator stops via
        // the shared `Arc<AtomicBool>`.
        self.intentional_stops.lock().insert(spec.alloc.clone(), Arc::new(AtomicBool::new(false)));
        Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: None })
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        {
            let mut allocations = self.allocations.lock();
            if !allocations.contains_key(&handle.alloc) {
                return Err(DriverError::NotFound { alloc: handle.alloc.clone() });
            }
            allocations.insert(handle.alloc.clone(), AllocationState::Terminated);
        }
        // Per `fix-exec-driver-exit-watcher` RCA §Approved fix item
        // 3: set `intentional_stop = true` BEFORE any further side
        // effect. The flag is shared with any in-flight scheduled
        // exit-event task; the next emission honours it.
        let flag = self
            .intentional_stops
            .lock()
            .entry(handle.alloc.clone())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone();
        flag.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        self.allocations
            .lock()
            .get(&handle.alloc)
            .cloned()
            .ok_or_else(|| DriverError::NotFound { alloc: handle.alloc.clone() })
    }

    async fn resize(
        &self,
        handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        if !self.allocations.lock().contains_key(&handle.alloc) {
            return Err(DriverError::NotFound { alloc: handle.alloc.clone() });
        }
        Ok(())
    }

    fn take_exit_receiver(&self) -> Option<mpsc::Receiver<ExitEvent>> {
        self.exit_rx.lock().take()
    }
}

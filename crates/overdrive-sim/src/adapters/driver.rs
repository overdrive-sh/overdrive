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
use tokio::sync::{mpsc, oneshot};

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
    /// Per-alloc Running-confirmed gate senders. Stashed at
    /// [`Driver::start`]; consumed by
    /// [`Driver::release_for_exit_emission`]. Mirrors
    /// `ExecDriver`'s `LiveAllocation::gate_sender` field; the
    /// matching `oneshot::Receiver` is parked in `gate_receivers`
    /// until [`SimDriver::inject_exit_after`] picks it up for the
    /// scheduled emit task. Per
    /// `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`
    /// (Solution 1').
    gate_senders: Mutex<HashMap<AllocationId, oneshot::Sender<()>>>,
    /// Per-alloc Running-confirmed gate receivers, parked between
    /// [`Driver::start`] and the first call to
    /// [`SimDriver::inject_exit_after`] for that alloc. Tests that
    /// never call `inject_exit_after` (i.e. the workload never
    /// "exits" in simulation) drop their receiver when the driver
    /// is dropped — no leak.
    gate_receivers: Mutex<HashMap<AllocationId, oneshot::Receiver<()>>>,
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
            gate_senders: Mutex::new(HashMap::new()),
            gate_receivers: Mutex::new(HashMap::new()),
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

    /// Test-only inspection hook — number of entries currently in the
    /// internal `allocations` map.
    ///
    /// The `Driver` trait does not (and should not) expose live-map
    /// cardinality. This accessor is the regression hook for
    /// `fix-terminated-slot-accumulation` Step 01-01: the sim adapter
    /// must mirror `ExecDriver`'s cardinality contract so the shared
    /// trait does not diverge across host/sim. The GREEN fix (Step
    /// 01-02) evicts the slot in `stop()` so `live_count()` returns 0
    /// after each round-trip; this accessor lets the regression test
    /// assert the post-stop cardinality is zero.
    ///
    /// `overdrive-sim` is `adapter-sim` class — only consumed by
    /// tests — so this accessor is plain `pub` rather than feature-gated.
    /// It is not on the `Driver` trait.
    pub fn live_count(&self) -> usize {
        self.allocations.lock().len()
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
        // Take the Running-confirmed gate receiver. Three cases:
        //
        // 1. `start` was already called for this alloc → receiver was
        //    parked at start-time, take it here. The matching sender
        //    is held in `gate_senders` and consumed by
        //    `release_for_exit_emission` (action-shim or
        //    exit_observer's degraded path).
        //
        // 2. `inject_exit_after` was called BEFORE `start` (the 01-01
        //    regression test does exactly this — inject sub-budget
        //    exit before the first convergence tick spawns
        //    `StartAllocation`). The alloc is NOT yet in
        //    `allocations`; we mint a fresh channel here, stash the
        //    sender for the future `start` call to find, and use
        //    the receiver for the spawned exit task's gate-await.
        //    Without this pre-mint, the spawned task would proceed
        //    with "no gate configured" semantics, recreating the
        //    original `find_prior_row → NoPriorRow` race the gate
        //    exists to prevent.
        //
        // 3. The gate fired already (action shim post-Ok firing site
        //    consumed the sender; receiver was consumed by an earlier
        //    `inject_exit_after`, OR `Driver::stop` ran and dropped
        //    both sides). The alloc IS in `allocations` but neither
        //    `gate_senders[alloc]` nor `gate_receivers[alloc]` is
        //    present. Proceed without a gate — the orphan path.
        //
        // Per `docs/feature/fix-exit-observer-running-gate/deliver/
        // rca.md` (Solution 1') and step 01-03 of that feature.
        // The three-case decomposition below is load-bearing — see the
        // "Three cases" comment block above. The `clippy::option_if_let_else`
        // lint would collapse the structure into a `map_or_else` that
        // hides the case analysis behind a closure boundary, defeating
        // the readability of cases 2 and 3 which are NOT a simple
        // default-value computation. `expect` (not `allow`) so the lint
        // gate self-removes if the structure is ever refactored such
        // that the lint legitimately stops firing.
        #[expect(
            clippy::option_if_let_else,
            reason = "three-case decomposition of receiver/sender state \
                      is the load-bearing artifact; map_or_else collapses it"
        )]
        let gate_receiver = {
            let existing = self.gate_receivers.lock().remove(&alloc);
            if let Some(rx) = existing {
                Some(rx)
            } else if !self.intentional_stops.lock().contains_key(&alloc) {
                // Case 2: neither `start` nor `stop` has been called
                // for this alloc — `intentional_stops` is populated
                // by both, so its absence means the alloc is unknown
                // to the driver. Pre-mint a channel for the future
                // `start` call to reuse so the gate's happens-before
                // edge stands even when `inject_exit_after` is
                // sequenced before the first `StartAllocation`
                // dispatch (the 01-01 regression test does exactly
                // this).
                let (tx, rx) = oneshot::channel::<()>();
                self.gate_senders.lock().insert(alloc.clone(), tx);
                Some(rx)
            } else {
                // Case 3: `start` was already called and either the
                // gate fired (sender consumed, receiver consumed by
                // a prior inject_exit_after) or `stop` cleared both
                // sides. Orphan path — proceed without a gate; any
                // production-flow ordering edge has already landed
                // (the action shim's post-Ok fire happened before
                // the action sequence reached `stop`).
                None
            }
        };
        // Get-or-insert the per-alloc intentional_stop flag — if the
        // alloc was never started via `Driver::start` (rare in tests
        // but defensible), construct a fresh `false` flag so the
        // emitted event still carries a valid bool. Done AFTER the
        // gate-receiver decision above so the case-2 / case-3
        // branching can use `intentional_stops.contains_key` as the
        // "alloc known to driver" signal — populating the entry here
        // would defeat that detection.
        let intentional_stop = self
            .intentional_stops
            .lock()
            .entry(alloc.clone())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone();
        tokio::spawn(async move {
            // Route the simulated emit-delay through the injected
            // `Clock`. Under DST (`SimClock::sleep` is a deterministic
            // park), the spawned task suspends until the test harness
            // calls `sim_clock.tick(after)` to advance logical time
            // past the deadline — at which point the timer wakes and
            // the body runs.
            clock.sleep(after).await;
            // Sim-internal: cooperative yield so a peer task awaiting
            // on the mpsc receiver actually gets scheduled before this
            // task continues. Required because the spawned task and
            // the observer task may share a single-threaded
            // `#[tokio::test]` runtime; without this yield, the
            // observer never gets a chance to drain the channel between
            // the timer wake and the `exit_tx.send()` below. This
            // belongs in the SimDriver — not in production code that
            // reads exit events — per
            // `.claude/rules/development.md` § "Production code is
            // not shaped by simulation".
            tokio::task::yield_now().await;
            let intentional = intentional_stop.load(Ordering::SeqCst);
            // When `intentional_stop` is true, the event collapses to
            // `CleanExit` so the observer writes Terminated (mirrors
            // the production `classify_exit` mapping in
            // `crates/overdrive-worker/src/driver.rs`).
            let final_kind = if intentional { ExitKind::CleanExit } else { kind };
            // SimDriver has no real stderr to capture — the watcher
            // contract leaves `stderr_tail = None`. Tests that want
            // to exercise the tail-render path inject explicit stderr
            // via the dedicated sim-driver helper (added when the
            // first such test lands per step 02-05 / ADR-0033
            // Amendment 2026-05-10).
            let event = ExitEvent {
                alloc,
                kind: final_kind,
                intentional_stop: intentional,
                stderr_tail: None,
            };
            // Running-confirmed gate await: symmetric with
            // `ExecDriver`'s watcher per
            // `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`
            // (Solution 1'). The action shim fires the gate via
            // `Driver::release_for_exit_emission` after committing
            // `obs.write(Running)` (or the May-2 degraded path).
            // Without this happens-before edge a sub-millisecond
            // emit delay would race the action shim's `Running`
            // write and the observer would silently drop the event
            // on its `find_prior_row → NoPriorRow` arm.
            //
            // ORDERING: gate await happens AFTER the simulated
            // emit-delay (`clock.sleep(after)`) AND AFTER the
            // sim-internal cooperative yield, BEFORE
            // `exit_tx.send`. This mirrors the `ExecDriver` watcher's
            // "after `child.wait()` AND stderr-tail drain budget,
            // before `exit_tx.send`" ordering.
            //
            // `tokio::sync::oneshot` is NOT `Clock`-dependent —
            // works under `SimClock` / turmoil / real tokio
            // identically. The gate is a logical happens-before
            // edge, not a wall-clock budget, so this is the
            // structural production ordering edge — NOT a sim
            // concession (per
            // `.claude/rules/development.md` § "Production code is
            // not shaped by simulation").
            //
            // `Err(RecvError)` (sender dropped without sending —
            // action-shim-crashed orphan path) AND `None`
            // (alloc whose `start` was never called) both
            // collapse to "proceed and emit"; symmetric with
            // `ExecDriver`'s orphan-path handling.
            if let Some(gate_receiver) = gate_receiver {
                let _ = gate_receiver.await;
            }
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
        // Mint the Running-confirmed gate per
        // `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`
        // (Solution 1'). Sender stashed for
        // `Driver::release_for_exit_emission` to consume; receiver
        // parked for `inject_exit_after` to pick up. Mirrors
        // `ExecDriver`'s start-time mint of the channel.
        //
        // Per step 01-03 of `fix-exit-observer-running-gate`: the
        // 01-02 transitional immediate-drop has been removed. The
        // sender is held in `gate_senders` until the action shim
        // (post-`obs.write(Running)` Ok) or the exit_observer (May-2
        // degraded escalation) fires it via
        // `Driver::release_for_exit_emission`. On `Driver::stop` the
        // sender is dropped (orphan path); the spawned
        // `inject_exit_after` task's `gate_receiver.await` resolves
        // to `Err(RecvError)` and emit proceeds.
        //
        // If `inject_exit_after` was called BEFORE this `start` for
        // the same alloc (the 01-01 regression test does this — sub-
        // budget exit injected before the first convergence tick),
        // the sender was pre-minted in `inject_exit_after` and the
        // receiver was parked into the spawned exit-emit task.
        // Reuse the existing sender so `release_for_exit_emission`
        // fires the SAME oneshot the spawned task is awaiting.
        let mut senders = self.gate_senders.lock();
        if !senders.contains_key(&spec.alloc) {
            let (gate_sender, gate_receiver) = oneshot::channel::<()>();
            senders.insert(spec.alloc.clone(), gate_sender);
            drop(senders);
            self.gate_receivers.lock().insert(spec.alloc.clone(), gate_receiver);
        }
        Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: None })
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        // Symmetric with `ExecDriver::stop` per
        // `fix-terminated-slot-accumulation` Step 01-02: the slot is
        // removed on stop, NOT overwritten with `Terminated`. Durable
        // terminal-state truth lives in the `ObservationStore`
        // (`AllocStatusRow`); the driver retains no terminal-state
        // memory. See the `Driver::status` rustdoc in `overdrive-core`
        // for the post-stop contract.
        {
            let mut allocations = self.allocations.lock();
            if allocations.remove(&handle.alloc).is_none() {
                return Err(DriverError::NotFound { alloc: handle.alloc.clone() });
            }
        }
        // Per `fix-exec-driver-exit-watcher` RCA §Approved fix item
        // 3: set `intentional_stop = true` BEFORE any further side
        // effect. The flag is shared with any in-flight scheduled
        // exit-event task; the next emission honours it. The flag is
        // intentionally NOT removed alongside the alloc slot — a
        // scheduled exit-event task may still fire after stop()
        // returns and must read `true` from the shared flag.
        let flag = self
            .intentional_stops
            .lock()
            .entry(handle.alloc.clone())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone();
        flag.store(true, Ordering::SeqCst);
        // Drop any unsent gate sender — if `release_for_exit_emission`
        // was never called before stop landed, the spawned
        // `inject_exit_after` task's `gate_receiver.await` resolves
        // to `Err(RecvError)` and the task proceeds with the emit.
        // Symmetric with `ExecDriver::stop`'s `drop(gate_sender)`.
        // Per `Driver::start` rustdoc § "Sender drop (orphan path)".
        let _dropped_sender = self.gate_senders.lock().remove(&handle.alloc);
        // Drop any unparked gate receiver — if `inject_exit_after`
        // was never called for this alloc (test never simulated an
        // exit), the receiver was sitting idle. No leak.
        let _dropped_receiver = self.gate_receivers.lock().remove(&handle.alloc);
        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        match self.allocations.lock().get(&handle.alloc) {
            Some(_) => Ok(AllocationState::Running),
            None => Err(DriverError::NotFound { alloc: handle.alloc.clone() }),
        }
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

    /// Fire the Running-confirmed gate for `handle.alloc`. Symmetric
    /// with `ExecDriver::release_for_exit_emission`. Idempotent: a
    /// call against an alloc whose gate has already fired (or whose
    /// alloc is unknown to the driver) is a no-op, NOT a panic. The
    /// structural exactly-once guarantee comes from
    /// `HashMap::remove` + `oneshot::Sender::send` consume-self.
    fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        let sender = self.gate_senders.lock().remove(&handle.alloc);
        if let Some(sender) = sender {
            // `Err(())` from a closed receiver (the spawned
            // `inject_exit_after` task already dropped its receiver
            // post-emit, or the alloc was never injected) is benign.
            let _ = sender.send(());
        }
        // Unknown alloc OR gate already fired: no-op per the
        // idempotent-fire contract.
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod release_for_exit_emission_tests {
    //! Trait-contract unit tests for the Running-confirmed gate
    //! introduced by step 01-02 of `fix-exit-observer-running-gate`.
    //! See `Driver::start` / `Driver::release_for_exit_emission`
    //! rustdoc on the `overdrive-core` trait for the contract under
    //! test. The integration-level happens-before edge (gate-await
    //! before `ExitEvent` emission) is exercised by the
    //! 01-01 regression test in
    //! `crates/overdrive-control-plane/tests/integration/workload_lifecycle/`
    //! once step 01-03 wires the firing site.
    use super::*;
    use overdrive_core::SpiffeId;
    use overdrive_core::traits::driver::{AllocationSpec, Resources};
    use std::str::FromStr;

    fn sample_spec(name: &str) -> AllocationSpec {
        AllocationSpec {
            alloc: AllocationId::from_str(name).expect("valid AllocationId"),
            identity: SpiffeId::from_str("spiffe://overdrive.local/test/wl")
                .expect("valid SpiffeId"),
            command: "/bin/true".to_owned(),
            args: vec![],
            resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
            probe_descriptors: Vec::new(),
            netns: None,
            host_veth: None,
        }
    }

    /// Behavior 1: idempotent fire — a second
    /// `release_for_exit_emission` against the same alloc is a no-op,
    /// NOT a panic. Protects future call sites firing the gate twice
    /// (e.g. successful retry AND May-2 degraded escalation both
    /// firing).
    #[tokio::test]
    async fn release_for_exit_emission_is_idempotent() {
        let driver = SimDriver::new(DriverType::Exec);
        let spec = sample_spec("alloc-idempotent");
        let handle = driver.start(&spec).await.expect("start succeeds");
        // First fire — consumes the stashed sender.
        driver.release_for_exit_emission(&handle);
        // Second fire — must NOT panic. (Asserted by the test
        // returning normally.)
        driver.release_for_exit_emission(&handle);
    }

    /// Behavior 2: release against an unknown alloc is a no-op, NOT
    /// a panic. Protects the action shim's call path against races
    /// (e.g. driver evicted the slot before the shim reached
    /// release).
    #[test]
    fn release_for_exit_emission_on_unknown_alloc_is_noop() {
        let driver = SimDriver::new(DriverType::Exec);
        let unknown = AllocationHandle {
            alloc: AllocationId::from_str("alloc-never-started").expect("valid AllocationId"),
            pid: None,
        };
        // No `start` call; no stashed sender. Must NOT panic.
        driver.release_for_exit_emission(&unknown);
    }
}

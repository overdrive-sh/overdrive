//! Test-side bindings for the `serve` lifetime port
//! (`overdrive_cli::commands::serve_lifetime`).
//!
//! The port injects two nondeterministic inputs: the operator stop-signal
//! source and the clock that measures the fail-stop outer bound. Production
//! binds real `SIGINT`/`SIGTERM` and `SystemClock`; these are the test
//! bindings the proofs drive instead. Neither fabricates a control-plane
//! outcome — they only decide *when* an operator signal is ready and *when*
//! logical time passes.

#![allow(dead_code, reason = "shared by several kvm-gated proof modules")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_cli::commands::serve::ServeHandle;
use overdrive_cli::commands::serve_lifetime::{
    ServeExit, ServeLifetime, ServeSignal, ServeSignals,
};
use overdrive_core::traits::clock::Clock;
use overdrive_sim::adapters::clock::SimClock;
use tokio::sync::{Notify, mpsc};

/// A [`ServeSignals`] source whose signals are delivered by the test through a
/// [`SignalDriver`]. A signal sent before the lifetime starts is ready on the
/// lifetime's first poll.
pub(super) struct DrivenSignals {
    rx: mpsc::UnboundedReceiver<ServeSignal>,
    consumed: Arc<AtomicUsize>,
}

/// The test's handle for delivering operator signals and observing whether
/// the lifetime consumed any.
#[derive(Clone)]
pub(super) struct SignalDriver {
    tx: mpsc::UnboundedSender<ServeSignal>,
    consumed: Arc<AtomicUsize>,
}

pub(super) fn driven_signals() -> (DrivenSignals, SignalDriver) {
    let (tx, rx) = mpsc::unbounded_channel();
    let consumed = Arc::new(AtomicUsize::new(0));
    (DrivenSignals { rx, consumed: Arc::clone(&consumed) }, SignalDriver { tx, consumed })
}

impl SignalDriver {
    /// Deliver one operator signal.
    pub(super) fn deliver(&self, signal: ServeSignal) {
        self.tx.send(signal).expect("the lifetime's signal source is alive");
    }

    /// How many delivered signals the lifetime has consumed.
    pub(super) fn consumed(&self) -> usize {
        self.consumed.load(Ordering::SeqCst)
    }
}

impl ServeSignals for DrivenSignals {
    async fn recv(&mut self) -> ServeSignal {
        match self.rx.recv().await {
            Some(signal) => {
                self.consumed.fetch_add(1, Ordering::SeqCst);
                signal
            }
            None => std::future::pending().await,
        }
    }
}

/// A [`Clock`] over [`SimClock`] that records every requested sleep and
/// notifies the test each time one is armed. Logical time passes only when
/// the test calls [`RecordingClock::advance`].
#[derive(Clone)]
pub(super) struct RecordingClock {
    sim: SimClock,
    armed: Arc<std::sync::Mutex<Vec<Duration>>>,
    notify: Arc<Notify>,
}

impl RecordingClock {
    pub(super) fn new() -> Self {
        Self {
            sim: SimClock::new(),
            armed: Arc::new(std::sync::Mutex::new(Vec::new())),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Every sleep duration requested so far, in request order.
    pub(super) fn armed(&self) -> Vec<Duration> {
        self.armed.lock().expect("armed-sleep log").clone()
    }

    /// Resolve once at least one sleep has been armed.
    pub(super) async fn wait_armed(&self) {
        loop {
            let notified = self.notify.notified();
            if !self.armed.lock().expect("armed-sleep log").is_empty() {
                return;
            }
            notified.await;
        }
    }

    /// Advance logical time, waking every sleep whose deadline has passed.
    pub(super) fn advance(&self, by: Duration) {
        self.sim.tick(by);
    }
}

#[async_trait]
impl Clock for RecordingClock {
    fn now(&self) -> Instant {
        self.sim.now()
    }

    fn unix_now(&self) -> Duration {
        self.sim.unix_now()
    }

    async fn sleep(&self, duration: Duration) {
        self.armed.lock().expect("armed-sleep log").push(duration);
        self.notify.notify_waiters();
        self.sim.sleep(duration).await;
    }
}

/// Abandon a serve owner through the lifetime port's killed mode — the single
/// in-process process-loss mechanism — and report anything but `Killed`.
pub(super) async fn kill_serve_owner(handle: ServeHandle) -> Result<(), String> {
    let (signals, driver) = driven_signals();
    driver.deliver(ServeSignal::Kill);
    match ServeLifetime::new(signals, Arc::new(RecordingClock::new())).run(handle).await {
        Ok(ServeExit::Killed) => Ok(()),
        other => Err(format!("killed mode returned {other:?}")),
    }
}

//! `SimLaunchSupersession` — the sim twin of the convergence lane's launch
//! supersession signal (ADR-0334).
//!
//! A test raises the signal; `superseded()` parks on a watch channel and
//! never auto-resolves (the `SimClock::sleep` shape). `raise()` makes
//! `is_superseded()` true and resolves every pending and every later
//! `superseded()` as one step.

use std::sync::Arc;

use async_trait::async_trait;
use overdrive_core::traits::driver::LaunchSupersession;
use tokio::sync::watch;

/// A launch supersession signal a test raises.
#[derive(Debug, Clone)]
pub struct SimLaunchSupersession {
    tx: Arc<watch::Sender<bool>>,
}

impl SimLaunchSupersession {
    /// An unraised signal.
    #[must_use]
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(false);
        Self { tx: Arc::new(tx) }
    }

    /// A signal that is already raised (a lane born raised).
    #[must_use]
    pub fn raised() -> Self {
        let signal = Self::new();
        signal.raise();
        signal
    }

    /// Raise the signal. Idempotent: a second raise behaves as one.
    pub fn raise(&self) {
        self.tx.send_replace(true);
    }
}

impl Default for SimLaunchSupersession {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LaunchSupersession for SimLaunchSupersession {
    fn is_superseded(&self) -> bool {
        *self.tx.borrow()
    }

    async fn superseded(&self) {
        let mut rx = self.tx.subscribe();
        // `wait_for` resolves at once when the value already satisfies the
        // predicate; the sender is owned by `self`, so the channel cannot
        // close while this future is alive.
        let _ = rx.wait_for(|raised| *raised).await;
    }
}

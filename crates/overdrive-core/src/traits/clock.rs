//! [`Clock`] — the sole source of time for Overdrive logic.
//!
//! Production wires this to a `SystemClock`; DST wires it to `turmoil`'s
//! controllable clock. `Instant::now()` and `tokio::time::sleep` are
//! forbidden outside wiring crates.

use std::time::{Duration, Instant};

use async_trait::async_trait;

#[async_trait]
pub trait Clock: Send + Sync + 'static {
    /// Monotonic clock reading.
    fn now(&self) -> Instant;

    /// Wall-clock duration since the UNIX epoch.
    fn unix_now(&self) -> Duration;

    /// Sleep for `duration`. In simulation this advances logical time;
    /// in production it yields to the Tokio timer.
    async fn sleep(&self, duration: Duration);
}

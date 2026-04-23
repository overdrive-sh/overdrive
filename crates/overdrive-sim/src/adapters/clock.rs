//! `SimClock` — logical-time clock for DST.
//!
//! The sim clock is explicit state, not wall-clock. `now()` returns a
//! monotonic [`Instant`] that only advances when the harness calls
//! [`SimClock::tick`] or a reconciler/workflow calls [`SimClock::sleep`].
//! `tokio::time::sleep` and `Instant::now()` remain banned in core logic
//! crates — the dst-lint gate (step 05-02) enforces that.
//!
//! The clock's internal counter is stored in nanoseconds as an
//! [`AtomicU64`] behind an [`Arc`], so clones share state. This shape
//! matches how turmoil exposes its own clock and keeps multi-task test
//! setups coherent without a lock on the read path.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;

use overdrive_core::traits::clock::Clock;

/// Deterministic clock backed by a shared [`AtomicU64`] counter.
///
/// Time is measured in nanoseconds since construction. The epoch is
/// captured at construction as an [`Instant`] so `now()` can return a
/// real [`Instant`] that satisfies downstream APIs expecting one — the
/// returned value advances ONLY through `tick` / `sleep`, never through
/// wall-clock passage.
pub struct SimClock {
    /// Real [`Instant`] captured at construction. Used as the base for
    /// the returned `now()` — never queried after construction.
    epoch: Instant,
    /// Wall-clock epoch captured at construction. `unix_now()` returns
    /// `unix_epoch + elapsed_logical` so UNIX timestamps are stable and
    /// reproducible across runs when paired with the same logical
    /// advance sequence.
    unix_epoch: Duration,
    /// Logical time elapsed since construction, in nanoseconds.
    elapsed_nanos: Arc<AtomicU64>,
}

impl SimClock {
    /// Construct a fresh sim clock at logical-time zero.
    #[must_use]
    pub fn new() -> Self {
        let unix_epoch = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0));
        Self { epoch: Instant::now(), unix_epoch, elapsed_nanos: Arc::new(AtomicU64::new(0)) }
    }

    /// Advance logical time by `duration`. `Duration` values that would
    /// overflow the internal `u64` nanosecond counter saturate at
    /// `u64::MAX` — a nonsensical test input, not a silent wrap.
    pub fn tick(&self, duration: Duration) {
        let nanos = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
        self.elapsed_nanos.fetch_add(nanos, Ordering::Relaxed);
    }

    fn elapsed(&self) -> Duration {
        Duration::from_nanos(self.elapsed_nanos.load(Ordering::Relaxed))
    }
}

impl Default for SimClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for SimClock {
    /// Cloning a [`SimClock`] shares the underlying counter — every
    /// clone sees the same logical time. This is what callers expect
    /// when handing a clock to both a harness and a system under test.
    fn clone(&self) -> Self {
        Self {
            epoch: self.epoch,
            unix_epoch: self.unix_epoch,
            elapsed_nanos: Arc::clone(&self.elapsed_nanos),
        }
    }
}

#[async_trait]
impl Clock for SimClock {
    fn now(&self) -> Instant {
        self.epoch + self.elapsed()
    }

    fn unix_now(&self) -> Duration {
        self.unix_epoch + self.elapsed()
    }

    async fn sleep(&self, duration: Duration) {
        // Advance logical time in place; do not yield to `tokio::time`.
        self.tick(duration);
    }
}

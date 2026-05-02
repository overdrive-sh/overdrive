//! `SimClock` — logical-time clock for DST.
//!
//! The sim clock is explicit state, not wall-clock. `now()` returns a
//! monotonic [`Instant`] that only advances when the harness calls
//! [`SimClock::tick`]. `tokio::time::sleep` and `Instant::now()` remain
//! banned in core logic crates — the dst-lint gate (step 05-02)
//! enforces that.
//!
//! [`SimClock::sleep`] is a *deterministic park*: it registers a waker
//! on a deadline and returns Pending; only [`SimClock::tick`] advances
//! logical time and wakes any timers whose deadline has passed. This
//! mirrors how every credible DST framework (turmoil, madsim,
//! `FoundationDB` flow sim) implements `sleep`. The harness — never the
//! system under test — drives logical time. Auto-advancing inside
//! `sleep` would defeat tests whose entire purpose is to control time
//! externally to verify a deadline-bound behavior in the SUT.
//!
//! The clock's internal counter is stored in nanoseconds as an
//! [`AtomicU64`] behind an [`Arc`], so clones share state. This shape
//! matches how turmoil exposes its own clock and keeps multi-task test
//! setups coherent without a lock on the read path.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use parking_lot::Mutex;

use overdrive_core::traits::clock::Clock;

/// Deterministic clock backed by a shared [`AtomicU64`] counter and a
/// shared waker registry.
///
/// Time is measured in nanoseconds since construction. The epoch is
/// captured at construction as an [`Instant`] so `now()` can return a
/// real [`Instant`] that satisfies downstream APIs expecting one — the
/// returned value advances ONLY through `tick`, never through wall-clock
/// passage and never through `sleep` (which parks until `tick` advances
/// past its deadline).
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
    /// Pending sleep timers, keyed by deadline (nanoseconds since
    /// construction). Each entry is a `(deadline, Waker)` pair. The
    /// vector is unsorted — `tick` walks it linearly to find expired
    /// timers; the expected entry count is small (one per parked task).
    timers: Arc<Mutex<Vec<(u64, Waker)>>>,
}

impl SimClock {
    /// Construct a fresh sim clock at logical-time zero.
    #[must_use]
    pub fn new() -> Self {
        let unix_epoch = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0));
        Self {
            epoch: Instant::now(),
            unix_epoch,
            elapsed_nanos: Arc::new(AtomicU64::new(0)),
            timers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Advance logical time by `duration`. `Duration` values that would
    /// overflow the internal `u64` nanosecond counter saturate at
    /// `u64::MAX` — a nonsensical test input, not a silent wrap.
    ///
    /// Any [`SimClock::sleep`] futures whose deadline has now been
    /// reached are woken; their next poll observes the elapsed counter
    /// past the deadline and returns `Ready`.
    pub fn tick(&self, duration: Duration) {
        let nanos = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
        let new_elapsed = self.elapsed_nanos.fetch_add(nanos, Ordering::Relaxed) + nanos;
        // Drain expired timers under the lock, then drop the lock
        // BEFORE waking. `Waker::wake` may reentrantly poll the woken
        // future, which under a single-threaded runtime can immediately
        // try to re-grab `self.timers`; parking_lot::Mutex is non-
        // reentrant and would deadlock.
        let mut timers = self.timers.lock();
        let mut expired: Vec<Waker> = Vec::new();
        let mut i = 0;
        while i < timers.len() {
            if timers[i].0 <= new_elapsed {
                expired.push(timers.swap_remove(i).1);
            } else {
                i += 1;
            }
        }
        drop(timers);
        for w in expired {
            w.wake();
        }
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
    /// Cloning a [`SimClock`] shares the underlying counter AND the
    /// timer registry — every clone sees the same logical time and the
    /// same parked timers. This is what callers expect when handing a
    /// clock to both a harness and a system under test.
    fn clone(&self) -> Self {
        Self {
            epoch: self.epoch,
            unix_epoch: self.unix_epoch,
            elapsed_nanos: Arc::clone(&self.elapsed_nanos),
            timers: Arc::clone(&self.timers),
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
        let nanos = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
        let deadline = self.elapsed_nanos.load(Ordering::Relaxed).saturating_add(nanos);
        SleepUntil {
            elapsed_nanos: Arc::clone(&self.elapsed_nanos),
            timers: Arc::clone(&self.timers),
            deadline,
        }
        .await;
    }
}

/// Future that resolves when [`SimClock`]'s elapsed counter has
/// advanced past `deadline`. Polling registers a waker on the shared
/// timer registry; [`SimClock::tick`] wakes timers whose deadline has
/// been reached.
struct SleepUntil {
    elapsed_nanos: Arc<AtomicU64>,
    timers: Arc<Mutex<Vec<(u64, Waker)>>>,
    deadline: u64,
}

impl std::future::Future for SleepUntil {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.elapsed_nanos.load(Ordering::Relaxed) >= self.deadline {
            return Poll::Ready(());
        }
        let our_waker = cx.waker().clone();
        let resolved = {
            let mut timers = self.timers.lock();
            // Remove any stale entry whose Waker.will_wake matches ours
            // (a re-polled future leaves a stale entry; clear it before
            // re-inserting so the registry does not grow unboundedly).
            timers.retain(|(_, w)| !w.will_wake(&our_waker));
            // Re-check elapsed under the lock to close the race between
            // the early-return load above and a concurrent `tick`
            // draining the registry between then and now.
            if self.elapsed_nanos.load(Ordering::Relaxed) >= self.deadline {
                true
            } else {
                timers.push((self.deadline, our_waker));
                false
            }
        };
        if resolved { Poll::Ready(()) } else { Poll::Pending }
    }
}

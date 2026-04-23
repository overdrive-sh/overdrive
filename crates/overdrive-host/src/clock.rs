//! Host [`Clock`] binding — `std::time` + `tokio::time`.

use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use overdrive_core::traits::clock::Clock;

/// Production clock backed by `std::time::Instant::now` and
/// `tokio::time::sleep`.
///
/// The sim counterpart is `overdrive_sim::adapters::SimClock`, which
/// advances turmoil's virtual time deterministically. Swap at the
/// wiring boundary; no call site should need both.
#[derive(Debug, Clone, Default)]
pub struct SystemClock;

#[async_trait]
impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_now(&self) -> Duration {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

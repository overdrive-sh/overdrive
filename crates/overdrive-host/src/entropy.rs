//! Host [`Entropy`] binding — OS RNG via `rand::rngs::OsRng`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use overdrive_core::traits::entropy::Entropy;
use rand::RngCore;

/// Production entropy source. Uses `rand::rngs::OsRng` — the OS RNG
/// — which `rand` documents as cryptographically secure.
#[derive(Debug, Default)]
pub struct OsEntropy;

impl Entropy for OsEntropy {
    fn u64(&self) -> u64 {
        let mut rng = rand::rngs::OsRng;
        rng.next_u64()
    }

    fn fill(&self, buf: &mut [u8]) {
        let mut rng = rand::rngs::OsRng;
        rng.fill_bytes(buf);
    }
}

/// Sanity wrapper around [`OsEntropy`] that counts every pull, so
/// a test can assert the host adapter (rather than a sim) is on the
/// path under a given wiring.
#[derive(Debug, Default)]
pub struct CountingOsEntropy {
    pulls: Arc<AtomicUsize>,
}

impl CountingOsEntropy {
    /// Construct a counting entropy wrapper.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Total number of `u64` / `fill` pulls observed.
    #[must_use]
    pub fn pulls(&self) -> usize {
        self.pulls.load(Ordering::Relaxed)
    }
}

impl Entropy for CountingOsEntropy {
    fn u64(&self) -> u64 {
        self.pulls.fetch_add(1, Ordering::Relaxed);
        OsEntropy.u64()
    }

    fn fill(&self, buf: &mut [u8]) {
        self.pulls.fetch_add(1, Ordering::Relaxed);
        OsEntropy.fill(buf);
    }
}

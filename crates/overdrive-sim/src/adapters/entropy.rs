//! `SimEntropy` — seeded `StdRng` implementation of the [`Entropy`]
//! port.
//!
//! The core-logic gate (step 05-02) bans `rand::random()` and
//! `rand::thread_rng()` outside wiring crates. Every "random" value in
//! Overdrive's simulation flows through a `SimEntropy` instance seeded
//! from the DST harness seed — identical seeds yield bit-identical
//! draws, which is what makes DST reproducible.
//!
//! The internal RNG is `StdRng` — `rand`'s documented reproducible RNG.
//! `thread_rng` deliberately is not; swapping to `thread_rng` would
//! silently introduce non-determinism.

use parking_lot::Mutex;
use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};

use overdrive_core::traits::entropy::Entropy;

/// Deterministic entropy source seeded from a single `u64`.
///
/// `SimEntropy` is `Send + Sync` so it can be handed to async tasks
/// that live across `.await` points. The interior mutex makes `u64()`
/// thread-safe; contention is irrelevant for DST workloads where every
/// tick is sequential.
pub struct SimEntropy {
    rng: Mutex<StdRng>,
}

impl SimEntropy {
    /// Construct a seeded entropy source. Two `SimEntropy` instances
    /// constructed with the same seed produce identical sequences of
    /// `u64()` / `fill` output.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self { rng: Mutex::new(StdRng::seed_from_u64(seed)) }
    }
}

impl Entropy for SimEntropy {
    fn u64(&self) -> u64 {
        self.rng.lock().next_u64()
    }

    fn fill(&self, buf: &mut [u8]) {
        self.rng.lock().fill_bytes(buf);
    }
}

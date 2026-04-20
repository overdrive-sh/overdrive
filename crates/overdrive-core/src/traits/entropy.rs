//! [`Entropy`] — the sole source of randomness for Overdrive logic.
//!
//! Production wires this to the OS RNG (`getrandom`); DST wires it to a
//! seeded `StdRng` so every "random" value is reproducible from the test seed.
//! `rand::random()` and `rand::thread_rng()` are forbidden outside wiring
//! crates.

pub trait Entropy: Send + Sync + 'static {
    /// A uniformly random `u64`.
    fn u64(&self) -> u64;

    /// Fill `buf` with random bytes.
    fn fill(&self, buf: &mut [u8]);
}

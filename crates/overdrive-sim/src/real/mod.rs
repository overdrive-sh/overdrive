//! Real (production) adapters — feature-gated under `real-adapters`.
//!
//! **Placement decision** (step 05-01): the real adapters live here
//! under a feature-gated module of `overdrive-sim` rather than in a
//! sibling `overdrive-adapters-real` crate. The rationale is twofold:
//!
//! 1. No consumer of the real adapters exists yet. Phase 1 builds the
//!    trait boundaries; the node agent, control plane, and gateway
//!    (all future crates) are the only call sites for the `Real*`
//!    types. Proliferating an empty sibling crate today buys nothing.
//! 2. The dst-lint gate (step 05-02) scans by crate, via the
//!    `package.metadata.overdrive.crate_class` key. `overdrive-sim` is
//!    classed `adapter-sim`, so dst-lint does NOT flag this module.
//!    Once the future `overdrive-node` crate lands, the `Real*` types
//!    migrate out and `overdrive-node` carries `crate_class =
//!    "wiring"`. Migration is a move, not a rewrite.
//!
//! These implementations are minimal — just enough for `cargo check
//! --features real-adapters` to succeed. Phase 2 wires them up and
//! fleshes them out; Phase 1 only needs to prove the trait surface
//! has a real impl *somewhere*.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use rand::RngCore;

use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::entropy::Entropy;

/// Production clock. Wraps `std::time::Instant::now` and
/// `tokio::time::sleep`.
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

/// Production entropy source. Uses `rand::rngs::OsRng` — the OS RNG —
/// which `rand` documents as cryptographically secure.
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

/// Production transport.
///
/// Phase 2 wires this to `tokio::net::*` via a small adapter layer.
/// The Phase-1 shape is a placeholder that satisfies the `Transport`
/// trait surface so callers can depend on the type name.
#[derive(Debug, Default)]
pub struct TcpTransport {
    _private: (),
}

#[async_trait]
impl overdrive_core::traits::transport::Transport for TcpTransport {
    async fn connect(
        &self,
        addr: std::net::SocketAddr,
    ) -> Result<
        Box<dyn overdrive_core::traits::transport::Connection>,
        overdrive_core::traits::transport::TransportError,
    > {
        Err(overdrive_core::traits::transport::TransportError::Connect {
            addr,
            source: std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "TcpTransport::connect — Phase 2 wires this to tokio::net",
            ),
        })
    }

    async fn send_datagram(
        &self,
        addr: std::net::SocketAddr,
        _payload: bytes::Bytes,
    ) -> Result<usize, overdrive_core::traits::transport::TransportError> {
        Err(overdrive_core::traits::transport::TransportError::Connect {
            addr,
            source: std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "TcpTransport::send_datagram — Phase 2 wires this to tokio::net",
            ),
        })
    }
}

/// Tiny sanity holder — counts every entropy pull so test suites can
/// verify that a given path uses a real adapter rather than a sim.
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

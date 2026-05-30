//! Sim bindings for the three Prober port traits.
//!
//! Per ADR-0054 §2 + `.claude/rules/development.md` § "Production
//! code is not shaped by simulation": sim adapters are queue-driven
//! outcome injection.
//!
//! Production uses real sockets / hyper / Command; neither side
//! imposes structural concessions on the other.
//!
//! Per ADR-0059 §2 / DDD-17: [`SimExecProber`] does NOT assert
//! cgroup membership — that's a Tier 3 concern. Membership is the
//! production-adapter contract.
//!
//! Sim adapters MUST validate inputs identically to the production
//! adapter per `nw-tdd-methodology` § "Integration Test Contract:
//! Test Doubles Must Validate Inputs". A [`SimTcpProber`] that
//! accepts a zero-port input that `TokioTcpProber` would reject is
//! a wiring-bug-hider.

#![allow(dead_code)]
#![allow(
    clippy::missing_const_for_fn,
    reason = "constructors take a `Mutex<...>` field which is not const-constructible in stable Rust 1.85"
)]

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::traits::prober::{
    ExecProber, HttpProber, ProbeFailure, ProbeOutcome, TcpProber,
};

/// Queue-driven [`TcpProber`] sim binding.
///
/// Outcomes are pushed by the harness via
/// [`SimTcpProber::enqueue_outcome`]; the trait's `probe()` pops
/// the front of the queue. If the queue is empty, returns
/// `ProbeOutcome::Pass` (the conservative happy-path default the
/// harness can override by enqueueing a `Fail`).
pub struct SimTcpProber {
    queue: Mutex<VecDeque<ProbeOutcome>>,
    probe_count: AtomicU64,
}

impl SimTcpProber {
    #[must_use]
    pub fn new() -> Self {
        Self { queue: Mutex::new(VecDeque::new()), probe_count: AtomicU64::new(0) }
    }

    /// Enqueue the outcome the next `probe()` call will return.
    /// Outcomes are FIFO — the property test
    /// `SimProberFifoIsObserved` (slice-01 acceptance suite) pins
    /// this invariant.
    ///
    /// # Panics
    ///
    /// Panics if the internal queue mutex is poisoned — which only
    /// happens if a previous `probe()` panicked while holding the
    /// lock. The trait's `probe()` body holds the lock for one
    /// `pop_front` call with no fallible operation between
    /// acquisition and release, so poisoning is not reachable
    /// through correct test usage.
    pub fn enqueue_outcome(&self, outcome: ProbeOutcome) {
        self.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push_back(outcome);
    }

    /// Total number of `probe()` invocations since construction.
    pub fn probe_call_count(&self) -> u64 {
        self.probe_count.load(Ordering::Relaxed)
    }
}

impl Default for SimTcpProber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TcpProber for SimTcpProber {
    async fn probe(
        &self,
        host: &str,
        port: u16,
        _timeout: Duration,
    ) -> Result<ProbeOutcome, ProbeFailure> {
        if host.is_empty() {
            return Err(ProbeFailure::InvalidTarget {
                reason: "tcp probe host must be non-empty".to_string(),
            });
        }
        if port == 0 {
            return Err(ProbeFailure::InvalidTarget {
                reason: "tcp probe port must be in 1..=65535".to_string(),
            });
        }
        self.probe_count.fetch_add(1, Ordering::Relaxed);
        let mut guard = self.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(guard.pop_front().unwrap_or(ProbeOutcome::Pass))
    }
}

/// Queue-driven [`HttpProber`] sim binding.
pub struct SimHttpProber {
    queue: Mutex<VecDeque<ProbeOutcome>>,
}

impl SimHttpProber {
    #[must_use]
    pub fn new() -> Self {
        Self { queue: Mutex::new(VecDeque::new()) }
    }

    /// Enqueue the outcome the next `probe()` call will return.
    pub fn enqueue_outcome(&self, outcome: ProbeOutcome) {
        self.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push_back(outcome);
    }
}

impl Default for SimHttpProber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HttpProber for SimHttpProber {
    async fn probe(&self, url: &str, _timeout: Duration) -> Result<ProbeOutcome, ProbeFailure> {
        if url.is_empty() {
            return Err(ProbeFailure::InvalidTarget {
                reason: "http probe url must be non-empty".to_string(),
            });
        }
        if !url.starts_with("http://") {
            return Err(ProbeFailure::InvalidTarget {
                reason: format!("http probe url must start with `http://`; got {url:?}"),
            });
        }
        let mut guard = self.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(guard.pop_front().unwrap_or(ProbeOutcome::Pass))
    }
}

/// Queue-driven [`ExecProber`] sim binding.
///
/// Does NOT assert cgroup membership (per ADR-0059 §2 — that's a
/// Tier 3 concern, asserted by the production-adapter integration
/// test).
pub struct SimExecProber {
    queue: Mutex<VecDeque<ProbeOutcome>>,
}

impl SimExecProber {
    #[must_use]
    pub fn new() -> Self {
        Self { queue: Mutex::new(VecDeque::new()) }
    }

    /// Enqueue the outcome the next `probe()` call will return.
    pub fn enqueue_outcome(&self, outcome: ProbeOutcome) {
        self.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push_back(outcome);
    }
}

impl Default for SimExecProber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExecProber for SimExecProber {
    async fn probe(
        &self,
        command: &[String],
        cgroup_scope_path: &str,
        _timeout: Duration,
    ) -> Result<ProbeOutcome, ProbeFailure> {
        if command.is_empty() {
            return Err(ProbeFailure::InvalidTarget {
                reason: "exec probe command must be non-empty".to_string(),
            });
        }
        if cgroup_scope_path.is_empty() {
            return Err(ProbeFailure::InvalidTarget {
                reason: "exec probe cgroup_scope_path must be non-empty".to_string(),
            });
        }
        let mut guard = self.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(guard.pop_front().unwrap_or(ProbeOutcome::Pass))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "test code per workspace convention")]
mod tests {
    //! Proptest invariants for the `SimProber` FIFO contract per
    //! AC#6 (slice-01).

    use proptest::prelude::*;

    use super::*;

    fn arb_probe_outcome() -> impl Strategy<Value = ProbeOutcome> {
        prop_oneof![
            Just(ProbeOutcome::Pass),
            "[a-zA-Z0-9 :]{0,30}".prop_map(|reason| ProbeOutcome::Fail { reason }),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// S-SHCP-FIFO-01 — for arbitrary outcome sequences,
        /// dequeue order equals enqueue order through [`SimTcpProber`].
        #[test]
        fn sim_tcp_prober_fifo_is_observed(
            outcomes in prop::collection::vec(arb_probe_outcome(), 1..=16),
        ) {
            let prober = SimTcpProber::new();
            for outcome in &outcomes {
                prober.enqueue_outcome(outcome.clone());
            }
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let observed: Vec<ProbeOutcome> = runtime.block_on(async {
                let mut acc = Vec::with_capacity(outcomes.len());
                for _ in 0..outcomes.len() {
                    let result = prober
                        .probe("127.0.0.1", 8080, Duration::from_millis(1))
                        .await
                        .expect("sim prober inputs valid");
                    acc.push(result);
                }
                acc
            });
            prop_assert_eq!(observed, outcomes);
        }
    }
}

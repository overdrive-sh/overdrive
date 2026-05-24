//! Sim bindings for the three Prober port traits.
//!
//! Per ADR-0054 §2 + `.claude/rules/development.md` § "Production
//! code is not shaped by simulation": sim adapters are
//! queue-driven outcome injection. Production uses real sockets /
//! hyper / Command; neither side imposes structural concessions
//! on the other.
//!
//! Per ADR-0059 §2 / DDD-17: `SimExecProber` does NOT assert
//! cgroup membership — that's a Tier 3 concern. Membership is the
//! production-adapter contract (asserted by
//! `crates/overdrive-worker/tests/integration/exec_probe_cgroup_
//! membership.rs`).
//!
//! Sim adapters MUST validate inputs identically to the production
//! adapter per `nw-tdd-methodology` § "Integration Test Contract:
//! Test Doubles Must Validate Inputs". A `SimTcpProber` that
//! accepts a zero-port input that `TokioTcpProber` would reject is
//! a wiring-bug-hider.
//!
//! RED scaffold — outcome-queue + injection API land in slice 01.
// SCAFFOLD: true

#![allow(
    clippy::too_long_first_doc_paragraph,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_value,
    reason = "RED scaffold; GREEN bodies in slice-01..03 introduce Mutex<VecDeque>, consume outcomes, and prevent the const-fn classification"
)]
#![allow(dead_code)]
#![expect(clippy::todo, reason = "RED scaffold; lands GREEN in slice-01")]

use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::traits::prober::{
    ExecProber, HttpProber, ProbeFailure, ProbeOutcome, TcpProber,
};

/// Queue-driven `TcpProber` sim binding. Outcomes are pushed by
/// the harness via `enqueue_outcome`; the trait's `probe()` pops
/// the front of the queue. If empty, returns a configured default
/// (typically `Pass` for happy-path tests, `Fail { "connection
/// refused" }` for sad-path).
pub struct SimTcpProber;

impl SimTcpProber {
    pub fn new() -> Self {
        Self
    }

    /// Enqueue the outcome the next `probe()` call will return.
    pub fn enqueue_outcome(&self, outcome: ProbeOutcome) {
        let _ = outcome;
        todo!(
            "RED scaffold: SimTcpProber::enqueue_outcome — Mutex<VecDeque<ProbeOutcome>> in slice-01"
        )
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
        timeout: Duration,
    ) -> Result<ProbeOutcome, ProbeFailure> {
        let _ = (host, port, timeout);
        todo!("RED scaffold: SimTcpProber::probe — pop from outcome queue in slice-01")
    }
}

/// Queue-driven `HttpProber` sim binding.
pub struct SimHttpProber;

impl SimHttpProber {
    pub fn new() -> Self {
        Self
    }

    pub fn enqueue_outcome(&self, outcome: ProbeOutcome) {
        let _ = outcome;
        todo!(
            "RED scaffold: SimHttpProber::enqueue_outcome — Mutex<VecDeque<ProbeOutcome>> in slice-02"
        )
    }
}

impl Default for SimHttpProber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HttpProber for SimHttpProber {
    async fn probe(&self, url: &str, timeout: Duration) -> Result<ProbeOutcome, ProbeFailure> {
        let _ = (url, timeout);
        todo!("RED scaffold: SimHttpProber::probe — pop from outcome queue in slice-02")
    }
}

/// Queue-driven `ExecProber` sim binding. Does NOT assert cgroup
/// membership (per ADR-0059 §2 — that's a Tier 3 concern, asserted
/// by the production-adapter integration test).
pub struct SimExecProber;

impl SimExecProber {
    pub fn new() -> Self {
        Self
    }

    pub fn enqueue_outcome(&self, outcome: ProbeOutcome) {
        let _ = outcome;
        todo!(
            "RED scaffold: SimExecProber::enqueue_outcome — Mutex<VecDeque<ProbeOutcome>> in slice-03"
        )
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
        timeout: Duration,
    ) -> Result<ProbeOutcome, ProbeFailure> {
        let _ = (command, cgroup_scope_path, timeout);
        todo!("RED scaffold: SimExecProber::probe — pop from outcome queue in slice-03")
    }
}

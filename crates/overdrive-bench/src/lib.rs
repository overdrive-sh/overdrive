//! `overdrive-bench` — the measurement core (ADR-0296, ADR-0300, ADR-0301).
//!
//! RED scaffold created by DISTILL: only the public surface pinned by the
//! DESIGN interface contract (DDD-6) exists, every behaviour is absent. The
//! acceptance tests under `tests/` fail against it for the behaviour they assert.
// SCAFFOLD: true

#![allow(
    clippy::missing_const_for_fn,
    clippy::unused_self,
    clippy::needless_pass_by_value,
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::unnecessary_wraps,
    clippy::needless_pass_by_ref_mut,
    clippy::unnecessary_literal_bound,
    clippy::use_self,
    clippy::trivially_copy_pass_by_ref,
    clippy::unused_async,
    unused_variables,
    reason = "DISTILL RED scaffold: bodies are intentionally empty until DELIVER"
)]

use std::process::ExitCode;

mod canary;
mod histogram;
pub mod record;
pub mod report;
mod stats;

#[cfg(test)]
use overdrive_testing::overdrive_test_default;

#[cfg(test)]
overdrive_test_default!(host_pure);

/// Whether a run is maintained-lane evidence or a `spike-scratch/` probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evidence {
    Maintained,
    Spike,
}

/// One declared benchmark (the macro's registration entry).
pub struct BenchmarkDecl {}

/// Entry point of a benchmark binary: maps [`Error`] to the exit statuses of DDD-9.
pub fn run_main(evidence: Evidence) -> ExitCode {
    eprintln!("RED scaffold: overdrive_bench::run_main is not implemented");
    ExitCode::from(70)
}

/// The smoke variant: one replicate per arm, no statistics held, no timing asserted.
pub fn smoke(decl: &BenchmarkDecl) {}

/// Crate error; every variant names its cause.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("usage: {reason}")]
    Usage { reason: String },
    #[error("family `{family}` is not declared (found: {found:?})")]
    FamilyNotDeclared { family: String, found: Vec<String> },
    #[error("setting `{setting}` is `{found}`, pinned `{pinned}`")]
    Refused { setting: String, found: String, pinned: String },
    #[error("setting `{setting}` is unreadable")]
    Unreadable {
        setting: String,
        #[source]
        source: std::io::Error,
    },
    #[error("enabling BPF statistics failed")]
    StatsEnable {
        #[source]
        source: aya::sys::SyscallError,
    },
    #[error("record {path:?} failed")]
    Record {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid record: line {line}: {reason}")]
    InvalidRecord { line: usize, reason: String },
}

/// Result alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A replicate's handle (opaque).
pub struct Replicate {}

impl Replicate {
    pub fn arm(&self) -> &str {
        ""
    }
    pub fn profile(&self) -> &str {
        ""
    }
    pub fn clock(&self) -> &HostClock {
        &HostClock {}
    }
    pub fn record(&mut self, metric: &str, value: f64) -> Result<(), ReplicateFailure> {
        Ok(())
    }
    pub fn record_series(&mut self, metric: &str, values: &[f64]) -> Result<(), ReplicateFailure> {
        Ok(())
    }
    pub fn record_failure(&mut self, metric: &str, reason: &str) -> Result<(), ReplicateFailure> {
        Ok(())
    }
    pub fn record_absent(&mut self, metric: &str, reason: &str) -> Result<(), ReplicateFailure> {
        Ok(())
    }
    pub fn note(&mut self, key: &str, value: &str) {}
    pub fn exec(&mut self, program: &str, args: &[&str]) -> Result<String, ReplicateFailure> {
        Ok(String::new())
    }
    pub fn bpf_stats(&self) -> Option<&BpfStatsHold> {
        None
    }
}

/// Host monotonic clock (`CLOCK_MONOTONIC_RAW`).
pub struct HostClock {}

impl HostClock {
    pub fn now(&self) -> HostInstant {
        HostInstant {}
    }
}

/// A host instant.
pub struct HostInstant {}

impl HostInstant {
    pub fn duration_since(self, earlier: HostInstant) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}

/// The descriptor-scoped BPF statistics hold.
pub struct BpfStatsHold {}

impl BpfStatsHold {
    pub fn read(&self, program_id: u32) -> Result<RunStats, StatsError> {
        Ok(RunStats { run_count: 0, run_time_ns: 0 })
    }
}

/// Cumulative run statistics; callers take deltas.
pub struct RunStats {
    pub run_count: u64,
    pub run_time_ns: u64,
}

/// A statistics read failure.
#[derive(Debug, thiserror::Error)]
#[error("BPF statistics read failed")]
pub struct StatsError {}

/// A failed replicate.
#[derive(Debug)]
pub struct ReplicateFailure {}

impl ReplicateFailure {
    pub fn fixture(msg: &str) -> Self {
        Self {}
    }
    pub fn measurement(msg: &str) -> Self {
        Self {}
    }
}

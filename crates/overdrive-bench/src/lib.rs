//! `overdrive-bench` — the measurement core (ADR-0296, ADR-0300, ADR-0301).
//!
//! The public surface is the DDD-6 contract: [`run_main`], [`smoke`], [`Replicate`] and its
//! companions, [`Error`] and the record and report modules. The schedule, statistics, canary,
//! host eligibility and runner are internal structure.

use std::process::ExitCode;

mod args;
mod canary;
mod histogram;
mod host;
mod marker;
pub mod record;
#[doc(hidden)]
pub mod registry;
mod replicate;
pub mod report;
mod runner;
mod schedule;
mod stats;
mod stats_hold;

pub use registry::BenchmarkDecl;
pub use replicate::{HostClock, HostInstant, Replicate, ReplicateFailure, smoke};
pub use stats_hold::{BpfStatsHold, RunStats, StatsError};

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

/// Entry point of a benchmark binary: maps [`Error`] to the exit statuses of DDD-9.
pub fn run_main(evidence: Evidence) -> ExitCode {
    runner::main(evidence)
}

/// Crate error; every variant names its cause.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("usage: {reason}")]
    Usage { reason: String },
    #[error(
        "family `{family}` is not declared (found: {found:?}); the usual cause is a bench `main` that does not reference the declaring crate with `use <crate> as _;`, so its registrations were not linked"
    )]
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
    #[error("cannot write benchmark record {}: {source}", path.display())]
    Record {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the loopback canary failed: {source}")]
    Canary {
        #[source]
        source: std::io::Error,
    },
    #[error("cannot write the result-directory line to stdout: {source}")]
    Stdout {
        #[source]
        source: std::io::Error,
    },
    #[error("invalid record: line {line}: {reason}")]
    InvalidRecord { line: usize, reason: String },
}

/// Result alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

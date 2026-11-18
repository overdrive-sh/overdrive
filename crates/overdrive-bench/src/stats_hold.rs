//! Per-process BPF run statistics (DDD-8).
//!
//! The runner asks the kernel for statistics with `BPF_ENABLE_STATS` and holds the returned
//! descriptor for the run: the kernel's global key is raised while any descriptor is open and
//! lowered when the last closes, including when the process is killed. The sysctl
//! `kernel.bpf_stats_enabled` is never written.

use std::os::fd::OwnedFd;
use std::sync::Mutex;

use aya::programs::{ProgramError, loaded_programs};
use aya::sys::{Stats, SyscallError, enable_stats};

/// Enables per-process statistics; the one production adapter is `aya::sys::enable_stats`.
pub trait StatsEnabler {
    fn enable(&self) -> Result<OwnedFd, SyscallError>;
}

/// `BPF_ENABLE_STATS(BPF_STATS_RUN_TIME)` through aya.
pub struct AyaStatsEnabler;

impl StatsEnabler for AyaStatsEnabler {
    fn enable(&self) -> Result<OwnedFd, SyscallError> {
        enable_stats(Stats::RunTime)
    }
}

/// The descriptor-scoped BPF statistics hold: dropping it releases the statistics.
pub struct BpfStatsHold {
    _descriptor: OwnedFd,
    reads: Mutex<Vec<(u32, RunStats)>>,
}

/// Cumulative run statistics of one program; callers take deltas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunStats {
    pub run_count: u64,
    pub run_time_ns: u64,
}

/// A statistics read that could not be completed.
#[derive(Debug, thiserror::Error)]
#[error("BPF statistics of program {program_id} could not be read")]
pub struct StatsError {
    program_id: u32,
    #[source]
    source: Option<ProgramError>,
}

impl BpfStatsHold {
    pub(crate) const fn new(descriptor: OwnedFd) -> Self {
        Self { _descriptor: descriptor, reads: Mutex::new(Vec::new()) }
    }

    /// The cumulative statistics of the loaded program `program_id`.
    pub fn read(&self, program_id: u32) -> Result<RunStats, StatsError> {
        let mut failure = None;
        for program in loaded_programs() {
            match program {
                Ok(info) if info.id() == program_id => {
                    let stats = RunStats {
                        run_count: info.run_count(),
                        run_time_ns: u64::try_from(info.run_time().as_nanos()).unwrap_or(u64::MAX),
                    };
                    self.reads
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push((program_id, stats));
                    return Ok(stats);
                }
                Ok(_) => {}
                Err(error) => failure = Some(error),
            }
        }
        Err(StatsError { program_id, source: failure })
    }

    /// The reads made since the last call, oldest first.
    pub(crate) fn take_reads(&self) -> Vec<(u32, RunStats)> {
        std::mem::take(&mut *self.reads.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}

/// The first program whose statistics were read at least twice in `reads` and whose `run_count`
/// did not move: statistics were requested but are not effective (DDD-8).
pub fn ineffective_program(reads: &[(u32, RunStats)]) -> Option<u32> {
    reads.iter().find_map(|(program, _)| {
        let mut of_program =
            reads.iter().filter(|(other, _)| other == program).map(|(_, stats)| stats);
        let first = of_program.next()?;
        let last = of_program.next_back()?;
        (last.run_count == first.run_count).then_some(*program)
    })
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "tests assert their own preconditions and compare exact stored floats"
)]
mod tests {
    use super::*;

    fn stats(run_count: u64) -> RunStats {
        RunStats { run_count, run_time_ns: run_count * 100 }
    }

    #[test]
    fn a_program_whose_run_count_never_moved_is_ineffective() {
        assert_eq!(ineffective_program(&[(7, stats(5)), (7, stats(5))]), Some(7));
        assert_eq!(
            ineffective_program(&[(7, stats(5)), (9, stats(1)), (9, stats(40)), (7, stats(5))]),
            Some(7)
        );
    }

    #[test]
    fn moving_counters_and_single_reads_are_effective() {
        assert_eq!(ineffective_program(&[(7, stats(5)), (7, stats(6))]), None);
        assert_eq!(ineffective_program(&[(7, stats(5))]), None, "one read has no delta");
        assert_eq!(ineffective_program(&[]), None);
    }

    #[test]
    fn reads_are_drained_by_take() {
        let hold = BpfStatsHold::new(std::fs::File::open("/dev/null").expect("open").into());
        hold.reads.lock().expect("lock").push((1, stats(1)));
        assert_eq!(hold.take_reads(), vec![(1, stats(1))]);
        assert!(hold.take_reads().is_empty());
    }
}

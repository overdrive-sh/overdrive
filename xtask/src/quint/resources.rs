//! Resource-aware admission for `cargo xtask quint check`.
//!
//! Every check reserves memory and CPUs according to its backend before it
//! starts, and starts only when its reservation fits the remaining budget:
//!
//! - **TLC.** Quint 0.32 launches TLC as `java <maxHeap> -Xss515m … tlc2.TLC
//!   -workers <workers> …` with `maxHeap = -Xmx8G` and `workers = auto` unless
//!   `quint verify --tlc-config <json>` supplies `{"maxHeap", "workers"}`
//!   (`quint/src/tlc.ts`). The runner always passes that file, so each TLC
//!   JVM gets the capped heap and worker count reserved here. Quint also keeps
//!   an Apalache server alive for the whole TLC check (it compiles the spec
//!   to TLA+), whose heap is capped separately ([`COMPILE_HEAP_MB`]).
//!   Reservation: TLC heap + compile-server heap + JVM non-heap for both +
//!   Quint; CPUs = TLC workers.
//! - **Apalache.** The Apalache server JVM's heap is set through `JVM_ARGS`
//!   (honoured by the Apalache launcher); Z3 runs single-threaded as native
//!   memory inside that process. Reservation: server heap + JVM non-heap +
//!   Z3 native allowance + Quint; CPUs = 1 (a dedicated CPU for Z3, so a
//!   timeout measures the check, not contention).
//!
//! The budget defaults to total memory minus [`DEFAULT_HEADROOM_MB`] and the
//! CPU count; `--jobs` remains an upper bound on concurrent checks.

use std::fmt;

use super::{Backend, Check};

/// Default heap of each TLC JVM (Quint's own default is 8 GiB).
pub const DEFAULT_TLC_HEAP_MB: u64 = 2048;
/// Default TLC worker threads (and CPUs reserved) per TLC check.
///
/// Memory admits about two TLC checks at once on the 8-CPU Lima VM, so four workers
/// each keep the CPUs busy.
pub const DEFAULT_TLC_WORKERS: u32 = 4;
/// Default heap of the Apalache server JVM of an Apalache check.
///
/// Measured on
/// `guest-flow-owner` with a 4 GiB cap: whole-check peak RSS ≤ 1.65 GiB.
pub const DEFAULT_APALACHE_HEAP_MB: u64 = 3072;
/// Heap of the Apalache server Quint keeps up during a TLC check.
///
/// It compiles the spec to TLA+ and stays resident while TLC runs. Measured on the
/// largest `guest-flow-owner` module: 1024 MiB fails with
/// `OutOfMemoryError`, 1536 MiB succeeds.
pub const COMPILE_HEAP_MB: u64 = 2048;
/// Non-heap memory of one JVM (metaspace, code cache, thread stacks, GC).
/// Measured: a 2048 MiB-heap server peaked at 2251 MiB RSS.
pub const JVM_OVERHEAD_MB: u64 = 384;
/// Native memory allowed for Z3 inside an Apalache server.
pub const Z3_NATIVE_MB: u64 = 1024;
/// Resident memory of the Quint (Deno) process on the TLC path, which holds
/// the compiled TLA+ module. Measured peak: 1845 MiB.
pub const QUINT_TLC_MB: u64 = 2048;
/// Resident memory of the Quint process on the Apalache path. Measured peak:
/// about 480 MiB.
pub const QUINT_APALACHE_MB: u64 = 512;
/// Memory left unbudgeted for the kernel, page cache and everything else.
pub const DEFAULT_HEADROOM_MB: u64 = 2048;
/// How long the head of the queue may be bypassed by smaller checks before
/// admission becomes strictly in-order (so a large check cannot starve).
pub const HEAD_BYPASS_SECS: u64 = 60;
/// Smallest heap a check may request.
const MIN_HEAP_MB: u64 = 256;

/// Memory and CPUs held by one running check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reservation {
    /// Resident memory in MiB.
    pub mem_mb: u64,
    /// CPUs.
    pub cpus: u32,
}

impl Reservation {
    /// Component-wise sum.
    #[must_use]
    pub const fn plus(self, other: Self) -> Self {
        Self { mem_mb: self.mem_mb + other.mem_mb, cpus: self.cpus + other.cpus }
    }

    /// Whether `self` fits within `budget`.
    #[must_use]
    pub const fn fits(self, budget: Self) -> bool {
        self.mem_mb <= budget.mem_mb && self.cpus <= budget.cpus
    }
}

impl fmt::Display for Reservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} MiB / {} CPU", self.mem_mb, self.cpus)
    }
}

/// Errors from parsing a resource option.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResourceError {
    /// A size is not `<n>`, `<n>m` or `<n>g`.
    #[error("invalid size `{0}`: expected <n> (MiB), <n>m or <n>g")]
    Size(String),
    /// A heap below the minimum.
    #[error("heap {0} MiB is below the {MIN_HEAP_MB} MiB minimum")]
    HeapTooSmall(u64),
}

/// Parse a memory size in MiB: `4096`, `4096m`, `4g` (case-insensitive).
pub fn parse_size_mb(s: &str) -> Result<u64, ResourceError> {
    let t = s.trim().to_ascii_lowercase();
    let (digits, mult) = match t.as_bytes().last() {
        Some(b'g') => (&t[..t.len() - 1], 1024),
        Some(b'm') => (&t[..t.len() - 1], 1),
        _ => (t.as_str(), 1),
    };
    let n: u64 = digits.parse().map_err(|_| ResourceError::Size(s.to_owned()))?;
    let mb = n.checked_mul(mult).ok_or_else(|| ResourceError::Size(s.to_owned()))?;
    if mb == 0 {
        return Err(ResourceError::Size(s.to_owned()));
    }
    Ok(mb)
}

/// Parse a heap size, rejecting heaps below the minimum.
pub fn parse_heap_mb(s: &str) -> Result<u64, ResourceError> {
    let mb = parse_size_mb(s)?;
    if mb < MIN_HEAP_MB {
        return Err(ResourceError::HeapTooSmall(mb));
    }
    Ok(mb)
}

/// Validate a heap given in MiB (per-check `heap_mb`).
pub const fn validate_heap_mb(mb: u64) -> Result<u64, ResourceError> {
    if mb < MIN_HEAP_MB { Err(ResourceError::HeapTooSmall(mb)) } else { Ok(mb) }
}

/// Run-wide resource settings (CLI options over the defaults).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceConfig {
    /// Total budget all running checks share.
    pub budget: Reservation,
    /// Default TLC heap (MiB) for checks without `heap_mb`.
    pub tlc_heap_mb: u64,
    /// Default TLC workers for checks without `workers`.
    pub tlc_workers: u32,
    /// Default Apalache server heap (MiB) for checks without `heap_mb`.
    pub apalache_heap_mb: u64,
}

/// Overrides from the command line; `None` keeps the default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResourceOverrides {
    /// `--mem-budget` (MiB).
    pub mem_budget_mb: Option<u64>,
    /// `--cpu-budget`.
    pub cpu_budget: Option<u32>,
    /// `--tlc-heap` (MiB).
    pub tlc_heap_mb: Option<u64>,
    /// `--tlc-workers`.
    pub tlc_workers: Option<u32>,
    /// `--apalache-heap` (MiB).
    pub apalache_heap_mb: Option<u64>,
}

impl ResourceConfig {
    /// Resolve the configuration from the overrides and the detected host.
    ///
    /// The default memory budget is total memory minus
    /// [`DEFAULT_HEADROOM_MB`] (at least one Apalache check's reservation
    /// when memory is unknown or tiny, so the run can always proceed); the
    /// default CPU budget is the CPU count.
    pub fn resolve(o: ResourceOverrides, cpus: usize, mem_total_mb: Option<u64>) -> Self {
        let tlc_heap_mb = o.tlc_heap_mb.unwrap_or(DEFAULT_TLC_HEAP_MB);
        let tlc_workers = o.tlc_workers.unwrap_or(DEFAULT_TLC_WORKERS).max(1);
        let apalache_heap_mb = o.apalache_heap_mb.unwrap_or(DEFAULT_APALACHE_HEAP_MB);
        let floor = apalache_reservation(apalache_heap_mb).mem_mb;
        let mem_mb = o.mem_budget_mb.unwrap_or_else(|| {
            mem_total_mb.map_or(floor, |m| m.saturating_sub(DEFAULT_HEADROOM_MB).max(floor))
        });
        let cpus = o.cpu_budget.unwrap_or_else(|| u32::try_from(cpus).unwrap_or(u32::MAX)).max(1);
        Self { budget: Reservation { mem_mb, cpus }, tlc_heap_mb, tlc_workers, apalache_heap_mb }
    }

    /// The heap and worker count a check runs with.
    pub fn sizing(&self, check: &Check) -> Sizing {
        match check.backend {
            Backend::Tlc => Sizing {
                heap_mb: check.heap_mb.unwrap_or(self.tlc_heap_mb),
                workers: check.workers.unwrap_or(self.tlc_workers),
            },
            Backend::Apalache => {
                Sizing { heap_mb: check.heap_mb.unwrap_or(self.apalache_heap_mb), workers: 1 }
            }
        }
    }

    /// What a check reserves while it runs.
    pub fn reservation(&self, check: &Check) -> Reservation {
        let s = self.sizing(check);
        match check.backend {
            Backend::Tlc => tlc_reservation(s.heap_mb, s.workers),
            Backend::Apalache => apalache_reservation(s.heap_mb),
        }
    }
}

/// Heap and workers of one check's main JVM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sizing {
    /// TLC heap, or Apalache server heap (MiB).
    pub heap_mb: u64,
    /// TLC workers; 1 for Apalache.
    pub workers: u32,
}

/// Reservation of a TLC check.
pub const fn tlc_reservation(heap_mb: u64, workers: u32) -> Reservation {
    Reservation {
        mem_mb: heap_mb + JVM_OVERHEAD_MB + COMPILE_HEAP_MB + JVM_OVERHEAD_MB + QUINT_TLC_MB,
        cpus: workers,
    }
}

/// Reservation of an Apalache check.
pub const fn apalache_reservation(heap_mb: u64) -> Reservation {
    Reservation { mem_mb: heap_mb + JVM_OVERHEAD_MB + Z3_NATIVE_MB + QUINT_APALACHE_MB, cpus: 1 }
}

/// Which queued check (index into `queue`) to start next, if any.
///
/// - Nothing starts once `running` reaches `max_jobs`.
/// - With nothing running the head always starts, even when it exceeds the
///   budget on its own (it could never fit otherwise).
/// - The head starts when it fits next to `in_use`.
/// - Otherwise a later check that fits may start ahead of it, but only while
///   the head has been blocked for less than `bypass_bound`; after that the
///   queue is strictly in order until the head starts.
pub fn admit(
    queue: &[Reservation],
    in_use: Reservation,
    running: usize,
    max_jobs: usize,
    budget: Reservation,
    head_blocked_for: std::time::Duration,
    bypass_bound: std::time::Duration,
) -> Option<usize> {
    if queue.is_empty() || running >= max_jobs {
        return None;
    }
    if running == 0 || in_use.plus(queue[0]).fits(budget) {
        return Some(0);
    }
    if head_blocked_for >= bypass_bound {
        return None;
    }
    queue.iter().skip(1).position(|r| in_use.plus(*r).fits(budget)).map(|i| i + 1)
}

/// Start order: longest previous duration first; checks without history are
/// treated as longest. Ties keep manifest order.
pub fn longest_first(history: &[Option<f64>]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..history.len()).collect();
    order.sort_by(|&a, &b| {
        let key = |i: usize| history[i].unwrap_or(f64::INFINITY);
        key(b).total_cmp(&key(a)).then(a.cmp(&b))
    });
    order
}

/// Process group and resident pages from a `/proc/<pid>/stat` line.
pub fn parse_stat_pgrp_rss(stat: &str) -> Option<(i32, u64)> {
    // Fields after `comm`: state(3) ppid(4) pgrp(5) … rss(24).
    let rest = stat.get(stat.rfind(')')? + 1..)?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let pgrp = fields.get(2)?.parse().ok()?;
    let rss = fields.get(21)?.parse().ok()?;
    Some((pgrp, rss))
}

/// `MemTotal` and `MemAvailable` (MiB) from `/proc/meminfo`.
pub fn parse_meminfo(info: &str) -> (Option<u64>, Option<u64>) {
    let field = |name: &str| {
        info.lines().find_map(|l| l.strip_prefix(name)).and_then(|v| {
            v.trim().trim_end_matches("kB").trim().parse::<u64>().ok().map(|kb| kb / 1024)
        })
    };
    (field("MemTotal:"), field("MemAvailable:"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::super::{Expect, Property};
    use super::*;

    fn check(backend: Backend, heap_mb: Option<u64>, workers: Option<u32>) -> Check {
        Check {
            name: "c".into(),
            spec: PathBuf::from("a.qnt"),
            main: "m".into(),
            property: Property::Invariant("I".into()),
            backend,
            max_steps: None,
            expect: Expect::Holds,
            ci: false,
            timeout_secs: None,
            heap_mb,
            workers,
        }
    }

    const fn r(mem_mb: u64, cpus: u32) -> Reservation {
        Reservation { mem_mb, cpus }
    }

    #[test]
    fn parse_size_accepts_mib_and_gib_suffixes() {
        assert_eq!(parse_size_mb("4096"), Ok(4096));
        assert_eq!(parse_size_mb("512m"), Ok(512));
        assert_eq!(parse_size_mb("512M"), Ok(512));
        assert_eq!(parse_size_mb(" 4g "), Ok(4096));
        assert_eq!(parse_size_mb("2G"), Ok(2048));
        for bad in ["", "g", "4k", "-1", "1.5g", "0", "0g", "x4g", "99999999999999999999g"] {
            assert_eq!(parse_size_mb(bad), Err(ResourceError::Size(bad.into())), "{bad}");
        }
    }

    #[test]
    fn heap_has_a_minimum() {
        assert_eq!(parse_heap_mb("256m"), Ok(256));
        assert_eq!(parse_heap_mb("255"), Err(ResourceError::HeapTooSmall(255)));
        assert_eq!(validate_heap_mb(256), Ok(256));
        assert_eq!(validate_heap_mb(100), Err(ResourceError::HeapTooSmall(100)));
    }

    #[test]
    fn default_budget_is_memory_minus_headroom_and_cpu_count() {
        let c = ResourceConfig::resolve(ResourceOverrides::default(), 8, Some(15948));
        assert_eq!(c.budget, r(15948 - DEFAULT_HEADROOM_MB, 8));
        assert_eq!(c.tlc_heap_mb, DEFAULT_TLC_HEAP_MB);
        assert_eq!(c.tlc_workers, DEFAULT_TLC_WORKERS);
        assert_eq!(c.apalache_heap_mb, DEFAULT_APALACHE_HEAP_MB);
        // Unknown or tiny memory still admits one Apalache check.
        let floor = apalache_reservation(DEFAULT_APALACHE_HEAP_MB).mem_mb;
        assert_eq!(
            ResourceConfig::resolve(ResourceOverrides::default(), 1, None).budget.mem_mb,
            floor
        );
        assert_eq!(
            ResourceConfig::resolve(ResourceOverrides::default(), 1, Some(1024)).budget.mem_mb,
            floor
        );
    }

    #[test]
    fn overrides_win_over_defaults() {
        let o = ResourceOverrides {
            mem_budget_mb: Some(1000),
            cpu_budget: Some(3),
            tlc_heap_mb: Some(3072),
            tlc_workers: Some(4),
            apalache_heap_mb: Some(2048),
        };
        let c = ResourceConfig::resolve(o, 8, Some(15948));
        assert_eq!(c.budget, r(1000, 3));
        assert_eq!((c.tlc_heap_mb, c.tlc_workers, c.apalache_heap_mb), (3072, 4, 2048));
    }

    #[test]
    fn reservation_by_backend_and_per_check_override() {
        let c = ResourceConfig::resolve(ResourceOverrides::default(), 8, Some(15948));
        let tlc = check(Backend::Tlc, None, None);
        assert_eq!(c.sizing(&tlc), Sizing { heap_mb: 2048, workers: 4 });
        assert_eq!(c.reservation(&tlc), r(2048 + 384 + 2048 + 384 + 2048, 4));
        let big = check(Backend::Tlc, Some(6144), Some(2));
        assert_eq!(c.reservation(&big), r(6144 + 384 + 2048 + 384 + 2048, 2));
        let apa = check(Backend::Apalache, None, None);
        assert_eq!(c.sizing(&apa), Sizing { heap_mb: 3072, workers: 1 });
        assert_eq!(c.reservation(&apa), r(3072 + 384 + 1024 + 512, 1));
        assert_eq!(c.reservation(&check(Backend::Apalache, Some(2048), None)).mem_mb, 3968);
    }

    #[test]
    fn default_reservations_fill_the_lima_vm_without_overcommit() {
        // 15948 MiB / 8 CPUs: two TLC checks (all 8 CPUs), or two Apalache
        // checks, or one of each — never three TLC 8 GiB heaps as before.
        let c = ResourceConfig::resolve(ResourceOverrides::default(), 8, Some(15948));
        let (t, a) = (tlc_reservation(2048, 4), apalache_reservation(3072));
        assert!(t.plus(t).fits(c.budget));
        assert!(a.plus(a).fits(c.budget));
        assert!(a.plus(t).fits(c.budget));
        assert!(!t.plus(t).plus(a).fits(c.budget));
        assert!(!a.plus(a).plus(a).fits(c.budget));
    }

    const NO_WAIT: Duration = Duration::ZERO;
    const BOUND: Duration = Duration::from_secs(HEAD_BYPASS_SECS);

    #[test]
    fn admit_respects_budget_jobs_and_empty_queue() {
        let budget = r(10_000, 8);
        assert_eq!(admit(&[], r(0, 0), 0, 4, budget, NO_WAIT, BOUND), None);
        assert_eq!(admit(&[r(4000, 2)], r(0, 0), 0, 4, budget, NO_WAIT, BOUND), Some(0));
        assert_eq!(admit(&[r(4000, 2)], r(6000, 2), 1, 4, budget, NO_WAIT, BOUND), Some(0));
        // Memory exactly at the budget fits; one MiB over does not.
        assert_eq!(admit(&[r(4001, 2)], r(6000, 2), 1, 4, budget, BOUND, BOUND), None);
        // CPUs bound independently of memory.
        assert_eq!(admit(&[r(100, 3)], r(100, 6), 1, 4, budget, BOUND, BOUND), None);
        assert_eq!(admit(&[r(100, 2)], r(100, 6), 1, 4, budget, BOUND, BOUND), Some(0));
        // --jobs caps the count even when resources remain.
        assert_eq!(admit(&[r(1, 1)], r(2, 2), 2, 2, budget, NO_WAIT, BOUND), None);
    }

    #[test]
    fn admit_starts_an_oversized_check_alone() {
        let budget = r(10_000, 8);
        assert_eq!(admit(&[r(20_000, 16)], r(0, 0), 0, 4, budget, NO_WAIT, BOUND), Some(0));
        assert_eq!(admit(&[r(20_000, 16)], r(1, 1), 1, 4, budget, BOUND, BOUND), None);
    }

    #[test]
    fn admit_bypasses_a_blocked_head_only_within_the_bound() {
        let budget = r(10_000, 8);
        let queue = [r(6000, 1), r(9000, 1), r(3000, 2)];
        let in_use = r(5000, 1);
        assert_eq!(admit(&queue, in_use, 1, 4, budget, NO_WAIT, BOUND), Some(2));
        assert_eq!(
            admit(&queue, in_use, 1, 4, budget, Duration::from_secs(HEAD_BYPASS_SECS - 1), BOUND),
            Some(2)
        );
        // Once the head has waited the bound, nothing overtakes it.
        assert_eq!(admit(&queue, in_use, 1, 4, budget, BOUND, BOUND), None);
        // Nothing fits: nothing starts.
        assert_eq!(admit(&[r(6000, 1), r(5001, 1)], in_use, 1, 4, budget, NO_WAIT, BOUND), None);
    }

    #[test]
    fn longest_first_puts_unknown_first_and_keeps_ties_stable() {
        assert_eq!(
            longest_first(&[Some(5.0), None, Some(900.0), Some(5.0), None]),
            vec![1, 4, 2, 0, 3]
        );
        assert!(longest_first(&[]).is_empty());
    }

    #[test]
    fn parses_stat_pgrp_and_rss() {
        let stat = "515209 (java) S 515141 515141 490704 0 -1 1077936384 1 2 3 4 5 6 7 8 20 0 \
                    30 0 9 1 634701 0 0";
        assert_eq!(parse_stat_pgrp_rss(stat), Some((515_141, 634_701)));
        // `comm` with spaces and parentheses.
        let odd = "7 (a b) c)) S 1 42 42 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 5 0 77 0";
        assert_eq!(parse_stat_pgrp_rss(odd), Some((42, 77)));
        assert_eq!(parse_stat_pgrp_rss("1 (x) S 1"), None);
        assert_eq!(parse_stat_pgrp_rss("garbage"), None);
    }

    #[test]
    fn parses_meminfo() {
        let info = "MemTotal:       16330752 kB\nMemFree:  1 kB\nMemAvailable:    8148992 kB\n";
        assert_eq!(parse_meminfo(info), (Some(15948), Some(7958)));
        assert_eq!(parse_meminfo(""), (None, None));
    }
}

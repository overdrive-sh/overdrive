//! The resource cost of one sustained window of the `mtls-path` family, read from `/proc`.
//!
//! Whole-host CPU comes from the aggregate line of `/proc/stat`: interrupt and kernel-worker
//! time is charged outside any benchmark cgroup, so a host-wide figure is the honest scope.
//! The forward and return pump work comes from `/proc/self/task/*/stat`, per thread name, with
//! the family's own threads (the fixture prefix) left out of the process total. The kernel
//! accounts thread CPU in 10 ms ticks, so a pump lighter than one tick reads as zero and the
//! heavy-to-light ratio is a lower bound.

use std::collections::BTreeMap;
use std::io::ErrorKind;

/// The name the outbound forward (userspace encrypt) pump thread carries (DDD-14).
pub(super) const FORWARD_PUMP: &str = "mtls-out-fwd";
/// The name the outbound return (kTLS-RX splice) pump thread carries (DDD-14).
pub(super) const RETURN_PUMP: &str = "mtls-out-ret";

/// The aggregate CPU counters of `/proc/stat`, in clock ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HostCpu {
    user: u64,
    system: u64,
    irq: u64,
    softirq: u64,
    /// Every accounted class, idle included; guest time is already inside `user`.
    total: u64,
}

/// The share of whole-host CPU capacity each class used over a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct HostRatios {
    pub(super) user: f64,
    pub(super) system: f64,
    pub(super) irq: f64,
    pub(super) softirq: f64,
}

/// The pump threads' share of the process's non-fixture CPU over a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PumpSplit {
    pub(super) forward: f64,
    pub(super) ret: f64,
    /// Heavy thread ticks over light thread ticks, the light one counted as at least one tick:
    /// a lower bound, because the light pump is below the tick resolution.
    pub(super) heavy_to_light_lower_bound: f64,
}

/// `/proc/stat` and the threads of this process at one instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Snapshot {
    host: HostCpu,
    /// Thread id to name and user plus system ticks.
    threads: BTreeMap<u32, (String, u64)>,
}

/// The aggregate `cpu` line of `/proc/stat`: user (with nice), system, irq, softirq and the
/// total. Fields beyond the seventh (steal and later) are optional on old kernels.
pub(super) fn parse_host_cpu(stat: &str) -> Option<HostCpu> {
    let line = stat.lines().next()?;
    let mut fields = line.split_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let counters: Vec<u64> = fields.map(str::parse).collect::<Result<_, _>>().ok()?;
    let at = |index: usize| counters.get(index).copied();
    let (user, nice, system, idle) = (at(0)?, at(1)?, at(2)?, at(3)?);
    let (iowait, irq, softirq) = (at(4)?, at(5)?, at(6)?);
    let steal = at(7).unwrap_or(0);
    Some(HostCpu {
        user: user.checked_add(nice)?,
        system,
        irq,
        softirq,
        total: [user, nice, system, idle, iowait, irq, softirq, steal]
            .into_iter()
            .try_fold(0u64, u64::checked_add)?,
    })
}

/// The thread name and `utime + stime` (ticks) of one `/proc/<pid>/task/<tid>/stat`. The name
/// is everything between the first `(` and the last `)`, so it may hold spaces and parentheses.
pub(super) fn parse_thread_stat(stat: &str) -> Option<(String, u64)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let name = stat.get(open + 1..close)?;
    let rest: Vec<&str> = stat.get(close + 1..)?.split_whitespace().collect();
    // After the name: state is field 3, utime field 14, stime field 15.
    let utime: u64 = rest.get(11)?.parse().ok()?;
    let stime: u64 = rest.get(12)?.parse().ok()?;
    Some((name.to_owned(), utime.checked_add(stime)?))
}

/// `VmRSS` of a `/proc/self/status`, in bytes.
pub(super) fn parse_resident_bytes(status: &str) -> Option<u64> {
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

impl Snapshot {
    /// Reads `/proc/stat` and every thread of this process. A thread that exits between the
    /// listing and the read is skipped; any other read failure is an error.
    pub(super) fn take() -> Result<Self, String> {
        let stat = std::fs::read_to_string("/proc/stat")
            .map_err(|error| format!("reading /proc/stat: {error}"))?;
        let host = parse_host_cpu(&stat).ok_or("the cpu line of /proc/stat is malformed")?;
        let mut threads = BTreeMap::new();
        let listing = std::fs::read_dir("/proc/self/task")
            .map_err(|error| format!("listing /proc/self/task: {error}"))?;
        for entry in listing {
            let entry = entry.map_err(|error| format!("listing /proc/self/task: {error}"))?;
            let Some(tid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            let path = format!("/proc/self/task/{tid}/stat");
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("reading {path}: {error}")),
            };
            let thread = parse_thread_stat(&text).ok_or_else(|| format!("{path} is malformed"))?;
            threads.insert(tid, thread);
        }
        Ok(Self { host, threads })
    }
}

/// The share of whole-host CPU capacity each class used between two snapshots.
pub(super) fn host_ratios(before: &Snapshot, after: &Snapshot) -> Result<HostRatios, String> {
    let delta = |after: u64, before: u64, what: &str| {
        after.checked_sub(before).ok_or_else(|| format!("the host {what} counter went backwards"))
    };
    let total = delta(after.host.total, before.host.total, "total cpu")?;
    if total == 0 {
        return Err("no host CPU time elapsed between the snapshots".to_owned());
    }
    let share = |after: u64, before: u64, what: &str| {
        delta(after, before, what).map(|ticks| ticks as f64 / total as f64)
    };
    Ok(HostRatios {
        user: share(after.host.user, before.host.user, "user")?,
        system: share(after.host.system, before.host.system, "system")?,
        irq: share(after.host.irq, before.host.irq, "irq")?,
        softirq: share(after.host.softirq, before.host.softirq, "softirq")?,
    })
}

/// The forward and return pump threads' share of the CPU the process's other threads used
/// between two snapshots, leaving out every thread whose name starts with `fixture_prefix`.
/// A thread present only in `after` counts from zero.
pub(super) fn pump_split(
    before: &Snapshot,
    after: &Snapshot,
    fixture_prefix: &str,
) -> Result<PumpSplit, String> {
    let mut process = 0u64;
    let mut forward = 0u64;
    let mut ret = 0u64;
    let mut forward_threads = 0u32;
    let mut return_threads = 0u32;
    for (tid, (name, ticks)) in &after.threads {
        if name.starts_with(fixture_prefix) {
            continue;
        }
        let earlier = before.threads.get(tid).map_or(0, |(_, ticks)| *ticks);
        let gained = ticks.saturating_sub(earlier);
        process = process.saturating_add(gained);
        match name.as_str() {
            FORWARD_PUMP => {
                forward = forward.saturating_add(gained);
                forward_threads += 1;
            }
            RETURN_PUMP => {
                ret = ret.saturating_add(gained);
                return_threads += 1;
            }
            _ => {}
        }
    }
    if forward_threads == 0 || return_threads == 0 {
        return Err(format!(
            "the process holds no `{FORWARD_PUMP}` and `{RETURN_PUMP}` thread pair at the end of the window"
        ));
    }
    if process == 0 {
        return Err("no thread outside the fixture accrued a CPU tick in the window".to_owned());
    }
    let (heavy, light) = (forward.max(ret), forward.min(ret).max(1));
    Ok(PumpSplit {
        forward: forward as f64 / process as f64,
        ret: ret as f64 / process as f64,
        heavy_to_light_lower_bound: heavy as f64 / light as f64,
    })
}

/// Resident set size of this process, in bytes.
pub(super) fn resident_bytes() -> Result<u64, String> {
    let status = std::fs::read_to_string("/proc/self/status")
        .map_err(|error| format!("reading /proc/self/status: {error}"))?;
    parse_resident_bytes(&status)
        .ok_or_else(|| "VmRSS is missing from /proc/self/status".to_owned())
}

/// Sockets this process holds open: its descriptors that name a socket.
pub(super) fn open_sockets() -> Result<u64, String> {
    let listing = std::fs::read_dir("/proc/self/fd")
        .map_err(|error| format!("listing /proc/self/fd: {error}"))?;
    let mut sockets = 0u64;
    for entry in listing {
        let entry = entry.map_err(|error| format!("listing /proc/self/fd: {error}"))?;
        match std::fs::read_link(entry.path()) {
            Ok(target) if target.to_string_lossy().starts_with("socket:") => sockets += 1,
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(format!("reading a descriptor of this process: {error}")),
        }
    }
    Ok(sockets)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::cast_precision_loss,
    reason = "tests assert their own preconditions and compare ratios built from the same integers"
)]
mod tests {
    //! The `/proc` readers over crafted text: which field feeds which class, what a malformed
    //! line does, and how the pump split leaves the fixture's threads out.

    use super::*;
    use overdrive_testing::placement;

    placement!(host_pure);

    const PREFIX: &str = "ovbm-";

    fn host(user: u64, system: u64, irq: u64, softirq: u64, total: u64) -> HostCpu {
        HostCpu { user, system, irq, softirq, total }
    }

    fn snapshot(host: HostCpu, threads: &[(u32, &str, u64)]) -> Snapshot {
        Snapshot {
            host,
            threads: threads
                .iter()
                .map(|(tid, name, ticks)| (*tid, ((*name).to_owned(), *ticks)))
                .collect(),
        }
    }

    #[test]
    fn host_cpu_folds_nice_into_user_and_leaves_guest_out_of_the_total() {
        let parsed = parse_host_cpu("cpu  10 5 20 100 7 3 2 1 50 40\ncpu0 1 1 1 1 1 1 1 1 1 1\n");
        assert_eq!(parsed, Some(host(15, 20, 3, 2, 10 + 5 + 20 + 100 + 7 + 3 + 2 + 1)));
    }

    #[test]
    fn host_cpu_accepts_a_kernel_without_steal() {
        assert_eq!(parse_host_cpu("cpu 1 2 3 4 5 6 7"), Some(host(3, 3, 6, 7, 28)));
    }

    #[test]
    fn host_cpu_refuses_a_short_a_foreign_or_a_non_numeric_line() {
        assert_eq!(parse_host_cpu("cpu 1 2 3 4 5 6"), None);
        assert_eq!(parse_host_cpu("cpu0 1 2 3 4 5 6 7 8"), None);
        assert_eq!(parse_host_cpu("cpu 1 2 3 4 5 6 x"), None);
        assert_eq!(parse_host_cpu(""), None);
    }

    #[test]
    fn host_ratios_are_each_class_delta_over_the_total_delta() {
        let before = snapshot(host(100, 50, 5, 5, 1000), &[]);
        let after = snapshot(host(140, 90, 15, 25, 1200), &[]);
        let ratios = host_ratios(&before, &after).unwrap();
        assert_eq!(ratios, HostRatios { user: 0.2, system: 0.2, irq: 0.05, softirq: 0.1 });
    }

    #[test]
    fn host_ratios_refuse_a_window_with_no_elapsed_time_or_a_backwards_counter() {
        let first = snapshot(host(1, 1, 1, 1, 100), &[]);
        assert!(host_ratios(&first, &first).is_err());
        let earlier = snapshot(host(1, 1, 1, 1, 90), &[]);
        assert!(host_ratios(&first, &earlier).is_err());
        let fewer_user = snapshot(host(0, 1, 1, 1, 200), &[]);
        assert!(host_ratios(&first, &fewer_user).is_err());
    }

    #[test]
    fn thread_stat_reads_a_name_with_spaces_and_parentheses() {
        let line = "42 (tokio (rt) w) S 1 2 3 4 5 6 7 8 9 10 111 22 0 0 20 0 1 0 5";
        assert_eq!(parse_thread_stat(line), Some(("tokio (rt) w".to_owned(), 133)));
    }

    #[test]
    fn thread_stat_refuses_a_truncated_line() {
        assert_eq!(parse_thread_stat("42 (x) S 1 2 3"), None);
        assert_eq!(parse_thread_stat("no name here"), None);
    }

    #[test]
    fn resident_bytes_is_the_vmrss_line_in_bytes() {
        let status = "Name:\tx\nVmHWM:\t 9000 kB\nVmRSS:\t   2048 kB\nThreads:\t3\n";
        assert_eq!(parse_resident_bytes(status), Some(2048 * 1024));
        assert_eq!(parse_resident_bytes("Name:\tx\n"), None);
    }

    #[test]
    fn pump_split_leaves_the_fixture_threads_out_of_the_process_total() {
        let before = snapshot(
            host(0, 0, 0, 0, 1),
            &[
                (1, FORWARD_PUMP, 100),
                (2, RETURN_PUMP, 0),
                (3, "ovbm-client", 10),
                (4, "tokio-rt-worker", 0),
            ],
        );
        let after = snapshot(
            host(0, 0, 0, 0, 2),
            &[
                (1, FORWARD_PUMP, 190),
                (2, RETURN_PUMP, 1),
                (3, "ovbm-client", 500),
                (4, "tokio-rt-worker", 9),
            ],
        );
        let split = pump_split(&before, &after, PREFIX).unwrap();
        assert_eq!(split.forward, 90.0 / 100.0);
        assert_eq!(split.ret, 1.0 / 100.0);
        assert_eq!(split.heavy_to_light_lower_bound, 90.0);
    }

    #[test]
    fn a_light_pump_below_one_tick_is_counted_as_one_in_the_lower_bound() {
        let before = snapshot(host(0, 0, 0, 0, 1), &[(1, FORWARD_PUMP, 0), (2, RETURN_PUMP, 7)]);
        let after = snapshot(host(0, 0, 0, 0, 2), &[(1, FORWARD_PUMP, 250), (2, RETURN_PUMP, 7)]);
        let split = pump_split(&before, &after, PREFIX).unwrap();
        assert_eq!(split.ret, 0.0);
        assert_eq!(split.heavy_to_light_lower_bound, 250.0);
    }

    #[test]
    fn a_thread_new_in_the_window_counts_from_zero_and_the_split_is_order_free() {
        let before = snapshot(host(0, 0, 0, 0, 1), &[(9, "tokio-rt-worker", 0)]);
        let after = snapshot(
            host(0, 0, 0, 0, 2),
            &[(9, "tokio-rt-worker", 0), (30, RETURN_PUMP, 4), (20, FORWARD_PUMP, 36)],
        );
        let split = pump_split(&before, &after, PREFIX).unwrap();
        assert_eq!((split.forward, split.ret), (0.9, 0.1));
    }

    #[test]
    fn the_split_refuses_a_process_with_no_pump_pair_or_no_accrued_work() {
        let none = snapshot(host(0, 0, 0, 0, 1), &[(1, "tokio-rt-worker", 5)]);
        assert!(pump_split(&none, &none, PREFIX).is_err());
        let one = snapshot(host(0, 0, 0, 0, 1), &[(1, FORWARD_PUMP, 5)]);
        assert!(pump_split(&one, &one, PREFIX).is_err());
        let idle = snapshot(host(0, 0, 0, 0, 1), &[(1, FORWARD_PUMP, 5), (2, RETURN_PUMP, 5)]);
        assert!(pump_split(&idle, &idle, PREFIX).is_err());
    }
}

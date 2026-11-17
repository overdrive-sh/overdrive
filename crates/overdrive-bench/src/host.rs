//! The host the run measures: reading its settings and deciding whether it is eligible (DDD-7).
//!
//! The [`HostProbe`] port is read-only. The runner reads every setting before deciding, so a
//! refused run still carries the full `host` block; [`HostReading::decide`] names the first
//! difference in the pinned order. A pinned setting with no source cannot be verified, so it is a
//! difference; a source that exists and cannot be read is `Error::Unreadable`; a failed probe
//! refuses as `host_probe` with every `host` key unreadable because nothing was read.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

use crate::record::{HOST_KEYS, HostValue};
use crate::{Error, Result};

const PINNED_KERNEL_SERIES: &str = "7.0";
const PINNED_BPF_STATS_ENABLED: &str = "0";
const PINNED_GOVERNOR: &str = "schedutil";
const PINNED_SMT: &str = "on";
const PINNED_BOOST: &str = "on";
const PINNED_NO_HOLDER: &str = "none";
const PINNED_PROBE: &str = "probe succeeds";
const PINNED_DEBUG_ASSERTIONS: &str = "off";

const OSRELEASE: &str = "/proc/sys/kernel/osrelease";
const BPF_STATS_ENABLED: &str = "/proc/sys/kernel/bpf_stats_enabled";
const CPU_ONLINE: &str = "/sys/devices/system/cpu/online";
const SMT_CONTROL: &str = "/sys/devices/system/cpu/smt/control";
const CPUFREQ_BOOST: &str = "/sys/devices/system/cpu/cpufreq/boost";
const INTEL_NO_TURBO: &str = "/sys/devices/system/cpu/intel_pstate/no_turbo";
const CPUINFO: &str = "/proc/cpuinfo";
const CLOCKSOURCE: &str = "/sys/devices/system/clocksource/clocksource0/current_clocksource";
/// The target a `BPF_ENABLE_STATS` descriptor renders as (`kernel/bpf/syscall.c:6014`,
/// `fs/anon_inodes.c:76` @ v7.0).
const BPF_STATS_TARGET: &str = "anon_inode:bpf-stats";

/// Read-only access to the host's settings (no write method exists).
pub trait HostProbe {
    /// The probe's own self-check: it can see what a setting read relies on.
    fn probe(&self) -> io::Result<()>;
    /// The text of a procfs or sysfs file; `NotFound` means the source does not exist.
    fn read(&self, path: &str) -> io::Result<String>;
    /// The pid of a process holding a `bpf-stats` descriptor, if any.
    fn bpf_stats_holder(&self) -> io::Result<Option<u32>>;
}

/// The procfs and sysfs reader of the running host.
pub struct ProcSysProbe;

impl HostProbe for ProcSysProbe {
    fn probe(&self) -> io::Result<()> {
        // The descriptor scan is the setting a missing /proc would silently blind: prove the
        // scan can enumerate processes, and that this process's own descriptors are visible.
        std::fs::read_dir("/proc/self/fd")?.try_for_each(|entry| entry.map(drop))
    }

    fn read(&self, path: &str) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn bpf_stats_holder(&self) -> io::Result<Option<u32>> {
        for process in std::fs::read_dir("/proc")? {
            let entry = process?;
            let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            if holds_bpf_stats(&entry.path().join("fd"))? {
                return Ok(Some(pid));
            }
        }
        Ok(None)
    }
}

/// Whether any descriptor in `fd_dir` is a `bpf-stats` one. A process that exits mid-scan is not
/// an error; any other failure to read a process's descriptors is.
fn holds_bpf_stats(fd_dir: &PathBuf) -> io::Result<bool> {
    let descriptors = match std::fs::read_dir(fd_dir) {
        Ok(descriptors) => descriptors,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    for descriptor in descriptors {
        let Ok(descriptor) = descriptor else { continue };
        let Ok(target) = std::fs::read_link(descriptor.path()) else { continue };
        if target.as_os_str() == BPF_STATS_TARGET {
            return Ok(true);
        }
    }
    Ok(false)
}

/// What reading one setting found.
enum Setting {
    Text(String),
    Absent(String),
    Unreadable(io::Error),
}

impl Setting {
    fn of(read: io::Result<String>, absent_reason: &str) -> Self {
        match read {
            Ok(text) => Self::Text(text.trim().to_owned()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Self::Absent(absent_reason.to_owned())
            }
            Err(error) => Self::Unreadable(error),
        }
    }

    fn host_value(&self) -> HostValue {
        match self {
            Self::Text(text) => HostValue::Text(text.clone()),
            Self::Absent(reason) => HostValue::Absent { absent: reason.clone() },
            Self::Unreadable(error) => HostValue::Unreadable { unreadable: error.to_string() },
        }
    }

    fn unreadable_copy(error: &io::Error) -> Self {
        Self::Unreadable(io::Error::new(error.kind(), error.to_string()))
    }
}

/// Everything the runner read from the host before deciding eligibility.
pub struct HostReading {
    settings: BTreeMap<&'static str, Setting>,
    /// `Err` when the probe failed (nothing else was read).
    probe: std::result::Result<(), io::Error>,
    holder: Option<std::result::Result<Option<u32>, io::Error>>,
}

impl HostReading {
    /// Probe, then (only when the probe succeeds) read every setting.
    pub fn read(probe: &dyn HostProbe) -> Self {
        if let Err(error) = probe.probe() {
            let settings = HOST_KEYS
                .iter()
                .map(|key| (*key, Setting::unreadable_copy(&error)))
                .collect();
            return Self { settings, probe: Err(error), holder: None };
        }
        let online = online_cpus(probe);
        let mut settings = BTreeMap::new();
        settings.insert("kernel", Setting::of(probe.read(OSRELEASE), "no kernel release source"));
        settings.insert(
            "cpu_model",
            cpu_model(probe.read(CPUINFO)),
        );
        settings.insert("cpu_count", count(&online));
        settings.insert("governor", governor(probe, &online));
        settings.insert("smt", Setting::of(probe.read(SMT_CONTROL), "no smt control"));
        settings.insert("boost", boost(probe));
        settings.insert(
            "bpf_stats_enabled",
            Setting::of(probe.read(BPF_STATS_ENABLED), "no bpf_stats_enabled source"),
        );
        settings.insert(
            "clocksource",
            Setting::of(probe.read(CLOCKSOURCE), "no clocksource source"),
        );
        settings.insert("irq_time_accounting", irq_time_accounting(probe, &settings));
        Self { settings, probe: Ok(()), holder: Some(probe.bpf_stats_holder()) }
    }

    /// The `host` block of `run_started`: every key, always.
    pub fn host_block(&self) -> BTreeMap<String, HostValue> {
        HOST_KEYS
            .iter()
            .filter_map(|key| {
                self.settings.get(key).map(|setting| ((*key).to_owned(), setting.host_value()))
            })
            .collect()
    }

    /// The first difference from the pinned list, in the DDD-7 order.
    pub fn decide(self, debug_assertions: bool) -> Result<()> {
        if let Err(error) = self.probe {
            return Err(Error::Refused {
                setting: "host_probe".to_owned(),
                found: error.to_string(),
                pinned: PINNED_PROBE.to_owned(),
            });
        }
        let Self { mut settings, holder, .. } = self;
        let mut pinned = |key: &'static str, pinned: &str, same: &dyn Fn(&str) -> bool| {
            match settings.remove(key) {
                Some(Setting::Text(found)) if same(&found) => Ok(()),
                Some(Setting::Text(found)) => refused(key, &found, pinned),
                Some(Setting::Unreadable(source)) => {
                    Err(Error::Unreadable { setting: key.to_owned(), source })
                }
                Some(Setting::Absent(_)) | None => refused(key, "absent", pinned),
            }
        };
        pinned("kernel", PINNED_KERNEL_SERIES, &|found| kernel_series(found) == PINNED_KERNEL_SERIES)?;
        pinned("bpf_stats_enabled", PINNED_BPF_STATS_ENABLED, &|found| found == PINNED_BPF_STATS_ENABLED)?;
        pinned("governor", PINNED_GOVERNOR, &|found| found == PINNED_GOVERNOR)?;
        pinned("smt", PINNED_SMT, &|found| found == PINNED_SMT)?;
        pinned("boost", PINNED_BOOST, &|found| found == PINNED_BOOST)?;
        match holder {
            Some(Ok(None)) | None => {}
            Some(Ok(Some(pid))) => {
                return refused("bpf_stats_holder", &format!("held by pid {pid}"), PINNED_NO_HOLDER);
            }
            Some(Err(source)) => {
                return Err(Error::Unreadable { setting: "bpf_stats_holder".to_owned(), source });
            }
        }
        if debug_assertions {
            return refused("debug_assertions", "on", PINNED_DEBUG_ASSERTIONS);
        }
        Ok(())
    }
}

fn refused(setting: &str, found: &str, pinned: &str) -> Result<()> {
    Err(Error::Refused {
        setting: setting.to_owned(),
        found: found.to_owned(),
        pinned: pinned.to_owned(),
    })
}

/// `7.0.0-38-generic` is series `7.0`.
fn kernel_series(release: &str) -> String {
    let mut parts = release.split('.');
    let major = parts.next().unwrap_or_default();
    let minor: String =
        parts.next().unwrap_or_default().chars().take_while(char::is_ascii_digit).collect();
    format!("{major}.{minor}")
}

/// The online CPU numbers, parsed from a list such as `0-3,6`.
fn online_cpus(probe: &dyn HostProbe) -> std::result::Result<Vec<u32>, io::Error> {
    let text = probe.read(CPU_ONLINE)?;
    let malformed = || io::Error::new(io::ErrorKind::InvalidData, format!("malformed CPU list `{}`", text.trim()));
    let mut cpus = Vec::new();
    for part in text.trim().split(',').filter(|part| !part.is_empty()) {
        let (first, last) = part.split_once('-').unwrap_or((part, part));
        let first: u32 = first.parse().map_err(|_| malformed())?;
        let last: u32 = last.parse().map_err(|_| malformed())?;
        cpus.extend(first..=last);
    }
    Ok(cpus)
}

fn count(online: &std::result::Result<Vec<u32>, io::Error>) -> Setting {
    match online {
        Ok(cpus) => Setting::Text(cpus.len().to_string()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Setting::Absent("no online CPU list".to_owned())
        }
        Err(error) => Setting::unreadable_copy(error),
    }
}

/// The governor shared by every online CPU, or the distinct governors sorted and joined by `,`.
fn governor(probe: &dyn HostProbe, online: &std::result::Result<Vec<u32>, io::Error>) -> Setting {
    let cpus = match online {
        Ok(cpus) => cpus,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Setting::Absent("no online CPU list".to_owned());
        }
        Err(error) => return Setting::unreadable_copy(error),
    };
    let mut governors = std::collections::BTreeSet::new();
    for cpu in cpus {
        let path = format!("/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor");
        match Setting::of(probe.read(&path), "no cpufreq governor") {
            Setting::Text(name) => {
                governors.insert(name);
            }
            other => return other,
        }
    }
    if governors.is_empty() {
        return Setting::Absent("no online CPU".to_owned());
    }
    Setting::Text(governors.into_iter().collect::<Vec<_>>().join(","))
}

/// CPU boost as `on` or `off`, from `cpufreq/boost`, else `intel_pstate/no_turbo`.
fn boost(probe: &dyn HostProbe) -> Setting {
    let flag = |text: &str, on: &str| if text == on { "on" } else { "off" }.to_owned();
    match Setting::of(probe.read(CPUFREQ_BOOST), "") {
        Setting::Text(text) => Setting::Text(flag(&text, "1")),
        Setting::Unreadable(error) => Setting::Unreadable(error),
        Setting::Absent(_) => match Setting::of(probe.read(INTEL_NO_TURBO), "") {
            Setting::Text(text) => Setting::Text(flag(&text, "0")),
            Setting::Unreadable(error) => Setting::Unreadable(error),
            Setting::Absent(_) => Setting::Absent("no cpufreq/boost or intel_pstate/no_turbo".to_owned()),
        },
    }
}

fn cpu_model(cpuinfo: io::Result<String>) -> Setting {
    match Setting::of(cpuinfo, "no /proc/cpuinfo") {
        Setting::Text(text) => text
            .lines()
            .find_map(|line| line.strip_prefix("model name"))
            .and_then(|rest| rest.split_once(':'))
            .map_or_else(
                || Setting::Absent("no `model name` in /proc/cpuinfo".to_owned()),
                |(_, model)| Setting::Text(model.trim().to_owned()),
            ),
        other => other,
    }
}

/// `CONFIG_IRQ_TIME_ACCOUNTING` of the running kernel's image configuration, where readable.
fn irq_time_accounting(probe: &dyn HostProbe, read: &BTreeMap<&'static str, Setting>) -> Setting {
    let Some(Setting::Text(release)) = read.get("kernel") else {
        return Setting::Absent("kernel release unknown".to_owned());
    };
    let path = format!("/boot/config-{release}");
    match Setting::of(probe.read(&path), &format!("no kernel configuration at {path}")) {
        Setting::Text(config) => {
            let value = config
                .lines()
                .find_map(|line| line.strip_prefix("CONFIG_IRQ_TIME_ACCOUNTING="))
                .unwrap_or("n");
            Setting::Text(value.to_owned())
        }
        other => other,
    }
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
pub mod tests {
    use super::*;

    /// A fixture host: path to text, a path to a failure kind, a holder, a probe verdict.
    pub struct Fixture {
        files: BTreeMap<String, String>,
        unreadable: Vec<String>,
        pub(crate) holder: Option<u32>,
        pub(crate) probe_fails: bool,
    }

    impl Fixture {
        /// A host at the pinned settings, two CPUs, boost through `cpufreq/boost`.
        pub fn eligible() -> Self {
            let files = [
                (OSRELEASE, "7.0.0-38-generic\n"),
                (BPF_STATS_ENABLED, "0\n"),
                (CPU_ONLINE, "0-1\n"),
                ("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor", "schedutil\n"),
                ("/sys/devices/system/cpu/cpu1/cpufreq/scaling_governor", "schedutil\n"),
                (SMT_CONTROL, "on\n"),
                (CPUFREQ_BOOST, "1\n"),
                (CPUINFO, "processor\t: 0\nmodel name\t: Test CPU @ 3GHz\n"),
                (CLOCKSOURCE, "tsc\n"),
                ("/boot/config-7.0.0-38-generic", "CONFIG_IRQ_TIME_ACCOUNTING=y\n"),
            ]
            .into_iter()
            .map(|(path, text)| (path.to_owned(), text.to_owned()))
            .collect();
            Self { files, unreadable: Vec::new(), holder: None, probe_fails: false }
        }

        pub fn with(mut self, path: &str, text: &str) -> Self {
            self.files.insert(path.to_owned(), text.to_owned());
            self
        }

        fn without(mut self, path: &str) -> Self {
            self.files.remove(path);
            self
        }

        fn unreadable(mut self, path: &str) -> Self {
            self.unreadable.push(path.to_owned());
            self
        }
    }

    impl HostProbe for Fixture {
        fn probe(&self) -> io::Result<()> {
            if self.probe_fails {
                return Err(io::Error::other("proc is not mounted"));
            }
            Ok(())
        }

        fn read(&self, path: &str) -> io::Result<String> {
            if self.unreadable.iter().any(|blocked| blocked == path) {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            self.files.get(path).cloned().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }

        fn bpf_stats_holder(&self) -> io::Result<Option<u32>> {
            Ok(self.holder)
        }
    }

    fn decide(fixture: &Fixture) -> Result<()> {
        HostReading::read(fixture).decide(false)
    }

    fn refusal(fixture: &Fixture) -> (String, String, String) {
        match decide(fixture) {
            Err(Error::Refused { setting, found, pinned }) => (setting, found, pinned),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn text(value: &str) -> HostValue {
        HostValue::Text(value.to_owned())
    }

    #[test]
    fn a_host_at_the_pinned_settings_is_eligible_and_its_block_has_every_key() {
        let fixture = Fixture::eligible();
        assert!(decide(&fixture).is_ok());
        let block = HostReading::read(&fixture).host_block();
        assert_eq!(block.len(), HOST_KEYS.len());
        assert_eq!(block["kernel"], text("7.0.0-38-generic"));
        assert_eq!(block["cpu_count"], text("2"));
        assert_eq!(block["governor"], text("schedutil"));
        assert_eq!(block["boost"], text("on"));
        assert_eq!(block["cpu_model"], text("Test CPU @ 3GHz"));
        assert_eq!(block["irq_time_accounting"], text("y"));
    }

    #[test]
    fn each_drift_names_its_setting_with_found_and_pinned_in_the_pinned_order() {
        let cases = [
            (Fixture::eligible().with(OSRELEASE, "6.8.0-1\n"), "kernel", "6.8.0-1", "7.0"),
            (Fixture::eligible().with(BPF_STATS_ENABLED, "1\n"), "bpf_stats_enabled", "1", "0"),
            (
                Fixture::eligible()
                    .with("/sys/devices/system/cpu/cpu1/cpufreq/scaling_governor", "performance\n"),
                "governor",
                "performance,schedutil",
                "schedutil",
            ),
            (Fixture::eligible().with(SMT_CONTROL, "off\n"), "smt", "off", "on"),
            (Fixture::eligible().with(CPUFREQ_BOOST, "0\n"), "boost", "off", "on"),
        ];
        for (fixture, setting, found, pinned) in cases {
            assert_eq!(refusal(&fixture), (setting.to_owned(), found.to_owned(), pinned.to_owned()));
        }
        let mut held = Fixture::eligible();
        held.holder = Some(4242);
        assert_eq!(
            refusal(&held),
            ("bpf_stats_holder".to_owned(), "held by pid 4242".to_owned(), "none".to_owned())
        );
        match HostReading::read(&Fixture::eligible()).decide(true) {
            Err(Error::Refused { setting, found, pinned }) => {
                assert_eq!((setting.as_str(), found.as_str(), pinned.as_str()), ("debug_assertions", "on", "off"));
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_first_difference_in_the_pinned_order_is_the_one_named() {
        let fixture = Fixture::eligible().with(SMT_CONTROL, "off\n").with(OSRELEASE, "6.1\n");
        assert_eq!(refusal(&fixture).0, "kernel");
    }

    #[test]
    fn a_pinned_setting_with_no_source_is_absent_in_the_block_and_refuses_as_absent() {
        let fixture = Fixture::eligible().without(SMT_CONTROL);
        assert_eq!(refusal(&fixture), ("smt".to_owned(), "absent".to_owned(), "on".to_owned()));
        assert!(matches!(
            HostReading::read(&fixture).host_block()["smt"],
            HostValue::Absent { .. }
        ));
    }

    #[test]
    fn boost_falls_back_to_intel_pstate_and_is_absent_when_neither_source_exists() {
        let intel = Fixture::eligible().without(CPUFREQ_BOOST).with(INTEL_NO_TURBO, "0\n");
        assert!(decide(&intel).is_ok(), "no_turbo 0 is boost on");
        let off = Fixture::eligible().without(CPUFREQ_BOOST).with(INTEL_NO_TURBO, "1\n");
        assert_eq!(refusal(&off).1, "off");
        let neither = Fixture::eligible().without(CPUFREQ_BOOST);
        assert_eq!(refusal(&neither).1, "absent");
    }

    #[test]
    fn a_pinned_source_that_cannot_be_read_refuses_as_unreadable_with_its_cause() {
        let fixture = Fixture::eligible().unreadable(SMT_CONTROL);
        match decide(&fixture) {
            Err(Error::Unreadable { setting, source }) => {
                assert_eq!(setting, "smt");
                assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
            }
            other => panic!("expected unreadable, got {other:?}"),
        }
        assert!(matches!(
            HostReading::read(&fixture).host_block()["smt"],
            HostValue::Unreadable { .. }
        ));
    }

    #[test]
    fn an_unreadable_recorded_only_source_is_recorded_and_never_refuses() {
        let fixture = Fixture::eligible().unreadable(CLOCKSOURCE).without("/boot/config-7.0.0-38-generic");
        assert!(decide(&fixture).is_ok());
        let block = HostReading::read(&fixture).host_block();
        assert!(matches!(block["clocksource"], HostValue::Unreadable { .. }));
        assert!(matches!(block["irq_time_accounting"], HostValue::Absent { .. }));
    }

    #[test]
    fn a_failed_probe_refuses_first_with_every_host_key_unreadable() {
        let mut fixture = Fixture::eligible().with(OSRELEASE, "6.1\n");
        fixture.probe_fails = true;
        assert_eq!(
            refusal(&fixture),
            ("host_probe".to_owned(), "proc is not mounted".to_owned(), "probe succeeds".to_owned())
        );
        let block = HostReading::read(&fixture).host_block();
        assert_eq!(block.len(), HOST_KEYS.len());
        assert!(block.values().all(|value| matches!(value, HostValue::Unreadable { .. })));
    }

    #[test]
    fn the_kernel_series_is_the_first_two_numeric_components() {
        assert_eq!(kernel_series("7.0.0-38-generic"), "7.0");
        assert_eq!(kernel_series("7.0"), "7.0");
        assert_eq!(kernel_series("6.18.2"), "6.18");
        assert_eq!(kernel_series("7.01.0"), "7.01");
    }

    #[test]
    fn a_cpu_list_expands_ranges_and_singles() {
        let probe = Fixture::eligible().with(CPU_ONLINE, "0-2,5\n");
        assert_eq!(online_cpus(&probe).expect("parses"), vec![0, 1, 2, 5]);
        let bad = Fixture::eligible().with(CPU_ONLINE, "a-b");
        assert!(online_cpus(&bad).is_err());
    }
}

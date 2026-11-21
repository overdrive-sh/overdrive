//! Node-local persistent volume (GH #348, ADR-0313, ADR-0314): the boot probe against a
//! guest that CARRIES the volume pool (`cgroup-default`, profile `volume-pool`), healthy
//! and then deliberately broken, one named test per scenario of ADR-0314.
//!
//! Driving port: `VolumePoolProbe::probe` on `LvmVolumePoolProbe`. Each guest is fresh, so
//! a test may damage its own pool. Sad paths are named example tests (Mandate 11).
//!
//! Every test runs in the volume-pool guest. Each healthy-pool test asserts the probe finds the
//! pool and returns a backing, and each broken-pool test asserts the probe fails with the
//! scenario of ADR-0314 that breakage exists to make.
//!
//! Evidence tier: Tier 3, real guest kernel and real `lvm`, example-only.

#![allow(clippy::doc_markdown, reason = "the CONTRACT_SHAPE line is machine-read verbatim")]

use overdrive_core::traits::volume_backing::{
    VolumePoolProbe, VolumeProbeDetail, VolumeProbeError, VolumeProbeScenario,
};
use overdrive_host::LvmVolumePoolProbe;
use overdrive_testing::overdrive_test;

use std::time::Duration;

use super::volume_pool_support::{
    free_bytes, logical_volumes, lvm, lvm_ok, lvm_processes, runtime,
};

/// `test` names the calling test in the failure message.
fn probe_error(test: &str) -> VolumeProbeError {
    runtime()
        .block_on(LvmVolumePoolProbe::new().probe())
        .err()
        .unwrap_or_else(|| panic!("a broken pool must fail its probe ({test})"))
}

/// Undoes a `mount --bind` over `target` when the test ends, even by panic.
struct BindOver(String);

impl BindOver {
    fn new(source: &str, target: &str) -> Self {
        let status = std::process::Command::new("mount")
            .args(["--bind", source, target])
            .status()
            .expect("spawn mount");
        assert!(status.success(), "bind {source} over {target}");
        Self(target.to_owned())
    }
}

impl Drop for BindOver {
    fn drop(&mut self) {
        let _ = std::process::Command::new("umount").arg(&self.0).status();
    }
}

fn lvm_binary() -> String {
    let out = std::process::Command::new("sh").args(["-c", "command -v lvm"]).output().expect("sh");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// ADR-0314: a healthy pool probes clean and yields a pool; the scratch volume it creates and
/// removes leaves no logical volume behind, and the free space is unchanged.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
fn a_healthy_pool_probes_clean_and_leaves_nothing_behind() {
    let free_before = free_bytes();
    let probed =
        runtime().block_on(LvmVolumePoolProbe::new().probe()).expect("a healthy pool probes clean");
    assert!(
        probed.is_some(),
        "the labelled partition is found (a_healthy_pool_probes_clean_and_leaves_nothing_behind)"
    );
    assert!(logical_volumes().is_empty(), "the probe's scratch volume was removed");
    assert_eq!(free_bytes(), free_before, "no extent leaked");
}

/// ADR-0314: a scratch volume left by a crashed probe (committed and inactive after a reboot)
/// is removed first, and the probe still passes.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
fn a_scratch_volume_left_by_a_crashed_probe_is_removed_first() {
    lvm_ok(&["lvcreate", "-L", "4m", "-n", "ovd-probe-leftover", "overdrive"]);
    lvm_ok(&["lvchange", "-an", "overdrive/ovd-probe-leftover"]);
    let probed = runtime()
        .block_on(LvmVolumePoolProbe::new().probe())
        .expect("the probe clears its own leftovers");
    assert!(
        probed.is_some(),
        "the labelled partition is found (a_scratch_volume_left_by_a_crashed_probe_is_removed_first)"
    );
    assert!(!logical_volumes().contains(&"ovd-probe-leftover".to_owned()), "the leftover is gone");
}

/// ADR-0314: a workload volume left inactive by a reboot is none of the probe's business: it
/// is neither activated nor removed (the launch path's `ensure` activates it).
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
fn the_probe_touches_no_workload_volume() {
    lvm_ok(&["lvcreate", "-L", "4m", "-n", "ovd-0123456789abcdef0123456789abcdef", "overdrive"]);
    lvm_ok(&["lvchange", "-an", "overdrive/ovd-0123456789abcdef0123456789abcdef"]);
    let probed =
        runtime().block_on(LvmVolumePoolProbe::new().probe()).expect("a healthy pool probes clean");
    assert!(
        probed.is_some(),
        "the labelled partition is found (the_probe_touches_no_workload_volume)"
    );
    assert_eq!(logical_volumes(), vec!["ovd-0123456789abcdef0123456789abcdef".to_owned()]);
    assert!(!super::volume_pool_support::lv_active("ovd-0123456789abcdef0123456789abcdef"));
}

/// ADR-0314: a pool with no free extent skips the scratch step (with an event) and still passes.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
fn a_pool_with_no_free_extent_still_probes_clean() {
    let free = free_bytes();
    lvm_ok(&[
        "lvcreate",
        "-L",
        &format!("{free}b"),
        "-n",
        "ovd-ffffffffffffffffffffffffffffffff",
        "overdrive",
    ]);
    assert_eq!(free_bytes(), 0);
    let probed =
        runtime().block_on(LvmVolumePoolProbe::new().probe()).expect("a full pool still probes");
    assert!(
        probed.is_some(),
        "the labelled partition is found (a_pool_with_no_free_extent_still_probes_clean)"
    );
}

/// Sad (scenario 1): the `lvm` tool is unusable.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
fn sad_an_unusable_lvm_tool_fails_the_probe_tools_or_device_mapper_unusable() {
    let _unusable = BindOver::new("/dev/null", &lvm_binary());
    let error =
        probe_error("sad_an_unusable_lvm_tool_fails_the_probe_tools_or_device_mapper_unusable");
    assert_eq!(error.scenario, VolumeProbeScenario::ToolsOrDeviceMapperUnusable, "{error}");
    assert!(
        matches!(error.detail, VolumeProbeDetail::Io(_)),
        "a binary that cannot be executed is a failed spawn (Io), got {:?}",
        error.detail
    );
}

/// Sad (scenario 2): the volume group is missing.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
fn sad_a_missing_volume_group_fails_the_probe_volume_group_missing() {
    lvm_ok(&["vgrename", "overdrive", "overdrive-renamed"]);
    let error = probe_error("sad_a_missing_volume_group_fails_the_probe_volume_group_missing");
    assert_eq!(error.scenario, VolumeProbeScenario::VolumeGroupMissing, "{error}");
    assert!(
        matches!(&error.detail, VolumeProbeDetail::CommandFailed { stderr, .. } if stderr.contains("overdrive")),
        "the detail is the failed lvm command, whose text names the group: {:?}",
        error.detail
    );
}

/// Sad (scenario 2): a label matched by more than one partition is the same scenario, and the
/// detail names the ambiguity.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
fn sad_an_ambiguous_label_fails_the_probe_volume_group_missing_naming_the_ambiguity() {
    let part = super::volume_pool_support::labelled_partition();
    let disk = part
        .to_string_lossy()
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .trim_end_matches('p')
        .to_owned();
    let status = std::process::Command::new("sh")
        .args(["-c", &format!("echo ',,,,' | sfdisk --append {disk} && sfdisk --part-label {disk} 2 overdrive-volumes")])
        .status()
        .expect("sh");
    assert!(status.success(), "a second partition with the same label (needs sfdisk in the guest)");
    let error = probe_error(
        "sad_an_ambiguous_label_fails_the_probe_volume_group_missing_naming_the_ambiguity",
    );
    assert_eq!(error.scenario, VolumeProbeScenario::VolumeGroupMissing, "{error}");
    assert!(
        matches!(&error.detail, VolumeProbeDetail::AmbiguousLabel { nodes } if nodes.len() >= 2),
        "the detail names every device carrying the label (AmbiguousLabel), got {:?}",
        error.detail
    );
}

/// Sad (scenario 3): the scratch volume's device node cannot appear (the device directory is
/// read-only), so the probe names the scratch create or node failure.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
fn sad_a_scratch_node_that_cannot_appear_fails_the_probe_scratch_create_or_node_failed() {
    let _read_only = {
        let bind = BindOver::new("/dev/mapper", "/dev/mapper");
        let status = std::process::Command::new("mount")
            .args(["-o", "remount,bind,ro", "/dev/mapper"])
            .status()
            .expect("mount");
        assert!(status.success(), "remount read-only");
        bind
    };
    let error = probe_error(
        "sad_a_scratch_node_that_cannot_appear_fails_the_probe_scratch_create_or_node_failed",
    );
    assert_eq!(error.scenario, VolumeProbeScenario::ScratchCreateOrNodeFailed, "{error}");
    assert!(
        matches!(
            error.detail,
            VolumeProbeDetail::NodeMissing { .. } | VolumeProbeDetail::CommandFailed { .. }
        ),
        "the scratch create failed or its node never appeared, got {:?}",
        error.detail
    );
    let _ = lvm(&["lvs", "overdrive"]);
}

/// Sad (scenario 4): the scratch volume cannot be removed because something holds it open.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
fn sad_a_scratch_volume_held_open_fails_the_probe_scratch_remove_failed() {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watcher = {
        let stop = std::sync::Arc::clone(&stop);
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                for entry in std::fs::read_dir("/dev/mapper").into_iter().flatten().flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if let (true, Ok(file)) = (
                        name.starts_with("overdrive-ovd--probe--"),
                        std::fs::File::open(entry.path()),
                    ) {
                        // Held open until the process ends: the scratch volume cannot be removed.
                        std::mem::forget(file);
                    }
                }
                std::thread::sleep(std::time::Duration::from_micros(100));
            }
        })
    };
    let error = probe_error("sad_a_scratch_volume_held_open_fails_the_probe_scratch_remove_failed");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    watcher.join().expect("watcher");
    assert_eq!(error.scenario, VolumeProbeScenario::ScratchRemoveFailed, "{error}");
    assert!(
        matches!(error.detail, VolumeProbeDetail::CommandFailed { exit_status: Some(_), .. }),
        "the failed lvremove exited non-zero (CommandFailed), got {:?}",
        error.detail
    );
}

/// Sad (P-4, ADR-0314 Bound): an `lvm` command that stalls on the volume group's lock. The probe
/// enforces its own 30 s deadline over the whole call, on the adapter's own timer: it returns a
/// `VolumeProbeError` whose detail is `Io` of kind `TimedOut` and whose scenario is that of the
/// command in flight, never a hang. The stalled command is not killed at the deadline: it is still
/// running when `probe` returns and ends on its own once the lock is released. A probe after that
/// passes and leaves no scratch volume.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
fn sad_a_probe_blocked_on_the_group_lock_times_out_as_an_io_failure() {
    runtime().block_on(async {
        std::fs::create_dir_all("/run/lock/lvm").expect("lock dir");
        let mut holder = std::process::Command::new("flock")
            // `--no-fork`: flock execs `sleep` in place, so the process the test kills is the one
            // holding the lock. The hold outlasts the 30 s deadline by a margin, and ends by
            // itself at 45 s so a probe with no deadline of its own is seen to return late.
            .args(["--no-fork", "/run/lock/lvm/V_overdrive", "sleep", "45"])
            .spawn()
            .expect("hold the group lock (needs flock in the guest)");
        tokio::time::sleep(Duration::from_millis(200)).await;
        let started = std::time::Instant::now();
        let result = LvmVolumePoolProbe::new().probe().await;
        let waited = started.elapsed();
        let still_running = lvm_processes();
        let _ = holder.kill();
        let _ = holder.wait();
        assert!(
            waited >= Duration::from_secs(29) && waited < Duration::from_secs(33),
            "the probe must return within its 30 s deadline, the adapter's own: {waited:?}"
        );
        let error = result.err().expect("a probe blocked past its deadline fails");
        assert!(
            matches!(&error.detail, VolumeProbeDetail::Io(source) if source.kind() == std::io::ErrorKind::TimedOut),
            "the deadline return is an Io failure of kind TimedOut, got {:?}",
            error.detail
        );
        // The in-flight command is the first to need the held group lock: `pvs` (reports as
        // `ToolsOrDeviceMapperUnusable`) or, if `pvs` takes no group lock, `vgs` (`VolumeGroupMissing`).
        assert!(
            matches!(
                error.scenario,
                VolumeProbeScenario::ToolsOrDeviceMapperUnusable
                    | VolumeProbeScenario::VolumeGroupMissing
            ),
            "the deadline return names the scenario of the command in flight, got {error}"
        );
        assert!(
            !still_running.is_empty(),
            "the stalled lvm command is not killed at the deadline: it is still running after probe returned"
        );
        let released = std::time::Instant::now();
        while !lvm_processes().is_empty() {
            assert!(
                released.elapsed() < Duration::from_secs(30),
                "the stalled lvm command ends on its own once the lock is released, still running: {:?}",
                lvm_processes()
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let probed = LvmVolumePoolProbe::new().probe().await.expect("a probe after the release passes");
        assert!(probed.is_some(), "the labelled partition is found again");
        assert!(logical_volumes().is_empty(), "no scratch volume is left behind");
    });
}

/// Sad (P-5, ADR-0314 scenario `SysfsUnreadable`): reading `/sys/class/block` to discover the
/// labelled partition fails. The read fails for real: the process has no file descriptor left, so
/// the kernel refuses to open the directory (`EMFILE`). The failure is reported as
/// `SysfsUnreadable` with an `Io` detail, not as an `lvm` or device-mapper failure. The scenario is
/// compared by its `Debug` name because the variant does not exist yet.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
fn sad_an_unreadable_sysfs_block_directory_fails_the_probe_sysfs_unreadable() {
    // The runtime (its poller and wakers hold descriptors) is built before the descriptors run out.
    let runtime = runtime();
    let mut held = Vec::new();
    let exhausted = loop {
        match std::fs::File::open("/dev/null") {
            Ok(file) => held.push(file),
            Err(error) => break error,
        }
    };
    let result = runtime.block_on(LvmVolumePoolProbe::new().probe());
    drop(held);
    assert!(
        matches!(exhausted.raw_os_error(), Some(23 | 24)),
        "the descriptors ran out (ENFILE or EMFILE), got {exhausted:?}"
    );
    let error = result.err().expect("a probe that cannot read the block devices fails");
    assert_eq!(
        format!("{:?}", error.scenario),
        "SysfsUnreadable",
        "a failed sysfs read must be reported as SysfsUnreadable, not as a tool failure: {error}"
    );
    assert!(
        matches!(error.detail, VolumeProbeDetail::Io(_)),
        "a failed directory read is an Io failure, got {:?}",
        error.detail
    );
}

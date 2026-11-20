//! Support for the volume pool Tier-3 tests that run in a `volume-pool` guest (GH #348):
//! the guest's own pool, inspected and, for the broken-pool catalogue, deliberately
//! damaged with the same `lvm` tools Overdrive uses. Nothing here is a driving port;
//! the tests drive `VolumePoolProbe` / `VolumeBacking`.

#![allow(clippy::expect_used, clippy::unwrap_used, dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use overdrive_core::id::WorkloadId;
use overdrive_core::traits::volume_backing::{VolumeBacking, VolumePoolProbe};
use overdrive_host::LvmVolumePoolProbe;

/// The partition the node's provisioning labelled `overdrive-volumes` (kernel `PARTNAME`).
pub fn labelled_partition() -> PathBuf {
    let mut found = Vec::new();
    for entry in std::fs::read_dir("/sys/class/block").expect("sysfs block devices") {
        let entry = entry.expect("entry");
        let uevent = std::fs::read_to_string(entry.path().join("uevent")).unwrap_or_default();
        if uevent.lines().any(|line| line == "PARTNAME=overdrive-volumes") {
            found.push(PathBuf::from("/dev").join(entry.file_name()));
        }
    }
    assert_eq!(found.len(), 1, "the guest carries exactly one labelled partition: {found:?}");
    found.remove(0)
}

/// `lvm <subcommand> ...` as Overdrive spawns it: `LVM_SYSTEM_DIR=/run/lvm` and the pinned
/// device filter, so a test sees exactly what the adapter sees.
pub fn lvm(args: &[&str]) -> Output {
    let part = labelled_partition();
    let config = format!(
        "activation{{udev_sync=0 udev_rules=0 monitoring=0}} devices{{filter=[\"a|^{}$|\",\"r|.*|\"] use_devicesfile=0}} backup{{backup=0 archive=0}} global{{use_lvmpolld=0 locking_dir=\"/run/lock/lvm\"}}",
        part.display()
    );
    Command::new("lvm")
        .args(args)
        .arg("--config")
        .arg(config)
        .env("LVM_SYSTEM_DIR", "/run/lvm")
        .output()
        .expect("spawn lvm")
}

pub fn lvm_ok(args: &[&str]) -> String {
    let out = lvm(args);
    assert!(out.status.success(), "lvm {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Pids of running `lvm` processes (the executable's `comm` contains `lvm`), read from `/proc` so
/// the guest needs no `pgrep`. A command the adapter spawned and has not been told to stop shows
/// here until it ends on its own.
pub fn lvm_processes() -> Vec<u32> {
    std::fs::read_dir("/proc")
        .expect("procfs")
        .flatten()
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let comm = std::fs::read_to_string(entry.path().join("comm")).ok()?;
            comm.contains("lvm").then_some(pid)
        })
        .collect()
}

/// Logical volume names in the group `overdrive`.
pub fn logical_volumes() -> Vec<String> {
    lvm_ok(&["lvs", "--noheadings", "-o", "lv_name", "overdrive"])
        .lines()
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect()
}

/// The size in bytes of logical volume `name`.
pub fn lv_bytes(name: &str) -> u64 {
    lvm_ok(&[
        "lvs",
        "--noheadings",
        "--units",
        "b",
        "--nosuffix",
        "-o",
        "lv_size",
        &format!("overdrive/{name}"),
    ])
    .trim()
    .parse()
    .expect("a byte count")
}

/// The device-mapper node name of a logical volume in the `overdrive` group: `<vg>-<lv>` with every
/// `-` inside the group and the volume name doubled, so `ovd-<hash>` is `overdrive-ovd--<hash>`.
pub fn dm_node_name(lv: &str) -> String {
    format!("overdrive-{}", lv.replace('-', "--"))
}

/// Whether `name` is activated (has a `/dev/mapper` node).
pub fn lv_active(name: &str) -> bool {
    Path::new("/dev/mapper").join(dm_node_name(name)).exists()
}

/// Free bytes in the group.
pub fn free_bytes() -> u64 {
    lvm_ok(&["vgs", "--noheadings", "--units", "b", "--nosuffix", "-o", "vg_free", "overdrive"])
        .trim()
        .parse()
        .expect("a byte count")
}

/// The pool the guest's provisioning created, probed through the production probe.
/// `test` names the calling test in the failure message.
pub async fn probed_backing(test: &str) -> Arc<dyn VolumeBacking> {
    LvmVolumePoolProbe::new()
        .probe()
        .await
        .expect("a healthy pool probes clean")
        .unwrap_or_else(|| panic!("the guest's pool is found ({test})"))
}

pub fn workload(id: &str) -> WorkloadId {
    WorkloadId::new(id).expect("a valid workload id")
}

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime")
}

//! Node-local persistent volume (GH #348, ADR-0311, ADR-0315, ADR-0317, ADR-0318): the
//! operator journey through a real in-process `overdrive serve` and `overdrive deploy` in a
//! `volume-pool` guest (a nested-launch guest that carries the volume pool), with the real
//! Cloud Hypervisor fork and the real LVM pool. Nothing is installed that production does
//! not install: the pool is the guest's own, probed at boot.
//!
//! Driving ports: `commands::deploy::{deploy, stop}`, `commands::workload::{restart, describe}`.
//! Faults enter only at the kernel: a SIGKILLed VMM, a suspended device (the old writer's
//! in-flight I/O is held, so its claim outlives its process), a device whose writes error.
//!
//! The guest command is a tiny script (`/sbin/boots`) that appends one 512-byte record
//! `BOOT-<k>` to the volume's first free block with an fsync and then sleeps (the drain test
//! runs `/sbin/boots-writing`, which keeps a write in flight; the failed-flush test runs
//! `/sbin/flushprobe`). The volume device is the device-mapper node
//! `/dev/mapper/overdrive-ovd--<hash>` (feature-delta § Port, the device path). The test reads
//! the device back from the host with direct I/O (the volume is only read there; the VMM holds
//! the writer claim), so what survives is observed in the volume itself, never in guest-reported state.
//!
//! Pending until the guest-kit volume-pool step (the profile's provisioning) and the launch
//! path land. Every test is `#[ignore]`d for the one missing resource, the guest's pool, which
//! the profile does not carry until prerequisite 1. Once it does, the spec carries an unknown
//! key and the deploy is refused, so each test fails at "a VM workload that declares
//! [vm.volume] must deploy" until the spec surface lands; its marker then moves to the
//! assertion the journey makes (the records read back from the device).
//!
//! Evidence tier: Tier 3, real guest kernel, real VMM, example-only, one named test per
//! scenario (Mandate 11). Placement: `guest = "cgroup-default", profile = "volume-pool"`.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(clippy::doc_markdown, reason = "the CONTRACT_SHAPE line is machine-read verbatim")]

use std::net::SocketAddr;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::ServeHandle;
use overdrive_cli::commands::workload::{
    DescribeArgs, RestartArgs, WorkloadDescribeOutput, describe, restart,
};
use overdrive_control_plane::api::AllocStateWire;
use overdrive_testing::overdrive_test;
use overdrive_testing::vm_fixture::VmFixture;
use serial_test::serial;
use tempfile::TempDir;

const BLOCK: u64 = 512;
const LISTENER_PORT: u16 = 18081;
const BLOCK_LEN: usize = 512;

fn staging_root() -> PathBuf {
    overdrive_testing::vm_fixture::default_staging_root()
}

fn server_tmp() -> TempDir {
    tempfile::Builder::new()
        .prefix("vm-volume-")
        .tempdir_in(staging_root())
        .expect("server tempdir on the reflink-capable staging root")
}

fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

async fn serve_at(data_dir: &Path, config_dir: &Path) -> ServeHandle {
    std::fs::create_dir_all(data_dir).expect("create data dir");
    std::fs::create_dir_all(config_dir).expect("create operator config dir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let args =
        super::serve_fixture::serve_args(bind, data_dir.to_path_buf(), config_dir.to_path_buf());
    super::serve_fixture::serve(args, super::serve_fixture::DataplaneChoice::Sim)
        .await
        .expect("serve must start")
}

/// The guest command: record the boot in the volume's first free block, fsync, sleep.
const BOOTS_SCRIPT: &str = r#"(while :; do nc -l -p 18081 >/dev/null 2>&1 || sleep 1; done) &
dev=/dev/disk/by-id/virtio-ovd-volume
[ -b "$dev" ] || dev=/dev/vdb
k=0
while [ "$(dd if=$dev bs=512 skip=$k count=1 2>/dev/null | head -c 5)" = "BOOT-" ]; do k=$((k+1)); done
printf 'BOOT-%s' "$k" | dd of=$dev bs=512 seek=$k count=1 conv=sync,fsync 2>/dev/null
exec sleep 3600"#;

/// The drain journey's guest command (spike B2b setup): the same boot record, then a write loop
/// that always has one `O_DIRECT` + fsync write in flight (block 4096, clear of the records), so a
/// suspended device holds a pending write and the old VMM's claim outlives its process.
const BOOTS_WRITING_SCRIPT: &str = r#"(while :; do nc -l -p 18081 >/dev/null 2>&1 || sleep 1; done) &
dev=/dev/disk/by-id/virtio-ovd-volume
[ -b "$dev" ] || dev=/dev/vdb
k=0
while [ "$(dd if=$dev bs=512 skip=$k count=1 2>/dev/null | head -c 5)" = "BOOT-" ]; do k=$((k+1)); done
printf 'BOOT-%s' "$k" | dd of=$dev bs=512 seek=$k count=1 conv=sync,fsync 2>/dev/null
while :; do
  dd if=/dev/zero of=$dev bs=512 seek=4096 count=1 oflag=direct conv=fsync 2>/dev/null || :
done"#;

/// The failed-flush journey's guest command (S-VOL-T3-11): an `O_DIRECT` write, a control `fsync`
/// on the block device that must succeed, a marker block saying so, then an `fsync` loop with
/// nothing dirty (so its only work is the flush); the first failing `fsync` ends the guest with
/// exit code 7, which the host reads from the allocation row.
const FLUSH_PROBE_SCRIPT: &str = r#"(while :; do nc -l -p 18081 >/dev/null 2>&1 || sleep 1; done) &
dev=/dev/disk/by-id/virtio-ovd-volume
[ -b "$dev" ] || dev=/dev/vdb
printf 'FLUSH-WRITTEN' | dd of=$dev bs=512 seek=0 count=1 oflag=direct conv=sync 2>/dev/null || exit 3
dd if=/dev/null of=$dev bs=512 count=0 conv=notrunc,fsync 2>/dev/null || exit 4
printf 'FLUSH-CONTROL-OK' | dd of=$dev bs=512 seek=1 count=1 oflag=direct conv=sync 2>/dev/null || exit 5
while :; do
  sleep 1
  dd if=/dev/null of=$dev bs=512 count=0 conv=notrunc,fsync 2>/dev/null || exit 7
done"#;

/// Stages a per-test copy of the shared rootfs with a static busybox shell and `/sbin/boots`.
fn stage_rootfs(tmp: &Path, fixture: &VmFixture) -> PathBuf {
    let scripts: [(&str, &str); 3] = [
        ("boots", BOOTS_SCRIPT),
        ("boots-writing", BOOTS_WRITING_SCRIPT),
        ("flushprobe", FLUSH_PROBE_SCRIPT),
    ];
    let copy = tmp.join("rootfs.ext4");
    std::fs::copy(&fixture.rootfs_path, &copy).expect("copy the shared fixture rootfs");
    let mnt = tmp.join("rootfs-mnt");
    std::fs::create_dir_all(&mnt).expect("mount point");
    let loop_dev = {
        let out = Command::new("losetup")
            .args(["--find", "--show"])
            .arg(&copy)
            .output()
            .expect("losetup");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    assert!(Command::new("mount").arg(&loop_dev).arg(&mnt).status().expect("mount").success());
    let busybox = [Path::new("/bin/busybox"), Path::new("/usr/bin/busybox")]
        .into_iter()
        .find(|candidate| candidate.is_file())
        .expect("a static busybox on the guest");
    let bin = mnt.join("bin");
    std::fs::create_dir_all(&bin).expect("guest /bin");
    std::fs::copy(busybox, bin.join("busybox")).expect("install busybox");
    std::fs::set_permissions(bin.join("busybox"), std::fs::Permissions::from_mode(0o755))
        .expect("chmod");
    for applet in ["sh", "sleep", "dd", "head", "printf", "nc"] {
        let target = bin.join(applet);
        let _ = std::fs::remove_file(&target);
        std::os::unix::fs::symlink("busybox", &target).expect("busybox applet");
    }
    std::fs::create_dir_all(mnt.join("sbin")).expect("guest /sbin");
    for (name, body) in scripts {
        let installed = mnt.join("sbin").join(name);
        std::fs::write(&installed, format!("#!/bin/sh\nset -eu\n{body}\n"))
            .expect("install script");
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755))
            .expect("chmod");
    }
    assert!(Command::new("umount").arg(&mnt).status().expect("umount").success());
    let _ = Command::new("losetup").arg("-d").arg(&loop_dev).status();
    copy
}

/// A single-replica Service spec: `workload restart` and a stop-then-resume place a successor
/// only for a Service (a Job's restart after its stop is a no-op, feature-delta § Restart
/// generation, "A finished Job"). The one declared listener is the Service admission shape; the
/// guest scripts open it (a background `nc` loop on `LISTENER_PORT`) so the startup gate over the declared listener passes.
fn service_toml(id: &str, command: &str, kernel: &Path, rootfs: &Path) -> String {
    format!(
        "[service]\nid = \"{id}\"\nreplicas = 1\n\n[[listener]]\nport = {LISTENER_PORT}\nprotocol = \"tcp\"\n\n[vm]\ncommand = \"{command}\"\nargs = []\nkernel = \"{}\"\nrootfs = \"{}\"\n\n[vm.volume]\nsize = \"1GiB\"\n\n[resources]\ncpu_milli = 500\nmemory_bytes = 134217728\n",
        kernel.display(),
        rootfs.display(),
    )
}

async fn describe_once(cfg: &Path, id: &str) -> WorkloadDescribeOutput {
    describe(DescribeArgs { id: id.to_owned(), config_path: cfg.to_owned() })
        .await
        .expect("workload describe must succeed while polling")
}

/// Polls until the newest allocation is `Running` and is not `not_alloc`.
async fn running_other_than(
    cfg: &Path,
    id: &str,
    not_alloc: Option<&str>,
    max_wait: Duration,
) -> WorkloadDescribeOutput {
    let deadline = tokio::time::Instant::now() + max_wait;
    loop {
        let out = describe_once(cfg, id).await;
        if out.snapshot.rows.iter().any(|row| {
            row.state == AllocStateWire::Running && Some(row.alloc_id.as_str()) != not_alloc
        }) {
            return out;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{id} never reached Running; rows: {:?}",
            out.snapshot.rows
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn running_alloc(out: &WorkloadDescribeOutput) -> String {
    out.snapshot
        .rows
        .iter()
        .find(|row| row.state == AllocStateWire::Running)
        .expect("a running allocation")
        .alloc_id
        .clone()
}

/// The workload's volume device node: the one `overdrive-ovd--<32 hex>` node under `/dev/mapper`
/// (dm name `<vg>-<lv>` with every `-` doubled; VG `overdrive`, LV `ovd-<hash>`; the scratch
/// volumes of the boot probe, `overdrive-ovd--probe--<x>`, are not 32 hex and are excluded).
fn volume_device() -> PathBuf {
    let nodes: Vec<PathBuf> = std::fs::read_dir("/dev/mapper")
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    name.strip_prefix(VOLUME_NODE_PREFIX).is_some_and(|hash| {
                        hash.len() == 32
                            && hash
                                .bytes()
                                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                    })
                })
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(nodes.len(), 1, "exactly one volume node: {nodes:?}");
    nodes.into_iter().next().expect("one node")
}

const VOLUME_NODE_PREFIX: &str = "overdrive-ovd--";

/// Block `k` of the device, read with direct I/O (never through the host page cache).
fn read_block(device: &Path, k: u64) -> Vec<u8> {
    let out = Command::new("dd")
        .arg(format!("if={}", device.display()))
        .arg("iflag=direct")
        .arg(format!("bs={BLOCK}"))
        .arg(format!("skip={k}"))
        .arg("count=1")
        .output()
        .expect("spawn dd");
    assert!(out.status.success(), "dd: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

fn boot_records(device: &Path) -> Vec<String> {
    let mut records = Vec::new();
    for k in 0..16 {
        let block = read_block(device, k);
        if !block.starts_with(b"BOOT-") {
            break;
        }
        records.push(
            String::from_utf8_lossy(
                &block[..block.iter().position(|b| *b == 0).unwrap_or(BLOCK_LEN)],
            )
            .into_owned(),
        );
    }
    records
}

/// Waits until the device holds exactly `n` boot records.
async fn wait_for_records(device: &Path, n: usize, max_wait: Duration) -> Vec<String> {
    let deadline = tokio::time::Instant::now() + max_wait;
    loop {
        let records = boot_records(device);
        if records.len() >= n {
            return records;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the volume holds {records:?}, wanted {n} records"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn expected(n: usize) -> Vec<String> {
    (0..n).map(|k| format!("BOOT-{k}")).collect()
}

fn cloud_hypervisor_argv(alloc: &str) -> Vec<String> {
    for entry in std::fs::read_dir("/proc").expect("read /proc").flatten() {
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
        let args: Vec<String> =
            cmdline.split(|b| *b == 0).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
        if args.first().is_some_and(|a| a.ends_with("cloud-hypervisor"))
            && args.iter().any(|a| a.contains(alloc))
        {
            return args;
        }
    }
    panic!("no cloud-hypervisor process for {alloc}");
}

fn cloud_hypervisor_pid(alloc: &str) -> i32 {
    for entry in std::fs::read_dir("/proc").expect("read /proc").flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else { continue };
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
        let text = String::from_utf8_lossy(&cmdline);
        if text.contains("cloud-hypervisor") && text.contains(alloc) {
            return pid;
        }
    }
    panic!("no cloud-hypervisor process for {alloc}");
}

fn kill(pid: i32) {
    assert!(
        Command::new("kill").args(["-KILL", &pid.to_string()]).status().expect("kill").success()
    );
}

struct Journey {
    fixture: VmFixture,
    tmp: TempDir,
    rootfs: PathBuf,
    server: TempDir,
}

impl Journey {
    fn new() -> Self {
        let fixture =
            VmFixture::provision(&staging_root()).expect("provision the shared VM fixture");
        let tmp = tempfile::Builder::new()
            .prefix("vm-volume-art-")
            .tempdir_in(staging_root())
            .expect("tempdir");
        let rootfs = stage_rootfs(tmp.path(), &fixture);
        Self { fixture, tmp, rootfs, server: server_tmp() }
    }

    fn data_dir(&self) -> PathBuf {
        self.server.path().join("data")
    }

    fn config_dir(&self) -> PathBuf {
        self.server.path().join("conf")
    }

    fn cfg(&self) -> PathBuf {
        config_path(self.server.path())
    }

    async fn deploy(&self, id: &str) -> String {
        self.deploy_running("/sbin/boots", id).await
    }

    async fn deploy_running(&self, command: &str, id: &str) -> String {
        let spec = self.tmp.path().join(format!("{id}.toml"));
        std::fs::write(&spec, service_toml(id, command, &self.fixture.kernel_path, &self.rootfs))
            .expect("write spec");
        deploy(DeployArgs { spec, config_path: self.cfg() })
            .await
            .unwrap_or_else(|error| {
                panic!("a VM workload that declares [vm.volume] must deploy ({id}): {error}")
            })
            .workload_id
    }
}

/// S-VOL-T3-07, T3-10: create, write, `workload restart`, read back: the volume the first
/// allocation wrote is the volume the successor attaches, with its data; `describe` shows it; the
/// volume node is `root:<VMM gid>` mode `0660`; the VMM's second disk is the single render site's
/// golden (explicit raw image type, direct I/O, the fixed serial) and follows the rootfs disk.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[tokio::test]
#[serial(cgroup)]
#[ignore = "needs the volume-pool guest's pool: the guest-kit volume-pool step provisions it (prerequisite 1)"]
#[should_panic(expected = "a VM workload that declares [vm.volume] must deploy (vol-restart)")]
async fn data_written_by_the_guest_survives_a_workload_restart() {
    let journey = Journey::new();
    let handle = serve_at(&journey.data_dir(), &journey.config_dir()).await;
    let id = journey.deploy("vol-restart").await;
    let first = running_other_than(&journey.cfg(), &id, None, Duration::from_secs(120)).await;
    let first_alloc = running_alloc(&first);
    let device = volume_device();
    assert_eq!(wait_for_records(&device, 1, Duration::from_secs(60)).await, expected(1));

    let meta = std::fs::metadata(&device).expect("stat the volume node");
    assert_eq!(meta.uid(), 0, "the node is root-owned");
    assert_ne!(meta.gid(), 0, "the node's group is the confined VMM's");
    assert_eq!(meta.mode() & 0o777, 0o660, "mode 0660");

    let argv = cloud_hypervisor_argv(&first_alloc);
    let disks: Vec<&String> = argv.iter().skip_while(|a| *a != "--disk").skip(1).collect();
    let volume_disk =
        format!("path={},image_type=raw,direct=on,serial=ovd-volume", device.display());
    assert!(
        disks.iter().any(|d| d.as_str() == volume_disk),
        "the volume disk is the golden argument: {argv:?}"
    );
    // The rootfs disk is the argv golden's rootfs shape: `path=<clone>,image_type=raw`, with
    // `,readonly=on` for a read-only image, and no `serial=` (the volume's alone).
    let rootfs_at = disks
        .iter()
        .position(|d| {
            d.starts_with("path=")
                && !d.contains("serial=")
                && (d.ends_with(",image_type=raw") || d.ends_with(",image_type=raw,readonly=on"))
        })
        .expect("the argv carries the rootfs disk (`path=<clone>,image_type=raw`)");
    let volume_at = disks
        .iter()
        .position(|d| d.as_str() == volume_disk)
        .expect("the argv carries the volume disk, the golden argument");
    assert!(rootfs_at < volume_at, "the volume follows the rootfs disk: {disks:?}");

    let described =
        overdrive_cli::render::workload_describe(&describe_once(&journey.cfg(), &id).await);
    assert!(described.contains("1GiB") && described.contains("ovd-volume"), "{described}");

    restart(RestartArgs { id: id.clone(), config_path: journey.cfg() })
        .await
        .expect("workload restart");
    running_other_than(&journey.cfg(), &id, Some(&first_alloc), Duration::from_secs(120)).await;
    assert_eq!(
        wait_for_records(&device, 2, Duration::from_secs(60)).await,
        expected(2),
        "the successor found the first allocation's record and appended its own"
    );
    assert_eq!(volume_device(), device, "the same volume");
    handle.shutdown().await.expect("shutdown");
}

/// S-VOL-T3-07 (durability through a guest flush): a record the guest fsynced is on the device
/// after the VMM is killed by SIGKILL, and the replacement reads it.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[tokio::test]
#[serial(cgroup)]
#[ignore = "needs the volume-pool guest's pool: the guest-kit volume-pool step provisions it (prerequisite 1)"]
#[should_panic(expected = "a VM workload that declares [vm.volume] must deploy (vol-killed)")]
async fn a_record_the_guest_flushed_survives_a_killed_vmm() {
    let journey = Journey::new();
    let handle = serve_at(&journey.data_dir(), &journey.config_dir()).await;
    let id = journey.deploy("vol-killed").await;
    let first = running_other_than(&journey.cfg(), &id, None, Duration::from_secs(120)).await;
    let first_alloc = running_alloc(&first);
    let device = volume_device();
    wait_for_records(&device, 1, Duration::from_secs(60)).await;
    kill(cloud_hypervisor_pid(&first_alloc));
    running_other_than(&journey.cfg(), &id, Some(&first_alloc), Duration::from_secs(150)).await;
    assert_eq!(wait_for_records(&device, 2, Duration::from_secs(60)).await, expected(2));
    handle.shutdown().await.expect("shutdown");
}

/// S-VOL-T3-07 (stop keeps the volume, resume reattaches): an operator stop ends the allocation and
/// keeps the volume; restarting the stopped workload attaches the same volume with its data.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[tokio::test]
#[serial(cgroup)]
#[ignore = "needs the volume-pool guest's pool: the guest-kit volume-pool step provisions it (prerequisite 1)"]
#[should_panic(expected = "a VM workload that declares [vm.volume] must deploy (vol-stop)")]
async fn an_operator_stop_keeps_the_volume_and_a_resume_reattaches_it() {
    let journey = Journey::new();
    let handle = serve_at(&journey.data_dir(), &journey.config_dir()).await;
    let id = journey.deploy("vol-stop").await;
    let first = running_other_than(&journey.cfg(), &id, None, Duration::from_secs(120)).await;
    let first_alloc = running_alloc(&first);
    let device = volume_device();
    wait_for_records(&device, 1, Duration::from_secs(60)).await;
    stop(StopArgs { id: id.clone(), config_path: journey.cfg() }).await.expect("stop");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while !describe_once(&journey.cfg(), &id)
        .await
        .snapshot
        .rows
        .iter()
        .all(|row| row.state == AllocStateWire::Terminated)
    {
        assert!(tokio::time::Instant::now() < deadline, "the workload did not stop");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(device.exists(), "the stop keeps the volume");
    assert_eq!(boot_records(&device), expected(1), "the stop keeps the data");
    restart(RestartArgs { id: id.clone(), config_path: journey.cfg() }).await.expect("resume");
    running_other_than(&journey.cfg(), &id, Some(&first_alloc), Duration::from_secs(120)).await;
    assert_eq!(wait_for_records(&device, 2, Duration::from_secs(60)).await, expected(2));
    handle.shutdown().await.expect("shutdown");
}

/// S-VOL-T3-09 (single writer across a replacement, spike B2b): the guest keeps a write in flight
/// (`/sbin/boots-writing` loops `O_DIRECT` + fsync writes), so when the device is suspended that
/// write is held and the VMM is killed, so its claim outlives its process; `workload
/// restart` mints the successor, which waits for the CLAIM, not the process: a second exclusive
/// opener is refused while it is held, the successor is not `Running` and did not fail, and once the
/// device resumes the claim drains and the successor attaches and sees the data.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[tokio::test]
#[serial(cgroup)]
#[ignore = "needs the volume-pool guest's pool: the guest-kit volume-pool step provisions it (prerequisite 1)"]
#[should_panic(expected = "a VM workload that declares [vm.volume] must deploy (vol-drain)")]
async fn a_replacement_waits_for_the_claim_while_the_old_writers_io_drains() {
    let journey = Journey::new();
    let handle = serve_at(&journey.data_dir(), &journey.config_dir()).await;
    let id = journey.deploy_running("/sbin/boots-writing", "vol-drain").await;
    let first = running_other_than(&journey.cfg(), &id, None, Duration::from_secs(120)).await;
    let first_alloc = running_alloc(&first);
    let device = volume_device();
    wait_for_records(&device, 1, Duration::from_secs(60)).await;
    let name = device.file_name().expect("node name").to_string_lossy().into_owned();
    let dmsetup = |verb: &str| {
        assert!(
            Command::new("dmsetup").args([verb, &name]).status().expect("dmsetup").success(),
            "dmsetup {verb}"
        );
    };
    // The guest's write loop has a write in flight: once suspended, the device holds it.
    dmsetup("suspend");
    kill(cloud_hypervisor_pid(&first_alloc));
    restart(RestartArgs { id: id.clone(), config_path: journey.cfg() })
        .await
        .expect("workload restart");
    tokio::time::sleep(Duration::from_secs(8)).await;
    assert_claim_held_by_a_dead_writer(&device, &first_alloc);
    let waiting = describe_once(&journey.cfg(), &id).await;
    assert!(
        !waiting
            .snapshot
            .rows
            .iter()
            .any(|row| row.state == AllocStateWire::Running && row.alloc_id != first_alloc),
        "the successor does not run while the claim is held"
    );
    assert!(
        !waiting
            .snapshot
            .rows
            .iter()
            .any(|row| row.state == AllocStateWire::Failed && row.alloc_id != first_alloc),
        "waiting for the drain is not a failure"
    );
    dmsetup("resume");
    running_other_than(&journey.cfg(), &id, Some(&first_alloc), Duration::from_secs(120)).await;
    assert_eq!(wait_for_records(&device, 2, Duration::from_secs(60)).await, expected(2));
    handle.shutdown().await.expect("shutdown");
}

/// The `/dev/dm-N` name of the volume's device-mapper node, read from sysfs by its dm name.
fn dm_block_name(device: &Path) -> String {
    let name = device.file_name().expect("node name").to_string_lossy().into_owned();
    for entry in std::fs::read_dir("/sys/block").expect("read /sys/block").flatten() {
        let dm_name = entry.path().join("dm").join("name");
        if std::fs::read_to_string(&dm_name).is_ok_and(|n| n.trim() == name) {
            return entry.file_name().to_string_lossy().into_owned();
        }
    }
    panic!("no /sys/block/dm-* entry carries the dm name {name}");
}

fn dmsetup_output(args: &[&str]) -> String {
    let out = Command::new("dmsetup").args(args).output().expect("spawn dmsetup");
    assert!(out.status.success(), "dmsetup {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The volume's current table with every `linear` segment rewritten to `flakey`, down for the
/// whole test: `<start> <len> flakey <dev> <offset> 0 <down> 1 error_writes`.
fn flakey_table(current: &str, down_seconds: u32) -> String {
    current
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            assert!(
                f.len() >= 5 && f[2] == "linear",
                "the volume's normal table is linear segments: {line}"
            );
            format!("{} {} flakey {} {} 0 {down_seconds} 1 error_writes", f[0], f[1], f[3], f[4])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// S-VOL-T3-11 (a failed host flush reaches the guest's `fsync` as an error in the production
/// write-back attachment, ADR-0316): the guest wrote its volume with `O_DIRECT` and a control
/// `fsync` on the block device succeeded; the test then swaps the volume's device-mapper table to `dm-flakey` erroring writes while the VMM
/// runs, so with nothing dirty the guest's next `fsync` has only the flush to do, and it must return
/// an error (the guest ends with exit code 7). Faults enter only at the kernel device.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[tokio::test]
#[serial(cgroup)]
#[ignore = "needs the volume-pool guest's pool: the guest-kit volume-pool step provisions it (prerequisite 1), with dmsetup and the dm-flakey module and loader"]
#[should_panic(expected = "a VM workload that declares [vm.volume] must deploy (vol-flush)")]
async fn a_failed_host_flush_reaches_the_guests_fsync_as_an_error() {
    let journey = Journey::new();
    let handle = serve_at(&journey.data_dir(), &journey.config_dir()).await;
    let id = journey.deploy_running("/sbin/flushprobe", "vol-flush").await;
    let first = running_other_than(&journey.cfg(), &id, None, Duration::from_secs(120)).await;
    let first_alloc = running_alloc(&first);
    let device = volume_device();

    // The control: the guest's O_DIRECT write reached the device and its fsync succeeded, so it
    // said so in the second block.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while !read_block(&device, 1).starts_with(b"FLUSH-CONTROL-OK") {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the control fsync on the normal linear table must succeed and be reported"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        read_block(&device, 0).starts_with(b"FLUSH-WRITTEN"),
        "the guest's O_DIRECT write is on the device"
    );

    // The precondition: the volume's dm device reports a write cache, or the guest's disk has none
    // and the flush never leaves the guest.
    let dm = dm_block_name(&device);
    let cache = std::fs::read_to_string(format!("/sys/block/{dm}/queue/write_cache"))
        .expect("read the dm device's write_cache");
    assert_eq!(
        cache.trim(),
        "write back",
        "precondition: the volume's dm device must report a write cache; without a write cache the \
         kernel completes the empty flush OK and the test would mean nothing"
    );

    // Swap the table: suspend, load the flakey table, resume.
    let name = device.file_name().expect("node name").to_string_lossy().into_owned();
    let table = flakey_table(&dmsetup_output(&["table", &name]), 3600);
    assert!(Command::new("dmsetup").args(["suspend", &name]).status().expect("dmsetup").success());
    let mut load = Command::new("dmsetup")
        .args(["load", &name])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("spawn dmsetup load");
    std::io::Write::write_all(
        load.stdin.as_mut().expect("dmsetup stdin"),
        format!("{table}\n").as_bytes(),
    )
    .expect("write the flakey table");
    assert!(load.wait().expect("dmsetup load").success(), "load the dm-flakey table");
    assert!(Command::new("dmsetup").args(["resume", &name]).status().expect("dmsetup").success());

    // The guest's fsync returns an error: its loop ends with exit code 7 (its control fsyncs
    // succeeded, so any other ending is not this).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let exit_code = loop {
        let out = describe_once(&journey.cfg(), &id).await;
        if let Some(row) = out
            .snapshot
            .rows
            .iter()
            .find(|row| row.alloc_id == first_alloc && row.state != AllocStateWire::Running)
        {
            break row.exit_code;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the guest's fsync on the failing volume must return an error, ending the guest; rows: {:?}",
            out.snapshot.rows
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    assert_eq!(
        exit_code,
        Some(7),
        "the guest's fsync on the failing volume must return an error (the guest ends with exit 7)"
    );
    handle.shutdown().await.expect("shutdown");
}

/// The pid of a live Cloud Hypervisor process whose command line mentions `needle` (an allocation
/// or workload id), or `None`. Observes process liveness through the host, never internals.
fn cloud_hypervisor_pid_matching(needle: &str) -> Option<i32> {
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else { continue };
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
        let text = String::from_utf8_lossy(&cmdline);
        if text.contains("cloud-hypervisor") && text.contains(needle) {
            return Some(pid);
        }
    }
    None
}

/// Fails on any `Failed` or charged (`restart_count > 0`) row of an allocation other than
/// `predecessor`.
fn assert_nothing_failed_or_charged(
    out: &WorkloadDescribeOutput,
    predecessor: Option<&str>,
    path: &str,
) {
    for row in &out.snapshot.rows {
        if Some(row.alloc_id.as_str()) == predecessor {
            continue;
        }
        assert_ne!(
            row.state,
            AllocStateWire::Failed,
            "a superseded launch writes no Failed row (path: {path}); rows: {:?}",
            out.snapshot.rows
        );
        assert_eq!(
            row.restart_count, 0,
            "a superseded launch charges nothing (path: {path}); rows: {:?}",
            out.snapshot.rows
        );
    }
}

/// S-VOL-T3-12 (boundary 4 and 4a, a stop while a replacement waits on a held claim): the old
/// writer's in-flight write is held by a suspended device so its claim outlives its process, the
/// replacement launch waits on the claim, and `workload stop` returns promptly (well under the 30 s
/// claim bound) because the stop supersedes the launch. No `Failed` row appears, no allocation is
/// charged, the volume is kept, and a later `workload restart` attaches it with its data.
/// Shape: example. Owner: `WorkloadLifecycle` + `VmDriver::start` through `serve`. Oracle: CLI
/// describe rows, the volume device contents. Tier 3.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[tokio::test]
#[serial(cgroup)]
#[ignore = "needs the volume-pool guest's pool, dmsetup and a real VMM: the guest-kit volume-pool step provisions them (prerequisite 1)"]
#[should_panic(expected = "a VM workload that declares [vm.volume] must deploy (vol-stop-held)")]
async fn a_workload_stop_during_a_held_claim_returns_promptly_with_no_failed_row() {
    let journey = Journey::new();
    let handle = serve_at(&journey.data_dir(), &journey.config_dir()).await;
    let id = journey.deploy_running("/sbin/boots-writing", "vol-stop-held").await;
    let first = running_other_than(&journey.cfg(), &id, None, Duration::from_secs(120)).await;
    let first_alloc = running_alloc(&first);
    let device = volume_device();
    wait_for_records(&device, 1, Duration::from_secs(60)).await;
    let name = device.file_name().expect("node name").to_string_lossy().into_owned();
    let dmsetup = |verb: &str| {
        assert!(
            Command::new("dmsetup").args([verb, &name]).status().expect("dmsetup").success(),
            "dmsetup {verb}"
        );
    };
    // The old writer's write is held: its claim outlives its process.
    dmsetup("suspend");
    kill(cloud_hypervisor_pid(&first_alloc));
    restart(RestartArgs { id: id.clone(), config_path: journey.cfg() })
        .await
        .expect("workload restart");
    tokio::time::sleep(Duration::from_secs(8)).await;
    assert_claim_held_by_a_dead_writer(&device, &first_alloc);

    let started = tokio::time::Instant::now();
    stop(StopArgs { id: id.clone(), config_path: journey.cfg() }).await.expect("workload stop");
    let took = started.elapsed();
    assert!(
        took < Duration::from_secs(10),
        "workload stop during a held claim must return promptly, not wait out the 30 s claim bound; took {took:?}"
    );
    tokio::time::sleep(Duration::from_secs(5)).await;
    let after_stop = describe_once(&journey.cfg(), &id).await;
    assert_nothing_failed_or_charged(
        &after_stop,
        Some(&first_alloc),
        "stop of a held-claim replacement",
    );
    assert!(
        !after_stop.snapshot.rows.iter().any(|row| row.state == AllocStateWire::Running),
        "the stopped workload has no running allocation; rows: {:?}",
        after_stop.snapshot.rows
    );
    assert!(device.exists(), "the stop keeps the volume");
    assert!(
        cloud_hypervisor_pid_matching(&id).is_none(),
        "the stopped workload has no VMM process while the claim is held: the replacement never started"
    );
    assert_claim_held_by_a_dead_writer(&device, &first_alloc);

    // The device drains: the superseded replacement must still never run or fail.
    dmsetup("resume");
    tokio::time::sleep(Duration::from_secs(10)).await;
    let after_drain = describe_once(&journey.cfg(), &id).await;
    assert_nothing_failed_or_charged(
        &after_drain,
        Some(&first_alloc),
        "stop of a held-claim replacement",
    );
    assert!(
        !after_drain.snapshot.rows.iter().any(|row| row.state == AllocStateWire::Running),
        "the replacement never appears after the old writer drains; rows: {:?}",
        after_drain.snapshot.rows
    );
    assert!(
        cloud_hypervisor_pid_matching(&id).is_none(),
        "no VMM process starts for the stopped workload after the claim clears"
    );

    // The volume is reattached by a later restart with its data.
    restart(RestartArgs { id: id.clone(), config_path: journey.cfg() })
        .await
        .expect("workload restart after the stop");
    running_other_than(&journey.cfg(), &id, Some(&first_alloc), Duration::from_secs(120)).await;
    assert_eq!(
        wait_for_records(&device, 2, Duration::from_secs(60)).await,
        expected(2),
        "the reattached volume holds the first allocation's record and the successor's"
    );
    handle.shutdown().await.expect("shutdown");
}

/// S-VOL-T3-13 (boundary 4, a stop during the guest boot; the Tier-3 twin of the volume driver's
/// `alive[a]` read in the Quint conformance driver): the VMM has been created and the guest has
/// not yet reported ready when `workload stop` arrives. The launch is abandoned, the VMM process
/// is terminated, no `Failed` row appears and nothing is charged, the volume is kept, and a later
/// `workload restart` attaches it again. A stop that loses the race to the guest's readiness is an
/// ordinary success that the stop then stops; the assertions after the stop hold either way.
/// Shape: example. Owner: `WorkloadLifecycle` + `VmDriver::start` through `serve`. Oracle: the
/// process table, CLI describe rows, the volume device node. Tier 3.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[tokio::test]
#[serial(cgroup)]
#[ignore = "needs the volume-pool guest's pool and a real VMM: the guest-kit volume-pool step provisions them (prerequisite 1)"]
#[should_panic(expected = "a VM workload that declares [vm.volume] must deploy (vol-stop-boot)")]
async fn a_workload_stop_during_guest_boot_terminates_the_vmm_and_keeps_the_volume() {
    let journey = Journey::new();
    let handle = serve_at(&journey.data_dir(), &journey.config_dir()).await;
    let id = journey.deploy("vol-stop-boot").await;
    // The VMM exists and no allocation is Running yet: the launch is in its guest-boot wait.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        if cloud_hypervisor_pid_matching(&id).is_some() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the launch must create its VMM before the stop ({id})"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Which path the stop takes depends on the race with the guest's readiness: a `Running` row
    // before the stop means the guest won (an ordinary stop of a running allocation), none means
    // the launch is still in its guest-boot wait (the abandon). Both are acceptable; the failure
    // text names the path taken.
    let running_before_stop = describe_once(&journey.cfg(), &id)
        .await
        .snapshot
        .rows
        .iter()
        .any(|row| row.state == AllocStateWire::Running);
    let path = if running_before_stop {
        "ordinary stop (a Running row was observed before the stop)"
    } else {
        "abandon (no Running row was observed before the stop)"
    };
    stop(StopArgs { id: id.clone(), config_path: journey.cfg() }).await.expect("workload stop");

    let gone = tokio::time::Instant::now() + Duration::from_secs(30);
    while cloud_hypervisor_pid_matching(&id).is_some() {
        assert!(
            tokio::time::Instant::now() < gone,
            "a stop during guest boot must terminate the VMM process ({id}); path: {path}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    tokio::time::sleep(Duration::from_secs(5)).await;
    let after_stop = describe_once(&journey.cfg(), &id).await;
    assert_nothing_failed_or_charged(&after_stop, None, path);
    assert!(
        !after_stop.snapshot.rows.iter().any(|row| row.state == AllocStateWire::Running),
        "the stopped workload has no running allocation (path: {path}); rows: {:?}",
        after_stop.snapshot.rows
    );
    let device = volume_device();
    assert!(device.exists(), "the stop keeps the volume (path: {path})");

    restart(RestartArgs { id: id.clone(), config_path: journey.cfg() })
        .await
        .expect("workload restart after the stop");
    running_other_than(&journey.cfg(), &id, None, Duration::from_secs(120)).await;
    wait_for_records(&device, 1, Duration::from_secs(60)).await;
    handle.shutdown().await.expect("shutdown");
}

/// The old writer's claim is held although its process is gone: the old VMM's pid no longer
/// exists, and a second `O_EXCL` open of the device is refused with `EBUSY`.
fn assert_claim_held_by_a_dead_writer(device: &Path, old_alloc: &str) {
    assert!(
        cloud_hypervisor_pid_matching(old_alloc).is_none(),
        "the old writer's VMM process for {old_alloc} is gone while its claim is held"
    );
    let refused =
        std::fs::OpenOptions::new().read(true).write(true).custom_flags_excl().open(device);
    let error =
        refused.expect_err("the old writer's claim is held: a second exclusive open is refused");
    assert_eq!(
        error.raw_os_error(),
        Some(libc::EBUSY),
        "the second exclusive open is refused as busy (the claim), not for another reason: {error}"
    );
}

trait ExclusiveOpen {
    fn custom_flags_excl(&mut self) -> &mut Self;
}

impl ExclusiveOpen for std::fs::OpenOptions {
    fn custom_flags_excl(&mut self) -> &mut Self {
        std::os::unix::fs::OpenOptionsExt::custom_flags(self, libc::O_EXCL)
    }
}

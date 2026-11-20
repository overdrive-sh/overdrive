//! Node-local persistent volume (GH #348, ADR-0312, ADR-0317, ADR-0318): the production
//! `VolumeBacking` (`LvmVolumeBacking`) against the real `lvm` tools and device-mapper of
//! a `volume-pool` guest. The same scenarios run on `SimVolumeBacking` in
//! `overdrive-sim/tests/acceptance/sim_volume_backing_contract.rs`, so a divergence between
//! the twins is one failing pair; `claim_state` is the named arbiter of the conformance
//! driver and its three cases below are the paired Tier-3 test (S-VOL-T3-04).
//!
//! Driving port: `VolumePoolProbe::probe` -> the returned `Arc<dyn VolumeBacking>` ->
//! `VolumeBacking`. Sad paths are named example tests (Mandate 11). Pending until the
//! guest-kit volume-pool step and `LvmVolumeBacking` exist. Every test is `#[ignore]`d for the
//! one missing resource, the guest's pool; once the guest carries it, the probe scaffold still
//! reports no pool, so each test fails at "the guest's pool is found" (the precondition the
//! scaffold's body denies it). The `lvm` contract test uses no Overdrive code and is a guard
//! once the pool exists.
//!
//! Evidence tier: Tier 3, real guest kernel and real `lvm`, example-only.

#![allow(clippy::doc_markdown, reason = "the CONTRACT_SHAPE line is machine-read verbatim")]

use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;
use std::time::Duration;

use overdrive_core::testing::volume::volume_size_gib;
use overdrive_core::traits::volume_backing::{ClaimState, VolumeBackingError};
use overdrive_core::volume::{VolumeOperation, VolumeSize};
use overdrive_testing::overdrive_test;

use super::volume_pool_support::{
    free_bytes, logical_volumes, lv_active, lv_bytes, lvm_ok, lvm_processes, probed_backing,
    runtime, workload,
};

const GIB: u64 = 1024 * 1024 * 1024;

fn gib(count: u64) -> VolumeSize {
    volume_size_gib(count)
}

fn node_name(device: &Path) -> String {
    device.file_name().expect("a node name").to_string_lossy().into_owned()
}

/// Opens the device node exclusively, as a VMM would (`O_EXCL` on a block device claims it).
fn open_exclusive(device: &Path) -> std::fs::File {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_EXCL)
        .open(device)
        .expect("the exclusive open takes the claim")
}

/// ADR-0312: `ensure` creates a logical volume of exactly the declared size (extents reserved),
/// activated, named `ovd-` plus 32 hex, in the `overdrive` group, and writes nothing to it.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
#[should_panic(
    expected = "the guest's pool is found (ensure_creates_the_declared_size_activated_under_the_hashed_name)"
)]
fn ensure_creates_the_declared_size_activated_under_the_hashed_name() {
    runtime().block_on(async {
        let backing =
            probed_backing("ensure_creates_the_declared_size_activated_under_the_hashed_name")
                .await;
        let device = backing.ensure(&workload("dev-box"), gib(1)).await.expect("ensure");
        let name = node_name(&device);
        // The ADR-0312 golden for "dev-box": the logical volume is `ovd-` + the SHA-256 prefix, its
        // device is the device-mapper node with every `-` doubled.
        let hash = "8cc272549bf232c0ea5e367b6b44997d";
        assert_eq!(
            device,
            Path::new("/dev/mapper").join(format!("overdrive-ovd--{hash}")),
            "the device is the exact dm node of the hashed volume"
        );
        assert_eq!(name, format!("overdrive-ovd--{hash}"));
        let lv = logical_volumes();
        assert_eq!(
            lv,
            vec![format!("ovd-{hash}")],
            "the logical volume is named ovd- plus the hash"
        );
        assert_eq!(lv.len(), 1, "{lv:?}");
        assert_eq!(lv_bytes(&lv[0]), GIB, "extents reserved for exactly the declared size");
        assert!(lv_active(&lv[0]), "activated");
        assert_eq!(
            backing.claim_state(&workload("dev-box")).await.expect("claim"),
            ClaimState::Free
        );
    });
}

/// A repeat `ensure` returns the same device and creates nothing; the existing size is not compared.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
#[should_panic(
    expected = "the guest's pool is found (ensure_is_idempotent_and_does_not_compare_the_existing_size)"
)]
fn ensure_is_idempotent_and_does_not_compare_the_existing_size() {
    runtime().block_on(async {
        let backing =
            probed_backing("ensure_is_idempotent_and_does_not_compare_the_existing_size").await;
        let w = workload("dev-box");
        let first = backing.ensure(&w, gib(1)).await.expect("ensure");
        let again = backing.ensure(&w, gib(1)).await.expect("ensure");
        let other_size = backing.ensure(&w, gib(2)).await.expect("ensure");
        assert_eq!((first.clone(), first), (again, other_size));
        let lv = logical_volumes();
        assert_eq!(lv.len(), 1);
        assert_eq!(lv_bytes(&lv[0]), GIB, "the existing volume keeps its size");
    });
}

/// V-1: a volume that exists but is inactive (the state after every node reboot) has no device
/// node: `claim_state` reports `NotFound`, and `ensure` activates the SAME volume.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
#[should_panic(
    expected = "the guest's pool is found (an_inactive_volume_is_activated_by_ensure_and_reads_not_found_until_then)"
)]
fn an_inactive_volume_is_activated_by_ensure_and_reads_not_found_until_then() {
    runtime().block_on(async {
        let backing = probed_backing(
            "an_inactive_volume_is_activated_by_ensure_and_reads_not_found_until_then",
        )
        .await;
        let w = workload("dev-box");
        let device = backing.ensure(&w, gib(1)).await.expect("ensure");
        let name = logical_volumes().remove(0);
        lvm_ok(&["lvchange", "-an", &format!("overdrive/{name}")]);
        assert!(matches!(backing.claim_state(&w).await, Err(VolumeBackingError::NotFound { .. })));
        let again = backing.ensure(&w, gib(1)).await.expect("ensure activates");
        assert_eq!(device, again);
        assert_eq!(logical_volumes().len(), 1, "reactivation creates nothing");
        assert_eq!(backing.claim_state(&w).await.expect("claim"), ClaimState::Free);
    });
}

/// S-VOL-T3-04 (the arbiter pairing of the conformance driver): held by an `O_EXCL` opener
/// reports `Held`; after release `Free`; deactivated `NotFound`, and `Free` once re-activated.
/// The same scenario on `SimVolumeBacking` must give the same three answers.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
#[should_panic(
    expected = "the guest's pool is found (claim_state_reports_held_free_and_not_found_as_the_kernel_does)"
)]
fn claim_state_reports_held_free_and_not_found_as_the_kernel_does() {
    runtime().block_on(async {
        let backing =
            probed_backing("claim_state_reports_held_free_and_not_found_as_the_kernel_does").await;
        let w = workload("dev-box");
        let device = backing.ensure(&w, gib(1)).await.expect("ensure");
        let holder = open_exclusive(&device);
        assert_eq!(backing.claim_state(&w).await.expect("claim"), ClaimState::Held);
        assert_eq!(
            backing.claim_state(&w).await.expect("claim"),
            ClaimState::Held,
            "reading never releases it"
        );
        drop(holder);
        assert_eq!(backing.claim_state(&w).await.expect("claim"), ClaimState::Free);
        let name = logical_volumes().remove(0);
        lvm_ok(&["lvchange", "-an", &format!("overdrive/{name}")]);
        assert!(matches!(backing.claim_state(&w).await, Err(VolumeBackingError::NotFound { .. })));
        backing.ensure(&w, gib(1)).await.expect("re-activate");
        assert_eq!(backing.claim_state(&w).await.expect("claim"), ClaimState::Free);
    });
}

/// Sad: a volume nobody created is `NotFound`.
/// CONTRACT_SHAPE: pure-function.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
#[should_panic(
    expected = "the guest's pool is found (sad_claim_state_of_an_absent_volume_is_not_found)"
)]
fn sad_claim_state_of_an_absent_volume_is_not_found() {
    runtime().block_on(async {
        let backing = probed_backing("sad_claim_state_of_an_absent_volume_is_not_found").await;
        assert!(matches!(
            backing.claim_state(&workload("ghost")).await,
            Err(VolumeBackingError::NotFound { .. })
        ));
    });
}

/// Sad: no free extent: typed `InsufficientSpace`, naming the declared size and the free space the
/// `lvm` report showed before the create; nothing is created.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
#[should_panic(
    expected = "the guest's pool is found (sad_insufficient_space_names_both_figures_and_creates_nothing)"
)]
fn sad_insufficient_space_names_both_figures_and_creates_nothing() {
    runtime().block_on(async {
        let backing =
            probed_backing("sad_insufficient_space_names_both_figures_and_creates_nothing").await;
        let free = free_bytes();
        let too_big = gib(free / GIB + 1);
        let err = backing.ensure(&workload("dev-box"), too_big).await.expect_err("it does not fit");
        match err {
            VolumeBackingError::InsufficientSpace { needed, free: observed } => {
                assert_eq!(needed, too_big);
                assert_eq!(observed.get(), free);
            }
            other => {
                panic!("an ensure that does not fit must fail InsufficientSpace, got {other:?}")
            }
        }
        assert!(logical_volumes().is_empty(), "nothing was created");
    });
}

/// Sad: an `lvm` command that fails is `PoolCommand` naming the operation; nothing is created.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
#[should_panic(
    expected = "the guest's pool is found (sad_a_failing_lvm_command_is_a_pool_command_error)"
)]
fn sad_a_failing_lvm_command_is_a_pool_command_error() {
    runtime().block_on(async {
        let backing = probed_backing("sad_a_failing_lvm_command_is_a_pool_command_error").await;
        let binary = String::from_utf8_lossy(
            &std::process::Command::new("sh")
                .args(["-c", "command -v lvm"])
                .output()
                .expect("sh")
                .stdout,
        )
        .trim()
        .to_owned();
        let status = std::process::Command::new("mount")
            .args(["--bind", "/dev/null", &binary])
            .status()
            .expect("mount");
        assert!(status.success());
        let err = backing.ensure(&workload("dev-box"), gib(1)).await;
        let _ = std::process::Command::new("umount").arg(&binary).status();
        assert!(
            matches!(
                err,
                Err(VolumeBackingError::PoolCommand {
                    operation: VolumeOperation::Report | VolumeOperation::Create,
                    ..
                })
            ),
            "got {err:?}"
        );
        assert!(logical_volumes().is_empty());
    });
}

/// Sad: an `lvm` command blocked past the `ensure` deadline (the volume group's lock is held
/// elsewhere). The adapter itself enforces the deadline (ADR-0317, B-1): `ensure` returns the typed
/// `PoolCommand` of kind `TimedOut` within the 30 s bound, whatever the pool does, not a hang. The
/// stalled command is not aborted or killed at the deadline: it is still running when `ensure`
/// returns, and ends on its own once the lock is released.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
#[should_panic(
    expected = "the guest's pool is found (sad_an_ensure_blocked_on_the_group_lock_times_out_as_a_command_failure)"
)]
fn sad_an_ensure_blocked_on_the_group_lock_times_out_as_a_command_failure() {
    runtime().block_on(async {
        let backing = probed_backing("sad_an_ensure_blocked_on_the_group_lock_times_out_as_a_command_failure").await;
        std::fs::create_dir_all("/run/lock/lvm").expect("lock dir");
        let mut holder = std::process::Command::new("flock")
            .args(["/run/lock/lvm/V_overdrive", "sleep", "90"])
            .spawn()
            .expect("hold the group lock (needs flock in the guest)");
        tokio::time::sleep(Duration::from_millis(200)).await;
        let started = std::time::Instant::now();
        let err = backing.ensure(&workload("dev-box"), gib(1)).await;
        let waited = started.elapsed();
        let still_running = lvm_processes();
        let _ = holder.kill();
        let _ = holder.wait();
        assert!(
            !still_running.is_empty(),
            "the stalled lvm command is not aborted at the deadline: it is still running after ensure returned TimedOut"
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
        match err {
            Err(VolumeBackingError::PoolCommand { source, .. }) => {
                assert_eq!(source.kind(), std::io::ErrorKind::TimedOut);
            }
            other => panic!("expected a timed-out PoolCommand, got {other:?}"),
        }
        assert!(
            waited >= Duration::from_secs(29) && waited < Duration::from_secs(33),
            "ensure must return within its 30 s deadline, the adapter's own: {waited:?}"
        );
    });
}

/// S-VOL-T3-05: two workloads' `ensure` at once both succeed with distinct volumes (LVM serialises
/// through its lock); the twin of the sim's `each_workload_gets_its_own_volume`.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
#[should_panic(
    expected = "the guest's pool is found (two_workloads_ensured_at_once_each_get_their_own_volume)"
)]
fn two_workloads_ensured_at_once_each_get_their_own_volume() {
    runtime().block_on(async {
        let backing =
            probed_backing("two_workloads_ensured_at_once_each_get_their_own_volume").await;
        let (a, b) = (workload("alpha"), workload("beta"));
        let (da, db) = tokio::join!(backing.ensure(&a, gib(1)), backing.ensure(&b, gib(1)));
        let (da, db) = (da.expect("alpha"), db.expect("beta"));
        assert_ne!(da, db);
        assert_eq!(logical_volumes().len(), 2);
    });
}

//! Full-bootstrap producer scenarios over the real durable systemd owner.
use super::native_controller::NativeController;
use super::support::*;
use std::fs;

/// CONTRACT_SHAPE: bounded-change (actual controller/service/finalizer records;
/// local dirty source is retained and mismatched remote source/body stay untouched).
#[test]
#[ignore = "pending DELIVER: metal-run-isolation composed no-sync refusal"]
fn sad_actual_no_sync_mismatch_aborts_the_registered_owner_before_start() {
    let controller = NativeController::new();
    success(&controller.fixture.sync(&controller.fixture.source));
    let healthy =
        controller.command(true).output().expect("healthy actual no-sync controller path");
    success(&healthy);
    let before_calls = fs::read(&controller.body).expect("healthy selected compiler actually ran");
    let remote = controller.fixture.checkout();
    let remote_before = snapshot(&remote);
    fs::write(controller.fixture.source.join("source-marker"), "changed local source\n")
        .expect("real dirty local input");
    let failed = controller.command(true).output().expect("actual no-sync mismatch");
    runner_error(&failed, "source_mismatch", 78);
    let id = NativeController::run_id(&failed);
    controller.assert_terminal(&id, "source_mismatch", 78);
    assert_eq!(fs::read(&controller.body).expect("body complement"), before_calls);
    assert_eq!(snapshot(&remote), remote_before);
    assert_eq!(text(&controller.fixture.source.join("source-marker")), "changed local source\n");
}

/// CONTRACT_SHAPE: bounded-change (registered preparation failure retains exact
/// typed status, never START/installation; no-sync avoids an unused prerequisite).
#[test]
#[ignore = "pending DELIVER: metal-run-isolation composed transfer prerequisite"]
fn sad_registered_missing_rsync_keeps_its_preparation_error_and_no_sync_needs_no_rsync() {
    let controller = NativeController::new();
    success(&controller.fixture.sync(&controller.fixture.source));
    let healthy = controller
        .command(true)
        .env("METAL_FIXTURE_MISSING_RSYNC", "1")
        .output()
        .expect("no-sync has no transfer prerequisite");
    success(&healthy);
    let before_calls = fs::read(&controller.body).expect("healthy selected compiler ran");
    let remote_before = snapshot(&controller.fixture.remote);
    let failed = controller
        .command(false)
        .env("METAL_FIXTURE_MISSING_RSYNC", "1")
        .output()
        .expect("registered missing transfer prerequisite");
    runner_error(&failed, "runner_preparation_failed", 75);
    assert!(String::from_utf8_lossy(&failed.stderr).contains("prerequisite=rsync"));
    let id = NativeController::run_id(&failed);
    controller.assert_terminal(&id, "runner_preparation_failed", 75);
    assert_eq!(fs::read(&controller.body).expect("body complement"), before_calls);
    assert_eq!(snapshot(&controller.fixture.remote), remote_before);
    assert!(!controller.fixture.temp.path().join("forbidden-install").exists());
}

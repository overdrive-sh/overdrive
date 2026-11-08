// Pre-authored transitions for the two existing qualification identities.
// Only explicit exclusive authority preserves their old arbitrary-command
// maintenance meaning; ordinary shared admission is covered independently.
use super::native_controller::NativeController;
use super::support::{executable, fields, runner_error, snapshot, success, text};
use super::{holder, terminate, wait_for_owner};
use std::fs;
use std::process::Stdio;

pub(super) fn exclusive_maintenance_and_sync_keep_their_distinct_boundaries() {
    let controller = NativeController::new();
    let fixture = &controller.fixture;
    let lock = fixture.temp.path().join("maintenance.lock");
    let owner = fixture.temp.path().join("maintenance.owner");
    let first = holder(&lock, &owner, "first-token", "run")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("existing exclusive maintenance holder");
    wait_for_owner(&owner);
    let metadata = text(&owner);
    for field in [
        "pid=",
        "started_at=",
        "action=run",
        "scenario=C-GTI-METAL-LEASE",
        "workspace=/workspace",
        "commit=deadbeef",
        "token=first-token",
    ] {
        assert!(metadata.contains(field), "missing preserved owner field {field}");
    }
    let before = snapshot(&fixture.remote);
    let blocked = controller
        .command_with_class(false, true, false)
        .output()
        .expect("explicit exclusive run contention");
    runner_error(&blocked, "maintenance_timeout", 75);
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("first-token"));
    assert_eq!(snapshot(&fixture.remote), before, "exclusive timeout cannot transfer source");
    let provision =
        fixture.bootstrap(&fixture.source, &[]).output().expect("existing provisioning contention");
    assert_eq!(provision.status.code(), Some(75));
    assert!(String::from_utf8_lossy(&provision.stderr).contains("first-token"));
    assert_eq!(snapshot(&fixture.remote), before, "provisioning timeout cannot mutate source");
    success(&fixture.sync(&fixture.source));
    assert_ne!(
        snapshot(&fixture.remote),
        before,
        "sync owns its checkout and performs actual transfer despite maintenance exclusion"
    );
    assert!(!fixture.temp.path().join("fixture-tree.owner").exists());
    let tree_lock = fixture.temp.path().join("fixture-tree.lock");
    let tree_owner = fixture.temp.path().join("fixture-tree.owner");
    let tree = holder(&tree_lock, &tree_owner, "tree-token", "sync")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("existing checkout holder");
    wait_for_owner(&tree_owner);
    let before = snapshot(&fixture.remote);
    let blocked = fixture.sync(&fixture.source);
    runner_error(&blocked, "checkout_timeout", 75);
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("tree-token"));
    assert_eq!(snapshot(&fixture.remote), before);
    terminate(tree, "checkout holder");
    terminate(first, "maintenance holder");
    assert!(!owner.exists());
    let reacquired = holder(&lock, &owner, "replacement-token", "bootstrap")
        .stdin(Stdio::null())
        .output()
        .expect("exclusive reacquisition after cancellation");
    success(&reacquired);
}

pub(super) fn privileged_command_gets_explicit_selected_artifacts_without_sudo_preservation() {
    let controller = NativeController::new();
    let forbidden = controller.fixture.temp.path().join("forbidden-sudo-preservation");
    executable(
        &controller.fixture.bin.join("sudo"),
        &format!(
            r#"#!/usr/bin/env bash
set -eu
[ "${{1:-}}" != -n ] || shift
if [ "${{1:-}}" = -E ]; then printf forbidden > '{}'; exit 93; fi
unset OVERDRIVE_METAL_KERNEL OVERDRIVE_METAL_ROOTFS
exec "$@"
"#,
            forbidden.display()
        ),
    );
    let output = controller
        .command_with_class(false, true, false)
        .output()
        .expect("explicit privileged exclusive command");
    success(&output);
    let environment = text(&controller.body.with_extension("environment"));
    assert!(
        environment.contains(&format!(
            "kernel={}",
            controller.fixture.temp.path().join("kernel").display()
        )),
        "{environment}"
    );
    assert!(
        environment.contains(&format!(
            "rootfs={}",
            controller.fixture.temp.path().join("rootfs").display()
        )),
        "{environment}"
    );
    assert!(!forbidden.exists(), "sudo -E cannot stand in for explicit artifact reinjection");
    let id = NativeController::run_id(&output);
    let metadata = fields(&controller.fixture.runs.join(id).join("metadata.v2"));
    assert_eq!(metadata["execution_class"], "exclusive");
    assert_eq!(metadata["execution_mode"], "privileged");
    assert!(!fs::read(&controller.body).expect("actual compiler ran").is_empty());
}

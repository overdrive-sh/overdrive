//! P2 real Linux flock/process ownership; systemd descendant-stop proof is the
//! independent native delivery capture, never this transport fixture.
use super::support::*;
use std::fs;
use std::io::Write as _;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

fn waiting_body(fixture: &Fixture, name: &str) -> String {
    format!(
        "printf started > '{}'; while [ ! -e '{}' ]; do sleep 0.02; done; exit 255",
        fixture.temp.path().join(format!("{name}.started")).display(),
        fixture.temp.path().join(format!("{name}.release")).display()
    )
}

fn assert_held_lock(owner: &std::path::Path, lock: &std::path::Path, mode: &str) {
    let pid = fields(owner)["pid"].clone();
    let descriptors = std::path::PathBuf::from(format!("/proc/{pid}/fd"));
    let descriptor = fs::read_dir(&descriptors)
        .expect("real owner descriptors")
        .find_map(|entry| {
            let entry = entry.expect("descriptor");
            (fs::read_link(entry.path()).ok().as_deref() == Some(lock)).then(|| entry.file_name())
        })
        .expect("durable owner must actually hold the named lock");
    let info = text(&std::path::PathBuf::from(format!(
        "/proc/{pid}/fdinfo/{}",
        descriptor.to_string_lossy()
    )));
    assert!(info.contains("FLOCK") && info.contains(mode), "actual fdinfo: {info}");
}

/// CONTRACT_SHAPE: bounded-change (two invocation process/record/descriptor
/// universes; completing one leaves the surviving owner's process and locks live).
#[test]
fn independent_ordinary_owners_overlap_after_controller_loss() {
    let fixture = Fixture::new();
    let mut first =
        Durable::spawn(&fixture, RUN_A, "first-tree", "ordinary", &waiting_body(&fixture, "first"));
    first.preparing();
    let first_owner = first.directory.join("owner.v1");
    assert_held_lock(&first_owner, &fixture.temp.path().join("maintenance.lock"), "READ");
    assert_held_lock(&first_owner, &fixture.temp.path().join("first-tree.lock"), "WRITE");
    assert_eq!(fields(&fixture.temp.path().join("first-tree.owner"))["token"], first.id);
    assert!(
        !fixture.temp.path().join("maintenance.owner").exists(),
        "ordinary cannot replace maintenance diagnostics"
    );
    first.send("START");
    first.wait_for(&fixture.temp.path().join("first.started"), "started");
    first.disconnect();
    let mut second = Durable::spawn(
        &fixture,
        RUN_B,
        "second-tree",
        "ordinary",
        &waiting_body(&fixture, "second"),
    );
    second.preparing();
    second.send("START");
    second.wait_for(&fixture.temp.path().join("second.started"), "started");
    assert!(first.is_live() && second.is_live(), "both real selected commands must overlap");
    let survivor_owner = text(&second.directory.join("owner.v1"));
    fs::write(fixture.temp.path().join("first.release"), "release")
        .expect("end selected first body");
    first.terminal();
    assert_eq!(fields(&first.directory.join("terminal.v1"))["exit_code"], "255");
    assert!(second.is_live());
    assert_eq!(text(&second.directory.join("owner.v1")), survivor_owner);
    assert_held_lock(
        &second.directory.join("owner.v1"),
        &fixture.temp.path().join("second-tree.lock"),
        "WRITE",
    );
    fs::write(fixture.temp.path().join("second.release"), "release").expect("end second body");
    second.terminal();
}

/// CONTRACT_SHAPE: bounded-change (a contending invocation registers/waits then
/// retains its typed timeout; held source, owner and selected body stay unchanged).
#[test]
fn sad_own_checkout_and_maintenance_deadlines_have_distinct_causes() {
    for (blocking_class, competing_tree, expected) in [
        ("ordinary", "same-tree", "checkout_timeout"),
        ("exclusive", "different-tree", "maintenance_timeout"),
    ] {
        let fixture = Fixture::new();
        let mut first = Durable::spawn(
            &fixture,
            RUN_A,
            "same-tree",
            blocking_class,
            &waiting_body(&fixture, "held"),
        );
        first.preparing();
        let before = snapshot(&fixture.source);
        let mut second = Durable::spawn(
            &fixture,
            RUN_B,
            competing_tree,
            "ordinary",
            "printf forbidden > must-not-run",
        );
        second.terminal();
        let terminal = fields(&second.directory.join("terminal.v1"));
        assert_eq!(terminal["kind"], "runner");
        assert_eq!(terminal["error"], expected);
        assert_eq!(fields(&second.directory.join("phase.v1"))["phase"], "waiting_for_lease");
        assert!(!second.source.join("must-not-run").exists());
        assert_eq!(snapshot(&fixture.source), before);
        assert_eq!(fields(&fixture.temp.path().join("same-tree.owner"))["token"], first.id);
        first.send("ABORT");
        first.terminal();
    }
}

/// CONTRACT_SHAPE: bounded-change (only one exact runner terminal is published;
/// tagged ABORT and EOF produce no selected-command/source/artifact delta).
#[test]
fn sad_prestart_controls_retain_the_approved_error_set_without_a_body() {
    for (control, expected) in [
        (Some("ABORT error=checkout_identity_conflict"), "checkout_identity_conflict"),
        (Some("ABORT error=source_mismatch"), "source_mismatch"),
        (Some("ABORT error=runner_preparation_failed"), "runner_preparation_failed"),
        (Some("ABORT error=preflight_failed"), "preflight_failed"),
        (Some("ABORT"), "launch_aborted"),
        (Some("ABORT error=not-approved"), "launch_aborted"),
        (Some("START extra"), "launch_aborted"),
        (None, "controller_eof_before_start"),
    ] {
        let fixture = Fixture::new();
        let before = snapshot(&fixture.source);
        let mut owner =
            Durable::spawn(&fixture, RUN_A, "tree", "ordinary", "printf forbidden > must-not-run");
        owner.preparing();
        assert_eq!(fields(&owner.directory.join("metadata.v2"))["execution_class"], "ordinary");
        assert!(!owner.directory.join("metadata.v1").exists());
        match control {
            Some(line) => owner.send(line),
            None => owner.disconnect(),
        }
        owner.terminal();
        assert_eq!(fields(&owner.directory.join("terminal.v1"))["error"], expected);
        assert_eq!(snapshot(&fixture.source), before);
        assert!(!owner.source.join("must-not-run").exists());
    }
}

/// CONTRACT_SHAPE: bounded-change (late control cannot replay a selected body;
/// exact command terminal survives controller disconnection).
#[test]
fn duplicate_start_never_replays_a_selected_command() {
    let fixture = Fixture::new();
    let count = fixture.temp.path().join("body-count");
    let body = format!("printf once >> '{}'; sleep 0.2; exit 255", count.display());
    let mut owner = Durable::spawn(&fixture, RUN_A, "tree", "ordinary", &body);
    owner.preparing();
    owner.send("START\nSTART");
    owner.disconnect();
    owner.terminal();
    assert_eq!(text(&count), "once");
    let terminal = fields(&owner.directory.join("terminal.v1"));
    assert_eq!(terminal["kind"], "command");
    assert_eq!(terminal["exit_code"], "255");
    assert_eq!(terminal["cause"], "command_returned");
}

/// CONTRACT_SHAPE: bounded-change (sync/pull own only their requested checkout
/// lock; a running owner excludes both without acquiring physical maintenance).
#[test]
fn sad_sync_and_pull_wait_on_the_running_checkout_without_source_mutation() {
    let fixture = Fixture::new();
    // The exact lock-path substitutions are the already-supported shell fixture
    // inputs, not permission for operators to split the physical admission domain.
    let lock = fixture.temp.path().join("tree.lock");
    let owner_path = fixture.temp.path().join("tree.owner");
    let mut holder = Command::new("bash")
        .arg(workspace_file("infra/metal/lease-holder.sh"))
        .arg(&lock)
        .env("OVERDRIVE_METAL_RUNS_ROOT", &fixture.runs)
        .arg(&owner_path)
        .args(["1", RUN_A, "run", "isolation", "/fixture", COMMIT])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("real checkout holder");
    for _ in 0..100 {
        if owner_path.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(owner_path.exists());
    for args in [vec!["--sync-only"], vec!["--pull", "--", "evidence"]] {
        let before = snapshot(&fixture.remote);
        let output = fixture
            .bootstrap(&fixture.source, &args)
            .env("OVERDRIVE_METAL_TREE_LOCK_PATH", &lock)
            .env("OVERDRIVE_METAL_TREE_OWNER_PATH", &owner_path)
            .output()
            .expect("competing source operation");
        runner_error(&output, "checkout_timeout", 75);
        assert_eq!(snapshot(&fixture.remote), before);
        assert_eq!(fields(&owner_path)["token"], RUN_A);
    }
    holder.stdin.take().expect("release stream").write_all(b"").expect("close holder");
    assert!(holder.wait().expect("reap checkout holder").success());
}

/// CONTRACT_SHAPE: bounded-change (one actual admission budget covers two real
/// contended locks; no selected body or unrelated source/owner state changes).
#[test]
fn sad_waiting_for_both_locks_cannot_restart_the_existing_admission_budget() {
    struct Holder(Option<std::process::Child>);
    impl Drop for Holder {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                drop(child.stdin.take());
                let _ = child.wait();
            }
        }
    }
    let fixture = Fixture::new();
    let hold = |name: &str, lock: &std::path::Path| {
        let owner = fixture.temp.path().join(format!("{name}.owner"));
        let child = Command::new("bash")
            .arg(workspace_file("infra/metal/lease-holder.sh"))
            .arg(lock)
            .env("OVERDRIVE_METAL_RUNS_ROOT", &fixture.runs)
            .arg(&owner)
            .args(["2", name, "run", "single-budget", "/fixture", COMMIT])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .expect("actual blocking holder");
        for _ in 0..100 {
            if owner.is_file() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(owner.is_file());
        Holder(Some(child))
    };
    let barrier = fixture.temp.path().join("maintenance.lock");
    let tree = fixture.temp.path().join("tree.lock");
    let barrier_holder = hold("maintenance-blocker", &barrier);
    let tree_holder = hold("checkout-blocker", &tree);
    let mut owner = Durable::spawn_with_timeout(
        &fixture,
        RUN_A,
        "tree",
        "ordinary",
        "printf forbidden > must-not-run",
        "2",
    );
    owner.wait_for(&owner.directory.join("phase.v1"), "phase=waiting_for_lease");
    thread::sleep(Duration::from_millis(1100));
    drop(tree_holder);
    let mut tree_held = false;
    for _ in 0..30 {
        let status = Command::new("/usr/bin/flock")
            .args(["--exclusive", "--nonblock"])
            .arg(&tree)
            .arg("true")
            .status()
            .expect("kernel lock observation");
        if status.code() == Some(1) {
            tree_held = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        tree_held,
        "actual ordinary owner must progress from checkout to maintenance contention"
    );
    thread::sleep(Duration::from_millis(1100));
    drop(barrier_holder);
    // A wrong new two-second budget reaches preparing after the barrier is freed.
    // End it via its real ABORT input so the eventual exact cause exposes that
    // defect without starting a body or leaving a fixture process behind.
    for _ in 0..100 {
        if owner.directory.join("terminal.v1").is_file() {
            break;
        }
        if owner.directory.join("phase.v1").is_file()
            && fields(&owner.directory.join("phase.v1"))["phase"] == "preparing"
        {
            owner.send("ABORT");
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    owner.terminal();
    let terminal = fields(&owner.directory.join("terminal.v1"));
    assert_eq!(terminal["kind"], "runner");
    assert_eq!(terminal["error"], "maintenance_timeout");
    assert!(!owner.source.join("must-not-run").exists());
}

/// CONTRACT_SHAPE: bounded-change (a run queued behind its own checkout holds
/// neither barrier mode; an unrelated shared admission is granted meanwhile).
#[test]
fn exclusive_run_waiting_for_its_checkout_never_holds_the_maintenance_barrier() {
    let fixture = Fixture::new();
    let mut first =
        Durable::spawn(&fixture, RUN_A, "same-tree", "ordinary", &waiting_body(&fixture, "held"));
    first.preparing();
    let mut second = Durable::spawn_with_timeout(
        &fixture,
        RUN_B,
        "same-tree",
        "exclusive",
        "printf forbidden > must-not-run",
        "30",
    );
    second.wait_for(&second.directory.join("phase.v1"), "phase=waiting_for_lease");
    thread::sleep(Duration::from_millis(500));
    let barrier = fixture.temp.path().join("maintenance.lock");
    let status = Command::new("/usr/bin/flock")
        .args(["--shared", "--nonblock"])
        .arg(&barrier)
        .arg("true")
        .status()
        .expect("kernel lock observation");
    assert_eq!(
        status.code(),
        Some(0),
        "a run waiting for its own checkout must not hold the node-wide barrier"
    );
    first.send("ABORT");
    first.terminal();
    drop(second);
}

/// CONTRACT_SHAPE: bounded-change (an exclusive admission pass costs the number
/// of unfinished runs, not the number of retained records; the barrier is held
/// only for that bounded pass).
#[test]
fn exclusive_admission_does_not_scale_with_retained_finished_runs() {
    let fixture = Fixture::new();
    for index in 0..400u32 {
        let id = format!("{:024x}", 0x1000 + index);
        fixture.retained(&id, 2, Some(("command", "0")), Some("complete"));
    }
    let mut exclusive = Durable::spawn_with_timeout(
        &fixture,
        RUN_A,
        "some-tree",
        "exclusive",
        "printf forbidden > must-not-run",
        "30",
    );
    // `wait_for` allows three seconds; a per-record validation pass over 400
    // retained records takes far longer than that.
    exclusive.preparing();
    exclusive.send("ABORT");
    exclusive.terminal();
}

/// CONTRACT_SHAPE: bounded-change (a finished run whose unit still reports
/// active is not an admitted ordinary unit; an unfinished one still is).
#[test]
fn exclusive_admission_waits_only_for_unfinished_ordinary_units() {
    let fixture = Fixture::new();
    fs::write(fixture.temp.path().join("unit-state"), "active\n").expect("units report live");
    let finished = format!("{:024x}", 0x2000);
    fixture.retained(&finished, 2, Some(("command", "0")), Some("complete"));
    let mut exclusive = Durable::spawn_with_timeout(
        &fixture,
        RUN_A,
        "tree-a",
        "exclusive",
        "printf forbidden > must-not-run",
        "30",
    );
    exclusive.preparing();
    exclusive.send("ABORT");
    exclusive.terminal();
    drop(exclusive);

    let unfinished = format!("{:024x}", 0x2001);
    let directory = fixture.retained(&unfinished, 2, None, None);
    fs::remove_file(directory.join("output.log")).expect("unfinished run has no log yet");
    let mut blocked =
        Durable::spawn(&fixture, RUN_B, "tree-b", "exclusive", "printf forbidden > must-not-run");
    blocked.terminal();
    let terminal = fields(&blocked.directory.join("terminal.v1"));
    assert_eq!(terminal["error"], "maintenance_timeout");
}

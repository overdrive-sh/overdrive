//! `metal cancel`: a registered run is stopped by its durable run id through its
//! own unit only. The supervisor's reaction to the stop signal is the real one;
//! the fixture `systemctl` models only the unit-stop signalling half.
use super::support::*;
use std::fs;

use overdrive_testing::placement;

placement!(host_pure);

/// CONTRACT_SHAPE: bounded-change (invalid/unknown ids change only output and
/// status; cancelling without a registration creates and cleans nothing).
#[test]
fn sad_cancel_without_a_run_preserves_absence() {
    let fixture = Fixture::new();
    for invalid in [
        "",
        "../x",
        "0123456789abcdef0123456",
        "0123456789abcdef012345678",
        "ABCDEF0123456789abcdef01",
    ] {
        runner_error(
            &fixture.cancel(invalid).output().expect("invalid run cancel"),
            "invalid_run_id",
            64,
        );
    }
    runner_error(&fixture.cancel(RUN_A).output().expect("unknown run cancel"), "run_not_found", 66);
    assert!(snapshot(&fixture.runs).is_empty());
}

/// CONTRACT_SHAPE: bounded-change (an already finished run is reported and every
/// retained record, including a contrasting run's, is byte-for-byte unchanged).
#[test]
fn cancelling_a_finished_run_reports_it_and_touches_nothing() {
    let fixture = Fixture::new();
    fixture.retained(RUN_A, 2, Some(("command", "0")), Some("complete"));
    fixture.retained(RUN_B, 2, Some(("command", "7")), Some("complete"));
    let before = snapshot(&fixture.runs);
    let output = fixture.cancel(RUN_A).output().expect("cancel finished run");
    success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OVERDRIVE_METAL_RUN_FINISHED run_id={RUN_A}\n")
    );
    assert_eq!(snapshot(&fixture.runs), before);
}

/// CONTRACT_SHAPE: bounded-change (the cancelled run's command ends, its terminal
/// records the supervisor stop and its checkout lock is released; the other run's
/// record and owner stay live and unchanged).
#[test]
fn cancelling_a_running_run_stops_its_command_and_releases_its_locks() {
    let fixture = Fixture::new();
    let started = fixture.temp.path().join("started");
    let body = format!("printf started > '{}'; exec sleep 30", started.display());
    let mut target = Durable::spawn(&fixture, RUN_A, "target-tree", "ordinary", &body);
    target.preparing();
    target.send("START");
    target.wait_for(&started, "started");
    target.wait_for(&target.directory.join("phase.v1"), "phase=running");
    let mut bystander = Durable::spawn(&fixture, RUN_B, "other-tree", "ordinary", &body);
    bystander.preparing();
    let bystander_owner = text(&bystander.directory.join("owner.v1"));
    fs::write(fixture.temp.path().join("unit-state"), "active\n").expect("unit observation");
    fs::write(fixture.temp.path().join("unit-pid"), target.pid().to_string())
        .expect("owner process of the unit");

    let output = fixture.cancel(&target.id).output().expect("cancel running run");
    success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OVERDRIVE_METAL_RUN_CANCELLED run_id={}\n", target.id)
    );
    target.terminal();
    let terminal = fields(&target.directory.join("terminal.v1"));
    assert_eq!(terminal["kind"], "command");
    assert_eq!(terminal["cause"], "supervisor_terminated");
    assert_eq!(terminal["exit_code"], "143", "the stopped command's own signal status");
    assert!(!target.directory.join("owner.v1").exists(), "owner record released");
    assert!(!fixture.temp.path().join("target-tree.owner").exists(), "checkout owner released");
    assert!(!target.is_live());

    assert!(bystander.is_live());
    assert_eq!(text(&bystander.directory.join("owner.v1")), bystander_owner);
    assert!(!bystander.directory.join("terminal.v1").exists());
}

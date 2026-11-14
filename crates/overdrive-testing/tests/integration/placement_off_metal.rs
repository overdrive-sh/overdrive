//! US-PLACE-01 AC (c) and (e): where a guest-placed body runs on and off a
//! Metal run. Each scenario runs the same fixture test as a child process with
//! a chosen environment; the oracle is a marker file on the host that the
//! fixture writes only when its body runs in that process, plus exit status and
//! output.

use super::scenario_support::{MARKER_ENV, METAL_VARIABLES, run_test, test_path};

const MARKER_TEST: &str = "isolated_guest_test_that_marks_where_it_ran";

fn marker_path(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("overdrive-placement-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("creating the marker directory");
    dir.join("marker")
}

/// Off a Metal run the machine is private: the body runs in the current
/// process, as it does today. CONTRACT_SHAPE: bounded-change (one marker file).
#[test]
#[ignore = "pending DELIVER: guest-placement-on-the-test"]
fn a_guest_placed_test_runs_its_body_in_place_when_no_metal_run_is_present() {
    let marker = marker_path("off-metal");
    let run = run_test(
        &test_path("shared_guest_fixtures::isolated_guest", MARKER_TEST),
        &[(MARKER_ENV, marker.to_str().expect("utf-8 path"))],
        &METAL_VARIABLES,
    );
    assert!(run.success, "the body runs in place and passes:\n{}", run.output);
    assert!(marker.exists(), "the body ran in this process:\n{}", run.output);
}

/// On a Metal run the body runs in a guest, never on the host (true of the
/// per-test guest today, kept true). CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: guest-placement-on-the-test (placement unresolved: the scenario drives guest boots from outside any guest, see design gap D-10)"]
fn a_guest_placed_test_never_runs_its_body_on_the_host_on_a_metal_run() {
    let marker = marker_path("on-metal");
    let run = run_test(
        &test_path("shared_guest_fixtures::isolated_guest", MARKER_TEST),
        &[(MARKER_ENV, marker.to_str().expect("utf-8 path"))],
        &[],
    );
    assert!(run.success, "{}", run.output);
    assert!(!marker.exists(), "the body ran on the host:\n{}", run.output);
}

/// A Metal context of any class counts as a Metal run: with the maintenance
/// class and an invalid run id the guest cannot start, and the test fails
/// naming its group and the stage, never running the body on the host (the
/// run-runtime rule refuses a set but malformed id; an absent id is not refused,
/// see the design-gap note in `distill/test-scenarios.md`).
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: guest-placement-on-the-test (placement unresolved: the scenario drives guest boots from outside any guest, see design gap D-10)"]
fn a_guest_placed_test_with_an_invalid_run_id_fails_naming_its_group_and_never_runs_on_the_host() {
    let marker = marker_path("exclusive");
    let run = run_test(
        &test_path("shared_guest_fixtures::isolated_guest", MARKER_TEST),
        &[
            (MARKER_ENV, marker.to_str().expect("utf-8 path")),
            ("OVERDRIVE_METAL_EXECUTION_CLASS", "exclusive"),
            ("OVERDRIVE_METAL_RUN_ID", "not-a-run-id"),
        ],
        &[],
    );
    assert!(!run.success, "the guest cannot start, so the test fails:\n{}", run.output);
    assert!(run.output.contains("cgroup-default"), "the failure names the group:\n{}", run.output);
    assert!(!marker.exists(), "the body ran on the host:\n{}", run.output);
}

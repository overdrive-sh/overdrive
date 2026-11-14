//! US-PLACE-01 AC (c) and (e): where a guest-placed body runs on and off a
//! Metal run. Each scenario runs the same fixture test as a child process with
//! a chosen environment; the oracle is a marker file on the host that the
//! fixture writes only when its body runs in that process, plus exit status and
//! output.

use overdrive_testing::overdrive_test;

use super::scenario_support::{MARKER_ENV, METAL_VARIABLES, run_test, test_path, timings};

const MARKER_TEST: &str = "isolated_guest_test_that_marks_where_it_ran";

fn marker_path(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("overdrive-placement-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("creating the marker directory");
    dir.join("marker")
}

/// Off a Metal run the machine is private: the body runs in the current
/// process, as it does today. CONTRACT_SHAPE: bounded-change (one marker file).
/// Placement on activation: `#[overdrive_test(host_pure)]` (it observes nothing
/// outside its own process). The behaviour is built by the guest-placement
/// slice (with its `isolated_guest` fixture); the test needs the host-pure form
/// to be placed on a Metal run, so it activates with `host-pure-placement-on-the-test`.
#[test]
#[overdrive_test(host_pure)]
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
/// Placement on activation: `#[overdrive_test(native_launcher)]` (it observes
/// the host from outside any guest).
#[test]
#[overdrive_test(native_launcher)]
fn a_guest_placed_test_never_runs_its_body_on_the_host_on_a_metal_run() {
    let marker = marker_path("on-metal");
    let run = run_test(
        &test_path("shared_guest_fixtures::isolated_guest", MARKER_TEST),
        &[(MARKER_ENV, marker.to_str().expect("utf-8 path"))],
        &[],
    );
    assert!(run.success, "{}", run.output);
    assert!(!marker.exists(), "the body ran on the host:\n{}", run.output);
    let lines = timings(&run.output);
    assert_eq!(lines.len(), 1, "exactly one guest booted for this test:\n{}", run.output);
    assert!(!lines[0].shared(), "an isolated guest, not a shared one:\n{}", run.output);
    assert_eq!(lines[0].field("group"), Some("cgroup-default"), "{}", run.output);
}

/// A Metal context of any class counts as a Metal run: with the maintenance
/// class and an invalid run id the guest cannot start, and the test fails
/// naming its group and the stage, never running the body on the host. The
/// run-runtime rule refuses a set but malformed run id; an absent run id with
/// another variable set is still handed to the guest, never run in place.
/// CONTRACT_SHAPE: bounded-change.
/// Placement on activation: `#[overdrive_test(native_launcher)]`.
#[test]
#[overdrive_test(native_launcher)]
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
    assert!(
        run.output.contains("not-a-run-id") || run.output.contains("OVERDRIVE_METAL_RUN_ID"),
        "the failure names the malformed id or the run-runtime stage that refused it:\n{}",
        run.output
    );
    assert!(!marker.exists(), "the body ran on the host:\n{}", run.output);
}

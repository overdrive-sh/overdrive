//! US-PLACE-03 at the library's driving port: a lane placement driven from
//! outside its lane package is refused before any guest boots, naming the
//! fixture and the test. (Fixture existence is owned by `cargo xtask
//! placement-lint`'s `missing-fixture` clause, not by this refusal.)

use overdrive_testing::guest_lane::OwnerLaneHost;
use overdrive_testing::overdrive_test;
use overdrive_testing::placement::{LanePlacement, RunInGuestError, run_lane};

/// The host script of the placement under test; the refusal comes before any
/// guest starts, so it never runs (and fails if it does).
fn no_host_script(_host: &mut OwnerLaneHost) -> Result<(), String> {
    Err("the refusal comes before the host script runs".to_owned())
}

/// CONTRACT_SHAPE: pure-function (a placement in, a typed failure out; no guest
/// is left behind). Placement on activation: `#[overdrive_test(native_launcher)]`
/// (it calls `run_lane` on the host).
#[test]
#[overdrive_test(native_launcher)]
fn a_lane_placement_driven_from_outside_its_lane_package_is_refused_naming_the_fixture_and_the_test()
 {
    let exe = std::env::current_exe().expect("this test binary");
    let fixture = "integration::in_guest_lane::a_fixture_nobody_wrote";
    let test = "integration::lane_placement::a_lane_test_that_cannot_run";
    let outcome = run_lane(&LanePlacement::owner(fixture, no_host_script), &exe, test);
    let error = outcome.expect_err("a lane driven from outside its lane package does not pass");
    // The hand-off refuses a lane placement used outside the lane package before
    // any guest starts: `LaneNotPrepared` (not a guest that ran and failed, not
    // a body failure), and its cause names the fixture.
    let RunInGuestError::LaneNotPrepared { test: refused_test, detail, .. } = &error else {
        panic!(
            "a lane placement outside its lane package is a refusal (LaneNotPrepared), not: {error}"
        );
    };
    assert_eq!(refused_test, test, "the refusal names the test: {error}");
    assert!(detail.contains(fixture), "the cause names the fixture: {error}");
}

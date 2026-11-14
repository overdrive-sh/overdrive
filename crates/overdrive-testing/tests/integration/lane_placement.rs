//! US-PLACE-03 at the library's driving port: a lane placement that names a
//! fixture its provider does not have fails naming the fixture and the test.

use overdrive_testing::placement::{LanePlacement, RunInGuestError, run_lane};

/// CONTRACT_SHAPE: pure-function (a placement in, a typed failure out; no guest
/// is left behind).
#[test]
#[ignore = "pending DELIVER: lane-and-launcher-placement-on-the-test"]
fn a_lane_test_naming_a_missing_fixture_fails_naming_the_fixture_and_the_test() {
    let exe = std::env::current_exe().expect("this test binary");
    let fixture = "integration::in_guest_lane::a_fixture_nobody_wrote";
    let test = "integration::lane_placement::a_lane_test_that_cannot_run";
    let outcome = run_lane(&LanePlacement::owner(fixture), &exe, test);
    let error = outcome.expect_err("a lane without its fixture does not pass");
    let text = error.to_string();
    assert!(text.contains(fixture), "the failure names the fixture: {text}");
    assert!(text.contains(test), "the failure names the test: {text}");
    assert!(
        !matches!(error, RunInGuestError::BodyFailed { .. }),
        "no body ran, so the body did not fail: {text}"
    );
}

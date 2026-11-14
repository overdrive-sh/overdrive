//! Fixtures the scenarios drive. Each is an ordinary test that passes under
//! nextest: its body asks to fail or to hold only when a scenario sets the
//! environment its guest request carries.
//!
//! `shared_module` declares one shared `cgroup-default` guest for its tests.
//! `scenario_clients` hands a named body to `run_in_guest` with a typed shared
//! placement of its own scope, so each owner scenario has a scope no other
//! test uses; the bodies run inside the guest and do nothing elsewhere.

use std::time::Duration;

use overdrive_testing::overdrive_test;
use overdrive_testing::placement::{GuestPlacement, Sharing, inside_guest, run_in_guest};

use super::scenario_support::{CLIENT_BODY_ENV, FAIL_ENV, HOLD_SECONDS_ENV, MARKER_ENV};

/// The body a scenario runs inside a guest: fail or hold when told to.
fn scenario_body() {
    if std::env::var_os(FAIL_ENV).is_some() {
        panic!("the scenario asked this test to fail");
    }
    if let Some(seconds) = std::env::var(HOLD_SECONDS_ENV).ok().and_then(|s| s.parse::<u64>().ok())
    {
        std::thread::sleep(Duration::from_secs(seconds));
    }
}

pub mod shared_module {
    //! Three tests of one module that share one `cgroup-default` guest, one
    //! that fails when told to, and one that keeps an explicit isolated guest.

    use super::{overdrive_test, scenario_body};

    overdrive_testing::shared_guest!(guest = "cgroup-default");

    #[overdrive_test]
    #[test]
    fn first_test_of_the_shared_module() {
        scenario_body();
    }

    #[overdrive_test]
    #[test]
    fn second_test_of_the_shared_module() {
        scenario_body();
    }

    #[overdrive_test]
    #[test]
    fn third_test_of_the_shared_module() {
        scenario_body();
    }

    #[overdrive_test]
    #[test]
    fn test_of_the_shared_module_that_fails_when_told_to() {
        scenario_body();
    }

    /// An explicit per-test form overrides the module's declaration: this test
    /// keeps its own isolated guest.
    #[overdrive_test(guest = "cgroup-default")]
    #[test]
    fn explicit_guest_test_in_the_shared_module() {
        scenario_body();
    }
}

pub mod isolated_guest {
    //! Guest-placed tests outside any shared-guest module: one guest each.

    use super::{MARKER_ENV, overdrive_test, scenario_body};

    #[overdrive_test(guest = "cgroup-default")]
    #[test]
    fn first_isolated_guest_test() {
        scenario_body();
    }

    #[overdrive_test(guest = "cgroup-default")]
    #[test]
    fn second_isolated_guest_test() {
        scenario_body();
    }

    /// Writes a host marker file only when its body runs in this process.
    #[overdrive_test(guest = "cgroup-default")]
    #[test]
    fn isolated_guest_test_that_marks_where_it_ran() {
        if let Some(path) = std::env::var_os(MARKER_ENV) {
            std::fs::write(path, b"ran").expect("writing the marker");
        }
    }
}

pub mod scenario_clients {
    //! One client per owner scenario. Under nextest each does nothing (no
    //! scenario names a body); a scenario sets `CLIENT_BODY_ENV` and runs one
    //! as a child process, which hands the body to its shared guest.

    use super::{
        CLIENT_BODY_ENV, GuestPlacement, Sharing, inside_guest, run_in_guest, scenario_body,
    };

    const ELECTION: GuestPlacement =
        GuestPlacement::declare("cgroup-default", "base", Sharing::Module, "scenario::election");
    const ORPHAN: GuestPlacement =
        GuestPlacement::declare("cgroup-default", "base", Sharing::Module, "scenario::orphan");
    const SIGNAL: GuestPlacement =
        GuestPlacement::declare("cgroup-default", "base", Sharing::Module, "scenario::signal");
    const CRASH: GuestPlacement =
        GuestPlacement::declare("cgroup-default", "base", Sharing::Module, "scenario::crash");
    const TAINT: GuestPlacement =
        GuestPlacement::declare("cgroup-default", "base", Sharing::Module, "scenario::taint");
    const FAILURE: GuestPlacement =
        GuestPlacement::declare("cgroup-default", "base", Sharing::Module, "scenario::failure");

    fn hand_to_the_shared_guest(placement: &GuestPlacement) {
        let Ok(body) = std::env::var(CLIENT_BODY_ENV) else { return };
        let exe = std::env::current_exe().expect("this test binary's path");
        if let Err(error) = run_in_guest(placement, &exe, &body) {
            panic!("{error}");
        }
    }

    #[test]
    fn election_client() {
        hand_to_the_shared_guest(&ELECTION);
    }

    #[test]
    fn orphan_client() {
        hand_to_the_shared_guest(&ORPHAN);
    }

    #[test]
    fn signal_client() {
        hand_to_the_shared_guest(&SIGNAL);
    }

    #[test]
    fn crash_client() {
        hand_to_the_shared_guest(&CRASH);
    }

    #[test]
    fn taint_client() {
        hand_to_the_shared_guest(&TAINT);
    }

    #[test]
    fn failure_client() {
        hand_to_the_shared_guest(&FAILURE);
    }

    /// The body the clients hand to the guest; outside a guest it does nothing.
    #[test]
    fn scenario_body_inside_the_guest() {
        if inside_guest() {
            scenario_body();
        }
    }
}

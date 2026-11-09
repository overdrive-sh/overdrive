# Placement rerun — guest-lane fixture context is lost before dispatch

This directory preserves the current placement capture copied from
/tmp/metal-native-remaining-20261109T072512Z-placement. The foreground
invocation reports exit 1 at source HEAD
e26f0448aaa09f692b7bb2be2d601a87ea79c43d; its dirty source status is retained
in foreground-invocation.json.

## Captured results

The direct-native and named-nested cases both passed, with run IDs
e6dff0eeabb316ade8cc2a40 and 0c0863c9f6923e640147a3dc. The selected
guest-lane operator attempt failed: run 57ad916f2b2fc3bf13ec8335 returned
aggregate status 100, and the foreground placement driver exited 1. Its
selected Nextest identity was:

    overdrive-init::bin/overdrive-init
    guest_flow::real_kernel::the_accept_path_matches_the_sim_per_connection

The raw output contains both:

    OVERDRIVE_METAL_TEST_ERROR error=lane_not_prepared
    guest group "" has no prepared attempt context

This detail comes from the dispatcher after the wrapper forwarded the attempt;
the wrapper's initial dispatcher/artifact gate did not emit the generic marker
alone. The capture contains no per-file remote stat, so it does not prove
which earlier file check would have failed if the wrapper had stopped there.

## Exact placement and dispatch path

infra/metal/test-placement.json contains the exact guest-lane-operator
descriptor for this selected identity, with fixture
integration::in_guest_lane::the_guest_accept_path_matches_the_sim_in_a_real_guest.
The fixture is valid and the selected identity is not missing or malformed.

infra/metal/test-wrapper.sh parses the descriptor into PLACEMENT_KIND,
PLACEMENT_GROUP, and PLACEMENT_FIXTURE. It exports
OVERDRIVE_FWD_GUEST_GROUP only for guest-group; for guest-lane descriptors the
value remains empty. It exports the actual selected test name in
OVERDRIVE_FWD_GUEST_EXACT, then invokes the existing
overdrive-fwd-guest-suite --nextest-wrapper dispatcher. The fixture field is
not passed to or consumed by that dispatcher.

In fwd_guest_suite.rs, nextest_wrapper_entry only passes the original runner
through when the selected NEXTEST_TEST_NAME itself starts with
integration::in_guest_lane::. This attempt's selected name starts with
guest_flow::real_kernel::, so that fixture-test branch does not apply. The
dispatcher then reads the unset OVERDRIVE_FWD_GUEST_GROUP as "", calls
nextest_guest_plan("", ...), which has no plan, and returns the observed
"guest group \"\" has no prepared attempt context" refusal. The current
placement-rerun-2026-11-09-065706Z failure was the earlier artifact-path gate;
this 072512Z run advances past it and exposes the missing guest-lane dispatch
route.

The existing lane setup and host-script helpers live in
crates/overdrive-init/tests/integration/in_guest_lane.rs; its run_cell uses
the prepared overdrive-init--overdrive-init test ELF through
GuestKit::run_in_guest_cell. The selected operator/owner descriptor fixture
names that existing context. The accepted contract requires executing the
selected guest cell once, without running the fixture test as an extra body.
The bounded private remediation is to route the already-validated guest-lane
descriptor through that existing lane setup/host-script path and preserve the
selected binary, test identity, and result. No new public API, placement kind,
group, body, or oracle is required.

The stable producer path is already corrected: guest-kit-prepare writes
target/guest-kit/in-guest/overdrive-init--overdrive-init, and the wrapper now
checks that path. The remaining failure is the dispatcher's missing guest-lane
plan selection, not a reason to move this identity into nested-boundary or
replay the fixture test body.

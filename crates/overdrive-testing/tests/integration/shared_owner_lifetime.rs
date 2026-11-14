//! The shared-guest owner against the real kernel: the invariants the
//! conformance replay does not drive (design decision N-4) and the boundary
//! scenarios of the lifecycle gates. Each scenario runs scenario clients
//! (`shared_guest_fixtures::scenario_clients`) as child processes, each in its
//! own scope, and reads the owner's processes and the guest kit's timing lines.
//!
//! Placement on activation: every scenario here takes
//! `#[overdrive_test(native_launcher)]` (each observes the owner and VMM
//! processes from outside any guest); no placement-table entry is ever added.
//!
//! Idle assumption: every scenario's clients follow one another within seconds,
//! far inside the owner's 30 s idle period (`SHARED_GUEST_IDLE`, design
//! DDD-17), so a scope has one owner unless the scenario kills it. Owners are
//! counted from a sampler that runs for the whole scenario, not from a single
//! look after the clients finish.
//!
//! Replayed-by-exemption invariants: `I_OWNER_UNIQUE` is proved by
//! `eight_simultaneous_first_arrivals_elect_exactly_one_owner`; `I_NO_ORPHAN` by
//! `killing_the_owner_during_a_guest_exec_leaves_no_vmm`.

use std::time::Duration;

use overdrive_testing::overdrive_test;

use super::scenario_support::{
    CLIENT_BODY_ENV, FAIL_ENV, HOLD_SECONDS_ENV, OwnerSampler, alive, descendants_of, kill,
    owners_in_this_run, run_test, shared_generations, spawn_test, test_path, timings, wait_until,
};

const CLIENTS: &str = "shared_guest_fixtures::scenario_clients";
const BODY: &str =
    "integration::shared_guest_fixtures::scenario_clients::scenario_body_inside_the_guest";
const SIGKILL: i32 = 9;

fn client(name: &str) -> String {
    test_path(CLIENTS, name)
}

/// `I_OWNER_UNIQUE`: eight test processes arrive together at a scope with no
/// owner; exactly one owner process starts and all eight pass on one guest.
/// CONTRACT_SHAPE: bounded-change (owner processes of one scope).
#[test]
#[overdrive_test(native_launcher)]
fn eight_simultaneous_first_arrivals_elect_exactly_one_owner() {
    let before = owners_in_this_run();
    let sampler = OwnerSampler::start();
    let environment = [(CLIENT_BODY_ENV, BODY)];
    #[allow(clippy::needless_collect, reason = "all eight clients start before any is awaited")]
    let clients: Vec<_> =
        (0..8).map(|_| spawn_test(&client("election_client"), &environment, false)).collect();
    let results: Vec<_> =
        clients.into_iter().map(super::scenario_support::Running::finish).collect();
    for (index, result) in results.iter().enumerate() {
        assert!(result.success, "arrival {index} passes:\n{}", result.output);
    }
    let started: Vec<_> = sampler.stop().difference(&before).copied().collect();
    assert_eq!(started.len(), 1, "exactly one owner started for eight first arrivals: {started:?}");
    let generations: std::collections::BTreeSet<u32> =
        results.iter().flat_map(|r| shared_generations(&r.output)).collect();
    assert_eq!(
        generations.len(),
        1,
        "the eight tests ran on one guest generation: {generations:?}"
    );
}

/// `I_NO_ORPHAN`: SIGKILL of the owner during a guest exec leaves no VMM; the
/// owner is init of its own PID namespace, so the kernel ends everything under
/// it. CONTRACT_SHAPE: bounded-change.
#[test]
#[overdrive_test(native_launcher)]
fn killing_the_owner_during_a_guest_exec_leaves_no_vmm() {
    let before = owners_in_this_run();
    let mut client_run = spawn_test(
        &client("orphan_client"),
        &[(CLIENT_BODY_ENV, BODY), (HOLD_SECONDS_ENV, "40")],
        false,
    );
    assert!(client_run.wait_for_line("shared=1", Duration::from_secs(120)), "the client attached");
    let owner = *owners_in_this_run()
        .difference(&before)
        .next()
        .expect("the owner of the scope is running");
    assert!(
        wait_until(Duration::from_secs(60), || !descendants_of(owner).is_empty()),
        "the owner started its VMM"
    );
    let under_owner = descendants_of(owner);
    kill(i32::try_from(owner).expect("a pid"), SIGKILL);
    assert!(
        wait_until(Duration::from_secs(5), || !alive(owner)
            && under_owner.iter().all(|p| !alive(*p))),
        "no process of the owner's tree outlives it: {:?}",
        under_owner.iter().filter(|p| alive(**p)).collect::<Vec<_>>()
    );
    let _ = client_run.child.kill();
    let _ = client_run.finish();
}

/// A group signal sent to the electing client's process group does not reach
/// the owner (it called `setsid`): the owner keeps serving the next client.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[overdrive_test(native_launcher)]
fn the_electing_client_killed_by_a_group_signal_leaves_the_owner_serving() {
    let before = owners_in_this_run();
    let sampler = OwnerSampler::start();
    let mut elector = spawn_test(
        &client("signal_client"),
        &[(CLIENT_BODY_ENV, BODY), (HOLD_SECONDS_ENV, "10")],
        true,
    );
    assert!(elector.wait_for_line("shared=1", Duration::from_secs(120)), "the elector attached");
    let owner = *owners_in_this_run().difference(&before).next().expect("the owner is running");
    kill(-i32::try_from(elector.pid()).expect("a pid"), SIGKILL);
    let _ = elector.finish();
    std::thread::sleep(Duration::from_secs(2));
    assert!(alive(owner), "the owner survived its elector's process group");
    let next = run_test(&client("signal_client"), &[(CLIENT_BODY_ENV, BODY)], &[]);
    assert!(next.success, "the next client is served:\n{}", next.output);
    let seen = sampler.stop();
    assert!(seen.contains(&owner), "the same owner served it");
    assert!(alive(owner), "the owner is still serving");
    assert_eq!(seen.difference(&before).count(), 1, "no second owner was elected: {seen:?}");
}

/// An owner that dies mid-module fails its attached tests with the owner stage
/// and the next client elects a new owner. CONTRACT_SHAPE: bounded-change.
#[test]
#[overdrive_test(native_launcher)]
fn an_owner_killed_midway_fails_its_attached_test_and_the_next_client_elects_a_new_owner() {
    let before = owners_in_this_run();
    let mut attached = spawn_test(
        &client("crash_client"),
        &[(CLIENT_BODY_ENV, BODY), (HOLD_SECONDS_ENV, "60")],
        false,
    );
    assert!(attached.wait_for_line("shared=1", Duration::from_secs(120)), "the client attached");
    let first_owner =
        *owners_in_this_run().difference(&before).next().expect("the owner is running");
    kill(i32::try_from(first_owner).expect("a pid"), SIGKILL);
    let lost = attached.finish();
    assert!(!lost.success, "the attached test fails:\n{}", lost.output);
    assert!(lost.output.contains("owner"), "the failure names the owner stage:\n{}", lost.output);
    assert!(wait_until(Duration::from_secs(5), || !alive(first_owner)), "the old owner is gone");

    let next = run_test(&client("crash_client"), &[(CLIENT_BODY_ENV, BODY)], &[]);
    assert!(next.success, "a new owner is elected and serves it:\n{}", next.output);
    let second_owner = *owners_in_this_run()
        .difference(&before)
        .find(|pid| **pid != first_owner)
        .expect("a new owner is running");
    assert_ne!(first_owner, second_owner);
}

/// A client that goes away while its test runs taints the guest: the next
/// client waits for that exec to end and runs on a replacement guest.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[overdrive_test(native_launcher)]
fn a_client_killed_while_its_test_runs_taints_the_guest_for_the_next_client() {
    let mut first = spawn_test(
        &client("taint_client"),
        &[(CLIENT_BODY_ENV, BODY), (HOLD_SECONDS_ENV, "8")],
        false,
    );
    assert!(first.wait_for_line("shared=1", Duration::from_secs(120)), "the first client attached");
    let attached = shared_generations(&first.finish_after_kill());
    let second = run_test(&client("taint_client"), &[(CLIENT_BODY_ENV, BODY)], &[]);
    assert!(second.success, "the next client is served:\n{}", second.output);
    let served = shared_generations(&second.output);
    assert!(!attached.is_empty(), "the killed client had attached to a generation");
    assert!(!served.is_empty(), "the next client attached to a generation:\n{}", second.output);
    assert!(
        served.iter().min() > attached.iter().max(),
        "the tainted generation was discarded: {attached:?} then {served:?}"
    );
}

/// The failed body of a shared exec is discarded like any failed test: the
/// verdict reaches the client and the next one boots again. Companion of the
/// seeded `OwnerCore` run on the real kernel. CONTRACT_SHAPE: bounded-change.
#[test]
#[overdrive_test(native_launcher)]
fn a_failed_shared_test_is_reported_with_its_status_and_its_guest_is_replaced() {
    let failed =
        run_test(&client("failure_client"), &[(CLIENT_BODY_ENV, BODY), (FAIL_ENV, "1")], &[]);
    assert!(!failed.success, "{}", failed.output);
    let next = run_test(&client("failure_client"), &[(CLIENT_BODY_ENV, BODY)], &[]);
    assert!(next.success, "{}", next.output);
    let (a, b) = (shared_generations(&failed.output), shared_generations(&next.output));
    assert!(!a.is_empty(), "the failed test attached a generation:\n{}", failed.output);
    assert!(!b.is_empty(), "the next test attached a generation:\n{}", next.output);
    assert!(b.iter().min() > a.iter().max(), "{a:?} then {b:?}");
    assert_eq!(timings(&next.output).iter().filter(|t| t.shared()).count(), 1);
}

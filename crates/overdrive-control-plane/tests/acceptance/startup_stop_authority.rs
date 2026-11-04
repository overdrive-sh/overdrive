//! A startup-probe failure stops the allocation; `WorkloadLifecycle` alone
//! restarts it and names the failure at give-up (feature
//! `service-allocation-readiness-and-ending`: D7, D8, B2; obligations S-22,
//! S-28, S-30 of `feature-delta.md` § Executable obligations for DISTILL).
//!
//! Composition: `restart_authority_world` (production runtime with both
//! reconcilers, action shim, exit observer, probe runner and Service stream;
//! Sim adapters at the driven ports). Tier 1 composed, seeded (seed printed).
//! Model: `single_restart_authority_liveness_trajectory.rs`.
//!
//! CONTRACT_SHAPE: bounded-change.

#![allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "scenario bodies are linear; the CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::time::Duration;

use overdrive_control_plane::streaming::ServiceSubmitEvent;
use overdrive_core::id::AllocationId;
use overdrive_core::transition_reason::ServiceFailureReason;
use overdrive_reconcilers::RESTART_BACKOFF_CEILING;

use crate::acceptance::restart_authority_world::{
    Dial, DriverCall, RestartWorld, Shape, assert_universe_unchanged, settle,
};

fn startup_stopped(row: &overdrive_core::traits::observation_store::AllocStatusRow) -> bool {
    format!("{:?}", row.state) == "Terminated" && format!("{:?}", row.terminal).contains("StartupProbe")
}

/// S-22 (short startup deadline): a Service whose guest stays alive and never
/// passes. Each attempt is stopped (row `Terminated` / `Stopped { StartupProbe }`,
/// guest stopped before the successor starts, supervisor gone), the stream stays
/// open across every stop, and at give-up the first terminal is
/// `Failed { StartupProbeFailed { 0, last_fail, 5 } }` with the row terminal
/// `ServiceFailed { StartupProbeFailed }`.
#[tokio::test]
#[ignore = "pending DELIVER: D7 a startup-probe failure stops the allocation and the restart authority names it at give-up"]
async fn never_ready_guest_is_stopped_and_restarted_until_give_up_names_the_startup_failure() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let stream = world.open_stream().await;
    let mut alloc = world.place_first_attempt().await;
    let mut attempts: Vec<AllocationId> = Vec::new();

    for attempt in 0..=RESTART_BACKOFF_CEILING {
        world.never_ready_until_the_look(2).await.expect("service-lifecycle stops the attempt");
        let row = world.row(&alloc).await.expect("attempt row");
        assert!(startup_stopped(&row), "attempt {attempt}: Terminated / Stopped {{ StartupProbe }}, got {row:?}");
        assert!(!stream.closed(), "attempt {attempt}: the stream stays open across the stop");
        assert_eq!(
            world.observe().await.get("supervised_allocations").map(String::as_str),
            Some("0"),
            "attempt {attempt}: the stopped attempt's probe supervisor is gone"
        );
        attempts.push(alloc.clone());
        if attempt < RESTART_BACKOFF_CEILING {
            alloc = world.restart_successor().await;
        }
    }
    world.advance(Duration::from_secs(2)).await;
    world.workload_tick().await.expect("workload-lifecycle gives up at the ceiling");
    settle().await;

    let calls = world.driver.calls();
    for pair in attempts.windows(2) {
        let stopped = calls.iter().position(|c| *c == DriverCall::Stop(pair[0].clone()));
        let started = calls.iter().position(|c| *c == DriverCall::Start(pair[1].clone()));
        assert!(
            stopped.is_some() && started.is_some() && stopped < started,
            "the guest of {} is stopped before its successor starts: {calls:?}",
            pair[0]
        );
    }
    match stream.first_terminal() {
        Some(ServiceSubmitEvent::Failed {
            reason: ServiceFailureReason::StartupProbeFailed { probe_idx: 0, attempts: 5, last_fail },
            ..
        }) => assert!(!last_fail.is_empty(), "last_fail is the final attempt's probe failure reason"),
        other => panic!("expected Failed {{ StartupProbeFailed {{ 0, _, 5 }} }}, got {other:?}"),
    }
    let last = world.row(&alloc).await.expect("final row");
    assert!(format!("{:?}", last.terminal).contains("StartupProbeFailed"));
    for id in &attempts[..attempts.len() - 1] {
        assert!(startup_stopped(&world.row(id).await.expect("row")), "predecessor {id} keeps its stop");
    }
}

/// S-22 (default startup deadline): the progress deadline is reached first. The
/// first terminal is `Failed { Timeout { 90 } }` carrying the first attempt's
/// startup stop as `last_attempt`; the give-up arrives on the row later.
#[tokio::test]
#[ignore = "pending DELIVER: D7 startup stop; D10 progress deadline carries the last attempt"]
async fn never_ready_guest_with_the_default_startup_window_times_out_naming_the_first_stop() {
    let mut world = RestartWorld::compose(Shape::DefaultStartupWindow).await;
    let stream = world.open_stream().await;
    let first = world.place_first_attempt().await;
    let every = Duration::from_secs(2);
    for _ in 0..30 {
        world.startup_period_for(Dial::Refuses, every).await;
    }
    world.service_tick().await.expect("service-lifecycle stops the first attempt at its deadline");
    let _second = world.restart_successor().await;
    for _ in 0..20 {
        if stream.closed() {
            break;
        }
        world.startup_period_for(Dial::Refuses, every).await;
    }
    settle().await;

    match stream.first_terminal() {
        Some(ServiceSubmitEvent::Failed {
            reason: ServiceFailureReason::Timeout { after_seconds: 90 },
            last_attempt: Some(last),
            ..
        }) => {
            assert_eq!(last.alloc_id, first.to_string(), "the detail names the first stopped attempt");
            assert!(format!("{:?}", last.terminal).contains("StartupProbe"));
        }
        other => panic!("expected Failed {{ Timeout {{ 90 }} }} with last_attempt, got {other:?}"),
    }
}

/// S-22 (a successor that binds): the stream ends `Stable`, and no
/// `StartupProbeFailed` is ever written.
#[tokio::test]
#[ignore = "pending DELIVER: D7 a startup-probe failure stops the allocation"]
async fn successor_that_binds_after_a_startup_stop_ends_the_stream_stable() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let stream = world.open_stream().await;
    let first = world.place_first_attempt().await;
    world.never_ready_until_the_look(2).await.expect("the first attempt is stopped");
    let successor = world.restart_successor().await;
    world.startup_period(Dial::Binds).await;
    world.service_tick().await.expect("look at the binding successor");
    settle().await;

    match stream.first_terminal() {
        Some(ServiceSubmitEvent::Stable { alloc_id, .. }) => assert_eq!(alloc_id, successor.to_string()),
        other => panic!("expected Stable, got {other:?}"),
    }
    let rows = world.rows().await;
    assert!(startup_stopped(rows.get(&first).expect("first attempt")));
    assert!(rows.values().all(|r| !format!("{:?}", r.terminal).contains("StartupProbeFailed")));
}

/// S-22 (opt-out): an opt-out Service never receives the startup disposition;
/// its first `Running` is its `Stable`.
#[tokio::test]
async fn opt_out_service_never_receives_the_startup_stop() {
    let mut world = RestartWorld::compose(Shape::OptOut).await;
    let alloc = world.place_first_attempt().await;
    world.service_tick().await.expect("service-lifecycle looks at the Running row");
    for _ in 0..5 {
        world.startup_period(Dial::Refuses).await;
        world.service_tick().await.expect("look");
    }
    let row = world.row(&alloc).await.expect("row");
    assert!(!format!("{:?}", row.terminal).contains("StartupProbe"));
    assert_eq!(world.driver.stop_calls_for(&alloc), 0, "an opt-out Service is never stopped for a startup failure");
}

/// S-28 (E-2): the guest crashes while the stop is dispatched, after the
/// evaluation and before the stop row. The written row is `Terminated` /
/// `Stopped { StartupProbe }` (the crash cause of that attempt is lost; restart,
/// budget and termination are unchanged) and the give-up names the startup
/// failure.
#[tokio::test]
#[ignore = "pending DELIVER: D7 a startup-probe failure stops the allocation and the restart authority names it at give-up"]
async fn crash_racing_the_startup_stop_leaves_the_stop_row_and_the_give_up_names_startup() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let mut alloc = world.place_first_attempt().await;
    for _ in 0..RESTART_BACKOFF_CEILING {
        world.never_ready_until_the_look(2).await.expect("the attempt is stopped");
        alloc = world.restart_successor().await;
    }
    world.crash_while_next_stop_is_dispatched(&alloc, 1);
    world.never_ready_until_the_look(2).await.expect("the final attempt's stop is dispatched");

    let row = world.row(&alloc).await.expect("final row");
    assert!(startup_stopped(&row), "the stop row overwrites the racing crash row: {row:?}");
    world.advance(Duration::from_secs(2)).await;
    world.workload_tick().await.expect("workload-lifecycle gives up at the ceiling");
    let row = world.row(&alloc).await.expect("final row");
    assert!(
        format!("{:?}", row.terminal).contains("StartupProbeFailed"),
        "the give-up names the startup failure: {:?}",
        row.terminal
    );
}

/// S-30 (D8): the first N stops fail; the stop is re-emitted exactly once per
/// new startup row (a look with no new row emits nothing); the row becomes
/// `Terminated` / `Stopped { StartupProbe }` after the first success; the
/// successor starts only after the stop landed.
#[tokio::test]
#[ignore = "pending DELIVER: D8 a startup stop that does not land is retried once per new written startup probe row"]
async fn failed_startup_stop_is_retried_once_per_new_startup_row() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let first = world.place_first_attempt().await;
    world.script_stop_failure(Some("scripted Driver::stop failure"));

    assert!(
        world.never_ready_until_the_look(2).await.is_err(),
        "the failed stop surfaces as the tick's driver error"
    );
    assert_eq!(world.driver.stop_calls_for(&first), 1);
    assert_eq!(format!("{:?}", world.row(&first).await.expect("row").state), "Running");

    let before = world.observe().await;
    let _ = world.service_tick().await;
    assert_universe_unchanged(&before, &world.observe().await);
    assert_eq!(world.driver.stop_calls_for(&first), 1, "no new startup row, no new stop");

    world.startup_period(Dial::Refuses).await;
    assert!(world.service_tick().await.is_err(), "the retry on the new row fails again");
    assert_eq!(world.driver.stop_calls_for(&first), 2);

    world.script_stop_failure(None);
    world.startup_period(Dial::Refuses).await;
    world.service_tick().await.expect("the third stop lands");
    assert_eq!(world.driver.stop_calls_for(&first), 3);
    assert!(startup_stopped(&world.row(&first).await.expect("row")));

    let successor = world.restart_successor().await;
    let calls = world.driver.calls();
    let stopped = calls.iter().rposition(|c| *c == DriverCall::Stop(first.clone()));
    let started = calls.iter().position(|c| *c == DriverCall::Start(successor.clone()));
    assert!(stopped < started, "the successor starts only after the stop landed: {calls:?}");
}

/// S-30 (D8, late `Pass`): the stop fails, then the startup probe passes: no
/// further stop is emitted and the allocation reaches `Stable`.
#[tokio::test]
#[ignore = "pending DELIVER: D8 a startup pass after a stop that did not land ends the retry and reaches Stable"]
async fn late_startup_pass_after_a_failed_stop_ends_the_retry_and_reaches_stable() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let first = world.place_first_attempt().await;
    world.script_stop_failure(Some("scripted Driver::stop failure"));
    assert!(world.never_ready_until_the_look(2).await.is_err());
    assert_eq!(world.driver.stop_calls_for(&first), 1);

    world.script_stop_failure(None);
    world.startup_period(Dial::Binds).await;
    world.service_tick().await.expect("look after the late pass");
    assert_eq!(world.driver.stop_calls_for(&first), 1, "no further stop after a Pass");
    let row = world.row(&first).await.expect("row");
    assert_eq!(format!("{:?}", row.state), "Running");
    assert!(format!("{:?}", row.terminal).contains("Stable"), "the instance reaches Stable: {:?}", row.terminal);
}

/// S-30 (B2): the stop's terminal row write fails once. The supervisor is still
/// registered after the failed write, the next probe row re-emits the stop, the
/// retry finds the guest stopped (not-found tolerated), writes the row, and
/// only then does the supervised-allocation count fall.
#[tokio::test]
#[ignore = "pending DELIVER: B2 on_alloc_terminal fires after the stop's terminal row write succeeds"]
async fn failed_stop_row_write_keeps_the_supervisor_and_the_next_probe_row_retries() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let first = world.place_first_attempt().await;
    world.startup_period(Dial::Refuses).await;
    world.startup_period(Dial::Refuses).await;
    assert_eq!(world.observe().await.get("supervised_allocations").map(String::as_str), Some("1"));

    world.fail_next_observation_write();
    assert!(world.service_tick().await.is_err(), "the failed row write surfaces as the tick's error");
    assert_eq!(format!("{:?}", world.row(&first).await.expect("row").state), "Running");
    assert_eq!(
        world.observe().await.get("supervised_allocations").map(String::as_str),
        Some("1"),
        "the supervisor is still registered after the failed write"
    );

    world.startup_period(Dial::Refuses).await;
    world.service_tick().await.expect("the retry finds the guest stopped and writes the row");
    assert_eq!(world.driver.stop_calls_for(&first), 2);
    assert!(startup_stopped(&world.row(&first).await.expect("row")));
    assert_eq!(world.observe().await.get("supervised_allocations").map(String::as_str), Some("0"));
}

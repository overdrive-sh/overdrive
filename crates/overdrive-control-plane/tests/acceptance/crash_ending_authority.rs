//! A crashed Service allocation's ending belongs to `WorkloadLifecycle` alone
//! (feature `service-allocation-readiness-and-ending`: D5, D6, B3; obligations
//! S-10, S-16, S-45 of `feature-delta.md` § Executable obligations for DISTILL).
//!
//! Composition (`restart_authority_world`): the production runtime with
//! `service-lifecycle` and `workload-lifecycle`, the production action shim,
//! the production exit observer (the author of every crash row: no crash row is
//! seeded), the production probe runner and the production Service stream, with
//! Sim adapters at the driven ports. Tier 1 composed, seeded (seed printed).
//!
//! CONTRACT_SHAPE: bounded-change.

#![allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "scenario bodies are linear; the CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use overdrive_control_plane::streaming::ServiceSubmitEvent;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::driver::OomFacts;
use overdrive_core::transition_reason::ServiceFailureReason;
use overdrive_reconcilers::RESTART_BACKOFF_CEILING;

use crate::acceptance::restart_authority_world::{
    Dial, RestartWorld, Shape, assert_universe_unchanged,
};

/// Crash every attempt through the production exit observer, with a
/// `service-lifecycle` look after each crash, restarting until the budget is
/// spent; the last call is the `workload-lifecycle` tick at the ceiling.
/// Returns the final attempt.
async fn crash_loop_to_the_ceiling(
    world: &mut RestartWorld,
    exit_code: Option<i32>,
    oom: Option<OomFacts>,
) -> AllocationId {
    let mut alloc = world.place_first_attempt().await;
    for _ in 0..RESTART_BACKOFF_CEILING {
        world.guest_exits(&alloc, exit_code, oom).await;
        world.service_tick().await.expect("service-lifecycle looks at the crashed row");
        alloc = world.restart_successor().await;
    }
    world.guest_exits(&alloc, exit_code, oom).await;
    world.service_tick().await.expect("service-lifecycle looks at the crashed row");
    world.advance(std::time::Duration::from_secs(2)).await;
    world.workload_tick().await.expect("workload-lifecycle decides at the ceiling");
    alloc
}

/// S-10 (D5 edge, first variant): a crashed allocation whose startup streak
/// reached `max_attempts` at the deadline, looked at first after the crash.
/// `ServiceLifecycle` writes nothing; `WorkloadLifecycle` restarts it; the
/// recorded cause stays the crash; the probe supervisor is gone (D1).
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D5 startup verdict announced only for a non-terminal allocation"]
async fn crashed_allocation_with_exhausted_startup_probes_is_restarted_not_finalised() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let first = world.place_first_attempt().await;
    world.service_tick().await.expect("first look of the Running row");
    world.startup_period(Dial::Refuses).await;
    world.startup_period(Dial::Refuses).await;
    world.guest_exits(&first, Some(1), None).await;
    let crashed = world.observe().await;
    assert_eq!(
        crashed.get("supervised_allocations").map(String::as_str),
        Some("0"),
        "the exit observer ends probe supervision at the crash (D1)"
    );

    let before = world.observe().await;
    world.service_tick().await.expect("service-lifecycle looks at the crashed row");
    let after = world.observe().await;
    assert_universe_unchanged(&before, &after);
    let row = world.row(&first).await.expect("the crashed row");
    assert_eq!(row.terminal, None, "ServiceLifecycle announces no verdict over a crashed row");
    assert!(format!("{:?}", row.reason).contains("WorkloadCrashedImmediately"));

    let successor = world.restart_successor().await;
    assert_ne!(successor, first, "the restart authority placed a fresh attempt");
    let predecessor = world.row(&first).await.expect("the predecessor row");
    assert_eq!(predecessor.terminal, None, "the predecessor stays observable as its crash");
}

/// S-10 (second variant): the crash lands after the deadline and before the
/// `Running` row was ever evaluated.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D5 startup verdict announced only for a non-terminal allocation"]
async fn crash_after_the_deadline_before_the_first_look_is_restarted_not_finalised() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let first = world.place_first_attempt().await;
    world.startup_period(Dial::Refuses).await;
    world.startup_period(Dial::Refuses).await;
    world.advance(std::time::Duration::from_secs(5)).await;
    world.guest_exits(&first, Some(1), None).await;

    let before = world.observe().await;
    world.service_tick().await.expect("service-lifecycle looks at the crashed row");
    let after = world.observe().await;
    assert_universe_unchanged(&before, &after);
    assert_eq!(world.row(&first).await.expect("row").terminal, None);
    let successor = world.restart_successor().await;
    assert_ne!(successor, first);
}

/// S-10 (Q3 consequence, BC-1): a crashed allocation past its startup deadline
/// with no `Pass` and a streak below `max_attempts`. Post-crash probing no
/// longer grows the streak; no `StartupProbeFailed` finalisation arises.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
async fn crash_with_a_partial_streak_is_not_probed_after_the_crash() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let first = world.place_first_attempt().await;
    world.startup_period(Dial::Refuses).await;
    world.guest_exits(&first, Some(1), None).await;
    let rows_at_the_crash = world.startup_row(&first).await.map(|r| r.run_seq.get());

    for _ in 0..5 {
        world.startup_period(Dial::Refuses).await;
    }
    let before = world.observe().await;
    world.service_tick().await.expect("service-lifecycle looks at the crashed row");
    let after = world.observe().await;
    assert_universe_unchanged(&before, &after);
    assert_eq!(
        world.startup_row(&first).await.map(|r| r.run_seq.get()),
        rows_at_the_crash,
        "no startup row is written for a dead guest"
    );
    assert_eq!(world.row(&first).await.expect("row").terminal, None);
}

/// S-16 (a): a declared startup probe, the guest exits 1 on every start. The
/// stream stays open through every restart and ends at give-up with
/// `EarlyExit { exit_code: Some(1) }` and the stderr tail, no earlier than the
/// last attempt's crash; every predecessor row stays `Failed` with its crash;
/// the only terminal write is `WorkloadLifecycle`'s.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D6 restart authority names a pre-startup crash loop at give-up"]
async fn pre_startup_crash_loop_ends_the_stream_with_early_exit_at_give_up() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let stream = world.open_stream().await;
    let first = world.place_first_attempt().await;
    world.guest_exits(&first, Some(1), None).await;
    world.service_tick().await.expect("service-lifecycle looks at the crashed row");
    assert!(!stream.closed(), "the first crash does not end the deploy");

    let mut alloc = world.restart_successor().await;
    for _ in 1..RESTART_BACKOFF_CEILING {
        world.guest_exits(&alloc, Some(1), None).await;
        world.service_tick().await.expect("look");
        assert!(!stream.closed(), "the stream stays open through every restart");
        alloc = world.restart_successor().await;
    }
    world.guest_exits(&alloc, Some(1), None).await;
    world.service_tick().await.expect("look");
    assert!(!stream.closed(), "the stream stays open until the budget is spent");
    world.advance(std::time::Duration::from_secs(2)).await;
    world.workload_tick().await.expect("workload-lifecycle gives up at the ceiling");
    crate::acceptance::restart_authority_world::settle().await;

    match stream.first_terminal() {
        Some(ServiceSubmitEvent::Failed {
            reason: ServiceFailureReason::EarlyExit { exit_code: Some(1) },
            stderr_tail,
            ..
        }) => assert!(stderr_tail.is_some(), "the stderr tail rides the give-up"),
        other => panic!("expected Failed {{ EarlyExit {{ exit_code: Some(1) }} }}, got {other:?}"),
    }
    let rows = world.rows().await;
    let finalised: Vec<_> = rows.values().filter(|r| r.terminal.is_some()).collect();
    assert_eq!(finalised.len(), 1, "exactly one row carries a terminal: the final attempt's");
    assert_eq!(finalised[0].alloc_id, alloc);
    assert!(
        format!("{:?}", finalised[0].terminal).contains("EarlyExit"),
        "the row terminal is ServiceFailed {{ EarlyExit }}"
    );
    for (id, row) in &rows {
        if *id != alloc {
            assert!(format!("{:?}", row.reason).contains("WorkloadCrashedImmediately"));
        }
    }
    assert_eq!(
        world.driver.terminal_hook_calls(),
        usize::try_from(RESTART_BACKOFF_CEILING + 1).unwrap(),
        "one terminal hook per crashed attempt (D1); ServiceLifecycle drives no teardown effect"
    );
}

/// S-16 (b): the first attempt crashes before a pass, the successor passes its
/// startup probe: the stream ends `Stable` and no `EarlyExit` is written.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D6 ServiceLifecycle no longer finalises a crashed allocation"]
async fn pre_startup_crash_then_a_passing_successor_ends_the_stream_stable() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let stream = world.open_stream().await;
    let first = world.place_first_attempt().await;
    world.guest_exits(&first, Some(1), None).await;
    world.service_tick().await.expect("look at the crashed row");
    assert!(!stream.closed(), "the crash alone does not end the deploy");

    let successor = world.restart_successor().await;
    world.startup_period(Dial::Binds).await;
    world.service_tick().await.expect("look at the passing successor");
    crate::acceptance::restart_authority_world::settle().await;

    match stream.first_terminal() {
        Some(ServiceSubmitEvent::Stable { alloc_id, .. }) => {
            assert_eq!(alloc_id, successor.to_string());
        }
        other => panic!("expected Stable, got {other:?}"),
    }
    let rows = world.rows().await;
    assert!(
        rows.values().all(|r| !format!("{:?}", r.terminal).contains("EarlyExit")),
        "no EarlyExit is ever written"
    );
    assert_eq!(rows.get(&first).expect("predecessor").terminal, None);
}

/// S-16 (c): the same loop with the guest crashing after a recorded startup
/// `Pass` ends `BackoffExhausted`.
#[tokio::test]
async fn crash_loop_after_a_recorded_startup_pass_gives_up_backoff_exhausted() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let stream = world.open_stream().await;
    let mut alloc = world.place_first_attempt().await;
    for _ in 0..RESTART_BACKOFF_CEILING {
        world.startup_period(Dial::Binds).await;
        world.guest_exits(&alloc, Some(1), None).await;
        alloc = world.restart_successor().await;
    }
    world.startup_period(Dial::Binds).await;
    world.guest_exits(&alloc, Some(1), None).await;
    world.advance(std::time::Duration::from_secs(2)).await;
    world.workload_tick().await.expect("workload-lifecycle gives up at the ceiling");
    crate::acceptance::restart_authority_world::settle().await;

    match stream.first_terminal() {
        Some(ServiceSubmitEvent::Failed { reason, .. }) => assert!(
            matches!(reason, ServiceFailureReason::BackoffExhausted { .. }),
            "a crash after a recorded pass is BackoffExhausted, got {reason:?}"
        ),
        other => panic!("expected Failed, got {other:?}"),
    }
}

/// S-16 (d): a control-plane restart between the last crash and the ceiling
/// decision re-derives the same terminal from the durable rows.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D6 restart authority names a pre-startup crash loop at give-up"]
async fn control_plane_restart_before_the_ceiling_decision_rederives_early_exit() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let mut alloc = world.place_first_attempt().await;
    for _ in 0..RESTART_BACKOFF_CEILING {
        world.guest_exits(&alloc, Some(1), None).await;
        alloc = world.restart_successor().await;
    }
    world.guest_exits(&alloc, Some(1), None).await;
    world.restart_control_plane().await;
    world.advance(std::time::Duration::from_secs(2)).await;
    world.workload_tick().await.expect("workload-lifecycle gives up at the ceiling");

    let row = world.row(&alloc).await.expect("final row");
    assert!(
        format!("{:?}", row.terminal).contains("EarlyExit"),
        "the terminal is re-derived from the durable rows: {:?}",
        row.terminal
    );
}

/// S-45 (B3): the guest is killed by the cgroup OOM killer on every start; the
/// give-up is `EarlyExit { exit_code: None }`.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: B3 pre-startup crash predicate accepts VmOutOfMemory"]
async fn out_of_memory_crash_loop_gives_up_as_early_exit_without_an_exit_code() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let stream = world.open_stream().await;
    let oom = OomFacts { limit_bytes: 128 * 1024 * 1024, oom_kill_count: 1 };
    let alloc = crash_loop_to_the_ceiling(&mut world, None, Some(oom)).await;
    crate::acceptance::restart_authority_world::settle().await;

    match stream.first_terminal() {
        Some(ServiceSubmitEvent::Failed {
            reason: ServiceFailureReason::EarlyExit { exit_code: None },
            ..
        }) => {}
        other => panic!("expected Failed {{ EarlyExit {{ exit_code: None }} }}, got {other:?}"),
    }
    let row = world.row(&alloc).await.expect("final row");
    assert!(format!("{:?}", row.terminal).contains("EarlyExit"));
    let before = world.observe().await;
    world.workload_tick().await.expect("an already-finalised row emits nothing");
    assert_universe_unchanged(&before, &world.observe().await);
}

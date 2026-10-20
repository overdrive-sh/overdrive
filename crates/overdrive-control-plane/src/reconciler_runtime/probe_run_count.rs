//! R5-PROBE-COUNT — probe-run counting through the composed production
//! owners (netns-density-295: PROBE-RUN-COUNT with D-1, ADR-0191;
//! PROBE-NOSAMPLE, ADR-0187; PROBE-NOSAMPLE-EVENT, ADR-0192).
//!
//! Contract: every probe run counts exactly once iff its row is written —
//! the probe task numbers its written rows (`run_seq`) and records the
//! observed streak (`same_status_runs`); the store keeps the row with the
//! greater `run_seq`; `ServiceLifecycle` keeps no streak, applies each
//! threshold to the latest row at read time, terminates a liveness probe
//! once per crossing row (`liveness_last_acted_run`) and retries a failed
//! stop on the next crossed row (D-1). A run that writes no row changes
//! nothing and emits one `probe.not_sampled` event.
//!
//! Production owner path: the probe runner's tasks (`ProbeRunner::start_alloc`)
//! → `ObservationStore::write_probe_result` → `ServiceLifecycle` hydration
//! and reconcile through `run_convergence_tick` → the action shim's
//! `StopAllocation` dispatch → `Driver::stop` and the terminal
//! `AllocStatusRow`. Composition: `probe_count_fixture`.
//!
//! Fault stimuli (each against the healthy baseline of one look per written
//! run): (a) coalesced looks, (b) adapter error, (c) write failure,
//! (d) dial refused `Capacity` / `Quiesced`, (e) backward wall-clock step,
//! (f) resubmit after GC, (g) a status flip between two looks, (h) a failed
//! stop. Oracle: the stored rows and their counts, the `Driver::stop` calls
//! (the production evidence for `NoReactOnSameRow` / `NoMissedTerminate`),
//! `liveness_last_acted_run`, the allocation's terminal row, the published
//! backend eligibility and the `probe.not_sampled` events. Bounds: at most 6
//! runs per key and 3 looks per run; cleanup by dropping the composition.
//! Evidence tier: seeded sim (Sim adapters at the driven ports, production
//! owners). Pairs with `specs/quint/probe-count` (`quint_probe_count.rs`)
//! and discharges PC-1 to PC-4.
//!
//! Every body is pending DELIVER step 9, which lands the probe runner's VM
//! dial dispatch and run numbering, the merge by `run_seq` and the
//! read-time threshold rule.

#![allow(
    clippy::single_option_map,
    clippy::expect_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "scenario bodies assert their own preconditions; the CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::time::Duration;

use overdrive_core::guest_transport::dial::{DialPurpose, GuestDialError};
use overdrive_core::guest_transport::wire::KernelErrno;
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeRunSeq, ProbeStatus};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
use overdrive_sim::adapters::guest_stream_dial::SimDialScript;

use super::probe_count_fixture::{
    CapturedEvent, EventCapture, LIVENESS_PORT, ProbeSet, ProbeWorld, RunOutcome, WORKLOAD_ADDR,
    settle,
};

/// The stored row's `(run_seq, status is Fail, same_status_runs)`.
fn counts(row: Option<&ProbeResultRow>) -> Option<(u64, bool, u32)> {
    row.map(|r| {
        (r.run_seq.get(), matches!(r.status, ProbeStatus::Fail { .. }), r.same_status_runs.get())
    })
}

fn seq(n: u64) -> ProbeRunSeq {
    ProbeRunSeq::new(n).expect("non-zero run sequence")
}

/// A captured field, with any quoting from a debug rendering removed.
fn field(event: &CapturedEvent, name: &str) -> Option<String> {
    event.fields.get(name).map(|v| v.trim_matches('"').to_owned())
}

/// `true` iff `event` is one `probe.not_sampled` of the world's allocation's
/// `role` probe 0 with `reason`.
fn is_not_sampled(
    world: &ProbeWorld,
    event: &CapturedEvent,
    role: ProbeRole,
    reason: &str,
) -> bool {
    field(event, "alloc").as_deref() == Some(world.alloc.as_str())
        && field(event, "role").as_deref() == Some(role.as_str())
        && field(event, "probe_idx").as_deref() == Some("0")
        && field(event, "reason").as_deref() == Some(reason)
}

/// Healthy baseline: a liveness probe failing at every run is stopped exactly
/// once, at the look that first sees its third consecutive written `Fail`
/// row; every written row carries the next run number and the true streak.
/// Pairs with `HappyPathThreeFailsTerminateAtThreshold`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 9"]
async fn liveness_probe_failing_three_runs_stops_the_allocation_once() {
    let mut world = ProbeWorld::compose(ProbeSet::AllTcp).await;

    for run in 1..=3_u32 {
        world.run_with(&RunOutcome::Fail, &[(ProbeRole::Startup, RunOutcome::Pass)]).await;
        world.look(true).await.expect("look");
        assert_eq!(
            counts(world.row(ProbeRole::Liveness).await.as_ref()),
            Some((u64::from(run), true, run)),
            "liveness row after run {run}",
        );
        assert_eq!(
            world.stop_calls_for(&world.alloc.clone()),
            usize::from(run == 3),
            "a liveness stop only at the look that first sees the third Fail row (run {run})",
        );
    }
    assert_eq!(world.acted_run(), Some(seq(3)), "the act-once memory names the crossed row");
    assert!(!world.running().await, "the successful stop wrote the liveness terminal");
    let terminal = world.alloc_row(&world.alloc.clone()).await.and_then(|row| row.terminal);
    assert_eq!(terminal, Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }));
    assert_eq!(world.probe_dials_of(ProbeRole::Liveness), 3, "one probe dial per run");
}

/// (a) Coalesced looks: three runs written between two looks terminate on the
/// first look that sees them (no undercount); many looks of the same row never
/// stop twice and change nothing. Pairs with
/// `CoalescedFailsTerminateOnFirstLook` and
/// `ReReadAfterFailedStopDoesNotReTerminate`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 9"]
async fn coalesced_runs_terminate_on_the_first_look_and_re_reads_change_nothing() {
    let mut world = ProbeWorld::compose(ProbeSet::ReadinessAndLiveness).await;

    for _ in 0..3 {
        world.run(RunOutcome::Fail).await;
    }
    // The stop fails, so the allocation stays Running and every re-read is a
    // chance to (wrongly) stop again.
    world.look(false).await.expect("look");
    assert_eq!(world.stop_calls_for(&world.alloc.clone()), 1);
    assert_eq!(world.acted_run(), Some(seq(3)));
    assert!(world.running().await, "a failed stop leaves the allocation Running");

    for _ in 0..3 {
        world.look(false).await.expect("re-read");
        assert_eq!(world.stop_calls_for(&world.alloc.clone()), 1, "a re-read stops nothing");
        assert_eq!(world.acted_run(), Some(seq(3)), "a re-read leaves the act-once memory");
        assert_eq!(counts(world.row(ProbeRole::Liveness).await.as_ref()), Some((3, true, 3)));
    }
}

/// (b) Adapter error: a VM HTTP liveness probe whose path passes the spec's
/// checks but is not a valid request target writes no row at any run and
/// emits one `probe.not_sampled { reason: "adapter_error", detail }` per run;
/// it never stops the allocation. The readiness probe beside it counts
/// normally.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 9"]
async fn http_probe_whose_path_is_not_a_request_target_records_no_sample() {
    let capture = EventCapture::default();
    let _guard = capture.install();
    let mut world = ProbeWorld::compose(ProbeSet::HttpLiveness { path: "/health check" }).await;

    for run in 1..=3_u32 {
        world.run_with(&RunOutcome::Pass, &[(ProbeRole::Readiness, RunOutcome::Fail)]).await;
        world.look(true).await.expect("look");
        assert_eq!(world.row(ProbeRole::Liveness).await, None, "no liveness row (run {run})");
        assert_eq!(
            counts(world.row(ProbeRole::Readiness).await.as_ref()),
            Some((u64::from(run), true, run)),
        );
    }
    let events = capture.not_sampled();
    assert_eq!(events.len(), 3, "one not-sampled event per run: {events:?}");
    for event in &events {
        assert!(is_not_sampled(&world, event, ProbeRole::Liveness, "adapter_error"), "{event:?}");
        assert!(field(event, "detail").is_some_and(|d| !d.is_empty()), "detail names the error");
    }
    assert_eq!(world.stop_calls_for(&world.alloc.clone()), 0);
    assert_eq!(world.acted_run(), None);
}

/// (c) Write failure: a run whose row write fails writes no row, consumes no
/// run number, neither breaks nor extends the streak, and emits one
/// `probe.not_sampled { reason: "write_failed", detail }`; the next written
/// row reuses the number and continues the streak of the last written row.
/// Pairs with `MissingSampleChangesNothing`; discharges PC-3 for
/// `SimObservationStore`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 9"]
async fn failed_row_write_counts_nothing_and_the_next_row_reuses_its_number() {
    let capture = EventCapture::default();
    let _guard = capture.install();
    let mut world = ProbeWorld::compose(ProbeSet::ReadinessAndLiveness).await;

    world.run(RunOutcome::Fail).await;
    world.look(true).await.expect("look");
    capture.clear();

    world.run(RunOutcome::WriteFail).await;
    world.look(true).await.expect("look after the failed write");
    assert_eq!(counts(world.row(ProbeRole::Liveness).await.as_ref()), Some((1, true, 1)));
    let events = capture.not_sampled();
    assert_eq!(events.len(), 2, "one event per probe of the run: {events:?}");
    for role in [ProbeRole::Readiness, ProbeRole::Liveness] {
        let event = events
            .iter()
            .find(|e| is_not_sampled(&world, e, role, "write_failed"))
            .unwrap_or_else(|| panic!("no write_failed event for {role:?}: {events:?}"));
        assert!(field(event, "detail").is_some_and(|d| !d.is_empty()));
    }

    world.run(RunOutcome::Fail).await;
    world.look(true).await.expect("look");
    assert_eq!(
        counts(world.row(ProbeRole::Liveness).await.as_ref()),
        Some((2, true, 2)),
        "the next written row reuses run number 2 and continues the streak",
    );
    assert_eq!(world.stop_calls_for(&world.alloc.clone()), 0, "two written Fail rows < 3");

    world.run(RunOutcome::Fail).await;
    world.look(true).await.expect("look");
    assert_eq!(world.stop_calls_for(&world.alloc.clone()), 1);
    assert_eq!(world.acted_run(), Some(seq(3)));
}

/// (d) No sample: a VM probe dial refused for node flow capacity or because
/// node forwarding is quiesced writes no row, consumes no run number and no
/// threshold progress, and emits `probe.not_sampled` with `node_capacity` /
/// `quiesced`; a healthy VM is not stopped across the window; every probe
/// dial carries `DialPurpose::Probe`. Pairs with
/// `MissingSampleChangesNothing`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 9"]
async fn capacity_or_quiescence_refusal_records_no_sample() {
    let capture = EventCapture::default();
    let _guard = capture.install();
    let mut world = ProbeWorld::compose(ProbeSet::ReadinessAndLiveness).await;

    world.run(RunOutcome::Fail).await;
    world.look(true).await.expect("look");
    world.run(RunOutcome::Fail).await;
    world.look(true).await.expect("look");
    capture.clear();

    world.run(RunOutcome::NoSampleCapacity).await;
    world.look(true).await.expect("look");
    world.run(RunOutcome::NoSampleQuiesced).await;
    world.look(true).await.expect("look");
    assert_eq!(counts(world.row(ProbeRole::Liveness).await.as_ref()), Some((2, true, 2)));
    assert_eq!(world.stop_calls_for(&world.alloc.clone()), 0, "no threshold advanced");
    let events = capture.not_sampled();
    assert_eq!(events.len(), 4, "one event per probe per refused run: {events:?}");
    for (role, reason) in [
        (ProbeRole::Readiness, "node_capacity"),
        (ProbeRole::Liveness, "node_capacity"),
        (ProbeRole::Readiness, "quiesced"),
        (ProbeRole::Liveness, "quiesced"),
    ] {
        assert_eq!(
            events.iter().filter(|e| is_not_sampled(&world, e, role, reason)).count(),
            1,
            "{role:?} {reason}: {events:?}",
        );
    }

    world.run(RunOutcome::Fail).await;
    world.look(true).await.expect("look");
    assert_eq!(
        counts(world.row(ProbeRole::Liveness).await.as_ref()),
        Some((3, true, 3)),
        "the next written row is run 3 and continues the Fail streak",
    );
    assert_eq!(world.stop_calls_for(&world.alloc.clone()), 1);
    assert!(
        world.dial.dials().iter().all(|d| d.purpose == DialPurpose::Probe),
        "every probe dial is DialPurpose::Probe",
    );
}

/// (e) A backward wall-clock step between written runs loses no row: the
/// stored row is the one with the greatest run number although its
/// observation time is earlier. Pairs with `ClockStepBackDoesNotLoseRow`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 9"]
async fn backward_clock_step_loses_no_row() {
    let mut world = ProbeWorld::compose(ProbeSet::ReadinessAndLiveness).await;

    world.run(RunOutcome::Fail).await;
    world.run(RunOutcome::Fail).await;
    let before = world.row(ProbeRole::Liveness).await.expect("row of run 2");
    world.clock.step_wall_clock_back(Duration::from_secs(10));
    world.run(RunOutcome::Fail).await;

    let after = world.row(ProbeRole::Liveness).await.expect("row of run 3");
    assert_eq!(counts(Some(&after)), Some((3, true, 3)));
    assert!(
        after.last_observed_at_unix_ms < before.last_observed_at_unix_ms,
        "the run-3 row carries the stepped-back observation time",
    );
    world.look(true).await.expect("look");
    assert_eq!(world.stop_calls_for(&world.alloc.clone()), 1);
}

/// (f) Resubmit after GC: the workload's intent is deleted, its allocation is
/// stopped `SystemGc` by the restart / GC authority, and the workload is
/// resubmitted. The new allocation's id differs from the earlier one; its
/// first row carries run number 1 and streak 1 and is stored; and its
/// verdicts follow its own rows only — the earlier allocation's crossed Fail
/// streak stops nothing of it. Discharges PC-1.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 9"]
async fn resubmitted_workload_counts_its_probe_runs_afresh() {
    let mut world =
        ProbeWorld::compose_with_workload_lifecycle(ProbeSet::ReadinessAndLiveness).await;
    let earlier = world.alloc.clone();
    for _ in 0..3 {
        world.run(RunOutcome::Fail).await;
    }
    let earlier_row = world.row(ProbeRole::Liveness).await;
    assert_eq!(counts(earlier_row.as_ref()), Some((3, true, 3)));

    world.delete_intent().await;
    for _ in 0..3 {
        world.workload_tick().await.expect("GC tick");
    }
    let earlier_terminal = world.alloc_row(&earlier).await.and_then(|row| row.terminal);
    assert_eq!(earlier_terminal, Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc }));

    world.resubmit_intent().await;
    let mut fresh = None;
    for _ in 0..5 {
        world.workload_tick().await.expect("resubmit tick");
        fresh = world.driver.started_specs().into_iter().rev().find(|spec| spec.alloc != earlier);
        if fresh.is_some() {
            break;
        }
    }
    let fresh_spec = fresh.expect("the resubmitted workload starts a fresh allocation");
    assert_ne!(fresh_spec.alloc, earlier, "a fresh AllocationId is minted");

    // The VM driver starts the fresh allocation's probes on Running.
    let _token = world.runner.start_alloc(&fresh_spec);
    settle().await;
    world.dial.script_default(SimDialScript::Refuse(GuestDialError::Refused {
        errno: KernelErrno::ECONNREFUSED,
    }));
    world.clock.tick(super::probe_count_fixture::PROBE_INTERVAL);
    settle().await;

    let fresh_row = world.row_of(&fresh_spec.alloc, ProbeRole::Liveness).await;
    assert_eq!(counts(fresh_row.as_ref()), Some((1, true, 1)), "the fresh key starts at run 1");
    for _ in 0..2 {
        world.look(true).await.expect("look");
    }
    assert_eq!(world.stop_calls_for(&fresh_spec.alloc), 0, "one Fail row of its own stops nothing");
    assert_eq!(world.acted_run_of(&fresh_spec.alloc), None);
}

/// (g) A status that changes between two looks — `Fail`, look, `Pass` and
/// `Fail` written, look — never terminates early: the latest row's streak is
/// 1. Pairs with `FlipBetweenLooksNoEarlyTerminate`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 9"]
async fn status_flip_between_looks_does_not_terminate_early() {
    let mut world = ProbeWorld::compose(ProbeSet::ReadinessAndLiveness).await;

    world.run(RunOutcome::Fail).await;
    world.look(true).await.expect("look");
    world.run(RunOutcome::Pass).await;
    world.run(RunOutcome::Fail).await;
    world.look(true).await.expect("look");

    assert_eq!(counts(world.row(ProbeRole::Liveness).await.as_ref()), Some((3, true, 1)));
    assert_eq!(world.stop_calls_for(&world.alloc.clone()), 0);
    assert_eq!(world.acted_run(), None);
}

/// (h) A failed stop: the first liveness stop fails (the allocation stays
/// Running), a re-read of the acted row stops nothing, each further crossed
/// row stops once more while the allocation is still Running, and nothing
/// stops after a stop succeeded. Pairs with `FailedStopRetriedOnNextRow` and
/// `NewStreakAfterPassTerminatesAgain`; discharges PC-4.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 9"]
async fn failed_stop_is_retried_once_per_new_crossed_row() {
    let mut world = ProbeWorld::compose(ProbeSet::ReadinessAndLiveness).await;
    let alloc = world.alloc.clone();

    for _ in 0..3 {
        world.run(RunOutcome::Fail).await;
    }
    world.look(false).await.expect("look: the stop fails");
    assert_eq!((world.stop_calls_for(&alloc), world.acted_run()), (1, Some(seq(3))));
    assert!(world.running().await);

    world.look(false).await.expect("re-read");
    assert_eq!(world.stop_calls_for(&alloc), 1, "a re-read of the acted row stops nothing");

    world.run(RunOutcome::Fail).await;
    world.look(false).await.expect("look: the retried stop fails again");
    assert_eq!((world.stop_calls_for(&alloc), world.acted_run()), (2, Some(seq(4))));
    assert!(world.running().await);

    world.run(RunOutcome::Fail).await;
    world.look(true).await.expect("look: the retried stop succeeds");
    assert_eq!((world.stop_calls_for(&alloc), world.acted_run()), (3, Some(seq(5))));
    assert!(!world.running().await);

    world.look(true).await.expect("re-read after the stop");
    world.run(RunOutcome::Fail).await;
    world.look(true).await.expect("a new crossed row after the stop");
    assert_eq!(world.stop_calls_for(&alloc), 3, "nothing stops after a stop succeeded");
    assert_eq!(world.acted_run(), Some(seq(5)));
}

/// After a failed stop, a `Pass` and a new crossed `Fail` streak stop the
/// allocation again. Pairs with `NewStreakAfterPassTerminatesAgain`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 9"]
async fn new_fail_streak_after_a_pass_stops_again() {
    let mut world = ProbeWorld::compose(ProbeSet::ReadinessAndLiveness).await;
    let alloc = world.alloc.clone();

    for _ in 0..3 {
        world.run(RunOutcome::Fail).await;
    }
    world.look(false).await.expect("look: the stop fails");
    world.run(RunOutcome::Pass).await;
    world.look(false).await.expect("look on the Pass row");
    assert_eq!(world.stop_calls_for(&alloc), 1);
    for _ in 0..2 {
        world.run(RunOutcome::Fail).await;
    }
    world.look(false).await.expect("look below threshold");
    assert_eq!(world.stop_calls_for(&alloc), 1, "a new streak of 2 is below the threshold");
    world.run(RunOutcome::Fail).await;
    world.look(false).await.expect("look on the crossed row");
    assert_eq!((world.stop_calls_for(&alloc), world.acted_run()), (2, Some(seq(7))));
    assert_eq!(counts(world.row(ProbeRole::Liveness).await.as_ref()), Some((7, true, 3)));
}

/// Readiness is a level re-derived at every look from the latest row: not
/// eligible below `RT` consecutive written `Pass` rows, eligible at `RT`,
/// lost on a `Fail`; a period with no sample neither creates nor clears
/// eligibility. Pairs with `ReadinessReachesThreshold`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 9"]
async fn readiness_eligibility_follows_the_latest_rows_streak() {
    let mut world = ProbeWorld::compose(ProbeSet::ReadinessAndLiveness).await;

    world.run(RunOutcome::Pass).await;
    world.look(true).await.expect("look");
    assert_ne!(world.backend_healthy().await, Some(true), "one Pass row < RT = 2");

    world.run(RunOutcome::Pass).await;
    world.look(true).await.expect("look");
    assert_eq!(world.backend_healthy().await, Some(true), "two consecutive Pass rows reach RT");

    world.run(RunOutcome::NoSampleCapacity).await;
    world.look(true).await.expect("look");
    world.look(true).await.expect("second look on the same row");
    assert_eq!(world.backend_healthy().await, Some(true), "no sample clears nothing");

    world.run(RunOutcome::Fail).await;
    world.look(true).await.expect("look");
    assert_eq!(world.backend_healthy().await, Some(false), "a Fail row ends eligibility");
    assert_eq!(counts(world.row(ProbeRole::Readiness).await.as_ref()), Some((3, true, 1)));
}

/// VM probe semantics per kind: a TCP-connect probe is healthy iff the dial
/// returns `Ok`; an HTTP probe sends its request over the guest stream and is
/// healthy iff a 2xx response arrives. Every dial targets the allocation's
/// `workload_addr` with `DialPurpose::Probe`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 9"]
async fn vm_probes_run_through_the_guest_stream_dial() {
    let mut world = ProbeWorld::compose(ProbeSet::HttpLiveness { path: "/healthz" }).await;

    world.run(RunOutcome::Pass).await;
    assert_eq!(world.row(ProbeRole::Readiness).await.map(|r| r.status), Some(ProbeStatus::Pass));
    assert_eq!(world.row(ProbeRole::Liveness).await.map(|r| r.status), Some(ProbeStatus::Pass));

    world.run_with(&RunOutcome::Pass, &[(ProbeRole::Liveness, RunOutcome::HttpServerError)]).await;
    assert!(matches!(
        world.row(ProbeRole::Liveness).await.map(|r| r.status),
        Some(ProbeStatus::Fail { .. })
    ));

    world.run(RunOutcome::Fail).await;
    let readiness = world.row(ProbeRole::Readiness).await.expect("readiness row");
    assert_eq!(
        readiness.status,
        ProbeStatus::Fail {
            last_fail_reason: GuestDialError::Refused { errno: KernelErrno::ECONNREFUSED }
                .as_str()
                .to_owned()
        },
        "a refused dial is a failure whose detail is the refusal's as_str",
    );

    let liveness_dials =
        world.dial.dials_of(std::net::SocketAddrV4::new(WORKLOAD_ADDR, LIVENESS_PORT));
    assert_eq!(liveness_dials.len(), 3);
    let first_request = String::from_utf8(liveness_dials[0].written()).expect("utf-8 request");
    assert!(
        first_request.starts_with("GET /healthz "),
        "request over the stream: {first_request:?}"
    );
    assert!(world.dial.dials().iter().all(|d| d.purpose == DialPurpose::Probe));
    assert!(world.dial.dials().iter().all(|d| *d.target.ip() == WORKLOAD_ADDR));
    assert_eq!(world.row(ProbeRole::Liveness).await.map(|r| r.probe_idx), Some(ProbeIdx::new(0)));
}

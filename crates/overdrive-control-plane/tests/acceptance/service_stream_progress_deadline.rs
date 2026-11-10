//! Feature `service-allocation-readiness-and-ending`, decision D10, obligation S-35
//! (composed stream; gate G-7) — the Service deploy stream is bounded by a progress
//! deadline derived from the startup deadline, not by the fixed 90 s cap.
//!
//! Every test submits through the production `submit_workload` handler (NDJSON lane)
//! over the shared composition in `service_stream_support` (real shim, real
//! `WorkloadLifecycle` and `ServiceLifecycle`, real `ProbeRunner`, `Sim*` adapters at
//! the driven ports) and advances the injected `SimClock`. No row is seeded. The
//! seed of every run is printed.
//!
//! CONTRACT_SHAPE: bounded-change. Universe: the lines the stream yields; the
//! workload's allocation rows (unchanged by the stream itself).

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]
#![allow(clippy::doc_markdown, clippy::print_stderr, clippy::large_futures)]

use std::time::Duration;

use overdrive_control_plane::streaming::{JobSubmitEvent, LastAttemptWire, ServiceSubmitEvent};
use overdrive_core::aggregate::{DriverInput, JobSpecInput, ResourcesInput};
use overdrive_core::api::submit::SubmitSpecInput;
use overdrive_core::traits::observation_store::AllocStatusRow;
use overdrive_core::transition_reason::{
    ServiceFailureReason, StoppedBy, TerminalCondition, TransitionReason,
};
use overdrive_reconcilers::RESTART_BACKOFF_CEILING;

use super::service_stream_support::{Node, Tap, service_spec, settle};

/// Advance the node one second at a time and run both reconcilers every `every`
/// seconds. Returns the second at which the stream's terminal line appeared, if it
/// did before `up_to`. The terminal is checked before the reconcilers run, so a
/// terminal the stream itself produced is observed with no reconciler tick after it.
async fn run_until_terminal(
    node: &Node,
    tap: &mut Tap,
    workload: &str,
    up_to: u64,
    every: u64,
    tick_base: u64,
) -> Option<u64> {
    for second in 1..=up_to {
        node.advance_secs(1).await;
        if tap.terminal_raw().is_some() {
            return Some(second);
        }
        if second % every == 0 {
            node.tick("workload-lifecycle", workload, tick_base + 2 * second).await;
            node.tick("service-lifecycle", workload, tick_base + 2 * second + 1).await;
        }
    }
    tap.terminal_raw().map(|_| up_to)
}

fn alloc_rows_without_clock_noise(rows: &[AllocStatusRow]) -> Vec<(String, String)> {
    rows.iter().map(|row| (row.alloc_id.to_string(), format!("{row:?}"))).collect()
}

/// S-35 (a) — startup deadline 120 s, `Stable` at about 100 s: the stream ends
/// `Stable` and never reports `Timeout` for the healthy, slow-starting Service
/// (`HealthySlowStartNeverTimesOut`, under DL-6).
#[tokio::test(flavor = "current_thread")]
/// CONTRACT_SHAPE: bounded-change.
async fn a_service_that_becomes_stable_after_ninety_seconds_ends_stable_not_timeout() {
    let seed = 3501;
    eprintln!("service-stream-progress-deadline seed={seed} case=slow-start");
    let node = Node::boot(seed).await;
    let spec = service_spec("slow-start", 10, 12);
    let mut tap = node.submit_streaming(&spec).await;
    node.tick("workload-lifecycle", "slow-start", 1).await;

    let mut stable_at = None;
    for second in 1..=130_u64 {
        if second == 99 {
            node.let_the_guest_accept();
        }
        node.advance_secs(1).await;
        if second % 5 == 0 {
            node.tick("service-lifecycle", "slow-start", 100 + second).await;
        }
        if tap.terminal_raw().is_some() {
            stable_at = Some(second);
            break;
        }
    }

    let second = stable_at.expect("the stream ends");
    let terminal = tap.terminal().expect("a terminal line");
    assert!(
        matches!(terminal, ServiceSubmitEvent::Stable { .. }),
        "a healthy slow start ends Stable, never Timeout; got {terminal:?} at {second} s",
    );
    assert!(second > 90, "the Service became Stable after the old 90 s cap (at {second} s)");
}

/// S-35 (b) — a never-ready Service with the default startup deadline (60 s): the
/// progress deadline (90 s) ends the stream `Failed { Timeout { 90 } }` carrying the
/// latest attempt-ending observation (the startup stop of the first attempt) as
/// labelled detail, `alloc_id: None`, and the stream writes no row.
#[tokio::test(flavor = "current_thread")]
/// CONTRACT_SHAPE: bounded-change.
async fn a_never_ready_service_times_out_at_the_progress_deadline_naming_the_last_attempt() {
    let seed = 3502;
    eprintln!("service-stream-progress-deadline seed={seed} case=never-ready");
    let node = Node::boot(seed).await;
    let spec = service_spec("never-ready", 5, 12);
    let mut tap = node.submit_streaming(&spec).await;
    node.tick("workload-lifecycle", "never-ready", 1).await;
    let first_attempt = node.current_alloc_id("never-ready").await;

    let ended_at = run_until_terminal(&node, &mut tap, "never-ready", 120, 5, 100).await;
    let rows_at_the_timeout = node.rows_of("never-ready").await;

    assert_eq!(ended_at, Some(90), "the stream ends exactly at its 90 s progress deadline");
    let terminal = tap.terminal().expect("a terminal line");
    let ServiceSubmitEvent::Failed { alloc_id, reason, stderr_tail, last_attempt } = terminal
    else {
        panic!("the never-ready Service ends Failed; got {terminal:?}");
    };
    assert_eq!(alloc_id, None, "a synthesised timeout names no allocation");
    assert_eq!(reason, ServiceFailureReason::Timeout { after_seconds: 90 });
    assert_eq!(stderr_tail, None);
    let Some(LastAttemptWire { alloc_id: attempt, reason: why, terminal: ended, detail }) =
        last_attempt
    else {
        panic!("the timeout carries the latest attempt-ending observation");
    };
    assert_eq!(attempt, first_attempt.to_string(), "the first attempt, startup-stopped");
    assert_eq!(why, TransitionReason::Stopped { by: StoppedBy::Reconciler });
    assert_eq!(ended, Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }));
    assert_eq!(detail, None, "the stop event carries no driver detail");
    assert_eq!(
        alloc_rows_without_clock_noise(&rows_at_the_timeout),
        alloc_rows_without_clock_noise(&node.rows_of("never-ready").await),
        "the stream wrote no row",
    );
}

/// S-35 (c) — a short startup deadline (2 s): the restart authority's give-up lands
/// inside the 90 s progress deadline and the stream ends with that give-up, with no
/// `last_attempt` (a give-up is the verdict, not the labelled detail).
#[tokio::test(flavor = "current_thread")]
/// CONTRACT_SHAPE: bounded-change.
async fn a_give_up_inside_the_progress_deadline_ends_the_stream_with_the_give_up() {
    let seed = 3503;
    eprintln!("service-stream-progress-deadline seed={seed} case=give-up-first");
    let node = Node::boot(seed).await;
    let spec = service_spec("short-deadline", 1, 2);
    let mut tap = node.submit_streaming(&spec).await;
    node.tick("workload-lifecycle", "short-deadline", 1).await;

    let ended_at = run_until_terminal(&node, &mut tap, "short-deadline", 85, 1, 100).await;

    let second = ended_at.expect("the give-up ends the stream before the progress deadline");
    assert!(second < 90, "the give-up came before the progress deadline (at {second} s)");
    let terminal = tap.terminal().expect("a terminal line");
    let ServiceSubmitEvent::Failed { alloc_id, reason, last_attempt, .. } = terminal else {
        panic!("the give-up ends the stream Failed; got {terminal:?}");
    };
    assert!(alloc_id.is_some(), "a give-up names its allocation");
    let ServiceFailureReason::StartupProbeFailed { probe_idx: 0, attempts, .. } = reason else {
        panic!("the verdict names the startup failure; got {reason:?}");
    };
    assert_eq!(attempts, RESTART_BACKOFF_CEILING, "attempts is the restart budget consumed");
    assert_eq!(last_attempt, None, "only the synthesised timeout carries a last attempt");
}

/// S-35 (d) — an `Unchanged` redeploy of a stuck Service with no recorded verdict
/// opens a new stream with a new clock; its `last_attempt` is drawn from events after
/// it opened only, never from an attempt that ended before.
#[tokio::test(flavor = "current_thread")]
/// CONTRACT_SHAPE: bounded-change.
async fn an_unchanged_redeploy_of_a_stuck_service_opens_a_new_clock_and_reports_only_new_attempts()
{
    let seed = 3504;
    eprintln!("service-stream-progress-deadline seed={seed} case=redeploy-stuck");
    let node = Node::boot(seed).await;
    let spec = service_spec("stuck", 5, 12);
    let mut first = node.submit_streaming(&spec).await;
    node.tick("workload-lifecycle", "stuck", 1).await;
    let first_ended = run_until_terminal(&node, &mut first, "stuck", 95, 5, 100).await;
    assert_eq!(first_ended, Some(90), "the first stream times out at its own deadline");

    let ended_before_second_open: Vec<String> = node
        .rows_of("stuck")
        .await
        .iter()
        .filter(|row| row.state.is_terminal())
        .map(|row| row.alloc_id.to_string())
        .collect();
    let mut second = node.submit_streaming(&spec).await;
    assert_eq!(second.accepted()["data"]["outcome"], "unchanged", "the same bytes are Unchanged");

    let ended_at = run_until_terminal(&node, &mut second, "stuck", 100, 5, 1_000).await;

    assert_eq!(ended_at, Some(90), "the second stream has a new clock: its own 90 s");
    let terminal = second.terminal().expect("a terminal line");
    let ServiceSubmitEvent::Failed { reason, last_attempt, .. } = terminal else {
        panic!("the stuck redeploy ends Failed; got {terminal:?}");
    };
    assert_eq!(reason, ServiceFailureReason::Timeout { after_seconds: 90 });
    let attempt = last_attempt.expect("an attempt ended while the second stream was open");
    assert!(
        !ended_before_second_open.contains(&attempt.alloc_id),
        "last_attempt {} ended after the stream opened, not before ({ended_before_second_open:?})",
        attempt.alloc_id,
    );
}

/// S-35 (e) — a Job keeps `streaming_cap` and its stream output is unchanged: the
/// terminal is the cap's `Failed { exit_code: -1, duration: "90s" }` line.
#[tokio::test(flavor = "current_thread")]
async fn a_job_stream_keeps_the_streaming_cap_and_its_timeout_line() {
    let seed = 3505;
    eprintln!("service-stream-progress-deadline seed={seed} case=job");
    let node = Node::boot(seed).await;
    let job = JobSpecInput {
        id: "batch-once".to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: DriverInput::Vm(overdrive_core::aggregate::VmInput {
            command: "/bin/sleep".into(),
            args: vec!["3600".into()],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
    };
    let mut tap = node.submit_streaming_input(SubmitSpecInput::Job(job)).await;

    for _ in 0..95 {
        node.advance_secs(1).await;
        if tap.terminal_raw().is_some() {
            break;
        }
    }

    let raw = tap.terminal_raw().expect("the Job stream ends at its cap");
    let event: JobSubmitEvent = serde_json::from_str(&raw).expect("a job event line");
    let JobSubmitEvent::Failed { exit_code, duration, attempts, max_attempts, stderr_tail } =
        &event
    else {
        panic!("the Job cap ends Failed; got {event:?}");
    };
    assert_eq!(*exit_code, Some(-1));
    assert_eq!(duration, "90s", "the Job lane keeps DEFAULT_STREAMING_CAP");
    assert_eq!(attempts, max_attempts);
    assert_eq!(stderr_tail.as_deref(), Some("did not converge in 90s"));
    assert_eq!(
        raw,
        serde_json::to_string(&event).expect("serialise"),
        "the Job line is the canonical serialisation of the unchanged event shape",
    );
    assert!(!raw.contains("last_attempt"), "the Job wire gains no field");
}

/// S-35 (f), exposure E-4 pinned — a guest that boots at `VM_BOOT_DEADLINE` (30 s) and
/// passes its startup probe on the last attempt reaches `Stable` after the stream's
/// progress deadline: the stream ends `Failed { Timeout }` with `last_attempt: None`
/// (an honest timeout, never a success claim), and the allocation then reaches
/// `Stable`.
#[tokio::test(flavor = "current_thread")]
async fn a_guest_that_boots_at_its_deadline_and_passes_on_its_last_attempt_ends_in_an_honest_timeout()
 {
    let seed = 3506;
    eprintln!("service-stream-progress-deadline seed={seed} case=exposure-e4");
    let node = Node::boot(seed).await;
    node.set_boot_delay(Duration::from_secs(30));
    let spec = service_spec("slow-boot", 5, 12);
    let mut tap = node.submit_streaming(&spec).await;
    let starter = {
        let node = node.clone();
        tokio::spawn(async move { node.tick("workload-lifecycle", "slow-boot", 1).await })
    };

    let mut ended_at = None;
    for second in 1..=95_u64 {
        if second == 89 {
            node.let_the_guest_accept();
        }
        node.advance_secs(1).await;
        if ended_at.is_none() && tap.terminal_raw().is_some() {
            ended_at = Some(second);
            break;
        }
    }
    starter.await.expect("the start tick completes after the boot delay");

    assert_eq!(ended_at, Some(90), "the stream ends at its 90 s deadline");
    let terminal = tap.terminal().expect("a terminal line");
    assert_eq!(
        terminal,
        ServiceSubmitEvent::Failed {
            alloc_id: None,
            reason: ServiceFailureReason::Timeout { after_seconds: 90 },
            stderr_tail: None,
            last_attempt: None,
        },
        "an honest timeout: no verdict, no last attempt, no success claim",
    );

    // The platform keeps going: the same allocation then reaches Stable.
    node.advance_secs(2).await;
    node.tick("service-lifecycle", "slow-boot", 500).await;
    settle().await;
    let rows = node.rows_of("slow-boot").await;
    assert!(
        rows.iter().any(|row| matches!(row.terminal, Some(TerminalCondition::Stable { .. }))),
        "the allocation reaches Stable after the stream gave up: {rows:?}",
    );
    assert_eq!(tap.drain().len(), 2, "the ended stream yielded Accepted and one terminal only");
}

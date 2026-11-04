//! D9 (ADR-0261, G-6) and B1 / R2-2 (ADR-0265): liveness begins at a committed
//! `Stable`, and a lost `Stable` is announced again, through the composed
//! production owners (`liveness_gate_fixture`).
//!
//! Traceability: S-32 (composed liveness gate; seeds vary when the startup
//! probe passes) and S-43 (a lost `Stable` write is re-announced; no view
//! marker; a control-plane restart between the view persist and the dispatch).
//! Oracles are observable surfaces only: the allocation row's terminal, the
//! stored probe rows, the runner's `is_role_live` / `active_alloc_count`, and
//! the lifecycle events the shim publishes.
//!
//! CONTRACT_SHAPE: bounded-change.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::print_stderr,
    reason = "scenarios assert their own preconditions; the seed is printed for reproduction"
)]

use overdrive_core::observation::ProbeRole;
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};

use super::liveness_gate_fixture::LivenessWorld;
use super::probe_count_fixture::{ProbeSet, RunOutcome};

fn is_liveness_stop(terminal: Option<&TerminalCondition>) -> bool {
    matches!(terminal, Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }))
}

fn is_stable(terminal: Option<&TerminalCondition>) -> bool {
    matches!(terminal, Some(TerminalCondition::Stable { .. }))
}

/// How many `Stable` lifecycle events the shim published so far.
fn drain_stable_events(
    rx: &mut tokio::sync::broadcast::Receiver<crate::action_shim::LifecycleEvent>,
) -> usize {
    let mut stable = 0;
    while let Ok(event) = rx.try_recv() {
        if is_stable(event.terminal.as_ref()) {
            stable += 1;
        }
    }
    stable
}

/// S-32: a Service whose liveness probe fails from the first period and whose
/// startup probe passes after a seeded number of periods is not stopped before
/// or at `Stable`; its first liveness row lands one interval after `Stable`
/// with streak 1; after `failure_threshold` post-`Stable` failures the
/// instance is stopped by the liveness probe and its supervisor ends.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: D9 liveness supervision begins at Stable"]
async fn slow_starting_service_with_failing_liveness_is_stopped_only_after_stable() {
    for seed in 0_u64..8 {
        let pass_at = 3 + (seed * 5) % 18;
        eprintln!("liveness gate composed: seed {seed}, startup passes at period {pass_at}");
        let mut w = LivenessWorld::compose(ProbeSet::AllTcp).await;

        for n in 1..pass_at {
            w.period(&RunOutcome::Fail, &[]).await;
            w.look().await.expect("look");
            let row = w.alloc_row().await.expect("row");
            assert!(!is_liveness_stop(row.terminal.as_ref()), "seed {seed} period {n}: stopped during startup");
            assert!(w.liveness_rows().await.is_empty(), "seed {seed} period {n}: a liveness row before Stable");
            assert!(!w.liveness_live(), "seed {seed} period {n}: liveness supervised before Stable");
        }

        w.period(&RunOutcome::Fail, &[(ProbeRole::Startup, RunOutcome::Pass)]).await;
        w.look().await.expect("the look that announces Stable");
        let row = w.alloc_row().await.expect("row");
        assert!(is_stable(row.terminal.as_ref()), "seed {seed}: Stable committed");
        assert!(w.liveness_live(), "seed {seed}: liveness starts once Stable is committed");
        assert!(w.liveness_rows().await.is_empty(), "seed {seed}: no liveness row at the Stable tick");

        w.period(&RunOutcome::Fail, &[]).await;
        w.look().await.expect("look");
        let rows = w.liveness_rows().await;
        assert_eq!(rows.len(), 1, "seed {seed}: first liveness row one interval after Stable");
        assert_eq!(rows[0].same_status_runs.get(), 1, "seed {seed}: streak starts at 1");
        let row = w.alloc_row().await.expect("row");
        assert!(!is_liveness_stop(row.terminal.as_ref()), "seed {seed}: not stopped on the first tick after Stable");

        for _ in 0..2 {
            w.period(&RunOutcome::Fail, &[]).await;
            w.look().await.expect("look");
        }
        let row = w.alloc_row().await.expect("row");
        assert!(
            is_liveness_stop(row.terminal.as_ref()),
            "seed {seed}: stopped after failure_threshold post-Stable failures, got {:?}",
            row.terminal
        );
        assert_eq!(w.runner.active_alloc_count(), 0, "seed {seed}: supervision ends with the stop");
    }
}

/// S-32: an opt-out Service (no startup probe) is `Stable` at its first
/// `Running`'s look, and its liveness starts at that moment.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: D9 liveness supervision begins at Stable"]
async fn opt_out_service_starts_liveness_at_its_first_stable() {
    let mut w = LivenessWorld::compose(ProbeSet::ReadinessAndLiveness).await;
    assert!(!w.liveness_live(), "Running alone starts no liveness");
    w.period(&RunOutcome::Pass, &[]).await;
    assert!(w.liveness_rows().await.is_empty(), "no liveness row before Stable");

    w.look().await.expect("first look announces Stable");
    assert!(is_stable(w.alloc_row().await.expect("row").terminal.as_ref()));
    assert!(w.liveness_live(), "liveness starts at the opt-out Service's Stable");

    w.period(&RunOutcome::Pass, &[]).await;
    let rows = w.liveness_rows().await;
    assert_eq!(rows.len(), 1, "first liveness row one interval after Stable");
    assert_eq!(rows[0].same_status_runs.get(), 1);
}

/// S-43: the `Stable` row write fails once, including on the allocation's very
/// first evaluation. The next evaluation emits `Stable` again (the fact carries
/// no committed `Stable`); once the write lands the row carries `Stable`,
/// liveness is live, and a committed `Stable` is not emitted again.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: B1 and R2-2 a lost Stable is re-announced; no view marker"]
async fn a_lost_stable_write_is_announced_again_until_it_commits() {
    let mut w = LivenessWorld::compose(ProbeSet::ReadinessAndLiveness).await;
    let mut events = w.state.lifecycle_events.subscribe();

    w.fail_next_write();
    let first = w.look().await;
    assert!(first.is_err(), "the failed Stable write surfaces from the dispatch");
    let row = w.alloc_row().await.expect("row");
    assert!(!is_stable(row.terminal.as_ref()), "no Stable on the row after a failed write");

    w.look().await.expect("the next evaluation announces Stable again");
    let row = w.alloc_row().await.expect("row");
    assert!(is_stable(row.terminal.as_ref()), "Stable committed by the re-announcement");
    assert!(w.liveness_live(), "liveness starts once Stable is committed");

    w.look().await.expect("look");
    w.look().await.expect("look");
    assert_eq!(drain_stable_events(&mut events), 1, "a committed Stable is announced exactly once");
}

/// S-43 variant: a control-plane restart between the view persist and the
/// dispatch of the failed `Stable` write. The reloaded view carries no marker
/// that could suppress the re-announcement.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: B1 and R2-2 a lost Stable is re-announced; no view marker"]
async fn a_lost_stable_write_is_announced_again_after_a_control_plane_restart() {
    let mut w = LivenessWorld::compose(ProbeSet::ReadinessAndLiveness).await;
    w.fail_next_write();
    assert!(w.look().await.is_err());

    w.restart_control_plane().await;
    w.look().await.expect("the reloaded runtime announces Stable again");
    let row = w.alloc_row().await.expect("row");
    assert!(is_stable(row.terminal.as_ref()), "Stable committed after the restart");
    assert!(w.liveness_live());
}

/// S-30 (B2, liveness stop): the liveness stop's terminal row write fails once.
/// The allocation's supervisor stays registered after the failed write (the
/// terminal hook fires only after the row write succeeds); the next probe row
/// re-emits the stop, the row becomes `Stopped { LivenessProbe }`, and only
/// then does the supervisor end.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: B2 the terminal hook fires after the stop row write succeeds (D9 liveness begins at Stable)"]
async fn failed_liveness_stop_row_write_keeps_the_supervisor_until_the_retry_lands() {
    let mut w = LivenessWorld::compose(ProbeSet::ReadinessAndLiveness).await;
    w.period(&RunOutcome::Pass, &[]).await;
    w.look().await.expect("first look announces Stable");
    assert!(is_stable(w.alloc_row().await.expect("row").terminal.as_ref()));
    assert!(w.liveness_live(), "liveness starts at Stable");

    // Failures up to, but not past, the point where the stop is emitted.
    for _ in 0..2 {
        w.period(&RunOutcome::Fail, &[]).await;
        w.look().await.expect("look");
        let row = w.alloc_row().await.expect("row");
        assert!(!is_liveness_stop(row.terminal.as_ref()), "below the threshold: not stopped");
    }
    assert_eq!(w.runner.active_alloc_count(), 1);

    // The crossing evaluation emits the stop; its row write fails.
    w.period(&RunOutcome::Fail, &[]).await;
    w.fail_next_write();
    assert!(w.look().await.is_err(), "the failed stop row write surfaces from the dispatch");
    let row = w.alloc_row().await.expect("row");
    assert!(!is_liveness_stop(row.terminal.as_ref()), "no stop on the row after a failed write");
    assert_eq!(
        w.runner.active_alloc_count(),
        1,
        "the supervisor stays registered while the stop row is not written"
    );

    // The next probe row re-emits the stop; the write lands.
    w.period(&RunOutcome::Fail, &[]).await;
    w.look().await.expect("the retry lands");
    let row = w.alloc_row().await.expect("row");
    assert!(
        is_liveness_stop(row.terminal.as_ref()),
        "stopped by the liveness probe after the retry, got {:?}",
        row.terminal
    );
    assert_eq!(w.runner.active_alloc_count(), 0, "the supervisor ends only after the row is written");
}

//! G-10 boundary (8), D13 item 9 and D14 (DG-8): a dropped listener report,
//! through the composed production owners (`liveness_gate_fixture`).
//!
//! The Service declares two TCP listeners and no explicit startup probe, so
//! its startup gate is the guest-listener gate. The guest reports one and the
//! other's `LISTEN` line never arrives: a ring drop is in-guest and its only
//! host-visible effect is that the line is absent, so the fault is the
//! omitted `guest_write` on the existing `SimBeaconIo`. The real
//! `VmDriver::start` runs over `SimVmm`; the real `ProbeRunner`, the shared
//! `GuestListenerBoard`, `ServiceLifecycle`, `WorkloadLifecycle`, the action
//! shim and the exit observer author every row, stop and restart.
//!
//! Traceability: G-10 required boundary scenario (8); model run
//! `ring_drop_leaves_set_partial` (`guest_report`); invariants
//! `GuestListenerPassImpliesAllDeclared`, `NoFailBeforeDeadline`,
//! `StableNeverForNonRunning`. The kernel's drop-and-count itself stays V37's
//! evidence. Oracles are observable surfaces only: the allocation row's
//! terminal, the stored startup probe rows and the lifecycle events.
//!
//! `NoPassAfterEnding` and `GuestListenerChangedNeverMissesAnInsertion` are not asserted
//! here: S-53 (`overdrive-worker/tests/acceptance/guest_listener_probe_rows.rs::no_row_is_written_after_stop_alloc`)
//! covers the first and S-52
//! (`overdrive-worker/tests/acceptance/guest_listener_board.rs::changed_never_misses_an_insertion_under_seeded_schedules`)
//! the second.
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

use std::time::Duration;

use overdrive_core::observation::{ProbeRole, ProbeStatus};
use overdrive_core::transition_reason::{ServiceFailureReason, StoppedBy, TerminalCondition};
use overdrive_reconcilers::RESTART_BACKOFF_CEILING;

use super::liveness_gate_fixture::{
    LivenessWorld, SECOND_LISTENER_PORT, is_stable, run_to_give_up,
};
use super::probe_count_fixture::LISTENER_PORT;

/// The derived startup deadline of the gate descriptor: interval 2 s x 30 attempts.
const STARTUP_DEADLINE_SECS: u64 = 60;

fn is_startup_stop(terminal: Option<&TerminalCondition>) -> bool {
    matches!(terminal, Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }))
}

/// A Service declares two listeners and the guest's report of one is lost.
/// Whichever listener the seed drops, the set stays partial: no `Pass`, no
/// row before the deadline, the first `Fail` at the deadline naming the
/// unreported listener, the stop by the startup gate, a restart by
/// `workload-lifecycle` under the one budget, and at the ceiling the give-up
/// `ServiceFailed { StartupProbeFailed }` naming it; never `Stable`.
#[tokio::test(flavor = "current_thread")]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D13/D14 a dropped listener report leaves the set partial: deadline Fail, startup stop, restart, give-up"]
async fn a_service_whose_listener_report_never_arrives_is_stopped_at_the_deadline_restarted_and_given_up()
 {
    // Two seeds: within one seed the six instances alternate which listener is dropped
    // ((seed + instance) % 2), so each seed already covers both orderings.
    for seed in 0_u64..2 {
        let mut w = LivenessWorld::compose_restartable_guest_listeners().await;
        let mut events = w.state.lifecycle_events.subscribe();
        let total = u64::from(RESTART_BACKOFF_CEILING) + 1;
        for instance in 0..total {
            // The seed picks which declared listener's line is dropped, and when
            // in the startup window the other one is reported.
            let (dropped, reported) = if (seed + instance) % 2 == 0 {
                (SECOND_LISTENER_PORT, LISTENER_PORT)
            } else {
                (LISTENER_PORT, SECOND_LISTENER_PORT)
            };
            let report_secs = 1 + (seed * 11 + instance * 7) % 50;
            let report_at = Duration::from_secs(report_secs);
            eprintln!(
                "guest listener ring drop: seed {seed}, instance {instance}, tcp:{dropped} never reported, tcp:{reported} reported {report_at:?} into the window"
            );

            let alloc = w.place_instance().await;
            let tag = format!("seed {seed} instance {instance} ({alloc})");
            assert_eq!(
                w.placed().len() as u64,
                instance + 1,
                "{tag}: each restart is a fresh allocation"
            );

            // The guest opens the reported listener; the dropped one's line is
            // simply never written.
            w.clock.tick(report_at);
            assert!(
                w.guest_writes(&format!("LISTEN tcp {reported}")),
                "{tag}: the guest's beacon is open"
            );
            super::probe_count_fixture::settle().await;

            // Before the deadline: a partial set writes no row, the look stops
            // nothing and the allocation is not Stable.
            w.clock.tick(Duration::from_secs(STARTUP_DEADLINE_SECS - 1 - report_secs));
            super::probe_count_fixture::settle().await;
            assert!(
                w.rows_for(ProbeRole::Startup).await.is_empty(),
                "{tag}: no startup row before the deadline (NoFailBeforeDeadline)"
            );
            w.look().await.expect("a look before the deadline");
            let row = w.alloc_row().await.expect("row");
            assert!(
                !is_stable(row.terminal.as_ref()) && !is_startup_stop(row.terminal.as_ref()),
                "{tag}: neither Stable nor stopped before the deadline, got {:?}",
                row.terminal
            );

            // At the deadline: the first Fail, naming the unreported listener only.
            w.clock.tick(Duration::from_secs(1));
            super::probe_count_fixture::settle().await;
            let rows = w.rows_for(ProbeRole::Startup).await;
            assert_eq!(rows.len(), 1, "{tag}: exactly the startup gate's row");
            let startup = &rows[0];
            assert_eq!(startup.same_status_runs.get(), 1, "{tag}: the first Fail has streak 1");
            match &startup.status {
                ProbeStatus::Fail { last_fail_reason } => {
                    assert!(
                        last_fail_reason.starts_with("guest listeners not open after"),
                        "{tag}: reason {last_fail_reason:?}"
                    );
                    assert!(
                        last_fail_reason.contains(&format!("tcp:{dropped}")),
                        "{tag}: the reason names the unreported listener, got {last_fail_reason:?}"
                    );
                    assert!(
                        !last_fail_reason.contains(&format!("tcp:{reported}")),
                        "{tag}: the reason omits the reported listener, got {last_fail_reason:?}"
                    );
                }
                ProbeStatus::Pass => panic!(
                    "{tag}: a Pass with a listener unreported (GuestListenerPassImpliesAllDeclared)"
                ),
            }

            // The startup gate stops the attempt.
            w.look().await.expect("the look that stops the attempt");
            w.beacon.observe_vm_exits();
            super::probe_count_fixture::settle().await;
            let row = w.alloc_row().await.expect("row");
            assert!(
                is_startup_stop(row.terminal.as_ref()),
                "{tag}: stopped by the startup gate, got {:?}",
                row.terminal
            );
            assert_eq!(w.runner.active_alloc_count(), 0, "{tag}: supervision ends with the stop");
            assert!(
                !w.rows_for(ProbeRole::Startup)
                    .await
                    .iter()
                    .any(|r| matches!(r.status, ProbeStatus::Pass)),
                "{tag}: no Pass at any point"
            );
        }

        // The budget is spent: the restart authority gives the Service up with
        // the startup cause, naming the listener the last attempt never reported.
        run_to_give_up(
            &mut w,
            &format!("seed {seed}"),
            total,
            |terminal| match terminal {
                Some(TerminalCondition::ServiceFailed {
                    reason:
                        ServiceFailureReason::StartupProbeFailed { probe_idx, last_fail, attempts },
                }) => {
                    assert_eq!(*probe_idx, 0, "seed {seed}: the gate is the sole startup descriptor");
                    assert_eq!(
                        *attempts, RESTART_BACKOFF_CEILING,
                        "seed {seed}: the restart budget consumed"
                    );
                    assert!(
                        last_fail.starts_with("guest listeners not open after")
                            && (last_fail.contains(&format!("tcp:{LISTENER_PORT}"))
                                != last_fail.contains(&format!("tcp:{SECOND_LISTENER_PORT}"))),
                        "seed {seed}: the give-up names exactly the one unreported listener, got {last_fail:?}"
                    );
                }
                other => panic!("seed {seed}: given up as StartupProbeFailed, got {other:?}"),
            },
            |terminal| {
                matches!(
                    terminal,
                    Some(TerminalCondition::ServiceFailed {
                        reason: ServiceFailureReason::StartupProbeFailed { .. }
                    })
                )
            },
        )
        .await;

        // StableNeverForNonRunning / never Stable: no Stable event was published.
        while let Ok(event) = events.try_recv() {
            assert!(
                !is_stable(event.terminal.as_ref()),
                "seed {seed}: a Stable event for a Service with an unreported listener"
            );
        }
    }
}

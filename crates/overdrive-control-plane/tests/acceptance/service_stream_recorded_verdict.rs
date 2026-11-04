//! Feature `service-allocation-readiness-and-ending`, decision D11, obligations
//! S-39, S-40, S-41 and S-42 (gate G-8; Quint module `stream_snapshot`) — the Service
//! deploy stream first reports the verdict the workload's current attempt already
//! carries, then waits.
//!
//! * S-39 — a redeploy of the same spec through the production handler ends at once
//!   with the recorded verdict (F-5), over the shared composition in
//!   `service_stream_support`.
//! * S-40 — ordering: commit, publish, subscribe and read at scripted points around
//!   the stream's own steps, including `Lagged` and a superseding fresh-id attempt.
//! * S-41 — the line built from a committed row equals the line built from the event
//!   the shim published for that write.
//! * S-42 — a failed observation read ends the stream with an error.
//!
//! S-40 and S-42 drive `build_service_stream` directly with rows written through
//! `SimObservationStore` and events sent on the bus at scripted points; the
//! `Accepted` line marks the subscribe point (the stream subscribes, then reads, on
//! its next poll). The give-up terminals of S-39 (b) are written by the production
//! shim from the action the restart authority returns (that authority's decision is
//! covered by the reconciler obligations); the subject here is what the stream does
//! with a recorded verdict.
//!
//! CONTRACT_SHAPE: bounded-change. Universe: the lines the stream yields; no row,
//! `View`, intent or event is written by the stream.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]
#![allow(clippy::doc_markdown, clippy::print_stderr, clippy::large_futures)]

use std::pin::Pin;
use std::time::Duration;

use axum::body::Bytes;
use futures::{FutureExt as _, Stream, StreamExt as _};
use overdrive_control_plane::action_shim::LifecycleEvent;
use overdrive_control_plane::api::{AllocStateWire, IdempotencyOutcome, TransitionSource};
use overdrive_control_plane::streaming::{
    ServiceSubmitEvent, build_service_accepted, build_service_stream, service_event_from_terminal,
};
use overdrive_core::TransitionReason;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::Action;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, ObservationStoreError,
};
use overdrive_core::transition_reason::{
    BackoffCause, ProbeWitness, ServiceFailureReason, StoppedBy, TerminalCondition,
};

use super::service_stream_support::{Node, service_spec, settle};

type Lines = Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>;

/// One poll of the stream: what it yields right now, if anything.
#[derive(Debug, PartialEq)]
#[allow(clippy::large_enum_variant, reason = "a test-local value compared once per step")]
enum Polled {
    Pending,
    Ended,
    Line(ServiceSubmitEvent),
    Error(String),
}

fn poll_once(stream: &mut Lines) -> Polled {
    match stream.next().now_or_never() {
        None => Polled::Pending,
        Some(None) => Polled::Ended,
        Some(Some(Ok(bytes))) => {
            Polled::Line(serde_json::from_slice(&bytes).expect("a service event line"))
        }
        Some(Some(Err(error))) => Polled::Error(error.to_string()),
    }
}

/// Open the stream for `workload` and consume its `Accepted` line (the subscribe
/// point: the stream has not yet subscribed or read).
async fn open(node: &Node, workload: &str, deadline: Duration) -> Lines {
    let accepted = build_service_accepted(
        "0".repeat(64),
        format!("workloads/{workload}"),
        IdempotencyOutcome::Unchanged,
    );
    let mut stream: Lines = Box::pin(build_service_stream(
        node.state.clone(),
        WorkloadId::new(workload).expect("workload id"),
        accepted,
        deadline,
    ));
    let first = stream.next().await.expect("an Accepted line").expect("a line");
    let first: ServiceSubmitEvent = serde_json::from_slice(&first).expect("a service event line");
    assert!(matches!(first, ServiceSubmitEvent::Accepted { .. }), "Accepted first; got {first:?}");
    stream
}

fn node_id() -> NodeId {
    NodeId::new("local").expect("node id")
}

fn stable_terminal() -> TerminalCondition {
    TerminalCondition::Stable {
        settled_in_ms: 700,
        witness: ProbeWitness {
            probe_idx: 0,
            role: "startup".to_owned(),
            mechanic_summary: "tcp 127.0.0.1:8080".to_owned(),
            inferred: false,
        },
    }
}

fn row(
    workload: &str,
    attempt: u32,
    counter: u64,
    state: AllocState,
    terminal: Option<TerminalCondition>,
    detail: Option<&str>,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::new(&format!("alloc-{workload}-{attempt}")).expect("alloc id"),
        workload_id: WorkloadId::new(workload).expect("workload id"),
        node_id: node_id(),
        state,
        updated_at: LogicalTimestamp { counter, writer: node_id() },
        reason: Some(TransitionReason::Started),
        detail: detail.map(str::to_owned),
        terminal,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

/// The shim's commit step: the row is written through the observation store.
async fn commit(node: &Node, row: &AllocStatusRow) {
    node.obs
        .write_alloc_lifecycle(
            row.clone(),
            overdrive_core::traits::observation_store::TransitionSource::Reconciler,
        )
        .await
        .expect("the row commits");
}

/// The shim's publish step: the event for the committed write is sent on the bus.
fn publish(node: &Node, row: &AllocStatusRow) {
    let _ = node.state.lifecycle_events.send(LifecycleEvent {
        alloc_id: row.alloc_id.clone(),
        workload_id: row.workload_id.clone(),
        from: AllocStateWire::Running,
        to: AllocStateWire::from(row.state),
        reason: row.reason.clone().expect("a reason"),
        detail: row.detail.clone(),
        source: TransitionSource::Reconciler,
        at: format!("{}@local", row.updated_at.counter),
        terminal: row.terminal.clone(),
    });
}

/// A verdict class: its name, the state its row lands in, its terminal, the row's detail
/// and the line the stream must yield for it.
type Verdict =
    (&'static str, AllocState, TerminalCondition, Option<&'static str>, ServiceSubmitEvent);

/// The verdict classes the stream projects to a line, for `workload`'s attempt 0.
fn verdicts(workload: &str) -> Vec<Verdict> {
    let alloc = format!("alloc-{workload}-0");
    vec![
        (
            "stable",
            AllocState::Running,
            stable_terminal(),
            None,
            ServiceSubmitEvent::Stable {
                alloc_id: alloc.clone(),
                settled_in_ms: 700,
                witness: ProbeWitness {
                    probe_idx: 0,
                    role: "startup".to_owned(),
                    mechanic_summary: "tcp 127.0.0.1:8080".to_owned(),
                    inferred: false,
                },
            },
        ),
        (
            "give-up",
            AllocState::Failed,
            TerminalCondition::BackoffExhausted { attempts: 5 },
            Some("exit 1"),
            ServiceSubmitEvent::Failed {
                alloc_id: Some(alloc.clone()),
                reason: ServiceFailureReason::BackoffExhausted {
                    attempts: 5,
                    cause: BackoffCause::AttemptBudget,
                    last_exit_code: None,
                },
                stderr_tail: Some("exit 1".to_owned()),
                last_attempt: None,
            },
        ),
        (
            "operator stop",
            AllocState::Terminated,
            TerminalCondition::Stopped { by: StoppedBy::Operator },
            None,
            ServiceSubmitEvent::Stopped { alloc_id: alloc, by: StoppedBy::Operator },
        ),
    ]
}

// ===========================================================================
// S-40 — ordering around the stream's subscription and read
// ===========================================================================

/// S-40 — a terminal committed and published after `Accepted` and before the stream
/// subscribes and reads is read, not missed: the line is the same one the live path
/// would deliver, and it is yielded exactly once.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: D11 recorded verdict read before the stream waits"]
async fn a_terminal_committed_and_published_before_the_stream_reads_is_reported_once() {
    let seed = 4001;
    eprintln!("service-stream-recorded-verdict seed={seed} schedule=commit+publish-before-read");
    let node = Node::boot(seed).await;
    for (index, (name, state, terminal, detail, _)) in verdicts("w0").into_iter().enumerate() {
        let workload = format!("w{index}");
        let (_, _, _, _, expected) =
            verdicts(&workload).into_iter().find(|v| v.0 == name).expect("the same class");
        let mut stream = open(&node, &workload, Duration::from_secs(3600)).await;
        let committed = row(&workload, 0, 1, state, Some(terminal), detail);
        commit(&node, &committed).await;
        publish(&node, &committed);

        assert_eq!(poll_once(&mut stream), Polled::Line(expected), "{name}: the recorded verdict");
        publish(&node, &committed);
        assert_eq!(poll_once(&mut stream), Polled::Ended, "{name}: exactly one terminal line");
    }
}

/// S-40 — a terminal committed after the stream has read and is waiting (the commit,
/// then the publish) is delivered live: the same line whichever path delivers it.
#[tokio::test(flavor = "current_thread")]
async fn a_terminal_committed_and_published_after_the_read_is_delivered_live() {
    let seed = 4002;
    eprintln!("service-stream-recorded-verdict seed={seed} schedule=commit+publish-after-read");
    let node = Node::boot(seed).await;
    for (index, (name, state, terminal, detail, _)) in verdicts("w0").into_iter().enumerate() {
        let workload = format!("w{index}");
        let (_, _, _, _, expected) =
            verdicts(&workload).into_iter().find(|v| v.0 == name).expect("the same class");
        let mut stream = open(&node, &workload, Duration::from_secs(3600)).await;
        assert_eq!(poll_once(&mut stream), Polled::Pending, "{name}: nothing recorded: waits");

        let committed = row(&workload, 0, 1, state, Some(terminal), detail);
        commit(&node, &committed).await;
        assert_eq!(poll_once(&mut stream), Polled::Pending, "{name}: a commit alone is no event");
        publish(&node, &committed);

        assert_eq!(poll_once(&mut stream), Polled::Line(expected), "{name}: the live line");
        assert_eq!(poll_once(&mut stream), Polled::Ended, "{name}: exactly one terminal line");
    }
}

/// S-40 — `Lagged`: the subscriber misses the event for a committed terminal (the bus
/// overflows with other workloads' events) and recovers the terminal by relisting the
/// rows.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: D11 relist on Lagged"]
async fn a_lagged_stream_recovers_the_terminal_it_missed_by_relisting() {
    let seed = 4003;
    eprintln!("service-stream-recorded-verdict seed={seed} schedule=lagged-then-relist");
    let node = Node::boot(seed).await;
    let mut stream = open(&node, "w-lag", Duration::from_secs(3600)).await;
    assert_eq!(poll_once(&mut stream), Polled::Pending, "nothing recorded: waits");

    let committed = row("w-lag", 0, 1, AllocState::Running, Some(stable_terminal()), None);
    commit(&node, &committed).await;
    publish(&node, &committed);
    let noise = row("noise", 0, 1, AllocState::Running, None, None);
    for _ in 0..(overdrive_control_plane::DEFAULT_LIFECYCLE_BROADCAST_CAPACITY + 8) {
        publish(&node, &noise);
    }

    let expected = verdicts("w-lag").into_iter().next().expect("stable").4;
    assert_eq!(poll_once(&mut stream), Polled::Line(expected), "the relist finds the terminal");
    assert_eq!(poll_once(&mut stream), Polled::Ended, "exactly one terminal line");
}

/// S-40 — a fresh-id restart supersedes attempt 0: the give-up recorded on attempt 0
/// is never reported; the stream waits on attempt 1 and ends with attempt 1's line.
#[tokio::test(flavor = "current_thread")]
async fn a_superseded_attempts_give_up_is_never_the_verdict() {
    let seed = 4004;
    eprintln!("service-stream-recorded-verdict seed={seed} schedule=superseding-attempt");
    let node = Node::boot(seed).await;
    let mut stream = open(&node, "w-sup", Duration::from_secs(3600)).await;

    let given_up = row(
        "w-sup",
        0,
        1,
        AllocState::Failed,
        Some(TerminalCondition::BackoffExhausted { attempts: 5 }),
        Some("exit 1"),
    );
    commit(&node, &given_up).await;
    publish(&node, &given_up);
    let successor = row("w-sup", 1, 2, AllocState::Running, None, None);
    commit(&node, &successor).await;
    publish(&node, &successor);
    assert_eq!(poll_once(&mut stream), Polled::Pending, "the current attempt has no terminal");

    let stable = row("w-sup", 1, 3, AllocState::Running, Some(stable_terminal()), None);
    commit(&node, &stable).await;
    publish(&node, &stable);

    let Polled::Line(ServiceSubmitEvent::Stable { alloc_id, .. }) = poll_once(&mut stream) else {
        panic!("the stream ends with the current attempt's Stable");
    };
    assert_eq!(alloc_id, "alloc-w-sup-1", "attempt 1, never the superseded attempt 0");
    assert_eq!(poll_once(&mut stream), Polled::Ended);
}

/// S-40 / D7 item 5 — a restartable probe stop recorded on the current attempt is not
/// a verdict: the stream waits, and ends with the successor's `Stable`.
#[tokio::test(flavor = "current_thread")]
async fn a_recorded_probe_stop_is_not_a_verdict_and_the_stream_waits() {
    let seed = 4005;
    eprintln!("service-stream-recorded-verdict seed={seed} schedule=restartable-stop");
    let node = Node::boot(seed).await;
    for (index, by) in [StoppedBy::StartupProbe, StoppedBy::LivenessProbe].into_iter().enumerate() {
        let workload = format!("w-probe-{index}");
        let mut stream = open(&node, &workload, Duration::from_secs(3600)).await;
        let stopped = row(
            &workload,
            0,
            1,
            AllocState::Terminated,
            Some(TerminalCondition::Stopped { by }),
            None,
        );
        commit(&node, &stopped).await;
        publish(&node, &stopped);
        assert_eq!(poll_once(&mut stream), Polled::Pending, "{by:?}: restartable: waits");

        let successor = row(&workload, 1, 2, AllocState::Running, Some(stable_terminal()), None);
        commit(&node, &successor).await;
        publish(&node, &successor);
        let Polled::Line(ServiceSubmitEvent::Stable { alloc_id, .. }) = poll_once(&mut stream)
        else {
            panic!("{by:?}: the successor's Stable ends the stream");
        };
        assert_eq!(alloc_id, format!("alloc-{workload}-1"));
    }
}

/// G-7 boundary (biased select) — an event that arrives with the deadline wins: a
/// `Stable` committed, published and a clock tick to the deadline all landing before
/// the next poll yield `Stable`, never `Timeout`.
#[tokio::test(flavor = "current_thread")]
async fn an_event_arriving_with_the_deadline_wins_over_the_timeout() {
    let seed = 4006;
    eprintln!("service-stream-recorded-verdict seed={seed} schedule=event-with-deadline");
    let mut node = Node::boot(seed).await;
    node.state.streaming_cap = Duration::from_secs(30);
    let mut stream = open(&node, "w-tie", Duration::from_secs(30)).await;
    assert_eq!(poll_once(&mut stream), Polled::Pending, "nothing recorded: waits");

    let committed = row("w-tie", 0, 1, AllocState::Running, Some(stable_terminal()), None);
    commit(&node, &committed).await;
    publish(&node, &committed);
    node.clock.tick(Duration::from_secs(30));

    let Polled::Line(line) = poll_once(&mut stream) else {
        panic!("the stream yields a terminal line");
    };
    assert!(matches!(line, ServiceSubmitEvent::Stable { .. }), "the event wins; got {line:?}");
}

// ===========================================================================
// S-42 — a failed read is not absorbed
// ===========================================================================

/// S-42 — a failed observation read at open ends the stream with an error: it does
/// not yield `Timeout`, wait for the deadline, or treat the fault as "no verdict";
/// the store is unchanged.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: D11 a failed read ends the stream with an error"]
async fn a_failed_read_at_open_ends_the_stream_with_an_error_and_never_waits() {
    let seed = 4201;
    eprintln!("service-stream-recorded-verdict seed={seed} case=read-fails-at-open");
    let node = Node::boot(seed).await;
    let mut stream = open(&node, "w-read", Duration::from_secs(3600)).await;
    let rows_before = node.obs.alloc_status_rows().await.expect("rows");
    node.obs.inject_alloc_status_rows_failure(ObservationStoreError::Unreachable {
        peer: "peer-b".to_owned(),
    });

    let polled = poll_once(&mut stream);

    assert!(matches!(polled, Polled::Error(_)), "a typed stream error, not a wait; got {polled:?}");
    assert_eq!(poll_once(&mut stream), Polled::Ended, "the stream ended");
    assert_eq!(node.obs.alloc_status_rows().await.expect("rows"), rows_before, "store unchanged");
    node.clock.tick(Duration::from_secs(7200));
    settle().await;
    assert_eq!(poll_once(&mut stream), Polled::Ended, "no Timeout after the deadline either");
}

/// S-42 — a failed read on the relist after `Lagged` ends the stream with an error.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: D11 relist on Lagged; a failed relist ends the stream with an error"]
async fn a_failed_relist_after_lagged_ends_the_stream_with_an_error() {
    let seed = 4202;
    eprintln!("service-stream-recorded-verdict seed={seed} case=relist-fails");
    let node = Node::boot(seed).await;
    let mut stream = open(&node, "w-relist", Duration::from_secs(3600)).await;
    assert_eq!(poll_once(&mut stream), Polled::Pending, "nothing recorded: waits");
    let noise = row("noise", 0, 1, AllocState::Running, None, None);
    for _ in 0..(overdrive_control_plane::DEFAULT_LIFECYCLE_BROADCAST_CAPACITY + 8) {
        publish(&node, &noise);
    }
    node.obs.inject_alloc_status_rows_failure(ObservationStoreError::Unreachable {
        peer: "peer-b".to_owned(),
    });

    let polled = poll_once(&mut stream);

    assert!(matches!(polled, Polled::Error(_)), "a typed stream error; got {polled:?}");
    assert_eq!(poll_once(&mut stream), Polled::Ended);
}

// ===========================================================================
// S-41 — the snapshot line equals the live line
// ===========================================================================

/// A named terminal class and the action that finalizes an allocation into it.
type TerminalClass = (&'static str, fn(&AllocationId) -> Action);

/// S-41 — for each terminal class written through the real shim, the line built from
/// the committed row equals (byte for byte) the line built from the event published
/// for that write, and an event received implies its row is readable.
#[tokio::test(flavor = "current_thread")]
async fn the_line_built_from_a_committed_row_equals_the_line_built_from_its_event() {
    let seed = 4101;
    eprintln!("service-stream-recorded-verdict seed={seed} case=snapshot-equals-live");
    let node = Node::boot(seed).await;
    let classes: Vec<TerminalClass> = vec![
        ("stable", |alloc| Action::FinalizeFailed {
            alloc_id: alloc.clone(),
            terminal: Some(stable_terminal()),
        }),
        ("backoff exhausted", |alloc| Action::FinalizeFailed {
            alloc_id: alloc.clone(),
            terminal: Some(TerminalCondition::BackoffExhausted { attempts: 5 }),
        }),
        ("early exit", |alloc| Action::FinalizeFailed {
            alloc_id: alloc.clone(),
            terminal: Some(TerminalCondition::ServiceFailed {
                reason: ServiceFailureReason::EarlyExit { exit_code: Some(1) },
            }),
        }),
        ("startup probe failed", |alloc| Action::FinalizeFailed {
            alloc_id: alloc.clone(),
            terminal: Some(TerminalCondition::ServiceFailed {
                reason: ServiceFailureReason::StartupProbeFailed {
                    probe_idx: 0,
                    last_fail: "connection refused".to_owned(),
                    attempts: 5,
                },
            }),
        }),
        ("liveness probe failed", |alloc| Action::FinalizeFailed {
            alloc_id: alloc.clone(),
            terminal: Some(TerminalCondition::ServiceFailed {
                reason: ServiceFailureReason::LivenessProbeFailed { probe_idx: 0, attempts: 5 },
            }),
        }),
        ("operator stop", |alloc| Action::StopAllocation {
            alloc_id: alloc.clone(),
            terminal: Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
        }),
    ];

    for (index, (name, action)) in classes.into_iter().enumerate() {
        let workload = format!("s41-{index}");
        let spec = service_spec(&workload, 5, 12);
        let (_tap, alloc) = node.deploy_and_start(&spec, 10 + index as u64).await;
        let mut events = node.state.lifecycle_events.subscribe();

        node.dispatch(vec![action(&alloc)], 100 + index as u64).await;

        let event = loop {
            let event = events.try_recv().unwrap_or_else(|e| {
                panic!("{name}: the shim published an event for the write ({e:?})")
            });
            if event.alloc_id == alloc && event.terminal.is_some() {
                break event;
            }
        };
        let committed = node
            .obs
            .alloc_status_row(&alloc)
            .await
            .expect("the read succeeds")
            .unwrap_or_else(|| panic!("{name}: an event received implies its row is readable"));
        let terminal = committed.terminal.clone().expect("the committed row carries the terminal");
        let from_row = service_event_from_terminal(
            committed.alloc_id.as_str(),
            &terminal,
            committed.detail.clone(),
            None,
        );
        let from_event = service_event_from_terminal(
            event.alloc_id.as_str(),
            event.terminal.as_ref().expect("a terminal"),
            event.detail.clone(),
            None,
        );
        assert_eq!(from_row, from_event, "{name}: the snapshot line equals the live line");
        assert_eq!(
            serde_json::to_string(&from_row).expect("serialise"),
            serde_json::to_string(&from_event).expect("serialise"),
            "{name}: byte-equal on the wire",
        );
    }
}

// ===========================================================================
// S-39 — a redeploy ends at once (F-5), composed through the production handler
// ===========================================================================

/// S-39 (a) — the first deploy reached `Stable`; the same spec submitted again yields
/// `Accepted { unchanged }` then `Stable` with the recorded `settled_in_ms` and
/// witness, with no clock advance, and the redeploy writes no row and no event.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: D11 redeploy reports the recorded Stable"]
async fn a_redeploy_of_a_stable_service_ends_stable_at_once() {
    let seed = 3901;
    eprintln!("service-stream-recorded-verdict seed={seed} case=redeploy-stable");
    let node = Node::boot(seed).await;
    let spec = service_spec("redeploy-stable", 5, 12);
    let (mut first, _alloc) = node.deploy_and_start(&spec, 1).await;
    node.let_the_guest_accept();
    for second in 1..=30_u64 {
        node.advance_secs(1).await;
        if second % 5 == 0 {
            node.tick("service-lifecycle", "redeploy-stable", 100 + second).await;
        }
        if first.terminal_raw().is_some() {
            break;
        }
    }
    let recorded = first.terminal().expect("the first deploy ends");
    assert!(matches!(recorded, ServiceSubmitEvent::Stable { .. }), "first reached Stable");
    let rows_before = node.rows_of("redeploy-stable").await;
    let mut events = node.state.lifecycle_events.subscribe();

    let mut second = node.submit_streaming(&spec).await;

    assert_eq!(second.accepted()["data"]["outcome"], "unchanged");
    assert_eq!(second.terminal(), Some(recorded), "the recorded Stable, at once");
    assert!(second.ended(), "the stream ended without a clock advance");
    assert_eq!(node.rows_of("redeploy-stable").await, rows_before, "the redeploy writes no row");
    assert!(events.try_recv().is_err(), "the redeploy publishes no event");
}

/// S-39 (b) — the first deploy was given up (each of the four give-up terminals): the
/// redeploy yields the recorded `Failed` at once, byte-equal to the line the first
/// deploy's stream carried.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: D11 redeploy reports the recorded give-up"]
async fn a_redeploy_of_a_given_up_service_reports_the_recorded_give_up_at_once() {
    let seed = 3902;
    eprintln!("service-stream-recorded-verdict seed={seed} case=redeploy-given-up");
    let node = Node::boot(seed).await;
    let give_ups = [
        TerminalCondition::BackoffExhausted { attempts: 5 },
        TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::EarlyExit { exit_code: Some(1) },
        },
        TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::StartupProbeFailed {
                probe_idx: 0,
                last_fail: "connection refused".to_owned(),
                attempts: 5,
            },
        },
        TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::LivenessProbeFailed { probe_idx: 0, attempts: 5 },
        },
    ];
    for (index, terminal) in give_ups.into_iter().enumerate() {
        let workload = format!("redeploy-given-up-{index}");
        let spec = service_spec(&workload, 5, 12);
        let (mut first, alloc) = node.deploy_and_start(&spec, 10 + index as u64).await;
        node.dispatch(
            vec![Action::FinalizeFailed { alloc_id: alloc, terminal: Some(terminal.clone()) }],
            100 + index as u64,
        )
        .await;
        settle().await;
        let recorded = first.terminal_raw().expect("the first stream carried the give-up");
        let rows_before = node.rows_of(&workload).await;

        let mut second = node.submit_streaming(&spec).await;

        assert_eq!(second.terminal_raw(), Some(recorded), "{terminal:?}: the recorded give-up");
        assert_eq!(node.rows_of(&workload).await, rows_before, "the redeploy writes no row");
    }
}

/// S-39 (c) — the first deploy is still starting: the redeploy yields no terminal until
/// the live event arrives, then ends with it.
#[tokio::test(flavor = "current_thread")]
async fn a_redeploy_of_a_starting_service_waits_for_the_live_event() {
    let seed = 3903;
    eprintln!("service-stream-recorded-verdict seed={seed} case=redeploy-starting");
    let node = Node::boot(seed).await;
    let spec = service_spec("redeploy-starting", 5, 12);
    let (_first, _alloc) = node.deploy_and_start(&spec, 1).await;
    let mut second = node.submit_streaming(&spec).await;
    assert_eq!(second.accepted()["data"]["outcome"], "unchanged");

    node.advance_secs(5).await;
    assert_eq!(second.terminal_raw(), None, "nothing recorded: the stream waits");

    node.let_the_guest_accept();
    for second_count in 1..=20_u64 {
        node.advance_secs(1).await;
        if second_count % 5 == 0 {
            node.tick("service-lifecycle", "redeploy-starting", 100 + second_count).await;
        }
        if second.terminal_raw().is_some() {
            break;
        }
    }
    assert!(
        matches!(second.terminal(), Some(ServiceSubmitEvent::Stable { .. })),
        "the live Stable ends the redeploy's stream",
    );
}

/// S-39 (d) — the first deploy's current attempt was stopped for a probe (startup,
/// then liveness) and its successor is starting: the stopped attempt's row is not a
/// verdict, so the redeploy yields no terminal until the successor reaches `Stable`,
/// and then ends with the successor's line, never the predecessor's stop.
#[tokio::test(flavor = "current_thread")]
async fn a_redeploy_while_a_probe_stopped_attempts_successor_starts_waits_for_the_successor() {
    let seed = 3904;
    eprintln!("service-stream-recorded-verdict seed={seed} case=redeploy-probe-stopped");
    let node = Node::boot(seed).await;
    for (index, by) in [StoppedBy::StartupProbe, StoppedBy::LivenessProbe].into_iter().enumerate() {
        let workload = format!("redeploy-probe-stopped-{index}");
        let spec = service_spec(&workload, 5, 12);
        let (_first, stopped) = node.deploy_and_start(&spec, 10 + index as u64).await;
        node.dispatch(
            vec![Action::StopAllocation {
                alloc_id: stopped.clone(),
                terminal: Some(TerminalCondition::Stopped { by }),
            }],
            100 + index as u64,
        )
        .await;
        node.advance_secs(10).await;
        node.tick("workload-lifecycle", &workload, 200 + index as u64).await;
        let successor = node.current_alloc_id(&workload).await;
        assert_ne!(successor, stopped, "{by:?}: the stopped attempt was replaced by a fresh id");

        let mut second = node.submit_streaming(&spec).await;
        node.advance_secs(5).await;
        assert_eq!(second.terminal_raw(), None, "{by:?}: a probe stop is not the verdict");

        node.let_the_guest_accept();
        for step in 1..=20_u64 {
            node.advance_secs(1).await;
            if step % 5 == 0 {
                node.tick("service-lifecycle", &workload, 300 + step).await;
            }
            if second.terminal_raw().is_some() {
                break;
            }
        }
        let Some(ServiceSubmitEvent::Stable { alloc_id, .. }) = second.terminal() else {
            panic!("{by:?}: the successor's Stable ends the redeploy's stream");
        };
        assert_eq!(
            alloc_id,
            successor.to_string(),
            "{by:?}: the successor, not the stopped attempt"
        );
    }
}

/// S-39 (e) — the first deploy was operator-stopped: the redeploy yields `Stopped`
/// naming the operator at once (the `/stop` sentinel persists until a restart).
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: D11 redeploy reports an operator stop"]
async fn a_redeploy_of_an_operator_stopped_service_reports_the_operator_stop_at_once() {
    let seed = 3905;
    eprintln!("service-stream-recorded-verdict seed={seed} case=redeploy-operator-stopped");
    let node = Node::boot(seed).await;
    let spec = service_spec("redeploy-stopped", 5, 12);
    let (_first, alloc) = node.deploy_and_start(&spec, 1).await;
    node.operator_stops("redeploy-stopped").await;
    node.tick("workload-lifecycle", "redeploy-stopped", 2).await;
    settle().await;

    let mut second = node.submit_streaming(&spec).await;

    assert_eq!(
        second.terminal(),
        Some(ServiceSubmitEvent::Stopped { alloc_id: alloc.to_string(), by: StoppedBy::Operator }),
        "Stopped naming the operator",
    );
}

/// S-39 (f) — `overdrive workload restart` after a give-up, then a redeploy while the
/// successor starts: the stream never reports the superseded attempt's give-up.
#[tokio::test(flavor = "current_thread")]
async fn a_redeploy_while_a_restarted_successor_starts_never_reports_the_superseded_give_up() {
    let seed = 3906;
    eprintln!("service-stream-recorded-verdict seed={seed} case=redeploy-after-restart");
    let node = Node::boot(seed).await;
    let spec = service_spec("redeploy-restarted", 5, 12);
    let (_first, given_up) = node.deploy_and_start(&spec, 1).await;
    node.dispatch(
        vec![Action::FinalizeFailed {
            alloc_id: given_up.clone(),
            terminal: Some(TerminalCondition::BackoffExhausted { attempts: 5 }),
        }],
        2,
    )
    .await;
    node.operator_restarts("redeploy-restarted").await;
    node.tick("workload-lifecycle", "redeploy-restarted", 3).await;
    let successor = node.current_alloc_id("redeploy-restarted").await;
    assert_ne!(successor, given_up, "the restart minted a fresh attempt id");

    let mut second = node.submit_streaming(&spec).await;
    node.advance_secs(5).await;

    assert_eq!(second.terminal_raw(), None, "the superseded give-up is not the verdict");
}

//! netns-density-295 correctness-recovery proof §3.6 — the `serve` process
//! lifetime owner consumes the shared guest-network fail-stop (correctness
//! gap 4).
//!
//! # Contract under proof
//!
//! `docs/feature/netns-density-295/feature-delta.md` § internal fail-stop
//! request to the CLI (D-295-DISTILL-8) and ADR-0124: the retained
//! shared-network supervisor's typed [`ServeShutdownRequest`] reaches the CLI
//! process-lifetime owner; the owner selects it before an operator `SIGINT`
//! when both are ready; shutdown after the request is outer-bounded to ten
//! seconds; the outcome maps to process exit status `1`.
//!
//! # How the proof drives it (in-process; no binary is spawned)
//!
//! Each test boots the real production composition through the CLI entry
//! point `serve::run_with_kek` (the production `run` differs only in the
//! kernel-keyring KEK, which refuses a cold host) and hands the returned
//! [`ServeHandle`] to the library lifetime owner [`ServeLifetime`] — the same
//! owner `main.rs` runs. The operator signal source and the outer-bound clock
//! are the port's injected inputs: the test delivers signals through
//! [`DrivenSignals`] and passes logical time through [`RecordingClock`].
//!
//! The fail-stop cause is real host state, not a fabricated request: the
//! shared IPv4 constant intercept program (`ip overdrive-mtls`) is deleted
//! from the kernel while no allocation holds a capability record. The
//! supervisor's one-second audit detects the loss, retries the real worker
//! convergence every 250 ms, and at the five-second recovery deadline emits
//! `guest_network.shared_owner_fail_stop cleanup=abandoned_to_shutdown` and
//! sends its typed request (`RecoveryDeadlineExceeded`, component `IpRules`).
//!
//! # Determinism
//!
//! Every test runs the whole composition on one current-thread Tokio runtime.
//! The supervisor emits the fail-stop evidence event and sends the request on
//! a capacity-1 channel within one poll of its task, with no await point that
//! can yield in between. On a current-thread runtime the test task therefore
//! observes the captured event only after that poll has returned, i.e. after
//! the request is buffered. Delivering `SIGINT` to the driven source before
//! the lifetime's first poll then makes both branches ready at that poll
//! without any sleep. For the outer bound, the test advances the injected
//! clock by exactly ten seconds as soon as the lifetime arms it; shutdown
//! needs many cross-task joins that each require another task to run, so on
//! a current-thread runtime it cannot complete before the lifetime's next
//! poll observes the elapsed bound.
//!
//! The only real-time waits are harness bounds (boot readiness, observation
//! of the supervisor's own deadline) and a pre-fault healthy baseline window.
//!
//! Hypothesis (pre-port `main.rs`): the lifetime waits on `SIGINT` alone,
//! never consumes the internal request, arms no bound, and exits 0.
//! Prediction: every fail-stop verdict is RED against the pre-port
//! transliteration and GREEN once the port consumes the request.
//! Falsification: the pre-port body returning a fail-stop outcome, or the
//! ported body consuming the ready `SIGINT`.
//!
//! CONTRACT_SHAPE: bounded-change.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::too_many_lines,
    reason = "Tier-3 proof fixture: fail-fast diagnostics and exact CONTRACT_SHAPE token"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::serve_lifetime::{
    FAIL_STOP_OUTER_BOUND, FailStopCleanup, ServeExit, ServeLifetime, ServeSignal,
};
use overdrive_core::guest_network::{
    ServeShutdownRequest, SharedGuestNetworkComponent, SharedGuestNetworkFailStopCause,
};
use serial_test::serial;
use tokio::sync::Notify;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::serve_lifetime_support::{RecordingClock, driven_signals};
use super::vm_walking_skeleton::shared_staging_root;

const SHARED_IPV4_INTERCEPT_TABLE: &str = "overdrive-mtls";
/// Pinned supervisor evidence emitted immediately before the typed request.
const SUPERVISOR_FAIL_STOP_EVENT: &str = "guest_network.shared_owner_fail_stop";
const SUPERVISOR_FAIL_STOP_CLEANUP: &str = "abandoned_to_shutdown";
const UNHEALTHY_EVENT: &str = "guest_network.shared_owner_unhealthy";
const RETRY_EVENT: &str = "guest_network.shared_owner_retry";
/// Harness bound for the real composition to boot.
const BOOT_BOUND: Duration = Duration::from_secs(90);
/// Pre-fault window spanning at least one one-second supervisor audit.
const HEALTHY_BASELINE_WINDOW: Duration = Duration::from_millis(1_500);
/// Harness bound for the supervisor's own 1 s detection + 5 s repair deadline.
const REQUEST_OBSERVATION_BOUND: Duration = Duration::from_secs(20);
/// Harness bound for a lifetime run that should end promptly.
const LIFETIME_OBSERVATION_BOUND: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Tracing capture (thread-local default: the whole composition runs on the
// test's current-thread runtime)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Captured {
    at: Duration,
    name: String,
    fields: BTreeMap<String, String>,
}

#[derive(Default)]
struct FieldVisitor {
    fields: BTreeMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}").trim_matches('"').to_owned());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields.insert(field.name().to_owned(), value.to_owned());
    }
}

#[derive(Clone)]
struct Capture {
    start: Instant,
    events: Arc<Mutex<Vec<Captured>>>,
    request_evidence: Arc<Notify>,
}

impl Capture {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            events: Arc::new(Mutex::new(Vec::new())),
            request_evidence: Arc::new(Notify::new()),
        }
    }

    fn snapshot(&self) -> Vec<Captured> {
        self.events.lock().expect("capture lock").clone()
    }

    fn count(&self, name: &str) -> usize {
        self.snapshot().iter().filter(|event| event.name == name).count()
    }

    fn request_evidence(&self) -> Option<Captured> {
        self.snapshot().into_iter().find(|event| {
            event.name == SUPERVISOR_FAIL_STOP_EVENT
                && event.fields.get("cleanup").map(String::as_str)
                    == Some(SUPERVISOR_FAIL_STOP_CLEANUP)
        })
    }

    /// Resolve once the supervisor's pre-send fail-stop evidence is captured.
    async fn wait_request_evidence(&self) -> Captured {
        loop {
            let notified = self.request_evidence.notified();
            if let Some(event) = self.request_evidence() {
                return event;
            }
            notified.await;
        }
    }

    fn render(&self, names: &[&str]) -> String {
        let mut out = String::new();
        for event in self.snapshot() {
            if names.iter().any(|name| event.name == *name) || event.name.starts_with("serve.") {
                let _ = writeln!(
                    out,
                    "  +{:>6}ms {} {:?}",
                    event.at.as_millis(),
                    event.name,
                    event.fields
                );
            }
        }
        out
    }
}

impl<S: Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let captured = Captured {
            at: self.start.elapsed(),
            name: event.metadata().name().to_owned(),
            fields: visitor.fields,
        };
        let is_request_evidence = captured.name == SUPERVISOR_FAIL_STOP_EVENT
            && captured.fields.get("cleanup").map(String::as_str)
                == Some(SUPERVISOR_FAIL_STOP_CLEANUP);
        self.events.lock().expect("capture lock").push(captured);
        if is_request_evidence {
            self.request_evidence.notify_waiters();
        }
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Run `body` with the capture installed on a current-thread runtime. The
/// runtime is shut down in the background afterwards: an abandoned shutdown
/// leaves tasks that production would discard with `process::exit(1)`.
fn on_current_thread<F: Future>(capture: &Capture, body: F) -> F::Output {
    let _subscriber = tracing::subscriber::set_default(
        tracing_subscriber::registry()
            .with(capture.clone().with_filter(tracing_subscriber::filter::LevelFilter::INFO)),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread proof runtime");
    let output = runtime.block_on(body);
    runtime.shutdown_background();
    output
}

fn shared_ipv4_intercept_program_present() -> bool {
    overdrive_netlink::nft::observe_shared_ip_intercept()
        .expect("observe the shared IPv4 intercept program")
        .is_some()
}

/// Given a real `serve` composition whose supervisor has audited the shared
/// network healthy across a baseline window.
async fn healthy_real_serve(root: &std::path::Path, capture: &Capture) -> ServeHandle {
    let data_dir = root.join("data");
    let config_dir = root.join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    let handle = tokio::time::timeout(
        BOOT_BOUND,
        overdrive_cli::commands::serve::run_with_kek(
            ServeArgs { bind: "127.0.0.1:0".parse().expect("loopback bind"), data_dir, config_dir },
            Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
        ),
    )
    .await
    .expect("the real composition boots within the harness bound")
    .expect("the real composition boots");
    assert!(
        shared_ipv4_intercept_program_present(),
        "precondition: boot installed the shared IPv4 intercept program"
    );
    tokio::time::sleep(HEALTHY_BASELINE_WINDOW).await;
    assert_eq!(
        capture.count(UNHEALTHY_EVENT),
        0,
        "precondition: the shared network stays healthy across one audit before the fault\n{}",
        capture.render(&[UNHEALTHY_EVENT, RETRY_EVENT, SUPERVISOR_FAIL_STOP_EVENT])
    );
    assert!(
        shared_ipv4_intercept_program_present(),
        "precondition: the shared IPv4 intercept program survives the baseline window"
    );
    handle
}

/// When the shared IPv4 intercept program is lost from the host kernel, wait
/// for the supervisor's own fail-stop evidence (the request is buffered once
/// it is observed — see the module docs).
async fn lose_shared_ipv4_rules(capture: &Capture) -> (Duration, Captured) {
    let lost_at = capture.start.elapsed();
    overdrive_netlink::nft::delete_table(SHARED_IPV4_INTERCEPT_TABLE)
        .expect("delete the shared IPv4 intercept table");
    assert!(
        !shared_ipv4_intercept_program_present(),
        "the shared IPv4 intercept program is absent after the host-side loss"
    );
    let evidence = tokio::time::timeout(REQUEST_OBSERVATION_BOUND, capture.wait_request_evidence())
        .await
        .unwrap_or_else(|_| {
            panic!(
                "upstream precondition: no supervisor fail-stop evidence within \
                 {REQUEST_OBSERVATION_BOUND:?} of the loss — the lifetime claims are unreachable\n{}",
                capture.render(&[UNHEALTHY_EVENT, RETRY_EVENT, SUPERVISOR_FAIL_STOP_EVENT])
            )
        });
    (lost_at, evidence)
}

fn staging_root(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(shared_staging_root())
        .expect("proof tempdir on the metal staging root")
}

struct Verdict {
    claim: &'static str,
    green: bool,
    evidence: String,
}

fn render_verdicts(verdicts: &[Verdict]) -> String {
    let mut table = String::new();
    for verdict in verdicts {
        let mark = if verdict.green { "GREEN" } else { "RED  " };
        let _ = writeln!(table, "  [{mark}] {} — {}", verdict.claim, verdict.evidence);
    }
    table
}

fn is_expected_request(request: &ServeShutdownRequest) -> bool {
    let ServeShutdownRequest::SharedGuestNetwork(fail_stop) = request;
    fail_stop.cause == SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded
        && fail_stop.component == SharedGuestNetworkComponent::IpRules
}

fn describe_outcome(outcome: Option<&Result<ServeExit, String>>) -> String {
    match outcome {
        None => "lifetime did not return within the observation bound".to_owned(),
        Some(Err(error)) => format!("lifetime returned Err({error})"),
        Some(Ok(exit)) => format!("{exit:?} exit_code={}", exit.exit_code()),
    }
}

// ---------------------------------------------------------------------------
// Proofs
// ---------------------------------------------------------------------------

/// Population control: an operator `SIGINT` on a healthy node is the normal
/// graceful shutdown with exit status 0 and arms no fail-stop bound.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[serial(cgroup)]
fn an_operator_interrupt_stops_a_healthy_serve_with_status_zero() {
    let capture = Capture::new();
    let (verdicts, trace) = on_current_thread(&capture, async {
        let root = staging_root("serve-lifetime-sigint-");
        let handle = healthy_real_serve(root.path(), &capture).await;
        let (signals, driver) = driven_signals();
        driver.deliver(ServeSignal::Interrupt);
        let clock = RecordingClock::new();
        let outcome = tokio::time::timeout(
            LIFETIME_OBSERVATION_BOUND,
            ServeLifetime::new(signals, Arc::new(clock.clone())).run(handle),
        )
        .await
        .ok()
        .map(|result| result.map_err(|error| error.to_string()));
        let described = describe_outcome(outcome.as_ref());
        let verdicts = vec![
            Verdict {
                claim: "an operator SIGINT ends a healthy serve lifetime with exit status 0",
                green: matches!(
                    &outcome,
                    Some(Ok(exit @ ServeExit::Stopped { signal: ServeSignal::Interrupt }))
                        if exit.exit_code() == 0
                ),
                evidence: described,
            },
            Verdict {
                claim: "a healthy SIGINT shutdown arms no fail-stop outer bound",
                green: clock.armed().is_empty(),
                evidence: format!("armed sleeps on the injected clock: {:?}", clock.armed()),
            },
        ];
        drop(root);
        (verdicts, capture.render(&[UNHEALTHY_EVENT, SUPERVISOR_FAIL_STOP_EVENT]))
    });
    let table = render_verdicts(&verdicts);
    eprintln!("verdicts:\n{table}trace:\n{trace}");
    assert!(verdicts.iter().all(|v| v.green), "healthy SIGINT control violated:\n{table}");
}

/// A real shared-network fail-stop reaches the lifetime owner, wins over a
/// `SIGINT` that is ready at the same poll, arms the ten-second outer bound on
/// the injected clock, drains, and maps to exit status 1.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[serial(cgroup)]
fn a_shared_network_fail_stop_wins_over_a_ready_interrupt_and_exits_status_one() {
    let capture = Capture::new();
    let (verdicts, trace) = on_current_thread(&capture, async {
        let root = staging_root("serve-lifetime-fail-stop-race-");
        let handle = healthy_real_serve(root.path(), &capture).await;
        let (lost_at, evidence) = lose_shared_ipv4_rules(&capture).await;
        // The request is buffered (module docs). Make the operator signal
        // ready too, before the lifetime's first poll.
        let (signals, driver) = driven_signals();
        driver.deliver(ServeSignal::Interrupt);
        let clock = RecordingClock::new();
        let outcome = tokio::time::timeout(
            LIFETIME_OBSERVATION_BOUND,
            ServeLifetime::new(signals, Arc::new(clock.clone())).run(handle),
        )
        .await
        .ok()
        .map(|result| result.map_err(|error| error.to_string()));
        let described = describe_outcome(outcome.as_ref());
        let request = match &outcome {
            Some(Ok(ServeExit::SharedGuestNetworkFailStop { request, .. })) => {
                Some(request.clone())
            }
            _ => None,
        };
        let verdicts = vec![
            Verdict {
                claim: "the real shared-network fail-stop request reaches the serve lifetime owner",
                green: request.as_ref().is_some_and(is_expected_request),
                evidence: format!(
                    "supervisor evidence {}ms after the loss; outcome: {described}",
                    evidence.at.saturating_sub(lost_at).as_millis()
                ),
            },
            Verdict {
                claim: "the internal request wins over a SIGINT ready at the same poll",
                green: request.is_some() && driver.consumed() == 0,
                evidence: format!(
                    "SIGINT consumed by the lifetime: {} (delivered before its first poll)",
                    driver.consumed()
                ),
            },
            Verdict {
                claim: "fail-stop shutdown is outer-bounded to exactly ten seconds on the injected clock",
                green: clock.armed() == vec![FAIL_STOP_OUTER_BOUND],
                evidence: format!("armed sleeps on the injected clock: {:?}", clock.armed()),
            },
            Verdict {
                claim: "the shared-network fail-stop outcome maps to exit status 1",
                green: matches!(&outcome, Some(Ok(exit)) if exit.exit_code() == 1),
                evidence: described.clone(),
            },
            Verdict {
                claim: "an unobstructed shutdown drains before the bound elapses",
                green: matches!(
                    &outcome,
                    Some(Ok(ServeExit::SharedGuestNetworkFailStop {
                        cleanup: FailStopCleanup::DrainedBeforeExit,
                        ..
                    }))
                ),
                evidence: described,
            },
        ];
        drop(root);
        (verdicts, capture.render(&[UNHEALTHY_EVENT, RETRY_EVENT, SUPERVISOR_FAIL_STOP_EVENT]))
    });
    let table = render_verdicts(&verdicts);
    eprintln!("verdicts:\n{table}trace:\n{trace}");
    assert!(
        verdicts.iter().all(|v| v.green),
        "serve lifetime fail-stop contract violated:\n{table}"
    );
}

/// When the ten-second outer bound elapses on the injected clock before
/// shutdown completes, the lifetime owner stops waiting, reports the
/// abandonment, and still maps to exit status 1.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[serial(cgroup)]
fn a_shared_network_fail_stop_shutdown_is_abandoned_when_the_ten_second_bound_elapses() {
    let capture = Capture::new();
    let (verdicts, trace) = on_current_thread(&capture, async {
        let root = staging_root("serve-lifetime-fail-stop-bound-");
        let handle = healthy_real_serve(root.path(), &capture).await;
        let (lost_at, evidence) = lose_shared_ipv4_rules(&capture).await;
        let (signals, driver) = driven_signals();
        let clock = RecordingClock::new();
        let lifetime =
            tokio::spawn(ServeLifetime::new(signals, Arc::new(clock.clone())).run(handle));
        let armed = tokio::time::timeout(LIFETIME_OBSERVATION_BOUND, clock.wait_armed()).await;
        let armed_durations = clock.armed();
        if armed.is_ok() {
            // Exactly ten seconds of logical time pass before shutdown can
            // complete (module docs).
            clock.advance(FAIL_STOP_OUTER_BOUND);
        }
        let outcome = if armed.is_ok() {
            tokio::time::timeout(LIFETIME_OBSERVATION_BOUND, lifetime).await.ok().map(|joined| {
                joined.expect("lifetime task joins").map_err(|error| error.to_string())
            })
        } else {
            lifetime.abort();
            None
        };
        let described = describe_outcome(outcome.as_ref());
        let verdicts = vec![
            Verdict {
                claim: "the real shared-network fail-stop request reaches the serve lifetime owner",
                green: matches!(
                    &outcome,
                    Some(Ok(ServeExit::SharedGuestNetworkFailStop { request, .. }))
                        if is_expected_request(request)
                ),
                evidence: format!(
                    "supervisor evidence {}ms after the loss; outcome: {described}",
                    evidence.at.saturating_sub(lost_at).as_millis()
                ),
            },
            Verdict {
                claim: "the lifetime arms exactly the ten-second outer bound when shutdown starts",
                green: armed_durations == vec![FAIL_STOP_OUTER_BOUND],
                evidence: format!(
                    "armed sleeps: {armed_durations:?} (arm observed: {})",
                    armed.is_ok()
                ),
            },
            Verdict {
                claim: "shutdown is abandoned when ten seconds elapse on the injected clock",
                green: matches!(
                    &outcome,
                    Some(Ok(ServeExit::SharedGuestNetworkFailStop {
                        cleanup: FailStopCleanup::AbandonedAtExit,
                        ..
                    }))
                ),
                evidence: described.clone(),
            },
            Verdict {
                claim: "an abandoned fail-stop shutdown still maps to exit status 1",
                green: matches!(&outcome, Some(Ok(exit)) if exit.exit_code() == 1),
                evidence: format!("{described}; operator signals consumed: {}", driver.consumed()),
            },
        ];
        drop(root);
        (verdicts, capture.render(&[UNHEALTHY_EVENT, RETRY_EVENT, SUPERVISOR_FAIL_STOP_EVENT]))
    });
    let table = render_verdicts(&verdicts);
    eprintln!("verdicts:\n{table}trace:\n{trace}");
    assert!(
        verdicts.iter().all(|v| v.green),
        "serve lifetime outer-bound contract violated:\n{table}"
    );
}

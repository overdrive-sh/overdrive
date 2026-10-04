//! netns-density-295 correctness-recovery proof §3.6 — the `serve` process
//! lifetime owner consumes the shared guest-network fail-stop (correctness
//! gap 4).
//!
//! # Contract under proof
//!
//! `docs/feature/netns-density-295/feature-delta.md` § internal fail-stop
//! request to the CLI (D-295-DISTILL-8), D-295-R17, and ADR-0124: the retained
//! shared-network supervisor's typed [`ServeShutdownRequest`] reaches the CLI
//! process-lifetime owner; the owner selects it before an operator `SIGINT`
//! when both are ready; shutdown after the request is outer-bounded to ten
//! seconds; the outcome maps to process exit status `1`. A shared-network loss
//! the supervisor repairs (D-295-R15) never ends the lifetime.
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
//! # The fault is real host state
//!
//! The fail-stop cause is real host state, not a fabricated request. While no
//! allocation holds a capability record, one owned constant rule of the shared
//! IPv4 intercept program (`table ip overdrive-mtls`) is rewritten to a
//! different listener target: the prerouting rule whose TPROXY target is leg F
//! names a canonical wrong port, and every other owned object is unchanged
//! (the S19-A wrong-target shape). The recorded targets are read from the
//! kernel (`nft -j list chain`, a diagnostic read) and checked against the
//! netlink adapter's own observation before the rewrite, which goes through the
//! adapter's conditional replace. The supervisor's one-second audit detects the
//! identity mismatch. Every repair attempt is refused, because a present
//! program whose target differs from the recorded one is never rewritten
//! (S19-A, unchanged by D-295-R15). At the five-second recovery deadline the
//! supervisor emits `guest_network.shared_owner_fail_stop
//! cleanup=abandoned_to_shutdown` and sends its typed request
//! (`RecoveryDeadlineExceeded`, component `IpRules`). The worker's shared
//! guard deletes only its recorded identity, so the retargeted program would
//! outlive the proof; a conditional cleanup deletes it afterwards only while
//! the kernel still carries exactly the retargeted identity.
//!
//! Deleting the whole table is not a fail-stop cause once D-295-R15 lands
//! (DELIVER step 08-03): the worker recreates an absent program at the recorded
//! targets. The contrast body uses that loss as its stimulus and proves that a
//! repaired loss never ends the serve lifetime: the lifetime stays pending with
//! no fail-stop request through the recovery window, and a later `SIGINT`
//! stops it with status 0.
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
//! of the supervisor's own deadline), a pre-fault healthy baseline window, and
//! the contrast body's repair window, which spans the supervisor's one-second
//! detection and its five-second recovery deadline with margin.
//!
//! Hypothesis (fail-stop bodies): an owned rule rewritten to a wrong target is
//! never repaired, today or under D-295-R15, so the supervisor still reaches
//! its deadline and the built lifetime port maps its request to status 1.
//! Prediction: the three fail-stop bodies are GREEN today and stay GREEN
//! through DELIVER step 08-03. Falsification: a request with another cause or
//! component, a lifetime that consumes the ready `SIGINT`, or a repaired
//! retarget.
//!
//! Hypothesis (contrast body): today's worker convergence refuses an absent
//! program — the observed identity is not the recorded one, so it returns
//! `PostconditionMismatch` — and the deleted table therefore reaches the
//! deadline and ends the lifetime with status 1. Prediction: RED until DELIVER
//! step 08-03 repairs an absent program, GREEN after it. Falsification: a GREEN
//! run before 08-03, or a fail-stop after it.
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
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::serve_lifetime::{
    FAIL_STOP_OUTER_BOUND, FailStopCleanup, ServeExit, ServeLifetime, ServeSignal,
};
use overdrive_core::guest_network::{
    ServeShutdownRequest, SharedGuestNetworkComponent, SharedGuestNetworkFailStopCause,
};
use overdrive_netlink::nft::{self, SharedIpInterceptIdentity};
use serde_json::Value;
use serial_test::serial;
use tokio::sync::Notify;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::serve_lifetime_support::{RecordingClock, driven_signals};
use super::vm_walking_skeleton::shared_staging_root;

const SHARED_IPV4_INTERCEPT_TABLE: &str = "overdrive-mtls";
const SHARED_IPV4_PREROUTING_CHAIN: &str = "prerouting";
/// Pinned supervisor evidence emitted immediately before the typed request.
const SUPERVISOR_FAIL_STOP_EVENT: &str = "guest_network.shared_owner_fail_stop";
const SUPERVISOR_FAIL_STOP_CLEANUP: &str = "abandoned_to_shutdown";
const UNHEALTHY_EVENT: &str = "guest_network.shared_owner_unhealthy";
const RETRY_EVENT: &str = "guest_network.shared_owner_retry";
const RECOVERED_EVENT: &str = "guest_network.shared_owner_recovered";
/// Harness bound for the real composition to boot.
const BOOT_BOUND: Duration = Duration::from_secs(90);
/// Pre-fault window spanning at least one one-second supervisor audit.
const HEALTHY_BASELINE_WINDOW: Duration = Duration::from_millis(1_500);
/// Harness bound for the supervisor's own 1 s detection + 5 s repair deadline.
const REQUEST_OBSERVATION_BOUND: Duration = Duration::from_secs(20);
/// Harness bound for a lifetime run that should end promptly.
const LIFETIME_OBSERVATION_BOUND: Duration = Duration::from_secs(30);
/// Real-time window after a repairable loss: the supervisor's 1 s detection
/// plus its 5 s recovery deadline, with margin. A loss that were not repaired
/// would have ended the lifetime inside it.
const REPAIR_OBSERVATION_WINDOW: Duration = Duration::from_secs(10);

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

    fn first(&self, name: &str) -> Option<Captured> {
        self.snapshot().into_iter().find(|event| event.name == name)
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

/// The shared IPv4 intercept program identity the netlink adapter observes.
fn observe_shared_ipv4_program() -> Option<SharedIpInterceptIdentity> {
    nft::observe_shared_ip_intercept().expect("observe the shared IPv4 intercept program")
}

fn shared_ipv4_intercept_program_present() -> bool {
    observe_shared_ipv4_program().is_some()
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

/// Every TPROXY statement's target port in an `nft -j` listing, in listing
/// order (rule order within the chain, statement order within the rule).
fn tproxy_ports(listing: &Value) -> Vec<u16> {
    let statements = listing
        .get("nftables")
        .and_then(Value::as_array)
        .expect("the nft JSON listing carries an `nftables` array")
        .iter()
        .filter_map(|entry| entry.get("rule"))
        .filter_map(|rule| rule.get("expr").and_then(Value::as_array))
        .flatten();
    statements
        .filter_map(|statement| statement.get("tproxy"))
        .map(|tproxy| {
            let port = tproxy.get("port").expect("a TPROXY statement names its target port");
            port.as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .or_else(|| port.as_str().and_then(|port| port.parse().ok()))
                .unwrap_or_else(|| panic!("unreadable TPROXY target port in the listing: {port}"))
        })
        .collect()
}

/// The recorded leg-F and leg-C listener targets, read from the kernel. The
/// prerouting chain carries exactly two TPROXY rules; in chain order the first
/// targets leg F and the second leg C.
fn recorded_listener_targets() -> (u16, u16) {
    let output = Command::new("nft")
        .args([
            "-j",
            "list",
            "chain",
            "ip",
            SHARED_IPV4_INTERCEPT_TABLE,
            SHARED_IPV4_PREROUTING_CHAIN,
        ])
        .output()
        .expect("run the diagnostic `nft -j list chain` read");
    assert!(
        output.status.success(),
        "`nft -j list chain ip {SHARED_IPV4_INTERCEPT_TABLE} {SHARED_IPV4_PREROUTING_CHAIN}` \
         failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let listing: Value =
        serde_json::from_slice(&output.stdout).expect("the nft JSON listing parses");
    let ports = tproxy_ports(&listing);
    let [leg_f, leg_c] = ports.as_slice() else {
        panic!(
            "expected exactly two TPROXY targets (leg F, leg C) in the listing, found {ports:?}"
        );
    };
    (*leg_f, *leg_c)
}

/// Assert that `wrong` differs from `recorded` in exactly one owned constant
/// rule — one prerouting rule — and return that rule's index.
fn one_differing_owned_rule(
    recorded: &SharedIpInterceptIdentity,
    wrong: &SharedIpInterceptIdentity,
) -> usize {
    let (recorded_tables, recorded_sets, recorded_prerouting, recorded_output) =
        recorded.normalized_parts();
    let (wrong_tables, wrong_sets, wrong_prerouting, wrong_output) = wrong.normalized_parts();
    assert_eq!(recorded_tables, wrong_tables, "the retarget keeps the table and chains");
    assert_eq!(recorded_sets, wrong_sets, "the retarget keeps the dynamic-set schemas");
    assert_eq!(recorded_output, wrong_output, "the retarget keeps every output rule");
    assert_eq!(
        recorded_prerouting.len(),
        wrong_prerouting.len(),
        "the retarget keeps the prerouting rule count"
    );
    let differing: Vec<usize> = recorded_prerouting
        .iter()
        .zip(&wrong_prerouting)
        .enumerate()
        .filter(|(_, (recorded, wrong))| recorded != wrong)
        .map(|(index, _)| index)
        .collect();
    let [index] = differing.as_slice() else {
        panic!(
            "the retarget must change exactly one owned rule; prerouting differs at {differing:?}"
        );
    };
    *index
}

/// A canonical wrong-target program the proof wrote over the recorded one.
/// Dropping it deletes the program only while the kernel still carries exactly
/// that identity, so it never removes state it did not write.
struct RetargetedSharedRule {
    wrong: SharedIpInterceptIdentity,
    record: String,
}

impl Drop for RetargetedSharedRule {
    fn drop(&mut self) {
        match nft::observe_shared_ip_intercept() {
            Ok(Some(current)) if current == self.wrong => {
                match nft::replace_shared_ip_intercept_atomically(Some(&self.wrong), None) {
                    Ok(()) => eprintln!(
                        "retarget cleanup: deleted the retargeted `ip \
                         {SHARED_IPV4_INTERCEPT_TABLE}` program"
                    ),
                    Err(error) => eprintln!(
                        "retarget cleanup: FAILED to delete the retargeted `ip \
                         {SHARED_IPV4_INTERCEPT_TABLE}` program: {error}"
                    ),
                }
            }
            Ok(Some(_)) => eprintln!(
                "retarget cleanup: left `ip {SHARED_IPV4_INTERCEPT_TABLE}` in place; it no \
                 longer carries the retargeted identity"
            ),
            Ok(None) => eprintln!(
                "retarget cleanup: nothing to delete; `ip {SHARED_IPV4_INTERCEPT_TABLE}` is absent"
            ),
            Err(error) => eprintln!(
                "retarget cleanup: left `ip {SHARED_IPV4_INTERCEPT_TABLE}` in place; observation \
                 failed: {error}"
            ),
        }
    }
}

/// When one owned constant rule of the shared IPv4 intercept program is
/// rewritten to a different listener target, wait for the supervisor's own
/// fail-stop evidence (the request is buffered once it is observed — see the
/// module docs).
async fn retarget_one_owned_ipv4_rule(
    capture: &Capture,
) -> (Duration, Captured, RetargetedSharedRule) {
    let (leg_f, leg_c) = recorded_listener_targets();
    let recorded = SharedIpInterceptIdentity::for_listener_ports(leg_f, leg_c)
        .expect("canonical identity for the recorded listener targets");
    assert_eq!(
        observe_shared_ipv4_program().as_ref(),
        Some(&recorded),
        "precondition: the listed TPROXY targets (leg F {leg_f}, leg C {leg_c}) rebuild exactly \
         the program the adapter observes"
    );
    let wrong_leg_f = (1..=u16::MAX)
        .map(|step| leg_f.wrapping_add(step))
        .find(|port| ![0, leg_f, leg_c].contains(port))
        .expect("a non-zero listener target distinct from both recorded targets");
    let wrong = SharedIpInterceptIdentity::for_listener_ports(wrong_leg_f, leg_c)
        .expect("canonical identity for the wrong leg-F target");
    let changed_rule = one_differing_owned_rule(&recorded, &wrong);

    let fault_at = capture.start.elapsed();
    nft::replace_shared_ip_intercept_atomically(Some(&recorded), Some(&wrong)).expect(
        "rewrite the owned leg-F rule to the wrong target (the conditional replace refuses \
         non-empty sets; this proof deploys nothing)",
    );
    let retarget = RetargetedSharedRule {
        wrong,
        record: format!(
            "fault +{}ms: rewrote `ip {SHARED_IPV4_INTERCEPT_TABLE}` \
             {SHARED_IPV4_PREROUTING_CHAIN} rule {changed_rule} from leg-F target \
             127.0.0.1:{leg_f} to 127.0.0.1:{wrong_leg_f}; leg C 127.0.0.1:{leg_c}, the table \
             and chains, the set schemas, and every output rule are unchanged",
            fault_at.as_millis()
        ),
    };
    eprintln!("{}", retarget.record);
    assert_eq!(
        observe_shared_ipv4_program().as_ref(),
        Some(&retarget.wrong),
        "the kernel carries exactly the retargeted program after the rewrite"
    );
    let evidence = tokio::time::timeout(REQUEST_OBSERVATION_BOUND, capture.wait_request_evidence())
        .await
        .unwrap_or_else(|_| {
            panic!(
                "upstream precondition: no supervisor fail-stop evidence within \
                 {REQUEST_OBSERVATION_BOUND:?} of the retarget — the lifetime claims are \
                 unreachable\n{}",
                capture.render(&[UNHEALTHY_EVENT, RETRY_EVENT, SUPERVISOR_FAIL_STOP_EVENT])
            )
        });
    (fault_at, evidence, retarget)
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

fn describe_event(event: Option<&Captured>, since: Duration) -> String {
    event.map_or_else(
        || "not captured".to_owned(),
        |event| {
            format!(
                "captured {}ms after the loss with {:?}",
                event.at.saturating_sub(since).as_millis(),
                event.fields
            )
        },
    )
}

// ---------------------------------------------------------------------------
// Proofs
// ---------------------------------------------------------------------------

/// Population control: an operator `SIGINT` on a healthy node is the normal
/// graceful shutdown with exit status 0 and arms no fail-stop bound.
///
/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-68 — The operator's serve process exits with status 1 on shared-network fail-stop
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
///
/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-68 — The operator's serve process exits with status 1 on shared-network fail-stop
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[serial(cgroup)]
fn a_shared_network_fail_stop_wins_over_a_ready_interrupt_and_exits_status_one() {
    let capture = Capture::new();
    let (verdicts, trace) = on_current_thread(&capture, async {
        let root = staging_root("serve-lifetime-fail-stop-race-");
        let handle = healthy_real_serve(root.path(), &capture).await;
        let (fault_at, evidence, retarget) = retarget_one_owned_ipv4_rule(&capture).await;
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
                    "supervisor evidence {}ms after the retarget; outcome: {described}",
                    evidence.at.saturating_sub(fault_at).as_millis()
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
        let trace = format!(
            "  {}\n{}",
            retarget.record,
            capture.render(&[UNHEALTHY_EVENT, RETRY_EVENT, SUPERVISOR_FAIL_STOP_EVENT])
        );
        drop(retarget);
        drop(root);
        (verdicts, trace)
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
///
/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-68 — The operator's serve process exits with status 1 on shared-network fail-stop
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[serial(cgroup)]
fn a_shared_network_fail_stop_shutdown_is_abandoned_when_the_ten_second_bound_elapses() {
    let capture = Capture::new();
    let (verdicts, trace) = on_current_thread(&capture, async {
        let root = staging_root("serve-lifetime-fail-stop-bound-");
        let handle = healthy_real_serve(root.path(), &capture).await;
        let (fault_at, evidence, retarget) = retarget_one_owned_ipv4_rule(&capture).await;
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
                    "supervisor evidence {}ms after the retarget; outcome: {described}",
                    evidence.at.saturating_sub(fault_at).as_millis()
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
        let trace = format!(
            "  {}\n{}",
            retarget.record,
            capture.render(&[UNHEALTHY_EVENT, RETRY_EVENT, SUPERVISOR_FAIL_STOP_EVENT])
        );
        drop(retarget);
        drop(root);
        (verdicts, trace)
    });
    let table = render_verdicts(&verdicts);
    eprintln!("verdicts:\n{table}trace:\n{trace}");
    assert!(
        verdicts.iter().all(|v| v.green),
        "serve lifetime outer-bound contract violated:\n{table}"
    );
}

/// Contrast: deleting the whole shared IPv4 intercept table is a loss the
/// supervisor repairs (D-295-R15), so it never ends the serve lifetime. The
/// lifetime stays pending through the recovery window with no fail-stop
/// request, the program is recreated at the recorded targets, and a later
/// operator `SIGINT` stops the lifetime with exit status 0.
///
/// Predicted RED until DELIVER step 08-03: today's worker convergence refuses
/// an absent program, so the deleted table reaches the recovery deadline and
/// the fail-stop request ends the lifetime with status 1.
///
/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-68 — The operator's serve process exits with status 1 on shared-network fail-stop
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[serial(cgroup)]
fn a_repaired_shared_network_loss_never_ends_serve() {
    let capture = Capture::new();
    let (verdicts, trace) = on_current_thread(&capture, async {
        let root = staging_root("serve-lifetime-repaired-loss-");
        let handle = healthy_real_serve(root.path(), &capture).await;
        let pre_fault = observe_shared_ipv4_program()
            .expect("precondition: boot installed the shared IPv4 intercept program");
        let (signals, driver) = driven_signals();
        let clock = RecordingClock::new();
        let mut lifetime =
            tokio::spawn(ServeLifetime::new(signals, Arc::new(clock.clone())).run(handle));

        let lost_at = capture.start.elapsed();
        nft::delete_table(SHARED_IPV4_INTERCEPT_TABLE)
            .expect("delete the shared IPv4 intercept table");
        let record = format!(
            "fault +{}ms: deleted the whole `ip {SHARED_IPV4_INTERCEPT_TABLE}` table",
            lost_at.as_millis()
        );
        eprintln!("{record}");
        assert!(
            !shared_ipv4_intercept_program_present(),
            "the shared IPv4 intercept program is absent after the host-side loss"
        );

        tokio::time::sleep(REPAIR_OBSERVATION_WINDOW).await;
        let pending_through_window = !lifetime.is_finished();
        let fail_stop_events = capture.count(SUPERVISOR_FAIL_STOP_EVENT);
        let unhealthy = capture.first(UNHEALTHY_EVENT);
        let recovered = capture.first(RECOVERED_EVENT);
        let program_after_window = observe_shared_ipv4_program();
        let armed_through_window = clock.armed();

        // A finished lifetime has dropped its signal source; the operator
        // signal is delivered only to a lifetime that is still running, so a
        // fail-stop that already ended it is reported by the verdicts below.
        let interrupt_delivered = !lifetime.is_finished();
        if interrupt_delivered {
            driver.deliver(ServeSignal::Interrupt);
        }
        let joined = tokio::time::timeout(LIFETIME_OBSERVATION_BOUND, &mut lifetime).await.ok();
        if joined.is_none() {
            lifetime.abort();
        }
        let outcome = joined.map(|joined| {
            joined
                .map_err(|error| format!("lifetime task did not join: {error}"))
                .and_then(|result| result.map_err(|error| error.to_string()))
        });
        let described = describe_outcome(outcome.as_ref());
        let ip_rules = format!("{:?}", SharedGuestNetworkComponent::IpRules);
        let verdicts = vec![
            Verdict {
                claim: "a repaired loss leaves the serve lifetime pending through the recovery window",
                green: pending_through_window,
                evidence: format!(
                    "lifetime finished within {REPAIR_OBSERVATION_WINDOW:?} of the loss: {}; \
                     final outcome: {described}",
                    !pending_through_window
                ),
            },
            Verdict {
                claim: "the supervisor requests no fail-stop for a loss it repairs",
                green: fail_stop_events == 0,
                evidence: format!("{SUPERVISOR_FAIL_STOP_EVENT} events: {fail_stop_events}"),
            },
            Verdict {
                claim: "the loss is detected as an IpRules failure, not missed",
                green: unhealthy.as_ref().is_some_and(|event| {
                    event.fields.get("component").map(String::as_str) == Some(ip_rules.as_str())
                }),
                evidence: format!(
                    "{UNHEALTHY_EVENT}: {}",
                    describe_event(unhealthy.as_ref(), lost_at)
                ),
            },
            Verdict {
                claim: "the supervisor reports the repair after detecting the loss",
                green: matches!(
                    (&unhealthy, &recovered),
                    (Some(unhealthy), Some(recovered)) if recovered.at >= unhealthy.at
                ),
                evidence: format!(
                    "{RECOVERED_EVENT}: {}",
                    describe_event(recovered.as_ref(), lost_at)
                ),
            },
            Verdict {
                claim: "the program is repaired in place at the recorded targets",
                green: program_after_window.as_ref() == Some(&pre_fault),
                evidence: format!(
                    "observed after the window: {}",
                    match &program_after_window {
                        None => "absent",
                        Some(observed) if *observed == pre_fault => "the pre-fault identity",
                        Some(_) => "a different identity",
                    }
                ),
            },
            Verdict {
                claim: "the injected clock arms no fail-stop bound while the lifetime waits",
                green: armed_through_window.is_empty(),
                evidence: format!("armed sleeps through the window: {armed_through_window:?}"),
            },
            Verdict {
                claim: "a later operator SIGINT stops the serve lifetime with exit status 0",
                green: matches!(
                    &outcome,
                    Some(Ok(exit @ ServeExit::Stopped { signal: ServeSignal::Interrupt }))
                        if exit.exit_code() == 0
                ),
                evidence: format!(
                    "{described}; SIGINT delivered: {interrupt_delivered}; operator signals \
                     consumed: {}",
                    driver.consumed()
                ),
            },
            Verdict {
                claim: "the graceful stop after a repair arms no fail-stop bound",
                green: clock.armed().is_empty(),
                evidence: format!("armed sleeps on the injected clock: {:?}", clock.armed()),
            },
        ];
        let trace = format!(
            "  {record}\n{}",
            capture.render(&[
                UNHEALTHY_EVENT,
                RETRY_EVENT,
                RECOVERED_EVENT,
                SUPERVISOR_FAIL_STOP_EVENT
            ])
        );
        drop(root);
        (verdicts, trace)
    });
    let table = render_verdicts(&verdicts);
    eprintln!("verdicts:\n{table}trace:\n{trace}");
    assert!(
        verdicts.iter().all(|v| v.green),
        "repaired shared-network loss contract violated:\n{table}"
    );
}

//! The VM probe task's run numbering and observed streak, and its
//! not-sampled outcomes (netns-density-295: PROBE-RUN-COUNT, ADR-0191, the
//! producer; PROBE-NOSAMPLE, ADR-0187; PROBE-NOSAMPLE-EVENT, ADR-0192).
//!
//! Driving port: `ProbeRunner::start_alloc` for a VM allocation whose probe
//! target is its `workload_addr`, so every run is a
//! `GuestStreamDial::dial(target, DialPurpose::Probe)`; Sim adapters at the
//! driven ports (`SimGuestStreamDial`, `SimObservationStore`, `SimClock`).
//! Observables: the stored probe row (through
//! `ObservationStore::list_probe_results_for_alloc`), the dials made, and the
//! `probe.not_sampled` events.
//!
//! Layer 2 (in-memory, the production probe runner over Sim adapters):
//! property-based over every sequence of run outcomes. The composed protocol
//! with `ServiceLifecycle` is R5-PROBE-COUNT and the `probe-count`
//! conformance driver in `overdrive-control-plane`. The dial-consumer cells
//! (a TCP probe healthy iff the dial is `Ok`, an HTTP probe over the
//! `GuestStream`, eight probes dialling at once; INB-1, R5-53, R5-60) are
//! pending the attachment-cut step 9; the run-counting cells are pending
//! DELIVER step 9.

#![allow(
    clippy::single_match_else,
    clippy::single_option_map,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    reason = "test fixtures assert their own preconditions; the CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::guest_transport::dial::{DialPurpose, GuestDialError, GuestStreamDial};
use overdrive_core::guest_transport::wire::KernelErrno;
use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GUEST_FLOW_CONTROL_PORT, GUEST_FLOW_STREAM_PORT, GuestCid, GuestFlowPorts,
    GuestTransportAssignment, LaunchToken,
};
use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{AllocationSpec, DriverPayload, Resources, VmPayload};
use overdrive_core::traits::observation_store::{ObservationStore, ObservationStoreError};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::guest_stream_dial::{SimDialScript, SimGuestStreamDial};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use proptest::prelude::*;
use tracing_subscriber::layer::SubscriberExt as _;

use super::ProbeRunner;

const WORKLOAD_ADDR: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 2);
const LIVENESS_PORT: u16 = 8083;
const INTERVAL: Duration = Duration::from_secs(1);

/// One probe run's outcome at the dial or the row write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Run {
    /// The dial returns `Ok`: a written `Pass` row.
    Pass,
    /// The dial is refused by the guest application: a written `Fail` row.
    Fail,
    /// The dial is refused for node flow capacity: no sample.
    Capacity,
    /// The dial is refused because node forwarding is quiesced: no sample.
    Quiesced,
    /// The dial returns `Ok`, the row's write fails: no row.
    WriteFail,
}

impl Run {
    fn script(self) -> SimDialScript {
        match self {
            Self::Pass | Self::WriteFail => SimDialScript::accept(),
            Self::Fail => {
                SimDialScript::Refuse(GuestDialError::Refused { errno: KernelErrno::ECONNREFUSED })
            }
            Self::Capacity => SimDialScript::Refuse(GuestDialError::Capacity),
            Self::Quiesced => SimDialScript::Refuse(GuestDialError::Quiesced),
        }
    }

    /// The status a written run records; `None` for a run that writes no row.
    const fn written(self) -> Option<bool> {
        match self {
            Self::Pass => Some(false),
            Self::Fail => Some(true),
            Self::Capacity | Self::Quiesced | Self::WriteFail => None,
        }
    }

    /// The `probe.not_sampled` reason of a run that writes no row.
    const fn not_sampled_reason(self) -> Option<&'static str> {
        match self {
            Self::Capacity => Some("node_capacity"),
            Self::Quiesced => Some("quiesced"),
            Self::WriteFail => Some("write_failed"),
            Self::Pass | Self::Fail => None,
        }
    }
}

fn arb_run() -> impl Strategy<Value = Run> {
    prop_oneof![
        Just(Run::Pass),
        Just(Run::Fail),
        Just(Run::Capacity),
        Just(Run::Quiesced),
        Just(Run::WriteFail),
    ]
}

/// The expected stored row's `(run_seq, is Fail, same_status_runs)` after
/// `runs`: the count of written runs, and the trailing count of written runs
/// of the last written status — the model of the producer.
fn expected_counts(runs: &[Run]) -> Option<(u64, bool, u32)> {
    let written: Vec<bool> = runs.iter().filter_map(|run| run.written()).collect();
    let last = *written.last()?;
    let streak = written.iter().rev().take_while(|status| **status == last).count();
    Some((
        u64::try_from(written.len()).expect("small"),
        last,
        u32::try_from(streak).expect("small"),
    ))
}

fn liveness_descriptor() -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role: ProbeRole::Liveness,
        mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port: LIVENESS_PORT },
        timeout_seconds: 1,
        interval_seconds: 1,
        max_attempts: 30,
        failure_threshold: Some(3),
        success_threshold: None,
        inferred: false,
    }
}

fn vm_spec(alloc: &AllocationId) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::for_allocation(&WorkloadId::new("probed").expect("workload"), alloc),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/serve".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/kernel"),
            rootfs: PathBuf::from("/rootfs"),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        probe_descriptors: vec![liveness_descriptor()],
        transport: Some(GuestTransportAssignment::new(
            WORKLOAD_ADDR,
            Ipv4Addr::new(100, 95, 0, 1),
            GuestCid::new(GUEST_CID_BASE).expect("first leased CID"),
            LaunchToken::from_bytes([3; 16]).expect("non-zero launch token"),
            GuestFlowPorts::new(GUEST_FLOW_STREAM_PORT, GUEST_FLOW_CONTROL_PORT)
                .expect("the default guest-flow listener ports"),
        )),
        service_ports: vec![NonZeroU16::new(8080).expect("non-zero port")],
    }
}

/// One captured `probe.not_sampled` event's fields.
#[derive(Clone, Default)]
struct NotSampled(Arc<std::sync::Mutex<Vec<BTreeMap<String, String>>>>);

struct Fields<'a>(&'a mut BTreeMap<String, String>);

impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}").trim_matches('"').to_owned());
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for NotSampled {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if event.metadata().name() != "probe.not_sampled" {
            return;
        }
        let mut fields = BTreeMap::new();
        event.record(&mut Fields(&mut fields));
        self.0.lock().expect("capture lock").push(fields);
    }
}

struct Probed {
    runner: ProbeRunner,
    obs: Arc<SimObservationStore>,
    clock: Arc<SimClock>,
    dial: Arc<SimGuestStreamDial>,
    alloc: AllocationId,
}

async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

impl Probed {
    async fn start() -> Self {
        let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("n").expect("node"), 0));
        let clock = Arc::new(SimClock::new());
        let dial = Arc::new(SimGuestStreamDial::new());
        let runner = ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::clone(&dial) as Arc<dyn GuestStreamDial>,
        );
        let alloc = AllocationId::new("probed-0").expect("alloc");
        let _token = runner.start_alloc(&vm_spec(&alloc));
        settle().await;
        Self { runner, obs, clock, dial, alloc }
    }

    async fn run(&self, run: Run) {
        self.dial.script(SocketAddrV4::new(WORKLOAD_ADDR, LIVENESS_PORT), run.script());
        if run == Run::WriteFail {
            self.obs.inject_write_failure(ObservationStoreError::Io(std::io::Error::other(
                "injected probe-row write failure",
            )));
        }
        self.clock.tick(INTERVAL);
        settle().await;
    }

    async fn liveness_row(&self) -> Option<ProbeResultRow> {
        self.obs
            .list_probe_results_for_alloc(&self.alloc)
            .await
            .expect("list rows")
            .into_iter()
            .find(|row| row.role == ProbeRole::Liveness && row.probe_idx == ProbeIdx::new(0))
    }
}

fn counts(row: Option<&ProbeResultRow>) -> Option<(u64, bool, u32)> {
    row.map(|r| {
        (r.run_seq.get(), matches!(r.status, ProbeStatus::Fail { .. }), r.same_status_runs.get())
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// For every sequence of probe runs, after each run the stored row
    /// carries the count of written runs as its `run_seq` and the trailing
    /// count of written runs of its status as its `same_status_runs`; a run
    /// that writes no row changes nothing and emits exactly one
    /// `probe.not_sampled` with its reason; every run is one
    /// `DialPurpose::Probe` dial of the allocation's `workload_addr`.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER step 9"]
    fn every_written_run_counts_once_and_missing_samples_count_nothing(
        runs in proptest::collection::vec(arb_run(), 1..=8)
    ) {
        let capture = NotSampled::default();
        let _guard = tracing::subscriber::set_default(
            tracing_subscriber::registry().with(capture.clone()),
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let probed = Probed::start().await;
            for (i, run) in runs.iter().enumerate() {
                let before = probed.liveness_row().await;
                let events_before = capture.0.lock().expect("lock").len();
                probed.run(*run).await;
                let after = probed.liveness_row().await;

                prop_assert_eq!(
                    counts(after.as_ref()),
                    expected_counts(&runs[..=i]),
                    "stored row after runs {:?}", &runs[..=i]
                );
                let events = capture.0.lock().expect("lock").clone();
                match run.not_sampled_reason() {
                    Some(reason) => {
                        prop_assert_eq!(&after, &before, "a run that writes no row changes nothing");
                        prop_assert_eq!(events.len(), events_before + 1);
                        let event = &events[events_before];
                        prop_assert_eq!(event.get("reason").map(String::as_str), Some(reason));
                        prop_assert_eq!(event.get("role").map(String::as_str), Some("liveness"));
                        prop_assert_eq!(event.get("probe_idx").map(String::as_str), Some("0"));
                        prop_assert_eq!(event.get("alloc").map(String::as_str), Some("probed-0"));
                        prop_assert_eq!(
                            event.contains_key("detail"),
                            reason == "write_failed",
                            "detail only for adapter_error / write_failed"
                        );
                    }
                    None => prop_assert_eq!(events.len(), events_before, "a written run emits no not-sampled event"),
                }
            }
            let dials = probed.dial.dials();
            prop_assert_eq!(dials.len(), runs.len(), "one dial per run");
            prop_assert!(dials.iter().all(|d| d.purpose == DialPurpose::Probe
                && d.target == SocketAddrV4::new(WORKLOAD_ADDR, LIVENESS_PORT)));
            probed.runner.stop_alloc(&probed.alloc);
            Ok(())
        })?;
    }
}

/// A TCP-connect VM probe is healthy iff its dial returns `Ok`, and every
/// refusal other than `Capacity` / `Quiesced` is a failure whose detail is
/// the refusal's `as_str` (INB-1, R5-53 from the probe runner). The stream of
/// a healthy run is dropped at once with both directions open — the drop
/// aborts the flow (EVMOD-STREAM-ABORT, EVMOD-AB): no half-close is sent.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER attachment-cut step (9)"]
async fn tcp_vm_probe_is_healthy_iff_the_dial_succeeds() {
    let probed = Probed::start().await;
    let target = SocketAddrV4::new(WORKLOAD_ADDR, LIVENESS_PORT);
    let refusals = [
        GuestDialError::NotServing,
        GuestDialError::NoSession,
        GuestDialError::Refused { errno: KernelErrno::ECONNREFUSED },
        GuestDialError::TimedOut { errno: KernelErrno::ETIMEDOUT },
        GuestDialError::PairingDeadline,
        GuestDialError::Unreachable { errno: KernelErrno::ENODEV },
    ];
    for refusal in refusals {
        probed.dial.script(target, SimDialScript::Refuse(refusal));
        probed.clock.tick(INTERVAL);
        settle().await;
        assert_eq!(
            probed.liveness_row().await.map(|row| row.status),
            Some(ProbeStatus::Fail { last_fail_reason: refusal.as_str().to_owned() }),
            "{refusal:?} is an unhealthy run",
        );
    }
    probed.dial.script(target, SimDialScript::AcceptOpen { reply: Vec::new() });
    probed.clock.tick(INTERVAL);
    settle().await;
    assert_eq!(probed.liveness_row().await.map(|row| row.status), Some(ProbeStatus::Pass));
    let healthy = probed.dial.dials_of(target).pop().expect("the healthy run dialled");
    assert!(healthy.host_closed(), "the runner drops the stream when the probe ends");
    assert!(
        !healthy.host_half_closed(),
        "a TCP-connect probe aborts its flow: it never half-closes the stream"
    );
    assert!(healthy.written().is_empty(), "a TCP-connect probe writes no byte");
    probed.runner.stop_alloc(&probed.alloc);
}

fn http_liveness_descriptor(port: u16) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role: ProbeRole::Liveness,
        mechanic: ProbeMechanic::Http { path: "/healthz".to_owned(), port, host: None },
        timeout_seconds: 1,
        interval_seconds: 1,
        max_attempts: 30,
        failure_threshold: Some(3),
        success_threshold: None,
        inferred: false,
    }
}

/// A probe runner over Sim adapters with one VM allocation declaring
/// `descriptors`.
async fn start_with(
    descriptors: Vec<ProbeDescriptor>,
) -> (ProbeRunner, Arc<SimObservationStore>, Arc<SimClock>, Arc<SimGuestStreamDial>, AllocationId) {
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("n").expect("node"), 0));
    let clock = Arc::new(SimClock::new());
    let dial = Arc::new(SimGuestStreamDial::new());
    let runner = ProbeRunner::new(
        Arc::new(SimTcpProber::new()),
        Arc::new(SimHttpProber::new()),
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&obs) as Arc<dyn ObservationStore>,
        Arc::clone(&dial) as Arc<dyn GuestStreamDial>,
    );
    let alloc = AllocationId::new("probed-0").expect("alloc");
    let mut spec = vm_spec(&alloc);
    spec.probe_descriptors = descriptors;
    let _token = runner.start_alloc(&spec);
    settle().await;
    (runner, obs, clock, dial, alloc)
}

async fn liveness_row_of(
    obs: &SimObservationStore,
    alloc: &AllocationId,
) -> Option<ProbeResultRow> {
    obs.list_probe_results_for_alloc(alloc)
        .await
        .expect("list rows")
        .into_iter()
        .find(|row| row.role == ProbeRole::Liveness)
}

/// An HTTP VM probe runs its request over the `GuestStream` of a
/// `DialPurpose::Probe` dial — never through the `HttpProber` — and is
/// healthy iff a 2xx response arrives: `200` is `Pass`, `503` is `Fail`; a
/// dial refused `Capacity` writes no row (INB-1, PROBE-NOSAMPLE, R5-53).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER attachment-cut step (9)"]
async fn http_vm_probe_runs_its_request_over_the_guest_stream() {
    let port = 8090;
    let (runner, obs, clock, dial, alloc) = start_with(vec![http_liveness_descriptor(port)]).await;
    let target = SocketAddrV4::new(WORKLOAD_ADDR, port);

    dial.script(
        target,
        SimDialScript::Accept { reply: b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec() },
    );
    clock.tick(INTERVAL);
    settle().await;
    assert_eq!(
        liveness_row_of(&obs, &alloc).await.map(|row| row.status),
        Some(ProbeStatus::Pass),
        "200 is healthy"
    );
    let first = dial.dials_of(target);
    assert_eq!(first.len(), 1, "one dial per run");
    assert_eq!(first[0].purpose, DialPurpose::Probe);
    assert!(
        first[0].written().starts_with(b"GET /healthz "),
        "the request runs over the stream: {:?}",
        String::from_utf8_lossy(&first[0].written())
    );
    assert!(first[0].host_closed(), "the runner drops the stream when the probe ends");

    dial.script(
        target,
        SimDialScript::Accept {
            reply: b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n".to_vec(),
        },
    );
    clock.tick(INTERVAL);
    settle().await;
    assert!(
        matches!(
            liveness_row_of(&obs, &alloc).await.map(|row| row.status),
            Some(ProbeStatus::Fail { .. })
        ),
        "a non-2xx response is unhealthy"
    );

    let before = liveness_row_of(&obs, &alloc).await;
    dial.script(target, SimDialScript::Refuse(GuestDialError::Capacity));
    clock.tick(INTERVAL);
    settle().await;
    assert_eq!(
        liveness_row_of(&obs, &alloc).await,
        before,
        "a dial refused for node capacity is no sample"
    );
    runner.stop_alloc(&alloc);
}

/// Let `by` elapse on both clocks a probe run may observe: the injected
/// clock (ticked) and the runtime's (paused, auto-advanced).
async fn elapse(clock: &SimClock, by: Duration) {
    clock.tick(by);
    settle().await;
    tokio::time::sleep(by).await;
    settle().await;
}

/// The probe's existing timeout bounds the dial and the request together
/// (§ *VM probe runner*): a dial with no answer, and a paired request whose
/// application never replies (a hung application, at its own quota or not),
/// each fail the run at the timeout — a failed attempt, never a no sample —
/// and the runner drops the dial future (the owner ends it like a pairing
/// deadline) or the stream (R5-53, probes).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (9)"]
async fn a_probe_whose_dial_or_request_outlives_the_timeout_fails_and_drops_it() {
    let port = 8091;
    let (runner, obs, clock, dial, alloc) = start_with(vec![http_liveness_descriptor(port)]).await;
    let target = SocketAddrV4::new(WORKLOAD_ADDR, port);
    // The descriptor's timeout, read off the descriptor the runner was given.
    let timeout = Duration::from_secs(u64::from(http_liveness_descriptor(port).timeout_seconds));

    let past_the_timeout = timeout + Duration::from_millis(1);

    // A dial the owner never answers.
    dial.script(target, SimDialScript::Pending);
    clock.tick(INTERVAL);
    settle().await;
    assert_eq!(dial.pending_dials(target), 1, "the run's dial waits for the owner's answer");
    elapse(&clock, past_the_timeout).await;
    assert!(
        matches!(
            liveness_row_of(&obs, &alloc).await.map(|row| row.status),
            Some(ProbeStatus::Fail { .. })
        ),
        "a dial that outlives the timeout is a failed attempt"
    );
    let unanswered = dial.dials_of(target)[0].clone();
    assert!(unanswered.cancelled(), "the runner dropped the dial future at the timeout");
    let after_dial_timeout = liveness_row_of(&obs, &alloc).await.map(|row| row.run_seq);

    // A paired request the application never answers.
    dial.script(target, SimDialScript::AcceptOpen { reply: Vec::new() });
    clock.tick(INTERVAL);
    settle().await;
    elapse(&clock, past_the_timeout).await;
    let row = liveness_row_of(&obs, &alloc).await.expect("the request's run wrote a row");
    assert!(
        matches!(row.status, ProbeStatus::Fail { .. }),
        "a hung application fails at the timeout"
    );
    assert!(
        after_dial_timeout.is_some_and(|before| row.run_seq > before),
        "the hung request's run is a new written run"
    );
    let hung = dial
        .dials_of(target)
        .into_iter()
        .find(|d| d.answered == SimDialScript::AcceptOpen { reply: Vec::new() })
        .expect("the hung request's dial");
    assert!(hung.written().starts_with(b"GET /healthz "), "the request was written");
    assert!(hung.host_closed(), "the runner dropped the stream at the timeout");
    runner.stop_alloc(&alloc);
}

/// R5-60 (D-UP-12, sim half): with `GUEST_PROBE_SLOTS` (8) probes on one VM
/// allocation — 3 startup, 3 readiness, 2 liveness — every probe dials its
/// own target with `DialPurpose::Probe` in the same period and writes its
/// row. The parse-time refusal of a ninth probe is the deploy validation's.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER attachment-cut step (9)"]
async fn eight_probes_of_one_vm_allocation_all_dial_in_one_period() {
    let roles = [(ProbeRole::Startup, 3_u32), (ProbeRole::Readiness, 3), (ProbeRole::Liveness, 2)];
    let mut descriptors = Vec::new();
    let mut port = 9000_u16;
    for (role, count) in roles {
        for idx in 0..count {
            descriptors.push(ProbeDescriptor {
                idx: ProbeIdx::new(idx),
                role,
                mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port },
                timeout_seconds: 1,
                interval_seconds: 1,
                max_attempts: 30,
                failure_threshold: Some(3),
                success_threshold: Some(1),
                inferred: false,
            });
            port += 1;
        }
    }
    assert_eq!(descriptors.len(), 8, "precondition: the cap's worth of probes");
    let (runner, obs, clock, dial, alloc) = start_with(descriptors).await;
    dial.script_default(SimDialScript::accept());
    clock.tick(INTERVAL);
    settle().await;

    let dials = dial.dials();
    let targets: std::collections::BTreeSet<SocketAddrV4> =
        dials.iter().map(|d| d.target).collect();
    let expected: std::collections::BTreeSet<SocketAddrV4> =
        (9000_u16..9008).map(|port| SocketAddrV4::new(WORKLOAD_ADDR, port)).collect();
    assert_eq!(targets, expected, "every one of the eight probes dialled its own target");
    assert!(dials.iter().all(|d| d.purpose == DialPurpose::Probe), "probe dials are quota-exempt");
    let rows = obs.list_probe_results_for_alloc(&alloc).await.expect("list rows");
    assert_eq!(rows.len(), 8, "every probe wrote its row: {rows:?}");
    assert!(rows.iter().all(|row| row.status == ProbeStatus::Pass));
    runner.stop_alloc(&alloc);
}

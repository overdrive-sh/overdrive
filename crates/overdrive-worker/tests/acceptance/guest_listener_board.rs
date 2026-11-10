//! D13 item 10 (PR-1) — the host-side board of guest-reported listeners and
//! the beacon reader that feeds it. Obligations S-50, S-51 and S-52 of
//! `docs/feature/service-allocation-readiness-and-ending/feature-delta.md`.
//!
//! Driving ports: `VmDriver::start`, `VmDriver::on_alloc_terminal` (through the
//! `Driver` trait) and the real beacon reader, driven by a stepped
//! `SimBeaconIo` (`queue_connection`, `guest_write`, `release_read`). The board
//! is never written directly: every insertion is a scripted guest line the
//! reader parses and routes. Observation is only through the read-only port
//! `GuestListenerReports` (`reported`, `changed`) and, for the exit report, the
//! driver's exit receiver.
//!
//! `@contract-shape:bounded-change` — insert-only while an entry exists; the
//! universe is the declared listeners of one allocation.
//!
//! Every test below depends on the board, the `LISTEN` wire form and the
//! reader's classification, none of which exist yet (RED scaffolds).

#![allow(
    clippy::missing_const_for_fn,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    reason = "test fixtures assert their preconditions; scripted schedules index small fixed tables"
)]

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::SpiffeId;
use overdrive_core::aggregate::Listener;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GuestCid, GuestFlowPorts, GuestTransportAssignment, LaunchToken,
    VsockListenPort,
};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, Driver, DriverPayload, ExitEvent, ExitKind, Resources,
    VmPayload,
};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::{GuestListenerReports, ReportedListeners};
use overdrive_core::vm::beacon::{BeaconConnKey, BeaconIo};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, VmConfinement, VmmIdentity, VsockPort,
};
use overdrive_sim::SimVmm;
use overdrive_sim::adapters::beacon_io::SimBeaconIo;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::guest_listener_reports::SimGuestListenerReports;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs};
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::vm_driver::VmHostLayout;
use overdrive_worker::{BeaconIntake, GuestListenerBoard, VmDriver};
use proptest::prelude::*;
use tempfile::TempDir;
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;

const FLOW_STREAM: u32 = 50_240;
const FLOW_CONTROL: u32 = 50_243;
const GUEST_SOURCE_PORT: u32 = 49_152;

/// The two declared listeners of every fixture Service, in declaration order.
fn tcp_8080() -> (Proto, NonZeroU16) {
    (Proto::Tcp, NonZeroU16::new(8080).expect("non-zero"))
}
fn udp_5353() -> (Proto, NonZeroU16) {
    (Proto::Udp, NonZeroU16::new(5353).expect("non-zero"))
}
/// A pair the Service does not declare.
fn tcp_9999() -> (Proto, NonZeroU16) {
    (Proto::Tcp, NonZeroU16::new(9999).expect("non-zero"))
}

fn declared() -> Vec<Listener> {
    [tcp_8080(), udp_5353()]
        .into_iter()
        .map(|(protocol, port)| Listener { port, protocol })
        .collect()
}

fn declared_set() -> BTreeSet<(Proto, NonZeroU16)> {
    [tcp_8080(), udp_5353()].into_iter().collect()
}

fn listen_line((protocol, port): (Proto, NonZeroU16)) -> String {
    let proto = match protocol {
        Proto::Tcp => "tcp",
        Proto::Udp => "udp",
    };
    format!("LISTEN {proto} {port}")
}

fn pairs_of(set: &ReportedListeners) -> BTreeSet<(Proto, NonZeroU16)> {
    [tcp_8080(), udp_5353(), tcp_9999()]
        .into_iter()
        .filter(|(protocol, port)| set.contains(*protocol, *port))
        .collect()
}

/// One launched allocation: its id, the VMM pid and the guest's beacon key.
struct Launched {
    alloc: AllocationId,
    pid: u32,
    key: BeaconConnKey,
}

/// A worker node: one `VmDriver` over a stepped beacon, one shared board.
struct Harness {
    _tmp: TempDir,
    root: std::path::PathBuf,
    vmm: SimVmm,
    io: Arc<SimBeaconIo>,
    driver: VmDriver,
    board: Arc<GuestListenerBoard>,
    exits: Receiver<ExitEvent>,
    next: u32,
}

impl Harness {
    async fn new() -> Self {
        let tmp = TempDir::new().expect("tmpdir");
        let root = tmp.path().to_path_buf();
        std::fs::write(root.join("master.img"), b"deterministic-fixture-rootfs-bytes")
            .expect("write synthetic master rootfs");
        let mut header = vec![0_u8; KERNEL_MAGIC_WINDOW];
        header[..4].copy_from_slice(b"\x7fELF");
        std::fs::write(root.join("vmlinuz"), &header).expect("stage the synthetic kernel image");
        let vmm = SimVmm::new();
        let io = Arc::new(SimBeaconIo::stepped(vmm.clone()));
        let intake = BeaconIntake::bind(
            Arc::clone(&io) as Arc<dyn BeaconIo>,
            VsockListenPort::KernelAssigned,
        )
        .await
        .expect("the sim beacon listener binds");
        let clock = SimClock::new();
        let wiring =
            overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(clock.clone()));
        assert!(wiring.supervisor().open_after_boot(), "fixture EXEC gate opens");
        let board = Arc::new(GuestListenerBoard::new());
        let probe_runner = Arc::new(ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::new(SimClock::new()),
            Arc::new(SimObservationStore::single_peer(
                NodeId::new("guest-listener-board").expect("valid node ID"),
                0,
            )) as Arc<dyn ObservationStore>,
            Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
            Arc::clone(&board) as Arc<dyn GuestListenerReports>,
        ));
        let driver = VmDriver::new(
            Arc::new(vmm.clone()),
            Arc::new(clock),
            Arc::new(SimCgroupFs::new()),
            Arc::new(SimCgroupAccounting::new()),
            probe_runner,
            wiring.gate(),
            VmHostLayout {
                cgroup_root: root.join("cgroup"),
                run_dir_root: root.join("run"),
                clone_index_dir: root.join("clone-index"),
                clone_staging_dir: root.join("clone-staging"),
                arch: HostArch::X86_64,
                confinement: VmConfinement::confined(
                    VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
                    1024,
                ),
            },
            intake,
            Arc::clone(&board),
        );
        let exits = driver.take_exit_receiver().expect("the exit receiver is taken once");
        Self { _tmp: tmp, root, vmm, io, driver, board, exits, next: 0 }
    }

    fn cid(index: u32) -> GuestCid {
        GuestCid::new(GUEST_CID_BASE + 2 + index).expect("an in-range guest CID")
    }

    fn spec(&self, alloc: &AllocationId, index: u32, launch: LaunchToken) -> AllocationSpec {
        AllocationSpec {
            declared_listeners: declared(),
            alloc: alloc.clone(),
            identity: SpiffeId::new("spiffe://overdrive.local/workload/board/alloc/x")
                .expect("valid spiffe id"),
            driver: DriverPayload::Vm(VmPayload {
                command: "/sbin/init".to_owned(),
                args: vec![],
                kernel: self.root.join("vmlinuz"),
                rootfs: self.root.join("master.img"),
            }),
            resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
            probe_descriptors: Vec::new(),
            transport: Some(GuestTransportAssignment::new(
                Ipv4Addr::new(100, 95, 0, 2 + index as u8),
                Ipv4Addr::new(100, 95, 0, 1),
                Self::cid(index),
                launch,
                GuestFlowPorts::new(VsockPort::new(FLOW_STREAM), VsockPort::new(FLOW_CONTROL))
                    .expect("two distinct ports above the reserved range"),
            )),
        }
    }

    /// Lets every task run until every beacon call is parked.
    async fn settle(&self) {
        for _ in 0..1_000 {
            tokio::time::sleep(Duration::from_millis(1)).await;
            if self.io.parked() {
                break;
            }
        }
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    /// Starts the next allocation through `VmDriver::start` and completes its
    /// READY, so the beacon reader is running for it.
    async fn launch(&mut self) -> Launched {
        let index = self.next;
        self.next += 1;
        let alloc = AllocationId::new(&format!("board-alloc-{index}")).expect("valid alloc id");
        let token = LaunchToken::from_bytes([0x5a + index as u8; 16]).expect("a non-zero token");
        let spec = self.spec(&alloc, index, token);
        let driver = self.driver.clone();
        let start = tokio::spawn(async move { driver.start(&spec).await });
        let cid = Self::cid(index);
        let pid = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(pid) = self.vmm.vsock_device_holders().get(&cid) {
                    break *pid;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the VMM opens its device within 10s");
        let key =
            BeaconConnKey { peer_cid: cid, peer_port: VsockPort::new(GUEST_SOURCE_PORT + index) };
        let ready = format!("READY pid=1 port=1234 launch={token}");
        self.io.queue_connection(key, pid, &[ready.as_str()]).expect("queued");
        self.settle().await;
        self.io.release_accept();
        self.settle().await;
        assert!(self.io.release_read(key), "READY is readable");
        self.settle().await;
        tokio::time::timeout(Duration::from_secs(10), start)
            .await
            .expect("the start completes")
            .expect("the start task did not panic")
            .expect("READY completes the start");
        Launched { alloc, pid, key }
    }

    /// The guest writes one line and the reader consumes it.
    async fn send(&self, launched: &Launched, line: &str) {
        let _ = self.io.guest_write(launched.key, line);
        let _ = self.io.release_read(launched.key);
        self.settle().await;
    }

    fn reported(&self, launched: &Launched) -> ReportedListeners {
        self.board.reported(&launched.alloc)
    }

    /// The allocation's terminal hook, as the shim fires it after the exit.
    fn terminal(&self, launched: &Launched) {
        self.driver.on_alloc_terminal(&launched.alloc);
    }

    /// Ends the VMM and returns the exit report the driver emits.
    async fn exit_report(&mut self, launched: &Launched) -> ExitEvent {
        assert!(self.vmm.exit_process(launched.pid, Some(0), None), "the VMM was running");
        self.io.observe_vm_exits();
        let handle = AllocationHandle { alloc: launched.alloc.clone(), pid: Some(launched.pid) };
        self.driver.release_for_exit_emission(&handle).await;
        tokio::time::timeout(Duration::from_secs(10), self.exits.recv())
            .await
            .expect("the exit event arrives within 10s")
            .expect("the exit channel is open")
    }
}

/// A captured `tracing` output, to see the reader's structured warn event.
#[derive(Clone, Default)]
struct Captured(Arc<parking_lot::Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock()).into_owned()
    }
}

// ---------------------------------------------------------------------------
// S-50 — the board's set
// ---------------------------------------------------------------------------

/// One scripted step of the S-50 sequence.
#[derive(Debug, Clone)]
enum Step {
    Declared(usize),
    Undeclared,
    Repeat(usize), // a repeat attempt; it inserts if the pair has not arrived yet
    OtherAllocation(usize),
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        (0..2_usize).prop_map(Step::Declared),
        Just(Step::Undeclared),
        (0..2_usize).prop_map(Step::Repeat),
        (0..2_usize).prop_map(Step::OtherAllocation),
    ]
}

fn declared_pair(i: usize) -> (Proto, NonZeroU16) {
    [tcp_8080(), udp_5353()][i]
}

/// S-50 (PR-1, D13 item 10): the board's set. `reported` is empty before
/// `start` and after `on_alloc_terminal`; during the entry's life the set only
/// grows and stays within the declared set; a duplicate changes nothing; a line
/// of another allocation does not reach this one; lines after close are no-ops.
/// Driven only through the beacon reader and `VmDriver::start` / terminal hook.
#[test]
/// CONTRACT_SHAPE: bounded-change.
fn the_reported_set_only_grows_within_the_declared_set_and_ends_with_the_entry() {
    let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig {
        cases: 12,
        failure_persistence: None,
        ..ProptestConfig::default()
    });
    runner
        .run(
            &(
                prop::collection::vec(step_strategy(), 0..10),
                prop::collection::vec(step_strategy(), 0..4),
            ),
            |(steps, after_close)| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime");
                rt.block_on(async move {
                    let mut harness = Harness::new().await;
                    let ghost = AllocationId::new("never-started").expect("valid alloc id");
                    prop_assert!(
                        harness.board.reported(&ghost).is_empty(),
                        "an id with no entry reads as the empty set"
                    );
                    let first = harness.launch().await;
                    let other = harness.launch().await;
                    prop_assert!(harness.reported(&first).is_empty(), "an open entry starts empty");
                    harness.send(&first, &listen_line(tcp_9999())).await;
                    prop_assert!(
                        harness.reported(&first).is_empty(),
                        "an undeclared first line leaves the entry empty"
                    );

                    let mut expected: BTreeSet<(Proto, NonZeroU16)> = BTreeSet::new();
                    let mut other_expected: BTreeSet<(Proto, NonZeroU16)> = BTreeSet::new();
                    let first_pair = declared_pair(0);
                    harness.send(&first, &listen_line(first_pair)).await;
                    expected.insert(first_pair);
                    let mut previous = harness.reported(&first);
                    prop_assert_eq!(pairs_of(&previous), expected.clone(), "first report inserts");
                    harness.send(&first, &listen_line(first_pair)).await;
                    prop_assert_eq!(
                        harness.reported(&first),
                        previous.clone(),
                        "a repeated report of an already recorded pair is a no-op"
                    );
                    for step in &steps {
                        let actual_duplicate = matches!(
                            step,
                            Step::Repeat(i) if expected.contains(&declared_pair(*i))
                        );
                        match step {
                            Step::Declared(i) | Step::Repeat(i) => {
                                let pair = declared_pair(*i);
                                harness.send(&first, &listen_line(pair)).await;
                                expected.insert(pair);
                            }
                            Step::Undeclared => {
                                harness.send(&first, &listen_line(tcp_9999())).await;
                            }
                            Step::OtherAllocation(i) => {
                                let pair = declared_pair(*i);
                                harness.send(&other, &listen_line(pair)).await;
                                other_expected.insert(pair);
                            }
                        }
                        let now = harness.reported(&first);
                        prop_assert_eq!(
                            pairs_of(&now),
                            expected.clone(),
                            "the set is the declared pairs reported so far"
                        );
                        prop_assert!(
                            pairs_of(&now).is_subset(&declared_set())
                                && now.len() == expected.len(),
                            "the set holds only declared pairs"
                        );
                        prop_assert!(
                            pairs_of(&previous).is_subset(&pairs_of(&now)),
                            "the set only grows while the entry exists"
                        );
                        if actual_duplicate {
                            prop_assert_eq!(&now, &previous, "a duplicate changes nothing");
                        }
                        prop_assert_eq!(
                            pairs_of(&harness.reported(&other)),
                            other_expected.clone(),
                            "another allocation's lines reach only its own set"
                        );
                        previous = now;
                    }

                    harness.terminal(&first);
                    prop_assert!(
                        harness.reported(&first).is_empty(),
                        "the entry ends at the terminal hook"
                    );
                    for step in &after_close {
                        let pair = match step {
                            Step::Declared(i) | Step::Repeat(i) | Step::OtherAllocation(i) => {
                                declared_pair(*i)
                            }
                            Step::Undeclared => tcp_9999(),
                        };
                        harness.send(&first, &listen_line(pair)).await;
                        prop_assert!(
                            harness.reported(&first).is_empty(),
                            "a line after close records nothing"
                        );
                    }
                    prop_assert_eq!(
                        pairs_of(&harness.reported(&other)),
                        other_expected,
                        "closing one allocation leaves another's set alone"
                    );
                    Ok(())
                })
            },
        )
        .expect("the board's set contract holds");
}

// ---------------------------------------------------------------------------
// S-51 — no completion on a no-op; the host reader
// ---------------------------------------------------------------------------

/// A `changed` waiter running as a task so the test can see whether it has
/// completed, without polling it itself.
fn waiter(
    reports: &Arc<dyn GuestListenerReports>,
    alloc: &AllocationId,
    seen: &ReportedListeners,
) -> JoinHandle<ReportedListeners> {
    let reports = Arc::clone(reports);
    let alloc = alloc.clone();
    let seen = seen.clone();
    tokio::spawn(async move { reports.changed(&alloc, &seen).await })
}

/// S-51 (PR-1): `changed` completes on none of: a duplicate, an undeclared
/// pair, another allocation's report, a no-entry id, a record after close; the
/// waiter pending at close is not woken; and it completes on a real insertion.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
async fn a_pending_changed_completes_on_an_insertion_and_on_no_no_op() {
    let mut harness = Harness::new().await;
    let reports = Arc::clone(&harness.board) as Arc<dyn GuestListenerReports>;
    let target = harness.launch().await;
    let other = harness.launch().await;
    let ghost = AllocationId::new("never-started").expect("valid alloc id");

    // A first declared insertion, so the duplicate case has something to repeat.
    harness.send(&target, &listen_line(tcp_8080())).await;
    let seen = harness.reported(&target);
    assert!(seen.contains(tcp_8080().0, tcp_8080().1), "precondition: the first report landed");

    let pending = waiter(&reports, &target.alloc, &seen);
    let ghost_waiter = waiter(&reports, &ghost, &ReportedListeners::default());
    harness.settle().await;
    assert!(!pending.is_finished(), "nothing differs from `seen` yet");

    harness.send(&target, &listen_line(tcp_8080())).await;
    assert!(!pending.is_finished(), "a duplicate does not complete `changed`");
    harness.send(&target, &listen_line(tcp_9999())).await;
    assert!(!pending.is_finished(), "an undeclared pair does not complete `changed`");
    harness.send(&other, &listen_line(udp_5353())).await;
    assert!(!pending.is_finished(), "another allocation's report does not complete `changed`");
    assert!(!ghost_waiter.is_finished(), "an id with no entry never completes");

    harness.send(&target, &listen_line(udp_5353())).await;
    let completed = tokio::time::timeout(Duration::from_secs(10), pending)
        .await
        .expect("the insertion completes `changed`")
        .expect("the waiter did not panic");
    assert_eq!(
        pairs_of(&completed),
        BTreeSet::from([tcp_8080(), udp_5353()]),
        "`changed` completes with the current set"
    );
    assert_ne!(completed, seen);

    // A waiter pending at close is not woken, and a record after close is a no-op.
    let seen_now = harness.reported(&target);
    let at_close = waiter(&reports, &target.alloc, &seen_now);
    harness.settle().await;
    harness.terminal(&target);
    harness.settle().await;
    assert!(!at_close.is_finished(), "a waiter pending at close is not woken");
    harness.send(&target, &listen_line(tcp_8080())).await;
    assert!(!at_close.is_finished(), "a record after close completes nothing");
    assert!(harness.reported(&target).is_empty(), "a record after close stores nothing");
    at_close.abort();
    ghost_waiter.abort();
}

/// S-51 (reader): a garbage line after READY is dropped with a structured warn
/// event naming the allocation, and the following `Listen` and `Exit` still
/// take effect.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
async fn a_garbage_line_is_dropped_with_a_warning_and_later_lines_take_effect() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let mut harness = Harness::new().await;
    let launched = harness.launch().await;
    harness.send(&launched, "!! this is not a beacon line").await;
    assert!(harness.reported(&launched).is_empty(), "a dropped line stores nothing");
    let text = captured.text();
    assert!(text.contains("WARN"), "the drop is a warn event, got: {text:?}");
    assert!(
        text.contains(launched.alloc.as_str()),
        "the warn event names the allocation, got: {text:?}"
    );

    harness.send(&launched, &listen_line(tcp_8080())).await;
    assert!(
        harness.reported(&launched).contains(tcp_8080().0, tcp_8080().1),
        "the Listen after the garbage line still takes effect"
    );
    harness.send(&launched, "EXIT 0").await;
    let event = harness.exit_report(&launched).await;
    assert_eq!(
        event.kind,
        ExitKind::CleanExit,
        "the Exit after the garbage line still took effect"
    );
}

/// S-51 (reader): a `Listen` line after `Exit` changes neither the set nor the
/// exit report, and a `Listen` never occupies the exit report: the report is the
/// `Exit` status, not the later line.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
async fn a_listen_after_exit_changes_neither_the_set_nor_the_exit_report() {
    let mut harness = Harness::new().await;
    let launched = harness.launch().await;
    harness.send(&launched, &listen_line(tcp_8080())).await;
    let before = harness.reported(&launched);
    harness.send(&launched, "EXIT 7").await;
    harness.send(&launched, &listen_line(udp_5353())).await;
    assert_eq!(harness.reported(&launched), before, "a Listen after Exit leaves the set alone");

    let event = harness.exit_report(&launched).await;
    assert_eq!(
        event.kind,
        ExitKind::Crashed { exit_code: Some(7), signal: None },
        "the exit report is the guest's Exit status, never the Listen line"
    );
}

/// S-51 (equivalence, design rule: a trait's contract is enforced by an
/// equivalence test): the board and `SimGuestListenerReports` run through one
/// call sequence over the port and agree on `reported` and on whether a pending
/// `changed` has completed, at every step. The sim's scripting layer models the
/// board's declared filter (it never inserts an undeclared pair), so the script
/// feeds the sim the same observable stimulus the reader gives the board.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
async fn guest_listener_reports_equivalence() {
    let mut harness = Harness::new().await;
    let launched = harness.launch().await;
    let board = Arc::clone(&harness.board) as Arc<dyn GuestListenerReports>;
    let sim = Arc::new(SimGuestListenerReports::new());
    sim.open(&launched.alloc);
    let sim_port = Arc::clone(&sim) as Arc<dyn GuestListenerReports>;

    // (guest line for the board, scripted insertion for the sim, or None for a
    // no-op the board's filter absorbs)
    let script: Vec<(String, Option<(Proto, NonZeroU16)>)> = vec![
        (listen_line(tcp_8080()), Some(tcp_8080())),
        (listen_line(tcp_8080()), Some(tcp_8080())),
        (listen_line(tcp_9999()), None),
        ("!! garbage".to_owned(), None),
        (listen_line(udp_5353()), Some(udp_5353())),
    ];
    let mut board_seen = board.reported(&launched.alloc);
    let mut sim_seen = sim_port.reported(&launched.alloc);
    assert_eq!(board_seen, sim_seen, "both start empty");
    for (line, insertion) in script {
        let board_waiter = waiter(&board, &launched.alloc, &board_seen);
        let sim_waiter = waiter(&sim_port, &launched.alloc, &sim_seen);
        harness.send(&launched, &line).await;
        if let Some((protocol, port)) = insertion {
            sim.insert(&launched.alloc, protocol, port);
        }
        harness.settle().await;
        assert_eq!(
            board.reported(&launched.alloc),
            sim_port.reported(&launched.alloc),
            "`reported` agrees after {line:?}"
        );
        assert_eq!(
            board_waiter.is_finished(),
            sim_waiter.is_finished(),
            "`changed` completion agrees after {line:?}"
        );
        board_seen = board.reported(&launched.alloc);
        sim_seen = sim_port.reported(&launched.alloc);
        board_waiter.abort();
        sim_waiter.abort();
    }

    harness.terminal(&launched);
    sim.close(&launched.alloc);
    assert_eq!(
        board.reported(&launched.alloc),
        sim_port.reported(&launched.alloc),
        "both read empty after close"
    );
    let board_closed = waiter(&board, &launched.alloc, &board.reported(&launched.alloc));
    let sim_closed = waiter(&sim_port, &launched.alloc, &sim_port.reported(&launched.alloc));
    harness.settle().await;
    assert_eq!(board_closed.is_finished(), sim_closed.is_finished(), "a closed id never completes");
    assert!(!board_closed.is_finished());
    board_closed.abort();
    sim_closed.abort();
}

// ---------------------------------------------------------------------------
// S-52 — `changed` never misses an insertion
// ---------------------------------------------------------------------------

/// One scripted operation of an S-52 schedule.
#[derive(Debug, Clone, Copy)]
enum Op {
    Reported,
    Changed,
    Record(usize),
    CancelOldest,
}

/// A tiny deterministic generator, so a failing schedule is reproduced from
/// its printed seed alone.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn schedule(seed: u64) -> Vec<Op> {
    let mut rng = Seeded(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    (0..14)
        .map(|_| match rng.next() % 5 {
            0 => Op::Reported,
            1 | 2 => Op::Changed,
            3 => Op::Record((rng.next() % 3) as usize),
            _ => Op::CancelOldest,
        })
        .collect()
}

/// S-52 `GuestListenerChangedNeverMissesAnInsertion` (PR-1): under seeded
/// schedules of `reported`, `changed`, a recorded insertion and a cancelled
/// waiter, a waiter is complete exactly when the set differs from the `seen` it
/// was given (so a `changed` called when the set already differs completes at
/// once, and an insertion between a `reported` and a `changed` is never lost);
/// it completes with a set that differs from `seen`; a cancelled waiter loses
/// nothing (the next `changed(seen)` completes at once). Completion needs no
/// clock: the test never advances time. The seed of each schedule is in the
/// failure message.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
async fn changed_never_misses_an_insertion_under_seeded_schedules() {
    for seed in 1..=6_u64 {
        let ops = schedule(seed);
        let mut harness = Harness::new().await;
        let launched = harness.launch().await;
        let reports = Arc::clone(&harness.board) as Arc<dyn GuestListenerReports>;
        let mut last_seen = reports.reported(&launched.alloc);
        let mut waiters: Vec<(ReportedListeners, JoinHandle<ReportedListeners>)> = Vec::new();
        for op in ops {
            match op {
                Op::Reported => last_seen = reports.reported(&launched.alloc),
                Op::Changed => {
                    waiters
                        .push((last_seen.clone(), waiter(&reports, &launched.alloc, &last_seen)));
                }
                Op::Record(i) => {
                    let pair = [tcp_8080(), udp_5353(), tcp_9999()][i];
                    harness.send(&launched, &listen_line(pair)).await;
                }
                Op::CancelOldest => {
                    if !waiters.is_empty() {
                        let (_, handle) = waiters.remove(0);
                        handle.abort();
                    }
                }
            }
            harness.settle().await;
            let current = reports.reported(&launched.alloc);
            for (seen, handle) in &waiters {
                assert_eq!(
                    handle.is_finished(),
                    &current != seen,
                    "seed={seed}: a waiter is complete exactly when the set differs from its `seen`"
                );
            }
        }
        // Cancel-safety: a dropped waiter then an insertion; the next
        // `changed(seen)` completes at once with the new set.
        let seen = reports.reported(&launched.alloc);
        let dropped = waiter(&reports, &launched.alloc, &seen);
        dropped.abort();
        let missing = [tcp_8080(), udp_5353()].into_iter().find(|(p, n)| !seen.contains(*p, *n));
        if let Some(pair) = missing {
            harness.send(&launched, &listen_line(pair)).await;
            let next = tokio::time::timeout(
                Duration::from_secs(10),
                reports.changed(&launched.alloc, &seen),
            )
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "seed={seed}: `changed(seen)` after a cancelled waiter must complete at once"
                )
            });
            assert!(
                next.contains(pair.0, pair.1),
                "seed={seed}: the insertion is in the completion"
            );
        }
        let empty = ReportedListeners::default();
        let immediate =
            tokio::time::timeout(Duration::ZERO, reports.changed(&launched.alloc, &empty))
                .await
                .ok();
        assert_eq!(
            immediate.is_some(),
            !reports.reported(&launched.alloc).is_empty(),
            "seed={seed}: a `changed` whose `seen` already differs completes at once"
        );
        for (_, handle) in waiters {
            handle.abort();
        }
    }
}

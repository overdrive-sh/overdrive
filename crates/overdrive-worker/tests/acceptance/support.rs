//! Shared `VmDriver` acceptance fixtures for the vsock attachment cut (GH #295,
//! CID-NOCLAIM / D17).
//!
//! The per-VM Unix beacon socket is gone: a guest's beacon connection now
//! arrives through the node beacon intake over the core `BeaconIo` port. These
//! fixtures put a `SimBeaconIo` (coupled to the test's `SimVmm`) behind a
//! test-local [`GuestWire`] decorator, so an acceptance test can play the guest
//! exactly as it used to over the Unix stream:
//!
//! - **dial** once the driver's VMM exists ([`BeaconHarness::dial_ready`]
//!   waits for the next `Vmm::create` through [`BeaconHarness::vmm`]'s
//!   decorator, then queues the connection keyed by the spec's leased CID with
//!   the READY line buffered);
//! - **read** the host → guest lines the driver wrote ([`GuestSession::next_line`],
//!   [`GuestSession::session_to_end`]);
//! - **write** guest → host lines ([`GuestSession::write`]) and close the
//!   guest's send direction ([`GuestSession::close_send`], the old `SHUT_WR`);
//! - **model the guest's receive side**: [`GuestSession::backpressure`] parks
//!   every host write (the old shrunk `SO_RCVBUF` against a 16 MiB EXEC), and
//!   [`GuestSession::refuse_host_writes`] fails them `BrokenPipe` (the old
//!   `SHUT_RD`).
//!
//! The decorator is test support over the driven port; it adds no production
//! surface. Its write log records only lines the inner `SimBeaconIo` accepted,
//! so it is the complete host → guest byte stream, and it survives the
//! driver's `close` (the sim forgets a closed connection's outbound lines).

#![allow(
    clippy::missing_const_for_fn,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    reason = "test fixtures assert their preconditions"
)]

use std::collections::BTreeMap;
use std::io;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GuestCid, GuestFlowPorts, GuestTransportAssignment, LaunchToken,
    VsockListenPort,
};
use overdrive_core::traits::driver::AllocationSpec;
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmProbeError,
};
use overdrive_core::vm::beacon::{BeaconConnKey, BeaconIo, BeaconSocket};
use overdrive_core::vm::config::{VmConfig, VsockPort};
use overdrive_sim::SimVmm;
use overdrive_sim::adapters::beacon_io::SimBeaconIo;
use overdrive_worker::BeaconIntake;
use parking_lot::Mutex;
use tokio::sync::{Notify, mpsc};

/// The launch token every fixture transport carries.
pub const TEST_LAUNCH_BYTES: [u8; 16] = [0x5a; 16];

/// The guest-side source vsock port of every fixture beacon connection.
pub const GUEST_BEACON_SOURCE_PORT: u32 = 49_152;

/// How long a fixture waits for the driver to reach a guest-visible point.
const GUEST_WAIT: Duration = Duration::from_secs(10);

/// The READY line a guest of a fixture transport writes: `launch=` carries
/// [`TEST_LAUNCH_BYTES`] as 32 lowercase hex characters.
pub fn ready_line() -> String {
    format!("READY pid=1 port=1234 launch={}", "5a".repeat(16))
}

/// The fixture launch token.
pub fn test_launch() -> LaunchToken {
    LaunchToken::from_bytes(TEST_LAUNCH_BYTES).expect("fixture launch token is non-zero")
}

/// The leased CID `GUEST_CID_BASE + offset` (CID-NOCLAIM: the VMM opens its
/// device itself; no claim is taken). Use a distinct offset per allocation
/// within one test: a `SimVmm` refuses a launch on a CID another live process
/// holds, as the kernel does.
pub fn leased_cid(offset: u32) -> GuestCid {
    GuestCid::new(GUEST_CID_BASE + offset).expect("fixture CID is in the guest range")
}

/// A transport handoff on the leased CID `GUEST_CID_BASE + cid_offset` with
/// the fixture launch token.
pub fn transport(
    workload_addr: Ipv4Addr,
    gateway: Ipv4Addr,
    cid_offset: u32,
) -> GuestTransportAssignment {
    GuestTransportAssignment::new(
        workload_addr,
        gateway,
        leased_cid(cid_offset),
        test_launch(),
        fixture_flow_ports(),
    )
}

/// The guest-flow owner's stream port every fixture transport carries
/// (GUEST-PORTS, ADR-0217). Deliberately not 1240: the driver must write the
/// assignment's port, never a constant.
pub const FIXTURE_FLOW_STREAM_PORT: u32 = 50_240;
/// The guest-flow owner's control port every fixture transport carries.
pub const FIXTURE_FLOW_CONTROL_PORT: u32 = 50_243;

/// The bound owner ports a fixture assignment hands the driver
/// (`GuestTransportAssignment::flow_ports`).
pub fn fixture_flow_ports() -> GuestFlowPorts {
    GuestFlowPorts::new(
        VsockPort::new(FIXTURE_FLOW_STREAM_PORT),
        VsockPort::new(FIXTURE_FLOW_CONTROL_PORT),
    )
    .expect("two distinct ports above the reserved range")
}

/// The kernel-reported key of the fixture guest's beacon connection for
/// `spec`: its leased CID and [`GUEST_BEACON_SOURCE_PORT`].
pub fn beacon_key(spec: &AllocationSpec) -> BeaconConnKey {
    let transport = spec.transport.as_ref().expect("a VM fixture spec carries a transport");
    BeaconConnKey { peer_cid: transport.cid(), peer_port: VsockPort::new(GUEST_BEACON_SOURCE_PORT) }
}

// ---------------------------------------------------------------------------
// GuestWire — test-local BeaconIo decorator over SimBeaconIo
// ---------------------------------------------------------------------------

/// How the guest's receive side treats host writes on one connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReceiveSide {
    /// The guest drains: writes complete.
    Drains,
    /// The guest's receive buffer is full: writes never complete.
    Backpressured,
    /// The guest shut its receive direction: writes fail `BrokenPipe`.
    Refused,
}

#[derive(Default)]
struct WireState {
    /// Accepted handle id → kernel key.
    keys: BTreeMap<u64, BeaconConnKey>,
    /// Every line a host write delivered, per key, surviving `close`.
    log: BTreeMap<BeaconConnKey, Vec<String>>,
    /// Per-key receive-side behaviour; absent = `Drains`.
    receive: BTreeMap<BeaconConnKey, ReceiveSide>,
}

/// Test-local [`BeaconIo`]: delegates to a `SimBeaconIo` and records the
/// complete host → guest line stream per connection.
pub struct GuestWire {
    inner: Arc<SimBeaconIo>,
    state: Mutex<WireState>,
    changed: Notify,
}

impl GuestWire {
    fn key_of(&self, socket: &BeaconSocket) -> Option<BeaconConnKey> {
        self.state.lock().keys.get(&socket.raw()).copied()
    }

    fn receive_side(&self, key: BeaconConnKey) -> ReceiveSide {
        self.state.lock().receive.get(&key).copied().unwrap_or(ReceiveSide::Drains)
    }
}

#[async_trait]
impl BeaconIo for GuestWire {
    async fn bind(&self, port: VsockListenPort) -> Result<VsockPort, io::Error> {
        self.inner.bind(port).await
    }

    async fn accept(&self) -> Result<(BeaconSocket, BeaconConnKey), io::Error> {
        let (socket, key) = self.inner.accept().await?;
        self.state.lock().keys.insert(socket.raw(), key);
        Ok((socket, key))
    }

    async fn read_line(&self, socket: &BeaconSocket) -> Result<Option<String>, io::Error> {
        self.inner.read_line(socket).await
    }

    async fn write_line(&self, socket: &BeaconSocket, line: &str) -> Result<(), io::Error> {
        let key = self.key_of(socket);
        match key.map_or(ReceiveSide::Drains, |key| self.receive_side(key)) {
            ReceiveSide::Backpressured => std::future::pending().await,
            ReceiveSide::Refused => {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "guest shut its receive side"))
            }
            ReceiveSide::Drains => {
                self.inner.write_line(socket, line).await?;
                if let Some(key) = key {
                    self.state.lock().log.entry(key).or_default().push(line.to_owned());
                }
                self.changed.notify_waiters();
                Ok(())
            }
        }
    }

    async fn close(&self, socket: BeaconSocket) {
        self.inner.close(socket).await;
        self.changed.notify_waiters();
    }
}

// ---------------------------------------------------------------------------
// PublishesCreated — Vmm decorator announcing each created VMM's pid
// ---------------------------------------------------------------------------

/// Test-only `Vmm` decorator: forwards every call and announces the pid of
/// each successfully created VMM, so the fixture guest can connect once its
/// VMM exists (the old "retry-connect until the listener is bound").
struct PublishesCreated {
    inner: Arc<dyn Vmm>,
    created: mpsc::UnboundedSender<u32>,
}

#[async_trait]
impl Vmm for PublishesCreated {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    async fn probe(&self) -> Result<(), VmmProbeError> {
        self.inner.probe().await
    }

    async fn create(&self, config: &VmConfig, cid: GuestCid) -> VmmResult<VmProcess> {
        let process = self.inner.create(config, cid).await?;
        let _ = self.created.send(process.control.pid);
        Ok(process)
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

// ---------------------------------------------------------------------------
// BeaconHarness — the guest side of the driver's beacon intake
// ---------------------------------------------------------------------------

/// The simulated guest side of one driver's beacon intake.
pub struct BeaconHarness {
    /// The sim listener the driver's intake binds (drives the guest).
    pub io: Arc<SimBeaconIo>,
    wire: Arc<GuestWire>,
    created_tx: mpsc::UnboundedSender<u32>,
    created_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<u32>>,
}

impl BeaconHarness {
    /// A harness whose `SimBeaconIo` is coupled to `sim` (the VMM whose
    /// processes the fixture guests run in).
    pub fn new(sim: &SimVmm) -> Self {
        let io = Arc::new(SimBeaconIo::new(sim.clone()));
        let wire = Arc::new(GuestWire {
            inner: Arc::clone(&io),
            state: Mutex::new(WireState::default()),
            changed: Notify::new(),
        });
        let (created_tx, created_rx) = mpsc::unbounded_channel();
        Self { io, wire, created_tx, created_rx: tokio::sync::Mutex::new(created_rx) }
    }

    /// `inner` wrapped so each created VMM's pid reaches this harness. Hand
    /// the result to `VmDriver::new`.
    pub fn vmm(&self, inner: Arc<dyn Vmm>) -> Arc<dyn Vmm> {
        Arc::new(PublishesCreated { inner, created: self.created_tx.clone() })
    }

    /// The driver's beacon intake, bound over this harness's wire on a
    /// kernel-assigned port (PORTS-CONFIG: a test never assumes a number; it
    /// reads `BeaconIntake::bound_port` back).
    pub async fn intake(&self) -> BeaconIntake {
        BeaconIntake::bind(
            Arc::clone(&self.wire) as Arc<dyn BeaconIo>,
            VsockListenPort::KernelAssigned,
        )
        .await
        .expect("the sim beacon listener binds")
    }

    /// The pid of the next VMM the driver creates.
    pub async fn next_created_pid(&self) -> u32 {
        tokio::time::timeout(GUEST_WAIT, async { self.created_rx.lock().await.recv().await })
            .await
            .expect("the driver created a VMM within 10s")
            .expect("the created-VMM channel stays open")
    }

    /// The guest of VMM `vm_pid` connects with `key`, having written
    /// `buffered` before the host accepts.
    pub fn dial(&self, key: BeaconConnKey, vm_pid: u32, buffered: &[&str]) -> GuestSession {
        self.io.queue_connection(key, vm_pid, buffered).expect("the sim beacon listener is bound");
        GuestSession { io: Arc::clone(&self.io), wire: Arc::clone(&self.wire), key, read: 0 }
    }

    /// Waits for the driver to create `spec`'s VMM, then its guest connects
    /// with the READY line of the fixture launch buffered.
    pub async fn dial_ready(&self, spec: &AllocationSpec) -> GuestSession {
        let pid = self.next_created_pid().await;
        let ready = ready_line();
        self.dial(beacon_key(spec), pid, &[ready.as_str()])
    }
}

/// One fixture guest's beacon connection. Dropping it closes the guest's
/// side entirely, as dropping its socket would: the host reads end of stream
/// and every later host write fails `BrokenPipe`.
pub struct GuestSession {
    io: Arc<SimBeaconIo>,
    wire: Arc<GuestWire>,
    key: BeaconConnKey,
    /// Host → guest lines already returned by [`Self::next_line`].
    read: usize,
}

impl GuestSession {
    /// The guest writes `line` (without terminator).
    pub fn write(&self, line: &str) {
        assert!(self.io.guest_write(self.key, line), "the guest's beacon connection is open");
    }

    /// The guest closes its send direction: the host reads end of stream
    /// once the buffer drains; host writes still reach the guest.
    pub fn close_send(&self) {
        assert!(self.io.guest_close(self.key), "the guest's beacon connection is open");
    }

    /// The guest stops draining its receive side: every later host write on
    /// this connection never completes.
    pub fn backpressure(&self) {
        self.wire.state.lock().receive.insert(self.key, ReceiveSide::Backpressured);
    }

    /// The guest shuts its receive direction: every later host write on this
    /// connection fails `BrokenPipe`.
    pub fn refuse_host_writes(&self) {
        self.wire.state.lock().receive.insert(self.key, ReceiveSide::Refused);
    }

    /// Every host → guest line delivered so far, in order.
    pub fn written(&self) -> Vec<String> {
        self.wire.state.lock().log.get(&self.key).cloned().unwrap_or_default()
    }

    fn closed_by_driver(&self) -> bool {
        self.io.beacon_socket_observation().closed_by_driver.contains(&self.key)
    }

    /// The next host → guest line (without terminator), or `None` once the
    /// driver closed the connection with nothing further written. Waits
    /// indefinitely; bound it with `tokio::time::timeout`.
    pub async fn next_line(&mut self) -> Option<String> {
        loop {
            let changed = self.wire.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some(line) = self.written().get(self.read).cloned() {
                self.read += 1;
                return Some(line);
            }
            if self.closed_by_driver() {
                return None;
            }
            changed.await;
        }
    }

    /// Waits until the driver closes the connection, then returns the
    /// complete host → guest line stream. Waits indefinitely; bound it with
    /// `tokio::time::timeout`.
    pub async fn session_to_end(&self) -> Vec<String> {
        loop {
            let changed = self.wire.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.closed_by_driver() {
                return self.written();
            }
            changed.await;
        }
    }
}

impl Drop for GuestSession {
    fn drop(&mut self) {
        self.wire.state.lock().receive.insert(self.key, ReceiveSide::Refused);
        // `false` once the driver already closed the connection: nothing to do.
        let _ = self.io.guest_close(self.key);
    }
}

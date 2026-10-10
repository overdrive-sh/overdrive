//! `SimBeaconIo` — the in-memory double of the node beacon listener port
//! (`overdrive_core::vm::beacon::BeaconIo`, D17 / DG-4; production adapter
//! `overdrive-host::VsockBeaconIo`).
//!
//! Design: `docs/feature/netns-density-295/feature-delta.md` DG-4 (the port
//! contract) and QPROJ-7 (this sim is the arbiter of the beacon accept queue:
//! its read surface [`SimBeaconIo::beacon_socket_observation`] is pinned).
//!
//! # What it models
//!
//! - **One listener, bound once.** `bind` is idempotent; a scripted bind
//!   failure binds nothing. `accept` before a successful `bind` is refused
//!   (`InvalidInput`), as `accept(2)` on an unbound socket is. A guest
//!   connection queued while nothing is bound is refused
//!   (`ConnectionRefused`), as the guest's `connect` would be.
//! - **Guest connections coupled to a [`SimVmm`] process (K-A2′).** Each
//!   queued connection names the pid of the simulated VMM whose guest opened
//!   it. A connection still in the accept queue when its VMM dies keeps its
//!   buffered lines: once accepted it delivers them, then end of stream
//!   (P-37). A connection accepted while its VMM was alive fails its next
//!   I/O (`ConnectionReset`) once the VMM is no longer live.
//! - **Line framing.** `read_line` returns one `\n`-terminated line without
//!   the terminator; a line over 4,096 bytes is consumed and refused
//!   `InvalidData` (the connection stays for the driver to `close`). `Ok(None)`
//!   once the guest closed its side and nothing is buffered.
//! - **Cancellation safety.** A line or a connection is removed from the sim's
//!   state only in the same non-`await`ing step that returns it, so dropping
//!   an `accept` / `read_line` future loses nothing.
//! - **Handles.** A [`BeaconSocket`] this instance did not issue is
//!   unreachable (`unreachable!`), as the port contract states.
//!
//! # Stepping (DISTILL test support)
//!
//! A sim built with [`SimBeaconIo::stepped`] holds every `accept` and every
//! `read_line` until the test grants it ([`release_accept`](SimBeaconIo::release_accept),
//! [`release_read`](SimBeaconIo::release_read)), so a conformance driver can
//! step `HostAcceptBeacon` / `HostReadReady` one at a time.
//! [`parked`](SimBeaconIo::parked) reports when every in-flight call is
//! waiting and nothing granted is left undelivered. A sim built with
//! [`SimBeaconIo::new`] delivers whatever is available.
//!
//! A [`SimVmm`] does not notify this sim when a process ends; after
//! terminating one, call [`observe_vm_exits`](SimBeaconIo::observe_vm_exits)
//! so parked calls re-check liveness.

#![allow(
    clippy::significant_drop_tightening,
    reason = "each method holds the state lock for one short non-awaiting step; the step IS the atomic check-and-act that keeps accept / read_line cancellation-safe"
)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use overdrive_core::guest_transport::LaunchToken;
use overdrive_core::vm::beacon::{BeaconConnKey, BeaconIo, BeaconSocket};
use parking_lot::Mutex;
use tokio::sync::Notify;

use super::vmm::SimVmm;

/// The longest beacon line, in bytes, without its terminator (DG-4).
pub const SIM_BEACON_LINE_MAX: usize = 4_096;

/// Distinguishes the handles of different sim instances inside one process,
/// so a handle another adapter issued is detected (the port contract's
/// `unreachable!`).
static NEXT_INSTANCE: AtomicU32 = AtomicU32::new(1);

/// The pinned QPROJ-7 read surface: the beacon accept queue with each queued
/// connection's buffered READY token, and the connections the driver closed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SimBeaconSocketObservation {
    /// Connections in the accept queue (not yet accepted), each with the
    /// launch token its buffered READY line carries. `None`: nothing buffered,
    /// or the buffered line carries no valid `launch=` field.
    pub queued: BTreeMap<BeaconConnKey, Option<LaunchToken>>,
    /// Connections the driver closed through [`BeaconIo::close`].
    pub closed_by_driver: BTreeSet<BeaconConnKey>,
}

/// One guest connection, from the guest's `connect` until the driver closes
/// it.
#[derive(Debug)]
struct Conn {
    key: BeaconConnKey,
    /// The pid of the `SimVmm` process whose guest opened the connection.
    vm_pid: u32,
    /// Guest → host lines, without terminators, in write order.
    inbound: VecDeque<String>,
    /// The guest closed its side: end of stream after `inbound` drains.
    guest_closed: bool,
    /// Host → guest lines the driver wrote, in order.
    outbound: Vec<String>,
    /// Whether the VMM was live when the driver accepted the connection
    /// (`None` while queued). `Some(false)`: the P-37 orphan, which delivers
    /// its buffer and then end of stream.
    live_at_accept: Option<bool>,
    /// Stepped mode: `read_line` deliveries granted and not yet taken.
    read_grants: usize,
    /// `read_line` calls currently waiting on this connection.
    read_waiters: usize,
}

#[derive(Debug, Default)]
struct State {
    bound: bool,
    /// One-shot scripted `bind` failure.
    bind_failure: Option<io::ErrorKind>,
    /// Accept queue, FIFO.
    queue: VecDeque<u64>,
    /// Every connection not yet closed by the driver, by handle sequence.
    conns: BTreeMap<u64, Conn>,
    /// Keys the driver closed.
    closed_by_driver: BTreeSet<BeaconConnKey>,
    /// Stepped mode: `accept` deliveries granted and not yet taken.
    accept_grants: usize,
    /// `accept` calls currently waiting.
    accept_waiters: usize,
    next_seq: u64,
}

/// In-memory [`BeaconIo`]: the node beacon listener over simulated guests.
///
/// Cloning is not offered; share it as `Arc<SimBeaconIo>` (the read surface
/// and the stepping methods are on the same instance the driver holds as
/// `Arc<dyn BeaconIo>`).
pub struct SimBeaconIo {
    instance: u32,
    vmm: SimVmm,
    stepped: bool,
    state: Mutex<State>,
    wake: Notify,
}

impl SimBeaconIo {
    /// A sim coupled to `vmm` that delivers whatever is available.
    #[must_use]
    pub fn new(vmm: SimVmm) -> Self {
        Self::build(vmm, false)
    }

    /// A sim coupled to `vmm` whose `accept` and `read_line` wait for an
    /// explicit grant ([`release_accept`](Self::release_accept),
    /// [`release_read`](Self::release_read)).
    #[must_use]
    pub fn stepped(vmm: SimVmm) -> Self {
        Self::build(vmm, true)
    }

    fn build(vmm: SimVmm, stepped: bool) -> Self {
        Self {
            instance: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed),
            vmm,
            stepped,
            state: Mutex::new(State::default()),
            wake: Notify::new(),
        }
    }

    // -----------------------------------------------------------------------
    // The pinned read surface (QPROJ-7)
    // -----------------------------------------------------------------------

    /// The beacon accept queue with each queued connection's buffered READY
    /// token, and the connections the driver closed. Tier-3 pairing: R5-41
    /// (beacon part).
    #[must_use]
    pub fn beacon_socket_observation(&self) -> SimBeaconSocketObservation {
        let state = self.state.lock();
        let queued = state
            .queue
            .iter()
            .filter_map(|seq| state.conns.get(seq))
            .map(|conn| (conn.key, conn.inbound.front().and_then(|line| launch_field(line))))
            .collect();
        SimBeaconSocketObservation { queued, closed_by_driver: state.closed_by_driver.clone() }
    }

    // -----------------------------------------------------------------------
    // Scripting (DISTILL test support)
    // -----------------------------------------------------------------------

    /// The next `bind` fails with `kind` and binds nothing. One-shot.
    pub fn fail_next_bind(&self, kind: io::ErrorKind) {
        self.state.lock().bind_failure = Some(kind);
    }

    /// Whether the listener is bound.
    #[must_use]
    pub fn is_bound(&self) -> bool {
        self.state.lock().bound
    }

    /// A guest of the `SimVmm` process `vm_pid` connects with kernel key
    /// `key`, having written `buffered` (each a line without terminator)
    /// before the host accepts. `Err(ConnectionRefused)` when nothing is
    /// bound, or when `key` already names an open connection (the kernel
    /// never reports one peer twice).
    pub fn queue_connection(
        &self,
        key: BeaconConnKey,
        vm_pid: u32,
        buffered: &[&str],
    ) -> io::Result<()> {
        let mut state = self.state.lock();
        if !state.bound {
            return Err(io::Error::new(io::ErrorKind::ConnectionRefused, "no beacon listener"));
        }
        if state.conns.values().any(|conn| conn.key == key) {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "the peer already has an open beacon connection",
            ));
        }
        let seq = state.next_seq;
        state.next_seq += 1;
        state.conns.insert(
            seq,
            Conn {
                key,
                vm_pid,
                inbound: buffered.iter().map(|line| (*line).to_owned()).collect(),
                guest_closed: false,
                outbound: Vec::new(),
                live_at_accept: None,
                read_grants: 0,
                read_waiters: 0,
            },
        );
        state.queue.push_back(seq);
        drop(state);
        self.wake.notify_waiters();
        Ok(())
    }

    /// The guest of the open connection `key` writes `line` (without
    /// terminator). `false` when no open connection has `key`.
    #[must_use]
    pub fn guest_write(&self, key: BeaconConnKey, line: &str) -> bool {
        let written = self.with_conn(key, |conn| conn.inbound.push_back(line.to_owned()));
        self.wake.notify_waiters();
        written
    }

    /// The guest of the open connection `key` closes its side: the driver
    /// reads end of stream once the buffer drains. `false` when no open
    /// connection has `key`.
    #[must_use]
    pub fn guest_close(&self, key: BeaconConnKey) -> bool {
        let closed = self.with_conn(key, |conn| conn.guest_closed = true);
        self.wake.notify_waiters();
        closed
    }

    /// The lines the driver wrote to the open connection `key` (EXEC,
    /// SHUTDOWN), in order; `None` when no open connection has `key`.
    #[must_use]
    pub fn written_lines(&self, key: BeaconConnKey) -> Option<Vec<String>> {
        let state = self.state.lock();
        state.conns.values().find(|conn| conn.key == key).map(|conn| conn.outbound.clone())
    }

    /// Wake every parked call so it re-checks its VMM's liveness. Call after
    /// terminating a `SimVmm` process.
    pub fn observe_vm_exits(&self) {
        self.wake.notify_waiters();
    }

    // -----------------------------------------------------------------------
    // Stepping (stepped mode)
    // -----------------------------------------------------------------------

    /// Grant one `accept` delivery (`HostAcceptBeacon`). No effect on a sim
    /// built with [`new`](Self::new).
    pub fn release_accept(&self) {
        self.state.lock().accept_grants += 1;
        self.wake.notify_waiters();
    }

    /// Grant one `read_line` delivery on the accepted connection `key`
    /// (`HostReadReady`). `false` when no accepted, open connection has `key`.
    #[must_use]
    pub fn release_read(&self, key: BeaconConnKey) -> bool {
        let granted = {
            let mut state = self.state.lock();
            state
                .conns
                .values_mut()
                .find(|conn| conn.key == key && conn.live_at_accept.is_some())
                .map(|conn| conn.read_grants += 1)
                .is_some()
        };
        self.wake.notify_waiters();
        granted
    }

    /// `true` when every in-flight `accept` / `read_line` is waiting and
    /// nothing granted is left undelivered: no grant is outstanding (stepped
    /// mode), and no waiting call could take a delivery now.
    #[must_use]
    pub fn parked(&self) -> bool {
        let state = self.state.lock();
        let accept_blocked = state.accept_waiters == 0 || !self.accept_ready(&state);
        let reads_blocked =
            state.conns.values().all(|conn| conn.read_waiters == 0 || !self.read_ready(conn));
        let grants_spent =
            state.accept_grants == 0 && state.conns.values().all(|conn| conn.read_grants == 0);
        accept_blocked && reads_blocked && grants_spent
    }

    // -----------------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------------

    fn with_conn(&self, key: BeaconConnKey, f: impl FnOnce(&mut Conn)) -> bool {
        let mut state = self.state.lock();
        state.conns.values_mut().find(|conn| conn.key == key).map(f).is_some()
    }

    fn seq_of(&self, socket: &BeaconSocket) -> u64 {
        let raw = socket.raw();
        let instance = u32::try_from(raw >> 32).unwrap_or(0);
        if instance != self.instance {
            unreachable!("a beacon handle another adapter issued was passed to SimBeaconIo");
        }
        raw & u64::from(u32::MAX)
    }

    fn handle(&self, seq: u64) -> BeaconSocket {
        BeaconSocket::from_raw((u64::from(self.instance) << 32) | seq)
    }

    fn accept_ready(&self, state: &State) -> bool {
        !state.queue.is_empty() && (!self.stepped || state.accept_grants > 0)
    }

    fn read_ready(&self, conn: &Conn) -> bool {
        // The P-37 orphan (accepted after its VMM died) delivers its buffer,
        // then end of stream: it never waits for a guest that cannot write.
        let orphan = conn.live_at_accept == Some(false);
        let deliverable =
            !conn.inbound.is_empty() || conn.guest_closed || orphan || self.dead(conn);
        deliverable && (!self.stepped || conn.read_grants > 0)
    }

    /// An accepted connection whose VMM died after the accept: its next I/O
    /// fails (K-A2′).
    fn dead(&self, conn: &Conn) -> bool {
        conn.live_at_accept == Some(true) && !self.vmm.is_live(conn.vm_pid)
    }

    /// One non-`await`ing attempt at an `accept` delivery.
    fn try_accept(&self) -> Option<io::Result<(BeaconSocket, BeaconConnKey)>> {
        let mut state = self.state.lock();
        if !state.bound {
            return Some(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "accept on an unbound beacon listener",
            )));
        }
        if !self.accept_ready(&state) {
            return None;
        }
        let seq = state.queue.pop_front()?;
        if self.stepped {
            state.accept_grants -= 1;
        }
        let Some(conn) = state.conns.get_mut(&seq) else {
            unreachable!("every queued handle names an open connection");
        };
        conn.live_at_accept = Some(self.vmm.is_live(conn.vm_pid));
        let key = conn.key;
        drop(state);
        Some(Ok((self.handle(seq), key)))
    }

    /// One non-`await`ing attempt at a `read_line` delivery.
    fn try_read(&self, seq: u64) -> Option<io::Result<Option<String>>> {
        let mut state = self.state.lock();
        let Some(conn) = state.conns.get_mut(&seq) else {
            unreachable!("a live BeaconSocket names an open connection");
        };
        if !self.read_ready(conn) {
            return None;
        }
        if self.stepped {
            conn.read_grants -= 1;
        }
        if self.dead(conn) {
            return Some(Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "the guest's VM is gone",
            )));
        }
        Some(match conn.inbound.pop_front() {
            Some(line) if line.len() > SIM_BEACON_LINE_MAX => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "beacon line longer than 4096 bytes",
            )),
            Some(line) => Ok(Some(line)),
            // Nothing buffered and deliverable: the guest closed, or the
            // P-37 orphan drained its buffer.
            None => Ok(None),
        })
    }
}

/// The launch token a buffered READY line carries in its `launch=` field.
/// The line is the guest's raw text; only the field is projected.
fn launch_field(line: &str) -> Option<LaunchToken> {
    if !line.starts_with("READY ") {
        return None;
    }
    line.split(' ')
        .find_map(|token| token.strip_prefix("launch="))
        .and_then(|value| LaunchToken::from_str(value).ok())
}

/// Decrements a waiter counter when a waiting call returns or is dropped.
struct Waiting<'a> {
    io: &'a SimBeaconIo,
    seq: Option<u64>,
}

impl<'a> Waiting<'a> {
    fn accept(io: &'a SimBeaconIo) -> Self {
        io.state.lock().accept_waiters += 1;
        Self { io, seq: None }
    }

    fn read(io: &'a SimBeaconIo, seq: u64) -> Self {
        if let Some(conn) = io.state.lock().conns.get_mut(&seq) {
            conn.read_waiters += 1;
        }
        Self { io, seq: Some(seq) }
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        let mut state = self.io.state.lock();
        match self.seq {
            None => state.accept_waiters -= 1,
            Some(seq) => {
                if let Some(conn) = state.conns.get_mut(&seq) {
                    conn.read_waiters -= 1;
                }
            }
        }
    }
}

#[async_trait]
impl BeaconIo for SimBeaconIo {
    async fn bind(&self) -> Result<(), io::Error> {
        let mut state = self.state.lock();
        if state.bound {
            return Ok(());
        }
        if let Some(kind) = state.bind_failure.take() {
            return Err(io::Error::new(kind, "sim-scripted beacon bind failure"));
        }
        state.bound = true;
        Ok(())
    }

    async fn accept(&self) -> Result<(BeaconSocket, BeaconConnKey), io::Error> {
        let _waiting = Waiting::accept(self);
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(outcome) = self.try_accept() {
                return outcome;
            }
            notified.await;
        }
    }

    async fn read_line(&self, socket: &BeaconSocket) -> Result<Option<String>, io::Error> {
        let seq = self.seq_of(socket);
        let _waiting = Waiting::read(self, seq);
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(outcome) = self.try_read(seq) {
                return outcome;
            }
            notified.await;
        }
    }

    async fn write_line(&self, socket: &BeaconSocket, line: &str) -> Result<(), io::Error> {
        let seq = self.seq_of(socket);
        let mut state = self.state.lock();
        let Some(conn) = state.conns.get_mut(&seq) else {
            unreachable!("a live BeaconSocket names an open connection");
        };
        if !self.vmm.is_live(conn.vm_pid) {
            return Err(io::Error::new(io::ErrorKind::ConnectionReset, "the guest's VM is gone"));
        }
        conn.outbound.push(line.to_owned());
        Ok(())
    }

    async fn close(&self, socket: BeaconSocket) {
        let seq = self.seq_of(&socket);
        let mut state = self.state.lock();
        let Some(conn) = state.conns.remove(&seq) else {
            unreachable!("a live BeaconSocket names an open connection");
        };
        state.closed_by_driver.insert(conn.key);
        drop(state);
        self.wake.notify_waiters();
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use futures::FutureExt as _;
    use overdrive_core::guest_transport::{GUEST_CID_BASE, GuestCid};
    use overdrive_core::traits::vmm::Vmm as _;
    use overdrive_core::vm::config::VsockPort;

    use super::*;

    const STEP2: &str = "GuestCid / LaunchToken bodies land in DELIVER vsock new-step 2";

    fn key(offset: u32, port: u32) -> BeaconConnKey {
        BeaconConnKey {
            peer_cid: GuestCid::new(GUEST_CID_BASE + offset).expect(STEP2),
            peer_port: VsockPort::new(port),
        }
    }

    const TOKEN: &str = "0102030405060708090a0b0c0d0e0f10";

    fn ready(token: &str) -> String {
        format!("READY pid=1 port=1234 launch={token}")
    }

    async fn bound(vmm: &SimVmm) -> SimBeaconIo {
        let io = SimBeaconIo::new(vmm.clone());
        io.bind().await.expect("bind");
        io
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn bind_is_idempotent() {
        let io = SimBeaconIo::new(SimVmm::new());
        assert!(!io.is_bound());
        io.bind().await.expect("first bind");
        io.bind().await.expect("a second bind is Ok and changes nothing");
        assert!(io.is_bound());
    }

    /// CONTRACT_SHAPE: unbounded-preservation.
    #[tokio::test]
    async fn a_scripted_bind_failure_binds_nothing_and_is_one_shot() {
        let io = SimBeaconIo::new(SimVmm::new());
        io.fail_next_bind(io::ErrorKind::AddrInUse);
        let err = io.bind().await.expect_err("the scripted bind fails");
        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
        assert!(!io.is_bound(), "nothing bound");
        io.bind().await.expect("the failure was one-shot");
        assert!(io.is_bound());
    }

    /// CONTRACT_SHAPE: unbounded-preservation.
    #[tokio::test]
    async fn accept_before_bind_is_refused() {
        let io = SimBeaconIo::new(SimVmm::new());
        let err = io.accept().await.expect_err("accept on an unbound listener");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    /// CONTRACT_SHAPE: unbounded-preservation.
    #[tokio::test]
    #[should_panic(expected = "another adapter issued")]
    async fn a_handle_another_adapter_issued_is_unreachable() {
        let io = SimBeaconIo::new(SimVmm::new());
        let foreign = BeaconSocket::from_raw(0);
        let _ = io.read_line(&foreign).await;
    }

    /// CONTRACT_SHAPE: unbounded-preservation.
    #[tokio::test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    async fn a_guest_cannot_connect_before_the_listener_is_bound() {
        let vmm = SimVmm::new();
        let io = SimBeaconIo::new(vmm.clone());
        let pid = vmm.spawn_bare_process().pid;
        let err = io.queue_connection(key(1, 50_000), pid, &[]).expect_err("refused");
        assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
        assert_eq!(io.beacon_socket_observation(), SimBeaconSocketObservation::default());
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    async fn a_queued_ready_is_observed_then_accepted_and_read() {
        let vmm = SimVmm::new();
        let io = bound(&vmm).await;
        let pid = vmm.spawn_bare_process().pid;
        let k = key(1, 50_000);
        let line = ready(TOKEN);
        io.queue_connection(k, pid, &[&line]).expect("connect");

        let token = LaunchToken::from_str(TOKEN).expect("a valid token");
        let before = io.beacon_socket_observation();
        assert_eq!(before.queued, BTreeMap::from([(k, Some(token))]));
        assert!(before.closed_by_driver.is_empty());

        let (socket, accepted) = io.accept().await.expect("accept");
        assert_eq!(accepted, k, "the kernel-reported key");
        assert!(io.beacon_socket_observation().queued.is_empty(), "no longer queued");
        assert_eq!(io.read_line(&socket).await.expect("read"), Some(line));

        io.write_line(&socket, "EXEC [\"/bin/true\"]").await.expect("write");
        assert_eq!(io.written_lines(k), Some(vec!["EXEC [\"/bin/true\"]".to_owned()]));

        io.close(socket).await;
        let after = io.beacon_socket_observation();
        assert!(after.queued.is_empty());
        assert_eq!(after.closed_by_driver, BTreeSet::from([k]));
        assert_eq!(io.written_lines(k), None, "the closed connection is gone");
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    async fn a_queued_connection_with_nothing_buffered_or_no_valid_token_projects_none() {
        let vmm = SimVmm::new();
        let io = bound(&vmm).await;
        let pid = vmm.spawn_bare_process().pid;
        io.queue_connection(key(1, 1), pid, &[]).expect("connect");
        io.queue_connection(key(2, 1), pid, &["READY pid=1 port=1234"]).expect("connect");
        io.queue_connection(key(3, 1), pid, &[&ready("XYZ")]).expect("connect");
        let queued = io.beacon_socket_observation().queued;
        assert_eq!(
            queued,
            BTreeMap::from([(key(1, 1), None), (key(2, 1), None), (key(3, 1), None)])
        );
    }

    /// K-A2′: an accepted connection whose VM died fails its next I/O.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    async fn an_accepted_connection_whose_vm_died_fails_its_next_io() {
        let vmm = SimVmm::new();
        let io = bound(&vmm).await;
        let control = vmm.spawn_bare_process();
        let k = key(4, 7);
        io.queue_connection(k, control.pid, &[]).expect("connect");
        let (socket, _) = io.accept().await.expect("accept while the VM is live");

        vmm.terminate(&control, Duration::ZERO).await.expect("terminate");
        io.observe_vm_exits();
        assert!(io.guest_write(k, &ready(TOKEN)), "a late buffered line is not delivered");

        let read = io.read_line(&socket).await.expect_err("the next read fails");
        assert_eq!(read.kind(), io::ErrorKind::ConnectionReset);
        let write = io.write_line(&socket, "SHUTDOWN").await.expect_err("the next write fails");
        assert_eq!(write.kind(), io::ErrorKind::ConnectionReset);
        io.close(socket).await;
    }

    /// A parked read on a connection whose VM then dies wakes and fails once
    /// the exit is observed.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    async fn a_parked_read_fails_when_its_vm_dies() {
        let vmm = SimVmm::new();
        let io = Arc::new(bound(&vmm).await);
        let control = vmm.spawn_bare_process();
        io.queue_connection(key(5, 7), control.pid, &[]).expect("connect");
        let (socket, _) = io.accept().await.expect("accept");

        let reader = {
            let io = Arc::clone(&io);
            tokio::spawn(async move {
                let outcome = io.read_line(&socket).await;
                io.close(socket).await;
                outcome
            })
        };
        tokio::task::yield_now().await;
        vmm.terminate(&control, Duration::ZERO).await.expect("terminate");
        io.observe_vm_exits();
        let outcome = reader.await.expect("join");
        assert_eq!(outcome.expect_err("fails").kind(), io::ErrorKind::ConnectionReset);
    }

    /// P-37: a connection still queued when its VM died delivers its buffered
    /// READY once accepted, then end of stream.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    async fn a_queued_connection_whose_vm_died_delivers_its_buffered_ready() {
        let vmm = SimVmm::new();
        let io = bound(&vmm).await;
        let control = vmm.spawn_bare_process();
        let k = key(6, 9);
        let line = ready(TOKEN);
        io.queue_connection(k, control.pid, &[&line]).expect("connect");
        vmm.terminate(&control, Duration::ZERO).await.expect("terminate");

        let token = LaunchToken::from_str(TOKEN).expect("a valid token");
        assert_eq!(
            io.beacon_socket_observation().queued,
            BTreeMap::from([(k, Some(token))]),
            "the queue keeps the buffered READY"
        );
        let (socket, _) = io.accept().await.expect("accept");
        assert_eq!(io.read_line(&socket).await.expect("buffered line"), Some(line));
        assert_eq!(io.read_line(&socket).await.expect("end of stream"), None);
        io.close(socket).await;
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    async fn a_line_over_4096_bytes_is_invalid_data_and_the_connection_stays() {
        let vmm = SimVmm::new();
        let io = bound(&vmm).await;
        let pid = vmm.spawn_bare_process().pid;
        let k = key(7, 1);
        let long = "x".repeat(SIM_BEACON_LINE_MAX + 1);
        let exact = "y".repeat(SIM_BEACON_LINE_MAX);
        io.queue_connection(k, pid, &[&long, &exact]).expect("connect");
        let (socket, _) = io.accept().await.expect("accept");

        let err = io.read_line(&socket).await.expect_err("over the limit");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(io.read_line(&socket).await.expect("at the limit"), Some(exact));
        io.close(socket).await;
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    async fn the_guest_closing_is_end_of_stream_after_its_buffer() {
        let vmm = SimVmm::new();
        let io = bound(&vmm).await;
        let pid = vmm.spawn_bare_process().pid;
        let k = key(8, 1);
        io.queue_connection(k, pid, &["EXIT 0"]).expect("connect");
        let (socket, _) = io.accept().await.expect("accept");
        assert!(io.guest_close(k));
        assert_eq!(io.read_line(&socket).await.expect("buffered"), Some("EXIT 0".to_owned()));
        assert_eq!(io.read_line(&socket).await.expect("eof"), None);
        io.close(socket).await;
    }

    /// Cancellation safety: a dropped `read_line` / `accept` loses nothing.
    /// CONTRACT_SHAPE: unbounded-preservation.
    #[tokio::test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    async fn dropped_calls_lose_no_connection_and_no_line() {
        let vmm = SimVmm::new();
        let io = bound(&vmm).await;
        let pid = vmm.spawn_bare_process().pid;
        let k = key(9, 1);

        assert!(io.accept().now_or_never().is_none(), "nothing queued: the accept parks");
        io.queue_connection(k, pid, &[]).expect("connect");
        let (socket, _) = io.accept().await.expect("the queued connection survived");

        assert!(io.read_line(&socket).now_or_never().is_none(), "nothing buffered: parks");
        assert!(io.guest_write(k, "EXIT 3"));
        assert_eq!(io.read_line(&socket).await.expect("read"), Some("EXIT 3".to_owned()));
        io.close(socket).await;
    }

    /// Stepped mode: `accept` and `read_line` wait for grants; `parked`
    /// reports quiescence.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    async fn stepped_mode_holds_every_delivery_until_granted() {
        let vmm = SimVmm::new();
        let io = Arc::new(SimBeaconIo::stepped(vmm.clone()));
        io.bind().await.expect("bind");
        let pid = vmm.spawn_bare_process().pid;
        let k = key(10, 1);
        io.queue_connection(k, pid, &[&ready(TOKEN)]).expect("connect");

        let acceptor = {
            let io = Arc::clone(&io);
            tokio::spawn(async move { io.accept().await })
        };
        tokio::task::yield_now().await;
        assert!(io.parked(), "the accept waits; nothing is granted");
        assert_eq!(io.beacon_socket_observation().queued.len(), 1, "still queued");

        io.release_accept();
        let (socket, accepted) = acceptor.await.expect("join").expect("accept");
        assert_eq!(accepted, k);
        assert!(io.parked(), "the grant was spent");

        let reader = {
            let io = Arc::clone(&io);
            tokio::spawn(async move {
                let line = io.read_line(&socket).await;
                (line, socket)
            })
        };
        tokio::task::yield_now().await;
        assert!(io.parked(), "the read waits for its grant");
        assert!(io.release_read(k));
        let (line, socket) = reader.await.expect("join");
        assert_eq!(line.expect("read"), Some(ready(TOKEN)));
        io.close(socket).await;
        assert!(io.parked());
    }
}

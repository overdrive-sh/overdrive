//! `BeaconIntake` — the node's bound beacon listener and its accept task
//! (D17, DG-4).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! DG-4 (`BeaconIntake::bind`, `BeaconIntakeError`) over the core
//! `overdrive_core::vm::beacon::BeaconIo` port (production `VsockBeaconIo`,
//! simulation `SimBeaconIo`).
//!
//! `BeaconIntake::bind` owns the node listener's accept task and routes
//! launch-attributed READY sessions to the driver's waiting start.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::Arc;

use overdrive_core::guest_transport::{GuestCid, LaunchToken, VsockListenPort};
use overdrive_core::id::AllocationId;
use overdrive_core::vm::beacon::{BeaconConnKey, BeaconIo, BeaconMessage, BeaconSocket};
use overdrive_core::vm::config::VsockPort;
use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub(crate) struct BeaconCandidate {
    pub(crate) socket: BeaconSocket,
    pub(crate) key: BeaconConnKey,
}

#[derive(Default)]
pub(crate) struct BeaconState {
    pub(crate) live: BTreeMap<AllocationId, (GuestCid, LaunchToken)>,
    pub(crate) claims: BTreeMap<GuestCid, (AllocationId, BeaconConnKey)>,
    pub(crate) unread: BTreeSet<BeaconConnKey>,
    pub(crate) pending: BTreeMap<BeaconConnKey, tokio::sync::watch::Sender<bool>>,
    pub(crate) ready: BTreeMap<GuestCid, BeaconCandidate>,
    pub(crate) notifiers: BTreeMap<GuestCid, Arc<Notify>>,
    pub(crate) accept_failure: Option<(io::ErrorKind, String)>,
}

/// D17: the node's bound beacon listener and its accept task. Exists only
/// bound; dropping it stops the accept task and closes the listener.
pub struct BeaconIntake {
    io: Arc<dyn BeaconIo>,
    bound_port: VsockPort,
    state: Arc<Mutex<BeaconState>>,
    shutdown: CancellationToken,
    accept_task: Option<JoinHandle<()>>,
}

impl BeaconIntake {
    /// Binds through `io` on `port` and starts the accept task. Production
    /// passes `VsockBeaconIo` and `config.vsock_ports.beacon()`; tests and
    /// `run_server` boots with a simulated VMM pass `SimBeaconIo`. Called
    /// once by `compose_vm_driver`, after `Vmm::probe`. Err →
    /// `health.startup.refused`, reason `vm_driver.beacon_bind`; nothing
    /// bound.
    pub async fn bind(
        io: Arc<dyn BeaconIo>,
        port: VsockListenPort,
    ) -> Result<Self, BeaconIntakeError> {
        let bound_port =
            io.bind(port).await.map_err(|source| BeaconIntakeError::Bind { port, source })?;
        let state = Arc::new(Mutex::new(BeaconState::default()));
        let shutdown = CancellationToken::new();
        let accept_task =
            tokio::spawn(run_accept_task(Arc::clone(&io), Arc::clone(&state), shutdown.clone()));
        Ok(Self { io, bound_port, state, shutdown, accept_task: Some(accept_task) })
    }

    /// PORTS-CONFIG: the port `bind` took; constant for the intake's life.
    /// The VM driver writes it on each guest's kernel command line
    /// (`beacon=`, ADR-0217).
    #[must_use]
    pub const fn bound_port(&self) -> VsockPort {
        self.bound_port
    }

    /// The exact adapter bound by this intake, for the driver's private
    /// accepted-session owner.
    pub(crate) fn io(&self) -> Arc<dyn BeaconIo> {
        Arc::clone(&self.io)
    }

    pub(crate) fn state(&self) -> Arc<Mutex<BeaconState>> {
        Arc::clone(&self.state)
    }

    pub(crate) async fn take_ready(&self, cid: GuestCid) -> Result<BeaconCandidate, io::Error> {
        let notify = {
            let mut state = self.state.lock();
            Arc::clone(state.notifiers.entry(cid).or_insert_with(|| Arc::new(Notify::new())))
        };
        loop {
            let notified = notify.notified();
            {
                let mut state = self.state.lock();
                if let Some(candidate) = state.ready.remove(&cid) {
                    return Ok(candidate);
                }
                if let Some((kind, message)) = &state.accept_failure {
                    return Err(io::Error::new(*kind, message.clone()));
                }
            }
            notified.await;
        }
    }

    pub(crate) async fn close(&self, socket: BeaconSocket) {
        self.io.close(socket).await;
    }
}

impl Drop for BeaconIntake {
    fn drop(&mut self) {
        self.shutdown.cancel();
        let state = self.state.lock();
        for cancel in state.pending.values() {
            let _ = cancel.send(true);
        }
        drop(state);
        if let Some(task) = self.accept_task.take() {
            drop(task);
        }
    }
}

async fn run_accept_task(
    io: Arc<dyn BeaconIo>,
    state: Arc<Mutex<BeaconState>>,
    shutdown: CancellationToken,
) {
    loop {
        let accepted = tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            result = io.accept() => result,
        };
        match accepted {
            Ok((socket, key)) => {
                let live = {
                    let mut state = state.lock();
                    let live = state
                        .live
                        .iter()
                        .find(|(_, (cid, _))| *cid == key.peer_cid)
                        .map(|(alloc, (_, launch))| (alloc.clone(), *launch));
                    let pending = live.map_or_else(
                        || None,
                        |live| {
                            let (cancel, cancel_rx) = tokio::sync::watch::channel(false);
                            state.unread.insert(key);
                            state.pending.insert(key, cancel);
                            Some((live, cancel_rx))
                        },
                    );
                    drop(state);
                    pending
                };
                let Some(((alloc, launch), cancel_rx)) = live else {
                    io.close(socket).await;
                    continue;
                };
                tokio::spawn(read_ready_candidate(
                    Arc::clone(&io),
                    Arc::clone(&state),
                    socket,
                    key,
                    alloc,
                    launch,
                    cancel_rx,
                ));
            }
            Err(error) => {
                let mut state = state.lock();
                state.accept_failure = Some((error.kind(), error.to_string()));
                for notify in state.notifiers.values() {
                    notify.notify_waiters();
                }
                drop(state);
                break;
            }
        }
    }

    let ready = {
        let mut state = state.lock();
        state.pending.values().for_each(|cancel| {
            let _ = cancel.send(true);
        });
        std::mem::take(&mut state.ready)
    };
    for (_, candidate) in ready {
        io.close(candidate.socket).await;
    }
}

async fn read_ready_candidate(
    io: Arc<dyn BeaconIo>,
    state: Arc<Mutex<BeaconState>>,
    socket: BeaconSocket,
    key: BeaconConnKey,
    alloc: AllocationId,
    launch: LaunchToken,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) {
    let line = tokio::select! {
        biased;
        () = wait_for_cancel(&mut cancel) => None,
        result = io.read_line(&socket) => Some(result),
    };
    let ready_launch = match line {
        Some(Ok(Some(line))) => match line.parse::<BeaconMessage>() {
            Ok(BeaconMessage::Ready { launch, .. }) => Some(launch),
            _ => None,
        },
        Some(Ok(None) | Err(_)) | None => None,
    };
    let mut socket = Some(socket);
    let accepted = {
        let mut state = state.lock();
        state.unread.remove(&key);
        state.pending.remove(&key);
        let current = state.live.get(&alloc);
        let valid = current == Some(&(key.peer_cid, launch))
            && ready_launch == Some(launch)
            && !state.claims.contains_key(&key.peer_cid);
        if valid {
            state.claims.insert(key.peer_cid, (alloc, key));
            if let Some(socket) = socket.take() {
                state.ready.insert(key.peer_cid, BeaconCandidate { socket, key });
            }
            if let Some(notify) = state.notifiers.get(&key.peer_cid) {
                notify.notify_one();
            }
        }
        valid
    };
    if !accepted && let Some(socket) = socket {
        io.close(socket).await;
    }
}

async fn wait_for_cancel(cancel: &mut tokio::sync::watch::Receiver<bool>) {
    if *cancel.borrow() {
        return;
    }
    while cancel.changed().await.is_ok() {
        if *cancel.borrow() {
            return;
        }
    }
}

/// Why the node beacon intake could not be bound.
#[derive(Debug, thiserror::Error)]
pub enum BeaconIntakeError {
    /// Binding the beacon vsock listener failed; nothing is bound. `port` is
    /// the configured `VsockListenPort`.
    #[error("binding the beacon vsock listener on port {port:?} failed: {source}")]
    Bind {
        /// The configured port.
        port: VsockListenPort,
        /// The bind error.
        #[source]
        source: std::io::Error,
    },
}

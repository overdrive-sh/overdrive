//! INB-1 (ADR-0173, ADR-0174, ADR-0187): the host guest-flow owner is the
//! production `GuestStreamDial` (§ *Driving port — guest stream dial
//! `GuestStreamDial`*). A host platform client (leg-S, the VM probe runner)
//! reaches a declared TCP port of an Active VM allocation only through a
//! flow this owner opens over vsock.
//!
//! Order of decision (the owner, in the target CID's serialized order, M-7):
//! not an Active lease's `workload_addr` with a declared port →
//! `NotServing`; node forwarding quiesced → `Quiesced`; the CID's control
//! session not live as the owner observes it → `NoSession`; node flow slots
//! full, or (for `DialPurpose::Relay` only) the allocation's quota full →
//! `Capacity`. Steps 1–4 return immediately and change nothing. Then:
//! `connect_guest(cid)` → `watch(V_h)` (EVMOD-END; a watch refused
//! `HalfEnded` ends the dial `Unreachable { errno }`, EVMOD-ATTACH-END) → the
//! TcpAccept request (peer = gateway:0, the declared port, the live launch
//! token) → the guest's `Paired` / `Refused { errno }` / `TimedOut { errno }`,
//! or `GUEST_DIAL_PAIRING_DEADLINE` → on `Paired`, `into_guest_stream(V_h,
//! watch, flow, target)`. The stream resolves every end it observes from the
//! watch (EVMOD-STREAM-END); an owner abort ends it at once without touching
//! `V_h` (EVMOD-STREAM-ABORT, WATCH-ABORT); its drop aborts the watch, then
//! closes `V_h`; the node flow slot and, for `Relay`, the quota place are
//! released at the drop or the owner's abort (EVMOD-AB).
//!
use std::net::SocketAddrV4;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use overdrive_core::guest_transport::GuestCid;
use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestDialError, GuestStream, GuestStreamDial, GuestStreamEnd, GuestStreamIo,
};
use overdrive_core::guest_transport::forwarder::ForwarderError;
use overdrive_core::guest_transport::wire::{
    ControlMessage, FlowId, FlowOrigin, FlowRefusal, FlowRequest, GUEST_DIAL_PAIRING_DEADLINE,
    GUEST_FLOW_MAX_PAIRS, KernelErrno,
};
use overdrive_core::net::ListenPort;
use overdrive_core::traits::clock::Clock;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::flow_io::{GuestFlowIo, RecordSocket};
use super::flow_owner::{CidPhase, DialObservation, DialPhase, OwnerRuntime};
use super::{FlowCloseCause, HostSharedGuestNetworkOwner};

#[async_trait::async_trait]
impl GuestStreamDial for HostSharedGuestNetworkOwner {
    #[allow(
        clippy::too_many_lines,
        reason = "dial admission and task ownership are one serialized owner-port operation"
    )]
    async fn dial(
        &self,
        target: SocketAddrV4,
        purpose: DialPurpose,
    ) -> Result<GuestStream, GuestDialError> {
        let Some(listen_port) = ListenPort::new(target.port()).ok() else {
            return Err(GuestDialError::NotServing);
        };
        let cid = {
            let state = self.runtime.state.lock();
            let Some((cid, entry)) =
                state.cids.iter().find(|(_, entry)| entry.workload_addr == *target.ip())
            else {
                return Err(GuestDialError::NotServing);
            };
            if entry.phase != CidPhase::Active || !entry.listen_ports.contains(&listen_port) {
                return Err(GuestDialError::NotServing);
            }
            let cid = *cid;
            drop(state);
            cid
        };
        let (flow, cancellation, reply_rx, launch, gateway, alloc) = {
            let mut state = self.runtime.state.lock();
            if state.admission == super::GuestAdmission::Quiesced {
                return Err(GuestDialError::Quiesced);
            }
            let (launch, alloc, quota_full, session_live) = {
                let entry = state.cids.get(&cid).ok_or(GuestDialError::NotServing)?;
                (
                    entry.launch,
                    entry.alloc.clone(),
                    purpose == DialPurpose::Relay
                        && entry.held_flows()
                            >= usize::try_from(super::GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION)
                                .unwrap_or(usize::MAX),
                    entry.session.is_some() && entry.session_sender.is_some(),
                )
            };
            if !session_live {
                return Err(GuestDialError::NoSession);
            }
            if quota_full {
                return Err(GuestDialError::Capacity);
            }
            let inventory = self.io.inventory().map_err(|_| GuestDialError::Capacity)?;
            let forwarder_places = inventory.live_pairs.saturating_add(inventory.live_watches);
            let occupied = state
                .occupied_node_flow_places()
                .max(usize::try_from(forwarder_places).unwrap_or(usize::MAX));
            if occupied >= usize::try_from(GUEST_FLOW_MAX_PAIRS).unwrap_or(usize::MAX) {
                return Err(GuestDialError::Capacity);
            }
            let flow = allocate_flow_id(&self.runtime)?;
            let cancellation = CancellationToken::new();
            let (reply_tx, reply_rx) = oneshot::channel();
            let gateway = self.ports.gateway;
            let entry = state.cids.get_mut(&cid).ok_or(GuestDialError::NotServing)?;
            entry.dials.insert(flow, DialObservation { purpose, phase: DialPhase::AwaitingReply });
            entry.dial_replies.insert(flow, reply_tx);
            entry.dial_cancels.insert(flow, cancellation.clone());
            let dial = (flow, cancellation, reply_rx, launch, gateway, alloc);
            drop(state);
            dial
        };
        let (result_tx, result_rx) = oneshot::channel();
        let runtime = Arc::clone(&self.runtime);
        let io = Arc::clone(&self.io);
        let clock = Arc::clone(&self.ports.clock);
        let task_cancel = cancellation.clone();
        tokio::spawn(async move {
            let result = run_dial(
                Arc::clone(&runtime),
                io,
                clock,
                cid,
                flow,
                purpose,
                target,
                listen_port,
                launch,
                gateway,
                alloc,
                reply_rx,
                task_cancel,
            )
            .await;
            let _ = result_tx.send(result);
        });
        let mut guard = DialCancelGuard { token: cancellation.clone(), complete: false };
        let result = result_rx.await.unwrap_or(Err(GuestDialError::PairingDeadline));
        guard.complete = true;
        result
    }
}

fn allocate_flow_id(runtime: &OwnerRuntime) -> Result<FlowId, GuestDialError> {
    let sequence = runtime
        .next_dial
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            (next < (1 << 31)).then_some(next + 1)
        })
        .map_err(|_| GuestDialError::Capacity)?;
    FlowId::new(FlowOrigin::Host, sequence).map_err(|_| GuestDialError::Capacity)
}

struct DialCancelGuard {
    token: CancellationToken,
    complete: bool,
}

impl Drop for DialCancelGuard {
    fn drop(&mut self) {
        if !self.complete {
            self.token.cancel();
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "the dial is one ordered owner step; keeping its ports and cancellation cleanup together preserves the attempt and abort ordering"
)]
async fn run_dial(
    runtime: Arc<OwnerRuntime>,
    io: Arc<dyn GuestFlowIo>,
    clock: Arc<dyn Clock>,
    cid: GuestCid,
    flow: FlowId,
    purpose: DialPurpose,
    target: SocketAddrV4,
    listen_port: ListenPort,
    launch: overdrive_core::guest_transport::LaunchToken,
    gateway: std::net::Ipv4Addr,
    alloc: overdrive_core::id::AllocationId,
    reply: oneshot::Receiver<ControlMessage>,
    cancel: CancellationToken,
) -> Result<GuestStream, GuestDialError> {
    let mut vsock: Option<RecordSocket> = None;
    let mut watch = None;
    let mut request_written = false;
    let mut abort_guest = false;
    let result = async {
        let connected = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(cancellation_error(&runtime, cid, flow)),
            result = io.connect_guest(cid) => result,
        };
        let socket = connected.map_err(|error| {
            let source = match error {
                super::flow_io::FlowIoError::Io { source } => source,
            };
            GuestDialError::Unreachable { errno: errno_from_io(&source) }
        })?;
        vsock = Some(socket);
        let Some(socket_ref) = vsock.as_ref() else {
            unreachable!("connected V_h is retained until pairing")
        };
        let observed_watch = match io.watch(socket_ref) {
            Ok(watch) => watch,
            Err(ForwarderError::HalfEnded { errno, .. }) => {
                let mut state = runtime.state.lock();
                state.forwarder_attach_ended = state.forwarder_attach_ended.saturating_add(1);
                drop(state);
                return Err(GuestDialError::Unreachable { errno });
            }
            Err(error) => {
                return Err(GuestDialError::Unreachable { errno: errno_from_forwarder(error) });
            }
        };
        watch = Some(observed_watch);
        {
            let mut state = runtime.state.lock();
            state.dial_watches.insert(observed_watch, (cid, flow));
            state.forwarder_attached = state.forwarder_attached.saturating_add(1);
        }
        let request = FlowRequest::TcpAccept { flow, peer: gateway, listen_port, launch };
        let encoded_request = request.encode();
        let Some(socket_ref) = vsock.as_ref() else {
            unreachable!("V_h remains held before request write")
        };
        let written = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(cancellation_error(&runtime, cid, flow)),
            result = io.write_record(socket_ref, &encoded_request) => result,
        };
        if let Err(super::flow_io::FlowIoError::Io { source }) = written {
            return Err(GuestDialError::Unreachable { errno: errno_from_io(&source) });
        }
        request_written = true;
        if let Some(entry) = runtime.state.lock().cids.get_mut(&cid) {
            entry.dial_requests.insert(flow);
        }
        let deadline = clock.sleep(GUEST_DIAL_PAIRING_DEADLINE);
        tokio::pin!(deadline);
        let message = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                let error = cancellation_error(&runtime, cid, flow);
                abort_guest = request_written
                    && matches!(
                        error,
                        GuestDialError::Quiesced
                            | GuestDialError::NotServing
                            | GuestDialError::PairingDeadline
                    );
                return Err(error);
            }
            () = &mut deadline => {
                abort_guest = true;
                return Err(GuestDialError::PairingDeadline);
            }
            message = reply => message.map_err(|_| GuestDialError::NoSession)?,
        };
        match message {
            ControlMessage::Paired { flow: paired } if paired == flow => {
                let generation = runtime.state.lock().forwarder_generation;
                if cancel.is_cancelled() {
                    return Err(cancellation_error(&runtime, cid, flow));
                }
                if let Some(entry) = runtime.state.lock().cids.get_mut(&cid) {
                    entry.dials.insert(
                        flow,
                        DialObservation {
                            purpose,
                            phase: DialPhase::Paired {
                                watch: observed_watch,
                                generation,
                                guest_end: None,
                                host_shut: false,
                            },
                        },
                    );
                    entry.dial_replies.remove(&flow);
                    entry.dial_cancels.remove(&flow);
                } else {
                    return Err(GuestDialError::NotServing);
                }
                let Some(socket) = vsock.take() else {
                    unreachable!("paired V_h moves to GuestStream")
                };
                let stream = io.into_guest_stream(socket, observed_watch, flow, target);
                Ok(GuestStream::new(
                    target,
                    Box::new(OwnerObservedStream {
                        stream,
                        runtime: Arc::clone(&runtime),
                        cid,
                        flow,
                        watch: observed_watch,
                        alloc,
                    }),
                ))
            }
            ControlMessage::Refused { flow: refused, refusal } if refused == flow => {
                let errno = match refusal {
                    FlowRefusal::ApplicationRefused { errno } => errno,
                    _ => KernelErrno::ECONNREFUSED,
                };
                Err(GuestDialError::Refused { errno })
            }
            ControlMessage::TimedOut { flow: timed_out, errno } if timed_out == flow => {
                Err(GuestDialError::TimedOut { errno })
            }
            _ => Err(GuestDialError::PairingDeadline),
        }
    }
    .await;

    if result.is_err() {
        if let Some(watch) = watch {
            if abort_guest {
                let sender = runtime
                    .state
                    .lock()
                    .cids
                    .get(&cid)
                    .and_then(|entry| entry.session_sender.clone());
                if let Some(sender) = sender {
                    let _ = sender.send(ControlMessage::Abort { flow });
                }
            }
            let _ = io.abort_forward(watch);
            runtime.state.lock().dial_watches.remove(&watch);
        }
        if let Some(vsock) = vsock {
            io.close(vsock.into()).await;
        }
        remove_dial(&runtime, cid, flow);
    }
    tracing::info!(name: "guest_dial.failed", cid = %cid, flow = flow.get(), purpose = purpose.as_str(), result = ?result);
    result
}

fn cancellation_error(runtime: &OwnerRuntime, cid: GuestCid, flow: FlowId) -> GuestDialError {
    let mut state = runtime.state.lock();
    let admission = state.admission;
    if let Some(entry) = state.cids.get_mut(&cid) {
        if let Some(error) = entry.dial_outcomes.remove(&flow) {
            return error;
        }
        if entry.phase == CidPhase::Retiring {
            return GuestDialError::NotServing;
        }
        if admission == super::GuestAdmission::Quiesced {
            return GuestDialError::Quiesced;
        }
        if entry.session.is_none() {
            return GuestDialError::NoSession;
        }
    }
    drop(state);
    GuestDialError::PairingDeadline
}

fn remove_dial(runtime: &OwnerRuntime, cid: GuestCid, flow: FlowId) {
    let mut state = runtime.state.lock();
    let watch = state.cids.get_mut(&cid).and_then(|entry| {
        let dial = entry.dials.remove(&flow);
        entry.dial_replies.remove(&flow);
        entry.dial_cancels.remove(&flow);
        entry.dial_outcomes.remove(&flow);
        entry.dial_requests.remove(&flow);
        dial.and_then(|dial| match dial.phase {
            DialPhase::AwaitingReply => None,
            DialPhase::Paired { watch, .. } => Some(watch),
        })
    });
    if let Some(watch) = watch {
        state.dial_watches.remove(&watch);
    }
}

fn errno_from_io(error: &std::io::Error) -> KernelErrno {
    let raw = error
        .raw_os_error()
        .and_then(|raw| u16::try_from(raw).ok())
        .filter(|raw| *raw != 0)
        .unwrap_or(104);
    KernelErrno::new(raw).unwrap_or(KernelErrno::ECONNRESET)
}

fn errno_from_forwarder(error: ForwarderError) -> KernelErrno {
    match error {
        ForwarderError::Io { source, .. } | ForwarderError::DeviceAbsent { source } => {
            errno_from_io(&source)
        }
        ForwarderError::Probe { source, .. } => errno_from_forwarder(*source),
        _ => KernelErrno::ECONNRESET,
    }
}

struct OwnerObservedStream {
    stream: GuestStream,
    runtime: Arc<OwnerRuntime>,
    cid: GuestCid,
    flow: FlowId,
    watch: overdrive_core::guest_transport::forwarder::ForwardId,
    alloc: overdrive_core::id::AllocationId,
}

impl std::fmt::Debug for OwnerObservedStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnerObservedStream")
            .field("flow", &self.flow)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl GuestStreamIo for OwnerObservedStream {
    async fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let result = self.stream.read(buffer).await;
        self.refresh_end();
        result
    }

    async fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.stream.write(buffer).await
    }

    async fn shutdown_write(&mut self) -> std::io::Result<()> {
        let result = self.stream.shutdown_write().await;
        if result.is_ok()
            && let Some(entry) = self.runtime.state.lock().cids.get_mut(&self.cid)
            && let Some(dial) = entry.dials.get_mut(&self.flow)
            && let DialPhase::Paired { host_shut, .. } = &mut dial.phase
        {
            *host_shut = true;
        }
        result
    }

    fn end(&self) -> Option<GuestStreamEnd> {
        self.stream.end()
    }
}

impl OwnerObservedStream {
    fn refresh_end(&self) {
        let Some(end) = self.stream.end() else { return };
        if let Some(entry) = self.runtime.state.lock().cids.get_mut(&self.cid)
            && let Some(dial) = entry.dials.get_mut(&self.flow)
            && let DialPhase::Paired { guest_end, .. } = &mut dial.phase
        {
            *guest_end = Some(end);
        }
    }
}

impl Drop for OwnerObservedStream {
    fn drop(&mut self) {
        let (phase, existed) = {
            let mut state = self.runtime.state.lock();
            let Some(entry) = state.cids.get_mut(&self.cid) else { return };
            let phase = entry.dials.remove(&self.flow).map(|dial| dial.phase);
            entry.dial_replies.remove(&self.flow);
            entry.dial_cancels.remove(&self.flow);
            entry.dial_requests.remove(&self.flow);
            state.dial_watches.remove(&self.watch);
            let existed = phase.is_some();
            drop(state);
            (phase, existed)
        };
        if existed {
            let cause = match phase {
                Some(DialPhase::Paired { guest_end: Some(GuestStreamEnd::HalfClose), .. }) => {
                    FlowCloseCause::PeerEof
                }
                _ => FlowCloseCause::StreamDropped,
            };
            tracing::info!(name: "guest_flow.closed", alloc = %self.alloc, cid = %self.cid, kind = "host_dial", cause = cause.as_str());
        }
    }
}

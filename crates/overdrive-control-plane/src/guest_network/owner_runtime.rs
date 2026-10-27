//! Async tasks and per-connection protocol handling for the host guest-flow
//! owner. All socket and forwarder effects go through `GuestFlowIo`.

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::guest_transport::dial::GuestDialError;
use overdrive_core::guest_transport::forwarder::{
    AbortCause, AbortResult, ForwardEnd, ForwardOutcome, PairSide,
};
use overdrive_core::guest_transport::wire::{
    CONTROL_MESSAGE_LEN, CONTROL_OPEN_LEN, ControlMessage, ControlOpen, FLOW_REQUEST_DEADLINE,
    FLOW_REQUEST_LEN, FlowId, FlowOrigin, FlowRefusal, FlowRequest, GUEST_FLOW_MAX_PAIRS,
};
use overdrive_core::guest_transport::{GuestCid, LaunchToken};
use overdrive_core::id::AllocationId;
use overdrive_core::traits::mtls_resolve::MtlsResolution;
use overdrive_worker::mtls_intercept_worker::ForwardedOutbound;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::flow_io::{
    FlowIoError, FlowListener, GuestFlowIo, HostRouteType, PairMode, PeerProtocol, PeerSocketSpec,
    RecordSocket,
};
use super::flow_owner::{
    CidPhase, DialPhase, FlowTask, GuestAdmission, GuestFlowPhase, OwnedPair, OwnerRuntime,
    OwnerState, OwnerTasks,
};
use super::{FlowCloseCause, GuestConnKey, HostSharedGuestNetworkOwner};

struct FlowAttribution {
    launch: LaunchToken,
    session: GuestConnKey,
    sender: mpsc::UnboundedSender<ControlMessage>,
}

enum FlowTaskStart {
    Admitted(FlowAttribution),
    Refused { attribution: FlowAttribution, refusal: FlowRefusal },
}

struct RefusalReaderContext {
    key: GuestConnKey,
    runtime: Arc<OwnerRuntime>,
    io: Arc<dyn GuestFlowIo>,
    clock: Arc<dyn overdrive_core::traits::clock::Clock>,
    attribution: FlowAttribution,
    cancel: CancellationToken,
}

pub(super) fn start(owner: &HostSharedGuestNetworkOwner) {
    if owner.runtime.tasks.lock().is_some() {
        return;
    }
    let shutdown = CancellationToken::new();
    let stream_accept = tokio::spawn(accept_loop(
        FlowListener::Stream,
        Arc::clone(&owner.runtime),
        Arc::clone(&owner.io),
        Arc::clone(&owner.ports.clock),
        Arc::clone(&owner.ports.resolve),
        Arc::clone(&owner.ports.mtls),
        owner.ports.node_prefix,
        owner.ports.gateway,
        shutdown.clone(),
    ));
    let control_accept = tokio::spawn(accept_loop(
        FlowListener::Control,
        Arc::clone(&owner.runtime),
        Arc::clone(&owner.io),
        Arc::clone(&owner.ports.clock),
        Arc::clone(&owner.ports.resolve),
        Arc::clone(&owner.ports.mtls),
        owner.ports.node_prefix,
        owner.ports.gateway,
        shutdown.clone(),
    ));
    let consume_ends = tokio::spawn(consume_ends(
        Arc::clone(&owner.runtime),
        Arc::clone(&owner.io),
        shutdown.clone(),
    ));
    *owner.runtime.tasks.lock() = Some(OwnerTasks {
        shutdown,
        stream_accept: Some(stream_accept),
        control_accept: Some(control_accept),
        consume_ends: Some(consume_ends),
    });
}

pub(super) fn restart_listener(owner: &HostSharedGuestNetworkOwner, listener: FlowListener) {
    let mut tasks_guard = owner.runtime.tasks.lock();
    let Some(tasks) = tasks_guard.as_mut() else {
        drop(tasks_guard);
        start(owner);
        return;
    };
    if tasks.shutdown.is_cancelled() {
        return;
    }
    let shutdown = tasks.shutdown.clone();
    let task_slot = match listener {
        FlowListener::Stream => &mut tasks.stream_accept,
        FlowListener::Control => &mut tasks.control_accept,
    };
    if !task_slot.as_ref().is_some_and(tokio::task::JoinHandle::is_finished) {
        return;
    }
    let replacement = accept_task(owner, listener, shutdown);
    *task_slot = Some(replacement);
    drop(tasks_guard);
}

pub(super) async fn shutdown(owner: &HostSharedGuestNetworkOwner) {
    let root_tasks = {
        let mut tasks_guard = owner.runtime.tasks.lock();
        let Some(tasks) = tasks_guard.as_mut() else { return };
        tasks.shutdown.cancel();
        let root_tasks =
            [tasks.stream_accept.take(), tasks.control_accept.take(), tasks.consume_ends.take()];
        drop(tasks_guard);
        root_tasks
    };
    for task in root_tasks.into_iter().flatten() {
        let _ = task.await;
    }

    let (child_tasks, refusal_reader_cancels, refusal_reader_keys) = {
        let mut state = owner.runtime.state.lock();
        let mut child_tasks = Vec::new();
        let mut refusal_reader_cancels = Vec::new();
        let mut refusal_reader_keys = Vec::new();

        for (cid, entry) in &mut state.cids {
            for task in std::mem::take(&mut entry.control_tasks).into_values() {
                task.cancel.cancel();
                if let Some(task) = task.task {
                    child_tasks.push(task);
                }
            }

            let drained = entry.take_flow_tasks_for_drain();
            for (key, cancel) in drained.refusal_readers {
                refusal_reader_cancels.push(cancel);
                refusal_reader_keys.push((*cid, key));
            }
            for mut task in drained.tasks {
                task.cancel.cancel();
                if let Some(task) = task.task.take() {
                    child_tasks.push(task);
                }
            }
        }

        drop(state);
        (child_tasks, refusal_reader_cancels, refusal_reader_keys)
    };

    for cancel in refusal_reader_cancels {
        cancel.cancel();
    }
    for task in child_tasks {
        let _ = task.await;
    }
    super::wait_for_drained_refusal_readers(&owner.runtime, &refusal_reader_keys).await;
}

fn accept_task(
    owner: &HostSharedGuestNetworkOwner,
    listener: FlowListener,
    shutdown: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(accept_loop(
        listener,
        Arc::clone(&owner.runtime),
        Arc::clone(&owner.io),
        Arc::clone(&owner.ports.clock),
        Arc::clone(&owner.ports.resolve),
        Arc::clone(&owner.ports.mtls),
        owner.ports.node_prefix,
        owner.ports.gateway,
        shutdown,
    ))
}

#[allow(
    clippy::too_many_arguments,
    reason = "the owner accept loop passes the accepted listener and its required composition ports directly"
)]
async fn accept_loop(
    listener: FlowListener,
    runtime: Arc<OwnerRuntime>,
    io: Arc<dyn GuestFlowIo>,
    clock: Arc<dyn overdrive_core::traits::clock::Clock>,
    resolve: Arc<dyn overdrive_core::traits::mtls_resolve::MtlsResolve>,
    mtls: Arc<overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker>,
    node_prefix: ipnet::Ipv4Net,
    gateway: Ipv4Addr,
    shutdown: CancellationToken,
) {
    loop {
        if listener == FlowListener::Stream
            && !wait_for_refusal_reader_capacity(&runtime, &shutdown).await
        {
            return;
        }
        let accepted = tokio::select! {
            biased;
            () = shutdown.cancelled() => return,
            accepted = io.accept(listener) => accepted,
        };
        if let Ok((socket, key)) = accepted {
            if listener == FlowListener::Control {
                spawn_control(socket, key, &runtime, &io, &shutdown).await;
            } else {
                spawn_flow(
                    socket,
                    key,
                    &runtime,
                    &io,
                    &clock,
                    &resolve,
                    &mtls,
                    node_prefix,
                    gateway,
                    &shutdown,
                )
                .await;
            }
        } else {
            runtime.state.lock().flow_listeners_running = false;
            return;
        }
    }
}

async fn spawn_control(
    socket: RecordSocket,
    key: GuestConnKey,
    runtime: &Arc<OwnerRuntime>,
    io: &Arc<dyn GuestFlowIo>,
    shutdown: &CancellationToken,
) {
    let cid = key.peer_cid;
    let accepted_launch = {
        let mut state = runtime.state.lock();
        state.cids.get_mut(&cid).filter(|entry| entry.phase != CidPhase::Retiring).map(|entry| {
            entry.unread.insert(key);
            entry.launch
        })
    };
    let Some(accepted_launch) = accepted_launch else {
        io.close(socket.into()).await;
        return;
    };

    let cancel = shutdown.child_token();
    let task_cancel = cancel.clone();
    let (ready_tx, ready_rx) = oneshot::channel();
    let task_runtime = Arc::clone(runtime);
    let task_io = Arc::clone(io);
    let mut task = Some(tokio::spawn(async move {
        if ready_rx.await.is_ok() {
            control_session(socket, key, task_runtime, task_io, task_cancel, accepted_launch).await;
        } else {
            task_io.close(socket.into()).await;
        }
    }));
    let registered = {
        let mut state = runtime.state.lock();
        if let Some(entry) = state.cids.get_mut(&cid)
            && entry.launch == accepted_launch
        {
            entry.control_tasks.insert(
                key,
                FlowTask {
                    cancel,
                    task: task.take(),
                    owner_launch: Some(accepted_launch),
                    flow: None,
                },
            );
            true
        } else {
            false
        }
    };
    if registered {
        let _ = ready_tx.send(());
    } else {
        drop(ready_tx);
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one control-session loop owns decode, dispatch and session-loss cleanup ordering"
)]
async fn control_session(
    socket: RecordSocket,
    key: GuestConnKey,
    runtime: Arc<OwnerRuntime>,
    io: Arc<dyn GuestFlowIo>,
    shutdown: CancellationToken,
    accepted_launch: LaunchToken,
) {
    let cid = key.peer_cid;
    let mut open = [0_u8; CONTROL_OPEN_LEN];
    let read = tokio::select! {
        biased;
        () = shutdown.cancelled() => Err(io::Error::from(io::ErrorKind::Interrupted)),
        result = read_fixed(io.as_ref(), &socket, &mut open, Duration::from_secs(2), Arc::clone(&runtime)) => result,
    };
    let Ok(()) = read else {
        io.close(socket.into()).await;
        finish_connection(&runtime, cid, key, false, accepted_launch);
        return;
    };
    let control_open = <&[u8; CONTROL_OPEN_LEN]>::try_from(open.as_slice())
        .ok()
        .and_then(|bytes| ControlOpen::decode(bytes).ok());
    let Some(control_open) = control_open else {
        tracing::info!(name: "guest_flow.connection_closed_unattributed", cid = %cid, port = key.listener.as_u32(), cause = FlowCloseCause::ControlMalformed.as_str());
        io.close(socket.into()).await;
        finish_connection(&runtime, cid, key, false, accepted_launch);
        return;
    };

    let (tx, mut rx) = mpsc::unbounded_channel::<ControlMessage>();
    let claim = {
        let mut state = runtime.state.lock();
        if let Some(entry) = state.cids.get_mut(&cid) {
            let task_belongs_to_launch = entry
                .control_tasks
                .get(&key)
                .is_some_and(|task| task.owner_launch == Some(accepted_launch));
            if task_belongs_to_launch {
                entry.unread.remove(&key);
            }
            if accepted_launch != entry.launch || control_open.launch != entry.launch {
                Some(Err(FlowCloseCause::StaleLaunch))
            } else if entry.phase == CidPhase::Retiring || entry.session.is_some() {
                Some(Err(FlowCloseCause::NoControlSession))
            } else {
                entry.session = Some(key);
                entry.session_sender = Some(tx);
                entry.session_cancel = Some(shutdown.clone());
                Some(Ok(()))
            }
        } else {
            Some(Err(FlowCloseCause::NoControlSession))
        }
    };
    match claim {
        Some(Ok(())) => {}
        Some(Err(cause)) => {
            tracing::info!(name: "guest_flow.connection_closed_unattributed", cid = %cid, port = key.listener.as_u32(), cause = cause.as_str());
            io.close(socket.into()).await;
            finish_connection(&runtime, cid, key, false, accepted_launch);
            return;
        }
        None => unreachable!("control session claim always resolves"),
    }

    loop {
        let incoming = tokio::select! {
            biased;
            () = shutdown.cancelled() => None,
            message = rx.recv() => {
                if let Some(message) = message {
                    if io.write_record(&socket, &message.encode()).await.is_err() { None } else { Some(None) }
                } else { Some(None) }
            }
            result = read_fixed_optional(io.as_ref(), &socket, CONTROL_MESSAGE_LEN) => {
                match result { Ok(Some(bytes)) => Some(Some(bytes)), _ => None }
            }
        };
        match incoming {
            Some(Some(bytes)) => {
                let message = <&[u8; CONTROL_MESSAGE_LEN]>::try_from(bytes.as_slice())
                    .ok()
                    .and_then(|bytes| ControlMessage::decode(bytes).ok());
                match message {
                    Some(ControlMessage::Abort { flow }) => {
                        if !abort_guest_flow(
                            &runtime,
                            &io,
                            cid,
                            flow,
                            FlowCloseCause::AbortReceived,
                        ) {
                            discard_control(cid, flow, "Abort");
                        }
                    }
                    Some(message @ ControlMessage::Paired { flow }) => {
                        if let Some(sender) = dial_reply_sender(&runtime, cid, flow) {
                            let _ = sender.send(message);
                        } else {
                            discard_control(cid, flow, "Paired");
                        }
                    }
                    Some(message @ ControlMessage::Refused { flow, .. }) => {
                        if let Some(sender) = dial_reply_sender(&runtime, cid, flow) {
                            let _ = sender.send(message);
                        } else {
                            discard_control(cid, flow, "Refused");
                        }
                    }
                    Some(ControlMessage::TimedOut { flow, errno }) => {
                        if let Some(sender) = dial_reply_sender(&runtime, cid, flow) {
                            let _ = sender.send(ControlMessage::TimedOut { flow, errno });
                        } else {
                            discard_control(cid, flow, "TimedOut");
                        }
                    }
                    None => {
                        tracing::info!(name: "guest_flow.closed", cid = %cid, alloc = %allocation_for(&runtime, cid), kind = "control", cause = FlowCloseCause::ControlMalformed.as_str());
                        break;
                    }
                }
            }
            Some(None) => {}
            None => break,
        }
    }
    let active = {
        let state = runtime.state.lock();
        state
            .cids
            .get(&cid)
            .is_some_and(|entry| entry.launch == accepted_launch && entry.session == Some(key))
    };
    if active {
        session_lost(&runtime, &io, cid, key, FlowCloseCause::ControlSessionLost);
    }
    io.close(socket.into()).await;
    finish_connection(&runtime, cid, key, active, accepted_launch);
}

#[allow(
    clippy::too_many_arguments,
    reason = "the accepted listener owner passes its driven ports directly to the tracked flow task"
)]
async fn spawn_flow(
    socket: RecordSocket,
    key: GuestConnKey,
    runtime: &Arc<OwnerRuntime>,
    io: &Arc<dyn GuestFlowIo>,
    clock: &Arc<dyn overdrive_core::traits::clock::Clock>,
    resolve: &Arc<dyn overdrive_core::traits::mtls_resolve::MtlsResolve>,
    mtls: &Arc<overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker>,
    node_prefix: ipnet::Ipv4Net,
    gateway: Ipv4Addr,
    shutdown: &CancellationToken,
) {
    let cid = key.peer_cid;
    let cancel = shutdown.child_token();
    let task_cancel = cancel.clone();
    let (ready_tx, ready_rx) = oneshot::channel::<FlowTaskStart>();
    let task_runtime = Arc::clone(runtime);
    let cleanup_runtime = Arc::clone(runtime);
    let task_io = Arc::clone(io);
    let task_clock = Arc::clone(clock);
    let task_resolve = Arc::clone(resolve);
    let task_mtls = Arc::clone(mtls);
    let mut task = Some(tokio::spawn(async move {
        match ready_rx.await {
            Ok(FlowTaskStart::Admitted(attribution)) => {
                handle_flow(
                    socket,
                    key,
                    task_runtime,
                    task_io,
                    task_clock,
                    task_resolve,
                    task_mtls,
                    node_prefix,
                    gateway,
                    attribution,
                    task_cancel,
                )
                .await;
            }
            Ok(FlowTaskStart::Refused { attribution, refusal }) => {
                handle_refusal_reader(
                    socket,
                    RefusalReaderContext {
                        key,
                        runtime: task_runtime,
                        io: task_io,
                        clock: task_clock,
                        attribution,
                        cancel: task_cancel,
                    },
                    refusal,
                )
                .await;
            }
            Err(_) => task_io.close(socket.into()).await,
        }
        finish_flow_task(&cleanup_runtime, cid, key);
    }));

    let (start, no_live_session) =
        register_flow_task(runtime, io.as_ref(), cid, key, &cancel, &mut task);
    if let Some(start) = start {
        let _ = ready_tx.send(start);
    } else {
        if no_live_session {
            tracing::info!(
                name: "guest_flow.connection_closed_unattributed",
                cid = %cid,
                port = key.listener.as_u32(),
                cause = FlowCloseCause::NoControlSession.as_str()
            );
        }
        drop(ready_tx);
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}

fn register_flow_task(
    runtime: &Arc<OwnerRuntime>,
    io: &dyn GuestFlowIo,
    cid: GuestCid,
    key: GuestConnKey,
    cancel: &CancellationToken,
    task: &mut Option<tokio::task::JoinHandle<()>>,
) -> (Option<FlowTaskStart>, bool) {
    let mut state = runtime.state.lock();
    let attribution = state.cids.get(&cid).and_then(|entry| {
        if entry.phase == CidPhase::Retiring {
            return None;
        }
        let session = entry.session?;
        let sender = entry.session_sender.clone()?;
        Some(FlowAttribution { launch: entry.launch, session, sender })
    });
    let result = match attribution {
        None => (None, true),
        Some(_)
            if state.cids.get(&cid).is_some_and(|entry| entry.flow_tasks.contains_key(&key)) =>
        {
            (None, false)
        }
        Some(attribution) => {
            let refusal = admission_refusal(&state, cid, io);
            if let Some(refusal) = refusal {
                if state.refusal_reader_count() >= GUEST_FLOW_MAX_PAIRS {
                    // The sole stream accept loop checks this predicate before
                    // accepting, so an accepted socket always has a bounded
                    // reader slot available here.
                    unreachable!("stream accept is serialized behind refusal-reader capacity");
                }
                state.insert_flow_task(
                    cid,
                    key,
                    FlowTask {
                        cancel: cancel.clone(),
                        task: task.take(),
                        owner_launch: Some(attribution.launch),
                        flow: None,
                    },
                );
                (Some(FlowTaskStart::Refused { attribution, refusal }), false)
            } else {
                state
                    .cids
                    .get_mut(&cid)
                    .unwrap_or_else(|| unreachable!("the attributed CID remains under the lock"))
                    .guest_flows
                    .insert(
                        key,
                        GuestFlowPhase::Registered { request: super::RequestRead::Nothing },
                    );
                state.insert_flow_task(
                    cid,
                    key,
                    FlowTask {
                        cancel: cancel.clone(),
                        task: task.take(),
                        owner_launch: None,
                        flow: None,
                    },
                );
                (Some(FlowTaskStart::Admitted(attribution)), false)
            }
        }
    };
    drop(state);
    result
}

async fn wait_for_refusal_reader_capacity(
    runtime: &Arc<OwnerRuntime>,
    shutdown: &CancellationToken,
) -> bool {
    loop {
        let notified = runtime.refusal_reader_capacity.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        if runtime.state.lock().refusal_reader_count() < GUEST_FLOW_MAX_PAIRS {
            return true;
        }

        tokio::select! {
            biased;
            () = shutdown.cancelled() => return false,
            () = notified => {}
        }
    }
}

fn admission_refusal(
    state: &OwnerState,
    cid: GuestCid,
    io: &dyn GuestFlowIo,
) -> Option<FlowRefusal> {
    let entry = state
        .cids
        .get(&cid)
        .unwrap_or_else(|| unreachable!("admission is checked only for a tracked CID"));
    if entry.phase != CidPhase::Active {
        return Some(FlowRefusal::NotActive);
    }
    if state.admission == GuestAdmission::Quiesced {
        return Some(FlowRefusal::Quiesced);
    }
    let per_allocation_full = entry.held_flows()
        >= usize::try_from(super::GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION).unwrap_or(usize::MAX);
    let node_full = io.inventory().map_or(true, |inventory| {
        let forwarder_places = inventory.live_pairs.saturating_add(inventory.live_watches);
        state
            .occupied_node_flow_places()
            .max(usize::try_from(forwarder_places).unwrap_or(usize::MAX))
            >= usize::try_from(GUEST_FLOW_MAX_PAIRS).unwrap_or(usize::MAX)
    });
    (per_allocation_full || node_full).then_some(FlowRefusal::Capacity)
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "one flow task owns request decoding, admission, pairing and refusal cleanup ordering"
)]
async fn handle_flow(
    socket: RecordSocket,
    key: GuestConnKey,
    runtime: Arc<OwnerRuntime>,
    io: Arc<dyn GuestFlowIo>,
    clock: Arc<dyn overdrive_core::traits::clock::Clock>,
    resolve: Arc<dyn overdrive_core::traits::mtls_resolve::MtlsResolve>,
    mtls: Arc<overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker>,
    node_prefix: ipnet::Ipv4Net,
    gateway: Ipv4Addr,
    attribution: FlowAttribution,
    cancel: CancellationToken,
) {
    let cid = key.peer_cid;
    let sender = &attribution.sender;
    let mut bytes = Vec::with_capacity(FLOW_REQUEST_LEN);
    let deadline = clock.sleep(FLOW_REQUEST_DEADLINE);
    tokio::pin!(deadline);
    let mut failed = false;
    loop {
        if bytes.len() == FLOW_REQUEST_LEN {
            break;
        }
        let remaining = FLOW_REQUEST_LEN - bytes.len();
        let read = tokio::select! {
            biased;
            () = cancel.cancelled() => { failed = true; break; },
            () = &mut deadline => { failed = true; break; },
            result = io.read_record(&socket, remaining) => result,
        };
        match read {
            Ok(chunk) if chunk.is_empty() => {
                failed = true;
                break;
            }
            Ok(chunk) => {
                bytes.extend_from_slice(&chunk);
                if bytes.len() < FLOW_REQUEST_LEN {
                    set_request_read(&runtime, cid, key, true);
                }
            }
            Err(_) => {
                failed = true;
                break;
            }
        }
    }
    if failed {
        remove_guest_flow(&runtime, cid, key);
        io.close(socket.into()).await;
        return;
    }
    let request_bytes: &[u8; FLOW_REQUEST_LEN] = bytes
        .as_slice()
        .try_into()
        .unwrap_or_else(|_| unreachable!("the fixed-length flow request was fully read"));
    let decoded = FlowRequest::decode(request_bytes);
    let request = match decoded {
        Ok(request) => request,
        Err(error) => {
            let raw_flow = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            let origin =
                if raw_flow & (1 << 31) == 0 { FlowOrigin::Guest } else { FlowOrigin::Host };
            let flow = FlowId::new(origin, raw_flow & !(1 << 31))
                .unwrap_or_else(|_| unreachable!("masked flow sequence is in range"));
            let refusal = if matches!(
                error,
                overdrive_core::guest_transport::wire::FlowWireError::UnsupportedVersion { .. }
            ) {
                FlowRefusal::Unsupported
            } else {
                FlowRefusal::Malformed
            };
            let token_bytes: [u8; 16] = bytes[16..32]
                .try_into()
                .unwrap_or_else(|_| unreachable!("request holds its launch bytes"));
            let token = LaunchToken::from_bytes(token_bytes).ok();
            if token == Some(attribution.launch) {
                send_flow_refusal_if_current(
                    &runtime,
                    key,
                    &attribution,
                    &cancel,
                    false,
                    ControlMessage::Refused { flow, refusal },
                );
            }
            remove_guest_flow(&runtime, cid, key);
            io.close(socket.into()).await;
            return;
        }
    };
    let flow = request.flow();
    let status = {
        let state = runtime.state.lock();
        if request.launch() != attribution.launch {
            Err(FlowCloseCause::StaleLaunch)
        } else if !flow_task_session_is_current(&state, key, &attribution, &cancel, false) {
            Err(FlowCloseCause::NoControlSession)
        } else {
            Ok(())
        }
    };
    if let Err(cause) = status {
        tracing::info!(name: "guest_flow.connection_closed_unattributed", cid = %cid, port = key.listener.as_u32(), cause = cause.as_str());
        remove_guest_flow(&runtime, cid, key);
        io.close(socket.into()).await;
        return;
    }

    let alloc =
        runtime.state.lock().cids.get(&cid).map_or_else(
            || unreachable!("the checked CID remains held"),
            |entry| entry.alloc.clone(),
        );

    let kind = request.kind();
    let datagram = matches!(&request, FlowRequest::DatagramStream { .. });
    if datagram {
        if !set_datagram_awaiting_paired(&runtime, cid, key, flow) {
            remove_guest_flow(&runtime, cid, key);
            io.close(socket.into()).await;
            return;
        }
    } else {
        set_request_attributed(&runtime, cid, key);
    }
    let result = match request {
        FlowRequest::TcpConnect { destination, .. } => {
            tcp_connect(
                &runtime,
                io.as_ref(),
                socket,
                key,
                flow,
                destination,
                &alloc,
                resolve.as_ref(),
                mtls.as_ref(),
                node_prefix,
                gateway,
                &cancel,
            )
            .await
        }
        FlowRequest::DatagramStream { destination, .. } => {
            datagram_associate(
                &runtime,
                io.as_ref(),
                socket,
                key,
                flow,
                destination,
                node_prefix,
                gateway,
                &cancel,
            )
            .await
        }
        FlowRequest::TcpAccept { .. } => {
            io.close(socket.into()).await;
            Err(FlowOutcome::Refused(FlowRefusal::Unsupported))
        }
    };
    if let Ok((pair, mode, forwarded)) = result {
        let (vsock, peer) = (pair.vsock, pair.peer);
        let generation = runtime.state.lock().forwarder_generation;
        {
            let mut state = runtime.state.lock();
            let phase = GuestFlowPhase::Paired { pair: pair.id, generation };
            if let Some(entry) = state.cids.get_mut(&cid) {
                match mode {
                    PairMode::Stream => {
                        entry.guest_flows.insert(key, phase);
                    }
                    PairMode::Datagram { .. } => {
                        entry.guest_flows.remove(&key);
                        entry.datagrams.insert(
                            flow,
                            super::AssociationObservation {
                                phase: super::AssociationPhase::Paired {
                                    pair: pair.id,
                                    generation,
                                },
                            },
                        );
                    }
                }
                entry.flow_ids.insert(flow, pair.id);
            }
            state.forwarder_attached = state.forwarder_attached.saturating_add(1);
            state.pairs.insert(
                pair.id,
                OwnedPair {
                    cid,
                    alloc: alloc.clone(),
                    flow,
                    generation,
                    mode,
                    vsock,
                    peer,
                    _forwarded: forwarded,
                },
            );
        }
        send_on(sender, ControlMessage::Paired { flow });
        tracing::info!(name: "guest_flow.paired", alloc = %alloc, cid = %cid, kind = if mode == PairMode::Stream { "tcp_connect" } else { "datagram" });
    } else {
        match result.err().unwrap_or(FlowOutcome::Refused(FlowRefusal::IntakeUnavailable)) {
            FlowOutcome::Refused(refusal) => {
                send_on(sender, ControlMessage::Refused { flow, refusal });
                remove_guest_flow(&runtime, cid, key);
                remove_datagram(&runtime, cid, flow);
                tracing::info!(name: "guest_flow.refused", alloc = %alloc, cid = %cid, kind = kind.as_str(), refusal = refusal.as_str());
            }
            FlowOutcome::Aborted => {
                send_on(sender, ControlMessage::Abort { flow });
                remove_guest_flow(&runtime, cid, key);
                remove_datagram(&runtime, cid, flow);
                tracing::info!(name: "guest_flow.closed", alloc = %alloc, cid = %cid, kind = kind.as_str(), flow = flow.get(), cause = FlowCloseCause::PeerReset.as_str());
            }
            FlowOutcome::GuestEnded => {
                remove_guest_flow(&runtime, cid, key);
                remove_datagram(&runtime, cid, flow);
                tracing::info!(name: "guest_flow.closed", alloc = %alloc, cid = %cid, kind = kind.as_str(), flow = flow.get(), cause = FlowCloseCause::AbortReceived.as_str());
            }
        }
    }
}

async fn handle_refusal_reader(
    socket: RecordSocket,
    context: RefusalReaderContext,
    admission_refusal: FlowRefusal,
) {
    let RefusalReaderContext { key, runtime, io, clock, attribution, cancel } = context;
    let mut bytes = Vec::with_capacity(FLOW_REQUEST_LEN);
    let deadline = clock.sleep(FLOW_REQUEST_DEADLINE);
    tokio::pin!(deadline);
    while bytes.len() < FLOW_REQUEST_LEN {
        let remaining = FLOW_REQUEST_LEN - bytes.len();
        let read = tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            () = &mut deadline => break,
            result = io.read_record(&socket, remaining) => result,
        };
        match read {
            Ok(chunk) if !chunk.is_empty() => bytes.extend_from_slice(&chunk),
            Ok(_) | Err(_) => break,
        }
    }

    if bytes.len() == FLOW_REQUEST_LEN && !cancel.is_cancelled() {
        let request_bytes: &[u8; FLOW_REQUEST_LEN] = bytes
            .as_slice()
            .try_into()
            .unwrap_or_else(|_| unreachable!("the fixed-length refusal request was fully read"));
        let response = match FlowRequest::decode(request_bytes) {
            Ok(request) if request.launch() == attribution.launch => {
                Some((request.flow(), admission_refusal))
            }
            Ok(_) => {
                tracing::info!(
                    name: "guest_flow.connection_closed_unattributed",
                    cid = %key.peer_cid,
                    port = key.listener.as_u32(),
                    cause = FlowCloseCause::StaleLaunch.as_str()
                );
                None
            }
            Err(error) => {
                let raw_flow = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                let origin =
                    if raw_flow & (1 << 31) == 0 { FlowOrigin::Guest } else { FlowOrigin::Host };
                let flow = FlowId::new(origin, raw_flow & !(1 << 31))
                    .unwrap_or_else(|_| unreachable!("masked flow sequence is in range"));
                let refusal = if matches!(
                    error,
                    overdrive_core::guest_transport::wire::FlowWireError::UnsupportedVersion { .. }
                ) {
                    FlowRefusal::Unsupported
                } else {
                    FlowRefusal::Malformed
                };
                let token_bytes: [u8; 16] = bytes[16..32]
                    .try_into()
                    .unwrap_or_else(|_| unreachable!("request holds its launch bytes"));
                match LaunchToken::from_bytes(token_bytes).ok() {
                    Some(token) if token == attribution.launch => Some((flow, refusal)),
                    Some(_) => {
                        tracing::info!(
                            name: "guest_flow.connection_closed_unattributed",
                            cid = %key.peer_cid,
                            port = key.listener.as_u32(),
                            cause = FlowCloseCause::StaleLaunch.as_str()
                        );
                        None
                    }
                    None => None,
                }
            }
        };
        if let Some((flow, refusal)) = response {
            send_flow_refusal_if_current(
                &runtime,
                key,
                &attribution,
                &cancel,
                true,
                ControlMessage::Refused { flow, refusal },
            );
        }
    }
    io.close(socket.into()).await;
}

fn send_flow_refusal_if_current(
    runtime: &Arc<OwnerRuntime>,
    key: GuestConnKey,
    attribution: &FlowAttribution,
    cancel: &CancellationToken,
    refusal_reader: bool,
    message: ControlMessage,
) {
    let state = runtime.state.lock();
    if flow_task_session_is_current(&state, key, attribution, cancel, refusal_reader) {
        let _ = attribution.sender.send(message);
    }
}

fn flow_task_session_is_current(
    state: &OwnerState,
    key: GuestConnKey,
    attribution: &FlowAttribution,
    cancel: &CancellationToken,
    refusal_reader: bool,
) -> bool {
    let cid = key.peer_cid;
    if cancel.is_cancelled() {
        return false;
    }
    let Some(entry) = state.cids.get(&cid) else { return false };
    if entry.phase == CidPhase::Retiring
        || entry.launch != attribution.launch
        || entry.session != Some(attribution.session)
        || !entry
            .session_sender
            .as_ref()
            .is_some_and(|sender| sender.same_channel(&attribution.sender))
    {
        return false;
    }
    let Some(task) = entry.flow_tasks.get(&key) else { return false };
    if task.cancel.is_cancelled() || task.task.is_none() {
        return false;
    }
    if refusal_reader { task.is_refusal_reader() } else { task.owner_launch.is_none() }
}

enum FlowOutcome {
    Refused(FlowRefusal),
    Aborted,
    GuestEnded,
}

impl From<FlowRefusal> for FlowOutcome {
    fn from(refusal: FlowRefusal) -> Self {
        Self::Refused(refusal)
    }
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "TCP-connect admission, leg-S setup, peer routing, connect and attach share one ordered flow outcome"
)]
async fn tcp_connect(
    runtime: &Arc<OwnerRuntime>,
    io: &dyn GuestFlowIo,
    vsock: RecordSocket,
    key: GuestConnKey,
    flow: overdrive_core::guest_transport::wire::FlowId,
    destination: SocketAddrV4,
    alloc: &AllocationId,
    resolve: &dyn overdrive_core::traits::mtls_resolve::MtlsResolve,
    mtls: &overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker,
    node_prefix: ipnet::Ipv4Net,
    gateway: Ipv4Addr,
    cancel: &CancellationToken,
) -> Result<(super::flow_io::AttachedPair, PairMode, Option<ForwardedOutbound>), FlowOutcome> {
    let mut vsock = Some(vsock);
    let result = async {
        let address = *destination.ip();
        if static_policy(address, false, node_prefix, gateway, destination.port()) {
            return Err(FlowOutcome::Refused(FlowRefusal::Policy));
        }
        if static_host_internal(address) {
            return Err(FlowOutcome::Refused(FlowRefusal::HostInternal));
        }
        let resolution = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(FlowOutcome::Refused(FlowRefusal::IntakeUnavailable)),
            result = resolve.resolve(destination) => result.map_err(|_| FlowRefusal::ResolveFailed)?,
        };
        let (peer_destination, spec, forwarded) = match resolution {
            MtlsResolution::Mesh(backend) => {
                let destination = mtls.leg_f_addr().ok_or(FlowRefusal::IntakeUnavailable)?;
                let spec = PeerSocketSpec {
                    protocol: PeerProtocol::Tcp,
                    owner_egress_mark: false,
                    abortive_linger: false,
                    bind: Some(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                };
                let peer = tokio::select! {
                    biased;
                    () = cancel.cancelled() => return Err(FlowOutcome::Refused(FlowRefusal::IntakeUnavailable)),
                    peer = io.open_peer(spec) => peer.map_err(|_| FlowRefusal::IntakeUnavailable)?,
                };
                let local = io.local_addr(&peer).map_err(|_| FlowRefusal::IntakeUnavailable)?;
                let forwarded = mtls
                    .register_forwarded_outbound(alloc, local, destination, backend)
                    .map_err(|_| FlowRefusal::IntakeUnavailable)?;
                if let Some(entry) = runtime.state.lock().cids.get_mut(&key.peer_cid) {
                    entry.guest_flows.insert(key, GuestFlowPhase::Connecting);
                }
                let connected = tokio::select! {
                    biased;
                    () = cancel.cancelled() => { io.abort(peer.into()).await; return Err(FlowOutcome::Refused(FlowRefusal::IntakeUnavailable)); },
                    result = io.connect_peer(&peer, destination) => result,
                };
                if connected.is_err() {
                    io.abort(peer.into()).await;
                    return Err(FlowOutcome::Refused(FlowRefusal::IntakeUnavailable));
                }
                let Some(vsock) = vsock.take() else {
                    unreachable!("one flow task owns its record socket")
                };
                return match io.attach(vsock, peer, PairMode::Stream) {
                    Ok(pair) => Ok((pair, PairMode::Stream, Some(forwarded))),
                    Err(refused) => {
                        let half_ended = matches!(refused.error, overdrive_core::guest_transport::forwarder::ForwarderError::HalfEnded { .. });
                        if half_ended {
                            {
                                let mut state = runtime.state.lock();
                                state.forwarder_attach_ended = state.forwarder_attach_ended.saturating_add(1);
                            }
                            io.abort(refused.vsock.into()).await;
                            io.abort(refused.peer.into()).await;
                        } else {
                            tracing::error!(name: "guest_flow.attach_refused", cid = %key.peer_cid, flow = flow.get(), error = ?refused.error);
                            io.close(refused.vsock.into()).await;
                            io.close(refused.peer.into()).await;
                        }
                        Err(if half_ended {
                            FlowOutcome::Aborted
                        } else {
                            FlowOutcome::Refused(FlowRefusal::IntakeUnavailable)
                        })
                    }
                };
            }
            MtlsResolution::MeshUnreachable => {
                return Err(FlowOutcome::Refused(FlowRefusal::MeshUnreachable));
            }
            MtlsResolution::NonMesh => (
                destination,
                PeerSocketSpec {
                    protocol: PeerProtocol::Tcp,
                    owner_egress_mark: true,
                    abortive_linger: true,
                    bind: None,
                },
                None,
            ),
        };
        match io.route_type(*peer_destination.ip()).await.map_err(|_| FlowRefusal::HostInternal)? {
            HostRouteType::Unicast => {}
            _ if peer_destination.ip() == &gateway && peer_destination.port() == 53 => {}
            _ => return Err(FlowOutcome::Refused(FlowRefusal::HostInternal)),
        }
        let peer = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(FlowOutcome::Refused(FlowRefusal::IntakeUnavailable)),
            peer = io.open_peer(spec) => peer.map_err(|_| FlowRefusal::DestinationUnreachable)?,
        };
        if let Some(entry) = runtime.state.lock().cids.get_mut(&key.peer_cid) {
            entry.guest_flows.insert(key, GuestFlowPhase::Connecting);
        }
        let connected = tokio::select! {
            biased;
            () = cancel.cancelled() => { io.abort(peer.into()).await; return Err(FlowOutcome::Refused(FlowRefusal::IntakeUnavailable)); },
            result = io.connect_peer(&peer, peer_destination) => result,
        };
        if connected.is_err() {
            io.abort(peer.into()).await;
            return Err(FlowOutcome::Refused(FlowRefusal::DestinationUnreachable));
        }
        let Some(vsock) = vsock.take() else {
            unreachable!("one flow task owns its record socket")
        };
        match io.attach(vsock, peer, PairMode::Stream) {
            Ok(pair) => Ok((pair, PairMode::Stream, forwarded)),
            Err(refused) => {
                let half_ended = matches!(refused.error, overdrive_core::guest_transport::forwarder::ForwarderError::HalfEnded { .. });
                if half_ended {
                    {
                        let mut state = runtime.state.lock();
                        state.forwarder_attach_ended = state.forwarder_attach_ended.saturating_add(1);
                    }
                    io.abort(refused.vsock.into()).await;
                    io.abort(refused.peer.into()).await;
                } else {
                    tracing::error!(name: "guest_flow.attach_refused", cid = %key.peer_cid, flow = flow.get(), error = ?refused.error);
                    io.close(refused.vsock.into()).await;
                    io.close(refused.peer.into()).await;
                }
                Err(if half_ended {
                    FlowOutcome::Aborted
                } else {
                    FlowOutcome::Refused(FlowRefusal::IntakeUnavailable)
                })
            }
        }
    }.await;
    if let Some(vsock) = vsock
        && result.is_err()
    {
        io.close(vsock.into()).await;
    }
    let _ = (key, flow);
    result
}

#[allow(
    clippy::too_many_arguments,
    reason = "the datagram owner step uses the same explicit listener, policy, peer and cancellation ports"
)]
async fn datagram_associate(
    runtime: &Arc<OwnerRuntime>,
    io: &dyn GuestFlowIo,
    vsock: RecordSocket,
    key: GuestConnKey,
    flow: overdrive_core::guest_transport::wire::FlowId,
    destination: SocketAddrV4,
    node_prefix: ipnet::Ipv4Net,
    gateway: Ipv4Addr,
    cancel: &CancellationToken,
) -> Result<(super::flow_io::AttachedPair, PairMode, Option<ForwardedOutbound>), FlowOutcome> {
    let mut vsock = Some(vsock);
    let result = async {
        let address = *destination.ip();
        if static_policy(address, true, node_prefix, gateway, destination.port()) {
            return Err(FlowOutcome::Refused(FlowRefusal::Policy));
        }
        if static_host_internal(address) {
            return Err(FlowOutcome::Refused(FlowRefusal::HostInternal));
        }
        let vip = runtime_vip_contains(runtime, address);
        if !vip {
            let route_type = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    return Err(pending_datagram_cancel_outcome(runtime, key.peer_cid, flow));
                }
                result = io.route_type(address) => result.map_err(|_| FlowRefusal::HostInternal)?,
            };
            match route_type {
                HostRouteType::Unicast => {}
                _ if address == gateway && destination.port() == 53 => {}
                _ => return Err(FlowOutcome::Refused(FlowRefusal::HostInternal)),
            }
        }
        let peer = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                return Err(pending_datagram_cancel_outcome(runtime, key.peer_cid, flow));
            }
            peer = io.open_peer(PeerSocketSpec {
                protocol: PeerProtocol::Udp,
                owner_egress_mark: !(vip || address == gateway && destination.port() == 53),
                abortive_linger: true,
                bind: None,
            }) => peer.map_err(|_| FlowRefusal::DestinationUnreachable)?,
        };
        let connected = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                io.abort(peer.into()).await;
                return Err(pending_datagram_cancel_outcome(runtime, key.peer_cid, flow));
            },
            result = io.connect_udp(&peer, destination) => result,
        };
        if connected.is_err() {
            io.abort(peer.into()).await;
            return Err(FlowOutcome::Refused(FlowRefusal::DestinationUnreachable));
        }
        let kernel_peer = io.peer_addr(&peer).map_err(|_| FlowRefusal::DestinationUnreachable)?;
        if node_prefix.contains(kernel_peer.ip()) && !(kernel_peer.ip() == &gateway && kernel_peer.port() == 53) {
            io.abort(peer.into()).await;
            return Err(FlowOutcome::Refused(FlowRefusal::Policy));
        }
        let mode = PairMode::Datagram { udp: PairSide::B };
        let Some(vsock) = vsock.take() else {
            unreachable!("one flow task owns its record socket")
        };
        match io.attach(vsock, peer, mode) {
            Ok(pair) => Ok((pair, mode, None)),
            Err(refused) => {
                let half_ended = matches!(refused.error, overdrive_core::guest_transport::forwarder::ForwarderError::HalfEnded { .. });
                if half_ended {
                    {
                        let mut state = runtime.state.lock();
                        state.forwarder_attach_ended = state.forwarder_attach_ended.saturating_add(1);
                    }
                    io.abort(refused.vsock.into()).await;
                    io.abort(refused.peer.into()).await;
                } else {
                    tracing::error!(name: "guest_flow.attach_refused", cid = %key.peer_cid, flow = flow.get(), error = ?refused.error);
                    io.close(refused.vsock.into()).await;
                    io.close(refused.peer.into()).await;
                }
                Err(if half_ended {
                    FlowOutcome::Aborted
                } else {
                    FlowOutcome::Refused(FlowRefusal::IntakeUnavailable)
                })
            }
        }
    }.await;
    if let Some(vsock) = vsock
        && result.is_err()
    {
        io.close(vsock.into()).await;
    }
    result
}

async fn consume_ends(
    runtime: Arc<OwnerRuntime>,
    io: Arc<dyn GuestFlowIo>,
    shutdown: CancellationToken,
) {
    loop {
        let next = tokio::select! { biased; () = shutdown.cancelled() => return, result = io.next_end() => result };
        let Ok(end) = next else { return };
        process_end(&runtime, &io, end).await;
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "end-record consumption updates every pair, dial, watch and close projection in the same owner step"
)]
async fn process_end(runtime: &Arc<OwnerRuntime>, io: &Arc<dyn GuestFlowIo>, end: ForwardEnd) {
    let (pair, watch, abort_cause) = {
        let mut state = runtime.state.lock();
        state.last_consumed_end = Some(end);
        state.consumed_ends = state.consumed_ends.saturating_add(1);
        let abort_cause = state.abort_causes.remove(&end.id);
        let pair = state.pairs.remove(&end.id);
        if let Some(pair) = &pair
            && let Some(cid) = state.cids.get_mut(&pair.cid)
        {
            cid.flow_ids.remove(&pair.flow);
            cid.guest_flows.retain(|_, phase| !matches!(phase, GuestFlowPhase::Paired { pair: id, .. } if *id == end.id));
            cid.datagrams.remove(&pair.flow);
        }
        let watch = state.dial_watches.remove(&end.id);
        if let Some((cid, flow)) = watch {
            if let Some(entry) = state.cids.get_mut(&cid)
                && let Some(dial) = entry.dials.get_mut(&flow)
                && let DialPhase::Paired { guest_end, .. } = &mut dial.phase
            {
                *guest_end = match end.outcome {
                    ForwardOutcome::GuestHalfClose => {
                        Some(overdrive_core::guest_transport::dial::GuestStreamEnd::HalfClose)
                    }
                    ForwardOutcome::Aborted { .. } => {
                        Some(overdrive_core::guest_transport::dial::GuestStreamEnd::Abort)
                    }
                    ForwardOutcome::Completed => *guest_end,
                };
            } else if let Some(entry) = state.cids.get_mut(&cid)
                && matches!(
                    entry.dials.get(&flow).map(|dial| dial.phase),
                    Some(DialPhase::AwaitingReply)
                )
            {
                let outcome = match abort_cause {
                    Some(FlowCloseCause::Quiesced) => GuestDialError::Quiesced,
                    Some(FlowCloseCause::Teardown) => GuestDialError::NotServing,
                    Some(FlowCloseCause::ControlSessionLost) => GuestDialError::NoSession,
                    _ => {
                        let errno = match end.outcome {
                            ForwardOutcome::Aborted {
                                cause: AbortCause::SocketError { errno, .. },
                            } => errno,
                            _ => overdrive_core::guest_transport::wire::KernelErrno::ECONNRESET,
                        };
                        GuestDialError::Unreachable { errno }
                    }
                };
                entry.dial_outcomes.insert(flow, outcome);
                if let Some(cancel) = entry.dial_cancels.get(&flow) {
                    cancel.cancel();
                }
            }
        }
        (pair, watch, abort_cause)
    };
    if let ForwardOutcome::Aborted { cause: AbortCause::Unclassified { side } } = &end.outcome {
        if let Some(pair) = pair.as_ref() {
            tracing::warn!(
                name: "guest_flow.unclassified_end",
                cid = %pair.cid,
                flow = pair.flow.get(),
                side = side.as_str(),
                generation = %pair.generation,
            );
        } else if let Some((cid, flow)) = watch {
            let generation = runtime.state.lock().cids.get(&cid).and_then(|entry| {
                entry.dials.get(&flow).and_then(|dial| match dial.phase {
                    DialPhase::Paired { generation, .. } => Some(generation),
                    DialPhase::AwaitingReply => None,
                })
            });
            if let Some(generation) = generation {
                tracing::warn!(
                    name: "guest_flow.unclassified_end",
                    cid = %cid,
                    flow = flow.get(),
                    side = side.as_str(),
                    generation = %generation,
                );
            }
        }
    }
    if pair.is_none() && watch.is_none() {
        return;
    }
    if let Some(pair) = pair {
        let cause = abort_cause.unwrap_or_else(|| match end.outcome {
            ForwardOutcome::Completed | ForwardOutcome::GuestHalfClose => FlowCloseCause::PeerEof,
            ForwardOutcome::Aborted {
                cause: AbortCause::SocketError { side: PairSide::B, .. },
            } if pair.mode != PairMode::Stream => FlowCloseCause::DestinationUnreachable,
            ForwardOutcome::Aborted { cause: AbortCause::FrameInvalid { .. } } => {
                FlowCloseCause::FrameInvalid
            }
            ForwardOutcome::Aborted { cause: AbortCause::Detached } => FlowCloseCause::OwnerAbort,
            ForwardOutcome::Aborted { .. } => FlowCloseCause::PeerReset,
        });
        if end.outcome == ForwardOutcome::Completed && pair.mode == PairMode::Stream {
            let _ = io.clear_linger(&pair.peer).await;
        }
        io.close(pair.peer.into()).await;
        io.close(pair.vsock.into()).await;
        tracing::info!(name: "guest_flow.closed", alloc = %pair.alloc, cid = %pair.cid, kind = if pair.mode == PairMode::Stream { "tcp_connect" } else { "datagram" }, cause = cause.as_str());
    }
}

fn abort_guest_flow(
    runtime: &Arc<OwnerRuntime>,
    io: &Arc<dyn GuestFlowIo>,
    cid: GuestCid,
    flow: overdrive_core::guest_transport::wire::FlowId,
    cause: FlowCloseCause,
) -> bool {
    let (pair, watch, cancel, pending_datagram) = {
        let mut state = runtime.state.lock();
        let Some(entry) = state.cids.get_mut(&cid) else { return false };
        let outcome = if let Some(id) = entry.flow_ids.get(&flow).copied() {
            (Some(id), None, None, false)
        } else if let Some(dial) = entry.dials.get(&flow).copied() {
            match dial.phase {
                DialPhase::AwaitingReply => {
                    entry.dial_outcomes.insert(
                        flow,
                        GuestDialError::Unreachable {
                            errno: overdrive_core::guest_transport::wire::KernelErrno::ECONNRESET,
                        },
                    );
                    (None, None, entry.dial_cancels.get(&flow).cloned(), false)
                }
                DialPhase::Paired { watch, .. } => {
                    entry.dials.remove(&flow);
                    entry.dial_replies.remove(&flow);
                    entry.dial_cancels.remove(&flow);
                    (None, Some(watch), None, false)
                }
            }
        } else if matches!(
            entry.datagrams.get(&flow).map(|association| association.phase),
            Some(super::AssociationPhase::AwaitingPaired)
        ) {
            let pending_key = entry
                .flow_tasks
                .iter()
                .find_map(|(key, task)| (task.flow == Some(flow)).then_some(*key));
            entry.datagrams.remove(&flow);
            let cancel = pending_key
                .and_then(|key| entry.flow_tasks.get(&key).map(|task| task.cancel.clone()));
            (None, None, cancel, true)
        } else {
            (None, None, None, false)
        };
        drop(state);
        outcome
    };
    let found = pending_datagram || pair.is_some() || watch.is_some() || cancel.is_some();
    if let Some(cancel) = cancel {
        cancel.cancel();
    }
    if let Some(id) = pair
        && matches!(io.abort_forward(id), Ok(AbortResult::Aborted))
    {
        runtime.state.lock().abort_causes.insert(id, cause);
    }
    if let Some(watch) = watch {
        if matches!(io.abort_forward(watch), Ok(AbortResult::Aborted)) {
            runtime.state.lock().abort_causes.insert(watch, cause);
        }
        runtime.state.lock().dial_watches.remove(&watch);
    }
    found
}

fn session_lost(
    runtime: &Arc<OwnerRuntime>,
    io: &Arc<dyn GuestFlowIo>,
    cid: GuestCid,
    key: GuestConnKey,
    cause: FlowCloseCause,
) {
    let (flows, cancels, watches) = {
        let mut state = runtime.state.lock();
        let Some(entry) = state.cids.get_mut(&cid) else { return };
        if entry.session != Some(key) {
            return;
        }
        entry.session = None;
        entry.session_sender = None;
        entry.session_cancel = None;
        let flows = entry.flow_ids.values().copied().collect::<Vec<_>>();
        let cancels = entry.flow_tasks.values().map(|task| task.cancel.clone()).collect::<Vec<_>>();
        let mut watches = Vec::new();
        let dial_flows =
            entry.dials.iter().map(|(flow, dial)| (*flow, dial.phase)).collect::<Vec<_>>();
        for (flow, phase) in dial_flows {
            if matches!(phase, DialPhase::AwaitingReply) {
                entry.dial_outcomes.insert(flow, GuestDialError::NoSession);
            }
            if let Some(cancel) = entry.dial_cancels.get(&flow) {
                cancel.cancel();
            }
            entry.dial_replies.remove(&flow);
            if let DialPhase::Paired { watch, .. } = phase {
                watches.push((flow, watch));
                entry.dials.remove(&flow);
                entry.dial_cancels.remove(&flow);
                entry.dial_requests.remove(&flow);
            }
        }
        let snapshot = (flows, cancels, watches);
        drop(state);
        snapshot
    };
    for cancel in cancels {
        cancel.cancel();
    }
    for id in flows {
        if matches!(io.abort_forward(id), Ok(AbortResult::Aborted)) {
            runtime.state.lock().abort_causes.insert(id, cause);
        }
    }
    for (flow, watch) in watches {
        if matches!(io.abort_forward(watch), Ok(AbortResult::Aborted)) {
            runtime.state.lock().abort_causes.insert(watch, cause);
        }
        runtime.state.lock().dial_watches.remove(&watch);
        tracing::info!(name: "guest_flow.closed", alloc = %allocation_for(runtime, cid), cid = %cid, kind = "host_dial", cause = cause.as_str(), flow = flow.get());
    }
}

fn finish_connection(
    runtime: &Arc<OwnerRuntime>,
    cid: GuestCid,
    key: GuestConnKey,
    clear_session: bool,
    accepted_launch: LaunchToken,
) {
    let mut state = runtime.state.lock();
    if let Some(entry) = state.cids.get_mut(&cid) {
        let task_belongs_to_launch = entry
            .control_tasks
            .get(&key)
            .is_some_and(|task| task.owner_launch == Some(accepted_launch));
        if task_belongs_to_launch {
            entry.unread.remove(&key);
            entry.control_tasks.remove(&key);
            if clear_session && entry.launch == accepted_launch && entry.session == Some(key) {
                entry.session = None;
                entry.session_sender = None;
                entry.session_cancel = None;
            }
        }
    }
    let remove_retired = state.cids.get(&cid).is_some_and(|entry| {
        entry.launch == accepted_launch
            && entry.phase == CidPhase::Retiring
            && entry.session.is_none()
            && entry.unread.is_empty()
            && entry.control_tasks.is_empty()
            && entry.guest_flows.is_empty()
            && entry.flow_tasks.is_empty()
            && entry.flow_ids.is_empty()
            && entry.dials.is_empty()
            && entry.datagrams.is_empty()
    }) && !state.pairs.values().any(|pair| pair.cid == cid)
        && !state.dial_watches.values().any(|(owner_cid, _)| *owner_cid == cid);
    if remove_retired {
        state.cids.remove(&cid);
    }
}

async fn read_fixed(
    io: &dyn GuestFlowIo,
    socket: &RecordSocket,
    buffer: &mut [u8],
    timeout: Duration,
    runtime: Arc<OwnerRuntime>,
) -> io::Result<()> {
    let bytes = tokio::select! {
        biased;
        () = runtime.clock.sleep(timeout) => return Err(io::Error::new(io::ErrorKind::TimedOut, "fixed record deadline elapsed")),
        result = read_fixed_optional(io, socket, buffer.len()) => result?,
    };
    let Some(bytes) = bytes else {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "connection ended before fixed record",
        ));
    };
    buffer.copy_from_slice(&bytes);
    Ok(())
}

async fn read_fixed_optional(
    io: &dyn GuestFlowIo,
    socket: &RecordSocket,
    length: usize,
) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::with_capacity(length);
    while bytes.len() < length {
        match io.read_record(socket, length - bytes.len()).await {
            Ok(chunk) if chunk.is_empty() && bytes.is_empty() => return Ok(None),
            Ok(chunk) if chunk.is_empty() => {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "partial fixed record"));
            }
            Ok(chunk) => bytes.extend_from_slice(&chunk),
            Err(FlowIoError::Io { source }) => return Err(source),
        }
    }
    Ok(Some(bytes))
}

fn finish_flow_task(runtime: &Arc<OwnerRuntime>, cid: GuestCid, key: GuestConnKey) {
    let removed = runtime.state.lock().remove_flow_task(cid, key);
    if removed.as_ref().is_some_and(FlowTask::is_refusal_reader) {
        runtime.refusal_reader_capacity.notify_waiters();
    }
}

fn set_request_read(runtime: &Arc<OwnerRuntime>, cid: GuestCid, key: GuestConnKey, partial: bool) {
    if let Some(entry) = runtime.state.lock().cids.get_mut(&cid)
        && let Some(phase) = entry.guest_flows.get_mut(&key)
        && matches!(phase, GuestFlowPhase::Registered { .. })
    {
        *phase = GuestFlowPhase::Registered {
            request: if partial {
                super::RequestRead::Partial
            } else {
                super::RequestRead::Nothing
            },
        };
    }
}

fn set_request_attributed(runtime: &Arc<OwnerRuntime>, cid: GuestCid, key: GuestConnKey) {
    if let Some(entry) = runtime.state.lock().cids.get_mut(&cid)
        && entry.guest_flows.contains_key(&key)
    {
        entry.guest_flows.insert(key, GuestFlowPhase::Attributed);
    }
}

fn set_datagram_awaiting_paired(
    runtime: &Arc<OwnerRuntime>,
    cid: GuestCid,
    key: GuestConnKey,
    flow: overdrive_core::guest_transport::wire::FlowId,
) -> bool {
    let mut state = runtime.state.lock();
    let changed = if state.admission == GuestAdmission::Quiesced {
        false
    } else if let Some(entry) = state.cids.get_mut(&cid) {
        if entry.session.is_none()
            || entry.phase != CidPhase::Active
            || !entry.guest_flows.contains_key(&key)
        {
            false
        } else if let Some(task) = entry.flow_tasks.get_mut(&key) {
            task.flow = Some(flow);
            entry.guest_flows.remove(&key);
            entry.datagrams.insert(
                flow,
                super::AssociationObservation { phase: super::AssociationPhase::AwaitingPaired },
            );
            true
        } else {
            false
        }
    } else {
        false
    };
    drop(state);
    changed
}

fn remove_datagram(
    runtime: &Arc<OwnerRuntime>,
    cid: GuestCid,
    flow: overdrive_core::guest_transport::wire::FlowId,
) {
    if let Some(entry) = runtime.state.lock().cids.get_mut(&cid) {
        entry.datagrams.remove(&flow);
    }
}

fn pending_datagram_cancel_outcome(
    runtime: &Arc<OwnerRuntime>,
    cid: GuestCid,
    flow: overdrive_core::guest_transport::wire::FlowId,
) -> FlowOutcome {
    let state = runtime.state.lock();
    if state.admission != GuestAdmission::Quiesced
        && state.cids.get(&cid).is_some_and(|entry| {
            entry.session.is_some()
                && entry.phase == CidPhase::Active
                && !entry.datagrams.contains_key(&flow)
        })
    {
        FlowOutcome::GuestEnded
    } else {
        FlowOutcome::Refused(FlowRefusal::IntakeUnavailable)
    }
}

fn send_on(sender: &mpsc::UnboundedSender<ControlMessage>, message: ControlMessage) {
    let _ = sender.send(message);
}

fn remove_guest_flow(runtime: &Arc<OwnerRuntime>, cid: GuestCid, key: GuestConnKey) {
    if let Some(entry) = runtime.state.lock().cids.get_mut(&cid) {
        entry.guest_flows.remove(&key);
    }
}

fn runtime_vip_contains(runtime: &Arc<OwnerRuntime>, address: Ipv4Addr) -> bool {
    runtime.vip_range.contains(address)
}

fn static_policy(
    address: Ipv4Addr,
    datagram: bool,
    node_prefix: ipnet::Ipv4Net,
    gateway: Ipv4Addr,
    port: u16,
) -> bool {
    let raw = u32::from(address);
    let excluded = raw & 0xff00_0000 == 0
        || address.is_multicast()
        || address == Ipv4Addr::BROADCAST
        || raw & 0xf000_0000 == 0xf000_0000;
    excluded || (datagram && node_prefix.contains(&address) && !(address == gateway && port == 53))
}

fn static_host_internal(address: Ipv4Addr) -> bool {
    address.is_loopback() || address.octets()[0] == 169 && address.octets()[1] == 254
}

fn allocation_for(runtime: &Arc<OwnerRuntime>, cid: GuestCid) -> String {
    runtime.state.lock().cids.get(&cid).map(|entry| entry.alloc.to_string()).unwrap_or_default()
}

fn dial_reply_sender(
    runtime: &Arc<OwnerRuntime>,
    cid: GuestCid,
    flow: FlowId,
) -> Option<oneshot::Sender<ControlMessage>> {
    runtime.state.lock().cids.get_mut(&cid)?.dial_replies.remove(&flow)
}

fn discard_control(cid: GuestCid, flow: FlowId, message: &str) {
    tracing::info!(name: "guest_flow.control_discarded", cid = %cid, flow = ?flow, message);
}

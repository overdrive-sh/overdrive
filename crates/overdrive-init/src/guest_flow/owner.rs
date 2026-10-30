use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{self, ErrorKind};
use std::net::SocketAddrV4;
use std::sync::Arc;
use std::time::Instant;

use nix::libc;
use overdrive_core::guest_transport::forwarder::{ForwardEnd, ForwardOutcome, ForwarderError};
use overdrive_core::guest_transport::wire::{
    CONTROL_MESSAGE_LEN, CONTROL_RECONNECT_BACKOFF, ControlMessage, ControlOpen,
    FLOW_PAIRING_DEADLINE, FLOW_REQUEST_DEADLINE, FLOW_REQUEST_LEN, FlowId, FlowOrigin,
    FlowRefusal, FlowRequest, GUEST_INBOUND_CONNECT_TIMEOUT, KernelErrno,
};
use overdrive_core::guest_transport::{
    GUEST_DATAGRAM_SLOT_IDLE, GUEST_OWNER_AUDIT_PERIOD, GUEST_TCP_PENDING_MAX, GUEST_UDP_SLOT_BASE,
    GUEST_UDP_SLOTS,
};

use super::{
    AppConn, CapturedConn, GuestAcceptPhase, GuestAttachRefused, GuestClock, GuestHangup,
    GuestIoEvent, GuestOwnerFatal, GuestOwnerIo, GuestPair, GuestRecord, GuestSideObservation,
    GuestTransportToken, HostChannel, HostConn, InboundConn, SlotObservation, SocketCookie,
    SocketId,
};

struct PendingOutbound {
    child: CapturedConn,
    destination: SocketAddrV4,
    cookie: SocketCookie,
    vsock: HostConn,
}

struct PendingInboundRead {
    conn: InboundConn,
    deadline: Instant,
}

struct AcceptedFlow {
    vsock: InboundConn,
    app: AppConn,
    deadline: Instant,
}

struct DatagramFlow {
    slot: SocketAddrV4,
    destination: SocketAddrV4,
    vsock: HostConn,
    deadline: Instant,
}

enum DeferredGuestClose {
    Stream { pair: GuestPair, clear_linger: bool },
    Datagram { slot: SocketAddrV4, release: bool, flow: Option<FlowId> },
}

struct SlotState {
    owner: Option<(SocketCookie, SocketAddrV4)>,
    association: Option<FlowId>,
    attached: Option<overdrive_core::guest_transport::forwarder::ForwardId>,
    last_counters: Option<(u64, u64)>,
    idle_since: Instant,
}

pub(crate) struct GuestFlowOwner {
    token: GuestTransportToken,
    io: Box<dyn GuestOwnerIo>,
    clock: Arc<dyn GuestClock>,
    session: Option<HostConn>,
    session_connected: bool,
    session_read_pending: bool,
    session_retry: Option<Instant>,
    audit_deadline: Option<Instant>,
    next_guest_flow: u32,
    pending_outbound: BTreeMap<FlowId, PendingOutbound>,
    pending_inbound: BTreeMap<SocketId, PendingInboundRead>,
    accepted: BTreeMap<FlowId, AcceptedFlow>,
    tcp_pairs: BTreeMap<FlowId, GuestPair>,
    pending_guest_closes: VecDeque<DeferredGuestClose>,
    pending_end_closes:
        BTreeMap<overdrive_core::guest_transport::forwarder::ForwardId, DeferredGuestClose>,
    datagrams: BTreeMap<FlowId, DatagramFlow>,
    slots: BTreeMap<SocketAddrV4, SlotState>,
    released_datagram_flows: BTreeSet<FlowId>,
    observation: GuestSideObservation,
}

impl GuestFlowOwner {
    pub(crate) fn new(
        token: GuestTransportToken,
        io: Box<dyn GuestOwnerIo>,
        clock: Arc<dyn GuestClock>,
    ) -> Self {
        let now = clock.now();
        let slots = (0..GUEST_UDP_SLOTS)
            .map(|offset| {
                let addr = SocketAddrV4::new(token.workload_addr, GUEST_UDP_SLOT_BASE + offset);
                (
                    addr,
                    SlotState {
                        owner: None,
                        association: None,
                        attached: None,
                        last_counters: None,
                        idle_since: now,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let slot_observation = slots
            .keys()
            .copied()
            .map(|slot| (slot, SlotObservation { owner: None, association: None, attached: None }))
            .collect();
        Self {
            token,
            io,
            clock,
            session: None,
            session_connected: false,
            session_read_pending: false,
            session_retry: Some(now),
            audit_deadline: None,
            next_guest_flow: 1,
            pending_outbound: BTreeMap::new(),
            pending_inbound: BTreeMap::new(),
            accepted: BTreeMap::new(),
            tcp_pairs: BTreeMap::new(),
            pending_guest_closes: VecDeque::new(),
            pending_end_closes: BTreeMap::new(),
            datagrams: BTreeMap::new(),
            slots,
            released_datagram_flows: BTreeSet::new(),
            observation: GuestSideObservation {
                session: false,
                accepted: BTreeMap::new(),
                flows: BTreeMap::new(),
                awaiting_paired: BTreeSet::new(),
                slots: slot_observation,
                forwarder_attach_ended: 0,
                forwarder_attached: 0,
                last_consumed_end: None,
                consumed_ends: 0,
            },
        }
    }

    pub(crate) fn dispatch_next(&mut self) -> Result<(), GuestOwnerFatal> {
        if let Some(close) = self.pending_guest_closes.pop_front() {
            self.apply_deferred_guest_close(close);
            return Ok(());
        }
        let deadline = self.next_deadline();
        let event = match self.io.next_event(deadline) {
            Ok(event) => event,
            Err(source) => {
                if source
                    .get_ref()
                    .is_some_and(|error| error.downcast_ref::<ForwarderError>().is_some())
                {
                    let Some(boxed) = source.into_inner() else {
                        return Err(owner_invariant("forwarder error source disappeared"));
                    };
                    let Ok(error) = boxed.downcast::<ForwarderError>() else {
                        return Err(owner_invariant("forwarder error source changed type"));
                    };
                    return Err(GuestOwnerFatal::Forwarder { source: *error });
                }
                return Err(GuestOwnerFatal::EventWait { source });
            }
        };
        if let Some(event) = event {
            self.handle_event(event)?;
            return Ok(());
        }
        let now = self.clock.now();
        let Some(due) = deadline.filter(|deadline| *deadline <= now) else {
            return Err(GuestOwnerFatal::EventWait {
                source: io::Error::new(ErrorKind::WouldBlock, "readiness returned before deadline"),
            });
        };
        self.handle_deadline(due)
    }

    pub(crate) fn run(mut self) -> GuestOwnerFatal {
        loop {
            if let Err(fatal) = self.dispatch_next() {
                return fatal;
            }
        }
    }

    pub(crate) fn guest_observation(&self) -> GuestSideObservation {
        self.observation.clone()
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        let mut next = if self.pending_guest_closes.is_empty() {
            self.session_retry
        } else {
            Some(self.clock.now())
        };
        let mut include = |candidate: Option<Instant>| {
            if let Some(candidate) = candidate {
                next = Some(next.map_or(candidate, |current| current.min(candidate)));
            }
        };
        include(self.audit_deadline);
        include(self.pending_inbound.values().map(|pending| pending.deadline).min());
        include(self.accepted.values().map(|accepted| accepted.deadline).min());
        include(self.datagrams.values().map(|flow| flow.deadline).min());
        next
    }

    fn handle_deadline(&mut self, due: Instant) -> Result<(), GuestOwnerFatal> {
        let now = self.clock.now();
        if self.session_retry == Some(due) {
            self.session_retry = None;
            self.start_control_connect(now);
        } else if self.audit_deadline == Some(due) {
            self.audit_deadline = Some(due + GUEST_OWNER_AUDIT_PERIOD);
            self.audit_slots();
        } else if let Some(id) = self
            .pending_inbound
            .iter()
            .find_map(|(id, pending)| (pending.deadline == due).then_some(*id))
        {
            if let Some(pending) = self.pending_inbound.remove(&id) {
                self.io.close(pending.conn.into());
            }
        } else if let Some(flow) = self
            .accepted
            .iter()
            .find_map(|(flow, accepted)| (accepted.deadline == due).then_some(*flow))
        {
            if let Some(accepted) = self.accepted.remove(&flow) {
                self.io.abort(accepted.app.into());
                self.io.abort(accepted.vsock.into());
                self.observation.accepted.remove(&flow);
                self.write_control(ControlMessage::TimedOut {
                    flow,
                    errno: errno(libc::ETIMEDOUT),
                });
            }
        } else if let Some(flow) = self
            .datagrams
            .iter()
            .find_map(|(flow, datagram)| (datagram.deadline == due).then_some(*flow))
            && let Some(datagram) = self.datagrams.remove(&flow)
        {
            self.observation.awaiting_paired.remove(&flow);
            self.end_slot_association(datagram.slot, false)?;
            self.write_control(ControlMessage::Abort { flow });
        }
        Ok(())
    }

    fn handle_event(&mut self, event: GuestIoEvent) -> Result<(), GuestOwnerFatal> {
        match event {
            GuestIoEvent::Connected { socket, result } => self.handle_connected(socket, result),
            GuestIoEvent::Read { socket, result } => self.handle_read(socket, result),
            GuestIoEvent::InboundAccepted { conn } => {
                let id = conn.id();
                match self.io.start_read_inbound(&conn) {
                    Ok(()) => {
                        self.pending_inbound.insert(
                            id,
                            PendingInboundRead {
                                conn,
                                deadline: self.clock.now() + FLOW_REQUEST_DEADLINE,
                            },
                        );
                    }
                    Err(_) => self.io.close(conn.into()),
                }
                Ok(())
            }
            GuestIoEvent::OutboundCaptured { child, original_destination, cookie } => {
                self.handle_capture(child, original_destination, cookie)
            }
            GuestIoEvent::Hangup { socket, hangup } => self.handle_hangup(socket, hangup),
            GuestIoEvent::SlotReady { slot } => self.start_datagram(slot),
            GuestIoEvent::SocketReleased { cookie } => {
                let slots = self
                    .slots
                    .iter()
                    .filter_map(|(slot, state)| {
                        state.owner.is_some_and(|(owner, _)| owner == cookie).then_some(*slot)
                    })
                    .collect::<Vec<_>>();
                for slot in slots {
                    self.release_slot(slot)?;
                }
                Ok(())
            }
            GuestIoEvent::ForwardEnded { end } => {
                self.handle_forward_end(end);
                Ok(())
            }
            GuestIoEvent::ListenerFailed { listener, kind } => {
                Err(GuestOwnerFatal::ListenerFailed { listener, kind })
            }
        }
    }

    fn start_control_connect(&mut self, now: Instant) {
        if self.session.is_some() {
            return;
        }
        if self.audit_deadline.is_none() {
            self.audit_deadline = Some(now + GUEST_OWNER_AUDIT_PERIOD);
        }
        if let Ok(conn) = self.io.start_connect_host(HostChannel::Control) {
            self.session = Some(conn);
            self.session_connected = false;
            self.session_read_pending = false;
            self.observation.session = true;
        } else {
            self.session_retry = Some(now + CONTROL_RECONNECT_BACKOFF);
            self.observation.session = false;
        }
    }

    fn handle_connected(
        &mut self,
        socket: SocketId,
        result: Result<(), ErrorKind>,
    ) -> Result<(), GuestOwnerFatal> {
        if self.session.as_ref().is_some_and(|conn| conn.id() == socket) {
            if result.is_err() {
                self.lose_session();
                return Ok(());
            }
            self.session_connected = true;
            let message = ControlOpen { launch: self.token.launch }.encode();
            if !self.write_session_bytes(&message) {
                self.lose_session();
                return Ok(());
            }
            self.arm_session_read();
            return Ok(());
        }
        if let Some(flow) = self
            .pending_outbound
            .iter()
            .find_map(|(flow, pending)| (pending.vsock.id() == socket).then_some(*flow))
        {
            return self.handle_outbound_connected(flow, result);
        }
        if let Some(flow) = self
            .accepted
            .iter()
            .find_map(|(flow, accepted)| (accepted.app.id() == socket).then_some(*flow))
        {
            let Some(accepted) = self.accepted.remove(&flow) else { return Ok(()) };
            if let Err(kind) = result {
                let errno = errno(error_errno(kind));
                self.io.abort(accepted.app.into());
                self.io.abort(accepted.vsock.into());
                self.observation.accepted.remove(&flow);
                self.write_control(ControlMessage::Refused {
                    flow,
                    refusal: FlowRefusal::ApplicationRefused { errno },
                });
                return Ok(());
            }
            match self.io.attach_inbound(accepted.vsock, accepted.app) {
                Ok(pair) => {
                    let id = pair.id;
                    self.tcp_pairs.insert(flow, pair);
                    self.observation.flows.insert(flow, id);
                    self.observation.accepted.insert(flow, GuestAcceptPhase::Paired { pair: id });
                    self.observation.forwarder_attached += 1;
                    self.write_control(ControlMessage::Paired { flow });
                }
                Err(refused) => {
                    self.handle_inbound_attach_refused(flow, refused);
                }
            }
            return Ok(());
        }
        if let Some(flow) = self
            .datagrams
            .iter()
            .find_map(|(flow, datagram)| (datagram.vsock.id() == socket).then_some(*flow))
        {
            if result.is_err() {
                if let Some(datagram) = self.datagrams.remove(&flow) {
                    self.observation.awaiting_paired.remove(&flow);
                    self.end_slot_association(datagram.slot, false)?;
                }
                return Ok(());
            }
            let Some(datagram) = self.datagrams.get(&flow) else { return Ok(()) };
            let request = FlowRequest::DatagramStream {
                flow,
                destination: datagram.destination,
                launch: self.token.launch,
            }
            .encode();
            if self.io.write_slot_request(&datagram.vsock, &request).is_err() {
                let Some(datagram) = self.datagrams.remove(&flow) else {
                    return Err(owner_invariant("pending datagram flow disappeared"));
                };
                self.observation.awaiting_paired.remove(&flow);
                self.end_slot_association(datagram.slot, false)?;
            } else {
                self.observation.awaiting_paired.insert(flow);
            }
            return Ok(());
        }
        Ok(())
    }

    fn handle_outbound_connected(
        &mut self,
        flow: FlowId,
        result: Result<(), ErrorKind>,
    ) -> Result<(), GuestOwnerFatal> {
        let Some(pending) = self.pending_outbound.get(&flow) else { return Ok(()) };
        if result.is_err() {
            let Some(pending) = self.pending_outbound.remove(&flow) else {
                return Err(owner_invariant("pending outbound flow disappeared"));
            };
            self.io.abort(pending.child.into());
            self.io.close(pending.vsock.into());
            return Ok(());
        }
        let request = FlowRequest::TcpConnect {
            flow,
            destination: pending.destination,
            launch: self.token.launch,
        }
        .encode();
        if self.io.write_request(&pending.vsock, &request).is_err() {
            let Some(pending) = self.pending_outbound.remove(&flow) else {
                return Err(owner_invariant("pending outbound flow disappeared"));
            };
            self.io.abort(pending.child.into());
            self.io.close(pending.vsock.into());
        } else {
            self.observation.awaiting_paired.insert(flow);
        }
        Ok(())
    }

    fn handle_read(
        &mut self,
        socket: SocketId,
        result: Result<Vec<u8>, ErrorKind>,
    ) -> Result<(), GuestOwnerFatal> {
        if self.session.as_ref().is_some_and(|conn| conn.id() == socket) {
            self.session_read_pending = false;
            let Ok(bytes) = result else {
                self.lose_session();
                return Ok(());
            };
            if bytes.len() != CONTROL_MESSAGE_LEN {
                self.lose_session();
                return Ok(());
            }
            let Ok(bytes) = <[u8; CONTROL_MESSAGE_LEN]>::try_from(bytes.as_slice()) else {
                self.lose_session();
                return Ok(());
            };
            let Ok(message) = ControlMessage::decode(&bytes) else {
                self.lose_session();
                return Ok(());
            };
            self.handle_control(message)?;
            if self.session.is_some() {
                self.arm_session_read();
            }
            return Ok(());
        }
        if let Some(pending) = self.pending_inbound.remove(&socket) {
            let Ok(bytes) = result else {
                self.io.close(pending.conn.into());
                return Ok(());
            };
            if bytes.len() != FLOW_REQUEST_LEN {
                self.io.close(pending.conn.into());
                return Ok(());
            }
            let Ok(bytes) = <[u8; FLOW_REQUEST_LEN]>::try_from(bytes.as_slice()) else {
                self.io.close(pending.conn.into());
                return Ok(());
            };
            let Ok(FlowRequest::TcpAccept { flow, peer, listen_port, launch }) =
                FlowRequest::decode(&bytes)
            else {
                self.io.close(pending.conn.into());
                return Ok(());
            };
            let launch_matches = launch == self.token.launch;
            let inbound_ended = if launch_matches {
                self.io.inbound_ended(&pending.conn).unwrap_or(true)
            } else {
                true
            };
            if !launch_matches || inbound_ended {
                self.io.close(pending.conn.into());
                return Ok(());
            }
            let client = SocketAddrV4::new(peer, 0);
            let destination = SocketAddrV4::new(self.token.workload_addr, listen_port.get());
            match self.io.start_connect_app(client, destination) {
                Ok(app) => {
                    let deadline = self.clock.now() + GUEST_INBOUND_CONNECT_TIMEOUT;
                    self.accepted.insert(flow, AcceptedFlow { vsock: pending.conn, app, deadline });
                    self.observation.accepted.insert(flow, GuestAcceptPhase::Accepted);
                }
                Err(error) => {
                    let errno = errno(error.raw_os_error().unwrap_or(libc::ECONNREFUSED));
                    self.io.abort(pending.conn.into());
                    self.write_control(ControlMessage::Refused {
                        flow,
                        refusal: FlowRefusal::ApplicationRefused { errno },
                    });
                }
            }
            return Ok(());
        }
        Ok(())
    }

    fn handle_capture(
        &mut self,
        child: CapturedConn,
        destination: SocketAddrV4,
        cookie: SocketCookie,
    ) -> Result<(), GuestOwnerFatal> {
        let pending_max = usize::try_from(GUEST_TCP_PENDING_MAX).unwrap_or(usize::MAX);
        if self.pending_outbound.len() >= pending_max || self.session.is_none() {
            self.io.abort(child.into());
            return Ok(());
        }
        let flow = self.next_guest_flow_id()?;
        match self.io.start_connect_host(HostChannel::FlowStream) {
            Ok(vsock) => {
                self.pending_outbound
                    .insert(flow, PendingOutbound { child, destination, cookie, vsock });
            }
            Err(_) => self.io.abort(child.into()),
        }
        Ok(())
    }

    fn next_guest_flow_id(&mut self) -> Result<FlowId, GuestOwnerFatal> {
        let flow = FlowId::new(FlowOrigin::Guest, self.next_guest_flow)
            .map_err(|_| owner_invariant("guest flow sequence exhausted"))?;
        self.next_guest_flow = self
            .next_guest_flow
            .checked_add(1)
            .ok_or_else(|| owner_invariant("guest flow sequence overflowed"))?;
        Ok(flow)
    }

    fn handle_control(&mut self, message: ControlMessage) -> Result<(), GuestOwnerFatal> {
        match message {
            ControlMessage::Paired { flow } => {
                if let Some(pending) = self.pending_outbound.remove(&flow) {
                    self.observation.awaiting_paired.remove(&flow);
                    match self.io.attach_outbound(pending.child, pending.vsock) {
                        Ok(pair) => {
                            self.observation.forwarder_attached += 1;
                            self.observation.flows.insert(flow, pair.id);
                            self.tcp_pairs.insert(flow, pair);
                        }
                        Err(refused) => {
                            if matches!(refused.error, ForwarderError::HalfEnded { .. }) {
                                self.observation.forwarder_attach_ended += 1;
                            }
                            self.close_refused(refused, true);
                        }
                    }
                } else if let Some(datagram) = self.datagrams.remove(&flow) {
                    self.observation.awaiting_paired.remove(&flow);
                    match self.io.attach_slot(datagram.slot, datagram.vsock) {
                        Ok(pair) => {
                            if let Some(slot) = self.slots.get_mut(&datagram.slot) {
                                slot.attached = Some(pair);
                                slot.last_counters = self.io.slot_counters(datagram.slot).ok();
                                slot.idle_since = self.clock.now();
                            }
                            self.sync_slot_observation(datagram.slot);
                            self.observation.forwarder_attached += 1;
                        }
                        Err(refused) => {
                            if matches!(&refused.error, ForwarderError::HalfEnded { .. }) {
                                self.observation.forwarder_attach_ended += 1;
                            }
                            self.io.close(refused.vsock.into());
                            self.end_slot_association(datagram.slot, false)?;
                        }
                    }
                } else if self.released_datagram_flows.remove(&flow) {
                    self.write_control(ControlMessage::Abort { flow });
                }
            }
            ControlMessage::Refused { flow, .. } => {
                if let Some(pending) = self.pending_outbound.remove(&flow) {
                    self.observation.awaiting_paired.remove(&flow);
                    self.io.abort(pending.child.into());
                    self.io.close(pending.vsock.into());
                } else if let Some(datagram) = self.datagrams.remove(&flow) {
                    self.observation.awaiting_paired.remove(&flow);
                    self.end_slot_association(datagram.slot, false)?;
                }
            }
            ControlMessage::Abort { flow } => {
                if let Some(pending) = self.pending_outbound.remove(&flow) {
                    self.observation.awaiting_paired.remove(&flow);
                    self.io.abort(pending.child.into());
                    self.io.close(pending.vsock.into());
                } else if let Some(accepted) = self.accepted.remove(&flow) {
                    self.observation.accepted.remove(&flow);
                    self.io.abort(accepted.app.into());
                    self.io.abort(accepted.vsock.into());
                } else if self.tcp_pairs.contains_key(&flow) {
                    self.abort_tcp_pair(flow)?;
                } else if let Some(datagram) = self.datagrams.remove(&flow) {
                    self.observation.awaiting_paired.remove(&flow);
                    self.end_slot_association(datagram.slot, false)?;
                } else if let Some((slot, _)) =
                    self.slots.iter().find(|(_, state)| state.association == Some(flow))
                {
                    self.end_slot_association(*slot, false)?;
                }
            }
            ControlMessage::TimedOut { .. } => {}
        }
        Ok(())
    }

    fn handle_inbound_attach_refused(&mut self, flow: FlowId, refused: GuestAttachRefused) {
        self.observation.accepted.remove(&flow);
        if let ForwarderError::HalfEnded { side, errno: end_errno } = refused.error {
            self.observation.forwarder_attach_ended += 1;
            self.io.abort(refused.outer);
            self.io.close(refused.vsock);
            if side == overdrive_core::guest_transport::forwarder::PairSide::B {
                self.write_control(ControlMessage::Refused {
                    flow,
                    refusal: FlowRefusal::ApplicationRefused { errno: end_errno },
                });
            }
        } else {
            self.io.abort(refused.outer);
            self.io.close(refused.vsock);
        }
    }

    fn close_refused(&mut self, refused: GuestAttachRefused, reset_outer: bool) {
        if reset_outer {
            self.io.abort(refused.outer);
        } else {
            self.io.close(refused.outer);
        }
        self.io.close(refused.vsock);
    }

    fn handle_hangup(
        &mut self,
        socket: SocketId,
        hangup: GuestHangup,
    ) -> Result<(), GuestOwnerFatal> {
        if self.session.as_ref().is_some_and(|conn| conn.id() == socket) {
            self.lose_session();
            return Ok(());
        }
        if let Some(flow) = self.pending_outbound.iter().find_map(|(flow, pending)| {
            (pending.child.id() == socket || pending.vsock.id() == socket).then_some(*flow)
        }) {
            if self.pending_outbound.get(&flow).is_some_and(|pending| pending.child.id() == socket)
                && hangup == GuestHangup::ReadHangup
            {
                // EVMOD-PAIR: the application may write and half-close I
                // before V_g reaches Paired.  The intake child remains in
                // CLOSE_WAIT so the attach-time run carries its queued bytes
                // and FIN; only a reset/error ends a pending TcpConnect.
                return Ok(());
            }
            let Some(pending) = self.pending_outbound.remove(&flow) else {
                return Err(owner_invariant("pending outbound flow disappeared"));
            };
            self.observation.awaiting_paired.remove(&flow);
            let silent = pending.vsock.id() == socket && hangup == GuestHangup::ReadHangup;
            self.io.abort(pending.child.into());
            self.io.close(pending.vsock.into());
            if !silent {
                self.write_control(ControlMessage::Abort { flow });
            }
            return Ok(());
        }
        if let Some(flow) = self.accepted.iter().find_map(|(flow, accepted)| {
            (accepted.vsock.id() == socket || accepted.app.id() == socket).then_some(*flow)
        }) {
            let Some(accepted) = self.accepted.remove(&flow) else {
                return Err(owner_invariant("accepted flow disappeared"));
            };
            self.observation.accepted.remove(&flow);
            let silent = accepted.vsock.id() == socket && hangup == GuestHangup::ReadHangup;
            self.io.abort(accepted.app.into());
            self.io.abort(accepted.vsock.into());
            if !silent {
                self.write_control(ControlMessage::Abort { flow });
            }
        }
        Ok(())
    }

    fn handle_forward_end(&mut self, end: ForwardEnd) {
        if let Some(close) = self.pending_end_closes.remove(&end.id) {
            self.observe_forward_end(end);
            self.pending_guest_closes.push_back(close);
            return;
        }
        if let Some(flow) =
            self.tcp_pairs.iter().find_map(|(flow, pair)| (pair.id == end.id).then_some(*flow))
        {
            self.observe_forward_end(end);
            if let Some(pair) = self.tcp_pairs.remove(&flow) {
                self.observation.flows.remove(&flow);
                self.observation.accepted.remove(&flow);
                self.pending_guest_closes.push_back(DeferredGuestClose::Stream {
                    pair,
                    clear_linger: end.outcome == ForwardOutcome::Completed,
                });
            }
            return;
        }
        if let Some(slot) = self
            .slots
            .iter()
            .find_map(|(slot, state)| (state.attached == Some(end.id)).then_some(*slot))
        {
            self.observe_forward_end(end);
            let flow = self.slots.get(&slot).and_then(|state| state.association);
            self.pending_guest_closes.push_back(DeferredGuestClose::Datagram {
                slot,
                release: false,
                flow,
            });
        }
    }

    const fn observe_forward_end(&mut self, end: ForwardEnd) {
        self.observation.last_consumed_end = Some(end);
        self.observation.consumed_ends += 1;
    }

    fn apply_deferred_guest_close(&mut self, close: DeferredGuestClose) {
        match close {
            DeferredGuestClose::Stream { pair, clear_linger } => {
                if clear_linger {
                    let _ = self.io.clear_linger(&pair.outer);
                }
                self.io.close(pair.outer.into());
                self.io.close(pair.vsock.into());
            }
            DeferredGuestClose::Datagram { slot, release, flow } => {
                self.finish_slot_association(slot, release, flow);
            }
        }
    }

    fn start_datagram(&mut self, slot: SocketAddrV4) -> Result<(), GuestOwnerFatal> {
        let Ok(Some((cookie, destination))) = self.io.slot_owner(slot) else { return Ok(()) };
        let Some(state) = self.slots.get(&slot) else { return Ok(()) };
        if state.association.is_some() {
            return Ok(());
        }
        {
            let Some(state) = self.slots.get_mut(&slot) else {
                return Err(owner_invariant("ready UDP slot disappeared"));
            };
            state.owner = Some((cookie, destination));
            state.idle_since = self.clock.now();
        }
        self.sync_slot_observation(slot);
        if self.session.is_none()
            || self.pending_outbound.len()
                >= usize::try_from(GUEST_TCP_PENDING_MAX).unwrap_or(usize::MAX)
        {
            return Ok(());
        }
        let flow = self.next_guest_flow_id()?;
        if let Ok(vsock) = self.io.start_connect_host(HostChannel::FlowStream) {
            let Some(state) = self.slots.get_mut(&slot) else {
                self.io.close(vsock.into());
                return Err(owner_invariant("ready UDP slot disappeared"));
            };
            state.association = Some(flow);
            state.last_counters = Some((0, 0));
            self.sync_slot_observation(slot);
            self.datagrams.insert(
                flow,
                DatagramFlow {
                    slot,
                    destination,
                    vsock,
                    deadline: self.clock.now() + FLOW_PAIRING_DEADLINE,
                },
            );
        }
        Ok(())
    }

    fn end_slot_association(
        &mut self,
        slot: SocketAddrV4,
        release: bool,
    ) -> Result<(), GuestOwnerFatal> {
        let flow = self.slots.get(&slot).and_then(|state| state.association);
        if let Some(flow) = flow {
            self.observation.awaiting_paired.remove(&flow);
            if let Some(datagram) = self.datagrams.remove(&flow) {
                self.io.close(datagram.vsock.into());
            }
            let attached = self.slots.get(&slot).and_then(|state| state.attached);
            if let Some(pair) = attached {
                if let Some(close) = self.pending_end_closes.get_mut(&pair) {
                    if let DeferredGuestClose::Datagram { release: pending_release, .. } = close {
                        *pending_release |= release;
                        return Ok(());
                    }
                    return Err(owner_invariant(
                        "a guest datagram slot shares a forwarder id with a stream close",
                    ));
                }
                self.hold_close_until_end(
                    pair,
                    DeferredGuestClose::Datagram { slot, release, flow: Some(flow) },
                );
                self.forwarder_abort(pair)?;
                return Ok(());
            }
        }
        self.finish_slot_association(slot, release, flow);
        Ok(())
    }

    fn hold_close_until_end(
        &mut self,
        id: overdrive_core::guest_transport::forwarder::ForwardId,
        close: DeferredGuestClose,
    ) {
        let previous = self.pending_end_closes.insert(id, close);
        debug_assert!(previous.is_none(), "duplicate deferred close for forwarder pair {id:?}");
    }

    fn finish_slot_association(&mut self, slot: SocketAddrV4, release: bool, flow: Option<FlowId>) {
        self.io.end_association(slot);
        if release {
            self.io.release_slot(slot);
        }
        if let Some(state) = self.slots.get_mut(&slot) {
            state.association = None;
            state.attached = None;
            state.last_counters = None;
            state.idle_since = self.clock.now();
            if release {
                state.owner = None;
            }
        }
        if let Some(flow) = flow
            && release
        {
            self.released_datagram_flows.insert(flow);
        }
        self.sync_slot_observation(slot);
    }

    fn release_slot(&mut self, slot: SocketAddrV4) -> Result<(), GuestOwnerFatal> {
        self.end_slot_association(slot, true)
    }

    fn sync_slot_observation(&mut self, slot: SocketAddrV4) {
        if let (Some(state), Some(observation)) =
            (self.slots.get(&slot), self.observation.slots.get_mut(&slot))
        {
            observation.owner = state.owner;
            observation.association = state.association;
            observation.attached = state.attached;
        }
    }

    fn audit_slots(&mut self) {
        let cookies = self
            .slots
            .values()
            .filter_map(|state| state.owner.map(|(cookie, _)| cookie))
            .collect::<BTreeSet<_>>();
        if let Ok(live) = self.io.live_cookies(&cookies) {
            let lost = self
                .slots
                .iter()
                .filter_map(|(slot, state)| {
                    state.owner.and_then(|(cookie, _)| (!live.contains(&cookie)).then_some(*slot))
                })
                .collect::<Vec<_>>();
            for slot in lost {
                let _ = self.release_slot(slot);
            }
        }
        let attached = self
            .slots
            .iter()
            .filter_map(|(slot, state)| state.attached.map(|_| *slot))
            .collect::<Vec<_>>();
        for slot in attached {
            let Ok(counters) = self.io.slot_counters(slot) else { continue };
            let Some(state) = self.slots.get_mut(&slot) else { continue };
            if state.last_counters != Some(counters) {
                state.last_counters = Some(counters);
                state.idle_since = self.clock.now();
            } else if self.clock.now().saturating_duration_since(state.idle_since)
                >= GUEST_DATAGRAM_SLOT_IDLE
            {
                let _ = self.release_slot(slot);
            }
        }
    }

    fn arm_session_read(&mut self) {
        let Some(session) = self.session.as_ref() else { return };
        if self.io.start_read_host(session, GuestRecord::ControlMessage).is_ok() {
            self.session_read_pending = true;
        } else {
            self.lose_session();
        }
    }

    fn write_session_bytes(&mut self, bytes: &[u8]) -> bool {
        self.session.as_ref().is_some_and(|session| self.io.write_host(session, bytes).is_ok())
    }

    fn write_control(&mut self, message: ControlMessage) {
        if self.session_connected && !self.write_session_bytes(&message.encode()) {
            self.lose_session();
        }
    }

    fn lose_session(&mut self) {
        if let Some(session) = self.session.take() {
            self.io.close(session.into());
        }
        self.session_connected = false;
        self.session_read_pending = false;
        self.observation.session = false;
        self.session_retry = Some(self.clock.now() + CONTROL_RECONNECT_BACKOFF);

        let outbound = std::mem::take(&mut self.pending_outbound);
        for (flow, pending) in outbound {
            self.observation.awaiting_paired.remove(&flow);
            self.io.abort(pending.child.into());
            self.io.close(pending.vsock.into());
        }
        let pending_inbound = std::mem::take(&mut self.pending_inbound);
        for (_, pending) in pending_inbound {
            self.io.abort(pending.conn.into());
        }
        let accepted = std::mem::take(&mut self.accepted);
        for (flow, accepted) in accepted {
            self.observation.accepted.remove(&flow);
            self.io.abort(accepted.app.into());
            self.io.abort(accepted.vsock.into());
        }
        let pairs = std::mem::take(&mut self.tcp_pairs);
        for (flow, pair) in pairs {
            self.observation.flows.remove(&flow);
            self.observation.accepted.remove(&flow);
            let id = pair.id;
            if self.forwarder_abort(id).is_ok() {
                self.hold_close_until_end(
                    id,
                    DeferredGuestClose::Stream { pair, clear_linger: false },
                );
            } else {
                self.io.close(pair.outer.into());
                self.io.close(pair.vsock.into());
            }
        }
        let datagrams = std::mem::take(&mut self.datagrams);
        for (flow, datagram) in datagrams {
            self.observation.awaiting_paired.remove(&flow);
            self.io.close(datagram.vsock.into());
            let _ = self.end_slot_association(datagram.slot, false);
        }
        let attached_slots = self
            .slots
            .iter()
            .filter_map(|(slot, state)| state.attached.map(|_| *slot))
            .collect::<Vec<_>>();
        for slot in attached_slots {
            let _ = self.end_slot_association(slot, false);
        }
    }

    fn abort_tcp_pair(&mut self, flow: FlowId) -> Result<(), GuestOwnerFatal> {
        let Some(pair) = self.tcp_pairs.remove(&flow) else { return Ok(()) };
        self.observation.flows.remove(&flow);
        self.observation.accepted.remove(&flow);
        let id = pair.id;
        self.hold_close_until_end(id, DeferredGuestClose::Stream { pair, clear_linger: false });
        self.forwarder_abort(id)?;
        Ok(())
    }

    fn forwarder_abort(
        &mut self,
        id: overdrive_core::guest_transport::forwarder::ForwardId,
    ) -> Result<(), GuestOwnerFatal> {
        self.io
            .abort_forward(id)
            .map(|_| ())
            .map_err(|source| GuestOwnerFatal::Forwarder { source })
    }
}

fn errno(raw: i32) -> KernelErrno {
    let raw = raw.clamp(1, i32::from(u16::MAX));
    let value = u16::try_from(raw).unwrap_or(u16::MAX);
    KernelErrno::new(value).unwrap_or(KernelErrno::ENODEV)
}

const fn error_errno(kind: ErrorKind) -> i32 {
    match kind {
        ErrorKind::ConnectionRefused => libc::ECONNREFUSED,
        ErrorKind::ConnectionReset => libc::ECONNRESET,
        ErrorKind::ConnectionAborted => libc::ECONNABORTED,
        ErrorKind::TimedOut => libc::ETIMEDOUT,
        ErrorKind::PermissionDenied => libc::EACCES,
        ErrorKind::NotFound => libc::ENOENT,
        ErrorKind::AddrInUse => libc::EADDRINUSE,
        ErrorKind::AddrNotAvailable => libc::EADDRNOTAVAIL,
        ErrorKind::NetworkUnreachable => libc::ENETUNREACH,
        ErrorKind::HostUnreachable => libc::EHOSTUNREACH,
        ErrorKind::BrokenPipe => libc::EPIPE,
        ErrorKind::NotConnected => libc::ENOTCONN,
        _ => libc::EIO,
    }
}

fn owner_invariant(message: &'static str) -> GuestOwnerFatal {
    GuestOwnerFatal::EventWait { source: io::Error::other(format!("owner invariant: {message}")) }
}

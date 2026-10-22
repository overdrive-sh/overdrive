//! Driving port — guest stream dial (INB-1, ADR-0173, ADR-0174; the
//! `purpose` parameter is PROBE-NOSAMPLE, ADR-0187).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Driving port — guest stream dial `GuestStreamDial`". A host
//! platform client (leg-S of the inbound mTLS intercept, the VM probe runner)
//! reaches a TCP port of a VM only through a host-opened vsock flow the
//! guest-flow owner opens. The production implementation is the guest-flow
//! owner (`overdrive-control-plane::guest_network`); the simulation
//! implementation is `overdrive-sim::SimGuestStreamDial`.
//!
//! The stream's end is resolved from the owner's forwarder watch of the host
//! vsock (EVMOD-STREAM-END), and an owner abort ends it without touching the
//! socket (EVMOD-STREAM-ABORT, WATCH-ABORT).
//!
use std::net::SocketAddrV4;

use super::wire::KernelErrno;

/// INB-1 (ADR-0173): opens a host-initiated TCP connection to a declared
/// listen port of an Active VM allocation, carried by one host-opened vsock
/// flow. Production impl: the guest-flow owner
/// (`overdrive-control-plane::guest_network`); sim impl:
/// `overdrive-sim::SimGuestStreamDial`. A required constructor argument of
/// every consumer (leg-S, the VM probe runner); never defaulted.
///
/// # Contract
///
/// - **Preconditions:** none on the caller. Every `target` is accepted; an
///   address outside the guest prefix, the gateway, port 0 and an undeclared
///   port are answered [`GuestDialError::NotServing`], never a panic.
/// - **Order of decision** (in the target CID's serialized order): not the
///   workload address of an Active lease or not one of its declared TCP
///   listen ports → `NotServing`; node forwarding quiesced → `Quiesced`; the
///   CID's control session not live → `NoSession`; the node's flow-slot
///   capacity full, or — for [`DialPurpose::Relay`] only — the allocation's
///   flow quota full → `Capacity`. Each of these returns immediately and
///   changes nothing.
/// - **Outcome:** the guest's pairing reply decides it: `Paired` →
///   `Ok(GuestStream)`; `Refused { errno }` → `Refused { errno }`;
///   `TimedOut { errno }` → `TimedOut { errno }`; no reply within
///   `GUEST_DIAL_PAIRING_DEADLINE` from a VM whose VMM still runs →
///   `PairingDeadline`; a vsock connect or I/O failure of the host vsock →
///   `Unreachable { errno }`. Every error closes the host vsock and releases
///   the slot (and a `Relay` dial's quota slot) before the result returns.
/// - **In-flight aborts:** the first quiescence hold → `Quiesced`; teardown
///   of the allocation → `NotServing`; the control-session loss the owner
///   observed → `NoSession`.
/// - **Late replies** for a dial that has returned are discarded and never
///   yield a stream.
/// - **Cancellation:** dropping the future aborts the dial exactly as the
///   pairing deadline does; no slot outlives its future.
/// - **Concurrency:** dials are independent; none blocks another.
#[async_trait::async_trait]
pub trait GuestStreamDial: Send + Sync {
    /// Dials `target` for `purpose`. See the trait-level contract.
    async fn dial(
        &self,
        target: SocketAddrV4,
        purpose: DialPurpose,
    ) -> Result<GuestStream, GuestDialError>;
}

/// PROBE-NOSAMPLE (ADR-0187): what a dial is for. It decides only whether the
/// dial counts against the target allocation's flow quota; every other step
/// of `dial` is the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialPurpose {
    /// leg-S relaying a client connection. Takes a node flow slot and counts
    /// against the allocation's flow quota, like every other flow.
    Relay,
    /// The VM probe runner. Takes a node flow slot; does not count against
    /// the allocation's flow quota. Each probe task holds at most one dial in
    /// flight, so the exemption is bounded by the probe count the
    /// allocation's spec declares (at most `GUEST_PROBE_SLOTS`). In every
    /// other respect a probe stream is a relay stream: `shutdown_write`
    /// half-closes it, and its drop aborts its watch and releases the node
    /// slot at once (EVMOD-AB).
    Probe,
}

impl DialPurpose {
    /// The purpose's canonical label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Relay => "relay",
            Self::Probe => "probe",
        }
    }
}

/// The host end of a paired host-opened flow: the connected host vsock
/// `V_h`, owned until the stream is dropped. The holder (leg-S's relay, the
/// probe runner) reads and writes the bytes itself; the forwarder moves none
/// of them. The owner **watches** `V_h` (`FlowForwarder::watch`, set up when
/// `V_h` connects, before the request is written) [EVMOD-END,
/// EVMOD-STREAM-END], so the guest's end of its direction is classified by
/// the forwarder's callback, not by the socket call that observed it.
///
/// # Contract
///
/// - **How an end is resolved (EVMOD-STREAM-END).** Every end the stream
///   observes on `V_h` — a `read` of 0, any `read` error (`ENOTCONN` and
///   `ECONNRESET` included), any `write` error — is resolved from the watch,
///   in this order: the owner's own end of the watch
///   (`Err(ConnectionAborted)`); else the watch's end record
///   (`GuestHalfClose` → `Ok(0)` for a read; `Aborted { .. }` →
///   `Err(ConnectionReset)`); else the record is in flight and the call waits
///   for it. A read's return is never the classification.
/// - **Reads.** `Ok(n > 0)`: guest application bytes in order. `Ok(0)`: the
///   guest direction ended with [`GuestStreamEnd::HalfClose`] and every byte
///   before it has been read. `Err(ConnectionReset)`: the guest direction
///   ended with [`GuestStreamEnd::Abort`]. `Err(ConnectionAborted)`: the
///   owner ended the stream (EVMOD-STREAM-ABORT).
/// - **Writes.** `Ok(n)` while the host→guest direction is open, also after
///   the guest half-closed its own direction. After `shutdown_write`:
///   `Err(BrokenPipe)`. Any `V_h` write error is `Err(ConnectionReset)`
///   unless the owner ended the stream, which is `Err(ConnectionAborted)`.
/// - **`shutdown_write`** sends a vsock SHUTDOWN with flags 0x2; a second
///   call returns `Ok(())` and sends nothing; after an owner abort,
///   `Err(ConnectionAborted)`. The same for both purposes (EVMOD-AB).
/// - **`end`** is `None` until the guest direction has ended; then the class,
///   fixed for the stream's life, resolved as the reads are. An owner abort
///   reports `Some(Abort)`.
/// - **Owner abort (EVMOD-STREAM-ABORT, WATCH-ABORT).** Quiescence's first
///   hold, the allocation's teardown, control-session loss and the
///   `Forwarder` recovery end the stream at once: the watch's record is
///   queued and `V_h` is untouched; every pending and later `read`, `write`
///   and `shutdown_write` returns `Err(ConnectionAborted)` without waiting on
///   `V_h`.
/// - **Drop.** Move-only. Dropping it aborts the watch, then closes `V_h`
///   (the guest receives `0x3`), in one step; a stream the owner already
///   ended only closes `V_h`. The node flow slot and (for `Relay`) the quota
///   place are released at the drop or the owner's abort, whichever comes
///   first (EVMOD-AB).
/// - **Cancellation.** A `read`, `write` or `shutdown_write` future dropped
///   before it completes consumes no byte and changes no state.
///
/// `GuestStream` itself delegates each call to its [`GuestStreamIo`]
/// unchanged; the rules above are the I/O's contract.
#[must_use]
#[derive(Debug)]
pub struct GuestStream {
    target: SocketAddrV4,
    io: Box<dyn GuestStreamIo>,
}

impl GuestStream {
    /// Constructed by a [`GuestStreamDial`] implementation only.
    pub fn new(target: SocketAddrV4, io: Box<dyn GuestStreamIo>) -> Self {
        Self { target, io }
    }

    /// The address `dial` was called with.
    #[expect(
        clippy::missing_const_for_fn,
        reason = "the feature delta pins this as a non-const public accessor"
    )]
    pub fn target(&self) -> SocketAddrV4 {
        self.target
    }

    /// Reads guest application bytes; see the type-level contract.
    pub async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.io.read(buf).await
    }

    /// Writes bytes toward the guest application; see the type-level
    /// contract.
    pub async fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.io.write(buf).await
    }

    /// Ends the host→guest direction cleanly: the guest receives a vsock
    /// SHUTDOWN with flags 0x2 and half-closes the application connection.
    pub async fn shutdown_write(&mut self) -> std::io::Result<()> {
        self.io.shutdown_write().await
    }

    /// The classified end of the guest→host direction, once it has ended.
    pub fn end(&self) -> Option<GuestStreamEnd> {
        self.io.end()
    }
}

/// The I/O behind a [`GuestStream`]: the owner's vsock adapter in production
/// (which resolves every end from the watch and is woken by the owner on an
/// owner abort, EVMOD-STREAM-ABORT), an in-memory pipe in `overdrive-sim`.
/// Same four operations, same contract as the `GuestStream` methods that
/// delegate to it.
#[async_trait::async_trait]
pub trait GuestStreamIo: Send + std::fmt::Debug {
    /// See [`GuestStream::read`].
    async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
    /// See [`GuestStream::write`].
    async fn write(&mut self, buf: &[u8]) -> std::io::Result<usize>;
    /// See [`GuestStream::shutdown_write`].
    async fn shutdown_write(&mut self) -> std::io::Result<()>;
    /// See [`GuestStream::end`].
    fn end(&self) -> Option<GuestStreamEnd>;
}

/// R2-END: how the guest ended its direction of the flow, as the watch's end
/// record reports it [EVMOD-REC, EVMOD-STREAM-END].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestStreamEnd {
    /// The guest sent a vsock SHUTDOWN with flags 0x2 (SEND only): the guest
    /// application half-closed its connection (record `GuestHalfClose`).
    HalfClose,
    /// The guest sent 0x3 while a direction was open, or an RST; or the VM
    /// ended (the device release's `sk_err`); or the owner ended the stream
    /// (record `Aborted { cause }`, or the owner's own end).
    Abort,
}

impl GuestStreamEnd {
    /// The end class's canonical label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HalfClose => "half_close",
            Self::Abort => "abort",
        }
    }
}

/// Why a dial returned no stream. Returned to the dialer only; it never
/// enters `GuestNetworkError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GuestDialError {
    /// The guest's connect to the application failed (its `Refused` reply);
    /// `ECONNREFUSED` when nothing listens on the port.
    #[error("the guest refused the connection: errno {errno}")]
    Refused {
        /// The guest kernel's errno.
        errno: KernelErrno,
    },
    /// The guest's connect to the application did not complete within
    /// `GUEST_INBOUND_CONNECT_TIMEOUT` (its `TimedOut` reply), e.g. a full
    /// listen backlog.
    #[error("the guest's connect to the application timed out: errno {errno}")]
    TimedOut {
        /// The guest kernel's errno (`ETIMEDOUT`).
        errno: KernelErrno,
    },
    /// No pairing reply within `GUEST_DIAL_PAIRING_DEADLINE` from a VM whose
    /// VMM is still running (a VMM exit ends the dial `Unreachable`).
    #[error("no pairing reply from the guest within the pairing deadline")]
    PairingDeadline,
    /// The address is not the workload address of an Active VM allocation,
    /// or the port is not one of its declared TCP listen ports; also the
    /// result of an in-flight dial that the allocation's teardown ended.
    #[error("no Active VM allocation serves this address and port")]
    NotServing,
    /// The allocation is Active but its control session is not live as the
    /// owner observes it, so no pairing reply could be delivered; also the
    /// result of an in-flight dial ended by the session loss the owner
    /// observed.
    #[error("the VM's control session is not live")]
    NoSession,
    /// Node forwarding is quiesced (ADR-0169); also the result of an
    /// in-flight dial that the first quiescence hold ended.
    #[error("node forwarding is quiesced")]
    Quiesced,
    /// The VM or its vsock device is gone while the owner still holds the
    /// session live: the vsock connect failed (`ENODEV` when no device holds
    /// the CID), or the VMM exited with the dial in flight and the device
    /// release reset the host vsock (`ECONNRESET`), or another connect or I/O
    /// error of the host vsock before pairing.
    #[error("the VM is unreachable over vsock: errno {errno}")]
    Unreachable {
        /// The host kernel's errno.
        errno: KernelErrno,
    },
    /// The node's flow-slot capacity is exhausted, or, for a
    /// [`DialPurpose::Relay`] dial, the allocation's flow quota is. A
    /// [`DialPurpose::Probe`] dial is never refused for the allocation quota
    /// (PROBE-NOSAMPLE).
    #[error("guest flow capacity exhausted")]
    Capacity,
}

impl GuestDialError {
    /// The error's canonical label (telemetry `guest_dial.failed { error }`).
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "the signature pinned by feature-delta § Driving port — guest stream dial takes `&self`"
    )]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Refused { .. } => "refused",
            Self::TimedOut { .. } => "timed_out",
            Self::PairingDeadline => "pairing_deadline",
            Self::NotServing => "not_serving",
            Self::NoSession => "no_session",
            Self::Quiesced => "quiesced",
            Self::Unreachable { .. } => "unreachable",
            Self::Capacity => "capacity",
        }
    }
}

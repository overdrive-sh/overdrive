//! Test support shared by the guest flow owner's acceptance tests and its
//! quint-connect conformance drivers (`#[cfg(test)]`).
//!
//! **Quiescence (DG-5-Q).** A single-threaded driver calls
//! `GuestFlowOwner::dispatch_next` only when it would not block: while the
//! sim holds a released, undelivered event
//! (`SimGuestOwnerIo::released_undelivered`), or while the owner's
//! `next_deadline()` is `Some(d)` with `d <= clock.now()`. When neither holds
//! the owner is quiescent — the QPROJ-5 consistency point "between two owner
//! steps" — and [`settle`] returns. No panic is raised or caught to detect
//! quiescence, and `next_event`'s `Err` keeps its production meaning (fatal).
//!
//! **The first dispatch.** DG-5: "The first dispatch with no session starts
//! the control-session connect." [`start`] makes that first call; every later
//! dispatch goes through [`settle`].
//!
//! **Timer actions (DG-5-A).** A driver's time action advances the
//! `SimGuestClock` to exactly the production deadline it names, then settles:
//! advancing the clock past several deadlines yields one dispatch per
//! deadline, in deadline order.

#![allow(
    clippy::panic,
    reason = "a settle that never quiesces or meets a GuestOwnerFatal is a test defect"
)]

use std::time::Instant;

use super::sim::{SimGuestClock, SimGuestOwnerIo};
use super::{GuestClock, GuestFlowOwner};

/// The bound on dispatches of one `settle`: a guest that keeps finding work
/// past it is a livelocked owner (or a driver feeding it forever).
const SETTLE_BOUND: usize = 100_000;

/// The owner's first dispatch (DG-5: it starts the control-session connect).
/// A `GuestOwnerFatal` panics here: no path using this expects one.
pub(super) fn start(owner: &mut GuestFlowOwner) {
    if let Err(fatal) = owner.dispatch_next() {
        panic!("a GuestOwnerFatal on the owner's first dispatch: {fatal}");
    }
}

/// The owner would not block: an event is released and undelivered, or its
/// earliest deadline is due on `clock`.
pub(super) fn ready(owner: &GuestFlowOwner, io: &SimGuestOwnerIo, clock: &SimGuestClock) -> bool {
    io.released_undelivered() || owner.next_deadline().is_some_and(|d| d <= clock.now())
}

/// Dispatches while [`ready`] holds (DG-5-Q), then returns. A
/// `GuestOwnerFatal` panics here (no path using this expects one).
pub(super) fn settle(owner: &mut GuestFlowOwner, io: &SimGuestOwnerIo, clock: &SimGuestClock) {
    for _ in 0..SETTLE_BOUND {
        if !ready(owner, io, clock) {
            return;
        }
        if let Err(fatal) = owner.dispatch_next() {
            panic!("a GuestOwnerFatal on a path that never expects one: {fatal}");
        }
    }
    panic!("the guest owner did not settle within {SETTLE_BOUND} dispatches");
}

/// Advances `clock` to exactly `at` (DG-5-A), then settles.
pub(super) fn advance_to(
    owner: &mut GuestFlowOwner,
    io: &SimGuestOwnerIo,
    clock: &SimGuestClock,
    at: Instant,
) {
    clock.advance_to(at);
    settle(owner, io, clock);
}

/// The guest owner process's lifecycle, beside the owner a conformance driver
/// holds directly (S-7, orchestrator lint ruling 2026-10-19: the driver's
/// `GuestFlowOwner` field is set at construction and is the only path to the
/// owner; this enum is test support for the process's life and never holds
/// the owner). `NotStarted`: built, its first dispatch not taken. `Alive`:
/// started and dispatching. `Dead`: the driver ended the guest process — the
/// seam's process exit (`SimGuestOwnerIo::guest_owner_dies`) — and the owner
/// field is kept, never called or projected again (the caller's own
/// observation, QPROJ-1 source (c)).
#[cfg(feature = "integration-tests")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuestProcess {
    NotStarted,
    Alive,
    Dead,
}

#[cfg(feature = "integration-tests")]
impl GuestProcess {
    /// The owner has taken its first dispatch and has not died.
    pub(super) const fn is_alive(self) -> bool {
        matches!(self, Self::Alive)
    }

    /// The driver ended the guest process.
    pub(super) const fn is_dead(self) -> bool {
        matches!(self, Self::Dead)
    }
}

/// One connection of the R5-59 (b) accept-path equivalence table: a
/// `TcpAccept` V_g opened on `GUEST_INBOUND_PORT` with its 32-byte request
/// buffered (`request_buffered`) or with no byte at all. The oracle is per
/// connection and order-insensitive: a connection with its request is served
/// (the application at `workload_addr:port` accepts it and `payload` reaches
/// it); one with no record yields no application connection. The sim half is
/// `tests::tcp_accept::accept_path_outcomes_by_first_record`; the real-kernel
/// half is `real_kernel::the_accept_path_matches_the_sim_per_connection`.
#[derive(Debug, Clone, Copy)]
pub(super) struct AcceptPathCase {
    /// The host-origin flow sequence number.
    pub(super) seq: u32,
    /// The declared guest listen port.
    pub(super) port: u16,
    /// The request is buffered on the connection.
    pub(super) request_buffered: bool,
    /// The bytes the host writes after `Paired` (served connections only).
    pub(super) payload: &'static [u8],
}

/// The R5-59 (b) connection table (D-UP-19: distinct connections, a first
/// record buffered or none, in an order the accept queue does not fix).
pub(super) const ACCEPT_PATH_CASES: [AcceptPathCase; 4] = [
    AcceptPathCase { seq: 1, port: 8080, request_buffered: true, payload: b"first" },
    AcceptPathCase { seq: 2, port: 8080, request_buffered: false, payload: b"" },
    AcceptPathCase { seq: 3, port: 8081, request_buffered: true, payload: b"third" },
    AcceptPathCase { seq: 4, port: 8081, request_buffered: false, payload: b"" },
];

//! Test support shared by the guest flow owner's acceptance tests and its
//! quint-connect conformance drivers (`#[cfg(test)]`).
//!
//! **Quiescence (`settle`).** The driver stops dispatching exactly when the
//! owner reaches a blocking wait with nothing to deliver. That is the only
//! exact signal available: the owner computes its own earliest deadline and
//! only `GuestOwnerIo::next_event` learns it (a deadline armed while handling
//! the last input is invisible to any read made before that call), and DG-5
//! pins the sim's answer to exactly "nothing released, deadline not passed":
//! it panics rather than block a single-threaded driver. The sim raises that
//! panic with the typed payload [`SimWouldBlock`]; `settle` catches only that
//! payload, so it stops at the owner's blocking wait — the QPROJ-5
//! consistency point "between two owner steps" — and re-raises every other
//! panic (a RED scaffold, an owner or seam-contract defect). It never injects
//! an `Err` into `next_event`: that is the owner's fatal `EventWait`.
//!
//! The wait's deadline is recorded by the sim (`armed_deadline`), so a
//! driver's time action advances the clock to it and settles again: once the
//! deadline is at or before now, `next_event` returns `Ok(None)` and the owner
//! dispatches its due timer.
//!
//! The unwind runs through the owner's `dispatch_next`. An owner that starts
//! effects before its wait and keeps their handles only in locals would have
//! them dropped by the unwind; the sim still records those effects
//! (`effects`, its socket table), so the owner's state and the seam's diverge
//! and the oracle reading both fails — loud, never silent. The sim has no
//! dedicated leaked-handle report. The residual is upstream UI-R2-22: a
//! pinned non-unwinding quiescence signal (an owner read of its next
//! deadline, or a blocking sim with the owner on its own thread and an
//! observation hand-off) is a DESIGN decision, not test support's to invent.

#![allow(
    clippy::panic,
    reason = "a settle that never quiesces or meets a GuestOwnerFatal is a test defect"
)]

use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

use super::sim::{SimGuestOwnerIo, SimWouldBlock};
use super::{GuestFlowOwner, GuestOwnerFatal};

thread_local! {
    /// Set while `settle` dispatches on this thread: its `SimWouldBlock`
    /// panic is the expected quiescence signal and is not printed.
    static SETTLING: Cell<bool> = const { Cell::new(false) };
}

/// Installs, once per process, a panic hook that silences a `SimWouldBlock`
/// raised inside `settle` and delegates every other panic to the previous
/// hook.
fn install_quiet_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if info.payload().is::<SimWouldBlock>() {
                if SETTLING.with(Cell::get) {
                    return;
                }
                eprintln!(
                    "SimGuestOwnerIo: next_event blocked with nothing released and no deadline \
                     passed outside settle: {:?}",
                    info.payload().downcast_ref::<SimWouldBlock>()
                );
            }
            previous(info);
        }));
    });
}

/// Clears `SETTLING` on every exit from `settle`, including an unwind.
struct Settling;

impl Settling {
    fn enter() -> Self {
        SETTLING.with(|s| s.set(true));
        Self
    }
}

impl Drop for Settling {
    fn drop(&mut self) {
        SETTLING.with(|s| s.set(false));
    }
}

/// Dispatches until the owner reaches a blocking wait with nothing to
/// deliver (the sim's [`SimWouldBlock`] panic; see the module doc), then
/// returns. Every other panic is re-raised; a `GuestOwnerFatal` panics here
/// (no path using this expects one).
pub(super) fn settle(owner: &mut GuestFlowOwner, io: &SimGuestOwnerIo) {
    install_quiet_hook();
    let _settling = Settling::enter();
    for _ in 0..1_000_000 {
        match panic::catch_unwind(AssertUnwindSafe(|| owner.dispatch_next())) {
            Ok(Ok(())) => {}
            Ok(Err(fatal)) => fail(&fatal, io),
            Err(payload) if payload.is::<SimWouldBlock>() => return,
            Err(payload) => panic::resume_unwind(payload),
        }
    }
    panic!("the guest owner did not settle");
}

fn fail(fatal: &GuestOwnerFatal, io: &SimGuestOwnerIo) -> ! {
    panic!(
        "a GuestOwnerFatal on a path that never expects one: {fatal} (last wait: {:?})",
        io.last_wait_record()
    );
}

/// The guest owner process a conformance driver holds: not started yet, alive
/// (the production owner under test), or dead (the driver dropped it — the
/// caller's own observation, QPROJ-1 source (d)).
#[cfg(feature = "integration-tests")]
pub(super) enum GuestProcess {
    NotStarted,
    Alive(GuestFlowOwner),
    Dead,
}

#[cfg(feature = "integration-tests")]
impl GuestProcess {
    /// The live owner, if any.
    pub(super) const fn owner(&self) -> Option<&GuestFlowOwner> {
        match self {
            Self::Alive(owner) => Some(owner),
            Self::NotStarted | Self::Dead => None,
        }
    }

    /// The live owner, mutably, if any.
    pub(super) const fn owner_mut(&mut self) -> Option<&mut GuestFlowOwner> {
        match self {
            Self::Alive(owner) => Some(owner),
            Self::NotStarted | Self::Dead => None,
        }
    }

    /// The driver dropped the owner.
    pub(super) const fn is_dead(&self) -> bool {
        matches!(self, Self::Dead)
    }
}

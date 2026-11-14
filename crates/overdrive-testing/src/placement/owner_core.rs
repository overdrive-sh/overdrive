//! The shared-guest owner's pure core: events in, effects out.
//!
//! It reads no clock and holds no handle. The owner shell executes the effects against the guest and the
//! socket; the conformance driver replays the Quint model against this core.

#![allow(
    clippy::todo,
    clippy::missing_const_for_fn,
    clippy::must_use_candidate,
    clippy::new_without_default,
    clippy::needless_pass_by_ref_mut,
    reason = "scaffold bodies are `todo!()` until the shared-guest owner is built"
)]

/// One attached client connection; the shell numbers its accepted connections.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct AttachId(u32);

impl AttachId {
    /// The id of the `n`th accepted connection.
    pub const fn new(n: u32) -> Self {
        Self(n)
    }
}

/// An event the shell delivers to the core.
pub enum Event {
    /// A client connected.
    Attached(AttachId),
    /// A client went away.
    ClientGone(AttachId),
    /// The boot ended: `true` the guest is ready, `false` the boot failed.
    BootFinished(bool),
    /// The exec ended with this exit status.
    ExecFinished(AttachId, i32),
    /// The guest was lost.
    GuestLost,
    /// The exec bound elapsed for this client's exec.
    ExecBoundElapsed(AttachId),
    /// The owner has been idle for the idle period.
    IdleElapsed,
}

/// An effect the shell executes.
pub enum Effect {
    /// Start booting a guest.
    StartBoot,
    /// Start the client's exec in the guest.
    StartExec(AttachId),
    /// Answer the client.
    Reply(AttachId, Verdict),
    /// Discard the guest.
    DiscardGuest,
    /// Exit the owner.
    Exit,
}

/// The verdict a client's request ends with.
pub enum Verdict {
    /// The test ran and exited with this status.
    Exit(i32),
    /// The boot failed.
    BootFailed,
    /// The guest was lost.
    GuestLost,
    /// The exec bound ended the exec.
    ExecBound,
}

/// The guest as the core observes it.
pub enum GuestView {
    /// No guest.
    NoGuest,
    /// A guest is booting.
    Booting,
    /// A booted guest is ready.
    Ready,
}

/// The pinned observation the conformance driver projects.
pub struct OwnerObservation {
    /// The guest's state.
    pub guest: GuestView,
    /// The current or last guest generation.
    pub generation: u32,
    /// The client whose exec is in flight.
    pub in_flight: Option<AttachId>,
    /// The attached clients waiting, in arrival order.
    pub queue: Vec<AttachId>,
    /// True after `Effect::Exit`.
    pub stopped: bool,
}

/// The owner's state machine.
pub struct OwnerCore {
    _private: (),
}

impl OwnerCore {
    /// A fresh owner: no guest, generation 0, nothing queued, not stopped.
    // SCAFFOLD: true
    pub fn new() -> Self {
        todo!("OwnerCore::new")
    }

    /// Applies one event and returns the effects it causes.
    // SCAFFOLD: true
    pub fn handle(&mut self, event: Event) -> Vec<Effect> {
        let _ = event;
        todo!("OwnerCore::handle")
    }

    /// The observation the conformance driver projects.
    // SCAFFOLD: true
    pub fn observe(&self) -> OwnerObservation {
        todo!("OwnerCore::observe")
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "integration-tests"))]
mod conformance;

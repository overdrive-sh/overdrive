//! The shared-guest owner's pure core: events in, effects out.
//!
//! It reads no clock and holds no handle. The owner shell executes the effects against the guest and the
//! socket; the conformance driver replays the Quint model against this core.

use std::collections::VecDeque;

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
#[derive(Clone, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuestView {
    /// No guest.
    NoGuest,
    /// A guest is booting.
    Booting,
    /// A booted guest is ready.
    Ready,
}

/// The pinned observation the conformance driver projects.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    guest: GuestView,
    generation: u32,
    in_flight: Option<AttachId>,
    /// The in-flight exec's client went away: its result reaches nobody and its guest is discarded
    /// when it ends.
    detached: bool,
    queue: VecDeque<AttachId>,
    stopped: bool,
}

impl Default for OwnerCore {
    fn default() -> Self {
        Self::new()
    }
}

impl OwnerCore {
    /// A fresh owner: no guest, generation 0, nothing queued, not stopped.
    pub const fn new() -> Self {
        Self {
            guest: GuestView::NoGuest,
            generation: 0,
            in_flight: None,
            detached: false,
            queue: VecDeque::new(),
            stopped: false,
        }
    }

    /// Applies one event and returns the effects it causes. An event that is not enabled in the
    /// state the owner is in returns no effects and changes nothing.
    pub fn handle(&mut self, event: Event) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.stopped {
            return effects;
        }
        match event {
            Event::Attached(client) => self.attached(client),
            Event::ClientGone(client) => self.client_gone(client),
            Event::BootFinished(ready) => self.boot_finished(ready, &mut effects),
            Event::ExecFinished(client, status) => {
                self.exec_finished(client, status, &mut effects);
            }
            Event::GuestLost => self.guest_lost(&mut effects),
            Event::ExecBoundElapsed(client) => self.exec_bound_elapsed(client, &mut effects),
            Event::IdleElapsed => return self.idle_elapsed(),
        }
        self.drive(&mut effects);
        effects
    }

    /// The observation the conformance driver projects.
    pub fn observe(&self) -> OwnerObservation {
        OwnerObservation {
            guest: self.guest.clone(),
            generation: self.generation,
            in_flight: self.in_flight,
            queue: self.queue.iter().copied().collect(),
            stopped: self.stopped,
        }
    }

    fn knows(&self, client: AttachId) -> bool {
        self.in_flight == Some(client) || self.queue.contains(&client)
    }

    fn attached(&mut self, client: AttachId) {
        if !self.knows(client) {
            self.queue.push_back(client);
        }
    }

    fn client_gone(&mut self, client: AttachId) {
        if self.in_flight == Some(client) {
            self.detached = true;
            return;
        }
        self.queue.retain(|queued| *queued != client);
    }

    fn boot_finished(&mut self, ready: bool, effects: &mut Vec<Effect>) {
        if self.guest != GuestView::Booting {
            return;
        }
        if ready {
            self.guest = GuestView::Ready;
            return;
        }
        self.guest = GuestView::NoGuest;
        if let Some(head) = self.queue.pop_front() {
            effects.push(Effect::Reply(head, Verdict::BootFailed));
        }
    }

    /// True when `client`'s exec is the one in flight on a ready guest.
    fn runs_on_ready_guest(&self, client: AttachId) -> bool {
        self.guest == GuestView::Ready && self.in_flight == Some(client)
    }

    fn exec_finished(&mut self, client: AttachId, status: i32, effects: &mut Vec<Effect>) {
        if !self.runs_on_ready_guest(client) {
            return;
        }
        let discard = status != 0 || self.detached;
        self.answer_in_flight(client, Verdict::Exit(status), effects);
        if discard {
            self.discard_guest(effects);
        }
    }

    fn exec_bound_elapsed(&mut self, client: AttachId, effects: &mut Vec<Effect>) {
        if !self.runs_on_ready_guest(client) {
            return;
        }
        self.answer_in_flight(client, Verdict::ExecBound, effects);
        self.discard_guest(effects);
    }

    fn guest_lost(&mut self, effects: &mut Vec<Effect>) {
        if self.guest != GuestView::Ready {
            return;
        }
        if let Some(client) = self.in_flight {
            self.answer_in_flight(client, Verdict::GuestLost, effects);
        }
        self.guest = GuestView::NoGuest;
    }

    /// Ends the in-flight exec, replying to its client unless that client went away.
    fn answer_in_flight(&mut self, client: AttachId, verdict: Verdict, effects: &mut Vec<Effect>) {
        if !self.detached {
            effects.push(Effect::Reply(client, verdict));
        }
        self.in_flight = None;
        self.detached = false;
    }

    fn discard_guest(&mut self, effects: &mut Vec<Effect>) {
        self.guest = GuestView::NoGuest;
        effects.push(Effect::DiscardGuest);
    }

    fn idle_elapsed(&mut self) -> Vec<Effect> {
        if !self.queue.is_empty() || self.in_flight.is_some() {
            return Vec::new();
        }
        self.guest = GuestView::NoGuest;
        self.stopped = true;
        vec![Effect::Exit]
    }

    /// With no exec in flight and a client waiting: serve the head on a ready guest, or boot one.
    fn drive(&mut self, effects: &mut Vec<Effect>) {
        if self.in_flight.is_some() || self.queue.is_empty() {
            return;
        }
        match self.guest {
            GuestView::Ready => {
                self.in_flight = self.queue.pop_front();
                self.detached = false;
                effects.extend(self.in_flight.map(Effect::StartExec));
            }
            GuestView::NoGuest => {
                self.guest = GuestView::Booting;
                self.generation += 1;
                effects.push(Effect::StartBoot);
            }
            GuestView::Booting => {}
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "integration-tests"))]
mod conformance;

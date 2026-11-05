//! `GuestListenerBoard` — the host-side board of the listeners a guest has
//! reported over the beacon channel (D13 item 10, PR-1).
//!
//! SCAFFOLD: true — RED scaffold created by DISTILL. The design pins only the
//! type's existence, that it implements
//! [`overdrive_core::traits::GuestListenerReports`], and its lifecycle
//! behaviour (entry created by `VmDriver::start` before the beacon reader runs,
//! removed by `VmDriver::on_alloc_terminal` after `ProbeRunner::stop_alloc` and
//! on a failed `start`; insertion-reporting check-and-act; three no-op cases).
//! Its methods are the crafter's: tests drive it only through the beacon reader
//! and `VmDriver`, and read it through the port. `new` is the one constructor
//! the composition roots (and so every test composing a `VmDriver`) need; the
//! design does not pin its signature, which is a recorded design gap.

use async_trait::async_trait;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::{GuestListenerReports, ReportedListeners};

/// Host-side board of reported guest listeners. See the module docs.
#[derive(Debug, Default)]
pub struct GuestListenerBoard {
    _scaffold: (),
}

impl GuestListenerBoard {
    /// An empty board with no entries.
    #[must_use]
    #[allow(
        clippy::missing_const_for_fn,
        reason = "the pinned shape is `new() -> Self`; the real board's constructor is not const"
    )]
    pub fn new() -> Self {
        Self { _scaffold: () }
    }
}

#[async_trait]
impl GuestListenerReports for GuestListenerBoard {
    fn reported(&self, _alloc: &AllocationId) -> ReportedListeners {
        // SCAFFOLD: true
        panic!("RED scaffold: PR-1 GuestListenerBoard::reported")
    }

    async fn changed(&self, _alloc: &AllocationId, _seen: &ReportedListeners) -> ReportedListeners {
        // SCAFFOLD: true
        panic!("RED scaffold: PR-1 GuestListenerBoard::changed")
    }
}

//! [`GuestListenerReports`] — the read-only port over the set of workload
//! listeners a guest has reported over the beacon channel (D13 item 10, PR-1).
//!
//! SCAFFOLD: true — RED scaffold created by DISTILL for the exact shape the
//! design pins in `feature-delta.md` § "D13 item 10" and § "Interface changes
//! (D13 to D16)". The trait and the value type are pinned, including the value
//! type's construction surface (`Default`, `FromIterator<Listener>`, `is_empty`;
//! DG-6).

use std::collections::BTreeSet;
use std::num::NonZeroU16;

use async_trait::async_trait;

use crate::aggregate::Listener;
use crate::dataplane::backend_key::Proto;
use crate::id::AllocationId;

/// An ordered set of `(Proto, NonZeroU16)` — the listeners a guest has reported
/// open for one allocation. A value type: `contains`, `len` and equality.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportedListeners {
    pairs: BTreeSet<(Proto, NonZeroU16)>,
}

impl ReportedListeners {
    /// True when the pair is in the set.
    #[must_use]
    pub fn contains(&self, protocol: Proto, port: NonZeroU16) -> bool {
        self.pairs.contains(&(protocol, port))
    }

    /// Number of distinct pairs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    /// True when no pair has been reported.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }
}

impl FromIterator<Listener> for ReportedListeners {
    fn from_iter<I: IntoIterator<Item = Listener>>(iter: I) -> Self {
        Self { pairs: iter.into_iter().map(|l| (l.protocol, l.port)).collect() }
    }
}

/// The read-only port over an allocation's reported listener set.
///
/// Behaviour contract (D13 item 10; pinned by S-50 to S-52):
/// - An allocation id has an entry from `open` to `close` and none before or
///   after. `reported` on an id with no entry returns the empty set. While the
///   entry exists the set only grows and holds only pairs the Service declares.
/// - `reported` is a level read: the current set, never blocks.
/// - `changed` completes with the current set as soon as it is not equal to
///   `seen`; if it already differs when called it completes at once. Otherwise
///   it waits for an insertion (not a timer). A duplicate, an undeclared pair or
///   another allocation's report does not complete it. It is cancel-safe. For an
///   id with no entry it does not complete; a waiter pending when the entry is
///   closed is not woken.
#[async_trait]
pub trait GuestListenerReports: Send + Sync {
    /// The current reported set for `alloc`; empty for an id with no entry.
    fn reported(&self, alloc: &AllocationId) -> ReportedListeners;

    /// Completes with the current set as soon as it differs from `seen`.
    async fn changed(&self, alloc: &AllocationId, seen: &ReportedListeners) -> ReportedListeners;
}

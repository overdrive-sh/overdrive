//! `SimGuestListenerReports` — the simulation implementation of the core
//! `GuestListenerReports` port (D13 item 10, PR-1) with scripted insertions.
//!
//! The scripting API (`open`, `insert`, `close`) is DISTILL's to shape (the
//! design: "its scripting API is DISTILL's"). It models the port's behaviour
//! contract exactly: an id has an entry from `open` to `close`; `reported` is a
//! level read; `changed` completes on an insertion that changes the set, never
//! on a no-op (duplicate, id with no entry), never for a closed id.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU16;

use async_trait::async_trait;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::{GuestListenerReports, ReportedListeners};
use parking_lot::Mutex;
use tokio::sync::Notify;

/// A scripted reported-set board for DST and acceptance tests.
#[derive(Debug, Default)]
pub struct SimGuestListenerReports {
    entries: Mutex<BTreeMap<AllocationId, BTreeSet<(Proto, NonZeroU16)>>>,
    wake: Notify,
}

impl SimGuestListenerReports {
    /// An empty board: no allocation has an entry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates `alloc`'s entry (empty). A second `open` of a live entry is a no-op.
    pub fn open(&self, alloc: &AllocationId) {
        self.entries.lock().entry(alloc.clone()).or_default();
    }

    /// Scripts one reported listener for `alloc`. Returns whether it was an
    /// insertion (`false` for a duplicate or an id with no entry); only an
    /// insertion wakes `changed` waiters.
    pub fn insert(&self, alloc: &AllocationId, protocol: Proto, port: NonZeroU16) -> bool {
        let inserted =
            self.entries.lock().get_mut(alloc).is_some_and(|set| set.insert((protocol, port)));
        if inserted {
            self.wake.notify_waiters();
        }
        inserted
    }

    /// Removes `alloc`'s entry. A waiter pending on it is not woken.
    pub fn close(&self, alloc: &AllocationId) {
        self.entries.lock().remove(alloc);
    }
}

#[async_trait]
impl GuestListenerReports for SimGuestListenerReports {
    fn reported(&self, alloc: &AllocationId) -> ReportedListeners {
        self.entries.lock().get(alloc).map(|set| set.iter().copied().collect()).unwrap_or_default()
    }

    async fn changed(&self, alloc: &AllocationId, seen: &ReportedListeners) -> ReportedListeners {
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let entries = self.entries.lock();
                if let Some(set) = entries.get(alloc) {
                    let current: ReportedListeners = set.iter().copied().collect();
                    if &current != seen {
                        return current;
                    }
                }
            }
            notified.await;
        }
    }
}

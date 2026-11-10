//! `GuestListenerBoard` — the host-side board of the listeners a guest has
//! reported over the beacon channel (D13 item 10, PR-1).
//!
//! An allocation's declared set bounds reports for the entry's lifetime. The
//! beacon reader opens the entry before it starts, records only new declared
//! pairs, and closes it after the probe supervisor has stopped.

use std::collections::{BTreeMap, BTreeSet};
use std::future::pending;
use std::num::NonZeroU16;
use std::sync::Arc;

use async_trait::async_trait;
use overdrive_core::aggregate::Listener;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::{GuestListenerReports, ReportedListeners};
use parking_lot::Mutex;
use tokio::sync::Notify;

#[derive(Debug)]
struct ListenerEntry {
    declared: BTreeSet<(Proto, NonZeroU16)>,
    reported: BTreeSet<(Proto, NonZeroU16)>,
    changed: Arc<Notify>,
    open: bool,
}

/// Host-side board of reported guest listeners. See the module docs.
#[derive(Debug, Default)]
pub struct GuestListenerBoard {
    entries: Mutex<BTreeMap<AllocationId, Arc<Mutex<ListenerEntry>>>>,
}

impl GuestListenerBoard {
    /// An empty board with no entries.
    #[must_use]
    #[allow(
        clippy::missing_const_for_fn,
        reason = "the pinned shape is `new() -> Self`; the real board's constructor is not const"
    )]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn open(&self, alloc: &AllocationId, declared: &[Listener]) {
        let declared = declared.iter().map(|listener| (listener.protocol, listener.port)).collect();
        let entry = Arc::new(Mutex::new(ListenerEntry {
            declared,
            reported: BTreeSet::new(),
            changed: Arc::new(Notify::new()),
            open: true,
        }));
        self.entries.lock().entry(alloc.clone()).or_insert(entry);
    }

    pub(crate) fn record(&self, alloc: &AllocationId, protocol: Proto, port: NonZeroU16) {
        let Some(entry) = self.entries.lock().get(alloc).cloned() else {
            return;
        };
        let changed = {
            let mut entry = entry.lock();
            let pair = (protocol, port);
            if !entry.open || !entry.declared.contains(&pair) || !entry.reported.insert(pair) {
                return;
            }
            Arc::clone(&entry.changed)
        };
        changed.notify_waiters();
    }

    pub(crate) fn close(&self, alloc: &AllocationId) {
        let entry = self.entries.lock().remove(alloc);
        if let Some(entry) = entry {
            entry.lock().open = false;
        }
    }
}

#[async_trait]
impl GuestListenerReports for GuestListenerBoard {
    fn reported(&self, alloc: &AllocationId) -> ReportedListeners {
        let entry = { self.entries.lock().get(alloc).cloned() };
        let Some(entry) = entry else {
            return ReportedListeners::default();
        };
        entry
            .lock()
            .reported
            .iter()
            .map(|(protocol, port)| Listener { protocol: *protocol, port: *port })
            .collect()
    }

    async fn changed(&self, alloc: &AllocationId, seen: &ReportedListeners) -> ReportedListeners {
        let entry = { self.entries.lock().get(alloc).cloned() };
        let Some(entry) = entry else {
            return pending().await;
        };
        let changed = Arc::clone(&entry.lock().changed);

        loop {
            let notified = changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            let reported = entry
                .lock()
                .reported
                .iter()
                .map(|(protocol, port)| Listener { protocol: *protocol, port: *port })
                .collect::<ReportedListeners>();
            if reported != *seen {
                return reported;
            }
            notified.await;
        }
    }
}

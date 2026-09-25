//! Socket-free simulation of the shared-gateway DNS owner port (GH #295,
//! D-295-R16).
//!
//! [`SimGuestDnsFactory`] implements
//! `overdrive_control_plane::dns_responder::GuestDnsFactory` and builds
//! [`SimGuestDns`] responders that bind no socket and answer no query. The
//! doubles script the three outcomes the serve task owner and the supervisor
//! observe: a probe refusal (shared by every responder one factory built), a
//! serve exit (return, panic, or pending), and a per-responder audit refusal.
//!
//! They serve `overdrive-control-plane`'s `tests/` suites and other crates.
//! That crate's own source-local tests implement a second compiled copy of the
//! control-plane traits and use its crate-private test-local ports instead.
//!
//! Both scripting slots are standing (never consumed) and are atomics stored
//! and loaded with `Ordering::SeqCst`, as in `SimSharedGuestNetworkOwner`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use overdrive_control_plane::dns_responder::responder::{DnsResponderError, Result};
use overdrive_control_plane::dns_responder::{GuestDns, GuestDnsDeps, GuestDnsFactory};
use parking_lot::Mutex;
use tokio::sync::Notify;

/// How a [`SimGuestDns`] serve future ends when the test ends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimGuestDnsServeExit {
    /// The serve future returns.
    Return,
    /// The serve future panics.
    Panic,
}

/// In-memory [`GuestDnsFactory`] double.
///
/// Every responder it builds shares its one standing probe slot; it keeps a
/// build log in build order.
#[derive(Debug, Default)]
pub struct SimGuestDnsFactory {
    /// The standing probe-refusal slot shared by every built responder.
    probe_armed: Arc<AtomicBool>,
    /// Every responder built, in build order.
    built: Mutex<Vec<Arc<SimGuestDns>>>,
}

impl SimGuestDnsFactory {
    /// Arm (`true`) or disarm (`false`) the standing probe refusal. While
    /// armed, every responder's `probe` — including one built earlier —
    /// returns `DnsResponderError::Probe`.
    pub fn script_probe_failure(&self, armed: bool) {
        self.probe_armed.store(armed, Ordering::SeqCst);
    }

    /// Every responder this factory built, in build order; a cloned,
    /// non-draining snapshot.
    #[must_use]
    pub fn responders(&self) -> Vec<Arc<SimGuestDns>> {
        self.built.lock().clone()
    }
}

impl GuestDnsFactory for SimGuestDnsFactory {
    /// Build a fresh [`SimGuestDns`] that shares this factory's probe slot,
    /// append it to the build log, and return it. `deps` is dropped: the
    /// double binds no socket and answers no query.
    fn responder(&self, deps: GuestDnsDeps) -> Arc<dyn GuestDns> {
        drop(deps);
        let responder = Arc::new(SimGuestDns {
            probe_armed: Arc::clone(&self.probe_armed),
            audit_armed: AtomicBool::new(false),
            ending: Mutex::new(None),
            ended: Notify::new(),
        });
        self.built.lock().push(Arc::clone(&responder));
        responder
    }
}

/// In-memory [`GuestDns`] double. Built only by [`SimGuestDnsFactory`].
#[derive(Debug)]
pub struct SimGuestDns {
    /// The factory's standing probe slot.
    probe_armed: Arc<AtomicBool>,
    /// This responder's standing audit-refusal slot.
    audit_armed: AtomicBool,
    /// The serve ending, decided by the first `end_serve` or `stop`.
    ending: Mutex<Option<SimGuestDnsServeExit>>,
    /// Wakes a pending serve future when the ending is decided.
    ended: Notify,
}

impl SimGuestDns {
    /// Decide how `serve` ends, if no earlier `end_serve` or `stop` decided
    /// it. An ending decided before `serve` is first polled applies at that
    /// poll.
    pub fn end_serve(&self, exit: SimGuestDnsServeExit) {
        self.decide(exit);
    }

    /// Arm (`true`) or disarm (`false`) this responder's standing audit
    /// refusal. The slot is per responder, so a replacement built after the
    /// fault audits clean.
    pub fn script_audit_failure(&self, armed: bool) {
        self.audit_armed.store(armed, Ordering::SeqCst);
    }

    fn decide(&self, exit: SimGuestDnsServeExit) {
        let mut ending = self.ending.lock();
        if ending.is_none() {
            *ending = Some(exit);
        }
        drop(ending);
        self.ended.notify_waiters();
    }
}

#[async_trait]
impl GuestDns for SimGuestDns {
    /// `Err(Probe)` while the factory's probe slot is armed when called;
    /// `Ok(())` otherwise.
    async fn probe(&self) -> Result<()> {
        if self.probe_armed.load(Ordering::SeqCst) {
            return Err(DnsResponderError::Probe {
                reason: "scripted sim DNS probe refusal".into(),
            });
        }
        Ok(())
    }

    /// Stays pending until `end_serve` or `stop`; `end_serve(Return)` and
    /// `stop` make it return, `end_serve(Panic)` makes it panic.
    #[allow(clippy::panic, reason = "a scripted serve panic is this double's contract")]
    async fn serve(self: Arc<Self>) {
        loop {
            // Created before the ending is read, so a decision made between the
            // read and the await still wakes this future.
            let notified = self.ended.notified();
            let ending = *self.ending.lock();
            match ending {
                Some(SimGuestDnsServeExit::Return) => return,
                Some(SimGuestDnsServeExit::Panic) => panic!("scripted sim DNS serve panic"),
                None => notified.await,
            }
        }
    }

    /// `Err(Socket)` while this responder's audit slot is armed; `Ok(())`
    /// otherwise.
    async fn audit(&self) -> Result<()> {
        if self.audit_armed.load(Ordering::SeqCst) {
            return Err(DnsResponderError::Socket {
                source: std::io::Error::other("scripted sim DNS audit refusal"),
            });
        }
        Ok(())
    }

    /// Makes `serve` return, unless an earlier call already decided its ending.
    fn stop(&self) {
        self.decide(SimGuestDnsServeExit::Return);
    }
}

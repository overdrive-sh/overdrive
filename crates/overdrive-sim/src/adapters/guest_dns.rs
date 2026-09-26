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

#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    clippy::expect_used,
    reason = "test contracts carry exact CONTRACT_SHAPE markers and diagnostic assertions"
)]
mod tests {
    use std::net::Ipv4Addr;
    use std::panic::AssertUnwindSafe;
    use std::task::Poll;
    use std::time::Duration;

    use futures::FutureExt;
    use overdrive_control_plane::dns_responder::frontend_addr_allocator::FrontendAddrAllocator;
    use overdrive_core::id::NodeId;

    use super::*;
    use crate::adapters::clock::SimClock;
    use crate::adapters::observation_store::SimObservationStore;

    /// Every bounded wait: a RED body fails instead of hanging.
    const WAIT: Duration = Duration::from_secs(2);

    /// The dependencies a responder is built from. The doubles drop them.
    fn deps() -> GuestDnsDeps {
        GuestDnsDeps {
            store: Arc::new(SimObservationStore::single_peer(
                NodeId::new("node-guest-dns").expect("valid node id"),
                0,
            )),
            clock: Arc::new(SimClock::new()),
            gateway: Ipv4Addr::new(10, 99, 0, 1),
            frontend: FrontendAddrAllocator::new(),
        }
    }

    /// `result` is the scripted probe refusal.
    fn assert_probe_refused(result: &Result<()>, context: &str) {
        assert!(
            matches!(
                result,
                Err(DnsResponderError::Probe { reason }) if reason == "scripted sim DNS probe refusal"
            ),
            "{context}: the armed probe slot refuses, got {result:?}",
        );
    }

    /// `result` is the scripted audit refusal.
    fn assert_audit_refused(result: &Result<()>, context: &str) {
        assert!(
            matches!(
                result,
                Err(DnsResponderError::Socket { source })
                    if source.to_string() == "scripted sim DNS audit refusal"
            ),
            "{context}: the armed audit slot refuses, got {result:?}",
        );
    }

    /// The factory's logged responder at `index` is the port object `built`.
    fn assert_logged(factory: &SimGuestDnsFactory, index: usize, built: &Arc<dyn GuestDns>) {
        let logged = factory.responders();
        assert!(
            std::ptr::addr_eq(Arc::as_ptr(&logged[index]), Arc::as_ptr(built)),
            "build-log entry {index} is the responder the factory returned",
        );
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-34 — Shared-gateway DNS stays truthful, and its loss closes new commands
    /// until a fresh responder is serving.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// One standing probe slot per factory, shared by every responder it
    /// built — before or after arming — and by no other factory's; never
    /// consumed; the build log lists responders in build order and does not
    /// drain (FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (the `SimGuestDns` rules: `responder`, `probe`, and standing slots)).
    #[tokio::test]
    async fn the_probe_slot_is_shared_by_every_responder_and_is_standing() {
        let factory = SimGuestDnsFactory::default();
        let other_factory = SimGuestDnsFactory::default();
        let early = factory.responder(deps());
        let unrelated = other_factory.responder(deps());
        early.probe().await.expect("a fresh factory's probe slot is disarmed");

        factory.script_probe_failure(true);
        let late = factory.responder(deps());
        for call in 0..2 {
            assert_probe_refused(
                &early.probe().await,
                &format!("built before arming, call {call}"),
            );
            assert_probe_refused(&late.probe().await, &format!("built after arming, call {call}"));
        }
        unrelated.probe().await.expect("another factory's responder has its own probe slot");

        factory.script_probe_failure(false);
        early.probe().await.expect("disarming reaches a responder built before arming");
        late.probe().await.expect("disarming reaches a responder built after arming");
        factory.script_probe_failure(true);
        assert_probe_refused(&early.probe().await, "re-armed");

        assert_eq!(factory.responders().len(), 2, "one build-log entry per responder built");
        assert_logged(&factory, 0, &early);
        assert_logged(&factory, 1, &late);
        assert_eq!(factory.responders().len(), 2, "the build log is a non-draining snapshot");
        assert_eq!(other_factory.responders().len(), 1);
        assert_logged(&other_factory, 0, &unrelated);
    }

    /// One way to end a serve future.
    #[derive(Debug, Clone, Copy)]
    enum Decision {
        EndReturn,
        EndPanic,
        Stop,
    }

    /// How a serve future ended.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Ending {
        Returned,
        Panicked,
    }

    /// Apply `decision` to one responder: `end_serve` on the double, `stop`
    /// through the port.
    fn decide(sim: &SimGuestDns, port: &Arc<dyn GuestDns>, decision: Decision) {
        match decision {
            Decision::EndReturn => sim.end_serve(SimGuestDnsServeExit::Return),
            Decision::EndPanic => sim.end_serve(SimGuestDnsServeExit::Panic),
            Decision::Stop => port.stop(),
        }
    }

    /// The ending a completed serve future reports; a panic must carry the
    /// double's scripted message.
    fn ending_of(outcome: std::thread::Result<()>) -> Ending {
        match outcome {
            Ok(()) => Ending::Returned,
            Err(payload) => {
                assert_eq!(
                    payload.downcast_ref::<&str>().copied(),
                    Some("scripted sim DNS serve panic"),
                    "a scripted serve panic carries the double's message",
                );
                Ending::Panicked
            }
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-34 — Shared-gateway DNS stays truthful, and its loss closes new commands
    /// until a fresh responder is serving.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// `serve` stays pending until `end_serve` or `stop`; `end_serve(Return)`
    /// and `stop` make it return, `end_serve(Panic)` makes it panic; the first
    /// call decides and later calls change nothing; an ending decided before
    /// the first poll applies at that poll (FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (the `SimGuestDns` rules: how `serve` ends)).
    #[tokio::test]
    async fn serve_ends_on_the_first_end_serve_or_stop_even_before_its_first_poll() {
        use Decision::{EndPanic, EndReturn, Stop};
        use Ending::{Panicked, Returned};
        // (decided before the first poll, decided while pending, ending)
        let rows: [(&[Decision], &[Decision], Option<Ending>); 12] = [
            (&[EndReturn], &[], Some(Returned)),
            (&[EndPanic], &[], Some(Panicked)),
            (&[Stop], &[], Some(Returned)),
            (&[EndReturn, EndPanic], &[], Some(Returned)),
            (&[EndPanic, Stop], &[], Some(Panicked)),
            (&[Stop, EndPanic], &[], Some(Returned)),
            (&[], &[EndReturn], Some(Returned)),
            (&[], &[EndPanic], Some(Panicked)),
            (&[], &[Stop], Some(Returned)),
            (&[], &[Stop, EndPanic], Some(Returned)),
            (&[], &[EndPanic, EndReturn], Some(Panicked)),
            (&[], &[], None),
        ];

        for (before, pending, expected) in rows {
            let row = format!("before first poll {before:?}, while pending {pending:?}");
            let factory = SimGuestDnsFactory::default();
            let port = factory.responder(deps());
            let sim = factory.responders().pop().expect("the factory logs what it builds");
            for decision in before {
                decide(&sim, &port, *decision);
            }

            let mut serve = AssertUnwindSafe(Arc::clone(&port).serve()).catch_unwind();
            let first = std::future::poll_fn(|cx| Poll::Ready(serve.poll_unpin(cx))).await;
            let observed = if before.is_empty() {
                assert!(first.is_pending(), "{row}: an undecided serve stays pending");
                tokio::task::yield_now().await;
                assert!(
                    std::future::poll_fn(|cx| Poll::Ready(serve.poll_unpin(cx))).await.is_pending(),
                    "{row}: it stays pending until an ending is decided",
                );
                for decision in pending {
                    decide(&sim, &port, *decision);
                }
                if pending.is_empty() {
                    None
                } else {
                    let outcome = tokio::time::timeout(WAIT, &mut serve)
                        .await
                        .unwrap_or_else(|_| panic!("{row}: the decision ends the pending serve"));
                    Some(ending_of(outcome))
                }
            } else {
                match first {
                    Poll::Ready(outcome) => Some(ending_of(outcome)),
                    Poll::Pending => {
                        panic!("{row}: an ending decided before the first poll applies at it")
                    }
                }
            };
            assert_eq!(observed, expected, "{row}");
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-34 — Shared-gateway DNS stays truthful, and its loss closes new commands
    /// until a fresh responder is serving.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// The audit slot belongs to one responder: while armed it refuses every
    /// audit of that responder with the scripted `Socket` error, a sibling and
    /// a replacement built after the fault audit clean, and the audit and
    /// probe slots do not reach each other (FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (the `SimGuestDns` rules: the per-responder audit slot and standing slots)).
    #[tokio::test]
    async fn the_audit_slot_is_per_responder_so_a_replacement_audits_clean() {
        let factory = SimGuestDnsFactory::default();
        let sibling = factory.responder(deps());
        let faulted = factory.responder(deps());
        let faulted_sim = Arc::clone(&factory.responders()[1]);
        faulted.audit().await.expect("a fresh responder audits clean");

        faulted_sim.script_audit_failure(true);
        for call in 0..2 {
            assert_audit_refused(&faulted.audit().await, &format!("call {call}"));
        }
        sibling.audit().await.expect("a responder built before the fault audits clean");
        let replacement = factory.responder(deps());
        replacement.audit().await.expect("a replacement built after the fault audits clean");
        faulted.probe().await.expect("an armed audit slot does not reach the probe");

        factory.script_probe_failure(true);
        replacement.audit().await.expect("an armed probe slot does not reach the audit");
        assert_audit_refused(&faulted.audit().await, "probe armed as well");
        factory.script_probe_failure(false);

        faulted_sim.script_audit_failure(false);
        faulted.audit().await.expect("disarming the audit slot restores a clean audit");
        assert_logged(&factory, 1, &faulted);
    }
}

//! Node-shared mTLS interception and allocation capability lifecycle.
//!
//! The worker owns the node-shared mTLS intercept listeners and the
//! per-allocation capability registrations that associate diverted
//! connections with their allocation. `start_shared_owner` binds one leg-F
//! and one leg-C listener and starts one cancelable accept task for each.
//! `start_alloc` installs the allocation's source and destination elements
//! under the converged shared program. `stop_alloc` retires that allocation's
//! registration and drains its enforced connections; `shutdown_owner` stops
//! both shared accept tasks and retires every remaining registration.
//!
//! Listener binding is supplied by [`MtlsIntercept`]. Production uses the
//! host's transparent sockets; simulation uses socket-free listeners. Every
//! accept path awaits [`MtlsResolve::resolve`] before deciding whether to
//! enforce, pass through, or fail closed.

#![allow(
    clippy::result_large_err,
    reason = "GH #295 exact shared-owner/install errors retain nested source-honest intercept outcomes"
)]
use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::{NonZeroU16, NonZeroU64};
#[cfg(any(test, feature = "integration-tests"))]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::AllocationSpec;
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnection, EnforcedConnectionId, InterceptedConnection, MtlsEnforcement,
    MtlsEnforcementError, Routed,
};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_core::{AllocationId, SpiffeId};
use parking_lot::{Mutex, RwLock};
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::mtls_intercept::{InterceptError, InterceptLeg, InterceptPostcondition};
use crate::mtls_intercept_port::{
    InterceptAcceptError, InterceptGuard, InterceptListener, MtlsIntercept,
};

/// A fail-closed allocation capability-registration failure.
///
/// [`MtlsInterceptWorker::start_alloc`] returns this type when it cannot
/// reserve and publish an allocation's source/destination elements under the
/// node's converged shared intercept program. The action shim surfaces the
/// error to the allocation lifecycle rather than continuing without the
/// intercept. The `OutboundTproxyInstall` and `Inbound` cases preserve the
/// typed [`InterceptError`] from the shared element port. Node listener bind
/// and address-audit failures belong to [`MtlsSharedOwnerError`], because the
/// listeners are started once for the node by `start_shared_owner`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MtlsInterceptInstallError {
    /// The process-local registration generation cannot advance.
    #[error("mTLS registration generation exhausted at {next}")]
    GenerationExhausted { next: u64 },
    /// Another Pending, Active, or Retiring capability owns this address.
    #[error("mTLS registration address is already reserved: {address}")]
    RegistrationConflict { address: Ipv4Addr },
    /// Retirement won after effects were acquired but before activation.
    #[error("mTLS registration for allocation {alloc_id} retired before activation")]
    RegistrationRetired { alloc_id: AllocationId },
    /// Allocation registration requires the healthy node-shared listener owner.
    #[error("shared mTLS owner unavailable")]
    SharedOwner {
        #[source]
        source: MtlsSharedOwnerError,
    },
    /// The process owner has entered its terminal shutdown fence. A late
    /// allocation registration is rejected before it can create work behind
    /// the owner's completion boundary.
    #[error("mTLS intercept owner is shutting down")]
    OwnerShutdown,

    /// A same-allocation replacement could not retire the complete prior
    /// listener/rule/connection owner. No replacement install is attempted,
    /// so the caller keeps EXEC closed and may retry the retained stop owner.
    #[error("mTLS prior intercept teardown failed: {source}")]
    PriorTeardown {
        #[source]
        source: MtlsInterceptStopError,
    },

    /// Shared outbound source-element installation failed. The source
    /// [`InterceptError`] identifies the failed element operation.
    ///
    /// `#[source]` (not `#[from]`): the sibling `Inbound` variant already
    /// owns the single `#[from] InterceptError` auto-conversion, so the
    /// outbound site names this constructor explicitly to keep the two
    /// `InterceptError` sources distinct in `Display`.
    #[error("mTLS outbound TPROXY install failed: {0}")]
    OutboundTproxyInstall(#[source] InterceptError),

    /// A leg-F transparent-listener bind error. Current node-shared listener
    /// startup reports this failure through [`MtlsSharedOwnerError::ListenerBind`].
    /// `start_alloc` does not bind a listener.
    #[error("mTLS leg-F listener bind failed: {0}")]
    LegFBind(#[source] InterceptError),

    /// Shared inbound destination-element installation failed. The source
    /// [`InterceptError`] identifies the failed element operation. A node
    /// listener bind failure is reported by [`MtlsSharedOwnerError`].
    #[error("mTLS inbound intercept install failed: {0}")]
    Inbound(#[from] InterceptError),

    /// A leg-F bound-address read failure. Current node-shared listener startup
    /// reports this through [`MtlsSharedOwnerError::ListenerLocalAddr`].
    #[error("mTLS leg-F listener address capture failed: {source}")]
    LegFLocalAddr {
        #[source]
        source: std::io::Error,
    },

    /// A leg-C bound-address read failure. Current node-shared listener startup
    /// reports this through [`MtlsSharedOwnerError::ListenerLocalAddr`].
    #[error("mTLS leg-C listener address capture failed: {source}")]
    LegCLocalAddr {
        #[source]
        source: std::io::Error,
    },
}

/// One allocation's retirement attempt did not converge (D-295-R10).
///
/// `Clone` so every caller joined on one attempt, and every later caller of a
/// finished owner shutdown, receives the stored result (DISTILL gap B-6). Each
/// typed source is shared through an `Arc`, so clones carry pointer-equal
/// sources and a caller reaches the cause through the field (`&*source`).
#[derive(Debug, Clone, thiserror::Error)]
pub enum MtlsInterceptStopError {
    /// In this attempt `MtlsEnforcement::teardown` returned `Err` for at least
    /// one published handle. `failures` names each such handle once, in the
    /// order the attempt tore them down, with its exact error. Element removal
    /// was not attempted; the failed handles, the drain with its element
    /// guards, and the Retiring record are retained.
    #[error(
        "allocation {alloc_id}: enforced-handle teardown failed for {count} handle(s): {details}",
        count = .failures.len(),
        details = format_handle_teardown_failures(.failures),
    )]
    HandleTeardown {
        /// Allocation whose retirement remains incomplete.
        alloc_id: AllocationId,
        /// Every handle whose teardown failed in this attempt, in teardown order.
        failures: Vec<HandleTeardownFailure>,
    },
    /// Every handle teardown in this attempt succeeded and
    /// `remove_allocation_elements` returned `Err`; `source` is that error. The
    /// drain, its element guards, and the Retiring record are retained.
    #[error("allocation {alloc_id}: shared intercept element removal failed: {source}")]
    ElementRemoval {
        /// Allocation whose retirement remains incomplete.
        alloc_id: AllocationId,
        /// The element-removal failure, shared by every clone.
        #[source]
        source: Arc<InterceptError>,
    },
}

fn format_handle_teardown_failures(failures: &[HandleTeardownFailure]) -> String {
    failures
        .iter()
        .map(|failure| format!("{}: {}", failure.connection, failure.source))
        .collect::<Vec<_>>()
        .join("; ")
}

/// One enforced-connection handle whose teardown failed in a retirement
/// attempt.
#[derive(Debug, Clone)]
pub struct HandleTeardownFailure {
    /// The connection whose authoritative teardown failed.
    pub connection: EnforcedConnectionId,
    /// The exact teardown error, shared by every clone.
    pub source: Arc<MtlsEnforcementError>,
}

/// A full worker-owner shutdown attempt that did not converge.
///
/// Every concurrent or later caller observes this same stored result. Owner
/// shutdown is one-shot: it seals new work and does not create retry generations.
#[derive(Clone, Debug, thiserror::Error)]
#[error("mTLS worker-owner shutdown failed: {failures:?}")]
pub struct MtlsInterceptOwnerShutdownError {
    /// Allocation-scoped teardown errors retained for exact retry.
    pub failures: Vec<MtlsInterceptStopError>,
}

/// Typed lifecycle failure for the one node-shared F/C listener owner.
#[derive(Debug, thiserror::Error)]
pub enum MtlsSharedOwnerError {
    #[error("shared mTLS owner has not started")]
    NotStarted,
    #[error("shared mTLS owner is shutting down")]
    OwnerShutdown,
    #[error("shared mTLS listener {leg:?} bind failed at {requested}")]
    ListenerBind {
        leg: crate::mtls_intercept::InterceptLeg,
        requested: SocketAddrV4,
        #[source]
        source: InterceptError,
    },
    #[error("shared mTLS listener {leg:?} address observation failed")]
    ListenerLocalAddr {
        leg: crate::mtls_intercept::InterceptLeg,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "shared mTLS listener {leg:?} postcondition mismatch: expected {expected}, observed {observed:?}"
    )]
    ListenerPostcondition {
        leg: crate::mtls_intercept::InterceptLeg,
        expected: SocketAddrV4,
        observed: Option<SocketAddrV4>,
    },
    #[error("shared mTLS rule/set convergence failed")]
    Intercept {
        #[source]
        source: InterceptError,
    },
    #[error("shared mTLS listener task {leg:?} returned")]
    TaskReturned { leg: crate::mtls_intercept::InterceptLeg },
    #[error("shared mTLS listener task {leg:?} failed")]
    TaskFailed {
        leg: crate::mtls_intercept::InterceptLeg,
        #[source]
        source: std::io::Error,
    },
    #[error("shared mTLS listener task {leg:?} panicked")]
    TaskPanicked { leg: crate::mtls_intercept::InterceptLeg },
    #[error("shared mTLS listener task {leg:?} was cancelled")]
    TaskCancelled { leg: crate::mtls_intercept::InterceptLeg },
    #[error("shared mTLS listener task observation channel closed")]
    TaskObserverClosed,
    #[error("shared mTLS dynamic members could not be cleared at boot")]
    BootMemberClear {
        #[source]
        source: InterceptError,
    },
    #[error(
        "shared mTLS dynamic members differ from the registry: expected {expected:?}, observed {observed:?}"
    )]
    MemberMismatch {
        expected: crate::mtls_intercept_port::InterceptMembers,
        observed: crate::mtls_intercept_port::InterceptMembers,
    },
    #[error("shared mTLS dynamic member repair failed")]
    MemberRepair {
        #[source]
        source: InterceptError,
    },
}

impl MtlsSharedOwnerError {
    /// The one shared-network component this error reports; the SSOT the
    /// supervisor consumes.
    ///
    /// | Variant | Component |
    /// |---|---|
    /// | `ListenerBind`, `ListenerLocalAddr`, `ListenerPostcondition`, `TaskReturned`, `TaskFailed`, `TaskPanicked`, `TaskCancelled` | `LegF` or `LegC`, by `leg` |
    /// | `Intercept` | `IpRules` |
    /// | `MemberMismatch`, `MemberRepair`, `BootMemberClear` | `IpSets` |
    /// | `NotStarted`, `OwnerShutdown`, `TaskObserverClosed` | `Supervisor` |
    #[allow(
        clippy::missing_const_for_fn,
        reason = "the accepted API pins component() as a non-const public method"
    )]
    #[must_use]
    pub fn component(&self) -> overdrive_core::guest_network::SharedGuestNetworkComponent {
        use overdrive_core::guest_network::SharedGuestNetworkComponent as Component;

        match self {
            Self::ListenerBind { leg, .. }
            | Self::ListenerLocalAddr { leg, .. }
            | Self::ListenerPostcondition { leg, .. }
            | Self::TaskReturned { leg }
            | Self::TaskFailed { leg, .. }
            | Self::TaskPanicked { leg }
            | Self::TaskCancelled { leg } => match leg {
                crate::mtls_intercept::InterceptLeg::F => Component::LegF,
                crate::mtls_intercept::InterceptLeg::C => Component::LegC,
            },
            Self::Intercept { .. } => Component::IpRules,
            Self::MemberMismatch { .. }
            | Self::MemberRepair { .. }
            | Self::BootMemberClear { .. } => Component::IpSets,
            Self::NotStarted | Self::OwnerShutdown | Self::TaskObserverClosed => {
                Component::Supervisor
            }
        }
    }
}

type SharedListenerTaskResult = std::io::Result<()>;

#[allow(dead_code, reason = "D11 RED scaffold precedes retained task ownership")]
struct SharedListenerTaskEvent {
    leg: crate::mtls_intercept::InterceptLeg,
    joined: std::result::Result<SharedListenerTaskResult, tokio::task::JoinError>,
}

#[allow(dead_code, reason = "D11 RED scaffold precedes abort-on-drop implementation")]
struct AbortOnDropListenerTask {
    task: Option<tokio::task::JoinHandle<SharedListenerTaskResult>>,
}

impl AbortOnDropListenerTask {
    async fn join(
        &mut self,
    ) -> Option<std::result::Result<SharedListenerTaskResult, tokio::task::JoinError>> {
        let task = self.task.as_mut()?;
        let joined = task.await;
        self.task.take();
        Some(joined)
    }
}

impl Drop for AbortOnDropListenerTask {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[allow(dead_code, reason = "D11 RED scaffold precedes retained task ownership")]
struct SharedListenerTaskSlot {
    task_abort: tokio::task::AbortHandle,
    observer: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
#[allow(dead_code, reason = "D11 RED scaffold precedes retained task ownership")]
struct SharedListenerTaskSlots {
    leg_f: Option<SharedListenerTaskSlot>,
    leg_c: Option<SharedListenerTaskSlot>,
}

#[allow(dead_code, reason = "D11 RED scaffold precedes retained task ownership")]
struct SharedListenerTaskOwner {
    slots: Mutex<SharedListenerTaskSlots>,
    event_tx: mpsc::WeakSender<SharedListenerTaskEvent>,
    event_rx: tokio::sync::Mutex<mpsc::Receiver<SharedListenerTaskEvent>>,
}

#[allow(
    dead_code,
    clippy::significant_drop_tightening,
    clippy::unused_self,
    reason = "D11 exact private task-owner surface intentionally holds mutex guards through each linearized slot operation"
)]
impl SharedListenerTaskOwner {
    fn new(
        leg_f: tokio::task::JoinHandle<SharedListenerTaskResult>,
        leg_c: tokio::task::JoinHandle<SharedListenerTaskResult>,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::channel(4);
        let leg_f = Self::observe(InterceptLeg::F, leg_f, &event_tx);
        let leg_c = Self::observe(InterceptLeg::C, leg_c, &event_tx);
        Self {
            slots: Mutex::new(SharedListenerTaskSlots { leg_f: Some(leg_f), leg_c: Some(leg_c) }),
            event_tx: event_tx.downgrade(),
            event_rx: tokio::sync::Mutex::new(event_rx),
        }
    }

    async fn wait_failure(&self) -> MtlsSharedOwnerError {
        let mut events = self.event_rx.lock().await;
        match events.try_recv() {
            Ok(event) => {
                self.consume_terminal(event.leg);
                return classify_shared_listener_task_exit(event.leg, event.joined);
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                return MtlsSharedOwnerError::TaskObserverClosed;
            }
            Err(mpsc::error::TryRecvError::Empty) => {}
        }
        match events.recv().await {
            Some(event) => {
                self.consume_terminal(event.leg);
                classify_shared_listener_task_exit(event.leg, event.joined)
            }
            None => MtlsSharedOwnerError::TaskObserverClosed,
        }
    }

    fn consume_terminal(&self, leg: InterceptLeg) {
        let mut slots = self.slots.lock();
        match leg {
            InterceptLeg::F => slots.leg_f.take(),
            InterceptLeg::C => slots.leg_c.take(),
        };
    }

    fn replace_terminal(
        &self,
        leg: InterceptLeg,
        task: impl FnOnce() -> tokio::task::JoinHandle<SharedListenerTaskResult>,
    ) -> std::result::Result<(), MtlsSharedOwnerError> {
        let mut slots = self.slots.lock();
        let slot = match leg {
            InterceptLeg::F => &mut slots.leg_f,
            InterceptLeg::C => &mut slots.leg_c,
        };
        if let Some(occupied) = slot.as_ref() {
            return Err(if occupied.observer.is_finished() {
                MtlsSharedOwnerError::TaskReturned { leg }
            } else {
                MtlsSharedOwnerError::TaskObserverClosed
            });
        }
        let Some(event_tx) = self.event_tx.upgrade() else {
            return Err(MtlsSharedOwnerError::TaskObserverClosed);
        };
        *slot = Some(Self::observe(leg, task(), &event_tx));
        Ok(())
    }

    async fn shutdown(self) {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        let slots = std::mem::take(&mut *self.slots.lock());
        let mut observers = Vec::new();
        for slot in [slots.leg_f, slots.leg_c].into_iter().flatten() {
            slot.task_abort.abort();
            observers.push(slot.observer);
        }
        for observer in observers {
            let _ = observer.await;
        }
    }

    async fn shutdown_shared(&self) {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        let slots = std::mem::take(&mut *self.slots.lock());
        let mut observers = Vec::new();
        for slot in [slots.leg_f, slots.leg_c].into_iter().flatten() {
            slot.task_abort.abort();
            observers.push(slot.observer);
        }
        for observer in observers {
            let _ = observer.await;
        }
    }

    fn observe(
        leg: crate::mtls_intercept::InterceptLeg,
        task: tokio::task::JoinHandle<SharedListenerTaskResult>,
        event_tx: &mpsc::Sender<SharedListenerTaskEvent>,
    ) -> SharedListenerTaskSlot {
        let task_abort = task.abort_handle();
        let observer_tx = event_tx.clone();
        let mut owned_task = AbortOnDropListenerTask { task: Some(task) };
        let observer = tokio::spawn(async move {
            let Some(joined) = owned_task.join().await else {
                return;
            };
            let _ = observer_tx.send(SharedListenerTaskEvent { leg, joined }).await;
        });
        SharedListenerTaskSlot { task_abort, observer }
    }

    fn is_live(&self, leg: InterceptLeg) -> bool {
        let slots = self.slots.lock();
        match leg {
            InterceptLeg::F => slots.leg_f.as_ref(),
            InterceptLeg::C => slots.leg_c.as_ref(),
        }
        .is_some_and(|slot| !slot.observer.is_finished())
    }

    fn dead_leg(&self) -> Option<InterceptLeg> {
        if !self.is_live(InterceptLeg::F) {
            return Some(InterceptLeg::F);
        }
        if !self.is_live(InterceptLeg::C) {
            return Some(InterceptLeg::C);
        }
        None
    }
}

async fn shutdown_shared_listener_tasks(tasks: Arc<SharedListenerTaskOwner>) {
    match Arc::try_unwrap(tasks) {
        Ok(owner) => owner.shutdown().await,
        Err(owner) => owner.shutdown_shared().await,
    }
}

fn classify_shared_listener_task_exit(
    leg: crate::mtls_intercept::InterceptLeg,
    joined: std::result::Result<SharedListenerTaskResult, tokio::task::JoinError>,
) -> MtlsSharedOwnerError {
    match joined {
        Ok(Ok(())) => MtlsSharedOwnerError::TaskReturned { leg },
        Ok(Err(source)) => MtlsSharedOwnerError::TaskFailed { leg, source },
        Err(source) if source.is_panic() => MtlsSharedOwnerError::TaskPanicked { leg },
        Err(source) if source.is_cancelled() => MtlsSharedOwnerError::TaskCancelled { leg },
        Err(_) => MtlsSharedOwnerError::TaskCancelled { leg },
    }
}

#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic,
    reason = "D11 source-local acceptance uses actual Tokio task outcomes and exact diagnostics"
)]
mod shared_listener_task_owner_acceptance {
    use super::*;
    use crate::mtls_intercept::InterceptLeg;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct TaskDropWitness(Arc<AtomicUsize>);

    impl Drop for TaskDropWitness {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    trait EventSenderProbe {
        fn strong_probe(&self) -> mpsc::Sender<SharedListenerTaskEvent>;
    }

    impl EventSenderProbe for mpsc::Sender<SharedListenerTaskEvent> {
        fn strong_probe(&self) -> mpsc::Sender<SharedListenerTaskEvent> {
            self.clone()
        }
    }

    impl EventSenderProbe for mpsc::WeakSender<SharedListenerTaskEvent> {
        fn strong_probe(&self) -> mpsc::Sender<SharedListenerTaskEvent> {
            self.upgrade().expect("live observers retain the event channel before shutdown")
        }
    }

    async fn consume_task_owner(owner: SharedListenerTaskOwner) -> bool {
        let sender = owner.event_tx.strong_probe();
        owner.shutdown().await;
        sender.is_closed()
    }

    async fn consume_disconnected_task_owner(owner: SharedListenerTaskOwner) {
        owner.shutdown().await;
    }

    fn pending_listener_task(
        drops: Arc<AtomicUsize>,
    ) -> tokio::task::JoinHandle<SharedListenerTaskResult> {
        tokio::spawn(async move {
            let _witness = TaskDropWitness(drops);
            std::future::pending::<()>().await;
            Ok(())
        })
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn actual_tokio_return_error_panic_and_cancel_map_to_the_exact_public_error() {
        let returned = tokio::spawn(async { Ok(()) }).await;
        assert!(matches!(
            classify_shared_listener_task_exit(InterceptLeg::F, returned),
            MtlsSharedOwnerError::TaskReturned { leg: InterceptLeg::F }
        ));

        let failed =
            tokio::spawn(async { Err(std::io::Error::from_raw_os_error(libc::ECONNABORTED)) })
                .await;
        assert!(matches!(
            classify_shared_listener_task_exit(InterceptLeg::C, failed),
            MtlsSharedOwnerError::TaskFailed {
                leg: InterceptLeg::C,
                source,
            } if source.raw_os_error() == Some(libc::ECONNABORTED)
        ));

        let panicked = tokio::spawn(async {
            panic!("actual shared-listener panic");
            #[allow(unreachable_code)]
            Ok(())
        })
        .await;
        assert!(matches!(
            classify_shared_listener_task_exit(InterceptLeg::F, panicked),
            MtlsSharedOwnerError::TaskPanicked { leg: InterceptLeg::F }
        ));

        let cancelled = tokio::spawn(std::future::pending::<SharedListenerTaskResult>());
        cancelled.abort();
        assert!(matches!(
            classify_shared_listener_task_exit(InterceptLeg::C, cancelled.await),
            MtlsSharedOwnerError::TaskCancelled { leg: InterceptLeg::C }
        ));
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn observer_close_replacement_and_intentional_shutdown_use_real_owned_tasks() {
        let owner = SharedListenerTaskOwner::new(
            tokio::spawn(std::future::pending::<SharedListenerTaskResult>()),
            tokio::spawn(std::future::pending::<SharedListenerTaskResult>()),
        );
        {
            let slots = owner.slots.lock();
            slots.leg_f.as_ref().expect("leg F slot").observer.abort();
            slots.leg_c.as_ref().expect("leg C slot").observer.abort();
        }
        assert!(matches!(owner.wait_failure().await, MtlsSharedOwnerError::TaskObserverClosed));

        let owner = SharedListenerTaskOwner::new(
            tokio::spawn(async { Ok(()) }),
            tokio::spawn(std::future::pending::<SharedListenerTaskResult>()),
        );
        assert!(matches!(
            owner.wait_failure().await,
            MtlsSharedOwnerError::TaskReturned { leg: InterceptLeg::F }
        ));
        owner
            .replace_terminal(InterceptLeg::F, || tokio::spawn(async { Ok(()) }))
            .expect("only the removed terminal leg can be replaced");
        assert!(matches!(
            owner.wait_failure().await,
            MtlsSharedOwnerError::TaskReturned { leg: InterceptLeg::F }
        ));
        owner.shutdown().await;
    }

    /// Outcome anchor: DISCUSS Elevator Pitch
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn dropping_every_observer_aborts_its_listener_and_closes_the_real_event_channel() {
        let drops = Arc::new(AtomicUsize::new(0));
        let owner = SharedListenerTaskOwner::new(
            pending_listener_task(Arc::clone(&drops)),
            pending_listener_task(Arc::clone(&drops)),
        );
        {
            let slots = owner.slots.lock();
            slots.leg_f.as_ref().expect("leg F slot").observer.abort();
            slots.leg_c.as_ref().expect("leg C slot").observer.abort();
        }

        let children_aborted = tokio::time::timeout(Duration::from_secs(2), async {
            while drops.load(Ordering::SeqCst) != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();
        let channel_closed = {
            let mut events = owner.event_rx.lock().await;
            matches!(events.try_recv(), Err(mpsc::error::TryRecvError::Disconnected))
        };

        consume_disconnected_task_owner(owner).await;
        assert!(children_aborted, "each observer owns an abort-on-drop listener task");
        assert!(
            channel_closed,
            "the owner retains only a WeakSender, so loss of both observers closes the channel"
        );
    }

    /// Outcome anchor: DISCUSS Elevator Pitch
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn one_real_join_event_removes_only_its_terminal_slot_before_replacement_and_consumed_shutdown()
     {
        let owner = SharedListenerTaskOwner::new(
            tokio::spawn(async { Ok(()) }),
            tokio::spawn(std::future::pending::<SharedListenerTaskResult>()),
        );
        assert!(matches!(
            owner.wait_failure().await,
            MtlsSharedOwnerError::TaskReturned { leg: InterceptLeg::F }
        ));
        let (leg_f_removed, leg_c_retained) = {
            let slots = owner.slots.lock();
            (slots.leg_f.is_none(), slots.leg_c.is_some())
        };
        owner
            .replace_terminal(InterceptLeg::F, || {
                tokio::spawn(std::future::pending::<SharedListenerTaskResult>())
            })
            .expect("only the consumed terminal slot is replaceable");
        let receiver_consumed = consume_task_owner(owner).await;

        assert!(leg_f_removed, "wait_failure consumes the exact terminal leg-F slot");
        assert!(leg_c_retained, "the independent live leg-C slot remains owned");
        assert!(receiver_consumed, "shutdown(self) consumes the event receiver before return");
    }

    /// Outcome anchor: DISCUSS Elevator Pitch
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn replacement_refuses_a_still_live_occupied_slot_without_detaching_either_listener() {
        let drops = Arc::new(AtomicUsize::new(0));
        let owner = SharedListenerTaskOwner::new(
            pending_listener_task(Arc::clone(&drops)),
            pending_listener_task(Arc::clone(&drops)),
        );
        let spawned = std::sync::atomic::AtomicBool::new(false);
        assert!(matches!(
            owner.replace_terminal(InterceptLeg::F, || {
                spawned.store(true, Ordering::SeqCst);
                tokio::spawn(async { Ok(()) })
            }),
            Err(MtlsSharedOwnerError::TaskObserverClosed)
        ));
        assert!(owner.is_live(InterceptLeg::F));
        assert!(owner.is_live(InterceptLeg::C));
        assert!(!spawned.load(Ordering::SeqCst), "an occupied slot starts no replacement task");
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(consume_task_owner(owner).await, "shutdown consumes the event receiver");
        assert_eq!(drops.load(Ordering::SeqCst), 2, "both original listeners are joined");
    }
}

impl MtlsInterceptInstallError {
    /// Associated constructor for the site-1 outbound nft-TPROXY install
    /// failure. `#[source]` wrap (not `#[from]`, which the `Inbound` variant
    /// owns), so the call site names this constructor explicitly.
    #[must_use]
    const fn outbound_tproxy_install(source: InterceptError) -> Self {
        Self::OutboundTproxyInstall(source)
    }

    /// The closed-vocabulary install-stage label for the
    /// [`TransitionReason::MtlsInterceptInstallFailed`] cause-class the shim
    /// writes. Maps the 5-variant error (and, for [`Self::Inbound`], the
    /// inner [`InterceptError`] variant) to the four pinned stage strings:
    /// `"outbound_tproxy_install"`, `"leg_f_bind"`,
    /// `"leg_c_transparent_listener"`, `"inbound_tproxy"`. The leg-F/leg-C
    /// `local_addr` capture failures (D-MTLS-18 sites 2/3) reuse the EXISTING
    /// leg-F / leg-C stage strings — the bind and its bound-addr capture are the
    /// same install stage from the shim's vocabulary perspective. Internal
    /// mapping helper — NOT new contract surface.
    ///
    /// [`TransitionReason::MtlsInterceptInstallFailed`]:
    ///     overdrive_core::transition_reason::TransitionReason::MtlsInterceptInstallFailed
    #[must_use]
    pub const fn stage(&self) -> &'static str {
        match self {
            Self::GenerationExhausted { .. } => "generation_exhausted",
            Self::RegistrationConflict { .. } => "registration_conflict",
            Self::RegistrationRetired { .. } => "registration_retired",
            Self::SharedOwner { .. } => "shared_owner",
            Self::OwnerShutdown => "owner_shutdown",
            Self::PriorTeardown { .. } => "prior_teardown",
            Self::OutboundTproxyInstall(_) => "outbound_tproxy_install",
            Self::LegFBind(_) | Self::LegFLocalAddr { .. } => "leg_f_bind",
            Self::LegCLocalAddr { .. }
            | Self::Inbound(InterceptError::TransparentListener { .. }) => {
                "leg_c_transparent_listener"
            }
            // Every other `InterceptError` reaching the install path is the
            // site-4 nft-TPROXY install (`NftRuleInstallFailed` /
            // `NftHandleRecoveryFailed` / `IpRuleAddFailed` /
            // `IpRouteLocalAddFailed`); the accept/orig-dst variants arise only
            // on the per-connection accept loop, never on `start_alloc`'s
            // install path, so they cannot reach here.
            Self::Inbound(_) => "inbound_tproxy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct RegistrationGeneration(NonZeroU64);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct CapabilityKey {
    alloc: AllocationId,
    generation: RegistrationGeneration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Capability {
    key: CapabilityKey,
    spiffe_id: SpiffeId,
    source_addr: Ipv4Addr,
    allowed_ports: BTreeSet<NonZeroU16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code, reason = "D-295-DISTILL-7 RED scaffold precedes implementation")]
enum CapabilityLifecycle {
    Pending,
    Active,
    Retiring,
}

struct CapabilityElements {
    outbound: Option<Box<dyn InterceptGuard>>,
    inbound: Vec<Box<dyn InterceptGuard>>,
}

#[derive(Clone)]
struct CapabilityRegistry {
    inner: Arc<CapabilityRegistryInner>,
}

struct PendingCapability {
    inner: Arc<CapabilityRegistryInner>,
    key: CapabilityKey,
    elements: Option<CapabilityElements>,
    completed: bool,
}

impl std::fmt::Debug for PendingCapability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingCapability")
            .field("key", &self.key)
            .field("completed", &self.completed)
            .finish_non_exhaustive()
    }
}

struct CapabilityClaim {
    inner: Arc<CapabilityRegistryInner>,
    key: CapabilityKey,
    capability: Capability,
    completed: bool,
}

struct CapabilityRetirement {
    inner: Arc<CapabilityRegistryInner>,
    key: CapabilityKey,
}

struct CapabilityDrain {
    inner: Arc<CapabilityRegistryInner>,
    key: CapabilityKey,
    handles: Vec<EnforcedConnection>,
    relays: Vec<JoinHandle<()>>,
    elements: CapabilityElements,
    removal: Option<(Ipv4Addr, Vec<SocketAddrV4>)>,
    completed: bool,
}

enum PublishDisposition {
    Published,
    Retired(EnforcedConnection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code, reason = "D-295-DISTILL-7 RED scaffold precedes implementation")]
enum ActivationDisposition {
    Activated,
    Retired,
}

struct CapabilityRecord {
    capability: Capability,
    lifecycle: CapabilityLifecycle,
    elements: CapabilityElements,
    has_acquired_elements: bool,
    handles: Vec<EnforcedConnection>,
    relays: Vec<JoinHandle<()>>,
    in_flight: usize,
    pending_owner: bool,
}

struct RegistryState {
    next_generation: u64,
    records: BTreeMap<CapabilityKey, CapabilityRecord>,
    allocations: BTreeMap<AllocationId, CapabilityKey>,
    sources: BTreeMap<Ipv4Addr, CapabilityKey>,
    destinations: BTreeMap<Ipv4Addr, CapabilityKey>,
}

struct CapabilityRegistryInner {
    state: Mutex<RegistryState>,
    wake: Notify,
}

impl RegistryState {
    const fn empty(next_generation: u64) -> Self {
        Self {
            next_generation,
            records: BTreeMap::new(),
            allocations: BTreeMap::new(),
            sources: BTreeMap::new(),
            destinations: BTreeMap::new(),
        }
    }

    fn remove_indexes(&mut self, key: &CapabilityKey) {
        if self.sources.get(&key_source(self, key)).is_some_and(|value| value == key) {
            self.sources.remove(&key_source(self, key));
        }
        if self.destinations.get(&key_source(self, key)).is_some_and(|value| value == key) {
            self.destinations.remove(&key_source(self, key));
        }
    }
}

fn key_source(state: &RegistryState, key: &CapabilityKey) -> Ipv4Addr {
    state.records.get(key).map_or(Ipv4Addr::UNSPECIFIED, |record| record.capability.source_addr)
}

#[allow(clippy::significant_drop_tightening)]
impl CapabilityRegistry {
    fn new() -> Self {
        Self {
            inner: Arc::new(CapabilityRegistryInner {
                state: Mutex::new(RegistryState::empty(1)),
                wake: Notify::new(),
            }),
        }
    }

    #[cfg(test)]
    fn with_next_generation(next_generation: u64) -> Self {
        Self {
            inner: Arc::new(CapabilityRegistryInner {
                state: Mutex::new(RegistryState::empty(next_generation)),
                wake: Notify::new(),
            }),
        }
    }

    fn begin_registration(
        &self,
        alloc: AllocationId,
        source_addr: Ipv4Addr,
        spiffe_id: SpiffeId,
        allowed_ports: BTreeSet<NonZeroU16>,
    ) -> Result<PendingCapability, MtlsInterceptInstallError> {
        let mut state = self.inner.state.lock();
        let next_generation = state.next_generation;
        let next = next_generation
            .checked_add(1)
            .ok_or(MtlsInterceptInstallError::GenerationExhausted { next: u64::MAX })?;
        let address_reserved = state.records.values().any(|record| {
            record.capability.source_addr == source_addr || record.capability.key.alloc == alloc
        });
        if address_reserved {
            return Err(MtlsInterceptInstallError::RegistrationConflict { address: source_addr });
        }
        let Some(generation) = NonZeroU64::new(next_generation) else {
            return Err(MtlsInterceptInstallError::GenerationExhausted { next: u64::MAX });
        };
        let key =
            CapabilityKey { alloc: alloc.clone(), generation: RegistrationGeneration(generation) };
        let capability = Capability { key: key.clone(), spiffe_id, source_addr, allowed_ports };
        state.next_generation = next;
        state.allocations.insert(alloc, key.clone());
        state.records.insert(
            key.clone(),
            CapabilityRecord {
                capability,
                lifecycle: CapabilityLifecycle::Pending,
                elements: CapabilityElements { outbound: None, inbound: Vec::new() },
                has_acquired_elements: false,
                handles: Vec::new(),
                relays: Vec::new(),
                in_flight: 0,
                pending_owner: true,
            },
        );
        Ok(PendingCapability {
            inner: Arc::clone(&self.inner),
            key,
            elements: Some(CapabilityElements { outbound: None, inbound: Vec::new() }),
            completed: false,
        })
    }

    fn claim_source(&self, source_addr: Ipv4Addr) -> Option<CapabilityClaim> {
        let mut state = self.inner.state.lock();
        let key = state.sources.get(&source_addr)?.clone();
        Self::claim_locked(&self.inner, &mut state, key)
    }

    fn claim_destination(
        &self,
        destination_addr: Ipv4Addr,
        destination_port: NonZeroU16,
    ) -> Option<CapabilityClaim> {
        let mut state = self.inner.state.lock();
        let key = state.destinations.get(&destination_addr)?.clone();
        let allowed = state.records.get(&key)?.capability.allowed_ports.contains(&destination_port);
        if !allowed {
            return None;
        }
        Self::claim_locked(&self.inner, &mut state, key)
    }

    fn begin_retire(&self, alloc: &AllocationId) -> Option<CapabilityRetirement> {
        let mut state = self.inner.state.lock();
        let key = state.allocations.get(alloc)?.clone();
        let was_active = state
            .records
            .get(&key)
            .is_some_and(|record| record.lifecycle == CapabilityLifecycle::Active);
        if let Some(record) = state.records.get_mut(&key) {
            record.lifecycle = CapabilityLifecycle::Retiring;
        }
        if was_active {
            state.remove_indexes(&key);
        }
        self.inner.wake.notify_waiters();
        Some(CapabilityRetirement { inner: Arc::clone(&self.inner), key })
    }

    fn expected_intercept_members(&self) -> crate::mtls_intercept_port::InterceptMembers {
        let state = self.inner.state.lock();
        let mut expected = crate::mtls_intercept_port::InterceptMembers::default();
        for record in state.records.values().filter(|record| record.has_acquired_elements) {
            let source = record.capability.source_addr;
            expected.managed_guest_ips.insert(source);
            expected.outbound_sources.insert(source);
            expected.inbound_destinations.extend(
                record
                    .capability
                    .allowed_ports
                    .iter()
                    .map(|port| SocketAddrV4::new(source, port.get())),
            );
        }
        expected
    }

    fn claim_locked(
        inner: &Arc<CapabilityRegistryInner>,
        state: &mut RegistryState,
        key: CapabilityKey,
    ) -> Option<CapabilityClaim> {
        let record = state.records.get_mut(&key)?;
        if record.lifecycle != CapabilityLifecycle::Active {
            return None;
        }
        record.in_flight += 1;
        Some(CapabilityClaim {
            inner: Arc::clone(inner),
            key,
            capability: record.capability.clone(),
            completed: false,
        })
    }
}

#[allow(clippy::significant_drop_tightening)]
impl PendingCapability {
    fn retain_outbound(&mut self, guard: Box<dyn InterceptGuard>) {
        if let Some(elements) = self.elements.as_mut() {
            elements.outbound = Some(guard);
            if let Some(record) = self.inner.state.lock().records.get_mut(&self.key) {
                record.has_acquired_elements = true;
            }
        }
    }

    fn retain_inbound(&mut self, guard: Box<dyn InterceptGuard>) {
        if let Some(elements) = self.elements.as_mut() {
            elements.inbound.push(guard);
            if let Some(record) = self.inner.state.lock().records.get_mut(&self.key) {
                record.has_acquired_elements = true;
            }
        }
    }

    fn activate(mut self) -> ActivationDisposition {
        let elements = self
            .elements
            .take()
            .unwrap_or(CapabilityElements { outbound: None, inbound: Vec::new() });
        let mut disposition = ActivationDisposition::Retired;
        let inner = Arc::clone(&self.inner);
        {
            let mut state = inner.state.lock();
            if let Some(record) = state.records.get_mut(&self.key) {
                record.elements = elements;
                match record.lifecycle {
                    CapabilityLifecycle::Pending => {
                        record.lifecycle = CapabilityLifecycle::Active;
                        record.pending_owner = false;
                        let source = record.capability.source_addr;
                        state.sources.insert(source, self.key.clone());
                        state.destinations.insert(source, self.key.clone());
                        disposition = ActivationDisposition::Activated;
                    }
                    CapabilityLifecycle::Retiring => {
                        record.pending_owner = false;
                    }
                    CapabilityLifecycle::Active => {}
                }
            }
        }
        self.completed = true;
        inner.wake.notify_waiters();
        disposition
    }
}

impl CapabilityClaim {
    const fn capability(&self) -> &Capability {
        &self.capability
    }

    fn publish(mut self, handle: EnforcedConnection) -> PublishDisposition {
        let disposition = {
            let mut state = self.inner.state.lock();
            match state.records.get_mut(&self.key) {
                Some(record) => {
                    record.in_flight = record.in_flight.saturating_sub(1);
                    match record.lifecycle {
                        CapabilityLifecycle::Active => {
                            record.handles.push(handle);
                            PublishDisposition::Published
                        }
                        CapabilityLifecycle::Pending | CapabilityLifecycle::Retiring => {
                            PublishDisposition::Retired(handle)
                        }
                    }
                }
                None => PublishDisposition::Retired(handle),
            }
        };
        self.completed = true;
        self.inner.wake.notify_waiters();
        disposition
    }

    fn retain_relay(&self, relay: JoinHandle<()>) {
        let mut rejected = Some(relay);
        if let Some(record) = self.inner.state.lock().records.get_mut(&self.key)
            && let Some(relay) = rejected.take()
        {
            record.relays.push(relay);
        }
        if let Some(relay) = rejected {
            relay.abort();
        }
    }

    fn release(mut self) {
        self.release_claim();
    }

    fn release_claim(&mut self) {
        if self.completed {
            return;
        }
        if let Some(record) = self.inner.state.lock().records.get_mut(&self.key) {
            record.in_flight = record.in_flight.saturating_sub(1);
        }
        self.completed = true;
        self.inner.wake.notify_waiters();
    }
}

#[allow(clippy::significant_drop_tightening)]
impl Drop for PendingCapability {
    fn drop(&mut self) {
        let elements = self.elements.take();
        drop(elements);
        if self.completed {
            return;
        }
        let mut state = self.inner.state.lock();
        let mut remove = false;
        if let Some(record) = state.records.get_mut(&self.key) {
            match record.lifecycle {
                CapabilityLifecycle::Pending => remove = true,
                CapabilityLifecycle::Retiring => record.pending_owner = false,
                CapabilityLifecycle::Active => {}
            }
        }
        if remove {
            state.allocations.remove(&self.key.alloc);
            state.records.remove(&self.key);
        }
        self.inner.wake.notify_waiters();
    }
}

#[allow(clippy::significant_drop_tightening)]
impl Drop for CapabilityClaim {
    fn drop(&mut self) {
        self.release_claim();
    }
}

#[allow(clippy::significant_drop_tightening)]
impl CapabilityRetirement {
    async fn wait_for_claims(self) -> CapabilityDrain {
        loop {
            let notified = self.inner.wake.notified();
            let ready = {
                let mut state = self.inner.state.lock();
                let Some(record) = state.records.get_mut(&self.key) else {
                    return CapabilityDrain {
                        inner: Arc::clone(&self.inner),
                        key: self.key,
                        handles: Vec::new(),
                        relays: Vec::new(),
                        elements: CapabilityElements { outbound: None, inbound: Vec::new() },
                        removal: None,
                        completed: false,
                    };
                };
                if record.lifecycle == CapabilityLifecycle::Retiring
                    && record.in_flight == 0
                    && !record.pending_owner
                {
                    Some((
                        std::mem::take(&mut record.handles),
                        std::mem::take(&mut record.relays),
                        std::mem::replace(
                            &mut record.elements,
                            CapabilityElements { outbound: None, inbound: Vec::new() },
                        ),
                        (
                            record.capability.source_addr,
                            record
                                .capability
                                .allowed_ports
                                .iter()
                                .map(|port| {
                                    SocketAddrV4::new(record.capability.source_addr, port.get())
                                })
                                .collect(),
                        ),
                    ))
                } else {
                    None
                }
            };
            if let Some((handles, relays, elements, removal)) = ready {
                return CapabilityDrain {
                    inner: Arc::clone(&self.inner),
                    key: self.key,
                    handles,
                    relays,
                    elements,
                    removal: Some(removal),
                    completed: false,
                };
            }
            notified.await;
        }
    }
}

#[allow(clippy::significant_drop_tightening)]
impl CapabilityDrain {
    fn take_handles(&mut self) -> Vec<EnforcedConnection> {
        std::mem::take(&mut self.handles)
    }

    fn take_relays(&mut self) -> Vec<JoinHandle<()>> {
        std::mem::take(&mut self.relays)
    }

    fn take_elements(&mut self) -> CapabilityElements {
        std::mem::replace(
            &mut self.elements,
            CapabilityElements { outbound: None, inbound: Vec::new() },
        )
    }

    fn complete(mut self) {
        if self.completed {
            return;
        }
        let mut state = self.inner.state.lock();
        if state.records.remove(&self.key).is_some() {
            state.allocations.remove(&self.key.alloc);
        }
        self.completed = true;
        self.inner.wake.notify_waiters();
    }
}

struct SharedOwner {
    leg_f_addr: SocketAddrV4,
    leg_c_addr: SocketAddrV4,
    leg_f_listener: Option<Arc<dyn InterceptListener>>,
    leg_c_listener: Option<Arc<dyn InterceptListener>>,
    stop: CancellationToken,
    tasks: Arc<SharedListenerTaskOwner>,
    guard: Option<Box<dyn InterceptGuard>>,
    expected: InterceptPostcondition,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SharedOwnerLifecycle {
    Absent,
    Starting,
    Published,
    ShuttingDown,
}

struct SharedOwnerState {
    lifecycle: SharedOwnerLifecycle,
    owner: Option<SharedOwner>,
}

impl SharedOwnerState {
    const fn new() -> Self {
        Self { lifecycle: SharedOwnerLifecycle::Absent, owner: None }
    }
}

fn shared_listener_task(
    listener: Arc<dyn InterceptListener>,
    stop: CancellationToken,
    worker: Weak<MtlsInterceptWorker>,
    leg: InterceptLeg,
) -> tokio::task::JoinHandle<SharedListenerTaskResult> {
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                () = stop.cancelled() => return Ok(()),
                accepted = listener.accept() => accepted,
            };
            let Some(worker) = worker.upgrade() else {
                return Ok(());
            };
            match leg {
                InterceptLeg::F => match accepted {
                    Ok(connection) => {
                        worker
                            .handle_shared_outbound(
                                *connection.peer.ip(),
                                connection.stream,
                                connection.local,
                            )
                            .await;
                    }
                    Err(InterceptAcceptError::Accept { source }) => return Err(source),
                    Err(InterceptAcceptError::OriginalDestination { source }) => {
                        tracing::warn!(
                            name: "health.mtls.shared_leg_f_acquire_failed",
                            error = %source,
                            "shared leg-F acquisition failed; dropping the connection"
                        );
                    }
                },
                InterceptLeg::C => {
                    let placeholder =
                        AllocationId::new("shared-listener-pending").unwrap_or_else(|_| {
                            unreachable!("static placeholder allocation id is valid")
                        });
                    match accepted {
                        Ok(connection) => worker.handle_shared_inbound(InterceptedConnection {
                            leg: connection.stream,
                            routed: Routed::Inbound { orig_dst: connection.local },
                            alloc: placeholder,
                            expected_peer: None,
                        }),
                        Err(InterceptAcceptError::Accept { source }) => return Err(source),
                        Err(InterceptAcceptError::OriginalDestination { source }) => {
                            tracing::warn!(
                                name: "health.mtls.shared_leg_c_acquire_failed",
                                error = %source,
                                "shared leg-C acquisition failed; dropping the connection"
                            );
                        }
                    }
                }
            }
        }
    })
}

#[allow(clippy::cast_possible_truncation, reason = "sockaddr_in has a fixed platform ABI size")]
fn shared_listener_address(
    leg: InterceptLeg,
    listener: &Arc<dyn InterceptListener>,
) -> Result<SocketAddrV4, MtlsSharedOwnerError> {
    match listener.local_addr() {
        Ok(address) if address.port() != 0 => Ok(address),
        Ok(address) => Err(MtlsSharedOwnerError::ListenerPostcondition {
            leg,
            expected: address,
            observed: Some(address),
        }),
        Err(source) => Err(MtlsSharedOwnerError::ListenerLocalAddr { leg, source }),
    }
}

fn shared_identity_for_ports(
    leg_f: SocketAddrV4,
    leg_c: SocketAddrV4,
) -> Result<InterceptPostcondition, MtlsSharedOwnerError> {
    let identity = overdrive_netlink::nft::SharedIpInterceptIdentity::for_listener_ports(
        leg_f.port(),
        leg_c.port(),
    )
    .map_err(|source| MtlsSharedOwnerError::Intercept {
        source: InterceptError::NftRuleInstallFailed { op: "shared-ip-expected", source },
    })?;
    let (table_and_chains, sets, prerouting, output) = identity.normalized_parts();
    Ok(InterceptPostcondition::ConstantRules { table_and_chains, sets, prerouting, output })
}

#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "D-295-DISTILL-7 acceptance tables use exact Contract Shape markers and diagnostics"
)]
mod capability_registry_acceptance {
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    use super::*;
    use overdrive_core::traits::mtls_enforcement::EnforcedConnectionId;

    struct DropGuard(Arc<AtomicUsize>);

    impl InterceptGuard for DropGuard {}

    impl Drop for DropGuard {
        fn drop(&mut self) {
            self.0.fetch_add(1, AtomicOrdering::SeqCst);
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RegistryRecordSnapshot {
        capability: Capability,
        lifecycle: CapabilityLifecycle,
        outbound_element: bool,
        inbound_elements: usize,
        handles: Vec<EnforcedConnectionId>,
        in_flight: usize,
        pending_owner: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RegistryUniverseSnapshot {
        next_generation: u64,
        records: BTreeMap<CapabilityKey, RegistryRecordSnapshot>,
        allocations: BTreeMap<AllocationId, CapabilityKey>,
        sources: BTreeMap<Ipv4Addr, CapabilityKey>,
        destinations: BTreeMap<Ipv4Addr, CapabilityKey>,
    }

    impl RegistryUniverseSnapshot {
        fn without(mut self, key: &CapabilityKey) -> Self {
            self.records.remove(key);
            self.allocations.retain(|_, value| value != key);
            self.sources.retain(|_, value| value != key);
            self.destinations.retain(|_, value| value != key);
            self
        }
    }

    fn registry_universe(registry: &CapabilityRegistry) -> RegistryUniverseSnapshot {
        let state = registry.inner.state.lock();
        RegistryUniverseSnapshot {
            next_generation: state.next_generation,
            records: state
                .records
                .iter()
                .map(|(key, record)| {
                    (
                        key.clone(),
                        RegistryRecordSnapshot {
                            capability: record.capability.clone(),
                            lifecycle: record.lifecycle,
                            outbound_element: record.elements.outbound.is_some(),
                            inbound_elements: record.elements.inbound.len(),
                            handles: record
                                .handles
                                .iter()
                                .map(|handle| handle.id().clone())
                                .collect(),
                            in_flight: record.in_flight,
                            pending_owner: record.pending_owner,
                        },
                    )
                })
                .collect(),
            allocations: state.allocations.clone(),
            sources: state.sources.clone(),
            destinations: state.destinations.clone(),
        }
    }

    fn alloc(name: &str) -> AllocationId {
        AllocationId::new(name).expect("allocation id")
    }

    fn identity(name: &str) -> SpiffeId {
        SpiffeId::new(&format!("spiffe://overdrive.local/workload/nd295/alloc/{name}"))
            .expect("SPIFFE ID")
    }

    fn ports() -> BTreeSet<NonZeroU16> {
        [8080_u16, 8443]
            .into_iter()
            .map(|port| NonZeroU16::new(port).expect("non-zero port"))
            .collect()
    }

    fn handle(alloc: &AllocationId, sequence: u64) -> EnforcedConnection {
        EnforcedConnection::new(EnforcedConnectionId::new(alloc.clone(), sequence))
    }

    /// Outcome anchor: DISCUSS Elevator Pitch
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn generation_boundaries_and_every_lifecycle_conflict_precede_effects() {
        let exhausted = CapabilityRegistry::with_next_generation(u64::MAX);
        let exhausted_before = registry_universe(&exhausted);
        let error = exhausted
            .begin_registration(
                alloc("generation-max"),
                Ipv4Addr::new(100, 95, 0, 2),
                identity("generation-max"),
                ports(),
            )
            .expect_err("u64::MAX is never minted");
        assert!(matches!(error, MtlsInterceptInstallError::GenerationExhausted { next: u64::MAX }));
        assert_eq!(registry_universe(&exhausted), exhausted_before);

        for lifecycle in [
            CapabilityLifecycle::Pending,
            CapabilityLifecycle::Active,
            CapabilityLifecycle::Retiring,
        ] {
            let registry = CapabilityRegistry::new();
            let first_alloc = alloc(&format!("owner-{lifecycle:?}"));
            let address = Ipv4Addr::new(100, 95, 0, 3);
            let pending = registry
                .begin_registration(
                    first_alloc.clone(),
                    address,
                    identity(first_alloc.as_str()),
                    ports(),
                )
                .expect("first reservation");
            let mut retirement = None;
            match lifecycle {
                CapabilityLifecycle::Pending => {}
                CapabilityLifecycle::Active => {
                    assert_eq!(pending.activate(), ActivationDisposition::Activated);
                }
                CapabilityLifecycle::Retiring => {
                    assert_eq!(pending.activate(), ActivationDisposition::Activated);
                    retirement = registry.begin_retire(&first_alloc);
                }
            }
            assert_eq!(
                retirement.is_some(),
                lifecycle == CapabilityLifecycle::Retiring,
                "only the Retiring partition owns a retirement token"
            );
            let before_conflict = registry_universe(&registry);
            let error = registry
                .begin_registration(
                    alloc(&format!("contender-{lifecycle:?}")),
                    address,
                    identity(&format!("contender-{lifecycle:?}")),
                    ports(),
                )
                .expect_err("every live lifecycle reserves its address");
            assert!(matches!(
                error,
                MtlsInterceptInstallError::RegistrationConflict { address: actual }
                    if actual == address
            ));
            assert_eq!(
                registry_universe(&registry),
                before_conflict,
                "conflict mutates neither generation, records, reservations, indexes, effects, claims, nor handles"
            );
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn max_minus_one_is_minted_once_then_max_is_refused_without_advancing_or_reserving() {
        let registry = CapabilityRegistry::with_next_generation(u64::MAX - 1);
        let first_alloc = alloc("generation-max-minus-one");
        let first_addr = Ipv4Addr::new(100, 95, 0, 11);
        let pending = registry
            .begin_registration(
                first_alloc.clone(),
                first_addr,
                identity(first_alloc.as_str()),
                ports(),
            )
            .expect("max-minus-one is the last mintable generation");
        assert_eq!(pending.activate(), ActivationDisposition::Activated);
        let claim = registry.claim_source(first_addr).expect("last minted capability is active");
        assert_eq!(claim.capability().key.generation.0.get(), u64::MAX - 1);
        drop(claim);

        let second_addr = Ipv4Addr::new(100, 95, 0, 12);
        let error = registry
            .begin_registration(
                alloc("generation-max-refused"),
                second_addr,
                identity("generation-max-refused"),
                ports(),
            )
            .expect_err("u64::MAX is never minted");
        assert!(matches!(error, MtlsInterceptInstallError::GenerationExhausted { next: u64::MAX }));
        assert!(registry.claim_source(second_addr).is_none());
        assert_eq!(
            registry
                .claim_source(first_addr)
                .expect("first capability remains unchanged")
                .capability()
                .key
                .generation
                .0
                .get(),
            u64::MAX - 1
        );
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn pending_retirement_waits_for_activation_and_drains_transferred_elements_once() {
        let registry = CapabilityRegistry::new();
        let allocation = alloc("pending-retirement");
        let address = Ipv4Addr::new(100, 95, 0, 4);
        let drops = Arc::new(AtomicUsize::new(0));
        let mut pending = registry
            .begin_registration(allocation.clone(), address, identity(allocation.as_str()), ports())
            .expect("reserve Pending");
        pending.retain_outbound(Box::new(DropGuard(Arc::clone(&drops))));
        pending.retain_inbound(Box::new(DropGuard(Arc::clone(&drops))));
        let retirement = registry.begin_retire(&allocation).expect("retire Pending");
        let waiting = tokio::spawn(async move { retirement.wait_for_claims().await });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished(), "drain cannot precede Pending-owner handoff");
        assert_eq!(pending.activate(), ActivationDisposition::Retired);
        let mut drain = waiting.await.expect("retirement waiter joins");
        assert!(registry.claim_source(address).is_none());
        assert!(drain.take_handles().is_empty());
        let elements = drain.take_elements();
        assert_eq!(drops.load(AtomicOrdering::SeqCst), 0);
        drop(elements);
        assert_eq!(drops.load(AtomicOrdering::SeqCst), 2);
        drain.complete();
        assert!(
            registry
                .begin_registration(
                    alloc("pending-successor"),
                    address,
                    identity("pending-successor"),
                    ports(),
                )
                .is_ok(),
            "reuse is permitted only after retirement completion"
        );
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn claim_drop_and_late_publication_wake_retirement_without_reattribution() {
        let registry = CapabilityRegistry::new();
        let allocation = alloc("claim-retirement");
        let address = Ipv4Addr::new(100, 95, 0, 5);
        let pending = registry
            .begin_registration(allocation.clone(), address, identity(allocation.as_str()), ports())
            .expect("reserve capability");
        assert_eq!(pending.activate(), ActivationDisposition::Activated);
        let claim_a = registry.claim_source(address).expect("source claim");
        let claim_b = registry
            .claim_destination(address, NonZeroU16::new(8080).expect("port"))
            .expect("destination claim");
        assert_eq!(claim_a.capability().key.alloc, allocation);

        let retirement = registry.begin_retire(&allocation).expect("retire Active");
        let waiting = tokio::spawn(async move { retirement.wait_for_claims().await });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(claim_a);
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished(), "one remaining claim keeps retirement parked");
        let late = handle(&allocation, 1);
        let disposition = claim_b.publish(late);
        assert!(matches!(disposition, PublishDisposition::Retired(_)));
        let PublishDisposition::Retired(late) = disposition else {
            unreachable!();
        };
        assert_eq!(late.id().alloc(), &allocation);
        let mut drain = waiting.await.expect("last claim wakes retirement");
        assert!(drain.take_handles().is_empty(), "late handle is never published");
        drain.complete();
    }

    /// Outcome anchor: DISCUSS Elevator Pitch
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn publication_before_retirement_is_owned_by_only_that_generation_and_allocation() {
        let registry = CapabilityRegistry::new();
        let first = alloc("published-first");
        let second = alloc("published-second");
        let first_addr = Ipv4Addr::new(100, 95, 0, 8);
        let second_addr = Ipv4Addr::new(100, 95, 0, 9);
        for (allocation, address) in [(&first, first_addr), (&second, second_addr)] {
            let pending = registry
                .begin_registration(
                    allocation.clone(),
                    address,
                    identity(allocation.as_str()),
                    ports(),
                )
                .expect("reserve independent capability");
            assert_eq!(pending.activate(), ActivationDisposition::Activated);
        }

        let first_claim = registry.claim_source(first_addr).expect("first source is claimable");
        let first_handle = handle(&first, 11);
        assert!(matches!(first_claim.publish(first_handle), PublishDisposition::Published));
        let second_claim = registry
            .claim_destination(second_addr, NonZeroU16::new(8443).expect("non-zero port"))
            .expect("second destination remains claimable");
        let first_key = registry
            .inner
            .state
            .lock()
            .allocations
            .get(&first)
            .expect("first allocation reservation")
            .clone();
        assert_eq!(second_claim.capability().key.alloc, second);
        drop(second_claim);
        let before_retire = registry_universe(&registry);

        let mut first_drain =
            registry.begin_retire(&first).expect("retire first only").wait_for_claims().await;
        let handles = first_drain.take_handles();
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].id().alloc(), &first);
        assert!(registry.claim_source(first_addr).is_none());
        assert!(registry.claim_source(second_addr).is_some());
        first_drain.complete();
        assert_eq!(
            registry_universe(&registry),
            before_retire.without(&first_key),
            "the exact first capability is the only registry-universe delta"
        );
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn released_address_selects_only_the_successor_and_never_reattributes_a_stale_claim() {
        let registry = CapabilityRegistry::new();
        let address = Ipv4Addr::new(100, 95, 0, 10);
        let predecessor = alloc("generation-a");
        let pending = registry
            .begin_registration(
                predecessor.clone(),
                address,
                identity(predecessor.as_str()),
                ports(),
            )
            .expect("reserve predecessor");
        assert_eq!(pending.activate(), ActivationDisposition::Activated);
        let stale = registry.claim_source(address).expect("claim generation A before removal");
        let retirement = registry.begin_retire(&predecessor).expect("retire generation A");
        let waiter = tokio::spawn(async move { retirement.wait_for_claims().await });
        let late = handle(&predecessor, 21);
        assert!(matches!(stale.publish(late), PublishDisposition::Retired(_)));
        let mut predecessor_drain = waiter.await.expect("generation A drain wakes");
        assert!(predecessor_drain.take_handles().is_empty());
        predecessor_drain.complete();

        let successor = alloc("generation-b");
        let pending = registry
            .begin_registration(successor.clone(), address, identity(successor.as_str()), ports())
            .expect("address becomes reusable only after generation A completion");
        assert_eq!(pending.activate(), ActivationDisposition::Activated);
        let current = registry.claim_source(address).expect("successor source is claimable");
        assert_eq!(current.capability().key.alloc, successor);
        assert_eq!(current.capability().spiffe_id, identity("generation-b"));
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn pending_cancellation_relinquishes_partial_effects_without_releasing_a_retiring_key() {
        for retire_first in [false, true] {
            let registry = CapabilityRegistry::new();
            let allocation = alloc(if retire_first { "cancel-retiring" } else { "cancel-pending" });
            let address = Ipv4Addr::new(100, 95, 0, u8::from(retire_first) + 6);
            let drops = Arc::new(AtomicUsize::new(0));
            let mut pending = registry
                .begin_registration(
                    allocation.clone(),
                    address,
                    identity(allocation.as_str()),
                    ports(),
                )
                .expect("reserve Pending");
            pending.retain_outbound(Box::new(DropGuard(Arc::clone(&drops))));
            let retirement =
                retire_first.then(|| registry.begin_retire(&allocation).expect("retire Pending"));
            drop(pending);
            assert_eq!(drops.load(AtomicOrdering::SeqCst), 1);

            if let Some(retirement) = retirement {
                let mut drain = retirement.wait_for_claims().await;
                assert!(matches!(
                    registry.begin_registration(
                        alloc("too-early-successor"),
                        address,
                        identity("too-early-successor"),
                        ports(),
                    ),
                    Err(MtlsInterceptInstallError::RegistrationConflict { .. })
                ));
                let elements = drain.take_elements();
                drop(elements);
                drain.complete();
            }
            assert!(
                registry
                    .begin_registration(
                        alloc("post-cancel-successor"),
                        address,
                        identity("post-cancel-successor"),
                        ports(),
                    )
                    .is_ok()
            );
        }
    }
}

/// The shared leg-C address recorded for one published allocation capability.
struct AllocIntercept {
    /// The node-shared leg-C listener address used by inbound capability
    /// registration and the diagnostic accessor.
    ///
    /// [`leg_c_addr`]: MtlsInterceptWorker::leg_c_addr
    leg_c_addr: SocketAddrV4,
}

struct AllocStop {
    fence: StopCompletion,
    result: Mutex<Option<Result<(), MtlsInterceptStopError>>>,
    retry_handles: Mutex<Vec<EnforcedConnection>>,
    retry_drain: Mutex<Option<CapabilityDrain>>,
}

struct OwnerStop {
    fence: StopCompletion,
    result: Mutex<Option<Result<(), MtlsInterceptOwnerShutdownError>>>,
}

impl OwnerStop {
    fn new() -> Self {
        Self { fence: StopCompletion::new(), result: Mutex::new(None) }
    }

    async fn wait(&self) -> Result<(), MtlsInterceptOwnerShutdownError> {
        self.fence.wait().await;
        self.result
            .lock()
            .clone()
            .unwrap_or_else(|| unreachable!("owner fence opens only after result is stored"))
    }
}

impl AllocStop {
    fn new() -> Self {
        Self {
            fence: StopCompletion::new(),
            result: Mutex::new(None),
            retry_handles: Mutex::new(Vec::new()),
            retry_drain: Mutex::new(None),
        }
    }

    async fn wait(&self) -> Result<(), MtlsInterceptStopError> {
        self.fence.wait().await;
        self.result
            .lock()
            .clone()
            .unwrap_or_else(|| unreachable!("completion fence opens only after result is stored"))
    }
}

/// Private, cancellation-safe completion for one allocation stop or the
/// worker's one process-owner shutdown. The independently owned supervisor
/// keeps running if the first waiter is cancelled; later waiters observe the
/// same retained terminal value.
#[derive(Clone)]
struct StopCompletion {
    inner: Arc<StopCompletionInner>,
}

struct StopCompletionInner {
    started: Mutex<bool>,
    complete: watch::Sender<bool>,
}

struct StopCompletionGuard(Arc<StopCompletionInner>);

impl Drop for StopCompletionGuard {
    fn drop(&mut self) {
        self.0.complete.send_replace(true);
    }
}

impl StopCompletion {
    fn new() -> Self {
        let (complete, _receiver) = watch::channel(false);
        Self { inner: Arc::new(StopCompletionInner { started: Mutex::new(false), complete }) }
    }

    fn is_complete(&self) -> bool {
        *self.inner.complete.borrow()
    }

    fn complete(&self) {
        self.inner.complete.send_replace(true);
    }

    fn start_with<F, Fut>(&self, work: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let starts = {
            let mut started = self.inner.started.lock();
            if *started {
                false
            } else {
                *started = true;
                true
            }
        };
        if starts {
            let inner = Arc::clone(&self.inner);
            tokio::spawn(async move {
                let _completion = StopCompletionGuard(inner);
                work().await;
            });
        }
    }

    async fn wait(&self) {
        let mut complete = self.inner.complete.subscribe();
        while !*complete.borrow() {
            if complete.changed().await.is_err() {
                break;
            }
        }
    }
}

/// The worker-side mTLS intercept-and-enforce lifecycle component.
///
/// Constructed ONCE at the control-plane composition root, AFTER
/// `IdentityMgr` (so `HostMtlsEnforcement` can read the held identity),
/// with both ports as REQUIRED `new()` params per
/// `.claude/rules/development.md` § "Port-trait dependencies". Held by
/// `AppState` as a required `Arc<MtlsInterceptWorker>` in every serve
/// composition.
pub struct MtlsInterceptWorker {
    /// The per-connection enforcement port (`HostMtlsEnforcement` in
    /// production; `SimMtlsEnforcement` under test composition).
    enforcement: Arc<dyn MtlsEnforcement>,
    /// The per-connection enrollment-resolve port (`ServiceBackendsResolve` in
    /// production; `SimMtlsResolve` under test composition; ADR-0071 fact 4,
    /// the #242 anti-corruption boundary). The outbound accept loop resolves
    /// each captured connection's `getsockname`-recovered `orig_dst` against
    /// the mesh through this port and branches on the returned
    /// [`MtlsResolution`] variant (the C1 3-arm decision —
    /// `Mesh`→enforce / `NonMesh`→cleartext pass-through /
    /// `MeshUnreachable`→fail-closed). Mandatory `new()` param, no builder
    /// (`.claude/rules/development.md` § "Port-trait dependencies").
    resolve: Arc<dyn MtlsResolve>,
    /// Injected `Clock` per the mandatory-port-dependency rule. Reserved
    /// for the deferred per-connection progress-stall watchdog
    /// ([#232](https://github.com/overdrive-sh/overdrive/issues/232));
    /// liveness in v1 is (C) kernel + (B) self-teardown, neither of which
    /// reads the clock here.
    _clock: Arc<dyn Clock>,
    /// The intercept port (`HostMtlsIntercept` in production;
    /// `SimMtlsIntercept` under test composition). It owns node-shared
    /// transparent listener binds and shared-program operations, plus the
    /// per-allocation source/destination elements. Mandatory `new()` param,
    /// no builder (`.claude/rules/development.md` § "Port-trait dependencies").
    intercept: Arc<dyn MtlsIntercept>,
    /// One process-local registration/capability state machine.
    #[allow(dead_code, reason = "D-295-DISTILL-7 RED scaffold activated by the single cut")]
    capabilities: CapabilityRegistry,
    /// The one node-owned listener/rule owner. Allocation records refer to
    /// this owner; they never own or replace its sockets.
    shared_owner: Mutex<SharedOwnerState>,
    /// Serializes allocation element installs and removals with shared member
    /// audit and repair operations.
    element_effects: tokio::sync::Mutex<()>,
    /// Per-alloc teardown bookkeeping (D-MTLS-16). `BTreeMap` per
    /// `.claude/rules/development.md` § "Ordered-collection choice" — the
    /// set is drained deterministically on stop.
    intercepts: Mutex<BTreeMap<AllocationId, AllocIntercept>>,
    /// Allocations whose shared capability registration has acquired effects
    /// but has not reached the atomic Pending -> Active/Retired linearization.
    /// A stop that arrives in this window transfers retirement ownership to
    /// the capability registry and waits for the pending owner handshake.
    pending_allocations: Mutex<BTreeSet<AllocationId>>,
    /// In-progress and completed stop generations. Kept until owner shutdown
    /// so duplicate callers and terminal retries observe the same result.
    stopping: Mutex<BTreeMap<AllocationId, Vec<Arc<AllocStop>>>>,
    /// Atomic install/shutdown gate. A write-side owner shutdown cannot take
    /// its intercept snapshot until every read-side install/stop mutation has
    /// completed; once closed, no new install can acquire the gate.
    lifecycle: RwLock<WorkerLifecycle>,
    /// One-shot process-owner completion. Unlike allocation stop, owner
    /// shutdown has no retry generation: it seals registration, joins every
    /// userspace child, and retains its aggregate result for all callers.
    shutdown: Arc<OwnerStop>,
    /// Exact action-boundary invocation witness used by integration tests that
    /// prove callers fence duplicate terminal transitions before asking this
    /// worker to stop the same allocation again.
    #[cfg(any(test, feature = "integration-tests"))]
    stop_alloc_calls: AtomicU64,
    /// Completed-stop witness retained only for test observation after
    /// production stopping entries are removed on convergence.
    #[cfg(any(test, feature = "integration-tests"))]
    completed_stops: Mutex<BTreeSet<AllocationId>>,
}

impl Drop for MtlsInterceptWorker {
    fn drop(&mut self) {
        if let Some(owner) = self.shared_owner.get_mut().owner.as_ref() {
            owner.stop.cancel();
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum WorkerLifecycle {
    Open,
    Shutdown,
}

impl MtlsInterceptWorker {
    /// Construct from the REQUIRED ports. `enforcement`, `resolve`, and `clock`
    /// are all mandatory — no defaulting, no builder
    /// (`.claude/rules/development.md` § "Port-trait dependencies": a builder
    /// makes the dependency optional, and "optional" means "tests can forget";
    /// the compiler enforces every call site is explicit).
    ///
    /// The node-shared listeners and constant nft program are started once by
    /// [`start_shared_owner`](Self::start_shared_owner). Each allocation adds
    /// source and destination elements under that program; it does not bind
    /// listeners or install per-interface rules.
    ///
    /// As of step 04-02 the worker holds the [`MtlsResolve`] port: the outbound
    /// accept loop resolves each captured connection's recovered `orig_dst`
    /// through it and branches on the [`MtlsResolution`] variant — production
    /// wires `ServiceBackendsResolve` (reading `service_backends`), tests wire
    /// `SimMtlsResolve`.
    ///
    /// The [`MtlsIntercept`] port supplies the node-shared listener and
    /// shared-program effects. Production wires `HostMtlsIntercept`; test
    /// compositions wire `SimMtlsIntercept` or a test-local port.
    #[must_use]
    pub fn new(
        enforcement: Arc<dyn MtlsEnforcement>,
        resolve: Arc<dyn MtlsResolve>,
        clock: Arc<dyn Clock>,
        intercept: Arc<dyn MtlsIntercept>,
    ) -> Self {
        Self {
            enforcement,
            resolve,
            _clock: clock,
            intercept,
            capabilities: CapabilityRegistry::new(),
            shared_owner: Mutex::new(SharedOwnerState::new()),
            element_effects: tokio::sync::Mutex::new(()),
            intercepts: Mutex::new(BTreeMap::new()),
            pending_allocations: Mutex::new(BTreeSet::new()),
            stopping: Mutex::new(BTreeMap::new()),
            lifecycle: RwLock::new(WorkerLifecycle::Open),
            shutdown: Arc::new(OwnerStop::new()),
            #[cfg(any(test, feature = "integration-tests"))]
            stop_alloc_calls: AtomicU64::new(0),
            #[cfg(any(test, feature = "integration-tests"))]
            completed_stops: Mutex::new(BTreeSet::new()),
        }
    }

    /// Start and publish the one node-shared listener owner only after its
    /// sockets, node guard, tasks, and full audit are complete.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn start_shared_owner(self: &Arc<Self>) -> Result<(), MtlsSharedOwnerError> {
        {
            let mut state = self.shared_owner.lock();
            match state.lifecycle {
                SharedOwnerLifecycle::Published => return Ok(()),
                SharedOwnerLifecycle::Starting => return Err(MtlsSharedOwnerError::NotStarted),
                SharedOwnerLifecycle::ShuttingDown => {
                    return Err(MtlsSharedOwnerError::OwnerShutdown);
                }
                SharedOwnerLifecycle::Absent => state.lifecycle = SharedOwnerLifecycle::Starting,
            }
        }

        let result = self.start_shared_owner_inner().await;
        let mut state = self.shared_owner.lock();
        match result {
            Ok(owner) => {
                state.owner = Some(owner);
                state.lifecycle = SharedOwnerLifecycle::Published;
                Ok(())
            }
            Err(source) => {
                state.owner = None;
                state.lifecycle = SharedOwnerLifecycle::Absent;
                Err(source)
            }
        }
    }

    /// Resolve when the retained F/C task observer reports an abnormal exit.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn wait_shared_owner_failure(&self) -> MtlsSharedOwnerError {
        let tasks = {
            let state = self.shared_owner.lock();
            let Some(owner) = state.owner.as_ref() else {
                return match state.lifecycle {
                    SharedOwnerLifecycle::ShuttingDown => MtlsSharedOwnerError::OwnerShutdown,
                    _ => MtlsSharedOwnerError::NotStarted,
                };
            };
            Arc::clone(&owner.tasks)
        };
        tasks.wait_failure().await
    }

    /// Repair only the failed shared listener/task at its recorded address.
    #[allow(clippy::unused_async, reason = "the exact seven-method worker surface remains async")]
    pub async fn converge_shared_owner(self: &Arc<Self>) -> Result<(), MtlsSharedOwnerError> {
        let mut owner = {
            let mut state = self.shared_owner.lock();
            if state.lifecycle == SharedOwnerLifecycle::ShuttingDown {
                return Err(MtlsSharedOwnerError::OwnerShutdown);
            }
            let Some(owner) = state.owner.take() else {
                return Err(MtlsSharedOwnerError::NotStarted);
            };
            state.lifecycle = SharedOwnerLifecycle::Starting;
            owner
        };
        let result = {
            let _element_effects = self.element_effects.lock().await;
            self.converge_shared_owner_inner(&mut owner)
        };
        let mut state = self.shared_owner.lock();
        state.owner = Some(owner);
        state.lifecycle = SharedOwnerLifecycle::Published;
        result
    }

    /// Non-repairing read-back of sockets, tasks, and the shared rule program.
    #[allow(
        clippy::significant_drop_tightening,
        clippy::unused_async,
        reason = "the exact seven-method worker surface remains async"
    )]
    pub async fn audit_shared_owner(&self) -> Result<(), MtlsSharedOwnerError> {
        let element_effects = self.element_effects.lock().await;
        #[cfg(feature = "integration-tests")]
        let hold_started = std::time::Instant::now();
        let result = {
            let state = self.shared_owner.lock();
            state.owner.as_ref().map_or_else(
                || match state.lifecycle {
                    SharedOwnerLifecycle::ShuttingDown => Err(MtlsSharedOwnerError::OwnerShutdown),
                    _ => Err(MtlsSharedOwnerError::NotStarted),
                },
                |owner| self.audit_shared_owner_snapshot(owner),
            )
        };
        drop(element_effects);
        #[cfg(feature = "integration-tests")]
        {
            let hold = hold_started.elapsed();
            tracing::info!(
                target: "overdrive::netns_density_benchmark",
                event = "e18.member_audit_mutex_hold",
                hold_secs = hold.as_secs(),
                hold_subsec_nanos = hold.subsec_nanos(),
                "E18 member audit mutex hold sample"
            );
        }
        result
    }

    #[allow(clippy::similar_names)]
    async fn start_shared_owner_inner(
        self: &Arc<Self>,
    ) -> Result<SharedOwner, MtlsSharedOwnerError> {
        self.clear_boot_members()?;
        let prior = self
            .intercept
            .observe_shared()
            .map_err(|source| MtlsSharedOwnerError::Intercept { source })?;
        let requested = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0);
        let leg_f_listener = self.intercept.bind_transparent(requested).map_err(|source| {
            MtlsSharedOwnerError::ListenerBind { leg: InterceptLeg::F, requested, source }
        })?;
        let leg_f_addr = shared_listener_address(InterceptLeg::F, &leg_f_listener)?;
        let leg_c_listener = match self.intercept.bind_transparent(requested) {
            Ok(listener) => listener,
            Err(source) => {
                return Err(MtlsSharedOwnerError::ListenerBind {
                    leg: InterceptLeg::C,
                    requested,
                    source,
                });
            }
        };
        let leg_c_addr = shared_listener_address(InterceptLeg::C, &leg_c_listener)?;
        let guard = self
            .intercept
            .converge_shared(prior.as_ref(), leg_f_addr, leg_c_addr)
            .map_err(|source| MtlsSharedOwnerError::Intercept { source })?;
        let expected = self
            .intercept
            .observe_shared()
            .map_err(|source| MtlsSharedOwnerError::Intercept { source })?
            .or_else(|| shared_identity_for_ports(leg_f_addr, leg_c_addr).ok())
            .ok_or_else(|| MtlsSharedOwnerError::Intercept {
                source: InterceptError::PostconditionMismatch {
                    expected: shared_identity_for_ports(leg_f_addr, leg_c_addr).unwrap_or_else(
                        |_| InterceptPostcondition::ListenerPort {
                            leg: InterceptLeg::F,
                            port: leg_f_addr.port(),
                        },
                    ),
                    observed: None,
                },
            })?;
        self.verify_boot_intercept_state(&expected)?;
        let stop = CancellationToken::new();
        let tasks = Arc::new(SharedListenerTaskOwner::new(
            shared_listener_task(
                Arc::clone(&leg_f_listener),
                stop.clone(),
                Arc::downgrade(self),
                InterceptLeg::F,
            ),
            shared_listener_task(
                Arc::clone(&leg_c_listener),
                stop.clone(),
                Arc::downgrade(self),
                InterceptLeg::C,
            ),
        ));
        let owner = SharedOwner {
            leg_f_addr,
            leg_c_addr,
            leg_f_listener: Some(leg_f_listener),
            leg_c_listener: Some(leg_c_listener),
            stop,
            tasks,
            guard: Some(guard),
            expected,
        };
        let audit = {
            let _element_effects = self.element_effects.lock().await;
            self.audit_shared_owner_snapshot(&owner)
        };
        if let Err(source) = audit {
            owner.stop.cancel();
            shutdown_shared_listener_tasks(Arc::clone(&owner.tasks)).await;
            drop(owner);
            return Err(source);
        }
        Ok(owner)
    }

    fn clear_boot_members(&self) -> Result<(), MtlsSharedOwnerError> {
        let empty = crate::mtls_intercept_port::InterceptMembers::default();
        let cleared = self
            .intercept
            .converge_allocation_elements(&empty)
            .map_err(|source| MtlsSharedOwnerError::BootMemberClear { source })?;
        if let Some(state) = cleared
            && state.members != empty
        {
            return Err(MtlsSharedOwnerError::BootMemberClear {
                source: InterceptError::MembersRemain { observed: state.members },
            });
        }
        Ok(())
    }

    fn verify_boot_intercept_state(
        &self,
        expected: &InterceptPostcondition,
    ) -> Result<(), MtlsSharedOwnerError> {
        let observed = self
            .intercept
            .observe_shared_state()
            .map_err(|source| MtlsSharedOwnerError::Intercept { source })?
            .ok_or_else(|| MtlsSharedOwnerError::Intercept {
                source: InterceptError::PostconditionMismatch {
                    expected: expected.clone(),
                    observed: None,
                },
            })?;
        if observed.program != *expected {
            return Err(MtlsSharedOwnerError::Intercept {
                source: InterceptError::PostconditionMismatch {
                    expected: expected.clone(),
                    observed: Some(observed.program),
                },
            });
        }
        if !observed.policy_route {
            return Err(MtlsSharedOwnerError::Intercept {
                source: InterceptError::PolicyRouteAbsent,
            });
        }
        if !observed.intercept_mark_guard {
            return Err(MtlsSharedOwnerError::Intercept {
                source: InterceptError::InterceptMarkGuardAbsent,
            });
        }
        if observed.members != crate::mtls_intercept_port::InterceptMembers::default() {
            return Err(MtlsSharedOwnerError::BootMemberClear {
                source: InterceptError::MembersRemain { observed: observed.members },
            });
        }
        Ok(())
    }

    fn audit_shared_owner_snapshot(&self, owner: &SharedOwner) -> Result<(), MtlsSharedOwnerError> {
        // The retained accept-task observer owns the terminal event that the
        // supervisor consumes before exact-port replacement. Prefer that
        // terminal outcome over a simultaneous socket-address read failure;
        // otherwise the repair path can reach `replace_terminal` while the
        // completed observer still occupies its slot. A live task still
        // reports its listener's exact local-address error below.
        if !owner.tasks.is_live(InterceptLeg::F) {
            return Err(MtlsSharedOwnerError::TaskReturned { leg: InterceptLeg::F });
        }
        if !owner.tasks.is_live(InterceptLeg::C) {
            return Err(MtlsSharedOwnerError::TaskReturned { leg: InterceptLeg::C });
        }
        let observed_f = owner
            .leg_f_listener
            .as_ref()
            .ok_or(MtlsSharedOwnerError::TaskObserverClosed)
            .and_then(|listener| shared_listener_address(InterceptLeg::F, listener))?;
        if observed_f != owner.leg_f_addr {
            return Err(MtlsSharedOwnerError::ListenerPostcondition {
                leg: InterceptLeg::F,
                expected: owner.leg_f_addr,
                observed: Some(observed_f),
            });
        }
        let observed_c = owner
            .leg_c_listener
            .as_ref()
            .ok_or(MtlsSharedOwnerError::TaskObserverClosed)
            .and_then(|listener| shared_listener_address(InterceptLeg::C, listener))?;
        if observed_c != owner.leg_c_addr {
            return Err(MtlsSharedOwnerError::ListenerPostcondition {
                leg: InterceptLeg::C,
                expected: owner.leg_c_addr,
                observed: Some(observed_c),
            });
        }
        let expected_members = self.capabilities.expected_intercept_members();
        let observed = self
            .intercept
            .observe_shared_state()
            .map_err(|source| MtlsSharedOwnerError::Intercept { source })?;
        let Some(observed) = observed else {
            return Err(MtlsSharedOwnerError::Intercept {
                source: InterceptError::PostconditionMismatch {
                    expected: owner.expected.clone(),
                    observed: None,
                },
            });
        };
        if observed.program != owner.expected {
            return Err(MtlsSharedOwnerError::Intercept {
                source: InterceptError::PostconditionMismatch {
                    expected: owner.expected.clone(),
                    observed: Some(observed.program),
                },
            });
        }
        if !observed.policy_route {
            return Err(MtlsSharedOwnerError::Intercept {
                source: InterceptError::PolicyRouteAbsent,
            });
        }
        if !observed.intercept_mark_guard {
            return Err(MtlsSharedOwnerError::Intercept {
                source: InterceptError::InterceptMarkGuardAbsent,
            });
        }
        if observed.members != expected_members {
            return Err(MtlsSharedOwnerError::MemberMismatch {
                expected: expected_members,
                observed: observed.members,
            });
        }
        Ok(())
    }

    #[allow(clippy::unused_async, reason = "exact-port recovery retains the async owner boundary")]
    fn converge_shared_owner_inner(
        self: &Arc<Self>,
        owner: &mut SharedOwner,
    ) -> Result<(), MtlsSharedOwnerError> {
        if let Some(dead_leg) = owner.tasks.dead_leg() {
            let (address, listener_slot) = match dead_leg {
                InterceptLeg::F => (owner.leg_f_addr, &mut owner.leg_f_listener),
                InterceptLeg::C => (owner.leg_c_addr, &mut owner.leg_c_listener),
            };
            drop(listener_slot.take());
            let listener = self.intercept.bind_transparent(address).map_err(|source| {
                MtlsSharedOwnerError::ListenerBind { leg: dead_leg, requested: address, source }
            })?;
            let rebound = shared_listener_address(dead_leg, &listener)?;
            if rebound != address {
                return Err(MtlsSharedOwnerError::ListenerPostcondition {
                    leg: dead_leg,
                    expected: address,
                    observed: Some(rebound),
                });
            }
            let task_listener = Arc::clone(&listener);
            let task_stop = owner.stop.clone();
            let task_worker = Arc::downgrade(self);
            owner.tasks.replace_terminal(dead_leg, || {
                shared_listener_task(task_listener, task_stop, task_worker, dead_leg)
            })?;
            *listener_slot = Some(listener);
        }

        let observed = self
            .intercept
            .observe_shared_state()
            .map_err(|source| MtlsSharedOwnerError::Intercept { source })?;
        let prior = match observed {
            None => None,
            Some(state) if state.program == owner.expected => Some(owner.expected.clone()),
            Some(state) => {
                return Err(MtlsSharedOwnerError::Intercept {
                    source: InterceptError::PostconditionMismatch {
                        expected: owner.expected.clone(),
                        observed: Some(state.program),
                    },
                });
            }
        };
        let guard = self
            .intercept
            .converge_shared(prior.as_ref(), owner.leg_f_addr, owner.leg_c_addr)
            .map_err(|source| MtlsSharedOwnerError::Intercept { source })?;
        if let Some(prior_guard) = owner.guard.replace(guard) {
            std::mem::forget(prior_guard);
        }

        let expected_members = self.capabilities.expected_intercept_members();
        self.intercept
            .converge_allocation_elements(&expected_members)
            .map_err(|source| MtlsSharedOwnerError::MemberRepair { source })?;
        self.audit_shared_owner_snapshot(owner)
    }

    /// Register one allocation under the already-started node-shared mTLS owner.
    ///
    /// A networked allocation adds its source address and declared destination
    /// ports under the converged shared program, then publishes a capability
    /// for connections accepted by the node's leg-F and leg-C listeners. It
    /// never binds allocation-owned listeners. Re-firing for the same
    /// allocation first retires the prior capability and waits for its owned
    /// enforcement and pass-through work to end.
    ///
    /// Registration is fail-closed: if the shared owner is unavailable or a
    /// shared element cannot be installed, the error is returned to the action
    /// shim so the allocation cannot proceed without the intercept. Partial
    /// element guards remain local to the pending registration and are dropped
    /// if activation does not complete.
    ///
    /// # Errors
    ///
    /// Returns [`MtlsInterceptInstallError`] when the allocation conflicts
    /// with another capability, the owner is shutting down, the previous
    /// capability cannot be retired, or one of the shared element installs
    /// fails.
    #[allow(
        clippy::similar_names,
        reason = "leg_c_addr (inbound) and leg_f_addr (outbound) are the deliberate \
                  symmetric vocabulary of this crate (D-TME-13 naming decision); the \
                  similarity is the point — leg-C and leg-F are the two TPROXY-divert \
                  targets, and renaming either to dodge the lint would break the \
                  established leg-C/leg-F naming the struct comments and AcceptLeg variants use"
    )]
    pub async fn start_alloc(
        self: &Arc<Self>,
        spec: &AllocationSpec,
    ) -> Result<(), MtlsInterceptInstallError> {
        {
            let lifecycle = self.lifecycle.read();
            if *lifecycle == WorkerLifecycle::Shutdown {
                return Err(MtlsInterceptInstallError::OwnerShutdown);
            }
        }
        // Re-fire safety: the prior exact owner must be completely gone before
        // any replacement listener or rule is acquired. A failed teardown is
        // typed and retryable through `begin_stop_alloc`; readiness/EXEC stays
        // closed because installation has not started.
        if let Some(prior_stop) = self.begin_stop_alloc(&spec.alloc) {
            prior_stop
                .wait()
                .await
                .map_err(|source| MtlsInterceptInstallError::PriorTeardown { source })?;
        }
        {
            let lifecycle = self.lifecycle.read();
            if *lifecycle == WorkerLifecycle::Shutdown {
                return Err(MtlsInterceptInstallError::OwnerShutdown);
            }
        }

        if spec.network.is_none() {
            return Ok(());
        }
        self.start_shared_allocation(spec).await
    }

    #[allow(clippy::similar_names)]
    #[allow(clippy::significant_drop_tightening)]
    async fn start_shared_allocation(
        self: &Arc<Self>,
        spec: &AllocationSpec,
    ) -> Result<(), MtlsInterceptInstallError> {
        let (leg_f_addr, leg_c_addr) = {
            let state = self.shared_owner.lock();
            let Some(owner) = state.owner.as_ref() else {
                return Err(MtlsInterceptInstallError::SharedOwner {
                    source: match state.lifecycle {
                        SharedOwnerLifecycle::ShuttingDown => MtlsSharedOwnerError::OwnerShutdown,
                        _ => MtlsSharedOwnerError::NotStarted,
                    },
                });
            };
            (owner.leg_f_addr, owner.leg_c_addr)
        };
        let allowed_ports = spec.service_ports.iter().copied().collect::<BTreeSet<_>>();
        let Some(network) = spec.network.as_ref() else {
            return Err(MtlsInterceptInstallError::SharedOwner {
                source: MtlsSharedOwnerError::NotStarted,
            });
        };
        let mut pending = {
            let lifecycle = self.lifecycle.write();
            if *lifecycle == WorkerLifecycle::Shutdown {
                return Err(MtlsInterceptInstallError::OwnerShutdown);
            }
            let pending = self.capabilities.begin_registration(
                spec.alloc.clone(),
                network.address,
                spec.identity.clone(),
                allowed_ports,
            )?;
            self.pending_allocations.lock().insert(spec.alloc.clone());
            #[cfg(any(test, feature = "integration-tests"))]
            self.completed_stops.lock().remove(&spec.alloc);
            pending
        };
        let element_effects = self.element_effects.lock().await;
        let effects = (|| {
            // Shared allocations register source/destination set elements
            // against the node-scoped constant program. Passing the TAP name
            // here would re-enter the retired per-interface rule installer
            // and create one nft rule per allocation.
            let outbound = self
                .intercept
                .install_outbound(network.address, leg_f_addr.port())
                .map_err(MtlsInterceptInstallError::outbound_tproxy_install)?;
            pending.retain_outbound(outbound);
            for port in &spec.service_ports {
                let inbound = self.intercept.install_inbound(
                    SocketAddrV4::new(network.address, port.get()),
                    leg_c_addr.port(),
                )?;
                pending.retain_inbound(inbound);
            }
            Ok::<(), MtlsInterceptInstallError>(())
        })();
        if let Err(source) = effects {
            drop(pending);
            self.pending_allocations.lock().remove(&spec.alloc);
            return Err(source);
        }
        drop(element_effects);
        // Let a concurrent stop/shutdown owner transfer retirement before the
        // Pending -> Active linearization.  The effect acquisition remains
        // synchronous, but activation is the explicit handoff boundary.
        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
        let activation = {
            // `begin_stop_alloc` holds the read side while it transfers
            // retirement ownership. Taking the write side here makes that
            // transfer and Pending -> Active mutually exclusive even when the
            // runtime schedules both callers on different worker threads.
            let lifecycle = self.lifecycle.write();
            if *lifecycle == WorkerLifecycle::Shutdown {
                let _ = self.capabilities.begin_retire(&spec.alloc);
            }
            let activation = pending.activate();
            if activation == ActivationDisposition::Activated {
                self.pending_allocations.lock().remove(&spec.alloc);
                self.intercepts.lock().insert(spec.alloc.clone(), AllocIntercept { leg_c_addr });
            }
            activation
        };
        if activation == ActivationDisposition::Retired {
            return Err(MtlsInterceptInstallError::RegistrationRetired {
                alloc_id: spec.alloc.clone(),
            });
        }
        Ok(())
    }

    /// Retire one allocation capability and drain its owned connection work.
    /// The node-shared listeners remain live. Idempotent: a stop for an
    /// unknown allocation is a no-op, and concurrent callers join the same
    /// retirement attempt.
    pub async fn stop_alloc(
        self: &Arc<Self>,
        alloc_id: &AllocationId,
    ) -> Result<(), MtlsInterceptStopError> {
        if let Some(stop) = self.begin_stop_alloc(alloc_id) {
            return stop.wait().await;
        }
        let shutdown_started = *self.lifecycle.read() == WorkerLifecycle::Shutdown;
        if !shutdown_started {
            return Ok(());
        }
        match self.shutdown_owner().await {
            Ok(()) => Ok(()),
            Err(shutdown) => shutdown
                .failures
                .into_iter()
                .find(|failure| stop_error_alloc_id(failure) == alloc_id)
                .map_or(Ok(()), Err),
        }
    }

    fn begin_stop_alloc(self: &Arc<Self>, alloc_id: &AllocationId) -> Option<Arc<AllocStop>> {
        let lifecycle = self.lifecycle.read();
        if *lifecycle == WorkerLifecycle::Shutdown {
            return None;
        }
        let mut intercepts = self.intercepts.lock();
        let mut pending_allocations = self.pending_allocations.lock();
        let mut stopping = self.stopping.lock();

        if let Some(previous) = stopping.get(alloc_id).and_then(|stops| stops.last()).cloned() {
            let previous_result = previous.result.lock().clone();
            match previous_result {
                None => return Some(previous),
                Some(Ok(())) => {
                    stopping.remove(alloc_id);
                }
                Some(Err(_)) if !previous.fence.is_complete() => return Some(previous),
                Some(Err(_)) => {
                    let retry_handles = std::mem::take(&mut *previous.retry_handles.lock());
                    let Some(drain) = previous.retry_drain.lock().take() else {
                        return Some(previous);
                    };
                    let retry = Arc::new(AllocStop::new());
                    stopping.entry(alloc_id.clone()).or_default().push(Arc::clone(&retry));
                    drop(stopping);
                    drop(pending_allocations);
                    drop(intercepts);
                    drop(lifecycle);
                    start_capability_drain_retry(
                        self,
                        &retry,
                        alloc_id.clone(),
                        drain,
                        retry_handles,
                    );
                    return Some(retry);
                }
            }
        }

        if intercepts.remove(alloc_id).is_some() {
            #[cfg(any(test, feature = "integration-tests"))]
            self.stop_alloc_calls.fetch_add(1, Ordering::SeqCst);
            #[cfg(any(test, feature = "integration-tests"))]
            self.completed_stops.lock().remove(alloc_id);
            let stop = Arc::new(AllocStop::new());
            let retirement = self.capabilities.begin_retire(alloc_id);
            stopping.entry(alloc_id.clone()).or_default().push(Arc::clone(&stop));
            drop(stopping);
            drop(pending_allocations);
            drop(intercepts);
            drop(lifecycle);
            start_capability_retirement(self, &stop, alloc_id.clone(), retirement, false);
            return Some(stop);
        }

        if pending_allocations.contains(alloc_id) {
            let retirement = self.capabilities.begin_retire(alloc_id)?;
            pending_allocations.remove(alloc_id);
            #[cfg(any(test, feature = "integration-tests"))]
            self.completed_stops.lock().remove(alloc_id);
            let stop = Arc::new(AllocStop::new());
            stopping.entry(alloc_id.clone()).or_default().push(Arc::clone(&stop));
            drop(stopping);
            drop(pending_allocations);
            drop(intercepts);
            drop(lifecycle);
            start_capability_retirement(self, &stop, alloc_id.clone(), Some(retirement), true);
            return Some(stop);
        }
        None
    }

    /// Transfer retirement ownership for a shared registration that is still
    /// acquiring its per-allocation effects. The registry keeps its address
    /// reservation and pending-owner bit until the pending owner handoff.
    #[allow(clippy::significant_drop_in_scrutinee)]
    fn begin_pending_stop(self: &Arc<Self>, alloc_id: &AllocationId) -> Option<Arc<AllocStop>> {
        let lifecycle = self.lifecycle.read();
        let intercepts = self.intercepts.lock();
        let mut pending_allocations = self.pending_allocations.lock();
        let mut stopping = self.stopping.lock();
        if let Some(previous) = stopping.get(alloc_id).and_then(|stops| stops.last()).cloned() {
            return Some(previous);
        }
        if !pending_allocations.contains(alloc_id) {
            return None;
        }
        let retirement = self.capabilities.begin_retire(alloc_id)?;
        pending_allocations.remove(alloc_id);
        #[cfg(any(test, feature = "integration-tests"))]
        self.completed_stops.lock().remove(alloc_id);
        let stop = Arc::new(AllocStop::new());
        stopping.entry(alloc_id.clone()).or_default().push(Arc::clone(&stop));
        drop(stopping);
        drop(pending_allocations);
        drop(intercepts);
        drop(lifecycle);
        start_capability_retirement(self, &stop, alloc_id.clone(), Some(retirement), true);
        Some(stop)
    }

    fn remove_successful_stop(&self, alloc_id: &AllocationId, stop: &Arc<AllocStop>) {
        let mut stopping = self.stopping.lock();
        let removed = stopping
            .get(alloc_id)
            .and_then(|stops| stops.last())
            .is_some_and(|current| Arc::ptr_eq(current, stop));
        if removed {
            stopping.remove(alloc_id);
        }
        drop(stopping);
        #[cfg(any(test, feature = "integration-tests"))]
        if removed {
            self.completed_stops.lock().insert(alloc_id.clone());
        }
    }

    /// Number of calls made to [`Self::stop_alloc`]. Test-only observation
    /// surface for action-boundary idempotency; production behavior is
    /// unchanged and no operator API exposes this counter.
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    #[must_use]
    pub fn stop_alloc_calls_for_test(&self) -> u64 {
        self.stop_alloc_calls.load(Ordering::SeqCst)
    }

    /// Whether one allocation's authoritative stop has joined its complete
    /// producer tree and drained every connection handle successfully.
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    #[must_use]
    pub fn alloc_stop_converged_for_test(&self, alloc_id: &AllocationId) -> bool {
        if self.intercepts.lock().contains_key(alloc_id) {
            return false;
        }
        self.stopping.lock().get(alloc_id).and_then(|stops| stops.last()).is_some_and(|stop| {
            stop.fence.is_complete()
                && matches!(*stop.result.lock(), Some(Ok(())))
                && stop.retry_handles.lock().is_empty()
        }) || self.completed_stops.lock().contains(alloc_id)
    }

    /// Invalidate the complete userspace dataplane owned by this worker and
    /// wait until every accept/enforce/pass-through child has ended.
    ///
    /// This is owner supervision, not workload lifecycle cleanup: callers do
    /// not stop a process or author a terminal row. It mirrors process death's
    /// userspace invalidation while deliberately relinquishing active rule
    /// guards without running their destructors, so the original mark-first
    /// rules remain fail-closed until replacement boot reclaims the VM and
    /// performs the ordinary stale-rule sweep.
    pub async fn shutdown_owner(self: &Arc<Self>) -> Result<(), MtlsInterceptOwnerShutdownError> {
        self.begin_shutdown_owner().wait().await
    }

    #[allow(
        clippy::significant_drop_tightening,
        reason = "the lifecycle write guard intentionally spans both owner-map snapshots: it is the atomic install/stop registration fence"
    )]
    #[allow(clippy::too_many_lines)]
    fn begin_shutdown_owner(self: &Arc<Self>) -> Arc<OwnerStop> {
        let attempt = Arc::clone(&self.shutdown);
        let owner = Arc::clone(self);
        let attempt_for_work = Arc::clone(&attempt);
        // Seal admission synchronously, before the one-shot drain task is
        // scheduled.  A Pending registration that reaches its activation
        // fence in the same executor turn therefore observes owner shutdown
        // and is retired rather than published.
        {
            let mut lifecycle = self.lifecycle.write();
            *lifecycle = WorkerLifecycle::Shutdown;
        }
        attempt.fence.start_with(move || async move {
            let (active, pending, in_progress, shared) = {
                let mut lifecycle = owner.lifecycle.write();
                *lifecycle = WorkerLifecycle::Shutdown;
                let shared = {
                    let mut state = owner.shared_owner.lock();
                    state.lifecycle = SharedOwnerLifecycle::ShuttingDown;
                    state.owner.take()
                };
                let active_allocations = std::mem::take(&mut *owner.intercepts.lock());
                let in_progress = owner
                    .stopping
                    .lock()
                    .values()
                    .filter_map(|stops| stops.last().cloned())
                    .collect::<Vec<_>>();
                let mut stopping = owner.stopping.lock();
                let active = active_allocations
                    .into_keys()
                    .map(|alloc_id| {
                        let stop = Arc::new(AllocStop::new());
                        stopping.entry(alloc_id.clone()).or_default().push(Arc::clone(&stop));
                        (alloc_id, stop)
                    })
                    .collect::<Vec<_>>();
                drop(stopping);
                let pending = owner.pending_allocations.lock().iter().cloned().collect::<Vec<_>>();
                (active, pending, in_progress, shared)
            };
            let mut failures = Vec::new();
            let pending_stops = pending
                .iter()
                .filter_map(|alloc_id| owner.begin_pending_stop(alloc_id))
                .collect::<Vec<_>>();
            if let Some(mut shared) = shared {
                shared.stop.cancel();
                shutdown_shared_listener_tasks(Arc::clone(&shared.tasks)).await;
                drop(shared.leg_f_listener.take());
                drop(shared.leg_c_listener.take());
                if let Some(guard) = shared.guard.take() {
                    // The sealed owner path intentionally retains the
                    // constant empty program for next-boot revalidation.
                    std::mem::forget(guard);
                }
            }
            for (alloc_id, stop) in active {
                let result = if let Some(retirement) = owner.capabilities.begin_retire(&alloc_id) {
                    let mut drain = retirement.wait_for_claims().await;
                    stop_cleartext_relays(drain.take_relays()).await;
                    let handles = drain.take_handles();
                    run_capability_stop_attempt(
                        &stop,
                        alloc_id.clone(),
                        drain,
                        handles,
                        Arc::clone(&owner.enforcement),
                        Arc::clone(&owner.intercept),
                        &owner.element_effects,
                    )
                    .await
                } else {
                    *stop.result.lock() = Some(Ok(()));
                    Ok(())
                };
                if result.is_ok() {
                    owner.remove_successful_stop(&alloc_id, &stop);
                } else if let Err(failure) = result {
                    push_stop_failure_once(&mut failures, failure);
                }
                stop.fence.complete();
            }
            for stop in in_progress {
                if let Err(source) = stop.wait().await {
                    push_stop_failure_once(&mut failures, source);
                }
            }
            for stop in pending_stops {
                if let Err(source) = stop.wait().await {
                    push_stop_failure_once(&mut failures, source);
                }
            }
            *attempt_for_work.result.lock() = Some(if failures.is_empty() {
                Ok(())
            } else {
                Err(MtlsInterceptOwnerShutdownError { failures })
            });
        });
        attempt
    }

    /// Dispatch one connection accepted by the node-shared leg-F listener.
    /// The source address is claimed before the asynchronous resolve/enforce
    /// work starts, so the immutable generation remains attached to the
    /// connection if that address is retired and reused meanwhile.
    async fn handle_shared_outbound(
        self: &Arc<Self>,
        source_addr: Ipv4Addr,
        leg_f: std::os::fd::OwnedFd,
        orig_dst: SocketAddrV4,
    ) {
        let Some(claim) = self.capabilities.claim_source(source_addr) else {
            drop(leg_f);
            return;
        };
        let resolution = match self.resolve.resolve(orig_dst).await {
            Ok(resolution) => resolution,
            Err(source) => {
                tracing::warn!(
                    name: "health.mtls.shared_resolve_failed",
                    source_addr = %source_addr,
                    orig_dst = %orig_dst,
                    error = %source,
                    "shared leg-F resolution failed; dropping the connection"
                );
                drop(claim);
                drop(leg_f);
                return;
            }
        };
        match decide_outbound(&resolution) {
            OutboundAction::Enforce { peer } => {
                let alloc = claim.capability().key.alloc.clone();
                self.spawn_shared_enforcement(
                    claim,
                    InterceptedConnection {
                        leg: leg_f,
                        routed: Routed::Outbound { peer },
                        alloc,
                        expected_peer: None,
                    },
                );
            }
            OutboundAction::PassThrough => {
                let alloc = claim.capability().key.alloc.clone();
                let runtime = tokio::runtime::Handle::current();
                let (relay, connected) =
                    spawn_cleartext_passthrough(&runtime, alloc, leg_f, orig_dst);
                claim.retain_relay(relay);
                let _ = connected.await;
                claim.release();
            }
            OutboundAction::FailClosed => {
                drop(claim);
                drop(leg_f);
            }
        }
    }

    /// Dispatch one connection accepted by the node-shared leg-C listener.
    /// The recovered destination selects the exact active capability.
    fn handle_shared_inbound(self: &Arc<Self>, mut connection: InterceptedConnection) {
        let Routed::Inbound { orig_dst } = connection.routed else {
            drop(connection);
            return;
        };
        let Some(port) = NonZeroU16::new(orig_dst.port()) else {
            drop(connection);
            return;
        };
        let Some(claim) = self.capabilities.claim_destination(*orig_dst.ip(), port) else {
            drop(connection);
            return;
        };
        connection.alloc = claim.capability().key.alloc.clone();
        self.spawn_shared_enforcement(claim, connection);
    }

    fn spawn_shared_enforcement(
        self: &Arc<Self>,
        claim: CapabilityClaim,
        connection: InterceptedConnection,
    ) {
        let enforcement = Arc::clone(&self.enforcement);
        tokio::spawn(async move {
            match enforcement.enforce(connection).await {
                Ok(handle) => match claim.publish(handle) {
                    PublishDisposition::Published => {}
                    PublishDisposition::Retired(handle) => {
                        let _ = enforcement.teardown(handle).await;
                    }
                },
                Err(source) => {
                    tracing::warn!(
                        name: "health.mtls.shared_enforce_failed",
                        error = %source,
                        "shared mTLS enforcement refused the connection"
                    );
                    drop(claim);
                }
            }
        });
    }

    /// The node-shared leg-C listener address while `alloc` has an active
    /// capability, or `None` when that allocation has no active capability.
    /// Every active allocation refers to the same listener address; the
    /// listener is owned by the node, not by an allocation.
    ///
    /// Any `AllocationId` is a valid query. This accessor exposes only the
    /// bound socket address and no identity material.
    #[must_use]
    pub fn leg_c_addr(&self, alloc: &AllocationId) -> Option<SocketAddrV4> {
        self.intercepts.lock().get(alloc).map(|i| i.leg_c_addr)
    }
}

fn start_capability_retirement(
    owner: &Arc<MtlsInterceptWorker>,
    stop: &Arc<AllocStop>,
    alloc_id: AllocationId,
    retirement: Option<CapabilityRetirement>,
    pending_handoff: bool,
) {
    let owner = Arc::clone(owner);
    let stop = Arc::clone(stop);
    let stop_for_work = Arc::clone(&stop);
    stop.fence.start_with(move || async move {
        let result = if let Some(retirement) = retirement {
            let mut drain = retirement.wait_for_claims().await;
            if pending_handoff {
                // Give the activation caller a bounded handoff window to
                // return RegistrationRetired and let its action-shim owner
                // quiesce the driver before the retirement owner tears down
                // the acquired mTLS elements.
                for _ in 0..32 {
                    tokio::task::yield_now().await;
                }
            }
            stop_cleartext_relays(drain.take_relays()).await;
            let handles = drain.take_handles();
            run_capability_stop_attempt(
                &stop_for_work,
                alloc_id.clone(),
                drain,
                handles,
                Arc::clone(&owner.enforcement),
                Arc::clone(&owner.intercept),
                &owner.element_effects,
            )
            .await
        } else {
            *stop_for_work.result.lock() = Some(Ok(()));
            Ok(())
        };
        if result.is_ok() {
            owner.remove_successful_stop(&alloc_id, &stop_for_work);
        }
    });
}

fn start_capability_drain_retry(
    owner: &Arc<MtlsInterceptWorker>,
    stop: &Arc<AllocStop>,
    alloc_id: AllocationId,
    drain: CapabilityDrain,
    handles: Vec<EnforcedConnection>,
) {
    let owner = Arc::clone(owner);
    let stop = Arc::clone(stop);
    let stop_for_work = Arc::clone(&stop);
    stop.fence.start_with(move || async move {
        let result = run_capability_stop_attempt(
            &stop_for_work,
            alloc_id.clone(),
            drain,
            handles,
            Arc::clone(&owner.enforcement),
            Arc::clone(&owner.intercept),
            &owner.element_effects,
        )
        .await;
        if result.is_ok() {
            owner.remove_successful_stop(&alloc_id, &stop_for_work);
        }
    });
}

#[allow(clippy::too_many_arguments)]
async fn run_capability_stop_attempt(
    stop: &Arc<AllocStop>,
    alloc_id: AllocationId,
    mut drain: CapabilityDrain,
    handles: Vec<EnforcedConnection>,
    enforcement: Arc<dyn MtlsEnforcement>,
    intercept: Arc<dyn MtlsIntercept>,
    element_effects: &tokio::sync::Mutex<()>,
) -> Result<(), MtlsInterceptStopError> {
    let mut failures = Vec::new();
    let mut retry_handles = Vec::new();
    for handle in handles {
        let id = handle.id().clone();
        let retry_handle = handle.clone();
        if let Err(source) = enforcement.teardown(handle).await {
            failures.push(HandleTeardownFailure { connection: id, source: Arc::new(source) });
            retry_handles.push(retry_handle);
        }
    }

    let result = if failures.is_empty() {
        if let Some((source_addr, destinations)) = drain.removal.clone() {
            let _element_effects = element_effects.lock().await;
            match remove_allocation_elements_blocking(
                Arc::clone(&intercept),
                source_addr,
                destinations,
            )
            .await
            {
                Ok(_) => {
                    drop(drain.take_elements());
                    drain.complete();
                    Ok(())
                }
                Err(source) => {
                    *stop.retry_drain.lock() = Some(drain);
                    Err(MtlsInterceptStopError::ElementRemoval {
                        alloc_id,
                        source: Arc::new(source),
                    })
                }
            }
        } else {
            drop(drain.take_elements());
            drain.complete();
            Ok(())
        }
    } else {
        *stop.retry_drain.lock() = Some(drain);
        Err(MtlsInterceptStopError::HandleTeardown { alloc_id, failures })
    };
    *stop.retry_handles.lock() = retry_handles;
    *stop.result.lock() = Some(result.clone());
    result
}

async fn remove_allocation_elements_blocking(
    intercept: Arc<dyn MtlsIntercept>,
    source_addr: Ipv4Addr,
    destinations: Vec<SocketAddrV4>,
) -> std::result::Result<crate::mtls_intercept_port::InterceptState, InterceptError> {
    match tokio::task::spawn_blocking(move || {
        intercept.remove_allocation_elements(source_addr, &destinations)
    })
    .await
    {
        Ok(result) => result,
        Err(source) => Err(InterceptError::NftElementUpdateFailed {
            set: crate::mtls_intercept::InterceptSet::ManagedGuestIps,
            operation: crate::mtls_intercept::InterceptElementOperation::Delete,
            key: crate::mtls_intercept::InterceptElementKey::Address(source_addr),
            source: crate::mtls_intercept::NetlinkError::nft(
                "shared-element-remove",
                std::io::Error::other(format!("blocking removal task failed: {source}")),
            ),
        }),
    }
}

const fn stop_error_alloc_id(error: &MtlsInterceptStopError) -> &AllocationId {
    match error {
        MtlsInterceptStopError::HandleTeardown { alloc_id, .. }
        | MtlsInterceptStopError::ElementRemoval { alloc_id, .. } => alloc_id,
    }
}

fn push_stop_failure_once(
    failures: &mut Vec<MtlsInterceptStopError>,
    failure: MtlsInterceptStopError,
) {
    if failures
        .iter()
        .all(|existing| stop_error_alloc_id(existing) != stop_error_alloc_id(&failure))
    {
        failures.push(failure);
    }
}

/// The OUTBOUND per-connection decision (the C1 3-arm action — a 1:1 projection
/// of the [`MtlsResolution`] variant the resolve port returns). Kept as a
/// distinct sum type so the decision is a pure, exhaustively-matched function
/// ([`decide_outbound`]) the mutation gate targets per arm — a dropped arm is a
/// security regression (a collapsed `FailClosed`→`PassThrough` = silent
/// cleartext to a should-be-mesh peer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutboundAction {
    /// `Mesh` → enforce mTLS to the RESOLVED backend `peer` (the resolved
    /// `ResolvedBackend.addr`, NOT `orig_dst`).
    Enforce { peer: SocketAddrV4 },
    /// `NonMesh` → cleartext pass-through to `orig_dst`, by design (the
    /// classification arm — not an error, not a fail-closed).
    PassThrough,
    /// `MeshUnreachable` (or an untrusted resolve fault) → refuse, NO cleartext.
    FailClosed,
}

/// The C1 3-arm decision: map an [`MtlsResolution`] to its [`OutboundAction`].
///
/// This is the security-critical core — each arm is independently
/// mutation-killed by the per-arm DST assertions, because a dropped/swapped arm
/// is a distinct bug:
/// - `Mesh(b)` → `Enforce { peer: b.addr }` (the only handshake-driving arm);
/// - `NonMesh` → `PassThrough` (cleartext, by design);
/// - `MeshUnreachable` → `FailClosed` (refuse, NO cleartext — collapsing this
///   to `PassThrough` is the silent-cleartext footgun the enrollment model
///   exists to remove).
///
/// Takes `&MtlsResolution` so the decision is a pure read (the caller still owns
/// the resolution); only the `Copy` `ResolvedBackend.addr` is projected out.
const fn decide_outbound(resolution: &MtlsResolution) -> OutboundAction {
    match resolution {
        MtlsResolution::Mesh(backend) => OutboundAction::Enforce { peer: backend.addr },
        MtlsResolution::NonMesh => OutboundAction::PassThrough,
        MtlsResolution::MeshUnreachable => OutboundAction::FailClosed,
    }
}

/// Spawn a `NonMesh` cleartext relay and report when its upstream connection is
/// established or the task ends. The worker stores its join handle under the
/// connection's capability before releasing the classification claim, so the
/// allocation owner can abort and join it during retirement. A dial failure
/// closes leg-F and completes the readiness signal.
async fn stop_cleartext_relays(relays: Vec<JoinHandle<()>>) {
    for relay in &relays {
        relay.abort();
    }
    for relay in relays {
        let _ = relay.await;
    }
}

fn spawn_cleartext_passthrough(
    runtime: &tokio::runtime::Handle,
    alloc: AllocationId,
    leg_f: std::os::fd::OwnedFd,
    orig_dst: SocketAddrV4,
) -> (JoinHandle<()>, oneshot::Receiver<()>) {
    let (connected_tx, connected_rx) = oneshot::channel();
    let task = runtime.spawn(async move {
        let mut connected_tx = Some(connected_tx);
        let downstream = std::net::TcpStream::from(leg_f);
        if let Err(source) = downstream.set_nonblocking(true) {
            tracing::warn!(
                name: "health.mtls.passthrough_leg_failed",
                alloc = %alloc,
                error = %source,
                "cleartext pass-through could not make captured leg asynchronous"
            );
            return;
        }
        let mut downstream = match tokio::net::TcpStream::from_std(downstream) {
            Ok(stream) => stream,
            Err(source) => {
                tracing::warn!(
                        name: "health.mtls.passthrough_leg_failed",
                        alloc = %alloc,
                        error = %source,
                    "cleartext pass-through could not adopt captured leg"
                );
                return;
            }
        };
        let mut upstream = match tokio::net::TcpStream::connect(orig_dst).await {
            Ok(stream) => stream,
            Err(source) => {
                // The non-mesh upstream is unreachable — close leg-F. This is a
                // plain connectivity failure on a cleartext path, NOT a mesh
                // fail-closed (the resolve already classified it `NonMesh`).
                tracing::warn!(
                    name: "health.mtls.passthrough_dial_failed",
                    alloc = %alloc,
                    orig_dst = %orig_dst,
                    error = %source,
                    "cleartext pass-through dial failed; closing leg-F"
                );
                return;
            }
        };
        if let Some(connected_tx) = connected_tx.take() {
            let _ = connected_tx.send(());
        }
        if let Err(source) = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await {
            tracing::warn!(
                name: "health.mtls.passthrough_relay_ended",
                alloc = %alloc,
                orig_dst = %orig_dst,
                error = %source,
                "cleartext pass-through relay ended"
            );
        }
        // Both streams drop here → both legs close.
    });
    (task, connected_rx)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    reason = "unit-test bodies: a failed precondition must panic with an informative message; \
              test docstrings reference enum-variant names (NonMesh, StoreUnreadable, …) in prose"
)]
mod tests {
    //! Default-lane tests for the node-shared per-connection resolve consumer
    //! (ADR-0071 fact 4 / C1).
    //!
    //! The scenario
    //! `outbound_resolve_consumer_drives_enforce_passthrough_failclosed_per_arm`
    //! drives the worker's outbound handling
    //! ([`MtlsInterceptWorker::handle_shared_outbound`], the driving port for the
    //! resolve consumer) against a scripted [`SimMtlsResolve`] (01-02) per arm
    //! and asserts the OBSERVABLE per-arm outcome at the driven-port boundary:
    //!
    //! - `Mesh(b)` → `enforce` is called with `Routed::Outbound { peer == b.addr }`
    //!   (the RESOLVED backend addr, not `orig_dst`), `expected_peer == None`;
    //! - `NonMesh` → `enforce` is NOT called; the captured leg is relayed
    //!   cleartext to a real upstream that receives the workload's bytes
    //!   (pass-through, by design);
    //! - `MeshUnreachable` → `enforce` is NOT called; NO upstream is dialed; the
    //!   captured leg is closed (the workload sees EOF — fail-closed, no
    //!   cleartext).
    //!
    //! Each arm is asserted DISTINCTLY so an arm-match mutation in
    //! [`decide_outbound`] (the security-critical 3-arm core — a collapsed
    //! `FailClosed`→`PassThrough` is silent cleartext) is independently killed.
    //! Authn-only boundary (Q4 / D-TME-8): the test asserts the
    //! enforce/pass-through/fail-closed routing only — it does NOT call the
    //! wrong-but-valid-peer case "protected" and does NOT thread `IdentityRead`
    //! (`expected_peer` is `None` until #242).

    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::io::Read as _;
    use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
    use std::num::NonZeroU16;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Weak};
    use std::time::Duration;

    use async_trait::async_trait;
    use overdrive_core::traits::driver::{
        AllocationSpec, DriverPayload, GuestNetworkAssignment, Resources, VmPayload,
    };
    use overdrive_core::traits::mtls_enforcement::{
        EnforcedConnection, EnforcedConnectionId, InterceptedConnection, MtlsEnforcement,
        MtlsEnforcementError, PumpLiveness,
    };
    use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve, ResolvedBackend};
    use overdrive_core::{AllocationId, SpiffeId};
    use overdrive_sim::adapters::SimMtlsResolve;
    use overdrive_sim::adapters::clock::SimClock;
    use parking_lot::Mutex;

    use super::{MtlsInterceptWorker, OutboundAction, decide_outbound};
    use crate::mtls_intercept::{
        InterceptElementKey, InterceptElementOperation, InterceptError, InterceptLeg,
        InterceptPostcondition, InterceptSet,
    };
    use crate::mtls_intercept_port::{
        InterceptAcceptError, InterceptAccepted, InterceptGuard, InterceptListener,
        InterceptMembers, MtlsIntercept,
    };

    // ---- shared-owner test support (TS § Intercept listener and stop-error
    // test support, DISTILL B-6/B-7) -------------------------------------------
    //
    // `overdrive-sim` depends on this crate, so in this source-local build the
    // sim's `SimMtlsIntercept` implements a second compiled copy of
    // `MtlsIntercept` and cannot be used. The doubles below carry its
    // semantics: a socket-free listener with scripted accept outcomes, a member
    // model with process-local element tokens, and the removal scripting the
    // B-6 caller-rule bodies need. Its listener follows the B-7 socket-free
    // accept contract so source-local worker tests do not bind host sockets.

    /// One scripted outcome for the next `accept` of a [`TestInterceptListener`]
    /// — the variants and meaning of `SimAcceptScript`.
    #[derive(Debug)]
    enum TestAcceptScript {
        /// The next `accept` returns `Ok(accepted)`.
        Connection(InterceptAccepted),
        /// The next `accept` returns `Err(OriginalDestination { source })`, with
        /// `io::Error::from_raw_os_error(errno)`; the listener stays usable.
        OriginalDestinationFailure { errno: i32 },
        /// That and every later `accept` returns `Err(Accept { source })`, with
        /// `io::Error::from_raw_os_error(errno)`: the listener accepts nothing more.
        ListenerLost { errno: i32 },
    }

    #[derive(Debug, Default)]
    struct TestListenerState {
        scripts: VecDeque<TestAcceptScript>,
        lost: Option<i32>,
        local_addr_errno: Option<i32>,
        parked: usize,
    }

    /// The socket-free listener of [`TestSharedIntercept`], with exactly
    /// `SimInterceptListener`'s semantics (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the `SimMtlsIntercept` contract), TS § Intercept
    /// listener and stop-error test support). Live from its bind until its last
    /// `Arc` drops; the intercept holds only a `Weak`.
    #[derive(Debug)]
    struct TestInterceptListener {
        addr: SocketAddrV4,
        state: Mutex<TestListenerState>,
        wake: tokio::sync::Notify,
    }

    /// Decrements the parked count when a pending accept completes or is dropped.
    struct ParkedAccept<'a> {
        listener: &'a TestInterceptListener,
    }

    impl Drop for ParkedAccept<'_> {
        fn drop(&mut self) {
            self.listener.state.lock().parked -= 1;
        }
    }

    impl TestInterceptListener {
        fn new(addr: SocketAddrV4) -> Self {
            Self {
                addr,
                state: Mutex::new(TestListenerState::default()),
                wake: tokio::sync::Notify::new(),
            }
        }

        fn take_outcome(
            &self,
        ) -> Option<std::result::Result<InterceptAccepted, InterceptAcceptError>> {
            let script = {
                let mut state = self.state.lock();
                if let Some(errno) = state.lost {
                    return Some(Err(InterceptAcceptError::Accept {
                        source: std::io::Error::from_raw_os_error(errno),
                    }));
                }
                let script = state.scripts.pop_front()?;
                if let TestAcceptScript::ListenerLost { errno } = script {
                    state.lost = Some(errno);
                }
                script
            };
            Some(match script {
                TestAcceptScript::Connection(accepted) => Ok(accepted),
                TestAcceptScript::OriginalDestinationFailure { errno } => {
                    Err(InterceptAcceptError::OriginalDestination {
                        source: std::io::Error::from_raw_os_error(errno),
                    })
                }
                TestAcceptScript::ListenerLost { errno } => Err(InterceptAcceptError::Accept {
                    source: std::io::Error::from_raw_os_error(errno),
                }),
            })
        }
    }

    #[async_trait]
    impl InterceptListener for TestInterceptListener {
        fn local_addr(&self) -> std::io::Result<SocketAddrV4> {
            let errno = self.state.lock().local_addr_errno;
            errno.map_or(Ok(self.addr), |errno| Err(std::io::Error::from_raw_os_error(errno)))
        }

        async fn accept(&self) -> std::result::Result<InterceptAccepted, InterceptAcceptError> {
            if tokio::runtime::Handle::try_current().is_err() {
                return Err(InterceptAcceptError::Accept {
                    source: std::io::Error::other(
                        "test intercept listener accept polled with no current Tokio runtime",
                    ),
                });
            }
            let mut parked: Option<ParkedAccept<'_>> = None;
            loop {
                // Created before the outcome is read, so a script appended
                // between the read and the await still wakes this future.
                let notified = self.wake.notified();
                if let Some(outcome) = self.take_outcome() {
                    drop(parked);
                    return outcome;
                }
                if parked.is_none() {
                    self.state.lock().parked += 1;
                    parked = Some(ParkedAccept { listener: self });
                }
                notified.await;
            }
        }
    }

    /// One dynamic member of the owned program's three sets.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum TestMember {
        ManagedGuest(Ipv4Addr),
        OutboundSource(Ipv4Addr),
        InboundDestination(SocketAddrV4),
    }

    /// The member sets plus the process-local element tokens that own them —
    /// the model `SimMtlsIntercept` keeps, so an allocation's members are what
    /// the host adapter's installs would add (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (`remove_allocation_elements` through the netlink surface)).
    #[derive(Debug, Default)]
    struct TestMemberState {
        members: InterceptMembers,
        /// Live token per member: `(generation, holders)`. A retired token is
        /// absent, so a guard holding its generation removes nothing.
        tokens: BTreeMap<TestMember, (u64, usize)>,
        next_generation: u64,
    }

    impl TestMemberState {
        fn insert(&mut self, member: TestMember) {
            match member {
                TestMember::ManagedGuest(address) => {
                    self.members.managed_guest_ips.insert(address);
                }
                TestMember::OutboundSource(address) => {
                    self.members.outbound_sources.insert(address);
                }
                TestMember::InboundDestination(destination) => {
                    self.members.inbound_destinations.insert(destination);
                }
            }
        }

        fn remove(&mut self, member: TestMember) {
            match member {
                TestMember::ManagedGuest(address) => {
                    self.members.managed_guest_ips.remove(&address);
                }
                TestMember::OutboundSource(address) => {
                    self.members.outbound_sources.remove(&address);
                }
                TestMember::InboundDestination(destination) => {
                    self.members.inbound_destinations.remove(&destination);
                }
            }
        }

        /// Acquire one holder of each member's token, adding absent members; a
        /// re-install adopts the live token rather than duplicating the member.
        fn acquire(&mut self, members: &[TestMember]) -> Vec<(TestMember, u64)> {
            members
                .iter()
                .map(|member| {
                    let generation =
                        if let Some((generation, holders)) = self.tokens.get_mut(member) {
                            *holders += 1;
                            *generation
                        } else {
                            self.next_generation += 1;
                            let generation = self.next_generation;
                            self.tokens.insert(*member, (generation, 1));
                            self.insert(*member);
                            generation
                        };
                    (*member, generation)
                })
                .collect()
        }
    }

    /// An allocation install's guard: its `Drop` releases one holder of each
    /// member token it acquired, removing a member whose last holder dropped,
    /// unless `remove_allocation_elements` retired the token first.
    struct TestElementGuard {
        state: Arc<Mutex<TestMemberState>>,
        keys: Vec<(TestMember, u64)>,
    }

    impl InterceptGuard for TestElementGuard {}

    impl Drop for TestElementGuard {
        fn drop(&mut self) {
            let mut state = self.state.lock();
            for (member, generation) in &self.keys {
                let Some((live, holders)) = state.tokens.get_mut(member) else {
                    continue;
                };
                if *live != *generation {
                    continue;
                }
                if *holders > 1 {
                    *holders -= 1;
                } else {
                    state.tokens.remove(member);
                    state.remove(*member);
                }
            }
        }
    }

    /// The node guard `converge_shared` returns; its `Drop` is counted so a
    /// body can tell a relinquished guard from a dropped one.
    struct TestNodeGuard(Arc<AtomicUsize>);

    impl InterceptGuard for TestNodeGuard {}

    impl Drop for TestNodeGuard {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Removal scripting (TS § Intercept listener and stop-error test support).
    #[derive(Default)]
    struct RemovalScript {
        /// Calls still to fail, and the fresh cause each returns.
        failures: Option<(usize, fn() -> InterceptError)>,
        /// While set, every call waits after entering until it is cleared.
        held: bool,
    }

    struct TestSharedIntercept {
        observation: Mutex<Option<InterceptPostcondition>>,
        members: Arc<Mutex<TestMemberState>>,
        /// Weak live-listener table keyed by bound address; the intercept never
        /// extends a listener's life.
        listeners: Mutex<BTreeMap<SocketAddrV4, Weak<TestInterceptListener>>>,
        removal: Mutex<RemovalScript>,
        removal_released: parking_lot::Condvar,
        removal_calls: AtomicUsize,
        node_guard_drops: Arc<AtomicUsize>,
    }

    impl TestSharedIntercept {
        fn new() -> Self {
            Self {
                observation: Mutex::new(None),
                members: Arc::new(Mutex::new(TestMemberState::default())),
                listeners: Mutex::new(BTreeMap::new()),
                removal: Mutex::new(RemovalScript::default()),
                removal_released: parking_lot::Condvar::new(),
                removal_calls: AtomicUsize::new(0),
                node_guard_drops: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn state(&self) -> Option<crate::mtls_intercept_port::InterceptState> {
            let program = self.observation.lock().clone()?;
            Some(crate::mtls_intercept_port::InterceptState {
                program,
                policy_route: true,
                intercept_mark_guard: true,
                members: self.members.lock().members.clone(),
            })
        }

        /// The dynamic members the owned program holds now.
        fn members(&self) -> InterceptMembers {
            self.members.lock().members.clone()
        }

        /// Append `script` to the FIFO of the live listener at `at` and wake a
        /// parked `accept`. `false`, recording nothing, when no live listener
        /// holds `at`.
        #[must_use]
        fn script_accept(&self, at: SocketAddrV4, script: TestAcceptScript) -> bool {
            let Some(listener) = self.live_listener(at) else {
                return false;
            };
            listener.state.lock().scripts.push_back(script);
            listener.wake.notify_waiters();
            true
        }

        /// Make the live listener at `at` report
        /// `Err(io::Error::from_raw_os_error(errno))` from `local_addr()` from now
        /// on. `false`, recording nothing, when no live listener holds `at`.
        #[must_use]
        fn script_local_addr_failure(&self, at: SocketAddrV4, errno: i32) -> bool {
            let Some(listener) = self.live_listener(at) else {
                return false;
            };
            listener.state.lock().local_addr_errno = Some(errno);
            true
        }

        /// The addresses of every live listener, ascending.
        #[must_use]
        fn live_listeners(&self) -> Vec<SocketAddrV4> {
            self.listeners
                .lock()
                .iter()
                .filter(|(_, listener)| listener.strong_count() > 0)
                .map(|(addr, _)| *addr)
                .collect()
        }

        /// The live listener at `at`'s pending, polled, not-dropped `accept`
        /// futures; 0 when no live listener holds `at`.
        #[must_use]
        fn parked_accepts(&self, at: SocketAddrV4) -> usize {
            self.live_listener(at).map_or(0, |listener| listener.state.lock().parked)
        }

        fn live_listener(&self, at: SocketAddrV4) -> Option<Arc<TestInterceptListener>> {
            self.listeners.lock().get(&at).and_then(Weak::upgrade)
        }

        /// Register a socket-free listener: port 0 takes the smallest port ≥
        /// 49152 no live listener of this intercept holds at that IP; a non-zero
        /// address is honoured exactly; an address a live listener holds is
        /// refused with `EADDRINUSE` (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (`bind_transparent` behaviour)).
        #[allow(
            dead_code,
            reason = "the DELIVER step that carries B-7 (05-01 at the latest) makes \
                      bind_transparent return this listener"
        )]
        fn register_listener(
            &self,
            addr: SocketAddrV4,
        ) -> crate::mtls_intercept::Result<Arc<TestInterceptListener>> {
            let mut listeners = self.listeners.lock();
            listeners.retain(|_, listener| listener.strong_count() > 0);
            let bound = if addr.port() == 0 {
                let port = (49_152..=u16::MAX)
                    .find(|port| !listeners.contains_key(&SocketAddrV4::new(*addr.ip(), *port)))
                    .ok_or_else(|| InterceptError::TransparentListener {
                        addr,
                        source: std::io::Error::from_raw_os_error(libc::EADDRINUSE),
                    })?;
                SocketAddrV4::new(*addr.ip(), port)
            } else if listeners.contains_key(&addr) {
                return Err(InterceptError::TransparentListener {
                    addr,
                    source: std::io::Error::from_raw_os_error(libc::EADDRINUSE),
                });
            } else {
                addr
            };
            let listener = Arc::new(TestInterceptListener::new(bound));
            listeners.insert(bound, Arc::downgrade(&listener));
            drop(listeners);
            Ok(listener)
        }

        /// The next `count` calls to `remove_allocation_elements` each return a
        /// fresh `cause()`.
        fn script_removal_failures(&self, count: usize, cause: fn() -> InterceptError) {
            self.removal.lock().failures = (count > 0).then_some((count, cause));
        }

        /// Every `remove_allocation_elements` call, counted on entry.
        fn removal_calls(&self) -> usize {
            self.removal_calls.load(Ordering::SeqCst)
        }

        /// While `held`, each `remove_allocation_elements` call waits after
        /// entering until the hold is released (the condvar barrier
        /// `RetirementBarrierIntercept` uses), so a body can join a caller to an
        /// attempt that is provably in flight.
        fn hold_removals(&self, held: bool) {
            self.removal.lock().held = held;
            if !held {
                self.removal_released.notify_all();
            }
        }

        /// Drops of the node guards `converge_shared` returned.
        fn node_guard_drops(&self) -> usize {
            self.node_guard_drops.load(Ordering::SeqCst)
        }

        /// Wait out a hold, then take one scripted failure if any remain.
        fn enter_removal(&self) -> Option<fn() -> InterceptError> {
            let mut removal = self.removal.lock();
            while removal.held {
                self.removal_released.wait(&mut removal);
            }
            let (remaining, cause) = removal.failures?;
            removal.failures = (remaining > 1).then_some((remaining - 1, cause));
            drop(removal);
            Some(cause)
        }

        /// The B-8 refusal for an element method with no recorded program;
        /// this double conflates the record with the observed program.
        const fn program_not_published() -> InterceptError {
            InterceptError::SharedProgramNotConverged
        }
    }

    type BoundListener = Arc<dyn InterceptListener>;

    impl MtlsIntercept for TestSharedIntercept {
        fn bind_transparent(
            &self,
            address: SocketAddrV4,
        ) -> crate::mtls_intercept::Result<BoundListener> {
            self.register_listener(address).map(|listener| listener as BoundListener)
        }

        fn converge_shared(
            &self,
            _prior: Option<&InterceptPostcondition>,
            leg_f: SocketAddrV4,
            leg_c: SocketAddrV4,
        ) -> crate::mtls_intercept::Result<Box<dyn InterceptGuard>> {
            let (table_and_chains, sets, prerouting, output) =
                overdrive_netlink::nft::SharedIpInterceptIdentity::for_listener_ports(
                    leg_f.port(),
                    leg_c.port(),
                )
                .map_err(|source| InterceptError::NftRuleInstallFailed {
                    op: "shared-test-identity",
                    source,
                })?
                .normalized_parts();
            *self.observation.lock() = Some(InterceptPostcondition::ConstantRules {
                table_and_chains,
                sets,
                prerouting,
                output,
            });
            Ok(Box::new(TestNodeGuard(Arc::clone(&self.node_guard_drops))))
        }

        fn observe_shared(&self) -> crate::mtls_intercept::Result<Option<InterceptPostcondition>> {
            Ok(self.observation.lock().clone())
        }

        fn install_outbound(
            &self,
            source_addr: Ipv4Addr,
            _agent_leg_f_port: u16,
        ) -> crate::mtls_intercept::Result<Box<dyn InterceptGuard>> {
            let keys = self.members.lock().acquire(&[
                TestMember::ManagedGuest(source_addr),
                TestMember::OutboundSource(source_addr),
            ]);
            Ok(Box::new(TestElementGuard { state: Arc::clone(&self.members), keys }))
        }

        fn install_inbound(
            &self,
            virt: SocketAddrV4,
            _agent_leg_c_port: u16,
        ) -> crate::mtls_intercept::Result<Box<dyn InterceptGuard>> {
            let keys = self.members.lock().acquire(&[TestMember::InboundDestination(virt)]);
            Ok(Box::new(TestElementGuard { state: Arc::clone(&self.members), keys }))
        }

        fn observe_shared_state(
            &self,
        ) -> crate::mtls_intercept::Result<Option<crate::mtls_intercept_port::InterceptState>>
        {
            Ok(self.state())
        }

        fn converge_allocation_elements(
            &self,
            expected: &InterceptMembers,
        ) -> crate::mtls_intercept::Result<Option<crate::mtls_intercept_port::InterceptState>>
        {
            if self.observation.lock().is_none() {
                return Ok(None);
            }
            self.members.lock().members = expected.clone();
            Ok(self.state())
        }

        fn remove_allocation_elements(
            &self,
            source_addr: Ipv4Addr,
            destinations: &[SocketAddrV4],
        ) -> crate::mtls_intercept::Result<crate::mtls_intercept_port::InterceptState> {
            self.removal_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(cause) = self.enter_removal() {
                return Err(cause());
            }
            let mut seen = BTreeSet::new();
            if let Some(rejected) = destinations
                .iter()
                .find(|destination| destination.port() == 0 || !seen.insert(**destination))
            {
                return Err(InterceptError::NftElementUpdateFailed {
                    set: InterceptSet::InboundDestinations,
                    operation: InterceptElementOperation::Delete,
                    key: InterceptElementKey::Destination(*rejected),
                    source: crate::mtls_intercept::NetlinkError::nft(
                        "shared-element-remove",
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "duplicate or zero-port destination",
                        ),
                    ),
                });
            }
            if self.observation.lock().is_none() {
                return Err(Self::program_not_published());
            }
            let requested =
                [TestMember::ManagedGuest(source_addr), TestMember::OutboundSource(source_addr)]
                    .into_iter()
                    .chain(
                        destinations
                            .iter()
                            .map(|destination| TestMember::InboundDestination(*destination)),
                    );
            let mut state = self.members.lock();
            for member in requested {
                // Convergent: an absent member is already its postcondition.
                // Retiring the token makes every guard over it a no-op.
                state.remove(member);
                state.tokens.remove(&member);
            }
            drop(state);
            self.state().ok_or_else(Self::program_not_published)
        }
    }

    /// Build a `SimMtlsResolve` that maps `orig_dst` to `arm` (any other addr
    /// resolves to the `NonMesh` default — the host-faithful default per the
    /// 01-02 review).
    fn resolve_scripting(orig_dst: SocketAddrV4, arm: MtlsResolution) -> Arc<dyn MtlsResolve> {
        let mut scripted = BTreeMap::new();
        scripted.insert(orig_dst, arm);
        Arc::new(SimMtlsResolve::new(scripted, MtlsResolution::NonMesh))
    }

    /// Test enforcement port for the pass-through lifecycle bodies. The
    /// resolve result should keep these calls empty; recording them makes an
    /// unexpected mTLS route observable without changing the fixture's wire.
    struct SpyEnforcement {
        calls: Arc<Mutex<Vec<()>>>,
        counter: std::sync::atomic::AtomicU64,
    }

    impl SpyEnforcement {
        fn new() -> (Arc<Self>, Arc<Mutex<Vec<()>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let spy = Arc::new(Self {
                calls: Arc::clone(&calls),
                counter: std::sync::atomic::AtomicU64::new(0),
            });
            (spy, calls)
        }
    }

    #[async_trait]
    impl MtlsEnforcement for SpyEnforcement {
        async fn probe(&self) -> overdrive_core::traits::mtls_enforcement::Result<()> {
            Ok(())
        }

        async fn enforce(
            &self,
            connection: InterceptedConnection,
        ) -> overdrive_core::traits::mtls_enforcement::Result<EnforcedConnection> {
            self.calls.lock().push(());
            let sequence = self.counter.fetch_add(1, Ordering::SeqCst);
            Ok(EnforcedConnection::new(EnforcedConnectionId::new(connection.alloc, sequence)))
        }

        fn liveness(&self, _handle: &EnforcedConnection) -> PumpLiveness {
            PumpLiveness::Running
        }

        async fn teardown(
            &self,
            _handle: EnforcedConnection,
        ) -> overdrive_core::traits::mtls_enforcement::Result<()> {
            Ok(())
        }
    }

    fn alloc(name: &str) -> AllocationId {
        AllocationId::new(name).expect("valid allocation id")
    }

    fn minimal_spec(alloc: AllocationId) -> AllocationSpec {
        AllocationSpec {
            identity: SpiffeId::for_allocation(
                &overdrive_core::WorkloadId::new("worker-owner-test").expect("valid workload id"),
                &alloc,
            ),
            alloc,
            driver: DriverPayload::Vm(VmPayload {
                command: "/bin/true".to_owned(),
                args: Vec::new(),
                kernel: PathBuf::from("/nonexistent/kernel"),
                rootfs: PathBuf::from("/nonexistent/rootfs"),
            }),
            resources: Resources { cpu_milli: 1, memory_bytes: 1 },
            probe_descriptors: Vec::new(),
            network: None,
            service_ports: Vec::new(),
        }
    }

    fn shared_spec(alloc: AllocationId, address: Ipv4Addr) -> AllocationSpec {
        let mut spec = minimal_spec(alloc);
        spec.network = Some(GuestNetworkAssignment {
            address,
            tap: format!("ovd-tp-{}", address.octets()[3]),
            mac: [0x02, 0x00, 100, 95, 0, address.octets()[3]],
            gateway: Ipv4Addr::new(100, 95, 0, 1),
            prefix: 16,
            dns: Ipv4Addr::new(100, 95, 0, 1),
        });
        spec.service_ports = vec![NonZeroU16::new(8443).expect("non-zero port")];
        spec
    }

    /// Stand up a loopback leg-F listener + a client dial, accept the client,
    /// and hand the accepted leg's [`OwnedFd`] back together with the listener's
    /// addr (== the `orig_dst` a getsockname on the accepted socket recovers on
    /// a plain loopback). The connected client stream is returned so the test
    /// can drive bytes / observe EOF through it.
    fn accepted_leg_f() -> (std::os::fd::OwnedFd, SocketAddrV4, TcpStream) {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("bind leg-F loopback listener");
        let leg_f_addr = match listener.local_addr().expect("local_addr") {
            std::net::SocketAddr::V4(a) => a,
            other @ std::net::SocketAddr::V6(_) => panic!("expected V4 addr, got {other}"),
        };
        let client = TcpStream::connect_timeout(&leg_f_addr.into(), Duration::from_secs(5))
            .expect("client dials leg-F");
        client.set_nodelay(true).ok();
        let (accepted, _peer) = listener.accept().expect("accept the client on leg-F");
        accepted.set_nodelay(true).ok();
        (std::os::fd::OwnedFd::from(accepted), leg_f_addr, client)
    }

    // ---- the pure 3-arm decision (the mutation-gate target, per arm) --------

    /// C1 — the 3-arm decision IS the [`MtlsResolution`] variant: `Mesh(b)` →
    /// `Enforce { peer: b.addr }`, `NonMesh` → `PassThrough`, `MeshUnreachable`
    /// → `FailClosed`. Each arm is asserted DISTINCTLY so an arm-match mutation
    /// (the canonical bug shape — a collapsed `FailClosed`→`PassThrough` is
    /// silent cleartext) is independently killed.
    #[test]
    fn decide_outbound_maps_each_resolution_arm_to_its_distinct_action() {
        let backend_addr = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 7), 8443);

        // Mesh → Enforce with the RESOLVED backend addr (not orig_dst).
        assert_eq!(
            decide_outbound(&MtlsResolution::Mesh(ResolvedBackend {
                addr: backend_addr,
                expected_svid: None,
            })),
            OutboundAction::Enforce { peer: backend_addr },
            "Mesh must drive enforce to the resolved backend addr",
        );

        // NonMesh → PassThrough (cleartext, by design — NOT FailClosed).
        assert_eq!(
            decide_outbound(&MtlsResolution::NonMesh),
            OutboundAction::PassThrough,
            "NonMesh must pass through cleartext, never fail-closed",
        );

        // MeshUnreachable → FailClosed (refuse, NO cleartext — NOT PassThrough;
        // collapsing this arm to PassThrough is the silent-cleartext footgun).
        assert_eq!(
            decide_outbound(&MtlsResolution::MeshUnreachable),
            OutboundAction::FailClosed,
            "MeshUnreachable must fail closed, never silently pass through cleartext",
        );
    }

    // ---- the integrated resolve consumer, per arm (port-to-port) -----------

    /// Each `MtlsInterceptInstallError` variant maps to its PINNED closed-
    /// vocabulary install-stage label (the `TransitionReason` cause-class the
    /// action-shim writes). The exact string per variant is load-bearing — the
    /// shim and any operator-facing diagnostic key off it — so each label is
    /// asserted EXACTLY, not merely "non-empty". This pins `leg_f_bind` (the
    /// stage for the leg-F IP_TRANSPARENT bind whose error type this change
    /// migrated to `InterceptError`) alongside its three siblings; replacing any
    /// label string turns this RED.
    #[test]
    fn stage_label_is_pinned_per_install_error_variant() {
        use super::{InterceptError, MtlsInterceptInstallError};
        use crate::mtls_intercept::NetlinkError;

        // The site-4 nft-TPROXY install failure, in the decomposed D3 shape:
        // `NftRuleInstallFailed` carrying the failing op + the real
        // errno-carrying `NetlinkError::Nft` source.
        let nft_install = || InterceptError::NftRuleInstallFailed {
            op: "append-inbound",
            source: NetlinkError::nft(
                "append-inbound",
                std::io::Error::from_raw_os_error(libc::EBUSY),
            ),
        };
        let transparent = || InterceptError::TransparentListener {
            addr: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };

        let cases: [(MtlsInterceptInstallError, &str); 4] = [
            (
                MtlsInterceptInstallError::OutboundTproxyInstall(nft_install()),
                "outbound_tproxy_install",
            ),
            // The leg-F bind site (site 2). Its inner `InterceptError` is what
            // `make_transparent_listener` produces — this change's surface.
            (MtlsInterceptInstallError::LegFBind(transparent()), "leg_f_bind"),
            // Inbound leg-C transparent-listener bind failure → the leg-C label.
            (MtlsInterceptInstallError::Inbound(transparent()), "leg_c_transparent_listener"),
            // Any other inbound `InterceptError` is the site-4 nft-TPROXY install.
            (MtlsInterceptInstallError::Inbound(nft_install()), "inbound_tproxy"),
        ];

        for (err, expected_stage) in cases {
            assert_eq!(
                err.stage(),
                expected_stage,
                "{err:?} must map to stage label {expected_stage:?}"
            );
        }
    }

    // ---- shared capability drain behavior ---------------------------------

    // ---- the orphaned-enforce-task regression (real stop_alloc + spawn_enforce)

    /// Spy [`MtlsEnforcement`] for the orphaned-task regression. `enforce`
    /// signals it has entered (in-flight), then BLOCKS on a release gate, then
    /// records and returns `Ok` — recreating the seconds-wide handshake window
    /// during which `stop_alloc` runs. `teardown` RECORDS the torn-down id so
    /// the test can prove the post-drain handle was reclaimed (fail-closed)
    /// rather than orphaned.
    struct GatedEnforcement {
        /// Set once `enforce` has entered and is about to block on the gate.
        entered: Arc<tokio::sync::Notify>,
        /// Released by the test to let the blocked `enforce` complete its push.
        release: Arc<tokio::sync::Notify>,
        /// The ids `teardown` was called with — the falsifiable surface: a
        /// reclaimed post-drain handle appears here; an orphaned one never does.
        torn_down: Arc<Mutex<Vec<EnforcedConnectionId>>>,
        /// Set when the in-flight enforce future is dropped or completes.
        exited: Arc<AtomicBool>,
        counter: std::sync::atomic::AtomicU64,
    }

    impl GatedEnforcement {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                entered: Arc::new(tokio::sync::Notify::new()),
                release: Arc::new(tokio::sync::Notify::new()),
                torn_down: Arc::new(Mutex::new(Vec::new())),
                exited: Arc::new(AtomicBool::new(false)),
                counter: std::sync::atomic::AtomicU64::new(0),
            })
        }

        /// Await until `enforce` has entered and is blocked on the release gate.
        async fn entered(&self) {
            self.entered.notified().await;
        }

        /// Release the blocked `enforce` so it completes and attempts its push.
        fn release(&self) {
            self.release.notify_one();
        }

        /// The connection ids `teardown` was called with — the falsifiable
        /// surface: a reclaimed post-drain handle appears here; an orphaned one
        /// never does.
        fn torn_down(&self) -> Vec<EnforcedConnectionId> {
            self.torn_down.lock().clone()
        }
    }

    #[async_trait]
    impl MtlsEnforcement for GatedEnforcement {
        async fn probe(&self) -> overdrive_core::traits::mtls_enforcement::Result<()> {
            Ok(())
        }

        async fn enforce(
            &self,
            conn: InterceptedConnection,
        ) -> overdrive_core::traits::mtls_enforcement::Result<EnforcedConnection> {
            struct Exit(Arc<AtomicBool>);
            impl Drop for Exit {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::SeqCst);
                }
            }
            let _exit = Exit(Arc::clone(&self.exited));
            // Announce that enforce is in flight, then block on the release gate
            // — this models the seconds-wide TLS-handshake + kTLS-arm window the
            // production race opens between spawn_enforce and stop_alloc.
            self.entered.notify_one();
            self.release.notified().await;
            // `conn.leg` drops here (the spy does not pump) — closing the leg.
            let counter = self.counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(EnforcedConnection::new(EnforcedConnectionId::new(conn.alloc, counter)))
        }

        fn liveness(&self, _handle: &EnforcedConnection) -> PumpLiveness {
            PumpLiveness::Running
        }

        async fn teardown(
            &self,
            handle: EnforcedConnection,
        ) -> overdrive_core::traits::mtls_enforcement::Result<()> {
            self.torn_down.lock().push(handle.id().clone());
            Ok(())
        }
    }

    /// Outcome anchor: DISCUSS Elevator Pitch
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn enforcement_returning_after_retirement_tears_down_the_real_returned_handle_before_drain()
     {
        let enforcement = GatedEnforcement::new();
        let (leg, orig_dst, _client) = accepted_leg_f();
        let worker = Arc::new(MtlsInterceptWorker::new(
            Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
            resolve_scripting(
                orig_dst,
                MtlsResolution::Mesh(ResolvedBackend { addr: orig_dst, expected_svid: None }),
            ),
            Arc::new(SimClock::new()),
            Arc::new(TestSharedIntercept::new()),
        ));
        let allocation = alloc("capability-enforcement-retirement");
        worker.start_shared_owner().await.expect("publish the shared listener owner");
        worker
            .start_alloc(&shared_spec(allocation.clone(), Ipv4Addr::LOCALHOST))
            .await
            .expect("publish one real shared allocation capability");
        tokio::spawn({
            let worker = Arc::clone(&worker);
            async move { worker.handle_shared_outbound(Ipv4Addr::LOCALHOST, leg, orig_dst).await }
        })
        .await
        .expect("production shared outbound dispatch returns");
        tokio::time::timeout(Duration::from_secs(2), enforcement.entered())
            .await
            .expect("production enforcement holds the immutable capability claim");
        let waiter = tokio::spawn({
            let worker = Arc::clone(&worker);
            let allocation = allocation.clone();
            async move { worker.stop_alloc(&allocation).await }
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished(), "allocation stop cannot pass the in-flight claim");
        enforcement.release();
        waiter
            .await
            .expect("allocation-stop owner joins")
            .expect("late-handle teardown completes outside the registry lock");
        assert_eq!(enforcement.torn_down().len(), 1);
        assert_eq!(enforcement.torn_down()[0].alloc(), &allocation);
        assert!(
            worker.capabilities.claim_source(Ipv4Addr::LOCALHOST).is_none(),
            "retirement publishes no late handle or claimable predecessor identity"
        );
        worker.shutdown_owner().await.expect("shared listener owner joins");
    }

    struct RetryingSharedEnforcement {
        enforced: tokio::sync::Notify,
        counter: std::sync::atomic::AtomicU64,
        teardown_calls: AtomicUsize,
        teardown_ids: Mutex<Vec<EnforcedConnectionId>>,
        /// Teardown calls still to fail, counted down from the first call.
        failures_left: AtomicUsize,
    }

    impl RetryingSharedEnforcement {
        /// The first teardown call fails; every later one succeeds.
        fn new() -> Arc<Self> {
            Self::failing_first(1)
        }

        /// The first `failures` teardown calls fail, each with its handle's
        /// typed `TeardownFailed`; every later one succeeds.
        fn failing_first(failures: usize) -> Arc<Self> {
            Arc::new(Self {
                enforced: tokio::sync::Notify::new(),
                counter: std::sync::atomic::AtomicU64::new(0),
                teardown_calls: AtomicUsize::new(0),
                teardown_ids: Mutex::new(Vec::new()),
                failures_left: AtomicUsize::new(failures),
            })
        }

        async fn wait_enforced(&self) {
            self.enforced.notified().await;
        }
    }

    #[async_trait]
    impl MtlsEnforcement for RetryingSharedEnforcement {
        async fn probe(&self) -> overdrive_core::traits::mtls_enforcement::Result<()> {
            Ok(())
        }

        async fn enforce(
            &self,
            connection: InterceptedConnection,
        ) -> overdrive_core::traits::mtls_enforcement::Result<EnforcedConnection> {
            let sequence = self.counter.fetch_add(1, Ordering::SeqCst);
            let handle =
                EnforcedConnection::new(EnforcedConnectionId::new(connection.alloc, sequence));
            self.enforced.notify_one();
            Ok(handle)
        }

        fn liveness(&self, _handle: &EnforcedConnection) -> PumpLiveness {
            PumpLiveness::Running
        }

        async fn teardown(
            &self,
            handle: EnforcedConnection,
        ) -> overdrive_core::traits::mtls_enforcement::Result<()> {
            self.teardown_calls.fetch_add(1, Ordering::SeqCst);
            self.teardown_ids.lock().push(handle.id().clone());
            let fails = self
                .failures_left
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1))
                .is_ok();
            if fails {
                return Err(
                    overdrive_core::traits::mtls_enforcement::MtlsEnforcementError::TeardownFailed {
                        id: handle.id().clone(),
                        source: std::io::Error::other("injected shared teardown failure"),
                    },
                );
            }
            Ok(())
        }
    }

    /// Outcome anchor: DISCUSS Elevator Pitch
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[allow(
        clippy::too_many_lines,
        reason = "one bounded-change narrative retains first-source, identical-handle, reservation, retry, completion, and successor evidence"
    )]
    async fn shared_teardown_failure_retains_the_exact_handle_drain_and_reservation_until_same_owner_retry()
     {
        let enforcement = RetryingSharedEnforcement::new();
        let (leg, orig_dst, _client) = accepted_leg_f();
        let worker = Arc::new(MtlsInterceptWorker::new(
            Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
            resolve_scripting(
                orig_dst,
                MtlsResolution::Mesh(ResolvedBackend { addr: orig_dst, expected_svid: None }),
            ),
            Arc::new(SimClock::new()),
            Arc::new(TestSharedIntercept::new()),
        ));
        let allocation = alloc("shared-teardown-retry");
        let address = Ipv4Addr::LOCALHOST;
        worker.start_shared_owner().await.expect("publish shared owner");
        worker
            .start_alloc(&shared_spec(allocation.clone(), address))
            .await
            .expect("publish shared allocation");
        tokio::spawn({
            let worker = Arc::clone(&worker);
            async move { worker.handle_shared_outbound(address, leg, orig_dst).await }
        })
        .await
        .expect("shared dispatch returns");
        tokio::time::timeout(Duration::from_secs(2), enforcement.wait_enforced())
            .await
            .expect("one shared handle reaches production publication");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let published = worker
                    .capabilities
                    .inner
                    .state
                    .lock()
                    .records
                    .values()
                    .any(|record| record.handles.len() == 1);
                if published {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the returned handle is owned before stop");

        let first = worker
            .stop_alloc(&allocation)
            .await
            .expect_err("the exact first teardown source is surfaced");
        let super::MtlsInterceptStopError::HandleTeardown { alloc_id, failures } = &first else {
            panic!("expected a handle-teardown failure, got {first:?}");
        };
        assert_eq!(*alloc_id, allocation);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].source.to_string().contains("injected shared teardown failure"));
        let expected_handle = EnforcedConnectionId::new(allocation.clone(), 0);
        let retained_retry_ids =
            worker.stopping.lock().get(&allocation).and_then(|stops| stops.last()).map_or_else(
                Vec::new,
                |stop| {
                    stop.retry_handles
                        .lock()
                        .iter()
                        .map(|handle| handle.id().clone())
                        .collect::<Vec<_>>()
                },
            );

        let reservation_retained = matches!(
            worker.capabilities.begin_registration(
                alloc("shared-teardown-contender"),
                address,
                SpiffeId::new("spiffe://overdrive.local/workload/shared-teardown/alloc/contender",)
                    .expect("SPIFFE ID"),
                std::iter::once(NonZeroU16::new(8443).expect("non-zero port")).collect(),
            ),
            Err(super::MtlsInterceptInstallError::RegistrationConflict { .. })
        );

        let retry = worker.stop_alloc(&allocation).await;
        let retry_count = enforcement.teardown_calls.load(Ordering::SeqCst);
        let successor = worker.capabilities.begin_registration(
            alloc("shared-teardown-successor"),
            address,
            SpiffeId::new("spiffe://overdrive.local/workload/shared-teardown/alloc/successor")
                .expect("SPIFFE ID"),
            std::iter::once(NonZeroU16::new(8443).expect("non-zero port")).collect(),
        );
        let successor_accepted = successor.is_ok();
        drop(successor);
        let shutdown = worker.shutdown_owner().await;

        assert!(reservation_retained, "the Retiring reservation survives the failed teardown");
        assert_eq!(
            retained_retry_ids.as_slice(),
            std::slice::from_ref(&expected_handle),
            "the exact failed opaque handle identity remains retry-owned"
        );
        retry.expect("same-owner retry tears down the retained handle and completes the drain");
        assert_eq!(retry_count, 2, "the identical handle is retried exactly once");
        assert_eq!(
            enforcement.teardown_ids.lock().as_slice(),
            [expected_handle.clone(), expected_handle],
            "the same stable handle identity reaches the first teardown and its retry"
        );
        assert!(successor_accepted, "address reuse opens only after retry completes the drain");
        shutdown.expect("shared listener owner joins after the successful retry");
        assert!(
            !worker.capabilities.inner.state.lock().allocations.contains_key(&allocation),
            "completion alone releases the exact allocation reservation"
        );
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-54 — Protection removal is convergent and its failures are typed.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A shared allocation's failed enforced-connection teardowns are surfaced
    /// as `HandleTeardown` with one typed failure per connection, in teardown
    /// order, and a retried stop converges on the retained handles (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the worker's `MtlsInterceptStopError` and its `Arc`-shared typed sources),
    /// D-295-R10). Two connections fail, so the stop error's `Display` — the
    /// one rendering every string consumer carries, including the persisted
    /// Failed-row `detail` — must name both per-connection causes in `failures`
    /// order (the same section, "`Display`, the text every string consumer
    /// carries", the user's decision DR-06 of 2026-09-29): `allocation <id>:
    /// enforced-handle teardown failed for 2 handle(s): <c1>: <e1>; <c2>: <e2>`,
    /// with `<ci>` the `EnforcedConnectionId` `Display` (`<alloc>#<counter>`)
    /// and `<ei>` the `Display` of its typed source. RETARGETED onto a shared
    /// allocation: the per-allocation record the original registered is deleted
    /// with B-7's step. The connections are delivered through the shared leg-F
    /// listener (`TestSharedIntercept::script_accept`), one after the other, so
    /// the teardown order is the publication order.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn allocation_stop_surfaces_teardown_failure_and_retry_converges() {
        let enforcement = RetryingSharedEnforcement::failing_first(2);
        let (first_leg, orig_dst, _first_client) = accepted_leg_f();
        let (second_leg, _second_leg_address, _second_client) = accepted_leg_f();
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = Arc::new(MtlsInterceptWorker::new(
            Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
            resolve_scripting(
                orig_dst,
                MtlsResolution::Mesh(ResolvedBackend { addr: orig_dst, expected_svid: None }),
            ),
            Arc::new(SimClock::new()),
            Arc::clone(&intercept) as Arc<dyn MtlsIntercept>,
        ));
        let allocation = alloc("alloc-stop-retry");
        worker.start_shared_owner().await.expect("publish the shared owner");
        worker
            .start_alloc(&shared_spec(allocation.clone(), Ipv4Addr::LOCALHOST))
            .await
            .expect("publish one shared allocation");
        let (leg_f, _leg_c) = shared_legs(&intercept, &worker, &allocation);
        // One connection at a time: each is published before the next is
        // delivered, so the capability's handle order is #0 then #1.
        for (published, leg) in [(1, first_leg), (2, second_leg)] {
            deliver_leg_f_connection(&intercept, leg_f, Ipv4Addr::LOCALHOST, orig_dst, leg);
            within_2s("the delivered connection reaches enforcement", || {
                enforcement.counter.load(Ordering::SeqCst) == published
            })
            .await;
            wait_published(&worker, &allocation, usize::try_from(published).expect("tiny count"))
                .await;
        }

        let first =
            worker.stop_alloc(&allocation).await.expect_err("the teardown failures surface");
        let super::MtlsInterceptStopError::HandleTeardown { alloc_id, failures } = &first else {
            panic!("expected a typed per-connection teardown failure, got {first:?}");
        };
        let expected_connections = [
            EnforcedConnectionId::new(allocation.clone(), 0),
            EnforcedConnectionId::new(allocation.clone(), 1),
        ];
        assert_eq!(alloc_id, &allocation);
        assert_eq!(failures.len(), 2, "one failure per failed connection");
        for (failure, expected_connection) in failures.iter().zip(&expected_connections) {
            assert_eq!(&failure.connection, expected_connection, "failures in teardown order");
            assert!(
                matches!(
                    &*failure.source,
                    MtlsEnforcementError::TeardownFailed { id, source }
                        if id == expected_connection
                            && source.to_string() == "injected shared teardown failure"
                ),
                "the failure keeps the exact typed teardown cause: {:?}",
                failure.source
            );
        }
        // DR-06 (user decision 2026-09-29): the rendered text names every
        // per-connection cause, in `failures` order, joined by "; ". Written out
        // in full, so a rendering that drops, reorders, or re-renders a cause
        // fails here rather than in an operator's Failed-row detail.
        assert_eq!(
            first.to_string(),
            "allocation alloc-stop-retry: enforced-handle teardown failed for 2 handle(s): \
             alloc-stop-retry#0: teardown of connection alloc-stop-retry#0 failed: injected \
             shared teardown failure; alloc-stop-retry#1: teardown of connection \
             alloc-stop-retry#1 failed: injected shared teardown failure",
            "the stop error's Display renders each failed connection and its own cause"
        );

        worker.stop_alloc(&allocation).await.expect("the retried stop converges");
        assert_eq!(
            enforcement.teardown_calls.load(Ordering::SeqCst),
            4,
            "each retained handle is torn down once more, and only once"
        );
        assert_eq!(
            *enforcement.teardown_ids.lock(),
            [
                expected_connections[0].clone(),
                expected_connections[1].clone(),
                expected_connections[0].clone(),
                expected_connections[1].clone(),
            ],
            "the retry tears down exactly the two retained handles, in order"
        );
        worker.shutdown_owner().await.expect("the shared owner joins");
    }

    /// CONTRACT_SHAPE: bounded-change (a late install creates no listener, rule, or child).
    #[tokio::test]
    async fn allocation_start_after_owner_shutdown_is_rejected_before_install() {
        let worker = Arc::new(MtlsInterceptWorker::new(
            GatedEnforcement::new(),
            resolve_scripting(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0), MtlsResolution::NonMesh),
            Arc::new(SimClock::new()),
            Arc::new(crate::mtls_intercept_port::HostMtlsIntercept::new()),
        ));
        worker.shutdown_owner().await.expect("worker owner shutdown converges");
        let alloc = alloc("alloc-late-after-owner-shutdown");

        assert!(matches!(
            worker.start_alloc(&minimal_spec(alloc.clone())).await,
            Err(super::MtlsInterceptInstallError::OwnerShutdown)
        ));
        assert_eq!(worker.leg_c_addr(&alloc), None);
    }

    // ---- GH #295 shared-allocation bodies (S-ND295-20/54/61/70) -------------

    /// Wait (bounded at 2 s real time, no clock advanced) until `cond` holds.
    async fn within_2s(label: &str, mut cond: impl FnMut() -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while !cond() {
            assert!(tokio::time::Instant::now() < deadline, "not observed within 2 s: {label}");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    /// A worker over `TestSharedIntercept`, a teardown-succeeding enforcement,
    /// and a `NonMesh` resolve.
    fn shared_worker(intercept: &Arc<TestSharedIntercept>) -> Arc<MtlsInterceptWorker> {
        let (enforcement, _calls) = SpyEnforcement::new();
        Arc::new(MtlsInterceptWorker::new(
            enforcement,
            resolve_scripting(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0), MtlsResolution::NonMesh),
            Arc::new(SimClock::new()),
            Arc::clone(intercept) as Arc<dyn MtlsIntercept>,
        ))
    }

    /// The node's two shared listeners, read through the port: leg C is the
    /// address a shared allocation records (`leg_c_addr`), leg F the other
    /// live listener the intercept owns.
    fn shared_legs(
        intercept: &TestSharedIntercept,
        worker: &MtlsInterceptWorker,
        allocation: &AllocationId,
    ) -> (SocketAddrV4, SocketAddrV4) {
        let leg_c =
            worker.leg_c_addr(allocation).expect("a shared allocation records the shared leg C");
        let live = intercept.live_listeners();
        assert_eq!(
            live.len(),
            2,
            "the port owns exactly the two shared listeners (none before B-7): {live:?}"
        );
        assert!(live.contains(&leg_c), "leg C {leg_c} is a port-owned listener: {live:?}");
        let leg_f = live
            .into_iter()
            .find(|address| *address != leg_c)
            .unwrap_or_else(|| unreachable!("two distinct live listeners, one of them leg C"));
        (leg_f, leg_c)
    }

    /// Deliver one connection from the guest source `source` that dialled
    /// `orig_dst` through the shared leg-F listener at `leg_f`.
    fn deliver_leg_f_connection(
        intercept: &TestSharedIntercept,
        leg_f: SocketAddrV4,
        source: Ipv4Addr,
        orig_dst: SocketAddrV4,
        stream: std::os::fd::OwnedFd,
    ) {
        assert!(
            intercept.script_accept(
                leg_f,
                TestAcceptScript::Connection(InterceptAccepted {
                    stream,
                    peer: SocketAddrV4::new(source, 40_000),
                    local: orig_dst,
                }),
            ),
            "the shared leg-F listener at {leg_f} is live and port-owned"
        );
    }

    /// Wait until `handles` enforced handles are published under `allocation`'s
    /// capability, so a stop observes them in its drain.
    async fn wait_published(
        worker: &MtlsInterceptWorker,
        allocation: &AllocationId,
        handles: usize,
    ) {
        within_2s("the enforced handle is published under the capability", || {
            worker
                .capabilities
                .inner
                .state
                .lock()
                .records
                .iter()
                .any(|(key, record)| &key.alloc == allocation && record.handles.len() == handles)
        })
        .await;
    }

    /// The members one shared allocation at `address` owns: its managed-guest
    /// and outbound-source member and one inbound destination per port.
    fn allocation_members(address: Ipv4Addr, ports: &[u16]) -> InterceptMembers {
        InterceptMembers {
            managed_guest_ips: BTreeSet::from([address]),
            outbound_sources: BTreeSet::from([address]),
            inbound_destinations: ports
                .iter()
                .map(|port| SocketAddrV4::new(address, *port))
                .collect(),
        }
    }

    /// The fault a scripted element removal returns.
    fn scripted_element_removal_cause() -> InterceptError {
        InterceptError::NftElementUpdateFailed {
            set: InterceptSet::ManagedGuestIps,
            operation: InterceptElementOperation::Delete,
            key: InterceptElementKey::Address(Ipv4Addr::new(100, 95, 0, 2)),
            source: crate::mtls_intercept::NetlinkError::nft(
                "scripted-element-remove",
                std::io::Error::from_raw_os_error(libc::EBUSY),
            ),
        }
    }

    fn is_scripted_element_removal_cause(error: &InterceptError) -> bool {
        matches!(
            error,
            InterceptError::NftElementUpdateFailed {
                operation: InterceptElementOperation::Delete,
                source: crate::mtls_intercept::NetlinkError::Nft { op, source },
                ..
            } if *op == "scripted-element-remove" && source.raw_os_error() == Some(libc::EBUSY)
        )
    }

    /// Drive `future` for up to 50 ms; it must still be pending. The first
    /// poll runs the call's synchronous admission, so a caller polled here has
    /// provably asked before the body continues.
    async fn assert_still_pending<F: std::future::Future + Unpin>(label: &str, future: &mut F) {
        assert!(
            tokio::time::timeout(Duration::from_millis(50), future).await.is_err(),
            "{label}: must still be waiting"
        );
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-54 — Protection removal is convergent and its failures are typed.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Every handle teardown succeeds and the element removal fails: the stop
    /// returns `ElementRemoval` with the typed cause, keeps the Retiring record
    /// (the address stays reserved) and the element guards (every member stays
    /// installed), and a retried stop runs exactly one new removal and releases
    /// the address (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the `remove_allocation_elements` contract, and why the typed sources are shared through `Arc`)).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn element_removal_failure_keeps_the_retiring_record_until_a_retry_converges() {
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = shared_worker(&intercept);
        let address = Ipv4Addr::new(100, 95, 0, 2);
        let allocation = alloc("element-removal-retry");
        worker.start_shared_owner().await.expect("publish the shared owner");
        worker
            .start_alloc(&shared_spec(allocation.clone(), address))
            .await
            .expect("publish one shared allocation");
        let installed = intercept.members();
        assert_eq!(installed, allocation_members(address, &[8443]), "precondition: installed");
        intercept.script_removal_failures(1, scripted_element_removal_cause);

        let failure =
            worker.stop_alloc(&allocation).await.expect_err("the removal failure surfaces");
        let super::MtlsInterceptStopError::ElementRemoval { alloc_id, source } = &failure else {
            panic!("expected ElementRemoval, got {failure:?}");
        };
        assert_eq!(alloc_id, &allocation);
        assert!(
            is_scripted_element_removal_cause(source),
            "the typed removal cause is reachable through the field: {source:?}"
        );
        assert_eq!(intercept.removal_calls(), 1, "the attempt ran one element removal");
        assert_eq!(
            intercept.members(),
            installed,
            "a failed removal keeps every member and element guard in place"
        );
        let contender =
            worker.start_alloc(&shared_spec(alloc("element-removal-contender"), address)).await;
        assert!(
            matches!(
                contender,
                Err(super::MtlsInterceptInstallError::RegistrationConflict { address: reserved })
                    if reserved == address
            ),
            "the Retiring record keeps the address reserved: {contender:?}"
        );

        worker.stop_alloc(&allocation).await.expect("the retried stop converges");
        assert_eq!(intercept.removal_calls(), 2, "the retry runs exactly one new removal");
        assert_eq!(intercept.members(), InterceptMembers::default(), "every member is removed");
        worker
            .start_alloc(&shared_spec(alloc("element-removal-successor"), address))
            .await
            .expect("the address is reusable once the retry converges");
        worker.shutdown_owner().await.expect("the shared owner joins");
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-54 — Protection removal is convergent and its failures are typed.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// B-6 caller rule 1 (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (what each caller receives, rule 1)): a second `stop_alloc(a)` issued while
    /// the first attempt's element removal is held joins that attempt. When the
    /// removal fails, both callers receive equal errors — same variant, same
    /// allocation — whose sources are `Arc::ptr_eq`, and the attempt called
    /// `remove_allocation_elements` once.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn callers_joined_on_one_failed_stop_receive_equal_failures_with_shared_sources() {
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = shared_worker(&intercept);
        let allocation = alloc("joined-stop-callers");
        worker.start_shared_owner().await.expect("publish the shared owner");
        worker
            .start_alloc(&shared_spec(allocation.clone(), Ipv4Addr::new(100, 95, 0, 2)))
            .await
            .expect("publish one shared allocation");
        intercept.script_removal_failures(1, scripted_element_removal_cause);
        intercept.hold_removals(true);

        let first = tokio::spawn({
            let worker = Arc::clone(&worker);
            let allocation = allocation.clone();
            async move { worker.stop_alloc(&allocation).await }
        });
        within_2s("the first caller's attempt enters element removal", || {
            intercept.removal_calls() == 1
        })
        .await;
        let mut second = Box::pin(worker.stop_alloc(&allocation));
        assert_still_pending("a caller joined on the held attempt", &mut second).await;
        assert_eq!(intercept.removal_calls(), 1, "the joined caller begins no attempt of its own");
        intercept.hold_removals(false);

        let first = tokio::time::timeout(Duration::from_secs(2), first)
            .await
            .expect("the attempt ends within 2 s")
            .expect("the first caller's task joins")
            .expect_err("the held removal fails");
        let second = tokio::time::timeout(Duration::from_secs(2), second)
            .await
            .expect("the joined caller returns within 2 s")
            .expect_err("the joined caller receives the same failure");
        let (
            super::MtlsInterceptStopError::ElementRemoval { alloc_id: first_alloc, source: first },
            super::MtlsInterceptStopError::ElementRemoval {
                alloc_id: second_alloc,
                source: second,
            },
        ) = (&first, &second)
        else {
            panic!("both callers must receive ElementRemoval, got {first:?} and {second:?}");
        };
        assert_eq!(first_alloc, &allocation);
        assert_eq!(second_alloc, &allocation);
        assert!(Arc::ptr_eq(first, second), "joined callers share one source Arc");
        assert!(is_scripted_element_removal_cause(first), "the typed cause survives: {first:?}");
        assert_eq!(intercept.removal_calls(), 1, "one attempt, one element removal");

        worker.stop_alloc(&allocation).await.expect("a later retry converges");
        worker.shutdown_owner().await.expect("the shared owner joins");
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-54 — Protection removal is convergent and its failures are typed.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// B-6 caller rule 3 (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (what each caller receives, rule 3)): after an attempt ends `Err`, two
    /// simultaneous `stop_alloc(a)` calls begin exactly one new attempt between
    /// them (`removal_calls()` rises by one, not two), and both receive that
    /// attempt's result — never the error it supersedes.
    ///
    /// "Simultaneous" is a real race: each round spawns the two callers on the
    /// multi-thread runtime behind one `Barrier`, so both pass the barrier
    /// together and contend for the retry claim from different worker threads.
    /// A claim that checks for an attempt under one lock acquisition and
    /// records it under another lets both callers through and begins two
    /// attempts, which the per-round count catches. The race repeats for
    /// `RULE_3_ROUNDS` rounds. Every round but the last fails again with a
    /// fresh scripted cause, so each round's two callers must share that
    /// round's source `Arc` and neither may hold the one it supersedes. The last
    /// round's attempt converges, and both callers receive `Ok`. The removal
    /// hold keeps each round's one attempt in flight until both callers have
    /// asked; a caller arriving after the attempt ended would be the first stop
    /// after a failure and would rightly begin its own.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[allow(
        clippy::too_many_lines,
        reason = "one repeated-race narrative keeps each round's claim count, both callers' \
                  results, and the superseded source together"
    )]
    async fn the_first_stop_after_a_failure_starts_one_retry_for_simultaneous_callers() {
        /// Races of the two simultaneous callers, each after a failed attempt.
        const RULE_3_ROUNDS: usize = 16;

        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = shared_worker(&intercept);
        let address = Ipv4Addr::new(100, 95, 0, 2);
        let allocation = alloc("simultaneous-retry-callers");
        worker.start_shared_owner().await.expect("publish the shared owner");
        worker
            .start_alloc(&shared_spec(allocation.clone(), address))
            .await
            .expect("publish one shared allocation");
        intercept.script_removal_failures(1, scripted_element_removal_cause);
        let first_failure =
            worker.stop_alloc(&allocation).await.expect_err("the first attempt fails");
        let super::MtlsInterceptStopError::ElementRemoval { source: first_source, .. } =
            &first_failure
        else {
            panic!("the superseded attempt failed at element removal: {first_failure:?}");
        };
        let mut superseded = Arc::clone(first_source);
        assert_eq!(intercept.removal_calls(), 1);

        for round in 0..RULE_3_ROUNDS {
            let converges = round + 1 == RULE_3_ROUNDS;
            if !converges {
                intercept.script_removal_failures(1, scripted_element_removal_cause);
            }
            let calls_before = intercept.removal_calls();
            intercept.hold_removals(true);
            let barrier = Arc::new(tokio::sync::Barrier::new(2));
            let [left, right] = [(); 2].map(|()| {
                let worker = Arc::clone(&worker);
                let allocation = allocation.clone();
                let barrier = Arc::clone(&barrier);
                tokio::spawn(async move {
                    barrier.wait().await;
                    worker.stop_alloc(&allocation).await
                })
            });
            within_2s(&format!("round {round}: the new attempt enters element removal"), || {
                intercept.removal_calls() > calls_before
            })
            .await;
            // Both callers are past the barrier and inside `stop_alloc`; give a
            // second claim every chance to begin before the attempt is released.
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(
                !left.is_finished() && !right.is_finished(),
                "round {round}: both callers wait on the held attempt"
            );
            assert_eq!(
                intercept.removal_calls(),
                calls_before + 1,
                "round {round}: two simultaneous later callers begin one attempt between them"
            );
            intercept.hold_removals(false);

            let left = tokio::time::timeout(Duration::from_secs(2), left)
                .await
                .unwrap_or_else(|_| panic!("round {round}: the left caller returns within 2 s"))
                .unwrap_or_else(|error| panic!("round {round}: the left caller joins: {error}"));
            let right = tokio::time::timeout(Duration::from_secs(2), right)
                .await
                .unwrap_or_else(|_| panic!("round {round}: the right caller returns within 2 s"))
                .unwrap_or_else(|error| panic!("round {round}: the right caller joins: {error}"));
            assert_eq!(
                intercept.removal_calls(),
                calls_before + 1,
                "round {round}: exactly one attempt ran for both callers"
            );
            if converges {
                left.unwrap_or_else(|error| {
                    panic!("round {round}: the left caller receives the converged retry: {error:?}")
                });
                right.unwrap_or_else(|error| {
                    panic!(
                        "round {round}: the right caller receives the converged retry: {error:?}"
                    )
                });
                continue;
            }
            let (
                Err(super::MtlsInterceptStopError::ElementRemoval {
                    alloc_id: left_alloc,
                    source: left_source,
                }),
                Err(super::MtlsInterceptStopError::ElementRemoval {
                    alloc_id: right_alloc,
                    source: right_source,
                }),
            ) = (&left, &right)
            else {
                panic!(
                    "round {round}: both callers receive the new attempt's ElementRemoval, got \
                     {left:?} and {right:?}"
                );
            };
            assert_eq!(left_alloc, &allocation, "round {round}");
            assert_eq!(right_alloc, &allocation, "round {round}");
            assert!(
                Arc::ptr_eq(left_source, right_source),
                "round {round}: both callers receive the one new attempt's shared source"
            );
            assert!(
                !Arc::ptr_eq(left_source, &superseded),
                "round {round}: neither caller receives the error the new attempt superseded"
            );
            assert!(
                is_scripted_element_removal_cause(left_source),
                "round {round}: the typed cause survives: {left_source:?}"
            );
            superseded = Arc::clone(left_source);
        }

        assert_eq!(
            intercept.removal_calls(),
            1 + RULE_3_ROUNDS,
            "one attempt for the first stop and one per round"
        );
        assert_eq!(
            intercept.members(),
            InterceptMembers::default(),
            "the converged retry removed every member"
        );
        worker.shutdown_owner().await.expect("the shared owner joins");
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-54 — Protection removal is convergent and its failures are typed.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// B-6 caller rules 4 and 5 (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (what each caller receives, rules 4 and 5)): with `a`'s failed attempt
    /// retained, `b` active, and removals held so the owner shutdown's teardown
    /// of `b` is in flight, `stop_alloc(a)` runs no removal or teardown of its
    /// own and returns, after the owner shutdown, an error equal to the
    /// shutdown result's entry for `a` (pointer-equal source); `stop_alloc(b)`
    /// returns `Ok(())` because the owner shutdown's teardown of `b` succeeded.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[allow(
        clippy::too_many_lines,
        reason = "one owner-shutdown narrative keeps the retained failure, the in-flight \
                  teardown, both late callers, and the stored result together"
    )]
    async fn a_stop_after_owner_shutdown_began_starts_nothing_and_returns_its_shutdown_entry() {
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = shared_worker(&intercept);
        let first = alloc("shutdown-entry-a");
        let second = alloc("shutdown-entry-b");
        worker.start_shared_owner().await.expect("publish the shared owner");
        worker
            .start_alloc(&shared_spec(first.clone(), Ipv4Addr::new(100, 95, 0, 2)))
            .await
            .expect("publish allocation a");
        worker
            .start_alloc(&shared_spec(second.clone(), Ipv4Addr::new(100, 95, 0, 3)))
            .await
            .expect("publish allocation b");
        intercept.script_removal_failures(1, scripted_element_removal_cause);
        let retained = worker.stop_alloc(&first).await.expect_err("a's attempt fails");
        let super::MtlsInterceptStopError::ElementRemoval { source: retained_source, .. } =
            &retained
        else {
            panic!("expected a's ElementRemoval, got {retained:?}");
        };
        assert_eq!(intercept.removal_calls(), 1);

        intercept.hold_removals(true);
        let shutdown = tokio::spawn({
            let worker = Arc::clone(&worker);
            async move { worker.shutdown_owner().await }
        });
        within_2s("the owner shutdown's teardown of b is in flight", || {
            intercept.removal_calls() == 2
        })
        .await;
        let mut stop_first = Box::pin(worker.stop_alloc(&first));
        let mut stop_second = Box::pin(worker.stop_alloc(&second));
        assert_still_pending("stop_alloc(a) after owner shutdown began", &mut stop_first).await;
        assert_still_pending("stop_alloc(b) after owner shutdown began", &mut stop_second).await;
        assert_eq!(
            intercept.removal_calls(),
            2,
            "a stop after owner shutdown began runs no removal of its own"
        );
        intercept.hold_removals(false);

        let shutdown = tokio::time::timeout(Duration::from_secs(2), shutdown)
            .await
            .expect("the owner shutdown ends within 2 s")
            .expect("the owner shutdown task joins")
            .expect_err("the owner shutdown stores a's retained failure");
        let entries = shutdown
            .failures
            .iter()
            .filter_map(|failure| match failure {
                super::MtlsInterceptStopError::ElementRemoval { alloc_id, source } => {
                    Some((alloc_id.clone(), Arc::clone(source)))
                }
                super::MtlsInterceptStopError::HandleTeardown { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(shutdown.failures.len(), 1, "at most one entry per allocation: {shutdown:?}");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, first, "the only entry is a's");
        assert!(
            Arc::ptr_eq(&entries[0].1, retained_source),
            "the entry is a clone of a's last attempt result"
        );

        let stop_first = tokio::time::timeout(Duration::from_secs(2), stop_first)
            .await
            .expect("stop_alloc(a) returns after the owner shutdown")
            .expect_err("stop_alloc(a) returns a's shutdown entry");
        let super::MtlsInterceptStopError::ElementRemoval { alloc_id, source } = &stop_first else {
            panic!("expected a's shutdown entry, got {stop_first:?}");
        };
        assert_eq!(alloc_id, &first);
        assert!(Arc::ptr_eq(source, &entries[0].1), "pointer-equal to the shutdown entry");
        tokio::time::timeout(Duration::from_secs(2), stop_second)
            .await
            .expect("stop_alloc(b) returns after the owner shutdown")
            .expect("b's teardown by the owner shutdown succeeded");
        assert_eq!(intercept.removal_calls(), 2, "owner shutdown began no retry for a");
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-61 — Lost members, policy route, or guard are detected within a second and repaired with live workloads.
    /// CONTRACT_SHAPE: pure-function.
    ///
    /// `MtlsSharedOwnerError::component()` is the one SSOT the supervisor
    /// consumes; its table equals FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the component SSOT table) row for row. The exhaustive
    /// `variant_name` match is the closed-set guard: a new variant fails to
    /// compile here until the table names it. The component depends on the
    /// variant alone, never on its source, so the rows also carry each source
    /// the observation checks construct (the same section, "The typed causes of
    /// the observation checks", pinned 2026-09-29): `Intercept` over
    /// `PostconditionMismatch`, `PolicyRouteAbsent`, and
    /// `InterceptMarkGuardAbsent` is `IpRules`; `BootMemberClear` over the boot
    /// clear's own `Err` and over `MembersRemain` is `IpSets`. `component()`
    /// lands at 08-03, so S-ND295-13D's 08-02 boot bodies assert no component
    /// and this table carries `BootMemberClear`'s. The
    /// `InterceptMarkGuardAbsent` row is R18-conditional: DELIVER step 08-01
    /// removes it with the variant if R18 is withdrawn.
    #[test]
    fn every_shared_owner_error_reports_its_one_component() {
        use super::MtlsSharedOwnerError as E;
        use overdrive_core::guest_network::SharedGuestNetworkComponent as Component;

        fn variant_name(error: &E) -> &'static str {
            match error {
                E::NotStarted => "NotStarted",
                E::OwnerShutdown => "OwnerShutdown",
                E::ListenerBind { .. } => "ListenerBind",
                E::ListenerLocalAddr { .. } => "ListenerLocalAddr",
                E::ListenerPostcondition { .. } => "ListenerPostcondition",
                E::Intercept { .. } => "Intercept",
                E::TaskReturned { .. } => "TaskReturned",
                E::TaskFailed { .. } => "TaskFailed",
                E::TaskPanicked { .. } => "TaskPanicked",
                E::TaskCancelled { .. } => "TaskCancelled",
                E::TaskObserverClosed => "TaskObserverClosed",
                E::BootMemberClear { .. } => "BootMemberClear",
                E::MemberMismatch { .. } => "MemberMismatch",
                E::MemberRepair { .. } => "MemberRepair",
            }
        }

        let requested = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0);
        let io = || std::io::Error::from_raw_os_error(libc::EIO);
        let intercept = || InterceptError::TransparentListener { addr: requested, source: io() };
        let leg_component = |leg: InterceptLeg| match leg {
            InterceptLeg::F => Component::LegF,
            InterceptLeg::C => Component::LegC,
        };
        let mut rows = Vec::new();
        for leg in [InterceptLeg::F, InterceptLeg::C] {
            rows.push((
                E::ListenerBind { leg, requested, source: intercept() },
                leg_component(leg),
            ));
            rows.push((E::ListenerLocalAddr { leg, source: io() }, leg_component(leg)));
            rows.push((
                E::ListenerPostcondition { leg, expected: requested, observed: None },
                leg_component(leg),
            ));
            rows.push((E::TaskReturned { leg }, leg_component(leg)));
            rows.push((E::TaskFailed { leg, source: io() }, leg_component(leg)));
            rows.push((E::TaskPanicked { leg }, leg_component(leg)));
            rows.push((E::TaskCancelled { leg }, leg_component(leg)));
        }
        // `Intercept` is `IpRules` whatever its source, including the three
        // sources the worker's observation checks construct (FD § "[REF] Driven
        // port — intercept element release, member convergence, boot clear …"
        // (the typed causes of the observation checks)).
        let program = InterceptPostcondition::ConstantRules {
            table_and_chains: Vec::new(),
            sets: Vec::new(),
            prerouting: Vec::new(),
            output: Vec::new(),
        };
        for source in [
            intercept(),
            InterceptError::PostconditionMismatch { expected: program, observed: None },
            InterceptError::PolicyRouteAbsent,
            InterceptError::InterceptMarkGuardAbsent,
        ] {
            rows.push((E::Intercept { source }, Component::IpRules));
        }
        rows.push((
            E::MemberMismatch {
                expected: InterceptMembers::default(),
                observed: allocation_members(Ipv4Addr::new(100, 95, 0, 2), &[8443]),
            },
            Component::IpSets,
        ));
        rows.push((E::MemberRepair { source: intercept() }, Component::IpSets));
        // `BootMemberClear` is `IpSets` for both of its sources: the boot clear's
        // own `Err`, and `MembersRemain` when the clear returned `Ok` with members
        // still present (boot steps 6.2 and 6.6).
        rows.push((E::BootMemberClear { source: intercept() }, Component::IpSets));
        rows.push((
            E::BootMemberClear {
                source: InterceptError::NftRuleInstallFailed {
                    op: "converge-shared-members",
                    source: crate::mtls_intercept::NetlinkError::nft(
                        "converge-shared-members",
                        std::io::Error::from_raw_os_error(libc::EBUSY),
                    ),
                },
            },
            Component::IpSets,
        ));
        rows.push((
            E::BootMemberClear {
                source: InterceptError::MembersRemain {
                    observed: allocation_members(Ipv4Addr::new(100, 95, 0, 9), &[8080]),
                },
            },
            Component::IpSets,
        ));
        rows.push((E::NotStarted, Component::Supervisor));
        rows.push((E::OwnerShutdown, Component::Supervisor));
        rows.push((E::TaskObserverClosed, Component::Supervisor));

        let covered = rows.iter().map(|(error, _)| variant_name(error)).collect::<BTreeSet<_>>();
        assert_eq!(covered.len(), 14, "the table names every variant: {covered:?}");
        for (error, expected) in &rows {
            assert_eq!(
                error.component(),
                *expected,
                "{} reports {expected:?}",
                variant_name(error)
            );
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-70 — The node's protection listeners belong to the protection port: a simulated node opens no socket, and a listener stops when its wait is cancelled.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Each leg parks exactly one accept; dropping the last worker reference
    /// cancels both waits and releases both listeners (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (what the worker's use of it guarantees): an accept
    /// task never keeps the worker alive).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_idle_accept_task_ends_when_the_last_worker_reference_drops() {
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = shared_worker(&intercept);
        worker.start_shared_owner().await.expect("publish the shared owner");
        let legs = intercept.live_listeners();
        assert_eq!(legs.len(), 2, "the port owns both shared listeners: {legs:?}");
        within_2s("each leg parks exactly one accept", || {
            legs.iter().all(|leg| intercept.parked_accepts(*leg) == 1)
        })
        .await;

        drop(worker);
        within_2s("both accept waits end and both listeners are released", || {
            intercept.live_listeners().is_empty()
                && legs.iter().all(|leg| intercept.parked_accepts(*leg) == 0)
        })
        .await;
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-70 — The node's protection listeners belong to the protection port: a simulated node opens no socket, and a listener stops when its wait is cancelled.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// When `shutdown_owner` returns `Ok`, both accept tasks have ended, both
    /// listeners are released, and the node guard was relinquished, not dropped
    /// (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (what the worker's use of it guarantees); D15).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn owner_shutdown_ends_both_accept_tasks_releases_both_listeners_and_relinquishes_the_node_guard()
     {
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = shared_worker(&intercept);
        worker.start_shared_owner().await.expect("publish the shared owner");
        let legs = intercept.live_listeners();
        assert_eq!(legs.len(), 2, "the port owns both shared listeners: {legs:?}");
        within_2s("each leg parks exactly one accept", || {
            legs.iter().all(|leg| intercept.parked_accepts(*leg) == 1)
        })
        .await;

        tokio::time::timeout(Duration::from_secs(2), worker.shutdown_owner())
            .await
            .expect("owner shutdown returns within 2 s")
            .expect("owner shutdown succeeds");
        assert!(
            legs.iter().all(|leg| intercept.parked_accepts(*leg) == 0),
            "both accept tasks have ended when shutdown returns"
        );
        assert!(intercept.live_listeners().is_empty(), "both listeners are released");
        assert_eq!(intercept.node_guard_drops(), 0, "the node guard is relinquished, not dropped");
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-70 — The node's protection listeners belong to the protection port: a simulated node opens no socket, and a listener stops when its wait is cancelled.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A lost leg-F listener ends only leg F's task (`TaskFailed { leg: F }`
    /// with the listener's errno) while leg C stays parked; the owner releases
    /// the dead listener before rebinding exactly the recorded leg-F address,
    /// so the rebind is never refused by its own listener, and the audit then
    /// passes (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (`InterceptListener::accept` behaviour, and what the worker's use of it guarantees)).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_lost_listener_ends_only_its_own_task_and_the_owner_rebinds_the_recorded_address() {
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = shared_worker(&intercept);
        let allocation = alloc("lost-listener-rebind");
        worker.start_shared_owner().await.expect("publish the shared owner");
        worker
            .start_alloc(&shared_spec(allocation.clone(), Ipv4Addr::new(100, 95, 0, 2)))
            .await
            .expect("publish one shared allocation");
        let (leg_f, leg_c) = shared_legs(&intercept, &worker, &allocation);
        within_2s("each leg parks exactly one accept", || {
            intercept.parked_accepts(leg_f) == 1 && intercept.parked_accepts(leg_c) == 1
        })
        .await;

        assert!(
            intercept.script_accept(leg_f, TestAcceptScript::ListenerLost { errno: libc::EINVAL })
        );
        let failure =
            tokio::time::timeout(Duration::from_secs(2), worker.wait_shared_owner_failure())
                .await
                .expect("the lost listener is observed within 2 s");
        assert!(
            matches!(
                &failure,
                super::MtlsSharedOwnerError::TaskFailed { leg: InterceptLeg::F, source }
                    if source.raw_os_error() == Some(libc::EINVAL)
            ),
            "leg F's task ends with the listener's own failure: {failure:?}"
        );
        assert_eq!(intercept.parked_accepts(leg_c), 1, "leg C keeps waiting");

        worker
            .converge_shared_owner()
            .await
            .expect("the owner rebinds exactly the recorded leg-F address");
        assert_eq!(
            intercept.live_listeners(),
            BTreeSet::from([leg_f, leg_c]).into_iter().collect::<Vec<_>>(),
            "the rebind took the recorded address, not another port"
        );
        within_2s("the rebound leg F parks a new accept", || intercept.parked_accepts(leg_f) == 1)
            .await;
        worker.audit_shared_owner().await.expect("the repaired owner audits clean");
        worker.shutdown_owner().await.expect("the shared owner joins");
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-70 — The node's protection listeners belong to the protection port: a simulated node opens no socket, and a listener stops when its wait is cancelled.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// An accepted connection whose original destination cannot be read is
    /// connection-scoped: it ends no task and the leg re-parks (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (`InterceptListener::accept` behaviour: `OriginalDestination`)).
    /// A later `ListenerLost` proves the task consumed the first script and
    /// kept waiting.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_connection_whose_destination_cannot_be_read_is_dropped_and_the_task_keeps_waiting() {
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = shared_worker(&intercept);
        let allocation = alloc("unreadable-destination");
        worker.start_shared_owner().await.expect("publish the shared owner");
        worker
            .start_alloc(&shared_spec(allocation.clone(), Ipv4Addr::new(100, 95, 0, 2)))
            .await
            .expect("publish one shared allocation");
        let (leg_f, _leg_c) = shared_legs(&intercept, &worker, &allocation);
        within_2s("leg F parks one accept", || intercept.parked_accepts(leg_f) == 1).await;

        assert!(intercept.script_accept(
            leg_f,
            TestAcceptScript::OriginalDestinationFailure { errno: libc::ENOTCONN }
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(200), worker.wait_shared_owner_failure())
                .await
                .is_err(),
            "a connection-scoped failure ends no accept task"
        );
        within_2s("leg F re-parks", || intercept.parked_accepts(leg_f) == 1).await;
        worker.audit_shared_owner().await.expect("the owner stays healthy");

        assert!(
            intercept.script_accept(leg_f, TestAcceptScript::ListenerLost { errno: libc::EIO })
        );
        let failure =
            tokio::time::timeout(Duration::from_secs(2), worker.wait_shared_owner_failure())
                .await
                .expect("the later loss is observed within 2 s");
        assert!(
            matches!(
                &failure,
                super::MtlsSharedOwnerError::TaskFailed { leg: InterceptLeg::F, source }
                    if source.raw_os_error() == Some(libc::EIO)
            ),
            "the task took the next script, so the first one was consumed: {failure:?}"
        );
        worker.shutdown_owner().await.expect("the shared owner joins");
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-70 — The node's protection listeners belong to the protection port: a simulated node opens no socket, and a listener stops when its wait is cancelled.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A listener whose bound address cannot be read makes the audit return
    /// `ListenerLocalAddr` for that leg with the adapter's cause (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (`InterceptListener::local_addr` behaviour)).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_unreadable_listener_address_is_reported_by_the_audit() {
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = shared_worker(&intercept);
        let allocation = alloc("unreadable-listener-address");
        worker.start_shared_owner().await.expect("publish the shared owner");
        worker
            .start_alloc(&shared_spec(allocation.clone(), Ipv4Addr::new(100, 95, 0, 2)))
            .await
            .expect("publish one shared allocation");
        let (_leg_f, leg_c) = shared_legs(&intercept, &worker, &allocation);

        assert!(intercept.script_local_addr_failure(leg_c, libc::EBADF));
        let error = worker.audit_shared_owner().await.expect_err("the audit reads leg C's address");
        assert!(
            matches!(
                &error,
                super::MtlsSharedOwnerError::ListenerLocalAddr { leg: InterceptLeg::C, source }
                    if source.raw_os_error() == Some(libc::EBADF)
            ),
            "the audit names leg C and the adapter's cause: {error:?}"
        );
        worker.shutdown_owner().await.expect("the shared owner joins");
    }

    /// `MtlsEnforcement` whose `enforce` succeeds at once and whose `teardown`
    /// waits on a release gate and fails the first time when armed — the
    /// `GatedTeardown` of the per-allocation originals, over a real enforce so
    /// a shared allocation can publish a handle.
    struct GatedSharedEnforcement {
        counter: std::sync::atomic::AtomicU64,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
        calls: AtomicUsize,
        fail_first: AtomicBool,
    }

    impl GatedSharedEnforcement {
        fn new(fail_first: bool) -> Arc<Self> {
            Arc::new(Self {
                counter: std::sync::atomic::AtomicU64::new(0),
                entered: tokio::sync::Notify::new(),
                release: tokio::sync::Notify::new(),
                calls: AtomicUsize::new(0),
                fail_first: AtomicBool::new(fail_first),
            })
        }
    }

    #[async_trait]
    impl MtlsEnforcement for GatedSharedEnforcement {
        async fn probe(&self) -> overdrive_core::traits::mtls_enforcement::Result<()> {
            Ok(())
        }

        async fn enforce(
            &self,
            connection: InterceptedConnection,
        ) -> overdrive_core::traits::mtls_enforcement::Result<EnforcedConnection> {
            let sequence = self.counter.fetch_add(1, Ordering::SeqCst);
            Ok(EnforcedConnection::new(EnforcedConnectionId::new(connection.alloc, sequence)))
        }

        fn liveness(&self, _handle: &EnforcedConnection) -> PumpLiveness {
            PumpLiveness::Running
        }

        async fn teardown(
            &self,
            handle: EnforcedConnection,
        ) -> overdrive_core::traits::mtls_enforcement::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_one();
            self.release.notified().await;
            if self.fail_first.swap(false, Ordering::SeqCst) {
                return Err(MtlsEnforcementError::TeardownFailed {
                    id: handle.id().clone(),
                    source: std::io::Error::other("injected teardown failure"),
                });
            }
            Ok(())
        }
    }

    /// A started shared owner over `enforcement` whose resolve enforces the
    /// connection `accepted_leg_f` returns, one shared allocation at loopback,
    /// and its one published handle, delivered through the shared leg-F
    /// listener.
    async fn shared_allocation_with_one_handle(
        enforcement: Arc<dyn MtlsEnforcement>,
        name: &str,
    ) -> (Arc<MtlsInterceptWorker>, Arc<TestSharedIntercept>, AllocationId, TcpStream) {
        let (leg, orig_dst, client) = accepted_leg_f();
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = Arc::new(MtlsInterceptWorker::new(
            enforcement,
            resolve_scripting(
                orig_dst,
                MtlsResolution::Mesh(ResolvedBackend { addr: orig_dst, expected_svid: None }),
            ),
            Arc::new(SimClock::new()),
            Arc::clone(&intercept) as Arc<dyn MtlsIntercept>,
        ));
        let allocation = alloc(name);
        worker.start_shared_owner().await.expect("publish the shared owner");
        worker
            .start_alloc(&shared_spec(allocation.clone(), Ipv4Addr::LOCALHOST))
            .await
            .expect("publish one shared allocation");
        let (leg_f, _leg_c) = shared_legs(&intercept, &worker, &allocation);
        deliver_leg_f_connection(&intercept, leg_f, Ipv4Addr::LOCALHOST, orig_dst, leg);
        wait_published(&worker, &allocation, 1).await;
        (worker, intercept, allocation, client)
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-20 — Node-shared listener and capability lifecycle; exact-port rebind; port theft.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Twin of `replacement_shutdown_waits_for_the_same_authoritative_teardown`
    /// over a shared allocation: cancelled and concurrent owner-shutdown
    /// callers share one sealed completion, and a later caller creates no
    /// second cleanup generation.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shared_replacement_shutdown_waits_for_the_same_authoritative_teardown() {
        let enforcement = GatedSharedEnforcement::new(true);
        let (worker, _intercept, _allocation, _client) = shared_allocation_with_one_handle(
            Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
            "shared-shutdown-fence",
        )
        .await;

        let leader = tokio::spawn({
            let worker = Arc::clone(&worker);
            async move { worker.shutdown_owner().await }
        });
        tokio::time::timeout(Duration::from_secs(1), enforcement.entered.notified())
            .await
            .expect("authoritative teardown starts");
        leader.abort();
        let mut replacement = tokio::spawn({
            let worker = Arc::clone(&worker);
            async move { worker.shutdown_owner().await }
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(25), &mut replacement).await.is_err(),
            "replacement caller cannot return before enforcement teardown"
        );
        enforcement.release.notify_one();
        let first = tokio::time::timeout(Duration::from_secs(1), replacement)
            .await
            .expect("replacement observes full-worker completion")
            .expect("replacement task joins");
        assert!(first.is_err(), "every concurrent caller observes the teardown failure");
        assert_eq!(enforcement.calls.load(Ordering::SeqCst), 1);

        assert!(worker.shutdown_owner().await.is_err());
        assert_eq!(
            enforcement.calls.load(Ordering::SeqCst),
            1,
            "a sealed process owner never retries failed teardown work"
        );
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-20 — Node-shared listener and capability lifecycle; exact-port rebind; port theft.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Twin of `same_owner_reinstall_waits_for_prior_teardown_before_readiness`
    /// over a shared allocation: a same-allocation re-install cannot report
    /// readiness before the prior teardown ends.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shared_same_owner_reinstall_waits_for_prior_teardown_before_readiness() {
        let enforcement = GatedSharedEnforcement::new(false);
        let (worker, _intercept, allocation, _client) = shared_allocation_with_one_handle(
            Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
            "shared-same-owner-reinstall-fence",
        )
        .await;

        let mut replacement = tokio::spawn({
            let worker = Arc::clone(&worker);
            let spec = shared_spec(allocation.clone(), Ipv4Addr::LOCALHOST);
            async move { worker.start_alloc(&spec).await }
        });
        tokio::time::timeout(Duration::from_secs(1), enforcement.entered.notified())
            .await
            .expect("prior teardown reaches its controllable fence");
        assert!(
            tokio::time::timeout(Duration::from_millis(25), &mut replacement).await.is_err(),
            "replacement readiness must remain pending while the prior exact owner is retiring"
        );
        enforcement.release.notify_one();
        tokio::time::timeout(Duration::from_secs(1), replacement)
            .await
            .expect("replacement readiness is bounded after teardown release")
            .expect("replacement task joins")
            .expect("same-owner reinstall succeeds after prior teardown");
        assert_eq!(enforcement.calls.load(Ordering::SeqCst), 1);
        worker.stop_alloc(&allocation).await.expect("replacement owner cleans up");
        worker.shutdown_owner().await.expect("the shared owner joins");
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-20 — Node-shared listener and capability lifecycle; exact-port rebind; port theft.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Twin of `same_owner_reinstall_failure_keeps_readiness_closed_until_retry`
    /// over a shared allocation: a failed prior teardown keeps the replacement
    /// closed and retryable.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shared_same_owner_reinstall_failure_keeps_readiness_closed_until_retry() {
        let enforcement = GatedSharedEnforcement::new(true);
        let (worker, _intercept, allocation, _client) = shared_allocation_with_one_handle(
            Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
            "shared-same-owner-reinstall-retry",
        )
        .await;

        let first = tokio::spawn({
            let worker = Arc::clone(&worker);
            let spec = shared_spec(allocation.clone(), Ipv4Addr::LOCALHOST);
            async move { worker.start_alloc(&spec).await }
        });
        tokio::time::timeout(Duration::from_secs(1), enforcement.entered.notified())
            .await
            .expect("first prior teardown starts");
        enforcement.release.notify_one();
        let first = first.await.expect("first replacement task joins");
        assert!(matches!(first, Err(super::MtlsInterceptInstallError::PriorTeardown { .. })));
        assert_eq!(
            worker.leg_c_addr(&allocation),
            None,
            "failed prior teardown cannot install or report a replacement"
        );

        let retry = tokio::spawn({
            let worker = Arc::clone(&worker);
            let spec = shared_spec(allocation.clone(), Ipv4Addr::LOCALHOST);
            async move { worker.start_alloc(&spec).await }
        });
        tokio::time::timeout(Duration::from_secs(1), enforcement.entered.notified())
            .await
            .expect("retry reaches the retained exact teardown handle");
        enforcement.release.notify_one();
        retry
            .await
            .expect("retry task joins")
            .expect("retry converges before replacement installation");
        assert!(worker.leg_c_addr(&allocation).is_some());
        assert_eq!(enforcement.calls.load(Ordering::SeqCst), 2);
        worker.stop_alloc(&allocation).await.expect("replacement owner cleans up");
        worker.shutdown_owner().await.expect("the shared owner joins");
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-20 — Node-shared listener and capability lifecycle; exact-port rebind; port theft.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Twin of `completed_enforce_handle_is_torn_down_not_orphaned` over a
    /// shared allocation: a completed enforcement's handle is retained under
    /// the capability and torn down exactly once by stop.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shared_completed_enforce_handle_is_torn_down_not_orphaned() {
        let spy = GatedEnforcement::new();
        let (leg, orig_dst, _client) = accepted_leg_f();
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = Arc::new(MtlsInterceptWorker::new(
            Arc::clone(&spy) as Arc<dyn MtlsEnforcement>,
            resolve_scripting(
                orig_dst,
                MtlsResolution::Mesh(ResolvedBackend { addr: orig_dst, expected_svid: None }),
            ),
            Arc::new(SimClock::new()),
            Arc::clone(&intercept) as Arc<dyn MtlsIntercept>,
        ));
        let the_alloc = alloc("shared-orphan-race");
        worker.start_shared_owner().await.expect("publish the shared owner");
        worker
            .start_alloc(&shared_spec(the_alloc.clone(), Ipv4Addr::LOCALHOST))
            .await
            .expect("publish one shared allocation");
        let (leg_f, _leg_c) = shared_legs(&intercept, &worker, &the_alloc);
        deliver_leg_f_connection(&intercept, leg_f, Ipv4Addr::LOCALHOST, orig_dst, leg);

        tokio::time::timeout(Duration::from_secs(5), spy.entered())
            .await
            .expect("enforce must enter (in-flight) within 5s");
        spy.release();
        wait_published(&worker, &the_alloc, 1).await;

        tokio::time::timeout(Duration::from_secs(10), worker.stop_alloc(&the_alloc))
            .await
            .expect("allocation stop is bounded")
            .expect("teardown succeeds");

        let recorded = spy.torn_down();
        assert!(spy.exited.load(Ordering::SeqCst), "the in-flight enforce child is joined");
        assert_eq!(recorded.len(), 1, "the completed handle is torn down exactly once");
        assert_eq!(recorded[0].alloc(), &the_alloc);
        worker.shutdown_owner().await.expect("the shared owner joins");
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-20 — Node-shared listener and capability lifecycle; exact-port rebind; port theft.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Twin of `allocation_stop_joins_an_inflight_enforce_child` over a shared
    /// allocation. The per-allocation original requires stop to abort the
    /// child before it would return; C-295-L instead has retirement wait for
    /// the in-flight claim and tear the late handle down rather than publish it
    /// (FD § "C-295-L — approved node-shared listener and capability contract" (the capability claim, publication, and retirement rules)), which the retained shared body
    /// `enforcement_returning_after_retirement_tears_down_the_real_returned_handle_before_drain`
    /// asserts. The DESIGN governs, so this twin keeps the join: stop does not
    /// return while the enforce child is in flight, and when it returns the
    /// child has ended and its handle is torn down exactly once.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shared_allocation_stop_joins_an_inflight_enforce_child() {
        let spy = GatedEnforcement::new();
        let (leg, orig_dst, _client) = accepted_leg_f();
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = Arc::new(MtlsInterceptWorker::new(
            Arc::clone(&spy) as Arc<dyn MtlsEnforcement>,
            resolve_scripting(
                orig_dst,
                MtlsResolution::Mesh(ResolvedBackend { addr: orig_dst, expected_svid: None }),
            ),
            Arc::new(SimClock::new()),
            Arc::clone(&intercept) as Arc<dyn MtlsIntercept>,
        ));
        let the_alloc = alloc("shared-stopped-owner-child");
        worker.start_shared_owner().await.expect("publish the shared owner");
        worker
            .start_alloc(&shared_spec(the_alloc.clone(), Ipv4Addr::LOCALHOST))
            .await
            .expect("publish one shared allocation");
        let (leg_f, _leg_c) = shared_legs(&intercept, &worker, &the_alloc);
        deliver_leg_f_connection(&intercept, leg_f, Ipv4Addr::LOCALHOST, orig_dst, leg);
        tokio::time::timeout(Duration::from_secs(2), spy.entered())
            .await
            .expect("enforce child enters its blocking gate");

        let mut stop = tokio::spawn({
            let worker = Arc::clone(&worker);
            let the_alloc = the_alloc.clone();
            async move { worker.stop_alloc(&the_alloc).await }
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(25), &mut stop).await.is_err(),
            "allocation stop cannot pass the in-flight enforce child"
        );
        spy.release();
        tokio::time::timeout(Duration::from_secs(10), stop)
            .await
            .expect("allocation stop is bounded")
            .expect("allocation stop task joins")
            .expect("allocation stop succeeds");
        assert!(spy.exited.load(Ordering::SeqCst), "the enforce child ended before stop returned");
        let torn_down = spy.torn_down();
        assert_eq!(torn_down.len(), 1, "the late handle is torn down exactly once, never orphaned");
        assert_eq!(torn_down[0].alloc(), &the_alloc);
        worker.shutdown_owner().await.expect("the shared owner joins");
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-20 — Node-shared listener and capability lifecycle; exact-port rebind; port theft.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Twin of `allocation_stop_joins_a_passthrough_child` over a shared
    /// allocation: stop joins every pass-through child before returning, so
    /// both relay legs are closed when it returns.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shared_allocation_stop_joins_a_passthrough_child() {
        let upstream = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("bind upstream server");
        let upstream_addr = match upstream.local_addr().expect("upstream local address") {
            std::net::SocketAddr::V4(addr) => addr,
            std::net::SocketAddr::V6(_) => panic!("test binds IPv4"),
        };
        let (accepted_tx, accepted_rx) = std::sync::mpsc::sync_channel(1);
        let accept_thread = std::thread::spawn(move || {
            let (stream, _) = upstream.accept().expect("accept pass-through dial");
            accepted_tx.send(stream).expect("return accepted upstream leg");
        });

        let (spy, _calls) = SpyEnforcement::new();
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = Arc::new(MtlsInterceptWorker::new(
            spy,
            resolve_scripting(upstream_addr, MtlsResolution::NonMesh),
            Arc::new(SimClock::new()),
            Arc::clone(&intercept) as Arc<dyn MtlsIntercept>,
        ));
        let the_alloc = alloc("shared-stopped-passthrough-child");
        worker.start_shared_owner().await.expect("publish the shared owner");
        worker
            .start_alloc(&shared_spec(the_alloc.clone(), Ipv4Addr::LOCALHOST))
            .await
            .expect("publish one shared allocation");
        let (leg_f, _leg_c) = shared_legs(&intercept, &worker, &the_alloc);
        let (leg, _addr, mut client) = accepted_leg_f();
        deliver_leg_f_connection(&intercept, leg_f, Ipv4Addr::LOCALHOST, upstream_addr, leg);
        let mut accepted = accepted_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("pass-through child connects to upstream");
        accept_thread.join().expect("upstream accept thread");

        tokio::time::timeout(Duration::from_secs(10), worker.stop_alloc(&the_alloc))
            .await
            .expect("allocation stop is bounded")
            .expect("allocation stop succeeds");

        client.set_read_timeout(Some(Duration::from_secs(1))).expect("set client timeout");
        accepted.set_read_timeout(Some(Duration::from_secs(1))).expect("set upstream timeout");
        let mut byte = [0_u8; 1];
        assert_eq!(client.read(&mut byte).expect("client leg closes after joined child"), 0);
        assert_eq!(accepted.read(&mut byte).expect("upstream leg closes after joined child"), 0);
        worker.shutdown_owner().await.expect("the shared owner joins");
    }

    /// An `MtlsResolve` whose `resolve` announces it has entered (the claim is
    /// held) and then blocks until released, so a body can hold a relay's
    /// classifying claim in flight while a stop runs.
    struct GatedResolve {
        arm: MtlsResolution,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl MtlsResolve for GatedResolve {
        async fn probe(&self) -> overdrive_core::traits::mtls_resolve::Result<()> {
            Ok(())
        }

        async fn resolve(
            &self,
            _orig_dst: SocketAddrV4,
        ) -> overdrive_core::traits::mtls_resolve::Result<MtlsResolution> {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(self.arm.clone())
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-20 — Node-shared listener and capability lifecycle; exact-port rebind; port theft.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// C-295-L pass-through obligation (FD § "Why a stop ends its pass-through
    /// relays", 2026-09-25): a relay whose connection is classified WHILE the
    /// stop's claim wait is in progress belongs to the retired generation, so
    /// `stop_alloc` returns only after both of that relay's legs are closed and
    /// no relay of the generation remains. An implementation that ends relays
    /// before the claim wait, or registers a relay after releasing its claim
    /// (the current `handle_shared_outbound` detach, `:3563-3567`), fails this.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shared_allocation_stop_ends_a_relay_classified_during_its_claim_wait() {
        let upstream = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("bind upstream server");
        let upstream_addr = match upstream.local_addr().expect("upstream local address") {
            std::net::SocketAddr::V4(addr) => addr,
            std::net::SocketAddr::V6(_) => panic!("test binds IPv4"),
        };
        let (accepted_tx, accepted_rx) = std::sync::mpsc::sync_channel(1);
        let accept_thread = std::thread::spawn(move || {
            let (stream, _) = upstream.accept().expect("accept pass-through dial");
            accepted_tx.send(stream).expect("return accepted upstream leg");
        });

        let (spy, _calls) = SpyEnforcement::new();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = Arc::new(MtlsInterceptWorker::new(
            spy,
            Arc::new(GatedResolve {
                arm: MtlsResolution::NonMesh,
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
            }) as Arc<dyn MtlsResolve>,
            Arc::new(SimClock::new()),
            Arc::clone(&intercept) as Arc<dyn MtlsIntercept>,
        ));
        let the_alloc = alloc("shared-relay-during-claim-wait");
        worker.start_shared_owner().await.expect("publish the shared owner");
        worker
            .start_alloc(&shared_spec(the_alloc.clone(), Ipv4Addr::LOCALHOST))
            .await
            .expect("publish one shared allocation");
        let (leg_f, _leg_c) = shared_legs(&intercept, &worker, &the_alloc);
        let (leg, _addr, mut client) = accepted_leg_f();
        deliver_leg_f_connection(&intercept, leg_f, Ipv4Addr::LOCALHOST, upstream_addr, leg);

        // The classifying claim is now held in flight (resolve is blocked).
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .expect("the relay's classifying claim is taken and resolve is in flight");

        // Stop begins while the claim is still in flight; it must not return
        // until the relay it will produce has ended.
        let mut stop = tokio::spawn({
            let worker = Arc::clone(&worker);
            let the_alloc = the_alloc.clone();
            async move { worker.stop_alloc(&the_alloc).await }
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(25), &mut stop).await.is_err(),
            "stop cannot return while a relay's classifying claim is in flight"
        );

        // Release the classification; the relay is established, then ended by
        // the stop's claim wait.
        release.notify_one();
        let _accepted = accepted_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the classified relay connects to upstream");
        accept_thread.join().expect("upstream accept thread");
        tokio::time::timeout(Duration::from_secs(10), stop)
            .await
            .expect("stop is bounded")
            .expect("stop task joins")
            .expect("stop succeeds");

        client.set_read_timeout(Some(Duration::from_secs(1))).expect("set client timeout");
        let mut byte = [0_u8; 1];
        assert_eq!(
            client.read(&mut byte).expect("the relay's client leg is closed when stop returns"),
            0,
            "no relay of the retired generation remains after stop"
        );
        worker.shutdown_owner().await.expect("the shared owner joins");
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-20 — Node-shared listener and capability lifecycle; exact-port rebind; port theft.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// C-295-L owner-shutdown obligation (FD § "Why a stop ends its pass-through
    /// relays"): when `shutdown_owner` returns, every relay of every allocation
    /// has ended — a live pass-through relay's legs are both closed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shared_owner_shutdown_ends_a_live_relay() {
        let upstream = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("bind upstream server");
        let upstream_addr = match upstream.local_addr().expect("upstream local address") {
            std::net::SocketAddr::V4(addr) => addr,
            std::net::SocketAddr::V6(_) => panic!("test binds IPv4"),
        };
        let (accepted_tx, accepted_rx) = std::sync::mpsc::sync_channel(1);
        let accept_thread = std::thread::spawn(move || {
            let (stream, _) = upstream.accept().expect("accept pass-through dial");
            accepted_tx.send(stream).expect("return accepted upstream leg");
        });

        let (spy, _calls) = SpyEnforcement::new();
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = Arc::new(MtlsInterceptWorker::new(
            spy,
            resolve_scripting(upstream_addr, MtlsResolution::NonMesh),
            Arc::new(SimClock::new()),
            Arc::clone(&intercept) as Arc<dyn MtlsIntercept>,
        ));
        let the_alloc = alloc("shared-relay-owner-shutdown");
        worker.start_shared_owner().await.expect("publish the shared owner");
        worker
            .start_alloc(&shared_spec(the_alloc.clone(), Ipv4Addr::LOCALHOST))
            .await
            .expect("publish one shared allocation");
        let (leg_f, _leg_c) = shared_legs(&intercept, &worker, &the_alloc);
        let (leg, _addr, mut client) = accepted_leg_f();
        deliver_leg_f_connection(&intercept, leg_f, Ipv4Addr::LOCALHOST, upstream_addr, leg);
        let mut accepted = accepted_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the live relay connects to upstream");
        accept_thread.join().expect("upstream accept thread");

        tokio::time::timeout(Duration::from_secs(10), worker.shutdown_owner())
            .await
            .expect("owner shutdown is bounded")
            .expect("the shared owner joins");

        client.set_read_timeout(Some(Duration::from_secs(1))).expect("set client timeout");
        accepted.set_read_timeout(Some(Duration::from_secs(1))).expect("set upstream timeout");
        let mut byte = [0_u8; 1];
        assert_eq!(
            client.read(&mut byte).expect("the relay's client leg is closed after owner shutdown"),
            0
        );
        assert_eq!(
            accepted
                .read(&mut byte)
                .expect("the relay's upstream leg is closed after owner shutdown"),
            0
        );
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-20 — Node-shared listener and capability lifecycle; exact-port rebind; port theft.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Twin of `allocation_start_after_owner_shutdown_is_rejected_before_install`
    /// over a shared allocation: a late install creates no listener, member, or
    /// child.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shared_allocation_start_after_owner_shutdown_is_rejected_before_install() {
        let intercept = Arc::new(TestSharedIntercept::new());
        let worker = shared_worker(&intercept);
        worker.start_shared_owner().await.expect("publish the shared owner");
        worker.shutdown_owner().await.expect("worker owner shutdown converges");
        let alloc = alloc("shared-late-after-owner-shutdown");

        assert!(matches!(
            worker.start_alloc(&shared_spec(alloc.clone(), Ipv4Addr::new(100, 95, 0, 2))).await,
            Err(super::MtlsInterceptInstallError::OwnerShutdown)
        ));
        assert_eq!(worker.leg_c_addr(&alloc), None);
        assert_eq!(intercept.members(), InterceptMembers::default(), "no member was installed");
        assert!(intercept.live_listeners().is_empty(), "no listener outlives the owner");
    }
}

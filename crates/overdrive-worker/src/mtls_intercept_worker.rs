//! The worker's mTLS intercept-and-enforce lifecycle component
//! (D-MTLS-16 / D-MTLS-17, GH #26; step 06-03).
//!
//! This is the **(β) separate lifecycle component** the action-shim fires
//! alongside the VM driver hooks. It owns the production mTLS intercept-install +
//! leg-acquire + `enforce` wiring per allocation:
//!
//! - [`start_alloc`](MtlsInterceptWorker::start_alloc) — fired at the
//!   action-shim's `on_alloc_running` site (after the alloc commits a
//!   `Running` row). Installs the OUTBOUND egress nft-TPROXY rule
//!   ([`install_outbound_tproxy`](crate::mtls_intercept::install_outbound_tproxy),
//!   D-TME-4 / ADR-0071 Path A) matching the allocation's host-side veth
//!   (`spec.host_veth`, set by the action-shim C3 provision seam, JOIN-6) and
//!   redirecting the workload's egress TCP to leg-F; stands up the agent's
//!   leg-F (outbound, plaintext) + leg-C (inbound, `IP_TRANSPARENT`)
//!   listeners, and spawns the accept→`enforce` tasks. Installs the INBOUND
//!   nft-TPROXY rules (D-A1, GH #241) — one per declared Service listener port,
//!   keyed `ip daddr <spec.workload_addr> tcp dport <service_port>` and
//!   tproxy-redirected to the agent's leg-C port — when `spec.workload_addr` is
//!   `Some` (the per-workload netns/veth the C3 seam provisions). N declared
//!   ports → N rules; a Job-kind / host-netns workload (`None` addr or empty
//!   `service_ports`) installs ZERO inbound rules. See the module-level note
//!   below.
//! - [`stop_alloc`](MtlsInterceptWorker::stop_alloc) — fired at the
//!   action-shim's `on_alloc_terminal` site. Drains the alloc's
//!   per-connection teardown set (`enforcement.teardown`), signals the
//!   accept tasks to stop, and drops the OUTBOUND + INBOUND intercept guards (each
//!   releases exactly what its install acquired — for the production
//!   `HostMtlsIntercept` that is its per-veth / per-virt nft rule, removed by
//!   handle; the node-global shared routing infra is left intact).

#![allow(
    clippy::result_large_err,
    reason = "GH #295 exact shared-owner/install errors retain nested source-honest intercept outcomes"
)]
//!   Idempotent.
//!
//! ## Supervision shape — (C)+(B), no central loop (ADR-0070 / D-MTLS-16)
//!
//! Connection liveness is **(C)** kernel `TCP_USER_TIMEOUT`/keepalive (set
//! inside `enforce` on the legs) **+ (B)** the per-connection pump task
//! self-tearing-down fail-closed on its own terminal exit. This worker
//! holds only **per-alloc lifecycle bookkeeping** (keyed by
//! `AllocationId`, drained on `on_alloc_terminal`) — NOT a central
//! liveness registry, NOT a `supervise_tick`, NOT a tick cadence. The
//! retired central `MtlsSupervisor` (shape (A)) is deleted.
//!
//! ## Outbound interception (ADR-0071 Path A) + inbound per-port install (D-A1)
//!
//! The OUTBOUND intercept is the per-veth egress nft-TPROXY rule: every TCP
//! flow the workload emits on its host-side veth (`iifname spec.host_veth`)
//! is TPROXY-redirected to the agent's leg-F listener, with the original
//! destination recovered per-flow via `getsockname` on the accepted leg-F
//! socket (D-TME-4, symmetric with the inbound TPROXY path). No per-peer
//! enumeration is needed — TPROXY captures ALL the workload's egress, so the
//! declared-peer `MTLS_REDIRECT_DEST` map + per-destination rewrite of the
//! retired cgroup mechanism are GONE (D-TME-3 RETIRED). As of step 04-02 the
//! per-connection [`MtlsResolve`](overdrive_core::traits::mtls_resolve::MtlsResolve)
//! consumer drives the outbound accept loop: each captured connection's
//! recovered `orig_dst` is resolved against the mesh and branched on the
//! returned `MtlsResolution` variant (ADR-0071 fact 4, C1) —
//! `Mesh`→`enforce` over mTLS to the resolved backend, `NonMesh`→cleartext
//! pass-through (by design), `MeshUnreachable`→fail-closed (refuse, NO
//! cleartext). The vestigial declared-peer `real_peer` slot is GONE (deleted
//! single-cut this step alongside the resolve consumer it superseded).
//!
//! The INBOUND nft-TPROXY rules are installed by `start_alloc` (D-A1, GH #241 —
//! the keystone that closed the prior `tproxy_guard = None` deferral): one
//! [`install_inbound_tproxy`](crate::mtls_intercept::install_inbound_tproxy)
//! per declared Service listener port (`spec.service_ports`), each keyed on the
//! canonical workload address `spec.workload_addr` + that port and
//! tproxy-redirected to the agent's leg-C port. The match `dport` is the
//! DECLARED service port (D-BLOCKER1 / D-TME-10 one-source/two-readers — the
//! SAME value `service_backends` advertises and the egress `MtlsResolve` keys
//! on), never the ephemeral leg-C port. `start_alloc` installs N rules for N
//! declared ports when `spec.workload_addr` is `Some`, and ZERO for a Job-kind
//! / host-netns workload (`None` addr or empty `service_ports`). Everything
//! (the outbound egress rule + the per-port inbound rules + leg-F + leg-C
//! listeners + both accept loops + `enforce` + the wire) is production.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::{NonZeroU16, NonZeroU64};
use std::os::fd::AsRawFd as _;
#[cfg(any(test, feature = "integration-tests"))]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::AllocationSpec;
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnection, InterceptedConnection, MtlsEnforcement, Routed,
};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_core::{AllocationId, SpiffeId};
use parking_lot::{Mutex, RwLock};
use tokio::sync::{Notify, mpsc, watch};
use tokio::task::JoinHandle;

use crate::mtls_intercept::{
    InterceptError, InterceptLeg, InterceptPostcondition, accept_inbound_leg,
    accept_outbound_and_recover_orig_dst,
};
use crate::mtls_intercept_port::{InterceptGuard, MtlsIntercept};

/// Per-alloc transparent-mTLS intercept-install failure (D-MTLS-18).
///
/// Returned by [`MtlsInterceptWorker::start_alloc`] when any of the three
/// install steps fails. The install is a **fail-closed security control**,
/// not a best-effort observability hook: an alloc whose intercept cannot be
/// installed MUST NOT run with cleartext egress/ingress, so the failure is
/// SURFACED to the action-shim (which drives the alloc to terminal `Failed`),
/// not swallowed in a `warn!`.
///
/// This enum invents NO new lower-level error surface. Its three install-step
/// variants wrap the typed [`InterceptError`] the install steps already produce
/// (the OUTBOUND egress nft-TPROXY install + the leg-F and leg-C transparent
/// listeners — both bound via
/// [`make_transparent_listener`](crate::mtls_intercept::make_transparent_listener)).
/// The two bound-address capture variants
/// ([`Self::LegFLocalAddr`] / [`Self::LegCLocalAddr`], D-MTLS-18 sites 2/3) carry
/// a raw [`std::io::Error`] `#[source]` — the `getsockname` failure
/// [`TcpListener::local_addr`](std::net::TcpListener::local_addr) returns — which
/// is a `std` type, not a new lower-level surface. They fail the install closed
/// rather than defaulting the bound addr to a broken port 0.
/// Each source `Display` names the privilege / kernel-feature
/// remediation an operator acts on. (The per-port inbound nft-TPROXY rule
/// install — D-A1 / GH #241 — IS an install step now: its decomposed
/// [`InterceptError::NftRuleInstallFailed`] / [`InterceptError::IpRuleAddFailed`]
/// / [`InterceptError::IpRouteLocalAddFailed`] failures flow through the
/// `Inbound` variant from the production `start_alloc` path, see the module
/// note.)
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
    /// allocation install is rejected before binding listeners or installing
    /// rules, so replacement startup cannot create work behind the old
    /// owner's completion boundary.
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

    /// OUTBOUND nft-TPROXY rule install (`install_outbound_tproxy`) failed
    /// (site 1). The egress rule matches the workload's host-side veth
    /// (`spec.host_veth`) and redirects its egress TCP to the agent's leg-F
    /// listener (D-TME-4, ADR-0071 Path A). Source `Display` names the
    /// `CAP_NET_ADMIN` / nft / shared-routing-infra remediation.
    ///
    /// `#[source]` (not `#[from]`): the sibling `Inbound` variant already
    /// owns the single `#[from] InterceptError` auto-conversion, so the
    /// outbound site names this constructor explicitly to keep the two
    /// `InterceptError` sources distinct in `Display`.
    #[error("mTLS outbound TPROXY install failed: {0}")]
    OutboundTproxyInstall(#[source] InterceptError),

    /// leg-F (outbound, workload-facing plaintext) `IP_TRANSPARENT` listener
    /// bind failed (site 2). leg-F is bound via
    /// [`make_transparent_listener`](crate::mtls_intercept::make_transparent_listener)
    /// — the SAME transparent-socket call leg-C (`Inbound`) uses — because the
    /// OUTBOUND egress `tproxy` divert is non-rewriting and delivers
    /// orig-dst-addressed packets a plain socket cannot receive. The source is
    /// therefore the typed [`InterceptError`] that transparent bind produces
    /// (most often [`InterceptError::TransparentListener`], whose `Display`
    /// names the `CAP_NET_ADMIN` / `IP_TRANSPARENT` remediation), NOT a bare
    /// `io::Error`. `#[source]` (not `#[from]`): the sibling `Inbound` variant
    /// already owns the single `#[from] InterceptError` auto-conversion, so the
    /// site-2 leg-F bind names its constructor explicitly to keep the two
    /// `InterceptError` sources distinct in `Display`.
    #[error("mTLS leg-F listener bind failed: {0}")]
    LegFBind(#[source] InterceptError),

    /// INBOUND intercept install failed (site 3). Two sources flow through this
    /// variant's `#[from] InterceptError` from the production `start_alloc`
    /// path: (a) the leg-C transparent listener bind
    /// ([`InterceptError::TransparentListener`]), and (b) any of the per-port
    /// inbound nft-TPROXY rule installs
    /// ([`install_inbound_tproxy`](crate::mtls_intercept::install_inbound_tproxy)
    /// → [`InterceptError::NftRuleInstallFailed`] /
    /// [`InterceptError::NftHandleRecoveryFailed`] /
    /// [`InterceptError::IpRuleAddFailed`] /
    /// [`InterceptError::IpRouteLocalAddFailed`]) now performed by `start_alloc`
    /// (D-A1, GH #241). Source `Display` names the privilege / kernel-feature /
    /// shared-routing-infra remediation. Fail-closed: an install error
    /// short-circuits, dropping every guard acquired this call.
    #[error("mTLS inbound intercept install failed: {0}")]
    Inbound(#[from] InterceptError),

    /// leg-F (outbound) listener bound-address capture failed (`local_addr()` /
    /// getsockname on the leg-F transparent listener). Distinct from `LegFBind`
    /// (the bind itself succeeded): the kernel could not report the bound addr, so
    /// the OUTBOUND TPROXY redirect target is unknown and the install MUST fail
    /// closed rather than redirect to port 0 (D-MTLS-18 site 2).
    #[error("mTLS leg-F listener address capture failed: {source}")]
    LegFLocalAddr {
        #[source]
        source: std::io::Error,
    },

    /// leg-C (inbound) listener bound-address capture failed (`local_addr()` /
    /// getsockname on the leg-C transparent listener). Distinct from the `Inbound`
    /// bind failure: fail closed rather than record a port-0 leg-C addr that would
    /// silently corrupt the #241 inbound-redirect read (D-MTLS-18 site 3).
    #[error("mTLS leg-C listener address capture failed: {source}")]
    LegCLocalAddr {
        #[source]
        source: std::io::Error,
    },
}

/// A terminal allocation stop reached one or more authoritative mTLS
/// connection teardowns that did not complete successfully.
#[derive(Clone, Debug, thiserror::Error)]
#[error("mTLS teardown failed for allocation {alloc_id}: {failures:?}")]
pub struct MtlsInterceptStopError {
    /// Allocation whose terminal cleanup remains incomplete.
    pub alloc_id: AllocationId,
    /// Stable diagnostics for every connection teardown that failed.
    pub failures: Vec<String>,
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
        leg: crate::mtls_intercept::InterceptLeg,
        task: tokio::task::JoinHandle<SharedListenerTaskResult>,
    ) -> std::result::Result<(), MtlsSharedOwnerError> {
        let mut slots = self.slots.lock();
        let slot = match leg {
            InterceptLeg::F => &mut slots.leg_f,
            InterceptLeg::C => &mut slots.leg_c,
        };
        if slot.is_some() {
            return Err(MtlsSharedOwnerError::TaskObserverClosed);
        }
        let Some(event_tx) = self.event_tx.upgrade() else {
            return Err(MtlsSharedOwnerError::TaskObserverClosed);
        };
        *slot = Some(Self::observe(leg, task, &event_tx));
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
            .replace_terminal(InterceptLeg::F, tokio::spawn(async { Ok(()) }))
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
            .replace_terminal(
                InterceptLeg::F,
                tokio::spawn(std::future::pending::<SharedListenerTaskResult>()),
            )
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
        let replacement = tokio::spawn(async { Ok(()) });
        assert!(matches!(
            owner.replace_terminal(InterceptLeg::F, replacement),
            Err(MtlsSharedOwnerError::TaskObserverClosed)
        ));
        assert!(owner.is_live(InterceptLeg::F));
        assert!(owner.is_live(InterceptLeg::C));
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(consume_task_owner(owner).await, "shutdown consumes the event receiver");
        assert_eq!(drops.load(Ordering::SeqCst), 2, "both original listeners are joined");
    }
}

impl MtlsInterceptInstallError {
    /// Associated constructor for the site-2 leg-F transparent-listener bind
    /// failure, per the project's "associated constructor per variant"
    /// convention. The source is the typed [`InterceptError`]
    /// [`make_transparent_listener`](crate::mtls_intercept::make_transparent_listener)
    /// produces. The `#[source]` wrap (not `#[from]`, which the `Inbound`
    /// variant owns for `InterceptError`) means there is no auto-conversion, so
    /// the call site names this constructor explicitly.
    #[must_use]
    const fn leg_f_bind(source: InterceptError) -> Self {
        Self::LegFBind(source)
    }

    /// Associated constructor for the site-1 outbound nft-TPROXY install
    /// failure. `#[source]` wrap (not `#[from]`, which the `Inbound` variant
    /// owns), so the call site names this constructor explicitly.
    #[must_use]
    const fn outbound_tproxy_install(source: InterceptError) -> Self {
        Self::OutboundTproxyInstall(source)
    }

    /// Associated constructor for the leg-F (outbound) listener bound-address
    /// capture failure (`local_addr()` getsockname error). Used as the `on_err`
    /// mapper at the leg-F `project_listener_v4` call site so the failure carries
    /// the leg-F stage (D-MTLS-18 site 2).
    #[must_use]
    const fn leg_f_local_addr(source: std::io::Error) -> Self {
        Self::LegFLocalAddr { source }
    }

    /// Associated constructor for the leg-C (inbound) listener bound-address
    /// capture failure (`local_addr()` getsockname error). Used as the `on_err`
    /// mapper at the leg-C `project_listener_v4` call site so the failure carries
    /// the leg-C stage (D-MTLS-18 site 3).
    #[must_use]
    const fn leg_c_local_addr(source: std::io::Error) -> Self {
        Self::LegCLocalAddr { source }
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
    elements: CapabilityElements,
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
    handles: Vec<EnforcedConnection>,
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
                handles: Vec::new(),
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

    fn has_live_records(&self) -> bool {
        !self.inner.state.lock().records.is_empty()
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
        }
    }

    fn retain_inbound(&mut self, guard: Box<dyn InterceptGuard>) {
        if let Some(elements) = self.elements.as_mut() {
            elements.inbound.push(guard);
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
        if self.completed {
            return;
        }
        if let Some(record) = self.inner.state.lock().records.get_mut(&self.key) {
            record.in_flight = record.in_flight.saturating_sub(1);
        }
        self.inner.wake.notify_waiters();
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
                        elements: CapabilityElements { outbound: None, inbound: Vec::new() },
                        completed: false,
                    };
                };
                if record.lifecycle == CapabilityLifecycle::Retiring
                    && record.in_flight == 0
                    && !record.pending_owner
                {
                    Some((
                        std::mem::take(&mut record.handles),
                        std::mem::replace(
                            &mut record.elements,
                            CapabilityElements { outbound: None, inbound: Vec::new() },
                        ),
                    ))
                } else {
                    None
                }
            };
            if let Some((handles, elements)) = ready {
                return CapabilityDrain {
                    inner: Arc::clone(&self.inner),
                    key: self.key,
                    handles,
                    elements,
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
    leg_f_listener: Option<std::net::TcpListener>,
    leg_c_listener: Option<std::net::TcpListener>,
    stop: Arc<AtomicBool>,
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
    listener: std::net::TcpListener,
    stop: Arc<AtomicBool>,
    worker: Weak<MtlsInterceptWorker>,
    leg: InterceptLeg,
) -> tokio::task::JoinHandle<SharedListenerTaskResult> {
    tokio::task::spawn_blocking(move || {
        loop {
            if stop.load(Ordering::SeqCst) {
                return Ok(());
            }
            if matches!(
                await_pending_connection(&listener, &stop, &worker),
                ConnectionReady::Stopped
            ) {
                return Ok(());
            }
            let Some(worker) = worker.upgrade() else {
                return Ok(());
            };
            match leg {
                InterceptLeg::F => match accept_outbound_and_recover_orig_dst(&listener) {
                    Ok((leg_f, orig_dst)) => {
                        let source_addr = peer_addr(&leg_f)?;
                        worker.handle_shared_outbound(source_addr, leg_f, orig_dst);
                    }
                    Err(InterceptError::Accept { source, .. }) => return Err(source),
                    Err(source) => {
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
                    match accept_inbound_leg(&listener, placeholder) {
                        Ok(connection) => worker.handle_shared_inbound(connection),
                        Err(InterceptError::Accept { source, .. }) => return Err(source),
                        Err(source) => {
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
fn peer_addr(leg: &std::os::fd::OwnedFd) -> std::io::Result<Ipv4Addr> {
    let mut address: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    // SAFETY: `address` and `length` describe a writable IPv4 sockaddr buffer
    // owned by this call, and `leg` remains live for the syscall.
    let result = unsafe {
        libc::getpeername(
            leg.as_raw_fd(),
            std::ptr::from_mut(&mut address).cast(),
            std::ptr::from_mut(&mut length),
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(Ipv4Addr::from(u32::from_be(address.sin_addr.s_addr)))
}

fn shared_listener_address(
    leg: InterceptLeg,
    listener: &std::net::TcpListener,
) -> Result<SocketAddrV4, MtlsSharedOwnerError> {
    match listener.local_addr() {
        Ok(std::net::SocketAddr::V4(address)) if address.port() != 0 => Ok(address),
        Ok(std::net::SocketAddr::V4(address)) => Err(MtlsSharedOwnerError::ListenerPostcondition {
            leg,
            expected: address,
            observed: Some(address),
        }),
        Ok(std::net::SocketAddr::V6(_)) => Err(MtlsSharedOwnerError::ListenerPostcondition {
            leg,
            expected: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1),
            observed: None,
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

/// Per-allocation intercept state held for the alloc's lifetime and
/// torn down on `stop_alloc`. This is lifecycle bookkeeping keyed by
/// `AllocationId` (NOT a liveness loop — D-MTLS-16).
struct AllocIntercept {
    /// `true` when this record is backed by the node-shared capability
    /// registry. Legacy host-netns fixtures retain their local owner shape.
    capability_owned: bool,
    /// The OUTBOUND egress-capture guard for this alloc's host-side veth
    /// ([`MtlsIntercept::install_outbound`], D-TME-4 / ADR-0071 Path A).
    /// Dropping it releases exactly what that install acquired, and nothing
    /// another guard owns (the [`InterceptGuard`] contract). WHAT is released
    /// is adapter-specific and NOT asserted here: `HostMtlsIntercept` removes
    /// the per-veth egress `nft` rule from the shared `prerouting` chain by
    /// handle, leaving the node-global shared routing infra intact; a
    /// simulation adapter releases nothing.
    /// `Some` on the mTLS-composed production boot (where the action-shim C3
    /// seam set `spec.host_veth`); `None` off the gate (a fixture with no
    /// provisioned veth), where the leg-F listener + accept loop still stand
    /// up but no egress capture is installed.
    _outbound_tproxy_guard: Option<Box<dyn InterceptGuard>>,
    /// The inbound redirect guards — ONE per declared Service listener port
    /// ([`MtlsIntercept::install_inbound`], D-A1, GH #241). Each guard's
    /// `Drop` releases exactly what its own install acquired and nothing
    /// another guard owns (the [`InterceptGuard`] contract); for
    /// `HostMtlsIntercept` that is its per-virt `nft` rule (keyed
    /// `ip daddr <workload_addr> tcp dport <service_port>`, tproxy-redirected
    /// to the ephemeral leg-C port), removed from the shared chain by handle,
    /// while a simulation adapter releases nothing.
    /// `start_alloc` installs one capture per `spec.service_ports` entry
    /// when `spec.workload_addr` is `Some`; the `Vec` is EMPTY for a Job-kind /
    /// host-netns workload (`None` addr or empty `service_ports`) — the
    /// unchanged 0-rules path. All guards drop together on `stop_alloc`.
    _inbound_tproxy_guards: Vec<Box<dyn InterceptGuard>>,
    /// The ephemeral loopback addr leg-C (the inbound `IP_TRANSPARENT`
    /// listener) was bound to in `start_alloc`, captured BEFORE the listener
    /// was moved into the spawned inbound `accept_loop` — mirroring the leg-F
    /// **capture pattern** (leg-F's addr is an inline local in `start_alloc`,
    /// not a public accessor; see `leg_f_addr` there). Retained so
    /// [`leg_c_addr`] can be a pure in-memory read — the listener itself has
    /// been consumed by the accept task and its `local_addr()` is no longer
    /// reachable from here. Private to the module; the only public surface is
    /// the [`leg_c_addr`] accessor.
    ///
    /// [`leg_c_addr`]: MtlsInterceptWorker::leg_c_addr
    leg_c_addr: SocketAddrV4,
    /// Cooperative stop flag for the blocking accept loops. The loops run
    /// on `spawn_blocking` threads, so `JoinHandle::abort` cannot interrupt
    /// a blocking `accept()`/`poll()` mid-syscall — the loops must observe
    /// this flag between bounded poll slices and exit themselves.
    /// `stop_alloc` sets it; without it a blocking accept loop outlives the
    /// alloc (and, in a test runtime, blocks the runtime drop forever).
    stop: Arc<AtomicBool>,
    /// The `EnforcedConnection` handles this alloc produced, drained
    /// through `enforcement.teardown` on stop. An [`EnforcedSet`] (not a raw
    /// `Arc<Mutex<Vec>>`): terminal cleanup first closes and joins the complete
    /// per-allocation producer task tree, then atomically drains this set. No
    /// producer can push after the final drain.
    enforced: EnforcedSet,
    /// Every accept, resolve, enforce, and pass-through child for this
    /// allocation. Terminal cleanup seals this owner and joins it before
    /// draining the final enforced-handle set.
    tasks: AllocationTaskOwner,
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

    #[cfg(any(test, feature = "integration-tests"))]
    fn is_complete(&self) -> bool {
        *self.inner.complete.borrow()
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

/// Private owner for the dynamically growing accept/resolve/enforce/
/// pass-through tree of exactly one allocation.
#[derive(Clone)]
struct AllocationTaskOwner {
    inner: Arc<AllocationTaskOwnerInner>,
}

struct AllocationTaskOwnerInner {
    state: Mutex<AllocationTaskState>,
    shutdown: StopCompletion,
}

#[derive(Default)]
struct AllocationTaskState {
    lifecycle: AllocationTaskLifecycle,
    tasks: Vec<JoinHandle<()>>,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum AllocationTaskLifecycle {
    #[default]
    Open,
    Stopping,
    Stopped,
}

impl AllocationTaskOwner {
    fn new() -> Self {
        Self {
            inner: Arc::new(AllocationTaskOwnerInner {
                state: Mutex::new(AllocationTaskState::default()),
                shutdown: StopCompletion::new(),
            }),
        }
    }

    fn spawn(&self, spawn: impl FnOnce() -> JoinHandle<()>) -> bool {
        let mut state = self.inner.state.lock();
        if state.lifecycle != AllocationTaskLifecycle::Open {
            return false;
        }
        state.tasks.push(spawn());
        true
    }

    async fn abort_and_join(&self) {
        let leader_tasks = {
            let mut state = self.inner.state.lock();
            match state.lifecycle {
                AllocationTaskLifecycle::Open => {
                    state.lifecycle = AllocationTaskLifecycle::Stopping;
                    Some(std::mem::take(&mut state.tasks))
                }
                AllocationTaskLifecycle::Stopping | AllocationTaskLifecycle::Stopped => None,
            }
        };
        if let Some(tasks) = leader_tasks {
            let inner = Arc::clone(&self.inner);
            self.inner.shutdown.start_with(move || async move {
                for task in &tasks {
                    task.abort();
                }
                for task in tasks {
                    let _ = task.await;
                }
                inner.state.lock().lifecycle = AllocationTaskLifecycle::Stopped;
            });
        }
        self.inner.shutdown.wait().await;
    }
}

impl Drop for AllocationTaskOwnerInner {
    fn drop(&mut self) {
        for task in self.state.get_mut().tasks.drain(..) {
            task.abort();
        }
    }
}

/// Per-allocation enforced-connection set.
///
/// The stop owner closes and joins the complete allocation task tree before
/// draining this set. That task fence is the admission boundary: every
/// successful enforcement handle is retained here, and no producer can push
/// after the final drain.
#[derive(Clone)]
struct EnforcedSet {
    inner: Arc<Mutex<Vec<EnforcedConnection>>>,
}

impl EnforcedSet {
    /// A fresh empty set.
    fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(Vec::new())) }
    }

    /// Atomically retain a completed handle. The stop owner drains only after
    /// joining every producer.
    fn push(&self, handle: EnforcedConnection) {
        self.inner.lock().push(handle);
    }

    /// Atomic drain after the producer task fence. Idempotent.
    fn drain(&self) -> Vec<EnforcedConnection> {
        std::mem::take(&mut *self.inner.lock())
    }

    /// Test-only count of currently-held (not-yet-drained) handles. Used by
    /// the per-arm resolve-consumer tests to observe that an enforced handle
    /// joined the set; NOT production surface (no `pub`, `#[cfg(test)]`).
    #[cfg(test)]
    fn held_count(&self) -> usize {
        self.inner.lock().len()
    }
}

/// The worker-side mTLS intercept-and-enforce lifecycle component.
///
/// Constructed ONCE at the control-plane composition root, AFTER
/// `IdentityMgr` (so `HostMtlsEnforcement` can read the held identity),
/// with both ports as REQUIRED `new()` params per
/// `.claude/rules/development.md` § "Port-trait dependencies". Held by
/// `AppState` as `Option<Arc<MtlsInterceptWorker>>` — `Some` in the
/// production `run_server` boot (and the Tier-3 e2e), `None` for the
/// non-mTLS fixture surface (mirroring the `ProbeRunner` shape).
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
    /// The per-alloc intercept-INSTALL port (`HostMtlsIntercept` in
    /// production; `SimMtlsIntercept` under test composition). Wraps the three
    /// privileged un-ownable primitives `start_alloc` performs — the
    /// `IP_TRANSPARENT` bind and the two nft-TPROXY installs — so the install
    /// surface is substitutable at the composition root. Mandatory `new()`
    /// param, no builder (`.claude/rules/development.md` § "Port-trait
    /// dependencies").
    intercept: Arc<dyn MtlsIntercept>,
    /// One process-local registration/capability state machine.
    #[allow(dead_code, reason = "D-295-DISTILL-7 RED scaffold activated by the single cut")]
    capabilities: CapabilityRegistry,
    /// The one node-owned listener/rule owner. Allocation records refer to
    /// this owner; they never own or replace its sockets.
    shared_owner: Mutex<SharedOwnerState>,
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
    #[cfg(any(test, feature = "integration-tests"))]
    owner_shutdown_failures: AtomicU64,
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
    /// As of step 04-01 (ADR-0071 Path A) the OUTBOUND intercept is the
    /// host-veth nft-TPROXY rule installed per-alloc in
    /// [`start_alloc`](Self::start_alloc) — NOT a `cgroup_connect4_mtls`
    /// attach — so the worker no longer holds an `MtlsDataplane` or a
    /// `cgroup_root`. The host-veth NAME the egress rule matches arrives
    /// per-alloc on `AllocationSpec.host_veth` (JOIN-6), not at construction.
    ///
    /// As of step 04-02 the worker holds the [`MtlsResolve`] port: the outbound
    /// accept loop resolves each captured connection's recovered `orig_dst`
    /// through it and branches on the [`MtlsResolution`] variant — production
    /// wires `ServiceBackendsResolve` (reading `service_backends`), tests wire
    /// `SimMtlsResolve`.
    ///
    /// As of GH #250 (ADR-0076) the worker holds the [`MtlsIntercept`]
    /// install port: `start_alloc`'s three privileged primitives (the two
    /// `IP_TRANSPARENT` binds and the two nft-TPROXY installs) go through it,
    /// so the install surface is substitutable at the composition root.
    /// Production wires `HostMtlsIntercept` (a one-for-one delegation to the
    /// same free functions `start_alloc` called before the port existed, so
    /// wiring it changes no behaviour); tests wire `SimMtlsIntercept`.
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
            intercepts: Mutex::new(BTreeMap::new()),
            pending_allocations: Mutex::new(BTreeSet::new()),
            stopping: Mutex::new(BTreeMap::new()),
            lifecycle: RwLock::new(WorkerLifecycle::Open),
            shutdown: Arc::new(OwnerStop::new()),
            #[cfg(any(test, feature = "integration-tests"))]
            stop_alloc_calls: AtomicU64::new(0),
            #[cfg(any(test, feature = "integration-tests"))]
            owner_shutdown_failures: AtomicU64::new(0),
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
        let result = self.converge_shared_owner_inner(&mut owner);
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
        let state = self.shared_owner.lock();
        let Some(owner) = state.owner.as_ref() else {
            return match state.lifecycle {
                SharedOwnerLifecycle::ShuttingDown => Err(MtlsSharedOwnerError::OwnerShutdown),
                _ => Err(MtlsSharedOwnerError::NotStarted),
            };
        };
        self.audit_shared_owner_snapshot(owner)
    }

    #[allow(clippy::similar_names)]
    async fn start_shared_owner_inner(
        self: &Arc<Self>,
    ) -> Result<SharedOwner, MtlsSharedOwnerError> {
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
        let task_f_listener = leg_f_listener.try_clone().map_err(|source| {
            MtlsSharedOwnerError::ListenerLocalAddr { leg: InterceptLeg::F, source }
        })?;
        let task_c_listener = leg_c_listener.try_clone().map_err(|source| {
            MtlsSharedOwnerError::ListenerLocalAddr { leg: InterceptLeg::C, source }
        })?;
        let stop = Arc::new(AtomicBool::new(false));
        let tasks = Arc::new(SharedListenerTaskOwner::new(
            shared_listener_task(
                task_f_listener,
                Arc::clone(&stop),
                Arc::downgrade(self),
                InterceptLeg::F,
            ),
            shared_listener_task(
                task_c_listener,
                Arc::clone(&stop),
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
        if let Err(source) = self.audit_shared_owner_snapshot(&owner) {
            owner.stop.store(true, Ordering::SeqCst);
            shutdown_shared_listener_tasks(Arc::clone(&owner.tasks)).await;
            drop(owner);
            return Err(source);
        }
        Ok(owner)
    }

    fn audit_shared_owner_snapshot(&self, owner: &SharedOwner) -> Result<(), MtlsSharedOwnerError> {
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
        if !owner.tasks.is_live(InterceptLeg::F) {
            return Err(MtlsSharedOwnerError::TaskReturned { leg: InterceptLeg::F });
        }
        if !owner.tasks.is_live(InterceptLeg::C) {
            return Err(MtlsSharedOwnerError::TaskReturned { leg: InterceptLeg::C });
        }
        // The host adapter's boot observation is intentionally strict about a
        // zero dynamic-element complement. Once an allocation is live, its
        // exact per-allocation rules/elements are owned by the capability
        // registry and the worker's audit boundary covers listener/task/socket
        // ownership; re-running the boot-only empty-complement observation
        // would misclassify healthy allocation state as a node-owner failure.
        if self.capabilities.has_live_records() {
            return Ok(());
        }
        let observed = self
            .intercept
            .observe_shared()
            .map_err(|source| MtlsSharedOwnerError::Intercept { source })?;
        if observed != Some(owner.expected.clone()) {
            return Err(MtlsSharedOwnerError::Intercept {
                source: InterceptError::PostconditionMismatch {
                    expected: owner.expected.clone(),
                    observed,
                },
            });
        }
        Ok(())
    }

    #[allow(clippy::unused_async, reason = "exact-port recovery retains the async owner boundary")]
    fn converge_shared_owner_inner(
        self: &Arc<Self>,
        owner: &mut SharedOwner,
    ) -> Result<(), MtlsSharedOwnerError> {
        let observed = self
            .intercept
            .observe_shared()
            .map_err(|source| MtlsSharedOwnerError::Intercept { source })?;
        if observed != Some(owner.expected.clone()) {
            return Err(MtlsSharedOwnerError::Intercept {
                source: InterceptError::PostconditionMismatch {
                    expected: owner.expected.clone(),
                    observed,
                },
            });
        }
        let Some(dead_leg) = owner.tasks.dead_leg() else {
            return self.audit_shared_owner_snapshot(owner);
        };
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
        let task_listener = listener
            .try_clone()
            .map_err(|source| MtlsSharedOwnerError::ListenerLocalAddr { leg: dead_leg, source })?;
        *listener_slot = Some(listener);
        owner.tasks.replace_terminal(
            dead_leg,
            shared_listener_task(
                task_listener,
                Arc::clone(&owner.stop),
                Arc::downgrade(self),
                dead_leg,
            ),
        )?;
        self.audit_shared_owner_snapshot(owner)
    }

    /// Install the per-alloc intercept and start the accept→`enforce`
    /// tasks. Fired from the action-shim's `on_alloc_running` site for every
    /// networked VM allocation. A Cloud Hypervisor VM terminates TCP inside
    /// the guest, so its traffic reaches the host-side interception boundary
    /// through the TAP-fed veth selected by its persisted canonical guest
    /// address. The intercept therefore installs before guest execution is
    /// released and does not rely on cgroup socket visibility.
    ///
    /// Idempotent: a re-fire for an alloc already intercepted (a Restart
    /// reusing the same alloc id) tears the prior intercept down first.
    ///
    /// **Fail-closed (D-MTLS-18, amends D-MTLS-17 item 4).** The per-alloc
    /// install is a security control, NOT a best-effort observability hook:
    /// an alloc whose intercept cannot be installed MUST NOT run with
    /// cleartext egress/ingress. On any of the three install-step failures
    /// (OUTBOUND egress nft-TPROXY install; leg-F bind; leg-C transparent
    /// listener) `start_alloc` returns the typed
    /// [`MtlsInterceptInstallError`] — surfacing the cause the worker
    /// previously discarded — and the action-shim drives the alloc to
    /// terminal `Failed`. The `ProbeRunner::start_alloc` fire-and-forget
    /// `()` contract does NOT transfer: a probe failure is itself an
    /// observation the reconciler consumes; an mTLS-install failure produces
    /// no such feedback loop, so "log and continue" would silently leave the
    /// confidentiality guarantee broken. The INBOUND nft-TPROXY rule install
    /// (one rule per declared Service port, D-A1 / GH #241) is itself a
    /// fail-closed site — an install error short-circuits via the `Inbound`
    /// variant, dropping every guard acquired this call.
    ///
    /// **Partial-teardown on the `Err` path.** Every guard acquired before
    /// the failing step (the OUTBOUND [`InterceptGuard`], the leg-F /
    /// leg-C listeners) is still a LOCAL at each failure point — it has not
    /// yet been handed to `spawn_legs_and_record`, so `stop_alloc` cannot find
    /// it in `self.intercepts`. Returning `Err` before recording drops those
    /// locals, and their `Drop` removes the egress nft rule / closes the
    /// listeners. The worker leaks NO half-installed intercept.
    ///
    /// # Errors
    ///
    /// [`MtlsInterceptInstallError::OutboundTproxyInstall`] (site 1),
    /// [`MtlsInterceptInstallError::LegFBind`] (site 2), or
    /// [`MtlsInterceptInstallError::Inbound`] (site 3 — the leg-C transparent
    /// listener bind OR any per-port inbound nft-TPROXY rule install, D-A1 /
    /// GH #241) when the corresponding install step fails. Additionally
    /// [`MtlsInterceptInstallError::LegFLocalAddr`] (site 2) /
    /// [`MtlsInterceptInstallError::LegCLocalAddr`] (site 3) when a listener
    /// binds but its bound-address capture (`local_addr()` / getsockname) fails:
    /// the install fails CLOSED rather than defaulting the redirect target to a
    /// broken port 0 (D-MTLS-18). Each source `Display` names the privilege /
    /// kernel-feature / shared-routing-infra remediation an operator acts on.
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

        if spec.network.is_some() {
            return self.start_shared_allocation(spec).await;
        }

        // The agent's leg-F (outbound, workload-facing plaintext) listener
        // — agent-chosen ephemeral loopback (D-MTLS-15). Leg F MUST be
        // `IP_TRANSPARENT`: the OUTBOUND egress rule the matching
        // `install_outbound_tproxy` appends is a NON-REWRITING
        // `tproxy to 127.0.0.1:<legF>` divert, so the kernel delivers the
        // workload's SYN with its ORIGINAL destination address intact (NOT
        // rewritten to leg-F's bound addr). A plain (non-transparent) socket
        // bound to `127.0.0.1:<legF>` cannot receive a SYN whose dst is the
        // orig-dst — the divert is refused and the workload sees
        // ConnectionRefused, breaking the Path-A outbound capture. The
        // transparent socket is ALSO what makes the per-flow `getsockname`
        // orig-dst recovery work (`accept_outbound_and_recover_orig_dst`):
        // under TPROXY the recovered orig-dst IS the accepted socket's local
        // addr, which is only the dialed dst on a transparent socket. This
        // mirrors the leg-C transparent bind below EXACTLY — leg-F and leg-C
        // are symmetric TPROXY-divert targets, not asymmetric. Bound FIRST so
        // its ephemeral port is the redirect target the OUTBOUND nft-TPROXY
        // rule points at.
        // Fail-closed (D-MTLS-18 site 2): on bind failure, return `Err`;
        // nothing is acquired yet, so there is nothing to tear down.
        let leg_f_listener = match self
            .intercept
            .bind_transparent(SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0))
        {
            Ok(l) => l,
            Err(source) => return Err(MtlsInterceptInstallError::leg_f_bind(source)),
        };
        // The agent's chosen leg-F address — the kernel-redirect TARGET the
        // OUTBOUND nft-TPROXY egress rule redirects the workload's egress to.
        // Load-bearing: it is the `agent_leg_f_port` the egress rule points at
        // (`install_outbound_tproxy(host_veth, leg_f_addr.port())` below). It is
        // NOT a dial target — the dial peer is the per-connection RESOLVED
        // backend addr (04-02), recovered in the accept loop, never this slot.
        // Fail-closed (D-MTLS-18 site 2): a `local_addr()` getsockname error
        // surfaces as the typed `LegFLocalAddr` rather than defaulting to a
        // broken port-0 redirect target. `leg_f_listener` (the only guard
        // acquired so far) drops on the `?` early return → closes.
        let _leg_f_addr = project_listener_v4(
            leg_f_listener.local_addr(),
            MtlsInterceptInstallError::leg_f_local_addr,
        )?;

        // OUTBOUND install (D-TME-4 / ADR-0071 Path A, site 1): append the
        // per-veth egress nft-TPROXY rule matching the workload's host-side
        // veth (`iifname spec.host_veth`) and redirecting ALL its egress TCP
        // to leg F. The host-veth NAME arrives per-alloc on
        // `AllocationSpec.host_veth` (JOIN-6), set by the action-shim C3
        // provision seam; `None` off the mTLS-composed boot (a fixture with no
        // provisioned veth), where the install is SKIPPED rather than matching
        // a bogus interface.
        // Fail-closed (D-MTLS-18 site 1): on install failure return `Err`;
        // `leg_f_listener` (the only guard acquired so far) drops here → close.
        // `None` host-veth (off the mTLS-composed boot gate) SKIPS the install
        // (no interface to match) but still stands up the leg-F listener +
        // accept loop — a fixture that drives leg-F directly exercises the
        // accept path without the kernel redirect.
        let outbound_tproxy_guard = None;

        // INBOUND install: the agent's leg-C IP_TRANSPARENT listener. The
        // accompanying per-port nft-TPROXY redirect rules that aim real client
        // traffic at this listener are installed below (D-A1 / GH #241), one per
        // declared Service port, tproxy-redirected to this listener's bound port.
        // Fail-closed (D-MTLS-18 site 3): a server workload with no leg-C
        // inbound listener accepts cleartext client connections — a
        // confidentiality breach symmetric to the outbound one. Return `Err`
        // (the inbound carve-out is REJECTED per D-MTLS-18 P2);
        // `outbound_tproxy_guard` + `leg_f_listener` (the guards acquired so
        // far) drop here → remove the egress rule / close the leg-F listener.
        let inbound_listener = match self
            .intercept
            .bind_transparent(SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0))
        {
            Ok(l) => l,
            Err(source) => return Err(MtlsInterceptInstallError::Inbound(source)),
        };
        // Capture leg-C's bound addr BEFORE the listener moves into the spawned
        // inbound `accept_loop` — mirroring the leg-F capture pattern above
        // (:378-382; leg-F's addr is an inline local consumed inline, with no
        // public accessor). Retained on `AllocIntercept` so `leg_c_addr(&self,
        // alloc)` stays a pure in-memory read (the listener is consumed by the
        // accept task; its `local_addr()` is no longer reachable from the
        // worker). It is the EXACT addr the spawned inbound accept loop accepts
        // on, so the per-port inbound rules installed below (D-TME-13) redirect
        // to it and land on the production inbound leg. Mirroring leg-F, the
        // per-port `install_inbound_tproxy` loop below reads this inline
        // `leg_c_addr` local for its tproxy-to target, NOT `self.leg_c_addr(alloc)`.
        // Fail-closed (D-MTLS-18 site 3): a `local_addr()` getsockname error
        // surfaces as the typed `LegCLocalAddr` rather than recording a port-0
        // leg-C addr that would silently corrupt the #241 inbound-redirect read.
        // `outbound_tproxy_guard` + `leg_f_listener` (the guards acquired so far)
        // drop on the `?` early return → remove the egress rule / close leg-F.
        let leg_c_addr = project_listener_v4(
            inbound_listener.local_addr(),
            MtlsInterceptInstallError::leg_c_local_addr,
        )?;

        // INBOUND nft-TPROXY rule install (D-A1, GH #241 — the keystone that
        // closes the prior `tproxy_guard = None` deferral). For each declared
        // Service listener port, append ONE per-virt rule keyed
        // `ip daddr <workload_addr> tcp dport <service_port>` that
        // tproxy-redirects the matched inbound connection to the agent's leg-C
        // `IP_TRANSPARENT` listener (`leg_c_addr.port()`, the ephemeral redirect
        // TARGET — NOT the match key). The match `dport` is the DECLARED service
        // port (D-BLOCKER1 / D-TME-10 one-source/two-readers — the SAME value
        // `service_backends` advertises and the egress `MtlsResolve` keys on),
        // never the ephemeral leg-C port (which would be the inert
        // self-referential shape matching no real inbound connection). N declared
        // ports → N rules; `None` `workload_addr` or empty `service_ports` →
        // ZERO rules (the host-netns / Job path, unchanged). Each returned guard
        // is retained on `AllocIntercept` for the alloc lifetime; its `Drop`
        // removes exactly that rule by handle on `stop_alloc`. Fail-closed: an
        // install error short-circuits via `?` (the `Inbound` variant's
        // `#[from] InterceptError`), dropping the guards acquired so far +
        // `outbound_tproxy_guard` + `leg_f_listener` → remove every rule installed
        // this call. (The OUTBOUND direction resolves orig_dst per-connection via
        // the `MtlsResolve` consumer wired in the accept loop below — see
        // [`Self::handle_outbound`].)
        let mut inbound_tproxy_guards = Vec::new();
        if let Some(workload_addr) = spec.network.as_ref().map(|network| network.address) {
            for port in &spec.service_ports {
                let virt = SocketAddrV4::new(workload_addr, port.get());
                inbound_tproxy_guards
                    .push(self.intercept.install_inbound(virt, leg_c_addr.port())?);
            }
        }

        self.spawn_legs_and_record(
            spec,
            outbound_tproxy_guard,
            inbound_tproxy_guards,
            leg_f_listener,
            inbound_listener,
            leg_c_addr,
        );
        Ok(())
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
            pending
        };
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
            pending.activate()
        };
        if activation == ActivationDisposition::Retired {
            return Err(MtlsInterceptInstallError::RegistrationRetired {
                alloc_id: spec.alloc.clone(),
            });
        }
        self.pending_allocations.lock().remove(&spec.alloc);
        self.intercepts.lock().insert(
            spec.alloc.clone(),
            AllocIntercept {
                capability_owned: true,
                _outbound_tproxy_guard: None,
                _inbound_tproxy_guards: Vec::new(),
                leg_c_addr,
                stop: Arc::new(AtomicBool::new(false)),
                enforced: EnforcedSet::new(),
                tasks: AllocationTaskOwner::new(),
            },
        );
        Ok(())
    }

    /// Spawn the outbound + inbound accept loops for an alloc and record the
    /// full intercept bookkeeping. Factored out of [`start_alloc`] so that
    /// method stays under the small-function budget; this owns the shared
    /// per-alloc state (`enforced` teardown set, cooperative `stop` flag) the
    /// two legs and the recorded intercept share.
    fn spawn_legs_and_record(
        self: &Arc<Self>,
        spec: &AllocationSpec,
        outbound_tproxy_guard: Option<Box<dyn InterceptGuard>>,
        inbound_tproxy_guards: Vec<Box<dyn InterceptGuard>>,
        leg_f_listener: std::net::TcpListener,
        inbound_listener: std::net::TcpListener,
        leg_c_addr: SocketAddrV4,
    ) {
        let enforced = EnforcedSet::new();
        let tasks = AllocationTaskOwner::new();
        // Cooperative stop flag the accept loops observe between poll slices.
        let stop = Arc::new(AtomicBool::new(false));

        self.spawn_accept_loop(
            spec.alloc.clone(),
            AcceptLeg::Outbound { listener: leg_f_listener },
            enforced.clone(),
            Arc::clone(&stop),
            &tasks,
        );
        self.spawn_accept_loop(
            spec.alloc.clone(),
            AcceptLeg::Inbound { listener: inbound_listener },
            enforced.clone(),
            Arc::clone(&stop),
            &tasks,
        );

        self.record_intercept_full(
            spec.alloc.clone(),
            outbound_tproxy_guard,
            inbound_tproxy_guards,
            leg_c_addr,
            enforced,
            stop,
            tasks,
        );
    }

    /// Tear the alloc's intercept down. Drains the per-connection
    /// teardown set through `enforcement.teardown`, signals the accept
    /// tasks, and drops the cgroup link + TPROXY guard (their `Drop`
    /// detaches the program / removes the nft rule). Idempotent — a
    /// stop for an unknown alloc is a no-op.
    pub async fn stop_alloc(
        self: &Arc<Self>,
        alloc_id: &AllocationId,
    ) -> Result<(), MtlsInterceptStopError> {
        match self.begin_stop_alloc(alloc_id) {
            Some(stop) => stop.wait().await,
            None => Ok(()),
        }
    }

    fn begin_stop_alloc(self: &Arc<Self>, alloc_id: &AllocationId) -> Option<Arc<AllocStop>> {
        let lifecycle = self.lifecycle.read();
        let intercept = self.intercepts.lock().remove(alloc_id);
        let Some(intercept) = intercept else {
            if self.pending_allocations.lock().contains(alloc_id) {
                return self.begin_pending_stop(alloc_id);
            }
            let previous =
                self.stopping.lock().get(alloc_id).and_then(|stops| stops.last()).cloned();
            let previous = previous?;
            let retry_handles = previous.retry_handles.lock().drain(..).collect::<Vec<_>>();
            if retry_handles.is_empty() {
                return Some(previous);
            }
            let retry = Arc::new(AllocStop::new());
            self.stopping.lock().entry(alloc_id.clone()).or_default().push(Arc::clone(&retry));
            let retry_drain = previous.retry_drain.lock().take();
            if let Some(drain) = retry_drain {
                start_capability_drain_retry(
                    &retry,
                    Arc::clone(&self.enforcement),
                    alloc_id.clone(),
                    drain,
                    retry_handles,
                );
            } else {
                start_handle_teardown(
                    &retry,
                    Arc::clone(&self.enforcement),
                    alloc_id.clone(),
                    retry_handles,
                );
            }
            return Some(retry);
        };
        #[cfg(any(test, feature = "integration-tests"))]
        self.stop_alloc_calls.fetch_add(1, Ordering::SeqCst);
        let stop = Arc::new(AllocStop::new());
        self.stopping.lock().entry(alloc_id.clone()).or_default().push(Arc::clone(&stop));
        let enforcement = Arc::clone(&self.enforcement);
        let capabilities = self.capabilities.clone();
        let alloc_id = alloc_id.clone();
        let stop_for_work = Arc::clone(&stop);
        stop.fence.start_with(move || async move {
            let AllocIntercept {
                capability_owned,
                _outbound_tproxy_guard: outbound_tproxy_guard,
                _inbound_tproxy_guards: inbound_tproxy_guards,
                leg_c_addr: _,
                stop,
                enforced,
                tasks,
            } = intercept;
            stop.store(true, Ordering::SeqCst);
            // Dropping rule guards and listener-owning task futures closes the
            // admission boundary before any connection teardown is awaited.
            drop(outbound_tproxy_guard);
            drop(inbound_tproxy_guards);
            tasks.abort_and_join().await;
            if capability_owned && let Some(retirement) = capabilities.begin_retire(&alloc_id) {
                let mut drain = retirement.wait_for_claims().await;
                let handles = std::mem::take(&mut drain.handles);
                let mut failures = Vec::new();
                let mut retry_handles = Vec::new();
                for handle in handles {
                    let id = handle.id().clone();
                    let retry_handle = handle.clone();
                    if let Err(source) = enforcement.teardown(handle).await {
                        failures.push(format!("{id}: {source}"));
                        retry_handles.push(retry_handle);
                    }
                }
                *stop_for_work.retry_handles.lock() = retry_handles;
                if failures.is_empty() {
                    drop(drain.take_elements());
                    drain.complete();
                } else {
                    *stop_for_work.retry_drain.lock() = Some(drain);
                }
                *stop_for_work.result.lock() = Some(if failures.is_empty() {
                    Ok(())
                } else {
                    Err(MtlsInterceptStopError { alloc_id, failures })
                });
                return;
            }
            finish_handle_teardown(&stop_for_work, enforcement, alloc_id, enforced.drain()).await;
        });
        drop(lifecycle);
        Some(stop)
    }

    /// Transfer retirement ownership for a shared registration that is still
    /// acquiring its per-allocation effects.  The registry keeps the address
    /// reservation and pending-owner bit until the installer either activates
    /// or drops its `PendingCapability`; the stop owner therefore cannot return
    /// before the exact partial-effect set has been drained.
    #[allow(clippy::significant_drop_in_scrutinee)]
    fn begin_pending_stop(self: &Arc<Self>, alloc_id: &AllocationId) -> Option<Arc<AllocStop>> {
        if let Some(previous) =
            self.stopping.lock().get(alloc_id).and_then(|stops| stops.last()).cloned()
        {
            return Some(previous);
        }
        let retirement = self.capabilities.begin_retire(alloc_id)?;
        self.pending_allocations.lock().remove(alloc_id);
        let stop = Arc::new(AllocStop::new());
        self.stopping.lock().entry(alloc_id.clone()).or_default().push(Arc::clone(&stop));
        let enforcement = Arc::clone(&self.enforcement);
        let alloc_id = alloc_id.clone();
        let stop_for_work = Arc::clone(&stop);
        stop.fence.start_with(move || async move {
            let mut drain = retirement.wait_for_claims().await;
            // Give the activation caller a bounded handoff window to return
            // `RegistrationRetired` and let its action-shim owner quiesce the
            // driver before this retirement owner tears down the acquired
            // mTLS elements. No clock, retry, or external control-plane hook
            // is introduced; this is only executor scheduling at the existing
            // Pending-owner linearization.
            for _ in 0..32 {
                tokio::task::yield_now().await;
            }
            let handles = drain.take_handles();
            let mut failures = Vec::new();
            for handle in handles {
                let id = handle.id().clone();
                if let Err(source) = enforcement.teardown(handle).await {
                    failures.push(format!("{id}: {source}"));
                }
            }
            drop(drain.take_elements());
            drain.complete();
            *stop_for_work.result.lock() = Some(if failures.is_empty() {
                Ok(())
            } else {
                Err(MtlsInterceptStopError { alloc_id, failures })
            });
        });
        Some(stop)
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

    /// Inject one typed full-owner shutdown failure at the worker boundary.
    /// The one-shot owner retains that failure for every later caller. This is
    /// used only to prove exact outer-server propagation without a retry API.
    #[doc(hidden)]
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn inject_owner_shutdown_failure_for_test(&self) {
        self.owner_shutdown_failures.fetch_add(1, Ordering::SeqCst);
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
        })
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
                let active = std::mem::take(&mut *owner.intercepts.lock());
                let in_progress = owner
                    .stopping
                    .lock()
                    .values()
                    .filter_map(|stops| stops.last().cloned())
                    .collect::<Vec<_>>();
                let pending = owner.pending_allocations.lock().iter().cloned().collect::<Vec<_>>();
                (active, pending, in_progress, shared)
            };

            let mut failures = Vec::new();
            let pending_stops = pending
                .iter()
                .filter_map(|alloc_id| owner.begin_pending_stop(alloc_id))
                .collect::<Vec<_>>();
            if let Some(mut shared) = shared {
                shared.stop.store(true, Ordering::SeqCst);
                shutdown_shared_listener_tasks(Arc::clone(&shared.tasks)).await;
                drop(shared.leg_f_listener.take());
                drop(shared.leg_c_listener.take());
                if let Some(guard) = shared.guard.take() {
                    // The sealed owner path intentionally retains the
                    // constant empty program for next-boot revalidation.
                    std::mem::forget(guard);
                }
            }
            for (alloc_id, intercept) in active {
                let AllocIntercept {
                    capability_owned,
                    _outbound_tproxy_guard: outbound_tproxy_guard,
                    _inbound_tproxy_guards: inbound_tproxy_guards,
                    leg_c_addr: _,
                    stop,
                    enforced,
                    tasks,
                } = intercept;
                stop.store(true, Ordering::SeqCst);

                // The process-owner boundary closes listener/task userspace,
                // but the original kernel rules must remain. Relinquishing
                // these boxes is the private, sealed equivalent of abrupt
                // process loss; normal allocation stop continues to drop them.
                if let Some(guard) = outbound_tproxy_guard {
                    std::mem::forget(guard);
                }
                for guard in inbound_tproxy_guards {
                    std::mem::forget(guard);
                }

                tasks.abort_and_join().await;
                if capability_owned
                    && let Some(retirement) = owner.capabilities.begin_retire(&alloc_id)
                {
                    let mut drain = retirement.wait_for_claims().await;
                    let handles = drain.take_handles();
                    let mut teardown_failures = Vec::new();
                    for handle in handles {
                        let id = handle.id().clone();
                        if let Err(source) = owner.enforcement.teardown(handle).await {
                            teardown_failures.push(format!("{id}: {source}"));
                        }
                    }
                    drop(drain.take_elements());
                    drain.complete();
                    if !teardown_failures.is_empty() {
                        failures
                            .push(MtlsInterceptStopError { alloc_id, failures: teardown_failures });
                    }
                    continue;
                }
                let stop = Arc::new(AllocStop::new());
                start_handle_teardown(
                    &stop,
                    Arc::clone(&owner.enforcement),
                    alloc_id,
                    enforced.drain(),
                );
                if let Err(source) = stop.wait().await {
                    failures.push(source);
                }
            }
            for stop in in_progress {
                if let Err(source) = stop.wait().await {
                    failures.push(source);
                }
            }
            for stop in pending_stops {
                if let Err(source) = stop.wait().await {
                    failures.push(source);
                }
            }
            #[cfg(any(test, feature = "integration-tests"))]
            if owner
                .owner_shutdown_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                failures.push(MtlsInterceptStopError {
                    alloc_id: AllocationId::new("injected-owner-shutdown")
                        .unwrap_or_else(|_| unreachable!("static allocation id is valid")),
                    failures: vec!["injected outer-boundary teardown failure".to_owned()],
                });
            }
            *attempt_for_work.result.lock() = Some(if failures.is_empty() {
                Ok(())
            } else {
                Err(MtlsInterceptOwnerShutdownError { failures })
            });
        });
        attempt
    }

    /// Spawn the accept→`enforce` loop for one leg. Each accepted
    /// connection is built into an `InterceptedConnection`, `enforce`d,
    /// and its handle pushed into the alloc's teardown set.
    fn spawn_accept_loop(
        self: &Arc<Self>,
        alloc: AllocationId,
        leg: AcceptLeg,
        enforced: EnforcedSet,
        stop: Arc<AtomicBool>,
        tasks: &AllocationTaskOwner,
    ) {
        // A blocked accept loop must not retain the worker forever. AppState is
        // the worker's owner; using Weak here lets a control-plane shutdown
        // drop the worker, its intercept guards, and its store-bearing ports.
        // The loop notices owner loss within one bounded poll slice.
        let worker = Arc::downgrade(self);
        let tasks_for_children = tasks.clone();
        let _registered = tasks.spawn(|| {
            tokio::task::spawn_blocking(move || {
                // The closure OWNS `alloc`/`leg`/`enforced`/`stop`; `accept_loop`
                // borrows them for the duration of the loop (it clones `alloc`
                // per connection and re-uses `leg`/`enforced`/`stop` by reference).
                Self::accept_loop(&worker, &alloc, &leg, &enforced, &stop, &tasks_for_children);
            })
        });
    }

    /// Blocking accept loop (the leg listeners are blocking
    /// `std::net::TcpListener`s — leg acquisition is a one-shot per
    /// intercepted connection, not an async pump). Exits when `stop` is set
    /// (observed between bounded poll slices) so the loop does not outlive the
    /// alloc on a `spawn_blocking` thread.
    ///
    /// The OUTBOUND leg drives the per-connection enrollment resolve (04-02):
    /// accept leg-F → recover `orig_dst` via `getsockname` → `MtlsResolve` →
    /// branch on the [`MtlsResolution`] variant ([`Self::handle_outbound`]).
    /// The INBOUND leg builds the `InterceptedConnection` from the
    /// TPROXY-recovered orig-dst and hands it to `enforce` directly (its routing
    /// fact needs no resolve — the server SVID is selected by the orig-dst).
    fn accept_loop(
        worker: &Weak<Self>,
        alloc: &AllocationId,
        leg: &AcceptLeg,
        enforced: &EnforcedSet,
        stop: &Arc<AtomicBool>,
        tasks: &AllocationTaskOwner,
    ) {
        loop {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            match leg {
                AcceptLeg::Outbound { listener } => {
                    // Poll for a pending connection (observing `stop`) before the
                    // blocking accept, so the loop exits cooperatively on teardown.
                    match await_pending_connection(listener, stop, worker) {
                        ConnectionReady::Pending => {}
                        ConnectionReady::ListenerClosed | ConnectionReady::Stopped => return,
                    }
                    let Some(worker) = worker.upgrade() else {
                        return;
                    };
                    // Accept leg-F + recover the dialed orig_dst, then run the
                    // per-connection resolve consumer. A closed listener (alloc
                    // torn down) exits the loop; any other leg-acquire fault skips
                    // this connection.
                    match accept_outbound_and_recover_orig_dst(listener) {
                        Ok((leg_f, orig_dst)) => {
                            worker.handle_outbound(alloc, leg_f, orig_dst, enforced, tasks);
                        }
                        Err(InterceptError::Accept { .. }) => return,
                        Err(source) => {
                            tracing::warn!(
                                name: "health.mtls.leg_acquire_failed",
                                alloc = %alloc,
                                error = %source,
                                "mTLS leg-F acquire failed; skipping this connection"
                            );
                        }
                    }
                }
                AcceptLeg::Inbound { listener } => {
                    // Poll for a pending connection (observing `stop`) before
                    // the blocking `accept()` inside `accept_inbound_leg`, so
                    // the inbound loop can also exit cooperatively on teardown
                    // rather than block on a stale listener fd forever.
                    match await_pending_connection(listener, stop, worker) {
                        ConnectionReady::Pending => {}
                        ConnectionReady::ListenerClosed | ConnectionReady::Stopped => return,
                    }
                    let Some(worker) = worker.upgrade() else {
                        return;
                    };
                    match accept_inbound_leg(listener, alloc.clone()) {
                        Ok(conn) => worker.spawn_enforce(alloc, conn, enforced, tasks),
                        Err(InterceptError::Accept { .. }) => return,
                        Err(source) => {
                            tracing::warn!(
                                name: "health.mtls.leg_acquire_failed",
                                alloc = %alloc,
                                error = %source,
                                "mTLS leg-C acquire failed; skipping this connection"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Per-connection OUTBOUND resolve consumer (04-02, ADR-0071 fact 4 / C1).
    ///
    /// Resolves the captured connection's recovered `orig_dst` against the mesh
    /// through the injected [`MtlsResolve`] port and acts on the
    /// [`MtlsResolution`] variant — the 3-arm decision IS the variant, never
    /// inferred from a sentinel:
    /// - [`Mesh(backend)`](MtlsResolution::Mesh) → build
    ///   `InterceptedConnection { routed: Outbound { peer: backend.addr } }`
    ///   (`expected_peer` stays `None` until #242 — v1 authn-only) and hand it
    ///   to `enforce` (mTLS to the resolved backend). The peer is the RESOLVED
    ///   backend addr, NOT `orig_dst` (v1 headless: they coincide, but the
    ///   worker uses the resolved addr so #167/#61 wires here unchanged).
    /// - [`NonMesh`](MtlsResolution::NonMesh) → cleartext pass-through, by
    ///   design: the workload dialed a non-mesh dst, so the agent relays leg-F
    ///   to a cleartext dial of `orig_dst` ([`spawn_cleartext_passthrough`]).
    ///   NO mTLS, NO `enforce` call.
    /// - [`MeshUnreachable`](MtlsResolution::MeshUnreachable) → FAIL-CLOSED:
    ///   `orig_dst` should be a mesh peer but cannot be reached/validated, so
    ///   the agent REFUSES — drops leg-F (closing the workload's connection),
    ///   NO cleartext, NO dial. This is the silent-cleartext footgun the
    ///   enrollment model exists to remove.
    ///
    /// A store-layer resolve `Err` (poisoned handle / corrupt table — NOT a
    /// per-connection classification) is treated fail-closed: the leg is
    /// dropped, no cleartext (a resolve the agent cannot trust must never
    /// degrade to silent cleartext).
    fn handle_outbound(
        self: &Arc<Self>,
        alloc: &AllocationId,
        leg_f: std::os::fd::OwnedFd,
        orig_dst: SocketAddrV4,
        enforced: &EnforcedSet,
        tasks: &AllocationTaskOwner,
    ) {
        // The resolve port is async; this loop runs on a `spawn_blocking`
        // thread (a blocking-pool thread, not a runtime worker), so
        // `Handle::block_on` is valid here — it drives the resolve future to
        // completion before the 3-arm decision.
        let runtime = tokio::runtime::Handle::current();
        let resolve = Arc::clone(&self.resolve);
        let resolution = match runtime.block_on(resolve.resolve(orig_dst)) {
            Ok(resolution) => resolution,
            Err(source) => {
                // A store-layer fault is NOT a per-connection classification —
                // but the agent cannot trust the resolve, so it must FAIL CLOSED
                // (drop leg-F, no cleartext) rather than guess.
                tracing::warn!(
                    name: "health.mtls.resolve_failed",
                    alloc = %alloc,
                    orig_dst = %orig_dst,
                    error = %source,
                    "mTLS resolve faulted; dropping leg-F fail-closed (no cleartext)"
                );
                drop(leg_f);
                return;
            }
        };

        match decide_outbound(&resolution) {
            OutboundAction::Enforce { peer } => {
                // Mesh → enforce mTLS to the RESOLVED backend addr.
                let conn = InterceptedConnection {
                    leg: leg_f,
                    routed: Routed::Outbound { peer },
                    alloc: alloc.clone(),
                    // v1 authn-only (F5 / #242): the expected-peer SAN-match is
                    // supplied downstream by east-west SPIFFE-ID resolution.
                    expected_peer: None,
                };
                self.spawn_enforce(alloc, conn, enforced, tasks);
            }
            OutboundAction::PassThrough => {
                // NonMesh → cleartext pass-through, by design: relay leg-F to a
                // cleartext dial of orig_dst. NO mTLS, NO enforce.
                let _registered = tasks.spawn(|| {
                    spawn_cleartext_passthrough(&runtime, alloc.clone(), leg_f, orig_dst)
                });
            }
            OutboundAction::FailClosed => {
                // MeshUnreachable → REFUSE: drop leg-F, NO cleartext, NO dial.
                tracing::warn!(
                    name: "health.mtls.outbound_fail_closed",
                    alloc = %alloc,
                    orig_dst = %orig_dst,
                    "leg-F connection refused fail-closed (orig_dst should be a mesh peer but \
                     is unreachable/invalid; no cleartext)"
                );
                drop(leg_f);
            }
        }
    }

    /// Dispatch one connection accepted by the node-shared leg-F listener.
    /// The source address is claimed before the asynchronous resolve/enforce
    /// work starts, so the immutable generation stays attached to the
    /// connection even if the address is retired and reused meanwhile.
    fn handle_shared_outbound(
        self: &Arc<Self>,
        source_addr: Ipv4Addr,
        leg_f: std::os::fd::OwnedFd,
        orig_dst: SocketAddrV4,
    ) {
        let Some(claim) = self.capabilities.claim_source(source_addr) else {
            drop(leg_f);
            return;
        };
        let runtime = tokio::runtime::Handle::current();
        let resolution = match runtime.block_on(self.resolve.resolve(orig_dst)) {
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
                drop(claim);
                drop(spawn_cleartext_passthrough(&runtime, alloc, leg_f, orig_dst));
            }
            OutboundAction::FailClosed => {
                drop(claim);
                drop(leg_f);
            }
        }
    }

    /// Dispatch one connection accepted by the node-shared leg-C listener.
    /// The recovered destination address and port select the exact active
    /// capability; an unknown address or disallowed port is closed without
    /// entering enforcement.
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

    /// Hold the exact capability claim across the awaited production
    /// enforcement call. A late returned handle is torn down immediately when
    /// retirement won the publication race; it is never inserted into a
    /// successor generation's drain set.
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

    /// Hand an [`InterceptedConnection`] to `enforce` on the tokio runtime.
    /// `enforce` is the single fail-closed gate; on `Ok` its handle joins the
    /// alloc's teardown set, on `Err` the port has already closed the leg and no
    /// cleartext egressed.
    fn spawn_enforce(
        self: &Arc<Self>,
        alloc: &AllocationId,
        conn: InterceptedConnection,
        enforced: &EnforcedSet,
        tasks: &AllocationTaskOwner,
    ) {
        let enforcement = Arc::clone(&self.enforcement);
        let enforced = enforced.clone();
        let alloc_for_log = alloc.clone();
        let handle = tokio::runtime::Handle::current();
        let _registered = tasks.spawn(|| {
            handle.spawn(async move {
                match enforcement.enforce(conn).await {
                    Ok(handle) => enforced.push(handle),
                    Err(source) => {
                        tracing::warn!(
                            name: "health.mtls.enforce_failed",
                            alloc = %alloc_for_log,
                            error = %source,
                            "mTLS enforce refused the connection (fail-closed; no cleartext)"
                        );
                    }
                }
            })
        });
    }

    /// Record a fully-installed (outbound + inbound) intercept.
    #[allow(
        clippy::too_many_arguments,
        reason = "private bookkeeping constructor: one arg per AllocIntercept field \
                  (the two tproxy guards, leg_c_addr per D-TME-13, tasks, enforced, \
                  stop); bundling them into a params struct would just move the same field \
                  list one indirection away with no clarity gain — the call site is the \
                  single internal caller in spawn_legs_and_record"
    )]
    fn record_intercept_full(
        &self,
        alloc: AllocationId,
        outbound_tproxy_guard: Option<Box<dyn InterceptGuard>>,
        inbound_tproxy_guards: Vec<Box<dyn InterceptGuard>>,
        leg_c_addr: SocketAddrV4,
        enforced: EnforcedSet,
        stop: Arc<AtomicBool>,
        tasks: AllocationTaskOwner,
    ) {
        self.intercepts.lock().insert(
            alloc,
            AllocIntercept {
                capability_owned: false,
                _outbound_tproxy_guard: outbound_tproxy_guard,
                _inbound_tproxy_guards: inbound_tproxy_guards,
                leg_c_addr,
                stop,
                enforced,
                tasks,
            },
        );
    }

    /// The ephemeral loopback address the live intercept's **leg-C** (the inbound,
    /// client-facing `IP_TRANSPARENT` listener) is bound to for `alloc`, or `None`
    /// when no intercept is currently installed for `alloc`.
    ///
    /// leg-C is the agent's inbound TPROXY-divert target: `start_alloc` binds it at
    /// a worker-chosen ephemeral `127.0.0.1:0` and spawns the inbound `accept_loop`
    /// over it. This accessor exposes that bound addr so a caller can observe WHERE
    /// the inbound intercept is listening — the diagnostic counterpart to the
    /// outbound leg-F port the egress nft-TPROXY rule already encodes
    /// (`install_outbound_tproxy(host_veth, leg_f_port)`).
    ///
    /// # `pub` legitimacy (operability, independent of #241)
    ///
    /// This is a production-legitimate diagnostic/observability surface in its own
    /// right: an operator/diagnostic caller can ask the worker "where is this
    /// alloc's inbound intercept listening?" — a genuine operability/analysability
    /// question for a security control that silently terminates client mTLS. That
    /// alone justifies `pub`; it is NOT a test-only hook. #241 (the production
    /// inbound-redirect install) is *expected* to reuse this read pending its
    /// install site/timing design — but whether #241 consumes `self.leg_c_addr(..)`
    /// or an inline `leg_c_addr` local in `start_alloc` (mirroring the leg-F
    /// capture pattern, which reads its port via the inline local
    /// `leg_f_addr.port()` and exposes no accessor) is #241's unresolved design.
    /// v1 does NOT depend on that question; the accessor stands on the operability
    /// ground above regardless. See D-TME-13 in `wave-decisions.md`.
    ///
    /// # Preconditions
    ///
    /// None. Any `AllocationId` is a valid query; an unknown alloc returns `None`.
    ///
    /// # Returns
    ///
    /// - `Some(addr)` — the bound leg-C `SocketAddrV4` (always `127.0.0.1:<ephemeral>`,
    ///   the addr `make_transparent_listener` bound in `start_alloc`) when a live
    ///   intercept exists for `alloc` (i.e. `start_alloc` succeeded and `stop_alloc`
    ///   has not since run for it).
    /// - `None` when no live intercept exists for `alloc` — never started, already
    ///   stopped, or an `alloc` this worker never intercepted.
    ///
    /// # Observable invariant
    ///
    /// For any `alloc`: `leg_c_addr(alloc).is_some()` ⇔ a live `AllocIntercept` is
    /// recorded for `alloc` in `self.intercepts`. The returned addr is stable for the
    /// life of that intercept (leg-C is bound once in `start_alloc` and never re-bound)
    /// and is the EXACT addr the spawned inbound `accept_loop` is accepting on — so a
    /// redirect installed at the returned addr lands on the production inbound leg.
    ///
    /// # Identity boundary (authn-only v1 — ADR-0071 / D-TME-8 / #242)
    ///
    /// This exposes ONLY a bound socket address — NO SVID, NO key, NO identity
    /// material of any kind. It is a bound-addr read, not an identity read. Workloads
    /// hold nothing and the worker exposes nothing about *who* leg-C will mTLS as; the
    /// expected-SVID / intended-peer join is strictly #242's (the
    /// `MtlsResolve.expected_svid` anti-corruption field, `None` in v1). The accessor
    /// is therefore inside the authn-only v1 boundary by construction.
    #[must_use]
    pub fn leg_c_addr(&self, alloc: &AllocationId) -> Option<SocketAddrV4> {
        self.intercepts.lock().get(alloc).map(|i| i.leg_c_addr)
    }
}

fn start_capability_drain_retry(
    stop: &Arc<AllocStop>,
    enforcement: Arc<dyn MtlsEnforcement>,
    alloc_id: AllocationId,
    mut drain: CapabilityDrain,
    handles: Vec<EnforcedConnection>,
) {
    let stop_for_work = Arc::clone(stop);
    stop.fence.start_with(move || async move {
        let mut failures = Vec::new();
        let mut retry_handles = Vec::new();
        for handle in handles {
            let id = handle.id().clone();
            let retry_handle = handle.clone();
            if let Err(source) = enforcement.teardown(handle).await {
                failures.push(format!("{id}: {source}"));
                retry_handles.push(retry_handle);
            }
        }
        *stop_for_work.retry_handles.lock() = retry_handles;
        if failures.is_empty() {
            drop(drain.take_elements());
            drain.complete();
        } else {
            *stop_for_work.retry_drain.lock() = Some(drain);
        }
        *stop_for_work.result.lock() = Some(if failures.is_empty() {
            Ok(())
        } else {
            Err(MtlsInterceptStopError { alloc_id, failures })
        });
    });
}

fn start_handle_teardown(
    stop: &Arc<AllocStop>,
    enforcement: Arc<dyn MtlsEnforcement>,
    alloc_id: AllocationId,
    handles: Vec<EnforcedConnection>,
) {
    let stop_for_work = Arc::clone(stop);
    stop.fence.start_with(move || async move {
        finish_handle_teardown(&stop_for_work, enforcement, alloc_id, handles).await;
    });
}

async fn finish_handle_teardown(
    stop: &Arc<AllocStop>,
    enforcement: Arc<dyn MtlsEnforcement>,
    alloc_id: AllocationId,
    handles: Vec<EnforcedConnection>,
) {
    let mut failures = Vec::new();
    let mut retry = Vec::new();
    for handle in handles {
        let id = handle.id().clone();
        if let Err(source) = enforcement.teardown(handle.clone()).await {
            failures.push(format!("{id}: {source}"));
            retry.push(handle);
        }
    }
    *stop.retry_handles.lock() = retry;
    *stop.result.lock() = Some(if failures.is_empty() {
        Ok(())
    } else {
        Err(MtlsInterceptStopError { alloc_id, failures })
    });
}

/// Which leg an accept loop is draining.
enum AcceptLeg {
    /// Outbound leg-F (workload-facing plaintext). The dialed orig-dst is
    /// recovered per-connection via `getsockname` on the accepted leg-F socket
    /// (`accept_outbound_and_recover_orig_dst`) and resolved against the mesh
    /// (`MtlsResolve`); the resolve outcome — NOT a declared-peer slot — drives
    /// whether the connection is enforced over mTLS to the resolved backend,
    /// passed through cleartext, or fail-closed (the C1 3-arm decision).
    Outbound { listener: std::net::TcpListener },
    /// Inbound leg-C (client-facing, TPROXY-redirected). orig-dst is
    /// recovered via `getsockname` inside `accept_inbound_leg`.
    Inbound { listener: std::net::TcpListener },
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

/// Outcome of waiting for a pending connection on a leg listener WITHOUT
/// consuming it.
enum ConnectionReady {
    /// A connection is pending (POLLIN) — the next `accept()` returns it.
    Pending,
    /// The listener was closed (POLLNVAL / fd torn down on alloc stop).
    ListenerClosed,
    /// The cooperative `stop` flag was set (alloc torn down) — exit the loop.
    Stopped,
}

/// Block until a connection is PENDING on `listener` without accepting it, so
/// the accept loop can observe the cooperative `stop` flag (and a torn-down
/// listener) between bounded poll slices BEFORE committing to a blocking
/// `accept()` — the loop must not block forever on a stale fd after teardown.
/// Returns [`ConnectionReady::ListenerClosed`] when the listener fd is invalidated
/// (the alloc was torn down and the listener dropped), or
/// [`ConnectionReady::Stopped`] when the cooperative `stop` flag is observed
/// set between poll slices. Polls in bounded (200ms) slices so both a
/// torn-down listener and a stop signal are observed promptly rather than
/// blocking forever on a stale fd.
fn await_pending_connection(
    listener: &std::net::TcpListener,
    stop: &AtomicBool,
    worker: &Weak<MtlsInterceptWorker>,
) -> ConnectionReady {
    use std::os::fd::AsRawFd as _;
    let fd = listener.as_raw_fd();
    loop {
        if stop.load(Ordering::SeqCst) || worker.strong_count() == 0 {
            return ConnectionReady::Stopped;
        }
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: `poll` on a single owned pollfd; the listener outlives the
        // borrow. 200ms slices so a closed listener / stop flag is observed
        // promptly.
        let pr = unsafe { libc::poll(std::ptr::from_mut(&mut pfd), 1, 200) };
        if pr < 0 {
            // EINTR or similar — retry the poll (re-checks `stop` at the top).
            continue;
        }
        if pfd.revents & (libc::POLLNVAL | libc::POLLERR | libc::POLLHUP) != 0 {
            return ConnectionReady::ListenerClosed;
        }
        if pfd.revents & libc::POLLIN != 0 {
            return ConnectionReady::Pending;
        }
        // Timeout (pr == 0) with no revents — loop and re-check stop + poll.
    }
}

/// Spawn the `NonMesh` cleartext pass-through: dial `orig_dst` in cleartext and
/// bidirectionally relay bytes between the captured leg-F and the dialed
/// upstream (the C1 `NonMesh → PASS-THROUGH (cleartext, by design)` arm).
///
/// The workload dialed a NON-mesh destination, so its egress proceeds in
/// cleartext exactly as it would have without interception — the agent merely
/// stands in the path the TPROXY redirect created. NO mTLS, NO `enforce`, NO
/// SVID: this is the classification arm, not a security control. (The byte-exact
/// relay correctness on a real intercepted connect is the Tier-3 05-01
/// obligation; here the relay is the minimal cleartext shuttle.)
///
/// Spawned as an owner-tracked task so it does not stall the accept loop; a
/// dial failure closes leg-F (the upstream is unreachable — nothing to relay).
fn spawn_cleartext_passthrough(
    runtime: &tokio::runtime::Handle,
    alloc: AllocationId,
    leg_f: std::os::fd::OwnedFd,
    orig_dst: SocketAddrV4,
) -> tokio::task::JoinHandle<()> {
    runtime.spawn(async move {
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
    })
}

/// Project a listener's `local_addr()` result into the bound `SocketAddrV4`,
/// failing closed on a genuine `getsockname` error rather than defaulting to a
/// broken port-0 address (D-MTLS-18). The listener is bound `AF_INET`
/// (`make_transparent_listener`), so `local_addr()` is always V4 — the V6 arm
/// is structurally unreachable. `on_err` maps the OS error to the site-specific
/// typed variant (leg-F vs leg-C) so each site's `Display` names its own stage.
fn project_listener_v4(
    local_addr: std::io::Result<std::net::SocketAddr>,
    on_err: impl FnOnce(std::io::Error) -> MtlsInterceptInstallError,
) -> Result<SocketAddrV4, MtlsInterceptInstallError> {
    match local_addr {
        Ok(std::net::SocketAddr::V4(v4)) => Ok(v4),
        Ok(std::net::SocketAddr::V6(v6)) => unreachable!(
            "transparent listener bound AF_INET via make_transparent_listener; \
             local_addr cannot be V6 (got {v6})"
        ),
        Err(source) => Err(on_err(source)),
    }
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
    //! Default-lane DST for the OUTBOUND per-connection resolve consumer
    //! (04-02, ADR-0071 fact 4 / C1).
    //!
    //! The scenario
    //! `outbound_resolve_consumer_drives_enforce_passthrough_failclosed_per_arm`
    //! drives the worker's outbound handling
    //! ([`MtlsInterceptWorker::handle_outbound`], the driving port for the
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

    use std::collections::BTreeMap;
    use std::io::{Read as _, Write as _};
    use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
    use std::num::NonZeroU16;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Weak};
    use std::time::Duration;

    use super::AllocationTaskOwner;
    use async_trait::async_trait;
    use overdrive_core::traits::clock::Clock;
    use overdrive_core::traits::driver::{
        AllocationSpec, DriverPayload, GuestNetworkAssignment, Resources, VmPayload,
    };
    use overdrive_core::traits::mtls_enforcement::{
        EnforcedConnection, EnforcedConnectionId, InterceptedConnection, MtlsEnforcement,
        PumpLiveness, Routed,
    };
    use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve, ResolvedBackend};
    use overdrive_core::{AllocationId, SpiffeId};
    use overdrive_sim::adapters::SimMtlsResolve;
    use overdrive_sim::adapters::clock::SimClock;
    use parking_lot::Mutex;

    use super::{
        AcceptLeg, ConnectionReady, EnforcedSet, MtlsInterceptWorker, OutboundAction,
        await_pending_connection, decide_outbound,
    };
    use crate::mtls_intercept::{InterceptError, InterceptPostcondition};
    use crate::mtls_intercept_port::{InterceptGuard, MtlsIntercept};

    struct TestSharedGuard;

    impl InterceptGuard for TestSharedGuard {}

    struct TestSharedIntercept {
        observation: Mutex<Option<InterceptPostcondition>>,
    }

    impl TestSharedIntercept {
        fn new() -> Self {
            Self { observation: Mutex::new(None) }
        }
    }

    impl MtlsIntercept for TestSharedIntercept {
        fn bind_transparent(
            &self,
            address: SocketAddrV4,
        ) -> crate::mtls_intercept::Result<TcpListener> {
            TcpListener::bind(address)
                .map_err(|source| InterceptError::TransparentListener { addr: address, source })
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
            Ok(Box::new(TestSharedGuard))
        }

        fn observe_shared(&self) -> crate::mtls_intercept::Result<Option<InterceptPostcondition>> {
            Ok(self.observation.lock().clone())
        }

        fn install_outbound(
            &self,
            _source_addr: Ipv4Addr,
            _agent_leg_f_port: u16,
        ) -> crate::mtls_intercept::Result<Box<dyn InterceptGuard>> {
            Ok(Box::new(TestSharedGuard))
        }

        fn install_inbound(
            &self,
            _virt: SocketAddrV4,
            _agent_leg_c_port: u16,
        ) -> crate::mtls_intercept::Result<Box<dyn InterceptGuard>> {
            Ok(Box::new(TestSharedGuard))
        }
    }

    /// CONTRACT_SHAPE: bounded-change (owner release stops an idle blocking accept loop).
    #[test]
    fn idle_accept_wait_stops_when_the_worker_owner_is_released() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind idle listener");
        let stop = AtomicBool::new(false);
        let worker = Weak::<MtlsInterceptWorker>::new();

        assert!(matches!(
            await_pending_connection(&listener, &stop, &worker),
            ConnectionReady::Stopped,
        ));
    }

    struct DropWitness(Arc<AtomicUsize>);

    impl Drop for DropWitness {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl InterceptGuard for DropWitness {}

    /// CONTRACT_SHAPE: bounded-change (one worker owner closes two listeners while retaining two original rules).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn owner_shutdown_joins_children_closes_sockets_and_retains_each_rule_guard() {
        let outbound = TcpListener::bind("127.0.0.1:0").expect("bind outbound listener");
        let inbound = TcpListener::bind("127.0.0.1:0").expect("bind inbound listener");
        let outbound_addr = outbound.local_addr().expect("outbound address");
        let inbound_addr = inbound.local_addr().expect("inbound address");
        let leg_c_addr = match inbound_addr {
            std::net::SocketAddr::V4(addr) => addr,
            std::net::SocketAddr::V6(_) => panic!("test binds IPv4"),
        };
        let (enforcement, _calls) = SpyEnforcement::new();
        let resolve: Arc<dyn MtlsResolve> =
            Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh));
        let worker = worker_with(enforcement, resolve);
        let alloc = alloc("alloc-owner-shutdown");
        let enforced = EnforcedSet::new();
        let stop = Arc::new(AtomicBool::new(false));
        let tasks = AllocationTaskOwner::new();
        let drops = Arc::new(AtomicUsize::new(0));

        worker.spawn_accept_loop(
            alloc.clone(),
            AcceptLeg::Outbound { listener: outbound },
            enforced.clone(),
            Arc::clone(&stop),
            &tasks,
        );
        worker.spawn_accept_loop(
            alloc.clone(),
            AcceptLeg::Inbound { listener: inbound },
            enforced.clone(),
            Arc::clone(&stop),
            &tasks,
        );
        worker.record_intercept_full(
            alloc.clone(),
            Some(Box::new(DropWitness(Arc::clone(&drops)))),
            vec![Box::new(DropWitness(Arc::clone(&drops)))],
            leg_c_addr,
            enforced,
            stop,
            tasks,
        );

        assert_eq!(worker.leg_c_addr(&alloc), Some(leg_c_addr));
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        tokio::time::timeout(Duration::from_secs(2), worker.shutdown_owner())
            .await
            .expect("owner shutdown joins every child")
            .expect("owner shutdown succeeds");

        assert_eq!(worker.leg_c_addr(&alloc), None, "allocation ownership is empty");
        assert_eq!(
            drops.load(Ordering::SeqCst),
            0,
            "process-owner shutdown relinquishes active rule guards without deleting their rules"
        );
        assert!(TcpStream::connect(outbound_addr).is_err(), "outbound socket is closed");
        assert!(TcpStream::connect(inbound_addr).is_err(), "inbound socket is closed");
    }

    /// One recorded `enforce` call — the observable driven-port surface the
    /// per-arm assertions read (the `Routed` routing fact + the alloc + whether
    /// `expected_peer` was set). A spy, NOT a mock: the test asserts on the
    /// recorded business outcome (the routed peer), not on call-count alone.
    #[derive(Debug, Clone)]
    struct EnforceCall {
        routed: Routed,
        alloc: AllocationId,
        expected_peer_is_some: bool,
    }

    /// Spy [`MtlsEnforcement`] recording every `enforce` call's `Routed` so the
    /// Mesh arm can assert `peer == b.addr`. `enforce` always succeeds (returns
    /// an `EnforcedConnection`) — the test exercises the WORKER's 3-arm routing,
    /// not the enforcement substrate (which has its own equivalence suite).
    struct SpyEnforcement {
        calls: Arc<Mutex<Vec<EnforceCall>>>,
        counter: std::sync::atomic::AtomicU64,
    }

    impl SpyEnforcement {
        fn new() -> (Arc<Self>, Arc<Mutex<Vec<EnforceCall>>>) {
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
            conn: InterceptedConnection,
        ) -> overdrive_core::traits::mtls_enforcement::Result<EnforcedConnection> {
            self.calls.lock().push(EnforceCall {
                routed: conn.routed,
                alloc: conn.alloc.clone(),
                expected_peer_is_some: conn.expected_peer.is_some(),
            });
            // `conn.leg` drops here (the spy does not pump) — closing the leg.
            let counter = self.counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(EnforcedConnection::new(EnforcedConnectionId::new(conn.alloc, counter)))
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

    /// Map an [`AbsentSvid`]-free spy onto the worker. The resolve port is the
    /// arm-under-test; the enforcement spy records the Mesh-arm routing.
    fn worker_with(
        enforcement: Arc<SpyEnforcement>,
        resolve: Arc<dyn MtlsResolve>,
    ) -> Arc<MtlsInterceptWorker> {
        let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
        Arc::new(MtlsInterceptWorker::new(
            enforcement,
            resolve,
            clock,
            Arc::new(crate::mtls_intercept_port::HostMtlsIntercept::new()),
        ))
    }

    /// Build a `SimMtlsResolve` that maps `orig_dst` to `arm` (any other addr
    /// resolves to the `NonMesh` default — the host-faithful default per the
    /// 01-02 review).
    fn resolve_scripting(orig_dst: SocketAddrV4, arm: MtlsResolution) -> Arc<dyn MtlsResolve> {
        let mut scripted = BTreeMap::new();
        scripted.insert(orig_dst, arm);
        Arc::new(SimMtlsResolve::new(scripted, MtlsResolution::NonMesh))
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

    /// Drive [`MtlsInterceptWorker::handle_outbound`] on a blocking thread (so
    /// its internal `Handle::block_on(resolve)` is valid — `handle_outbound`
    /// runs on a `spawn_blocking` thread in production), then await the spawned
    /// `JoinHandle`. The `enforced` teardown set is returned so a test can read
    /// the produced handles.
    async fn run_handle_outbound(
        worker: &Arc<MtlsInterceptWorker>,
        alloc: AllocationId,
        leg_f: std::os::fd::OwnedFd,
        orig_dst: SocketAddrV4,
    ) -> (EnforcedSet, AllocationTaskOwner) {
        let enforced = EnforcedSet::new();
        let tasks = AllocationTaskOwner::new();
        let worker = Arc::clone(worker);
        let enforced_for_task = enforced.clone();
        let tasks_for_call = tasks.clone();
        tokio::task::spawn_blocking(move || {
            worker.handle_outbound(&alloc, leg_f, orig_dst, &enforced_for_task, &tasks_for_call);
        })
        .await
        .expect("handle_outbound blocking task joins");
        (enforced, tasks)
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

    /// Mesh arm: `enforce` is called with `Routed::Outbound { peer == b.addr }`
    /// (the RESOLVED backend addr, provably NOT `orig_dst`), `expected_peer`
    /// `None` (authn-only). The worker recovered `orig_dst` from the leg-F
    /// socket, resolved it to `Mesh(b)`, and stamped `b.addr` into the routing
    /// fact — the resolved addr, not the recovered dst.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mesh_arm_enforces_to_the_resolved_backend_addr() {
        let (leg_f, orig_dst, _client) = accepted_leg_f();
        // The resolved backend addr DELIBERATELY differs from orig_dst so the
        // assertion proves the worker uses `b.addr`, not the recovered dst.
        let backend_addr = SocketAddrV4::new(Ipv4Addr::new(10, 9, 8, 7), 4443);
        assert_ne!(backend_addr, orig_dst, "backend addr must differ from orig_dst for the proof");

        let (spy, calls) = SpyEnforcement::new();
        let resolve = resolve_scripting(
            orig_dst,
            MtlsResolution::Mesh(ResolvedBackend { addr: backend_addr, expected_svid: None }),
        );
        let worker = worker_with(Arc::clone(&spy), resolve);

        let (enforced, _tasks) =
            run_handle_outbound(&worker, alloc("alloc-mesh"), leg_f, orig_dst).await;

        // `enforce` is dispatched on a spawned task; spin briefly (bounded) until
        // it is recorded so the assertion is not racing the spawn.
        let recorded = wait_for_calls(&calls, 1).await;
        assert_eq!(recorded.len(), 1, "Mesh must drive exactly one enforce call");
        match recorded[0].routed {
            Routed::Outbound { peer } => assert_eq!(
                peer, backend_addr,
                "enforce must be called with the RESOLVED backend addr, not orig_dst",
            ),
            Routed::Inbound { orig_dst } => {
                panic!("expected Outbound, got Inbound {{ {orig_dst} }}")
            }
        }
        assert_eq!(recorded[0].alloc, alloc("alloc-mesh"), "alloc must round-trip to enforce");
        assert!(!recorded[0].expected_peer_is_some, "v1 authn-only: expected_peer is None");
        // The handle is pushed into the teardown set AFTER `enforce` returns Ok
        // (inside the spawned task, after the spy recorded the call) — wait
        // (bounded, real-time) until it lands so the assertion does not race the push.
        wait_until("enforced handle joins teardown set", || enforced.held_count() == 1).await;
        assert_eq!(enforced.held_count(), 1, "the enforced handle joins the teardown set");
    }

    /// Wait (bounded, in real wall-clock time) until `cond` holds. Polls on a
    /// real timer instead of a fixed `yield_now` budget so a spawned `enforce`
    /// task gets genuine scheduling even under heavy CPU contention — the old
    /// 1000-iteration yield-spin elapsed in microseconds and starved the task
    /// under the high-parallelism mutants profile ("got 0 calls"). `yield_now`
    /// only reschedules among READY tasks; it grants no wall-clock time for a
    /// starved task to become ready. Panics on a 5s timeout (the spawned work
    /// is genuinely broken, not merely slow).
    async fn wait_until(label: &str, mut cond: impl FnMut() -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !cond() {
            assert!(tokio::time::Instant::now() < deadline, "condition not met within 5s: {label}");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    /// Wait (bounded, real-time) until `calls` holds at least `n` recorded
    /// `enforce` calls, then return a clone. The enforce dispatch is a spawned
    /// task; this closes the race between "handle_outbound returned" and "the
    /// spawned enforce ran" without a fixed sleep or a starvable yield budget.
    async fn wait_for_calls(calls: &Arc<Mutex<Vec<EnforceCall>>>, n: usize) -> Vec<EnforceCall> {
        wait_until("enforce calls recorded", || calls.lock().len() >= n).await;
        calls.lock().clone()
    }

    /// NonMesh arm: `enforce` is NOT called; the captured leg is relayed
    /// cleartext to a real upstream bound at `orig_dst`, which receives the
    /// workload's bytes (pass-through, by design). The upstream-receives-bytes
    /// assertion is the falsifiable core: it proves cleartext egress reached the
    /// dialed dst, NOT a fail-closed drop.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nonmesh_arm_passes_through_cleartext_to_orig_dst() {
        // A real upstream server bound on a concrete loopback addr — this IS the
        // `orig_dst` the workload "dialed" (the leg-F getsockname recovers the
        // accepted socket's local addr, so we bind the upstream there is not
        // possible; instead we point orig_dst AT a server we control and assert
        // the relay reaches it). We bind the upstream first and use ITS addr as
        // orig_dst, then make leg-F a separate accepted socket.
        let upstream = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("bind upstream server");
        let upstream_addr = match upstream.local_addr().expect("local_addr") {
            std::net::SocketAddr::V4(a) => a,
            other @ std::net::SocketAddr::V6(_) => panic!("expected V4 addr, got {other}"),
        };

        let (leg_f, _leg_f_addr, mut client) = accepted_leg_f();
        let (spy, calls) = SpyEnforcement::new();
        // orig_dst is the upstream's addr → NonMesh → relay to it.
        let resolve = resolve_scripting(upstream_addr, MtlsResolution::NonMesh);
        let worker = worker_with(Arc::clone(&spy), resolve);

        // Upstream echoes what it receives so the client can read its own bytes
        // back THROUGH the relay (down→up→down) — proving bidirectional
        // cleartext pass-through.
        let upstream_thread = std::thread::spawn(move || {
            let (mut conn, _peer) = upstream.accept().expect("upstream accepts the relayed dial");
            let mut buf = [0u8; 5];
            conn.read_exact(&mut buf).expect("upstream reads the relayed bytes");
            conn.write_all(&buf).expect("upstream echoes back");
            conn.flush().ok();
            buf
        });

        // Drive the resolve consumer with orig_dst == upstream_addr.
        let (_enforced, _tasks) =
            run_handle_outbound(&worker, alloc("alloc-nonmesh"), leg_f, upstream_addr).await;

        // The workload writes through leg-F (the client side of the accepted
        // pair); the relay carries it to the upstream, which echoes it back.
        client.write_all(b"HELLO").expect("workload writes cleartext through leg-F");
        client.flush().ok();
        let mut echoed = [0u8; 5];
        client.read_exact(&mut echoed).expect("workload reads the echoed bytes back through relay");

        assert_eq!(
            &echoed, b"HELLO",
            "cleartext bytes must round-trip through the pass-through relay"
        );
        assert_eq!(
            upstream_thread.join().expect("upstream thread"),
            *b"HELLO",
            "the upstream must receive the workload's cleartext bytes (pass-through)",
        );
        assert!(calls.lock().is_empty(), "NonMesh must NOT call enforce (no mTLS, pass-through)");
    }

    /// MeshUnreachable arm: `enforce` is NOT called; NO upstream is dialed; the
    /// captured leg is closed so the workload's connection sees EOF (fail-closed,
    /// NO cleartext). The EOF-on-the-client assertion is the falsifiable core: a
    /// pass-through (the bug) would keep the leg open and try to relay.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mesh_unreachable_arm_fails_closed_no_cleartext() {
        let (leg_f, orig_dst, mut client) = accepted_leg_f();
        let (spy, calls) = SpyEnforcement::new();
        let resolve = resolve_scripting(orig_dst, MtlsResolution::MeshUnreachable);
        let worker = worker_with(Arc::clone(&spy), resolve);

        let (_enforced, _tasks) =
            run_handle_outbound(&worker, alloc("alloc-unreach"), leg_f, orig_dst).await;

        // The worker dropped leg-F (fail-closed) → the client's read returns EOF
        // (0 bytes), NOT a relayed response. A short read timeout guards against
        // a hang if the leg were (wrongly) kept open.
        client.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let mut buf = [0u8; 1];
        let n = client.read(&mut buf).expect("read on a closed leg returns Ok(0) (EOF)");
        assert_eq!(n, 0, "MeshUnreachable must close leg-F (EOF), never relay cleartext");
        assert!(calls.lock().is_empty(), "MeshUnreachable must NOT call enforce (fail-closed)");
    }

    /// A store-layer resolve `Err` (StoreUnreadable — NOT a per-connection
    /// classification) is treated FAIL-CLOSED: `enforce` is NOT called and the
    /// leg is closed (EOF). An untrusted resolve must never degrade to silent
    /// cleartext.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resolve_store_fault_fails_closed_no_cleartext() {
        let (leg_f, orig_dst, mut client) = accepted_leg_f();
        let (spy, calls) = SpyEnforcement::new();
        // Construct a resolve and arm a one-shot store fault for the next call.
        let mut scripted = BTreeMap::new();
        scripted.insert(
            orig_dst,
            MtlsResolution::Mesh(ResolvedBackend { addr: orig_dst, expected_svid: None }),
        );
        let sim = SimMtlsResolve::new(scripted, MtlsResolution::NonMesh);
        sim.script_resolve_fault("poisoned service_backends handle");
        let resolve: Arc<dyn MtlsResolve> = Arc::new(sim);
        let worker = worker_with(Arc::clone(&spy), resolve);

        let (_enforced, _tasks) =
            run_handle_outbound(&worker, alloc("alloc-fault"), leg_f, orig_dst).await;

        client.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let mut buf = [0u8; 1];
        let n = client.read(&mut buf).expect("read on a closed leg returns Ok(0) (EOF)");
        assert_eq!(n, 0, "a resolve store-fault must close leg-F fail-closed (no cleartext)");
        assert!(calls.lock().is_empty(), "a faulted resolve must NOT call enforce");
    }

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

    /// Regression (D-MTLS-18): `project_listener_v4` MUST fail closed on a
    /// `local_addr()`/getsockname error, returning the site-specific typed
    /// variant — NEVER a broken port-0 `SocketAddrV4`. This is the assertion the
    /// pre-fix `.ok().and_then(socketaddr_v4).unwrap_or_else(|| ...:0)` chain
    /// could never satisfy: it swallowed the `Err` and yielded `Ok(127.0.0.1:0)`,
    /// which flowed into `install_outbound_tproxy(host_veth, 0)` as a silent
    /// `tproxy to 127.0.0.1:0` install. The Err→typed-variant assertion below is
    /// the discriminator between the buggy and fixed behaviour; the Ok(V4)
    /// passthrough pins the success path unchanged.
    #[test]
    fn project_listener_v4_fails_closed_on_local_addr_error_never_port_zero() {
        use super::{MtlsInterceptInstallError, project_listener_v4};

        // --- Err arm: leg-F mapper fails closed to LegFLocalAddr (NOT port 0) ---
        let leg_f = project_listener_v4(
            Err(std::io::Error::from(std::io::ErrorKind::Other)),
            MtlsInterceptInstallError::leg_f_local_addr,
        );
        assert!(
            matches!(leg_f, Err(MtlsInterceptInstallError::LegFLocalAddr { .. })),
            "a leg-F local_addr() error must fail closed as LegFLocalAddr, never a port-0 addr; got {leg_f:?}",
        );

        // --- Err arm: leg-C mapper fails closed to LegCLocalAddr (NOT port 0) ---
        let leg_c = project_listener_v4(
            Err(std::io::Error::from(std::io::ErrorKind::Other)),
            MtlsInterceptInstallError::leg_c_local_addr,
        );
        assert!(
            matches!(leg_c, Err(MtlsInterceptInstallError::LegCLocalAddr { .. })),
            "a leg-C local_addr() error must fail closed as LegCLocalAddr, never a port-0 addr; got {leg_c:?}",
        );

        // --- Ok(V4) passthrough: the bound addr is returned unchanged ----------
        let bound = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 54321);
        let ok = project_listener_v4(
            Ok(std::net::SocketAddr::V4(bound)),
            MtlsInterceptInstallError::leg_f_local_addr,
        );
        assert_eq!(
            ok.expect("Ok(V4) must project to the bound addr, not fail"),
            bound,
            "the success path must return the exact bound SocketAddrV4 unchanged",
        );
    }

    // ---- EnforcedSet: post-task-fence drain primitive (pure unit) ----------

    /// Build an `EnforcedConnection` with a stable, asserter-readable id so a
    /// drained / handed-back handle can be matched by id.
    fn enforced_conn(alloc_name: &str, counter: u64) -> EnforcedConnection {
        EnforcedConnection::new(EnforcedConnectionId::new(alloc(alloc_name), counter))
    }

    /// The `EnforcedSet` post-task-fence drain contract. Every completed handle
    /// remains retained until the stop owner has joined all producers and
    /// performs the final drain.
    #[test]
    fn enforced_set_retains_late_push_for_the_post_task_fence_drain() {
        let set = EnforcedSet::new();

        // Push retains each completed handle.
        let h0 = enforced_conn("set-alloc", 0);
        let h1 = enforced_conn("set-alloc", 1);
        set.push(h0.clone());
        set.push(h1.clone());
        assert_eq!(set.held_count(), 2, "both pushes are retained while the set is open");

        // DRAIN: returns exactly the stored handles, in push order.
        let drained = set.drain();
        let drained_ids: Vec<_> = drained.iter().map(|h| h.id().clone()).collect();
        assert_eq!(
            drained_ids,
            vec![h0.id().clone(), h1.id().clone()],
            "drain must return exactly the handles that were pushed",
        );
        assert_eq!(set.held_count(), 0, "the set is empty after draining");

        // A later producer is retained for the stop owner's final
        // post-task-fence drain.
        let late = enforced_conn("set-alloc", 2);
        set.push(late.clone());
        assert_eq!(set.held_count(), 1, "the stop owner retains the late handle");
        assert_eq!(set.drain()[0].id(), late.id());

        // Idempotent: a second drain is empty.
        assert!(set.drain().is_empty(), "a second drain observes an already-empty set");
    }

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
        tokio::task::spawn_blocking({
            let worker = Arc::clone(&worker);
            move || worker.handle_shared_outbound(Ipv4Addr::LOCALHOST, leg, orig_dst)
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

    struct GatedTeardown {
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
        calls: AtomicUsize,
        fail_first: AtomicBool,
    }

    struct RetryingSharedEnforcement {
        enforced: tokio::sync::Notify,
        counter: std::sync::atomic::AtomicU64,
        teardown_calls: AtomicUsize,
        teardown_ids: Mutex<Vec<EnforcedConnectionId>>,
        fail_first: AtomicBool,
    }

    impl RetryingSharedEnforcement {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                enforced: tokio::sync::Notify::new(),
                counter: std::sync::atomic::AtomicU64::new(0),
                teardown_calls: AtomicUsize::new(0),
                teardown_ids: Mutex::new(Vec::new()),
                fail_first: AtomicBool::new(true),
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
            if self.fail_first.swap(false, Ordering::SeqCst) {
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
        tokio::task::spawn_blocking({
            let worker = Arc::clone(&worker);
            move || worker.handle_shared_outbound(address, leg, orig_dst)
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
        assert_eq!(first.alloc_id, allocation);
        assert_eq!(first.failures.len(), 1);
        assert!(first.failures[0].contains("injected shared teardown failure"));
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

    #[async_trait]
    impl MtlsEnforcement for GatedTeardown {
        async fn probe(&self) -> overdrive_core::traits::mtls_enforcement::Result<()> {
            Ok(())
        }

        async fn enforce(
            &self,
            _conn: InterceptedConnection,
        ) -> overdrive_core::traits::mtls_enforcement::Result<EnforcedConnection> {
            unreachable!("shutdown-fence test seeds the owned handle directly")
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
                return Err(overdrive_core::traits::mtls_enforcement::MtlsEnforcementError::TeardownFailed {
                    id: handle.id().clone(),
                    source: std::io::Error::other("injected teardown failure"),
                });
            }
            Ok(())
        }
    }

    /// CONTRACT_SHAPE: bounded-change (cancelled and concurrent shutdown callers share one sealed worker completion).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn replacement_shutdown_waits_for_the_same_authoritative_teardown() {
        let enforcement = Arc::new(GatedTeardown {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            calls: AtomicUsize::new(0),
            fail_first: AtomicBool::new(true),
        });
        let worker = Arc::new(MtlsInterceptWorker::new(
            Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
            resolve_scripting(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0), MtlsResolution::NonMesh),
            Arc::new(SimClock::new()),
            Arc::new(crate::mtls_intercept_port::HostMtlsIntercept::new()),
        ));
        let the_alloc = alloc("alloc-shutdown-fence");
        let enforced = EnforcedSet::new();
        enforced.push(enforced_conn("alloc-shutdown-fence", 1));
        worker.record_intercept_full(
            the_alloc,
            None,
            Vec::new(),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0),
            enforced,
            Arc::new(AtomicBool::new(false)),
            AllocationTaskOwner::new(),
        );

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

        // Owner shutdown is one-shot. A later caller receives the retained
        // aggregate and cannot create a second cleanup generation.
        assert!(worker.shutdown_owner().await.is_err());
        assert_eq!(
            enforcement.calls.load(Ordering::SeqCst),
            1,
            "a sealed process owner never retries failed teardown work"
        );
    }

    /// CONTRACT_SHAPE: bounded-change (same-owner reinstall cannot report readiness before prior teardown).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn same_owner_reinstall_waits_for_prior_teardown_before_readiness() {
        let enforcement = Arc::new(GatedTeardown {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            calls: AtomicUsize::new(0),
            fail_first: AtomicBool::new(false),
        });
        let worker = Arc::new(MtlsInterceptWorker::new(
            Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
            resolve_scripting(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0), MtlsResolution::NonMesh),
            Arc::new(SimClock::new()),
            Arc::new(crate::mtls_intercept_port::HostMtlsIntercept::new()),
        ));
        let the_alloc = alloc("alloc-same-owner-reinstall-fence");
        let enforced = EnforcedSet::new();
        enforced.push(enforced_conn("alloc-same-owner-reinstall-fence", 1));
        worker.record_intercept_full(
            the_alloc.clone(),
            None,
            Vec::new(),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0),
            enforced,
            Arc::new(AtomicBool::new(false)),
            AllocationTaskOwner::new(),
        );

        let mut replacement = tokio::spawn({
            let worker = Arc::clone(&worker);
            let spec = minimal_spec(the_alloc.clone());
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
        worker.stop_alloc(&the_alloc).await.expect("replacement owner cleans up");
    }

    /// CONTRACT_SHAPE: bounded-change (failed prior teardown keeps replacement closed and retryable).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn same_owner_reinstall_failure_keeps_readiness_closed_until_retry() {
        let enforcement = Arc::new(GatedTeardown {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            calls: AtomicUsize::new(0),
            fail_first: AtomicBool::new(true),
        });
        let worker = Arc::new(MtlsInterceptWorker::new(
            Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
            resolve_scripting(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0), MtlsResolution::NonMesh),
            Arc::new(SimClock::new()),
            Arc::new(crate::mtls_intercept_port::HostMtlsIntercept::new()),
        ));
        let the_alloc = alloc("alloc-same-owner-reinstall-retry");
        let enforced = EnforcedSet::new();
        enforced.push(enforced_conn("alloc-same-owner-reinstall-retry", 1));
        worker.record_intercept_full(
            the_alloc.clone(),
            None,
            Vec::new(),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0),
            enforced,
            Arc::new(AtomicBool::new(false)),
            AllocationTaskOwner::new(),
        );

        let first = tokio::spawn({
            let worker = Arc::clone(&worker);
            let spec = minimal_spec(the_alloc.clone());
            async move { worker.start_alloc(&spec).await }
        });
        tokio::time::timeout(Duration::from_secs(1), enforcement.entered.notified())
            .await
            .expect("first prior teardown starts");
        enforcement.release.notify_one();
        let first = first.await.expect("first replacement task joins");
        assert!(matches!(first, Err(super::MtlsInterceptInstallError::PriorTeardown { .. })));
        assert_eq!(
            worker.leg_c_addr(&the_alloc),
            None,
            "failed prior teardown cannot install or report a replacement listener"
        );

        let retry = tokio::spawn({
            let worker = Arc::clone(&worker);
            let spec = minimal_spec(the_alloc.clone());
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
        assert!(worker.leg_c_addr(&the_alloc).is_some());
        assert_eq!(enforcement.calls.load(Ordering::SeqCst), 2);
        worker.stop_alloc(&the_alloc).await.expect("replacement owner cleans up");
    }

    /// CONTRACT_SHAPE: bounded-change (failed authoritative teardown is surfaced and retried before completion).
    #[tokio::test]
    async fn allocation_stop_surfaces_teardown_failure_and_retry_converges() {
        let enforcement = Arc::new(GatedTeardown {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            calls: AtomicUsize::new(0),
            fail_first: AtomicBool::new(true),
        });
        let worker = Arc::new(MtlsInterceptWorker::new(
            Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
            resolve_scripting(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0), MtlsResolution::NonMesh),
            Arc::new(SimClock::new()),
            Arc::new(crate::mtls_intercept_port::HostMtlsIntercept::new()),
        ));
        let the_alloc = alloc("alloc-stop-retry");
        let enforced = EnforcedSet::new();
        enforced.push(enforced_conn("alloc-stop-retry", 1));
        worker.record_intercept_full(
            the_alloc.clone(),
            None,
            Vec::new(),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0),
            enforced,
            Arc::new(AtomicBool::new(false)),
            AllocationTaskOwner::new(),
        );

        enforcement.release.notify_one();
        assert!(worker.stop_alloc(&the_alloc).await.is_err());
        enforcement.release.notify_one();
        worker.stop_alloc(&the_alloc).await.expect("retry completes authoritative teardown");
        assert_eq!(enforcement.calls.load(Ordering::SeqCst), 2);
    }

    /// REGRESSION (P1, GH #26): a completed `spawn_enforce` task must retain its
    /// handle inside allocation ownership until authoritative teardown.
    ///
    /// Drives the real production path: `record_intercept_full` registers an
    /// alloc sharing an [`EnforcedSet`]; `spawn_enforce` fires an enforce gated
    /// at completion, then the test releases it and observes the handle enter
    /// the owned set before stop. The assertion: stop drains and tears down that
    /// exact handle.
    ///
    /// The task owner makes admission closure, child completion, and the final
    /// handle drain one ordered teardown sequence; the concurrent cancellation
    /// partition is covered separately by the owner-shutdown fence test.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn completed_enforce_handle_is_torn_down_not_orphaned() {
        let spy = GatedEnforcement::new();
        let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
        let resolve =
            resolve_scripting(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0), MtlsResolution::NonMesh);
        let enforcement: Arc<dyn MtlsEnforcement> = Arc::clone(&spy) as Arc<dyn MtlsEnforcement>;
        let worker = Arc::new(MtlsInterceptWorker::new(
            enforcement,
            resolve,
            clock,
            Arc::new(crate::mtls_intercept_port::HostMtlsIntercept::new()),
        ));

        let the_alloc = alloc("alloc-orphan-race");
        // Register an alloc that shares `enforced` — the SAME set spawn_enforce
        // pushes into and stop_alloc drains. No real listeners/guards (None /
        // empty); we drive the enforce + stop path directly, not start_alloc
        // (which would bind real IP_TRANSPARENT listeners → needs root).
        let enforced = EnforcedSet::new();
        let recorded_spec = AllocationSpec {
            alloc: the_alloc.clone(),
            identity: SpiffeId::new(
                "spiffe://overdrive.local/workload/orphan-race/alloc/alloc-orphan-race",
            )
            .expect("valid fixture SPIFFE id"),
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
        };
        let tasks = AllocationTaskOwner::new();
        worker.record_intercept_full(
            recorded_spec.alloc,
            None,
            Vec::new(),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0),
            enforced.clone(),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            tasks.clone(),
        );

        // Fire an in-flight enforce through the real spawn_enforce path. The
        // leg is a real accepted loopback socket (as accepted_leg_f hands back).
        let (leg, _addr, _client) = accepted_leg_f();
        let conn = InterceptedConnection {
            leg,
            routed: Routed::Outbound { peer: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 9) },
            alloc: the_alloc.clone(),
            expected_peer: None,
        };
        worker.spawn_enforce(&the_alloc, conn, &enforced, &tasks);

        // Wait until enforce is in flight, then make its successful completion
        // and ownership transfer causally explicit.
        tokio::time::timeout(Duration::from_secs(5), spy.entered())
            .await
            .expect("enforce must enter (in-flight) within 5s");
        spy.release();
        tokio::time::timeout(Duration::from_secs(2), async {
            while enforced.held_count() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("completed enforce handle enters the allocation-owned set");

        tokio::time::timeout(Duration::from_secs(10), worker.stop_alloc(&the_alloc))
            .await
            .expect("allocation stop is bounded")
            .expect("teardown succeeds");

        let recorded = spy.torn_down();
        assert!(spy.exited.load(Ordering::SeqCst), "the in-flight enforce child is joined");
        assert!(enforced.held_count() == 0, "no enforced handle remains outside teardown");
        assert_eq!(recorded.len(), 1, "the completed handle is torn down exactly once");
        assert_eq!(recorded[0].alloc(), &the_alloc);
    }

    /// CONTRACT_SHAPE: bounded-change (allocation stop joins every enforce child before returning).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn allocation_stop_joins_an_inflight_enforce_child() {
        let spy = GatedEnforcement::new();
        let enforcement: Arc<dyn MtlsEnforcement> = Arc::clone(&spy) as Arc<dyn MtlsEnforcement>;
        let worker = Arc::new(MtlsInterceptWorker::new(
            enforcement,
            resolve_scripting(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0), MtlsResolution::NonMesh),
            Arc::new(SimClock::new()),
            Arc::new(crate::mtls_intercept_port::HostMtlsIntercept::new()),
        ));
        let the_alloc = alloc("alloc-stopped-owner-child");
        let enforced = EnforcedSet::new();
        let tasks = AllocationTaskOwner::new();
        worker.record_intercept_full(
            the_alloc.clone(),
            None,
            Vec::new(),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0),
            enforced.clone(),
            Arc::new(AtomicBool::new(false)),
            tasks.clone(),
        );
        let (leg, _addr, _client) = accepted_leg_f();
        worker.spawn_enforce(
            &the_alloc,
            InterceptedConnection {
                leg,
                routed: Routed::Outbound { peer: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 9) },
                alloc: the_alloc.clone(),
                expected_peer: None,
            },
            &enforced,
            &tasks,
        );
        tokio::time::timeout(Duration::from_secs(2), spy.entered())
            .await
            .expect("enforce child enters its blocking gate");

        tokio::time::timeout(Duration::from_secs(10), worker.stop_alloc(&the_alloc))
            .await
            .expect("allocation stop is bounded")
            .expect("allocation stop succeeds");
        let ended_before_manual_release = spy.exited.load(Ordering::SeqCst);
        if !ended_before_manual_release {
            spy.release();
            wait_until("pre-fix escaped RED child exits for fixture cleanup", || {
                spy.exited.load(Ordering::SeqCst)
            })
            .await;
        }

        assert!(
            ended_before_manual_release,
            "allocation stop must abort and join an in-flight enforce child before returning"
        );
    }

    /// CONTRACT_SHAPE: bounded-change (allocation stop joins every pass-through child before returning).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn allocation_stop_joins_a_passthrough_child() {
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
        let worker = worker_with(spy, resolve_scripting(upstream_addr, MtlsResolution::NonMesh));
        let the_alloc = alloc("alloc-stopped-passthrough-child");
        let tasks = AllocationTaskOwner::new();
        worker.record_intercept_full(
            the_alloc.clone(),
            None,
            Vec::new(),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0),
            EnforcedSet::new(),
            Arc::new(AtomicBool::new(false)),
            tasks.clone(),
        );
        let (leg, _addr, mut client) = accepted_leg_f();
        let enforced = EnforcedSet::new();
        let worker_for_handle = Arc::clone(&worker);
        let tasks_for_handle = tasks.clone();
        let alloc_for_handle = the_alloc.clone();
        tokio::task::spawn_blocking(move || {
            worker_for_handle.handle_outbound(
                &alloc_for_handle,
                leg,
                upstream_addr,
                &enforced,
                &tasks_for_handle,
            );
        })
        .await
        .expect("handle outbound joins");
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
}

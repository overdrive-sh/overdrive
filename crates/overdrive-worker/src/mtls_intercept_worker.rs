//! The worker's mTLS intercept-and-enforce lifecycle component
//! (D-MTLS-16 / D-MTLS-17, GH #26; step 06-03).
//!
//! This is the **(β) separate lifecycle component** the action-shim fires
//! alongside the driver hooks (NOT held by `ExecDriver`; `ExecDriver` is
//! UNTOUCHED). It owns the production mTLS intercept-install +
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
//!   listeners, and spawns the accept→`enforce` tasks. The INBOUND nft-TPROXY
//!   rule stays #178-deferred (its match key is the server workload's logical
//!   virt address — an east-west service-resolution fact v1 has no production
//!   source for); the leg-C listener + accept loop ARE production. See the
//!   module-level note below.
//! - [`stop_alloc`](MtlsInterceptWorker::stop_alloc) — fired at the
//!   action-shim's `on_alloc_terminal` site. Drains the alloc's
//!   per-connection teardown set (`enforcement.teardown`), aborts the
//!   accept tasks, and drops the OUTBOUND + INBOUND `TproxyInterceptGuard`s
//!   (each removes its per-veth / per-virt nft rule by handle; the
//!   node-global shared routing infra is left intact). Idempotent.
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
//! ## Outbound interception (ADR-0071 Path A) + inbound #178-deferral
//!
//! The OUTBOUND intercept is the per-veth egress nft-TPROXY rule: every TCP
//! flow the workload emits on its host-side veth (`iifname spec.host_veth`)
//! is TPROXY-redirected to the agent's leg-F listener, with the original
//! destination recovered per-flow via `getsockname` on the accepted leg-F
//! socket (D-TME-4, symmetric with the inbound TPROXY path). No per-peer
//! enumeration is needed — TPROXY captures ALL the workload's egress, so the
//! declared-peer `MTLS_REDIRECT_DEST` map + per-destination rewrite of the
//! retired cgroup mechanism are GONE (D-TME-3 RETIRED). The per-connection
//! `MtlsResolve` consumer that decides Mesh-vs-NonMesh per recovered orig-dst
//! lands in step 04-02; until then the outbound accept loop carries the
//! vestigial declared-peer `real_peer` slot (inert — nothing programs it).
//!
//! The INBOUND nft-TPROXY rule is deferred: its match key is the server
//! workload's logical (virt) address — the loopback addr/port clients dial —
//! which is an east-west service-resolution fact with no v1 production source.
//! So `start_alloc` installs NO inbound TPROXY rule (it records
//! `tproxy_guard = None`); the [`install_inbound_tproxy`](crate::mtls_intercept::install_inbound_tproxy)
//! free function stays the named #178 production-install site, exercised today
//! only by the worker integration tests (which supply a real, distinct virt).
//! Everything else (the outbound egress rule + leg-F + leg-C listeners + both
//! accept loops + `enforce` + the wire) is production.

use std::collections::BTreeMap;
use std::net::SocketAddrV4;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use overdrive_core::AllocationId;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::AllocationSpec;
use overdrive_core::traits::mtls_enforcement::{EnforcedConnection, MtlsEnforcement};
use parking_lot::Mutex;

use crate::mtls_intercept::{
    self, InterceptError, TproxyInterceptGuard, accept_inbound_leg, accept_outbound_leg,
    install_outbound_tproxy, make_transparent_listener,
};

/// Per-alloc transparent-mTLS intercept-install failure (D-MTLS-18).
///
/// Returned by [`MtlsInterceptWorker::start_alloc`] when any of the three
/// install steps fails. The install is a **fail-closed security control**,
/// not a best-effort observability hook: an alloc whose intercept cannot be
/// installed MUST NOT run with cleartext egress/ingress, so the failure is
/// SURFACED to the action-shim (which drives the alloc to terminal `Failed`),
/// not swallowed in a `warn!`.
///
/// This enum invents NO new lower-level error surface — it wraps the typed
/// [`InterceptError`] the install steps already produce (the OUTBOUND egress
/// nft-TPROXY install + the leg-C transparent listener) plus the leg-F bind
/// `io::Error`. Each source `Display` names the privilege / kernel-feature
/// remediation an operator acts on. (The inbound nft-TPROXY rule install is
/// #178-deferred — see the module note — so it is not an install step and has
/// no failure site here; the [`InterceptError::TproxyInstall`] variant still
/// flows through `Inbound` from the [`install_inbound_tproxy`](crate::mtls_intercept::install_inbound_tproxy)
/// free function's own callers.)
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MtlsInterceptInstallError {
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

    /// leg-F (outbound, workload-facing plaintext) listener bind failed
    /// (site 2). `#[source]` (not `#[from]`): a bare `io::Error` from-impl
    /// would be too greedy, and a named constructor keeps the site-2 cause
    /// distinct in `Display`.
    #[error("mTLS leg-F listener bind failed: {0}")]
    LegFBind(#[source] std::io::Error),

    /// INBOUND leg-C transparent listener bind failed (site 3,
    /// [`InterceptError::TransparentListener`]). Source `Display` names the
    /// privilege / kernel-feature remediation. (The inbound nft-TPROXY rule
    /// install is #178-deferred and not performed by `start_alloc`, so
    /// [`InterceptError::TproxyInstall`] does not reach this variant from the
    /// production path — it flows only from the
    /// [`install_inbound_tproxy`](crate::mtls_intercept::install_inbound_tproxy)
    /// free function's test callers.)
    #[error("mTLS inbound intercept install failed: {0}")]
    Inbound(#[from] InterceptError),
}

impl MtlsInterceptInstallError {
    /// Associated constructor for the site-2 leg-F bind failure, per the
    /// project's "associated constructor per variant" convention. The
    /// `#[source]` wrap (not `#[from]`) means there is no auto-conversion, so
    /// the call site names this constructor explicitly.
    #[must_use]
    const fn leg_f_bind(source: std::io::Error) -> Self {
        Self::LegFBind(source)
    }

    /// Associated constructor for the site-1 outbound nft-TPROXY install
    /// failure. `#[source]` wrap (not `#[from]`, which the `Inbound` variant
    /// owns), so the call site names this constructor explicitly.
    #[must_use]
    const fn outbound_tproxy_install(source: InterceptError) -> Self {
        Self::OutboundTproxyInstall(source)
    }

    /// The closed-vocabulary install-stage label for the
    /// [`TransitionReason::MtlsInterceptInstallFailed`] cause-class the shim
    /// writes. Maps the 3-variant error (and, for [`Self::Inbound`], the
    /// inner [`InterceptError`] variant) to the four pinned stage strings:
    /// `"outbound_tproxy_install"`, `"leg_f_bind"`,
    /// `"leg_c_transparent_listener"`, `"inbound_tproxy"`. Internal mapping
    /// helper — NOT new contract surface.
    ///
    /// [`TransitionReason::MtlsInterceptInstallFailed`]:
    ///     overdrive_core::transition_reason::TransitionReason::MtlsInterceptInstallFailed
    #[must_use]
    pub const fn stage(&self) -> &'static str {
        match self {
            Self::OutboundTproxyInstall(_) => "outbound_tproxy_install",
            Self::LegFBind(_) => "leg_f_bind",
            Self::Inbound(InterceptError::TransparentListener { .. }) => {
                "leg_c_transparent_listener"
            }
            // Every other `InterceptError` reaching the install path is the
            // site-4 TPROXY install (`TproxyInstall`); the accept/orig-dst
            // variants arise only on the per-connection accept loop, never on
            // `start_alloc`'s install path, so they cannot reach here.
            Self::Inbound(_) => "inbound_tproxy",
        }
    }
}

/// Per-allocation intercept state held for the alloc's lifetime and
/// torn down on `stop_alloc`. This is lifecycle bookkeeping keyed by
/// `AllocationId` (NOT a liveness loop — D-MTLS-16).
struct AllocIntercept {
    /// The OUTBOUND nft-TPROXY egress-rule guard for this alloc's host-side
    /// veth (`install_outbound_tproxy`, D-TME-4 / ADR-0071 Path A). Dropping
    /// it removes the per-veth egress rule from the shared `prerouting`
    /// chain by handle (the node-global shared routing infra is left intact).
    /// `Some` on the mTLS-composed production boot (where the action-shim C3
    /// seam set `spec.host_veth`); `None` off the gate (a fixture with no
    /// provisioned veth), where the leg-F listener + accept loop still stand
    /// up but no egress rule is installed.
    _outbound_tproxy_guard: Option<TproxyInterceptGuard>,
    /// The inbound nft-TPROXY redirect guard. Dropping it removes the
    /// per-virt rule from the shared chain. `None` while the inbound rule is
    /// #178-deferred (the leg-C listener + accept loop ARE production; only
    /// the inbound nft rule has no v1 virt source).
    _tproxy_guard: Option<TproxyInterceptGuard>,
    /// The spawned accept→enforce tasks (outbound + inbound). Aborted on
    /// teardown so a blocked `accept()` does not outlive the alloc.
    accept_tasks: Vec<tokio::task::JoinHandle<()>>,
    /// Cooperative stop flag for the blocking accept loops. The loops run
    /// on `spawn_blocking` threads, so `JoinHandle::abort` cannot interrupt
    /// a blocking `accept()`/`poll()` mid-syscall — the loops must observe
    /// this flag between bounded poll slices and exit themselves.
    /// `stop_alloc` sets it; without it a blocking accept loop outlives the
    /// alloc (and, in a test runtime, blocks the runtime drop forever).
    stop: Arc<AtomicBool>,
    /// The `EnforcedConnection` handles this alloc produced, drained
    /// through `enforcement.teardown` on stop. Behind the worker's
    /// per-alloc `Mutex` so the spawned accept task can push as it
    /// `enforce`s.
    enforced: Arc<Mutex<Vec<EnforcedConnection>>>,
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
    /// Injected `Clock` per the mandatory-port-dependency rule. Reserved
    /// for the deferred per-connection progress-stall watchdog
    /// ([#232](https://github.com/overdrive-sh/overdrive/issues/232));
    /// liveness in v1 is (C) kernel + (B) self-teardown, neither of which
    /// reads the clock here.
    _clock: Arc<dyn Clock>,
    /// Per-alloc teardown bookkeeping (D-MTLS-16). `BTreeMap` per
    /// `.claude/rules/development.md` § "Ordered-collection choice" — the
    /// set is drained deterministically on stop.
    intercepts: Mutex<BTreeMap<AllocationId, AllocIntercept>>,
}

impl MtlsInterceptWorker {
    /// Construct from the REQUIRED ports. `enforcement` and `clock` are both
    /// mandatory — no defaulting, no builder
    /// (`.claude/rules/development.md` § "Port-trait dependencies").
    ///
    /// As of step 04-01 (ADR-0071 Path A) the OUTBOUND intercept is the
    /// host-veth nft-TPROXY rule installed per-alloc in
    /// [`start_alloc`](Self::start_alloc) — NOT a `cgroup_connect4_mtls`
    /// attach — so the worker no longer holds an `MtlsDataplane` or a
    /// `cgroup_root`. The host-veth NAME the egress rule matches arrives
    /// per-alloc on `AllocationSpec.host_veth` (JOIN-6), not at construction.
    #[must_use]
    pub fn new(enforcement: Arc<dyn MtlsEnforcement>, clock: Arc<dyn Clock>) -> Self {
        Self { enforcement, _clock: clock, intercepts: Mutex::new(BTreeMap::new()) }
    }

    /// Install the per-alloc intercept and start the accept→`enforce`
    /// tasks. Fired UNCONDITIONALLY from the action-shim's
    /// `on_alloc_running` site for every exec allocation (D-MTLS-15: every
    /// host-socket exec alloc is intercepted; the predicate is
    /// `DriverType::Exec`, which is unconditionally true on the worker's
    /// exec lifecycle path).
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
    /// confidentiality guarantee broken. (The INBOUND nft-TPROXY rule install
    /// is #178-deferred — see the module note — so it is not an install step
    /// here and has no fail-closed site.)
    ///
    /// **Partial-teardown on the `Err` path.** Every guard acquired before
    /// the failing step (the OUTBOUND [`TproxyInterceptGuard`], the leg-F /
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
    /// listener) when the corresponding install step fails. Each source
    /// `Display` names the privilege / kernel-feature remediation an operator
    /// acts on. (The inbound nft-TPROXY rule is #178-deferred; it is not
    /// installed here, so there is no site-4 failure.)
    pub fn start_alloc(
        self: &Arc<Self>,
        spec: &AllocationSpec,
    ) -> Result<(), MtlsInterceptInstallError> {
        // Re-fire safety: drop any prior intercept for this alloc first
        // (Restart reuses the alloc id).
        self.stop_alloc(&spec.alloc);

        // The agent's leg-F (outbound, workload-facing plaintext) listener
        // — agent-chosen ephemeral loopback (D-MTLS-15). Leg F needs no
        // IP_TRANSPARENT; a plain bound listener suffices. Bound FIRST so its
        // ephemeral port is the redirect target the OUTBOUND nft-TPROXY rule
        // points at.
        // Fail-closed (D-MTLS-18 site 2): on bind failure, return `Err`;
        // nothing is acquired yet, so there is nothing to tear down.
        let leg_f_listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(source) => return Err(MtlsInterceptInstallError::leg_f_bind(source)),
        };
        // The agent's chosen leg-F address — the kernel-redirect TARGET the
        // OUTBOUND nft-TPROXY egress rule redirects the workload's egress to.
        // Recorded in the per-alloc bookkeeping (the declared-peer slot, still
        // present until 04-02, reads it).
        let leg_f_addr = leg_f_listener
            .local_addr()
            .ok()
            .and_then(socketaddr_v4)
            .unwrap_or_else(|| SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0));

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
        let outbound_tproxy_guard = match spec.host_veth.as_deref() {
            Some(host_veth) => Some(
                install_outbound_tproxy(host_veth, leg_f_addr.port())
                    .map_err(MtlsInterceptInstallError::outbound_tproxy_install)?,
            ),
            None => None,
        };

        // INBOUND install: the agent's leg-C IP_TRANSPARENT listener. The
        // accompanying nft-TPROXY redirect that would aim real client traffic
        // at this listener is #178-DEFERRED (see below) — production stands up
        // the listener + accept loop, but installs NO production TPROXY rule.
        // Fail-closed (D-MTLS-18 site 3): a server workload with no leg-C
        // inbound listener accepts cleartext client connections — a
        // confidentiality breach symmetric to the outbound one. Return `Err`
        // (the inbound carve-out is REJECTED per D-MTLS-18 P2);
        // `outbound_tproxy_guard` + `leg_f_listener` (the guards acquired so
        // far) drop here → remove the egress rule / close the leg-F listener.
        let inbound_listener =
            match make_transparent_listener(SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0)) {
                Ok(l) => l,
                Err(source) => return Err(MtlsInterceptInstallError::Inbound(source)),
            };

        // The inbound nft-TPROXY rule install is #178-DEFERRED, symmetric with
        // the OUTBOUND `MTLS_REDIRECT_DEST` redirect above. The rule's match
        // key is the server workload's logical (virt) address — the loopback
        // addr/port clients actually dial — and v1 has NO production source for
        // that value: `AllocationSpec` carries no listen-addr field and the
        // workload binds its own socket at runtime (the same east-west
        // service-resolution gap that defers the outbound peer set;
        // [#178](https://github.com/overdrive-sh/overdrive/issues/178), whose
        // thread names the inbound orig-dst→real-backend resolution and the
        // `server_dial_addr` / D-MTLS-15 replacement site as #178's job).
        // So `start_alloc` records `tproxy_guard = None` and installs no rule;
        // the [`install_inbound_tproxy`] free function stays the named #178
        // production-install site, exercised today only by the worker
        // integration tests (which supply a real, distinct virt) — the SAME
        // "only test callers until #178" shape as the outbound
        // `program_declared_peer_redirect` seam. A `virt` synthesised from the
        // agent's own ephemeral leg-C port (the prior shape) installed a
        // self-referential rule that matched no real inbound connection —
        // inert in production while reading as "inbound mTLS works".
        self.spawn_legs_and_record(
            spec,
            outbound_tproxy_guard,
            None,
            leg_f_listener,
            leg_f_addr,
            inbound_listener,
        );
        Ok(())
    }

    /// Spawn the outbound + inbound accept loops for an alloc and record the
    /// full intercept bookkeeping. Factored out of [`start_alloc`] so that
    /// method stays under the small-function budget; this owns the shared
    /// per-alloc state (`enforced` teardown set, `real_peer` dial-target slot,
    /// cooperative `stop` flag) the two legs and the recorded intercept share.
    fn spawn_legs_and_record(
        self: &Arc<Self>,
        spec: &AllocationSpec,
        outbound_tproxy_guard: Option<TproxyInterceptGuard>,
        tproxy_guard: Option<TproxyInterceptGuard>,
        leg_f_listener: std::net::TcpListener,
        leg_f_addr: SocketAddrV4,
        inbound_listener: std::net::TcpListener,
    ) {
        let enforced: Arc<Mutex<Vec<EnforcedConnection>>> = Arc::new(Mutex::new(Vec::new()));
        // The declared-peer dial target, supplied by the #178 stand-in seam
        // AFTER this `start_alloc` returns. Cloned into the OUTBOUND accept
        // loop (which reads it per accept) and into the recorded intercept
        // (which `program_declared_peer_redirect` writes). `None` until the
        // seam records it; a leg-F connection only arrives once a redirect is
        // programmed, so the loop sees `Some(real_peer)` by then.
        let real_peer: Arc<Mutex<Option<SocketAddrV4>>> = Arc::new(Mutex::new(None));
        // Cooperative stop flag the accept loops observe between poll slices.
        let stop = Arc::new(AtomicBool::new(false));

        let outbound_task = self.spawn_accept_loop(
            spec.alloc.clone(),
            AcceptLeg::Outbound {
                listener: leg_f_listener,
                leg_f_addr,
                real_peer: Arc::clone(&real_peer),
            },
            Arc::clone(&enforced),
            Arc::clone(&stop),
        );
        let inbound_task = self.spawn_accept_loop(
            spec.alloc.clone(),
            AcceptLeg::Inbound { listener: inbound_listener },
            Arc::clone(&enforced),
            Arc::clone(&stop),
        );

        self.record_intercept_full(
            spec.alloc.clone(),
            outbound_tproxy_guard,
            tproxy_guard,
            vec![outbound_task, inbound_task],
            enforced,
            stop,
        );
    }

    /// Tear the alloc's intercept down. Drains the per-connection
    /// teardown set through `enforcement.teardown`, aborts the accept
    /// tasks, and drops the cgroup link + TPROXY guard (their `Drop`
    /// detaches the program / removes the nft rule). Idempotent — a
    /// stop for an unknown alloc is a no-op.
    pub fn stop_alloc(self: &Arc<Self>, alloc_id: &AllocationId) {
        let Some(intercept) = self.intercepts.lock().remove(alloc_id) else {
            return;
        };

        // Signal the blocking accept loops to exit cooperatively. They run on
        // `spawn_blocking` threads, so `JoinHandle::abort` alone cannot
        // interrupt a blocking `accept()`/`poll()` — the loops observe this
        // flag between bounded (200ms) poll slices and return. The abort()
        // below is still issued so the task is not re-polled once it yields.
        intercept.stop.store(true, Ordering::SeqCst);
        for task in &intercept.accept_tasks {
            task.abort();
        }

        // Drain the per-connection teardown set fail-closed. `teardown`
        // is async; spawn a detached task that tears down each handle so
        // `stop_alloc` (a sync lifecycle hook) does not block. The
        // cgroup link + TPROXY guard drop synchronously here (their
        // `Drop` detaches), which is correct: detaching the intercept
        // stops new connections immediately while in-flight ones are
        // torn down off-thread.
        let handles: Vec<EnforcedConnection> = std::mem::take(&mut intercept.enforced.lock());
        if !handles.is_empty() {
            let enforcement = Arc::clone(&self.enforcement);
            tokio::spawn(async move {
                for handle in handles {
                    if let Err(source) = enforcement.teardown(handle.clone()).await {
                        tracing::warn!(
                            name: "health.mtls.teardown_failed",
                            connection = %handle.id(),
                            error = %source,
                            "mTLS connection teardown failed on alloc stop"
                        );
                    }
                }
            });
        }
        // `intercept` (cgroup link + TPROXY guard) drops here → detach.
        drop(intercept);
    }

    /// Spawn the accept→`enforce` loop for one leg. Each accepted
    /// connection is built into an `InterceptedConnection`, `enforce`d,
    /// and its handle pushed into the alloc's teardown set.
    fn spawn_accept_loop(
        self: &Arc<Self>,
        alloc: AllocationId,
        leg: AcceptLeg,
        enforced: Arc<Mutex<Vec<EnforcedConnection>>>,
        stop: Arc<AtomicBool>,
    ) -> tokio::task::JoinHandle<()> {
        let worker = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            // The closure OWNS `alloc`/`leg`/`enforced`/`stop`; `accept_loop`
            // borrows them for the duration of the loop (it clones `alloc`
            // per connection and re-uses `leg`/`enforced`/`stop` by reference).
            worker.accept_loop(&alloc, &leg, &enforced, &stop);
        })
    }

    /// Blocking accept loop (the leg listeners are blocking
    /// `std::net::TcpListener`s — leg acquisition is a one-shot per
    /// intercepted connection, not an async pump). Each accept builds the
    /// `InterceptedConnection` and hands it to `enforce` on the tokio
    /// runtime; `enforce`'s own task owns the pumps + (B) self-teardown.
    /// Exits when `stop` is set (observed between bounded poll slices) so the
    /// loop does not outlive the alloc on a `spawn_blocking` thread.
    fn accept_loop(
        self: &Arc<Self>,
        alloc: &AllocationId,
        leg: &AcceptLeg,
        enforced: &Arc<Mutex<Vec<EnforcedConnection>>>,
        stop: &Arc<AtomicBool>,
    ) {
        loop {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            let built = match leg {
                AcceptLeg::Outbound { listener, leg_f_addr, real_peer } => {
                    let _ = leg_f_addr; // the redirect TARGET, never the dial target
                    // TRANSITIONAL (this whole declared-peer gate is DELETED in
                    // step 04-02 when the `MtlsResolve` consumer lands). As of
                    // the Path-A egress nft-TPROXY mechanism (D-TME-4, the
                    // RETIRED `cgroup_connect4_mtls` rewrite gone in step
                    // 04-01) the outbound orig-dst IS getsockname-recoverable
                    // from the accepted leg-F socket — `accept_outbound_leg`
                    // below builds `Routed::Outbound { peer }` from that
                    // recovered addr (symmetric with inbound TPROXY) and
                    // IGNORES the `real_peer` arg. The vestigial `real_peer`
                    // slot below is now ALWAYS `None` (nothing programs it
                    // since `program_declared_peer_redirect` was deleted in
                    // 04-01), so leg-F traffic currently fail-closed-drops
                    // until 04-02 wires the resolve consumer.
                    //
                    // Block until a connection is PENDING on the listener
                    // WITHOUT consuming it, THEN read the (vestigial)
                    // `real_peer` slot.
                    match await_pending_connection(listener, stop) {
                        ConnectionReady::Pending => {}
                        ConnectionReady::ListenerClosed | ConnectionReady::Stopped => return,
                    }
                    let Some(peer) = *real_peer.lock() else {
                        // A pending leg-F connection with no recorded declared
                        // peer — an anomaly (the seam records before it
                        // programs the redirect). Fail CLOSED: accept-and-drop
                        // so the workload's connection is closed and NO
                        // cleartext egresses — never self-loop to `leg_f_addr`.
                        match accept_drop_outbound(listener) {
                            AcceptOutcome::Dropped => {
                                tracing::warn!(
                                    name: "health.mtls.outbound_no_declared_peer",
                                    alloc = %alloc,
                                    "leg-F connection with no recorded declared peer; \
                                     dropped fail-closed (no cleartext, no self-loop)"
                                );
                                continue;
                            }
                            AcceptOutcome::ListenerClosed => return,
                        }
                    };
                    // The connection is pending; `accept_outbound_leg`'s
                    // internal `accept()` returns it immediately, built into
                    // `Routed::Outbound { peer }` from the getsockname-recovered
                    // orig-dst (D-TME-4, symmetric with inbound) — so `enforce`
                    // dials the REAL peer.
                    //
                    // TRANSITIONAL (03-02→04-02): the passed `peer` (from the
                    // declared-peer `real_peer` slot) is now IGNORED inside
                    // `accept_outbound_leg` (received as `_peer`); the routing
                    // fact comes from getsockname, not this arg. This
                    // declared-peer call-site is removed in step 04-02 when the
                    // resolve consumer orphans the declared-peer model.
                    accept_outbound_leg(listener, alloc.clone(), peer)
                }
                AcceptLeg::Inbound { listener } => {
                    // Poll for a pending connection (observing `stop`) before
                    // the blocking `accept()` inside `accept_inbound_leg`, so
                    // the inbound loop can also exit cooperatively on teardown
                    // rather than block on a stale listener fd forever.
                    match await_pending_connection(listener, stop) {
                        ConnectionReady::Pending => {}
                        ConnectionReady::ListenerClosed | ConnectionReady::Stopped => return,
                    }
                    accept_inbound_leg(listener, alloc.clone())
                }
            };
            let conn = match built {
                Ok(conn) => conn,
                Err(mtls_intercept::InterceptError::Accept { .. }) => {
                    // The listener was closed (alloc torn down / task
                    // aborted) — exit the loop cleanly.
                    return;
                }
                Err(source) => {
                    tracing::warn!(
                        name: "health.mtls.leg_acquire_failed",
                        alloc = %alloc,
                        error = %source,
                        "mTLS leg-acquire failed; skipping this connection"
                    );
                    continue;
                }
            };

            // Hand the intercepted connection to `enforce` on the tokio
            // runtime. `enforce` is the single fail-closed gate; on `Ok`
            // its handle joins the teardown set, on `Err` the leg is
            // already closed by the port and no cleartext egressed.
            let enforcement = Arc::clone(&self.enforcement);
            let enforced = Arc::clone(enforced);
            let alloc_for_log = alloc.clone();
            let handle = tokio::runtime::Handle::current();
            handle.spawn(async move {
                match enforcement.enforce(conn).await {
                    Ok(handle) => enforced.lock().push(handle),
                    Err(source) => {
                        tracing::warn!(
                            name: "health.mtls.enforce_failed",
                            alloc = %alloc_for_log,
                            error = %source,
                            "mTLS enforce refused the connection (fail-closed; no cleartext)"
                        );
                    }
                }
            });
        }
    }

    /// Record a fully-installed (outbound + inbound) intercept.
    fn record_intercept_full(
        &self,
        alloc: AllocationId,
        outbound_tproxy_guard: Option<TproxyInterceptGuard>,
        tproxy_guard: Option<TproxyInterceptGuard>,
        accept_tasks: Vec<tokio::task::JoinHandle<()>>,
        enforced: Arc<Mutex<Vec<EnforcedConnection>>>,
        stop: Arc<AtomicBool>,
    ) {
        self.intercepts.lock().insert(
            alloc,
            AllocIntercept {
                _outbound_tproxy_guard: outbound_tproxy_guard,
                _tproxy_guard: tproxy_guard,
                accept_tasks,
                stop,
                enforced,
            },
        );
    }
}

/// Which leg an accept loop is draining.
enum AcceptLeg {
    /// Outbound leg-F (workload-facing plaintext). `leg_f_addr` is the
    /// agent's own listener addr the kernel redirected the workload to —
    /// it is the redirect TARGET, never the dial target. `real_peer` is
    /// the shared slot the declared-peer seam records the workload's
    /// ORIGINAL destination into; the accept loop reads it to build
    /// `Routed::Outbound { peer: real_peer }` so `enforce` dials the REAL
    /// peer (not a self-loop back to `leg_f_addr`).
    Outbound {
        listener: std::net::TcpListener,
        leg_f_addr: SocketAddrV4,
        real_peer: Arc<Mutex<Option<SocketAddrV4>>>,
    },
    /// Inbound leg-C (client-facing, TPROXY-redirected). orig-dst is
    /// recovered via `getsockname` inside `accept_inbound_leg`.
    Inbound { listener: std::net::TcpListener },
}

/// Outcome of an accept-and-drop on the outbound leg when no declared peer
/// is recorded — distinguishes a dropped connection (fail-closed, continue
/// the loop) from a closed listener (alloc torn down, exit the loop).
enum AcceptOutcome {
    /// A connection was accepted and immediately dropped (fail-closed).
    Dropped,
    /// The listener was closed (alloc torn down / task aborted).
    ListenerClosed,
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

/// Block until a connection is PENDING on `listener` without accepting it,
/// so the caller can read the declared-peer slot AFTER a connection has
/// arrived (closing the stale-read window) and THEN accept. Returns
/// [`ConnectionReady::ListenerClosed`] when the listener fd is invalidated
/// (the alloc was torn down and the listener dropped), or
/// [`ConnectionReady::Stopped`] when the cooperative `stop` flag is observed
/// set between poll slices. Polls in bounded (200ms) slices so both a
/// torn-down listener and a stop signal are observed promptly rather than
/// blocking forever on a stale fd.
fn await_pending_connection(
    listener: &std::net::TcpListener,
    stop: &AtomicBool,
) -> ConnectionReady {
    use std::os::fd::AsRawFd as _;
    let fd = listener.as_raw_fd();
    loop {
        if stop.load(Ordering::SeqCst) {
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

/// Accept one connection on the outbound leg-F listener and drop it
/// immediately (fail-closed). Used when a leg-F connection arrives with no
/// recorded declared peer — the connection is closed with NO cleartext
/// egress and NO self-loop dial. Returns [`AcceptOutcome::ListenerClosed`]
/// when the listener has been closed (the alloc was torn down).
fn accept_drop_outbound(listener: &std::net::TcpListener) -> AcceptOutcome {
    match listener.accept() {
        // The accepted stream drops at end of scope → the workload's leg-F
        // connection is closed. No `enforce`, no leg-B dial, no cleartext.
        Ok(_) => AcceptOutcome::Dropped,
        Err(_) => AcceptOutcome::ListenerClosed,
    }
}

/// Narrow `SocketAddr → SocketAddrV4` projection (the legs are bound on
/// IPv4 loopback; single-node Phase-1 scope is IPv4-only).
const fn socketaddr_v4(addr: std::net::SocketAddr) -> Option<SocketAddrV4> {
    match addr {
        std::net::SocketAddr::V4(v4) => Some(v4),
        std::net::SocketAddr::V6(_) => None,
    }
}

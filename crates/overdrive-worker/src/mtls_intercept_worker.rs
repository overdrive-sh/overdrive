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
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnection, InterceptedConnection, MtlsEnforcement, Routed,
};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use parking_lot::Mutex;

use crate::mtls_intercept::{
    InterceptError, TproxyInterceptGuard, accept_inbound_leg, accept_outbound_and_recover_orig_dst,
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
    /// The per-connection enrollment-resolve port (`ServiceBackendsResolve` in
    /// production; `SimMtlsResolve` under test composition; ADR-0071 fact 4,
    /// the #178 anti-corruption boundary). The outbound accept loop resolves
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
    /// Per-alloc teardown bookkeeping (D-MTLS-16). `BTreeMap` per
    /// `.claude/rules/development.md` § "Ordered-collection choice" — the
    /// set is drained deterministically on stop.
    intercepts: Mutex<BTreeMap<AllocationId, AllocIntercept>>,
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
    #[must_use]
    pub fn new(
        enforcement: Arc<dyn MtlsEnforcement>,
        resolve: Arc<dyn MtlsResolve>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self { enforcement, resolve, _clock: clock, intercepts: Mutex::new(BTreeMap::new()) }
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
        // Load-bearing: it is the `agent_leg_f_port` the egress rule points at
        // (`install_outbound_tproxy(host_veth, leg_f_addr.port())` below). It is
        // NOT a dial target — the dial peer is the per-connection RESOLVED
        // backend addr (04-02), recovered in the accept loop, never this slot.
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
        // integration tests (which supply a real, distinct virt). A `virt`
        // synthesised from the agent's own ephemeral leg-C port (the prior
        // shape) installed a self-referential rule that matched no real inbound
        // connection — inert in production while reading as "inbound mTLS
        // works". (The OUTBOUND direction is NOT #178-gated: it resolves
        // orig_dst per-connection via the `MtlsResolve` consumer wired in the
        // accept loop below — see [`Self::handle_outbound`].)
        self.spawn_legs_and_record(
            spec,
            outbound_tproxy_guard,
            None,
            leg_f_listener,
            inbound_listener,
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
        outbound_tproxy_guard: Option<TproxyInterceptGuard>,
        tproxy_guard: Option<TproxyInterceptGuard>,
        leg_f_listener: std::net::TcpListener,
        inbound_listener: std::net::TcpListener,
    ) {
        let enforced: Arc<Mutex<Vec<EnforcedConnection>>> = Arc::new(Mutex::new(Vec::new()));
        // Cooperative stop flag the accept loops observe between poll slices.
        let stop = Arc::new(AtomicBool::new(false));

        let outbound_task = self.spawn_accept_loop(
            spec.alloc.clone(),
            AcceptLeg::Outbound { listener: leg_f_listener },
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
            match leg {
                AcceptLeg::Outbound { listener } => {
                    // Poll for a pending connection (observing `stop`) before the
                    // blocking accept, so the loop exits cooperatively on teardown.
                    match await_pending_connection(listener, stop) {
                        ConnectionReady::Pending => {}
                        ConnectionReady::ListenerClosed | ConnectionReady::Stopped => return,
                    }
                    // Accept leg-F + recover the dialed orig_dst, then run the
                    // per-connection resolve consumer. A closed listener (alloc
                    // torn down) exits the loop; any other leg-acquire fault skips
                    // this connection.
                    match accept_outbound_and_recover_orig_dst(listener) {
                        Ok((leg_f, orig_dst)) => {
                            self.handle_outbound(alloc, leg_f, orig_dst, enforced);
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
                    match await_pending_connection(listener, stop) {
                        ConnectionReady::Pending => {}
                        ConnectionReady::ListenerClosed | ConnectionReady::Stopped => return,
                    }
                    match accept_inbound_leg(listener, alloc.clone()) {
                        Ok(conn) => self.spawn_enforce(alloc, conn, enforced),
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
    ///   (`expected_peer` stays `None` until #178 — v1 authn-only) and hand it
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
        enforced: &Arc<Mutex<Vec<EnforcedConnection>>>,
    ) {
        // The resolve port is async; this loop runs on a `spawn_blocking`
        // thread (a blocking-pool thread, not a runtime worker), so
        // `Handle::block_on` is valid here — it drives the resolve future to
        // completion before the 3-arm decision.
        let runtime = tokio::runtime::Handle::current();
        let resolution = match runtime.block_on(self.resolve.resolve(orig_dst)) {
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
                    // v1 authn-only (F5 / #178): the expected-peer SAN-match is
                    // supplied downstream by east-west SPIFFE-ID resolution.
                    expected_peer: None,
                };
                self.spawn_enforce(alloc, conn, enforced);
            }
            OutboundAction::PassThrough => {
                // NonMesh → cleartext pass-through, by design: relay leg-F to a
                // cleartext dial of orig_dst. NO mTLS, NO enforce.
                spawn_cleartext_passthrough(&runtime, alloc.clone(), leg_f, orig_dst);
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

    /// Hand an [`InterceptedConnection`] to `enforce` on the tokio runtime.
    /// `enforce` is the single fail-closed gate; on `Ok` its handle joins the
    /// alloc's teardown set, on `Err` the port has already closed the leg and no
    /// cleartext egressed.
    fn spawn_enforce(
        self: &Arc<Self>,
        alloc: &AllocationId,
        conn: InterceptedConnection,
        enforced: &Arc<Mutex<Vec<EnforcedConnection>>>,
    ) {
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
/// Spawned as a detached blocking task so it does not stall the accept loop; a
/// dial failure closes leg-F (the upstream is unreachable — nothing to relay).
fn spawn_cleartext_passthrough(
    runtime: &tokio::runtime::Handle,
    alloc: AllocationId,
    leg_f: std::os::fd::OwnedFd,
    orig_dst: SocketAddrV4,
) {
    runtime.spawn_blocking(move || {
        let upstream = match std::net::TcpStream::connect(orig_dst) {
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
                drop(leg_f);
                return;
            }
        };
        // `OwnedFd → TcpStream` is the safe stdlib conversion (RAII close on
        // drop); `leg_f` is the accepted TCP socket handed over by
        // `accept_outbound_and_recover_orig_dst`, so there is exactly one owner.
        let downstream = std::net::TcpStream::from(leg_f);
        if let Err(source) = relay_cleartext(&downstream, &upstream) {
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
}

/// Minimal bidirectional cleartext relay between the captured workload leg
/// (`downstream`) and the dialed non-mesh upstream. Returns when EITHER side
/// reaches EOF / errors (the connection is done). One thread copies
/// down→up; this thread copies up→down. NO crypto — cleartext both ways, by
/// design (the `NonMesh` classification arm).
fn relay_cleartext(
    downstream: &std::net::TcpStream,
    upstream: &std::net::TcpStream,
) -> std::io::Result<()> {
    let mut down_to_up = downstream.try_clone()?;
    let mut up_for_writer = upstream.try_clone()?;
    let copy_thread = std::thread::spawn(move || {
        // EOF / error on either end ends the copy; the result is informational.
        let _ = std::io::copy(&mut down_to_up, &mut up_for_writer);
        // Half-close the upstream write side so the peer sees EOF.
        let _ = up_for_writer.shutdown(std::net::Shutdown::Write);
    });
    let mut up_to_down = upstream.try_clone()?;
    let mut down_for_writer = downstream.try_clone()?;
    let _ = std::io::copy(&mut up_to_down, &mut down_for_writer);
    let _ = down_for_writer.shutdown(std::net::Shutdown::Write);
    // Best-effort join so the relay task does not leak the copy thread.
    let _ = copy_thread.join();
    Ok(())
}

/// Narrow `SocketAddr → SocketAddrV4` projection (the legs are bound on
/// IPv4 loopback; single-node Phase-1 scope is IPv4-only).
const fn socketaddr_v4(addr: std::net::SocketAddr) -> Option<SocketAddrV4> {
    match addr {
        std::net::SocketAddr::V4(v4) => Some(v4),
        std::net::SocketAddr::V6(_) => None,
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
    //! (`expected_peer` is `None` until #178).

    use std::collections::BTreeMap;
    use std::io::{Read as _, Write as _};
    use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use overdrive_core::AllocationId;
    use overdrive_core::traits::clock::Clock;
    use overdrive_core::traits::mtls_enforcement::{
        EnforcedConnection, EnforcedConnectionId, InterceptedConnection, MtlsEnforcement,
        PumpLiveness, Routed,
    };
    use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve, ResolvedBackend};
    use overdrive_sim::adapters::SimMtlsResolve;
    use overdrive_sim::adapters::clock::SimClock;
    use parking_lot::Mutex;

    use super::{MtlsInterceptWorker, OutboundAction, decide_outbound};

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
        Arc::new(MtlsInterceptWorker::new(enforcement, resolve, clock))
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
    ) -> Arc<Mutex<Vec<EnforcedConnection>>> {
        let enforced: Arc<Mutex<Vec<EnforcedConnection>>> = Arc::new(Mutex::new(Vec::new()));
        let worker = Arc::clone(worker);
        let enforced_for_task = Arc::clone(&enforced);
        tokio::task::spawn_blocking(move || {
            worker.handle_outbound(&alloc, leg_f, orig_dst, &enforced_for_task);
        })
        .await
        .expect("handle_outbound blocking task joins");
        enforced
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

        let enforced = run_handle_outbound(&worker, alloc("alloc-mesh"), leg_f, orig_dst).await;

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
        // (inside the spawned task, after the spy recorded the call) — spin
        // (bounded) until it lands so the assertion does not race the push.
        for _ in 0..1000 {
            if enforced.lock().len() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(enforced.lock().len(), 1, "the enforced handle joins the teardown set");
    }

    /// Spin (bounded — no unbounded wait) until `calls` holds at least `n`
    /// recorded `enforce` calls, then return a clone. The enforce dispatch is a
    /// spawned task; this closes the race between "handle_outbound returned" and
    /// "the spawned enforce ran" without a fixed sleep.
    async fn wait_for_calls(calls: &Arc<Mutex<Vec<EnforceCall>>>, n: usize) -> Vec<EnforceCall> {
        for _ in 0..1000 {
            if calls.lock().len() >= n {
                break;
            }
            tokio::task::yield_now().await;
        }
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
        let _enforced =
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

        let _enforced = run_handle_outbound(&worker, alloc("alloc-unreach"), leg_f, orig_dst).await;

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

        let _enforced = run_handle_outbound(&worker, alloc("alloc-fault"), leg_f, orig_dst).await;

        client.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let mut buf = [0u8; 1];
        let n = client.read(&mut buf).expect("read on a closed leg returns Ok(0) (EOF)");
        assert_eq!(n, 0, "a resolve store-fault must close leg-F fail-closed (no cleartext)");
        assert!(calls.lock().is_empty(), "a faulted resolve must NOT call enforce");
    }
}

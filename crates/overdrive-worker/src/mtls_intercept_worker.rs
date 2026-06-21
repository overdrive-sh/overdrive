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
    /// silently corrupt the #178 inbound-redirect read (D-MTLS-18 site 3).
    #[error("mTLS leg-C listener address capture failed: {source}")]
    LegCLocalAddr {
        #[source]
        source: std::io::Error,
    },
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
            Self::OutboundTproxyInstall(_) => "outbound_tproxy_install",
            Self::LegFBind(_) | Self::LegFLocalAddr { .. } => "leg_f_bind",
            Self::LegCLocalAddr { .. }
            | Self::Inbound(InterceptError::TransparentListener { .. }) => {
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
    /// through `enforcement.teardown` on stop. An [`EnforcedSet`] (not a raw
    /// `Arc<Mutex<Vec>>`): its seal+drain is ATOMIC with push, so a
    /// `spawn_enforce` task that wins the handshake race AFTER `stop_alloc`
    /// has sealed the set is handed its handle back to tear down inline
    /// (fail-closed), instead of orphaning a live kTLS-armed handle into a
    /// vec nothing will drain again. Closes the `stop_alloc`-drain vs
    /// `spawn_enforce`-push TOCTOU.
    enforced: EnforcedSet,
}

/// Per-alloc enforced-connection set with an atomic seal+drain.
///
/// A push to a *sealed* set hands the handle back so the caller tears it
/// down inline (fail-closed) instead of orphaning it. This closes the
/// race between [`MtlsInterceptWorker::stop_alloc`] draining the set and an
/// in-flight [`MtlsInterceptWorker::spawn_enforce`] task pushing a freshly
/// enforced handle: previously the drain was a `std::mem::take` on a raw
/// `Arc<Mutex<Vec>>` while "stop accepting handles" lived in a *different*
/// primitive (the `stop` `AtomicBool`), so seal-and-drain and push were not
/// atomic under one lock (a TOCTOU — `.claude/rules/development.md` §
/// "Check-and-act must be atomic (no TOCTOU)"). Here the `sealed` flag and
/// the handle collection live under ONE `parking_lot::Mutex`, so the
/// check-and-act is a single locked op.
#[derive(Clone)]
struct EnforcedSet {
    inner: Arc<Mutex<EnforcedState>>,
}

/// The `EnforcedSet` payload: the seal flag and the handle collection,
/// held together under one lock so push and seal+drain cannot interleave.
struct EnforcedState {
    sealed: bool,
    handles: Vec<EnforcedConnection>,
}

impl EnforcedSet {
    /// A fresh, open (unsealed), empty set.
    fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(EnforcedState { sealed: false, handles: Vec::new() })) }
    }

    /// Atomic: push the handle iff the set is open; if the set is SEALED,
    /// return the handle so the caller tears it down inline (fail-closed).
    /// One locked op — no separate "is it sealed?" check, so no TOCTOU
    /// window between the check and the push.
    #[must_use]
    fn push_or_reject(&self, handle: EnforcedConnection) -> Option<EnforcedConnection> {
        let mut st = self.inner.lock();
        if st.sealed {
            Some(handle)
        } else {
            st.handles.push(handle);
            None
        }
    }

    /// Atomic seal+drain: mark the set sealed (future pushes are rejected by
    /// [`push_or_reject`](Self::push_or_reject)) and return the handles to
    /// tear down, in one locked op. Idempotent — a second call drains an
    /// already-empty, already-sealed set and returns `Vec::new()`.
    fn seal_and_drain(&self) -> Vec<EnforcedConnection> {
        let mut st = self.inner.lock();
        st.sealed = true;
        std::mem::take(&mut st.handles)
    }

    /// Test-only count of currently-held (not-yet-drained) handles. Used by
    /// the per-arm resolve-consumer tests to observe that an enforced handle
    /// joined the set; NOT production surface (no `pub`, `#[cfg(test)]`).
    #[cfg(test)]
    fn held_count(&self) -> usize {
        self.inner.lock().handles.len()
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
    /// listener) when the corresponding install step fails. Additionally
    /// [`MtlsInterceptInstallError::LegFLocalAddr`] (site 2) /
    /// [`MtlsInterceptInstallError::LegCLocalAddr`] (site 3) when a listener
    /// binds but its bound-address capture (`local_addr()` / getsockname) fails:
    /// the install fails CLOSED rather than defaulting the redirect target to a
    /// broken port 0 (D-MTLS-18). Each source `Display` names the privilege /
    /// kernel-feature remediation an operator acts on. (The inbound nft-TPROXY
    /// rule is #178-deferred; it is not installed here, so there is no site-4
    /// failure.)
    #[allow(
        clippy::similar_names,
        reason = "leg_c_addr (inbound) and leg_f_addr (outbound) are the deliberate \
                  symmetric vocabulary of this crate (D-TME-13 naming decision); the \
                  similarity is the point — leg-C and leg-F are the two TPROXY-divert \
                  targets, and renaming either to dodge the lint would break the \
                  established leg-C/leg-F naming the struct comments and AcceptLeg variants use"
    )]
    pub fn start_alloc(
        self: &Arc<Self>,
        spec: &AllocationSpec,
    ) -> Result<(), MtlsInterceptInstallError> {
        // Re-fire safety: drop any prior intercept for this alloc first
        // (Restart reuses the alloc id).
        self.stop_alloc(&spec.alloc);

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
        let leg_f_listener =
            match make_transparent_listener(SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0)) {
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
        let leg_f_addr = project_listener_v4(
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
        // Capture leg-C's bound addr BEFORE the listener moves into the spawned
        // inbound `accept_loop` — mirroring the leg-F capture pattern above
        // (:378-382; leg-F's addr is an inline local consumed inline, with no
        // public accessor). Retained on `AllocIntercept` so `leg_c_addr(&self,
        // alloc)` stays a pure in-memory read (the listener is consumed by the
        // accept task; its `local_addr()` is no longer reachable from the
        // worker). It is the EXACT addr the spawned inbound accept loop accepts
        // on, so a redirect installed at it lands on the production inbound leg
        // (D-TME-13). #178 is *expected* to reuse this read for its production
        // inbound-redirect install, pending that install's site/timing design;
        // if #178 mirrors leg-F and installs in `start_alloc` it would read an
        // inline `leg_c_addr` local, NOT `self.leg_c_addr(alloc)`.
        // Fail-closed (D-MTLS-18 site 3): a `local_addr()` getsockname error
        // surfaces as the typed `LegCLocalAddr` rather than recording a port-0
        // leg-C addr that would silently corrupt the #178 inbound-redirect read.
        // `outbound_tproxy_guard` + `leg_f_listener` (the guards acquired so far)
        // drop on the `?` early return → remove the egress rule / close leg-F.
        let leg_c_addr = project_listener_v4(
            inbound_listener.local_addr(),
            MtlsInterceptInstallError::leg_c_local_addr,
        )?;

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
            leg_c_addr,
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
        leg_c_addr: SocketAddrV4,
    ) {
        let enforced = EnforcedSet::new();
        // Cooperative stop flag the accept loops observe between poll slices.
        let stop = Arc::new(AtomicBool::new(false));

        let outbound_task = self.spawn_accept_loop(
            spec.alloc.clone(),
            AcceptLeg::Outbound { listener: leg_f_listener },
            enforced.clone(),
            Arc::clone(&stop),
        );
        let inbound_task = self.spawn_accept_loop(
            spec.alloc.clone(),
            AcceptLeg::Inbound { listener: inbound_listener },
            enforced.clone(),
            Arc::clone(&stop),
        );

        self.record_intercept_full(
            spec.alloc.clone(),
            outbound_tproxy_guard,
            tproxy_guard,
            leg_c_addr,
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

        // Seal-and-drain the per-connection teardown set fail-closed.
        // `teardown` is async; spawn a detached task that tears down each
        // drained handle so `stop_alloc` (a sync lifecycle hook) does not
        // block. The cgroup link + TPROXY guard drop synchronously here
        // (their `Drop` detaches), which is correct: detaching the
        // intercept stops new connections immediately while in-flight ones
        // are torn down off-thread.
        //
        // The SEAL is the load-bearing change (not the prior `mem::take`):
        // it atomically rejects any FUTURE push from a `spawn_enforce` task
        // still in flight (its `enforce` awaits a seconds-wide TLS
        // handshake + kTLS arm), so a handle produced AFTER this drain is
        // handed back to `spawn_enforce` and torn down inline rather than
        // orphaned into a vec nothing drains again. Closes the
        // `stop_alloc`-drain vs `spawn_enforce`-push TOCTOU.
        let handles: Vec<EnforcedConnection> = intercept.enforced.seal_and_drain();
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
        enforced: EnforcedSet,
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
        enforced: &EnforcedSet,
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
        enforced: &EnforcedSet,
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
        enforced: &EnforcedSet,
    ) {
        let enforcement = Arc::clone(&self.enforcement);
        let enforced = enforced.clone();
        let alloc_for_log = alloc.clone();
        let handle = tokio::runtime::Handle::current();
        handle.spawn(async move {
            match enforcement.enforce(conn).await {
                Ok(handle) => {
                    // Atomic push-or-reject: if `stop_alloc` sealed the set
                    // while this `enforce` was awaiting its handshake, the
                    // handle is handed back here and torn down INLINE
                    // (fail-closed) — never orphaned into a drained vec. The
                    // `push_or_reject` call returns an OWNED `Option` (the
                    // lock is released inside the method) BEFORE any `.await`,
                    // so the lock is never held across the `teardown` await
                    // (`.claude/rules/development.md` § "Concurrency & async").
                    if let Some(orphan) = enforced.push_or_reject(handle) {
                        let orphan_id = orphan.id().clone();
                        if let Err(source) = enforcement.teardown(orphan).await {
                            tracing::warn!(
                                name: "health.mtls.teardown_failed",
                                alloc = %alloc_for_log,
                                connection = %orphan_id,
                                error = %source,
                                "mTLS enforce won after alloc stop; inline fail-closed teardown failed"
                            );
                        }
                    }
                }
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
    #[allow(
        clippy::too_many_arguments,
        reason = "private bookkeeping constructor: one arg per AllocIntercept field \
                  (the two tproxy guards, leg_c_addr per D-TME-13, accept_tasks, enforced, \
                  stop); bundling them into a params struct would just move the same field \
                  list one indirection away with no clarity gain — the call site is the \
                  single internal caller in spawn_legs_and_record"
    )]
    fn record_intercept_full(
        &self,
        alloc: AllocationId,
        outbound_tproxy_guard: Option<TproxyInterceptGuard>,
        tproxy_guard: Option<TproxyInterceptGuard>,
        leg_c_addr: SocketAddrV4,
        accept_tasks: Vec<tokio::task::JoinHandle<()>>,
        enforced: EnforcedSet,
        stop: Arc<AtomicBool>,
    ) {
        self.intercepts.lock().insert(
            alloc,
            AllocIntercept {
                _outbound_tproxy_guard: outbound_tproxy_guard,
                _tproxy_guard: tproxy_guard,
                leg_c_addr,
                accept_tasks,
                stop,
                enforced,
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
    /// # `pub` legitimacy (operability, independent of #178)
    ///
    /// This is a production-legitimate diagnostic/observability surface in its own
    /// right: an operator/diagnostic caller can ask the worker "where is this
    /// alloc's inbound intercept listening?" — a genuine operability/analysability
    /// question for a security control that silently terminates client mTLS. That
    /// alone justifies `pub`; it is NOT a test-only hook. #178 (the production
    /// inbound-redirect install) is *expected* to reuse this read pending its
    /// install site/timing design — but whether #178 consumes `self.leg_c_addr(..)`
    /// or an inline `leg_c_addr` local in `start_alloc` (mirroring the leg-F
    /// capture pattern, which reads its port via the inline local
    /// `leg_f_addr.port()` and exposes no accessor) is #178's unresolved design.
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
    /// # Identity boundary (authn-only v1 — ADR-0071 / D-TME-8 / #178)
    ///
    /// This exposes ONLY a bound socket address — NO SVID, NO key, NO identity
    /// material of any kind. It is a bound-addr read, not an identity read. Workloads
    /// hold nothing and the worker exposes nothing about *who* leg-C will mTLS as; the
    /// expected-SVID / intended-peer join is strictly #178's (the
    /// `MtlsResolve.expected_svid` anti-corruption field, `None` in v1). The accessor
    /// is therefore inside the authn-only v1 boundary by construction.
    #[must_use]
    pub fn leg_c_addr(&self, alloc: &AllocationId) -> Option<SocketAddrV4> {
        self.intercepts.lock().get(alloc).map(|i| i.leg_c_addr)
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

    use super::{EnforcedSet, MtlsInterceptWorker, OutboundAction, decide_outbound};

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
    ) -> EnforcedSet {
        let enforced = EnforcedSet::new();
        let worker = Arc::clone(worker);
        let enforced_for_task = enforced.clone();
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

        let tproxy = || InterceptError::TproxyInstall { reason: "boom".to_owned() };
        let transparent = || InterceptError::TransparentListener {
            addr: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };

        let cases: [(MtlsInterceptInstallError, &str); 4] = [
            (MtlsInterceptInstallError::OutboundTproxyInstall(tproxy()), "outbound_tproxy_install"),
            // The leg-F bind site (site 2). Its inner `InterceptError` is what
            // `make_transparent_listener` produces — this change's surface.
            (MtlsInterceptInstallError::LegFBind(transparent()), "leg_f_bind"),
            // Inbound leg-C transparent-listener bind failure → the leg-C label.
            (MtlsInterceptInstallError::Inbound(transparent()), "leg_c_transparent_listener"),
            // Any other inbound `InterceptError` is the site-4 nft-TPROXY install.
            (MtlsInterceptInstallError::Inbound(tproxy()), "inbound_tproxy"),
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

    // ---- EnforcedSet: the atomic seal+drain primitive (pure unit) ----------

    /// Build an `EnforcedConnection` with a stable, asserter-readable id so a
    /// drained / handed-back handle can be matched by id.
    fn enforced_conn(alloc_name: &str, counter: u64) -> EnforcedConnection {
        EnforcedConnection::new(EnforcedConnectionId::new(alloc(alloc_name), counter))
    }

    /// The `EnforcedSet` seal+drain contract — the mutation-gate target for the
    /// fix. Each clause is the inverse of a way the race could leak:
    /// - an OPEN `push_or_reject` STORES the handle and returns `None` (the
    ///   handle is retained for the eventual drain);
    /// - `seal_and_drain` RETURNS the stored handles AND seals the set;
    /// - a SEALED `push_or_reject` HANDS THE HANDLE BACK (`Some`) and does NOT
    ///   store it — the caller tears it down inline rather than orphaning it
    ///   (this clause is the one a `if st.sealed` mutation flips, and the one
    ///   the whole fix exists to guarantee);
    /// - a second `seal_and_drain` is idempotent (drains empty).
    #[test]
    fn enforced_set_seals_then_hands_back_pushes_atomically() {
        let set = EnforcedSet::new();

        // OPEN: push stores and returns None.
        let h0 = enforced_conn("set-alloc", 0);
        let h1 = enforced_conn("set-alloc", 1);
        assert!(
            set.push_or_reject(h0.clone()).is_none(),
            "an open set must STORE the handle and return None",
        );
        assert!(
            set.push_or_reject(h1.clone()).is_none(),
            "an open set must STORE the second handle and return None",
        );
        assert_eq!(set.held_count(), 2, "both pushes are retained while the set is open");

        // SEAL + DRAIN: returns exactly the stored handles, in push order.
        let drained = set.seal_and_drain();
        let drained_ids: Vec<_> = drained.iter().map(|h| h.id().clone()).collect();
        assert_eq!(
            drained_ids,
            vec![h0.id().clone(), h1.id().clone()],
            "seal_and_drain must return exactly the handles that were pushed",
        );
        assert_eq!(set.held_count(), 0, "the set is empty after draining");

        // SEALED: a post-seal push is HANDED BACK (Some) and NOT stored.
        let late = enforced_conn("set-alloc", 2);
        let handed_back = set.push_or_reject(late.clone());
        assert_eq!(
            handed_back.map(|h| h.id().clone()),
            Some(late.id().clone()),
            "a push to a SEALED set must hand the SAME handle back (fail-closed inline teardown)",
        );
        assert_eq!(
            set.held_count(),
            0,
            "a rejected push must NOT be stored — otherwise it orphans into a drained set",
        );

        // Idempotent: a second seal_and_drain drains an empty, sealed set.
        assert!(
            set.seal_and_drain().is_empty(),
            "a second seal_and_drain drains an already-sealed, already-empty set",
        );
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
        counter: std::sync::atomic::AtomicU64,
    }

    impl GatedEnforcement {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                entered: Arc::new(tokio::sync::Notify::new()),
                release: Arc::new(tokio::sync::Notify::new()),
                torn_down: Arc::new(Mutex::new(Vec::new())),
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

    /// REGRESSION (P1, GH #26): a `spawn_enforce` task that wins its handshake
    /// AFTER `stop_alloc` has drained the alloc's teardown set must NOT orphan
    /// its kTLS-armed handle — the handle MUST still be torn down (fail-closed).
    ///
    /// Drives the real production path: `record_intercept_full` registers an
    /// alloc sharing an [`EnforcedSet`]; `spawn_enforce` fires an in-flight
    /// `enforce` (gated mid-handshake); `stop_alloc` then seal-and-drains (the
    /// set is still empty — the handle has not been pushed yet); the gate is
    /// released so `enforce` completes and pushes. The assertion: `teardown`
    /// was called for that connection.
    ///
    /// Against the pre-fix code (raw `mem::take` drain, push gated by the
    /// separate `stop` flag the detached enforce task never reads) the
    /// post-drain push lands in a vec nothing drains again → `teardown` is
    /// never called → the bounded wait times out → RED. With the seal+drain
    /// fix the sealed set hands the handle back to `spawn_enforce`, which tears
    /// it down inline → GREEN.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn enforce_winning_after_stop_alloc_drain_is_torn_down_not_orphaned() {
        let spy = GatedEnforcement::new();
        let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
        let resolve =
            resolve_scripting(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0), MtlsResolution::NonMesh);
        let enforcement: Arc<dyn MtlsEnforcement> = Arc::clone(&spy) as Arc<dyn MtlsEnforcement>;
        let worker = Arc::new(MtlsInterceptWorker::new(enforcement, resolve, clock));

        let the_alloc = alloc("alloc-orphan-race");
        // Register an alloc that shares `enforced` — the SAME set spawn_enforce
        // pushes into and stop_alloc drains. No real listeners/guards (None /
        // empty); we drive the enforce + stop path directly, not start_alloc
        // (which would bind real IP_TRANSPARENT listeners → needs root).
        let enforced = EnforcedSet::new();
        worker.record_intercept_full(
            the_alloc.clone(),
            None,
            None,
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0),
            vec![],
            enforced.clone(),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
        worker.spawn_enforce(&the_alloc, conn, &enforced);

        // Wait until enforce is in flight (blocked on the gate) — the handle is
        // NOT yet pushed, so stop_alloc's drain sees an empty set.
        tokio::time::timeout(Duration::from_secs(5), spy.entered())
            .await
            .expect("enforce must enter (in-flight) within 5s");

        // T2: stop_alloc seal-and-drains the (still-empty) set.
        worker.stop_alloc(&the_alloc);

        // T3: release the gate → enforce completes and attempts its push into
        // the now-sealed set.
        spy.release();

        // The handle produced post-drain MUST be torn down (handed back to
        // spawn_enforce by the sealed set, torn down inline). Pre-fix it is
        // orphaned and teardown is never called → this bounded wait times out.
        wait_until("post-drain enforce handle is torn down (fail-closed)", || {
            !spy.torn_down().is_empty()
        })
        .await;

        let recorded = spy.torn_down();
        assert_eq!(
            recorded.len(),
            1,
            "the post-drain enforce handle must be torn down exactly once (fail-closed), not orphaned",
        );
        assert_eq!(
            recorded[0].alloc(),
            &the_alloc,
            "the torn-down handle must be the alloc's enforced connection",
        );
    }
}

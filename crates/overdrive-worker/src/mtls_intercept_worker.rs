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
//!   `Running` row). Attaches `cgroup_connect4_mtls` to the allocation's
//!   own `.scope` cgroup (the F5-exempt per-workload subtree,
//!   [`MtlsDataplane::attach_alloc`]), stands up the agent's leg-F
//!   (outbound, plaintext) + leg-C (inbound, `IP_TRANSPARENT`) listeners,
//!   and spawns the accept→`enforce` tasks. It programs **NEITHER** of the
//!   two east-west service-resolution facts v1 has no production source for:
//!   not the OUTBOUND `MTLS_REDIRECT_DEST` redirect, and not the INBOUND
//!   nft-TPROXY rule (whose match key is the server workload's logical virt
//!   address). Both are [#178](https://github.com/overdrive-sh/overdrive/issues/178)
//!   — see the module-level "DECLARED-PEER" note below.
//! - [`stop_alloc`](MtlsInterceptWorker::stop_alloc) — fired at the
//!   action-shim's `on_alloc_terminal` site. Drains the alloc's
//!   per-connection teardown set (`enforcement.teardown`), aborts the
//!   accept tasks, drops the `MtlsCgroupLink` (detach the cgroup
//!   program), and drops the `TproxyInterceptGuard` (remove the nft
//!   rule/route). Idempotent.
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
//! ## DECLARED-PEER scoping (v1 authn-only, #178 upgrade)
//!
//! The OUTBOUND intercept is **per-destination**: the
//! `cgroup_connect4_mtls` program rewrites `connect(real_peer)` only when
//! `MTLS_REDIRECT_DEST[real_peer]` is programmed (on a map MISS the
//! `connect` passes through unchanged). v1 has NO production source for
//! "the set of peers this workload will dial" — that enumeration is
//! east-west service resolution
//! ([#178](https://github.com/overdrive-sh/overdrive/issues/178) /
//! [#61](https://github.com/overdrive-sh/overdrive/issues/61), DEFERRED).
//! So `start_alloc` does NOT program the redirect; the per-alloc
//! [`MtlsDataplane`] handle is exposed (under `integration-tests`) so the
//! e2e activation gate can program the single declared-peer entry as the
//! #178 stand-in.
//!
//! The INBOUND nft-TPROXY rule is deferred **symmetrically**: its match key
//! is the server workload's logical (virt) address — the loopback addr/port
//! clients dial — which is the same #178 east-west fact with no v1 production
//! source. So `start_alloc` installs NO inbound TPROXY rule (it records
//! `tproxy_guard = None`); the [`install_inbound_tproxy`](crate::mtls_intercept::install_inbound_tproxy)
//! free function stays the named #178 production-install site, exercised today
//! only by the worker integration tests (which supply a real, distinct virt).
//! Everything else (load + attach + leg-F + leg-C listeners + both accept
//! loops + `enforce` + the wire) is production.

use std::collections::BTreeMap;
use std::net::SocketAddrV4;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use overdrive_core::AllocationId;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::AllocationSpec;
use overdrive_core::traits::mtls_enforcement::{EnforcedConnection, MtlsEnforcement};
use overdrive_dataplane::mtls::{MtlsCgroupLink, MtlsDataplane, MtlsDataplaneError};
use parking_lot::Mutex;

use crate::cgroup_manager::CgroupPath;
use crate::mtls_intercept::{
    self, InterceptError, TproxyInterceptGuard, accept_inbound_leg, accept_outbound_leg,
    make_transparent_listener,
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
/// errors the install steps already produce
/// ([`MtlsDataplaneError`] for the cgroup attach, [`InterceptError`] for the
/// leg-C transparent listener) which the worker previously discarded. Each
/// source `Display` names the privilege / kernel-feature remediation an
/// operator acts on. (The inbound nft-TPROXY rule install is #178-deferred —
/// see the module DECLARED-PEER note — so it is not an install step and has no
/// failure site here; the [`InterceptError::TproxyInstall`] variant still
/// flows through `Inbound` from the [`install_inbound_tproxy`](crate::mtls_intercept::install_inbound_tproxy)
/// free function's own callers.)
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MtlsInterceptInstallError {
    /// OUTBOUND `cgroup_connect4_mtls` attach to the alloc `.scope` failed
    /// (site 1). Source `Display` names the `CAP_BPF` / `CAP_NET_ADMIN` /
    /// missing-scope remediation.
    #[error("mTLS outbound cgroup attach failed: {0}")]
    OutboundAttach(#[from] MtlsDataplaneError),

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

    /// The closed-vocabulary install-stage label for the
    /// [`TransitionReason::MtlsInterceptInstallFailed`] cause-class the shim
    /// writes. Maps the 3-variant error (and, for [`Self::Inbound`], the
    /// inner [`InterceptError`] variant) to the four pinned stage strings:
    /// `"outbound_attach"`, `"leg_f_bind"`, `"leg_c_transparent_listener"`,
    /// `"inbound_tproxy"`. Internal mapping helper — NOT new contract
    /// surface.
    ///
    /// [`TransitionReason::MtlsInterceptInstallFailed`]:
    ///     overdrive_core::transition_reason::TransitionReason::MtlsInterceptInstallFailed
    #[must_use]
    pub const fn stage(&self) -> &'static str {
        match self {
            Self::OutboundAttach(_) => "outbound_attach",
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
    /// The `cgroup_connect4_mtls` attach link for this alloc's `.scope`.
    /// Dropping it detaches the program from the workload subtree.
    _cgroup_link: MtlsCgroupLink,
    /// The inbound nft-TPROXY redirect guard. Dropping it removes the
    /// per-virt rule from the shared chain.
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
    /// The agent's leg-F (outbound) listener address for this alloc —
    /// the kernel-redirect target `cgroup_connect4_mtls` rewrites a
    /// declared-peer `connect()` to. Recorded so the #178 declared-peer
    /// stand-in seam can resolve THIS alloc's own leg-F when it programs
    /// `MTLS_REDIRECT_DEST[real_peer] = leg_f` (the test never has to
    /// supply — or even observe — the worker-chosen ephemeral port).
    /// Read ONLY by the `integration-tests`-gated
    /// `program_declared_peer_redirect` seam — in a production build the
    /// field is recorded but never read (v1 production has no east-west
    /// peer enumeration; #178), so the `dead_code` allow is correct.
    #[cfg_attr(not(feature = "integration-tests"), allow(dead_code))]
    leg_f_addr: SocketAddrV4,
    /// The single declared peer's REAL destination address — the addr the
    /// workload originally `connect()`ed to.
    ///
    /// TRANSITIONAL (03-02→04-02): this declared-peer slot is now VESTIGIAL.
    /// As of step 03-02 (D-TME-4, the shipped nft-TPROXY mechanism) the
    /// outbound orig-dst IS recovered via `getsockname` on the accepted
    /// leg-F socket — `accept_outbound_leg` builds `Routed::Outbound { peer }`
    /// from that recovered addr, exactly symmetric with inbound TPROXY, and
    /// no longer reads `real_peer` to route. (The earlier claim that the
    /// rewrite was lossy and the orig-dst NOT `getsockname`-recoverable
    /// described the RETIRED `cgroup_connect4` rewrite, D-TME-3 RETIRED.)
    /// This field/slot is DELETED in step 04-02, when the resolve consumer
    /// orphans the declared-peer model.
    ///
    /// Still true while it lives: the declared-peer seam records it here
    /// (`program_declared_peer_redirect` already receives `real_peer` to
    /// program `MTLS_REDIRECT_DEST`). Shared (`Arc<Mutex<_>>`) because the
    /// seam writes it AFTER `start_alloc` has already spawned the accept
    /// loop; the same `Arc` is cloned into [`AcceptLeg::Outbound`]. `None`
    /// until a redirect is programmed — and a connection only arrives on
    /// leg-F once one is, so the accept loop fails-closed (logs + skips)
    /// rather than proceeding if it somehow reads `None`.
    ///
    /// This is the #178 stand-in: it gates whether a leg-F redirect was
    /// programmed at all (the SINGLE declared peer is the ratified D-MTLS-15
    /// scope). It no longer SUPPLIES the dial target — getsockname recovery
    /// does (see the TRANSITIONAL note above). General per-connection
    /// multi-peer orig-dst recovery remains
    /// [#178](https://github.com/overdrive-sh/overdrive/issues/178).
    ///
    /// Read back from the struct ONLY by the `integration-tests`-gated
    /// `program_declared_peer_redirect` seam; the `AcceptLeg::Outbound` loop
    /// no longer reads this field to route (it builds the routing fact from
    /// the getsockname-recovered orig-dst). In a production build the field
    /// is recorded but never read — the same shape as `leg_f_addr` above
    /// (#178), so the `dead_code` allow is correct.
    #[cfg_attr(not(feature = "integration-tests"), allow(dead_code))]
    real_peer: Arc<Mutex<Option<SocketAddrV4>>>,
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
    /// The production mTLS BPF intercept-install surface. `attach_alloc`
    /// is `&mut self`, so the dataplane sits behind a `Mutex` — per-alloc
    /// attach is serialised, which is correct (alloc lifecycle events are
    /// not a hot path; D-MTLS-17 item 1).
    dataplane: Mutex<MtlsDataplane>,
    /// The cgroupfs root (`/sys/fs/cgroup`) the alloc `.scope` paths
    /// resolve under.
    cgroup_root: PathBuf,
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
    /// Construct from the REQUIRED ports. `enforcement` and `dataplane`
    /// are both mandatory — no defaulting, no builder
    /// (`.claude/rules/development.md` § "Port-trait dependencies").
    #[must_use]
    pub fn new(
        enforcement: Arc<dyn MtlsEnforcement>,
        dataplane: MtlsDataplane,
        cgroup_root: PathBuf,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            enforcement,
            dataplane: Mutex::new(dataplane),
            cgroup_root,
            _clock: clock,
            intercepts: Mutex::new(BTreeMap::new()),
        }
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
    /// (OUTBOUND `cgroup_connect4_mtls` attach; leg-F bind; leg-C transparent
    /// listener) `start_alloc` returns the typed
    /// [`MtlsInterceptInstallError`] — surfacing the cause the worker
    /// previously discarded — and the action-shim drives the alloc to
    /// terminal `Failed`. The `ProbeRunner::start_alloc` fire-and-forget
    /// `()` contract does NOT transfer: a probe failure is itself an
    /// observation the reconciler consumes; an mTLS-install failure produces
    /// no such feedback loop, so "log and continue" would silently leave the
    /// confidentiality guarantee broken. (The INBOUND nft-TPROXY rule install
    /// is #178-deferred — see the module DECLARED-PEER note — so it is not an
    /// install step here and has no fail-closed site.)
    ///
    /// **Partial-teardown on the `Err` path.** Every guard acquired before
    /// the failing step (the [`MtlsCgroupLink`], the leg-F / leg-C listeners)
    /// is still a LOCAL at each failure point —
    /// it has not yet been handed to `spawn_legs_and_record`, so `stop_alloc`
    /// cannot find it in `self.intercepts`. Returning `Err` before recording
    /// drops those
    /// locals, and their `Drop` detaches the cgroup program / closes the
    /// listeners / removes the nft rule. The worker leaks NO half-installed
    /// intercept.
    ///
    /// # Errors
    ///
    /// [`MtlsInterceptInstallError::OutboundAttach`] (site 1),
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

        let scope_path = CgroupPath::for_alloc(&spec.alloc).resolve(&self.cgroup_root);

        // OUTBOUND install: attach cgroup_connect4_mtls to THIS alloc's
        // own .scope (the F5-exempt per-workload subtree). The
        // MTLS_REDIRECT_DEST programming is DEFERRED to #178 (the e2e gate
        // programs the single declared-peer entry as the stand-in) — see
        // the module DECLARED-PEER note.
        // Take the lock into a `let` so the guard drops before the match
        // body runs (clippy `significant_drop_in_scrutinee`).
        // Fail-closed (D-MTLS-18 site 1): `?` surfaces the typed
        // `MtlsDataplaneError` via `#[from]` as `OutboundAttach` — nothing is
        // acquired yet, so there is nothing to tear down.
        let cgroup_link = self.dataplane.lock().attach_alloc(&scope_path)?;

        // The agent's leg-F (outbound, workload-facing plaintext) listener
        // — agent-chosen ephemeral loopback (D-MTLS-15). Leg F needs no
        // IP_TRANSPARENT; a plain bound listener suffices.
        // Fail-closed (D-MTLS-18 site 2): on bind failure, return `Err`;
        // `cgroup_link` (the only guard acquired so far) drops here → detach.
        let leg_f_listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(source) => return Err(MtlsInterceptInstallError::leg_f_bind(source)),
        };
        // The agent's chosen leg-F address — the kernel-redirect target a
        // declared-peer `connect()` is rewritten to. Recorded in the
        // per-alloc bookkeeping so the #178 declared-peer stand-in seam can
        // resolve THIS alloc's own leg-F when it programs the redirect.
        let leg_f_addr = leg_f_listener
            .local_addr()
            .ok()
            .and_then(socketaddr_v4)
            .unwrap_or_else(|| SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0));

        // INBOUND install: the agent's leg-C IP_TRANSPARENT listener. The
        // accompanying nft-TPROXY redirect that would aim real client traffic
        // at this listener is #178-DEFERRED (see below) — production stands up
        // the listener + accept loop, but installs NO production TPROXY rule.
        // Fail-closed (D-MTLS-18 site 3): a server workload with no leg-C
        // inbound listener accepts cleartext client connections — a
        // confidentiality breach symmetric to the outbound one. Return `Err`
        // (the inbound carve-out is REJECTED per D-MTLS-18 P2); `cgroup_link`
        // + `leg_f_listener` (the guards acquired so far) drop here → detach
        // the cgroup program / close the leg-F listener.
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
            cgroup_link,
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
        cgroup_link: MtlsCgroupLink,
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
            cgroup_link,
            tproxy_guard,
            vec![outbound_task, inbound_task],
            enforced,
            leg_f_addr,
            real_peer,
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

    /// Test-only (#178 stand-in): program the single declared-peer
    /// `MTLS_REDIRECT_DEST[real_peer] = <alloc's own leg-F>` entry into
    /// THIS worker's own `MtlsDataplane`, so the e2e activation gate can
    /// drive a workload that dials a KNOWN declared peer through the
    /// production boot path. The leg-F target is resolved from the alloc's
    /// OWN per-alloc bookkeeping (recorded by `start_alloc`) — the test
    /// never supplies (nor needs to observe) the worker-chosen ephemeral
    /// leg-F port, so the redirect always lands on the worker's real
    /// accept→`enforce` listener (a test-chosen leg-F would bypass the
    /// accept loop, never reach `enforce`, and produce no TLS on the
    /// peer wire).
    ///
    /// NOT a production surface — v1 production has no east-west peer
    /// enumeration (that is #178); `start_alloc` never programs the
    /// redirect itself. `alloc` MUST have been `start_alloc`-ed first
    /// (so its leg-F is recorded); a redirect for an unknown alloc
    /// returns [`MtlsInterceptError::UnknownAlloc`].
    ///
    /// # Errors
    ///
    /// [`MtlsInterceptError::UnknownAlloc`] when `alloc` has no recorded
    /// intercept (it was never `start_alloc`-ed, or was already stopped);
    /// [`MtlsInterceptError::Dataplane`] when the `MTLS_REDIRECT_DEST`
    /// update syscall fails.
    #[cfg(feature = "integration-tests")]
    pub fn program_declared_peer_redirect(
        &self,
        alloc: &AllocationId,
        real_peer: SocketAddrV4,
    ) -> Result<(), MtlsInterceptError> {
        // Resolve the alloc's own leg-F AND its shared `real_peer` slot in
        // one lock acquisition (the slot is what the OUTBOUND accept loop
        // reads to dial the REAL peer instead of self-looping to leg-F).
        let (leg_f, real_peer_slot) = {
            let intercepts = self.intercepts.lock();
            let resolved = intercepts
                .get(alloc)
                .map(|intercept| (intercept.leg_f_addr, Arc::clone(&intercept.real_peer)));
            drop(intercepts);
            resolved.ok_or_else(|| MtlsInterceptError::UnknownAlloc { alloc: alloc.clone() })?
        };
        // Record the real peer FIRST, then program the kernel redirect. The
        // ordering is load-bearing: the redirect is what causes a workload
        // `connect(real_peer)` to land on leg-F, so the slot MUST hold
        // `Some(real_peer)` before any connection can arrive. Recording after
        // programming opens a window where the first redirected connection is
        // accepted while the slot is still `None` (dropped fail-closed). The
        // worker already receives `real_peer` here — it was discarding it;
        // recording it is the whole of the #178 stand-in's dial-target supply.
        *real_peer_slot.lock() = Some(real_peer);
        self.dataplane
            .lock()
            .program_redirect(real_peer, leg_f)
            .map_err(|source| MtlsInterceptError::Dataplane { source })
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
                    // OUTBOUND: the `cgroup_connect4_mtls` rewrite is LOSSY —
                    // it rewrote the workload's `connect(real_peer)` →
                    // `connect(leg_f)` in place, and the original destination
                    // is NOT recoverable from the accepted leg-F socket
                    // (unlike inbound TPROXY's `getsockname` orig-dst). The
                    // declared-peer seam SUPPLIES the dial target: read the
                    // recorded `real_peer` and route leg B to IT.
                    //
                    // Block until a connection is PENDING on the listener
                    // WITHOUT consuming it, THEN read `real_peer`. Reading the
                    // slot AFTER a connection is pending (not before the
                    // blocking accept) closes the stale-read window: the seam
                    // records `real_peer` before it programs the redirect, and
                    // the redirect is what routes a connection here, so by the
                    // time `await_pending_connection` reports POLLIN the slot
                    // holds `Some(real_peer)`.
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
    #[allow(clippy::too_many_arguments)]
    fn record_intercept_full(
        &self,
        alloc: AllocationId,
        cgroup_link: MtlsCgroupLink,
        tproxy_guard: Option<TproxyInterceptGuard>,
        accept_tasks: Vec<tokio::task::JoinHandle<()>>,
        enforced: Arc<Mutex<Vec<EnforcedConnection>>>,
        leg_f_addr: SocketAddrV4,
        real_peer: Arc<Mutex<Option<SocketAddrV4>>>,
        stop: Arc<AtomicBool>,
    ) {
        self.intercepts.lock().insert(
            alloc,
            AllocIntercept {
                _cgroup_link: cgroup_link,
                _tproxy_guard: tproxy_guard,
                accept_tasks,
                stop,
                enforced,
                leg_f_addr,
                real_peer,
            },
        );
    }
}

/// Test-only error for the #178 declared-peer stand-in seam.
///
/// Returned by [`MtlsInterceptWorker::program_declared_peer_redirect`].
/// Gated out of production builds entirely — the production worker never
/// programs the redirect (v1 has no east-west peer enumeration), so this
/// error has no production call site.
#[cfg(feature = "integration-tests")]
#[derive(Debug, thiserror::Error)]
pub enum MtlsInterceptError {
    /// `program_declared_peer_redirect` was called for an alloc with no
    /// recorded intercept (never `start_alloc`-ed, or already stopped),
    /// so its leg-F target cannot be resolved.
    #[error("no recorded mTLS intercept for alloc {alloc} (start_alloc must run first)")]
    UnknownAlloc { alloc: AllocationId },
    /// The underlying `MTLS_REDIRECT_DEST` map-update syscall failed.
    #[error("mTLS redirect program failed: {source}")]
    Dataplane {
        #[source]
        source: overdrive_dataplane::mtls::MtlsDataplaneError,
    },
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

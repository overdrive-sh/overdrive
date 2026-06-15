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
//!   installs the inbound nft-TPROXY redirect, and spawns the
//!   accept→`enforce` tasks. It does **NOT** program `MTLS_REDIRECT_DEST`
//!   — v1 has no production east-west peer enumeration (that is
//!   [#178](https://github.com/overdrive-sh/overdrive/issues/178); see the
//!   module-level "DECLARED-PEER" note below).
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
//! #178 stand-in. Everything else (load + attach + the inbound TPROXY
//! install + `enforce` + the wire) is production.

use std::collections::BTreeMap;
use std::net::SocketAddrV4;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use overdrive_core::AllocationId;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::AllocationSpec;
use overdrive_core::traits::mtls_enforcement::{EnforcedConnection, MtlsEnforcement};
use overdrive_dataplane::mtls::{MtlsCgroupLink, MtlsDataplane};
use parking_lot::Mutex;

use crate::cgroup_manager::CgroupPath;
use crate::mtls_intercept::{
    self, TproxyInterceptGuard, accept_inbound_leg, accept_outbound_leg, install_inbound_tproxy,
    make_transparent_listener,
};

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
    /// workload originally `connect()`ed to (and that `cgroup_connect4_mtls`
    /// rewrote to leg-F in place). The kernel rewrite is lossy (unlike
    /// inbound TPROXY, the original destination is NOT recoverable via
    /// `getsockname` on the accepted leg-F socket), so the worker cannot
    /// observe `real_peer` from the connection alone. The declared-peer
    /// seam SUPPLIES it: `program_declared_peer_redirect` records it here
    /// (it already receives `real_peer` to program `MTLS_REDIRECT_DEST`),
    /// and the OUTBOUND accept loop reads it to build
    /// `Routed::Outbound { peer: real_peer }` so `enforce` dials the REAL
    /// peer — not the agent's own leg-F (the self-loop the recorded `peer`
    /// would otherwise be). Shared (`Arc<Mutex<_>>`) because the seam
    /// writes it AFTER `start_alloc` has already spawned the accept loop;
    /// the same `Arc` is cloned into [`AcceptLeg::Outbound`] so the loop
    /// reads whatever the seam last recorded. `None` until a redirect is
    /// programmed — and a connection only arrives on leg-F once one is, so
    /// the accept loop fails-closed (logs + skips) rather than self-looping
    /// if it somehow reads `None`.
    ///
    /// This is the #178 stand-in doing exactly what #178 will do — supply
    /// the dial target — while the SINGLE declared peer is the ratified
    /// D-MTLS-15 scope. General per-connection multi-peer orig-dst recovery
    /// remains [#178](https://github.com/overdrive-sh/overdrive/issues/178).
    ///
    /// Read back from the struct ONLY by the `integration-tests`-gated
    /// `program_declared_peer_redirect` seam (the `AcceptLeg::Outbound`
    /// loop reads a SEPARATE `Arc` clone, not this field). In a production
    /// build the field is recorded but never read — the same shape as
    /// `leg_f_addr` above (#178), so the `dead_code` allow is correct.
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
    /// Failures are logged (NOT propagated) — the alloc is already Running
    /// and the lifecycle hook is fire-and-forget, mirroring
    /// `ProbeRunner::start_alloc`; a structured `health.*` warn names the
    /// cause so an intercept-install failure is observable.
    pub fn start_alloc(self: &Arc<Self>, spec: &AllocationSpec) {
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
        let attach_result = self.dataplane.lock().attach_alloc(&scope_path);
        let cgroup_link = match attach_result {
            Ok(link) => link,
            Err(source) => {
                tracing::warn!(
                    name: "health.mtls.intercept_install_failed",
                    reason = "cgroup_connect4_mtls.attach",
                    alloc = %spec.alloc,
                    scope = %scope_path.display(),
                    error = %source,
                    "mTLS outbound intercept attach failed; alloc runs without transparent mTLS"
                );
                return;
            }
        };

        // The agent's leg-F (outbound, workload-facing plaintext) listener
        // — agent-chosen ephemeral loopback (D-MTLS-15). Leg F needs no
        // IP_TRANSPARENT; a plain bound listener suffices.
        let leg_f_listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(source) => {
                tracing::warn!(
                    name: "health.mtls.intercept_install_failed",
                    reason = "leg_f.bind",
                    alloc = %spec.alloc,
                    error = %source,
                    "mTLS leg-F listener bind failed; alloc runs without transparent mTLS"
                );
                return;
            }
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

        // INBOUND install: the agent's leg-C IP_TRANSPARENT listener +
        // the nft-TPROXY redirect aimed at it. `virt` is the server
        // workload's logical loopback addr; in single-node v1 the
        // orig-dst recovered via getsockname IS this addr.
        let inbound_listener =
            match make_transparent_listener(SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0)) {
                Ok(l) => l,
                Err(source) => {
                    tracing::warn!(
                        name: "health.mtls.intercept_install_failed",
                        reason = "leg_c.transparent_listener",
                        alloc = %spec.alloc,
                        error = %source,
                        "mTLS leg-C IP_TRANSPARENT listener setup failed; \
                         alloc runs without inbound transparent mTLS"
                    );
                    // Outbound is already attached; keep it. Record the
                    // alloc with no inbound guard so stop_alloc still
                    // detaches the cgroup program.
                    self.record_intercept(
                        spec.alloc.clone(),
                        cgroup_link,
                        None,
                        Vec::new(),
                        leg_f_addr,
                        Arc::new(Mutex::new(None)),
                        Arc::new(AtomicBool::new(false)),
                    );
                    return;
                }
            };

        let agent_port = inbound_listener.local_addr().map(|a| a.port()).unwrap_or_default();
        // The inbound TPROXY redirect for this workload's virtual addr.
        // In single-node v1 the virt is the alloc's loopback server addr;
        // a failure here leaves outbound intact (inbound is best-effort
        // for the alloc, surfaced as a warn).
        let virt = SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, agent_port);
        let tproxy_guard = match install_inbound_tproxy(virt, agent_port) {
            Ok(guard) => Some(guard),
            Err(source) => {
                tracing::warn!(
                    name: "health.mtls.intercept_install_failed",
                    reason = "inbound.tproxy_install",
                    alloc = %spec.alloc,
                    error = %source,
                    "mTLS inbound TPROXY install failed; alloc runs without inbound transparent mTLS"
                );
                None
            }
        };

        self.spawn_legs_and_record(
            spec,
            cgroup_link,
            tproxy_guard,
            leg_f_listener,
            leg_f_addr,
            inbound_listener,
        );
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
                    // `Routed::Outbound { peer: real_peer }` so `enforce` dials
                    // the REAL peer.
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

    /// Record an outbound-only intercept (inbound listener setup failed).
    #[allow(clippy::too_many_arguments)]
    fn record_intercept(
        &self,
        alloc: AllocationId,
        cgroup_link: MtlsCgroupLink,
        tproxy_guard: Option<TproxyInterceptGuard>,
        accept_tasks: Vec<tokio::task::JoinHandle<()>>,
        leg_f_addr: SocketAddrV4,
        real_peer: Arc<Mutex<Option<SocketAddrV4>>>,
        stop: Arc<AtomicBool>,
    ) {
        self.record_intercept_full(
            alloc,
            cgroup_link,
            tproxy_guard,
            accept_tasks,
            Arc::new(Mutex::new(Vec::new())),
            leg_f_addr,
            real_peer,
            stop,
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

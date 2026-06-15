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
                    self.record_intercept(spec.alloc.clone(), cgroup_link, None, Vec::new());
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

        let enforced: Arc<Mutex<Vec<EnforcedConnection>>> = Arc::new(Mutex::new(Vec::new()));
        let leg_f_addr = leg_f_listener
            .local_addr()
            .ok()
            .and_then(socketaddr_v4)
            .unwrap_or_else(|| SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0));

        let outbound_task = self.spawn_accept_loop(
            spec.alloc.clone(),
            AcceptLeg::Outbound { listener: leg_f_listener, leg_f_addr },
            Arc::clone(&enforced),
        );
        let inbound_task = self.spawn_accept_loop(
            spec.alloc.clone(),
            AcceptLeg::Inbound { listener: inbound_listener },
            Arc::clone(&enforced),
        );

        self.record_intercept_full(
            spec.alloc.clone(),
            cgroup_link,
            tproxy_guard,
            vec![outbound_task, inbound_task],
            enforced,
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

        // Abort the accept loops so a blocked `accept()` cannot outlive
        // the alloc (the listeners drop when the tasks are aborted).
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
    /// `MTLS_REDIRECT_DEST[real_peer] = leg_f` entry into THIS worker's
    /// own `MtlsDataplane`, so the e2e activation gate can drive a
    /// workload that dials a KNOWN declared peer through the production
    /// boot path. NOT a production surface — v1 production has no
    /// east-west peer enumeration (that is #178); `start_alloc` never
    /// programs the redirect itself.
    ///
    /// # Errors
    ///
    /// Surfaces [`overdrive_dataplane::mtls::MtlsDataplaneError`] when the
    /// `MTLS_REDIRECT_DEST` update syscall fails.
    #[cfg(feature = "integration-tests")]
    pub fn program_declared_peer_redirect(
        &self,
        real_peer: SocketAddrV4,
        leg_f: SocketAddrV4,
    ) -> Result<(), overdrive_dataplane::mtls::MtlsDataplaneError> {
        self.dataplane.lock().program_redirect(real_peer, leg_f)
    }

    /// Spawn the accept→`enforce` loop for one leg. Each accepted
    /// connection is built into an `InterceptedConnection`, `enforce`d,
    /// and its handle pushed into the alloc's teardown set.
    fn spawn_accept_loop(
        self: &Arc<Self>,
        alloc: AllocationId,
        leg: AcceptLeg,
        enforced: Arc<Mutex<Vec<EnforcedConnection>>>,
    ) -> tokio::task::JoinHandle<()> {
        let worker = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            // The closure OWNS `alloc`/`leg`/`enforced`; `accept_loop`
            // borrows them for the duration of the loop (it clones `alloc`
            // per connection and re-uses `leg`/`enforced` by reference).
            worker.accept_loop(&alloc, &leg, &enforced);
        })
    }

    /// Blocking accept loop (the leg listeners are blocking
    /// `std::net::TcpListener`s — leg acquisition is a one-shot per
    /// intercepted connection, not an async pump). Each accept builds the
    /// `InterceptedConnection` and hands it to `enforce` on the tokio
    /// runtime; `enforce`'s own task owns the pumps + (B) self-teardown.
    fn accept_loop(
        self: &Arc<Self>,
        alloc: &AllocationId,
        leg: &AcceptLeg,
        enforced: &Arc<Mutex<Vec<EnforcedConnection>>>,
    ) {
        loop {
            let built = match leg {
                AcceptLeg::Outbound { listener, leg_f_addr } => {
                    // OUTBOUND `peer` is the workload's intended
                    // destination. v1 has no production peer enumeration
                    // (#178); the accepted leg's peer addr is the
                    // workload's connect target as the kernel rewrote it.
                    // For the authn-only v1 contract the routing `peer`
                    // is the leg-F address the workload was redirected to
                    // (the declared-peer the e2e programmed) — recovered
                    // here from the accepted socket.
                    accept_outbound_leg(listener, alloc.clone(), *leg_f_addr)
                }
                AcceptLeg::Inbound { listener } => accept_inbound_leg(listener, alloc.clone()),
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
        cgroup_link: MtlsCgroupLink,
        tproxy_guard: Option<TproxyInterceptGuard>,
        accept_tasks: Vec<tokio::task::JoinHandle<()>>,
        enforced: Arc<Mutex<Vec<EnforcedConnection>>>,
    ) {
        self.intercepts.lock().insert(
            alloc,
            AllocIntercept {
                _cgroup_link: cgroup_link,
                _tproxy_guard: tproxy_guard,
                accept_tasks,
                enforced,
            },
        );
    }

    /// Record an outbound-only intercept (inbound listener setup failed).
    fn record_intercept(
        &self,
        alloc: AllocationId,
        cgroup_link: MtlsCgroupLink,
        tproxy_guard: Option<TproxyInterceptGuard>,
        accept_tasks: Vec<tokio::task::JoinHandle<()>>,
    ) {
        self.record_intercept_full(
            alloc,
            cgroup_link,
            tproxy_guard,
            accept_tasks,
            Arc::new(Mutex::new(Vec::new())),
        );
    }
}

/// Which leg an accept loop is draining.
enum AcceptLeg {
    /// Outbound leg-F (workload-facing plaintext). `leg_f_addr` is the
    /// agent's listener addr the kernel redirected the workload to —
    /// carried into `Routed::Outbound { peer }` for the authn-only v1
    /// contract.
    Outbound { listener: std::net::TcpListener, leg_f_addr: SocketAddrV4 },
    /// Inbound leg-C (client-facing, TPROXY-redirected). orig-dst is
    /// recovered via `getsockname` inside `accept_inbound_leg`.
    Inbound { listener: std::net::TcpListener },
}

/// Narrow `SocketAddr → SocketAddrV4` projection (the legs are bound on
/// IPv4 loopback; single-node Phase-1 scope is IPv4-only).
const fn socketaddr_v4(addr: std::net::SocketAddr) -> Option<SocketAddrV4> {
    match addr {
        std::net::SocketAddr::V4(v4) => Some(v4),
        std::net::SocketAddr::V6(_) => None,
    }
}

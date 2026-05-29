//! `ProbeRunner` subsystem — per-alloc-per-probe tokio task graph
//! that ticks declared/inferred probes and writes
//! `ProbeResultRow`s to the `ObservationStore`.
//!
//! Per ADR-0054:
//! - `overdrive-worker` placement (probe execution is observation
//!   production — belongs to the machine running the workload per
//!   C1).
//! - Per-alloc supervisor + per-probe-instance tokio task shape
//!   (matches K8s prober.Manager archetype; D-02).
//! - Three port traits per ADR-0054 §3 (`TcpProber` / `HttpProber` /
//!   `ExecProber`); each backed by a production adapter
//!   (`TokioTcpProber` / `HyperHttpProber` / `CgroupExecProber`)
//!   and a sim adapter (in `crates/overdrive-sim/src/adapters/
//!   probers.rs`).
//! - Earned Trust gate at composition root (DDD-21): [`ProbeRunner::probe`]
//!   runs after construction and before serving any request; failure
//!   refuses startup via `health.startup.refused` (invocation at the
//!   composition root lands in step 01-03d).

#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    clippy::module_name_repetitions,
    clippy::missing_errors_doc,
    reason = "shared docstring style for the ProbeRunner subsystem"
)]

pub mod exec_prober;
pub mod http_prober;
pub mod supervisor;
pub mod tcp_prober;

pub use exec_prober::CgroupExecProber;
pub use http_prober::HyperHttpProber;
pub use supervisor::{AllocSupervisor, ProbeTaskHandle};
pub use tcp_prober::TokioTcpProber;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::id::AllocationId;
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::prober::{ProbeFailure, ProbeOutcome, TcpProber};
use parking_lot::Mutex;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

/// Subsystem entry point — owned by the worker's composition root.
///
/// Per ADR-0054 §2: per-alloc supervisor supervises N per-probe
/// tokio tasks. Each task ticks its probe on the configured
/// interval, writes a `ProbeResultRow` to the `ObservationStore`,
/// and surrenders on its `CancellationToken.child_token()` when
/// the alloc reaches a terminal state.
///
/// Production wiring at the composition root passes
/// `Arc<TokioTcpProber>` / `Arc<HyperHttpProber>` /
/// `Arc<CgroupExecProber>` plus the production `Clock` and
/// `ObservationStore`. Tests pass the sim equivalents.
#[allow(
    clippy::struct_field_names,
    reason = "Per-mechanic prober field naming is operator-readable; renaming loses the per-mechanic split documented in ADR-0054 §3"
)]
pub struct ProbeRunner {
    tcp_prober: Arc<dyn TcpProber>,
    // SCAFFOLD: true — http_prober + exec_prober remain wired but
    // unused at slice 01; HTTP body in 02-01, Exec body in 02-02.
    // Per-descriptor task spawn for Http / Exec mechanics logs +
    // returns; the production loop does NOT panic on them.
    #[allow(dead_code)]
    http_prober: Arc<dyn overdrive_core::traits::prober::HttpProber>,
    #[allow(dead_code)]
    exec_prober: Arc<dyn overdrive_core::traits::prober::ExecProber>,
    /// Injected clock. Required by `start_alloc`'s spawned tick
    /// tasks (`clock.sleep(interval)` in the supervised loop) and by
    /// `probe_once_and_record` (timestamps the `ProbeResultRow`).
    /// Mandatory constructor parameter per
    /// `.claude/rules/development.md` § "Port-trait dependencies"
    /// — no builder, no default-to-production. Under simulation, tests
    /// inject `SimClock` so `tick(interval)` deterministically wakes
    /// the spawned probe tasks.
    clock: Arc<dyn Clock>,
    /// Injected observation store. Required by `start_alloc`'s
    /// spawned tick tasks (every probe outcome lands as a
    /// `ProbeResultRow` write). Mandatory constructor parameter per
    /// `.claude/rules/development.md` § "Port-trait dependencies".
    observation_store: Arc<dyn ObservationStore>,
    /// Per-alloc supervisors. `BTreeMap` per
    /// `.claude/rules/development.md` § "Ordered-collection choice"
    /// — the supervisor map is drained on `stop_alloc` and iterated
    /// by per-alloc cleanup paths; deterministic order is required.
    supervisors: Mutex<BTreeMap<AllocationId, AllocSupervisor>>,
}

impl ProbeRunner {
    /// Construct a `ProbeRunner` with injected adapters. Per
    /// `.claude/rules/development.md` § "Port-trait dependencies":
    /// adapters are MANDATORY constructor parameters — no
    /// `with_xxx` builder, no default-to-production inside the
    /// constructor.
    #[must_use]
    pub fn new(
        tcp_prober: Arc<dyn TcpProber>,
        http_prober: Arc<dyn overdrive_core::traits::prober::HttpProber>,
        exec_prober: Arc<dyn overdrive_core::traits::prober::ExecProber>,
        clock: Arc<dyn Clock>,
        observation_store: Arc<dyn ObservationStore>,
    ) -> Self {
        Self {
            tcp_prober,
            http_prober,
            exec_prober,
            clock,
            observation_store,
            supervisors: Mutex::new(BTreeMap::new()),
        }
    }

    /// Earned Trust gate per DDD-21 + ADR-0054 §7. Runs after
    /// construction and before the runtime serves any request.
    /// Sacrificial-listener path validates the injected TCP adapter
    /// end-to-end; a failure surfaces as a typed
    /// [`ProbeRunnerError::EarnedTrustFailure`] and the caller refuses
    /// startup with a structured `health.startup.refused` event.
    ///
    /// Per ADR-0054 §7 the sacrificial listener binds to
    /// `127.0.0.1:0` (kernel-assigned port; no race per the
    /// risk-table mitigation).
    ///
    /// The composition-root invocation + the `health.startup.refused`
    /// tracing event + the non-zero CLI exit on failure land in step
    /// 01-03d. This method (declaration + body exercising the
    /// sacrificial probe) lands in 01-03c; the structural defense
    /// against the method being removed is the `xtask::dst_lint`
    /// scanner clause that walks `impl ProbeRunner` blocks asserting
    /// `fn probe(&self)` is declared.
    pub async fn probe(&self) -> Result<(), ProbeRunnerError> {
        // Bind a sacrificial loopback listener on 127.0.0.1:0 so the
        // kernel picks the port (no race per ADR-0054 §7). The
        // listener is dropped immediately after we read its address;
        // the probe attempt then fires against the ephemeral port —
        // which will either connect (race window: kernel hasn't yet
        // reaped) or refuse. Both outcomes prove the adapter wired
        // end-to-end. Production semantics only require that the
        // probe returns SOMETHING (Pass or Fail) — a probe panic or
        // a `ProbeFailure` is the failure signal.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|err| {
            ProbeRunnerError::EarnedTrustFailure {
                reason: format!("failed to bind sacrificial loopback listener: {err}"),
            }
        })?;
        let addr = listener.local_addr().map_err(|err| ProbeRunnerError::EarnedTrustFailure {
            reason: format!("failed to read sacrificial listener address: {err}"),
        })?;
        // Hold the listener open across the probe attempt so the port
        // is guaranteed accepting. Drop happens at end of scope.
        let _listener = listener;
        match self
            .tcp_prober
            .probe(&addr.ip().to_string(), addr.port(), Duration::from_secs(2))
            .await
        {
            Ok(ProbeOutcome::Pass) => Ok(()),
            Ok(ProbeOutcome::Fail { reason }) => Err(ProbeRunnerError::EarnedTrustFailure {
                reason: format!("sacrificial loopback probe returned Fail: {reason}"),
            }),
            Err(ProbeFailure::InvalidTarget { reason }) => {
                Err(ProbeRunnerError::EarnedTrustFailure {
                    reason: format!("sacrificial probe rejected by adapter: {reason}"),
                })
            }
            Err(ProbeFailure::ExecSpawnFailed { reason }) => {
                Err(ProbeRunnerError::EarnedTrustFailure {
                    reason: format!("sacrificial probe spawn failed: {reason}"),
                })
            }
        }
    }

    /// Probe one descriptor once against the injected adapter and
    /// write a `ProbeResultRow` to the observation store.
    ///
    /// This is the single-tick primitive that the per-alloc
    /// supervisor's per-probe task loops over. Exposed as a public
    /// method so Tier 1 acceptance tests can exercise the production
    /// `ProbeRunner` → adapter → store path through one call without
    /// spinning up a long-lived supervisor.
    ///
    /// # Behaviour
    /// - Resolves the appropriate adapter (today: only `Tcp`;
    ///   `Http` / `Exec` bodies land in slice 02 / 03).
    /// - Calls `prober.probe(...)` with the descriptor's mechanic
    ///   parameters.
    /// - Translates the outcome to a [`ProbeResultRow`] keyed by
    ///   `(alloc_id, probe_idx)` with `last_observed_at_unix_ms`
    ///   sourced from the injected `clock`.
    /// - Writes via [`ObservationStore::write_probe_result`].
    ///
    /// Errors at any step surface as [`ProbeRunnerError`] variants;
    /// the caller (supervisor loop) decides whether to retry.
    pub async fn probe_once_and_record(
        &self,
        alloc_id: &AllocationId,
        probe_idx: ProbeIdx,
        descriptor: &ProbeDescriptor,
        clock: &dyn Clock,
        observation_store: &dyn ObservationStore,
    ) -> Result<ProbeResultRow, ProbeRunnerError> {
        // Delegate to the free-standing `probe_tick` body so the
        // public single-tick surface and the spawned supervised
        // loop share one source of truth. The `clock` and
        // `observation_store` call-site parameters are honoured
        // verbatim (the public API contract); the spawned loop in
        // `start_alloc` instead uses the stored `Arc<dyn Clock>` /
        // `Arc<dyn ObservationStore>`.
        probe_tick(
            self.tcp_prober.as_ref(),
            clock,
            observation_store,
            alloc_id,
            probe_idx,
            descriptor,
        )
        .await
    }

    /// Register an allocation supervisor — owns its
    /// [`CancellationToken`] and zero or more per-probe task handles.
    /// Per-probe tasks are spawned via
    /// [`ProbeRunner::spawn_probe_task`] which the supervisor's
    /// owner drives.
    ///
    /// Idempotent: re-registering the same `alloc_id` returns the
    /// existing supervisor's [`CancellationToken`] without disturbing
    /// running tasks. This is the shape the worker's exec-driver
    /// integration uses to attach probes lazily after the alloc
    /// reaches Running.
    pub fn register_alloc(&self, alloc_id: &AllocationId) -> CancellationToken {
        let mut supervisors = self.supervisors.lock();
        supervisors.entry(alloc_id.clone()).or_default().token()
    }

    /// Lifecycle hook called by the worker driver's
    /// `on_alloc_running` callback when an allocation reaches the
    /// `Running` state. Spawns one tokio task per declared/inferred
    /// `ProbeDescriptor` under the per-alloc supervisor.
    ///
    /// Per ADR-0054 § 2: every spawned task ticks its descriptor on
    /// the descriptor-declared interval, invokes the adapter, and
    /// writes the resulting `ProbeResultRow` to the
    /// `ObservationStore`. Cooperative shutdown via the supervisor's
    /// child [`CancellationToken`] — task bodies observe the token
    /// in a `select!` arm and exit on the next async yield. No
    /// `JoinHandle::abort()` per `.claude/rules/testing.md`
    /// § cooperative-shutdown discipline.
    ///
    /// Tick ordering: **tick-then-sleep** (K8s `prober.Manager`
    /// precedent). The first probe attempt fires after the FIRST
    /// `clock.sleep(interval)` resolves, NOT before — this matches
    /// the existing TCP-prober + `last_observed_at_unix_ms` contract
    /// (every row carries a clock-derived timestamp; firing
    /// pre-sleep would emit a row at the moment of registration
    /// regardless of the descriptor's declared interval). Tests that
    /// need to observe the first row drive `clock.tick(interval)`
    /// explicitly.
    ///
    /// Per ADR-0054 § 3: the descriptors are carried on
    /// [`overdrive_core::traits::driver::AllocationSpec`] and
    /// projected from the reconciler-emitted action through the
    /// action shim into the driver's `on_alloc_running` hook.
    /// Phase-1 Job-kind workloads pass an empty `Vec` — the loop
    /// body is structurally a no-op for them (zero descriptors,
    /// zero spawned tasks). Service-kind workloads project from
    /// `ServiceSpec.health_check` per ADR-0057.
    ///
    /// Idempotent: re-calling against the same `alloc_id` returns
    /// the existing supervisor's token. The current implementation
    /// **does not re-spawn tasks on re-register** — the action-shim
    /// invariant is that `on_alloc_running` fires exactly once per
    /// allocation reaching `Running`, so re-spawning would
    /// duplicate the supervised loops.
    ///
    /// HTTP / Exec mechanic bodies for the per-tick probe call land
    /// in slice 02 / 03; spawned tasks for those mechanics today
    /// log + return on the first iteration via
    /// [`Self::probe_once_and_record`]'s
    /// [`ProbeRunnerError::MechanicNotYetImplemented`] surface (no
    /// panic, no retry storm — the structural plumbing accepts
    /// them gracefully).
    pub fn start_alloc(
        &self,
        alloc_id: &AllocationId,
        probe_descriptors: Vec<ProbeDescriptor>,
    ) -> CancellationToken {
        let root_token = self.register_alloc(alloc_id);

        // Per-descriptor task spawn. Each task carries cloned Arcs
        // for its prober adapter, the injected clock, and the
        // observation store — no `&self` borrow escapes into the
        // spawned future. The supervisor's child token is the
        // cooperative-shutdown handle observed by the `select!`
        // arm.
        let supervisors = self.supervisors.lock();
        let Some(supervisor) = supervisors.get(alloc_id) else {
            // Logically unreachable — `register_alloc` above just
            // inserted the entry. The match shape keeps the lint
            // surface honest per `.claude/rules/development.md`
            // § "Logically unreachable `None` / `Err`".
            return root_token;
        };
        for (idx, descriptor) in probe_descriptors.into_iter().enumerate() {
            let handle = supervisor.spawn_probe_task();
            let child_token = handle.cancellation_token();
            let tcp_prober = Arc::clone(&self.tcp_prober);
            let clock = Arc::clone(&self.clock);
            let observation_store = Arc::clone(&self.observation_store);
            let alloc_id_for_task = alloc_id.clone();
            // ProbeIdx is 0-indexed across the descriptor vector
            // per the `discuss/shared-artifacts-registry.md`
            // contract — saturating cast keeps this safe past
            // u32::MAX descriptors (operationally impossible; the
            // max-attempts ceiling is 30).
            let probe_idx = ProbeIdx::new(u32::try_from(idx).unwrap_or(u32::MAX));
            tokio::spawn(async move {
                supervised_probe_loop(
                    tcp_prober,
                    clock,
                    observation_store,
                    alloc_id_for_task,
                    probe_idx,
                    descriptor,
                    child_token,
                )
                .await;
            });
        }
        drop(supervisors);
        root_token
    }

    /// Cancel every probe task spawned under `alloc_id` and drop the
    /// per-alloc supervisor. Cooperative shutdown only — task bodies
    /// observe the cancellation on their next `select!` round and
    /// return. No `JoinHandle::abort()` per
    /// `.claude/rules/testing.md` § "Leaked workload cgroups across
    /// runs" / the equivalent cooperative-shutdown rule for
    /// network-bound probes.
    ///
    /// Idempotent: stopping an unknown / already-stopped alloc is a
    /// no-op.
    pub fn stop_alloc(&self, alloc_id: &AllocationId) {
        let supervisor = {
            let mut supervisors = self.supervisors.lock();
            supervisors.remove(alloc_id)
        };
        if let Some(supervisor) = supervisor {
            supervisor.cancel();
        }
    }

    /// Count of live per-alloc supervisors — exposed for inspection
    /// by tests and operator-facing diagnostics.
    #[must_use]
    pub fn active_alloc_count(&self) -> usize {
        self.supervisors.lock().len()
    }
}

/// Errors surfaced by the runner subsystem.
///
/// `EarnedTrustFailure` is the variant that triggers
/// `health.startup.refused` per ADR-0054 §7.
#[derive(Debug, Error)]
pub enum ProbeRunnerError {
    #[error("Earned Trust probe failed: {reason}")]
    EarnedTrustFailure { reason: String },
    #[error("probe adapter rejected attempt for alloc {alloc_id} probe_idx {probe_idx}: {source}")]
    ProbeAdapterFailed {
        alloc_id: AllocationId,
        probe_idx: ProbeIdx,
        #[source]
        source: ProbeFailure,
    },
    #[error(
        "observation store rejected probe-result write for alloc {alloc_id} probe_idx {probe_idx}: {source}"
    )]
    ObservationWriteFailed {
        alloc_id: AllocationId,
        probe_idx: ProbeIdx,
        #[source]
        source: overdrive_core::traits::observation_store::ObservationStoreError,
    },
    #[error("mechanic for role {role:?} not yet implemented in this slice")]
    MechanicNotYetImplemented { role: ProbeRole },
}

/// Wall-clock to UNIX-epoch milliseconds, sourced from the injected
/// [`Clock::unix_now`]. Per ADR-0013 / ADR-0054 §5 every wall-clock
/// read goes through the trait surface — never `Instant::now` /
/// `SystemTime::now`.
fn unix_ms_from_clock(clock: &dyn Clock) -> u64 {
    // `Duration::as_millis` returns `u128`; the row field is `u64`.
    // Saturating cast — overflow happens past year 584,942,417 AD,
    // outside the platform's lifetime.
    u64::try_from(clock.unix_now().as_millis()).unwrap_or(u64::MAX)
}

/// Pure per-tick probe logic — invoke the adapter, build the row,
/// write to the store. Free-standing so the spawned per-descriptor
/// task body and the public `probe_once_and_record` surface
/// delegate to the same function — no `&self` borrow escapes into
/// the supervised loop, and the public/loop paths cannot diverge
/// silently.
async fn probe_tick(
    tcp_prober: &dyn TcpProber,
    clock: &dyn Clock,
    observation_store: &dyn ObservationStore,
    alloc_id: &AllocationId,
    probe_idx: ProbeIdx,
    descriptor: &ProbeDescriptor,
) -> Result<ProbeResultRow, ProbeRunnerError> {
    let timeout = Duration::from_secs(u64::from(descriptor.timeout_seconds));
    let status = match &descriptor.mechanic {
        ProbeMechanic::Tcp { host, port } => {
            match tcp_prober.probe(host, *port, timeout).await.map_err(|err| {
                ProbeRunnerError::ProbeAdapterFailed {
                    alloc_id: alloc_id.clone(),
                    probe_idx,
                    source: err,
                }
            })? {
                ProbeOutcome::Pass => ProbeStatus::Pass,
                ProbeOutcome::Fail { reason } => ProbeStatus::Fail { last_fail_reason: reason },
            }
        }
        ProbeMechanic::Http { .. } | ProbeMechanic::Exec { .. } => {
            return Err(ProbeRunnerError::MechanicNotYetImplemented { role: descriptor.role });
        }
    };
    let last_observed_at_unix_ms = unix_ms_from_clock(clock);
    let row = ProbeResultRow {
        alloc_id: alloc_id.clone(),
        probe_idx,
        role: descriptor.role,
        status,
        last_observed_at_unix_ms,
        inferred: descriptor.inferred,
    };
    observation_store.write_probe_result(row.clone()).await.map_err(|err| {
        ProbeRunnerError::ObservationWriteFailed {
            alloc_id: alloc_id.clone(),
            probe_idx,
            source: err,
        }
    })?;
    Ok(row)
}

/// Per-descriptor supervised tick loop body — runs as a `tokio::spawn`
/// task under the per-alloc supervisor. Tick-then-sleep ordering: the
/// loop parks on `clock.sleep(interval)` BEFORE each probe attempt
/// (the first row lands after the first interval elapses) racing
/// against `child_token.cancelled()` in a `select!`.
///
/// On `clock.sleep` resolve the loop calls
/// [`probe_tick`] and emits a `tracing::warn!` on every adapter or
/// store error — no panic, no retry storm. The failure row itself
/// IS the observable: the `ServiceLifecycleReconciler` consumes
/// the LWW projection and decides the next action. A
/// [`ProbeRunnerError::MechanicNotYetImplemented`] (Http / Exec
/// today) is logged once per tick at warn level; the loop continues
/// so the supervisor's lifecycle stays tied to the cancellation
/// token, not to the first error.
async fn supervised_probe_loop(
    tcp_prober: Arc<dyn TcpProber>,
    clock: Arc<dyn Clock>,
    observation_store: Arc<dyn ObservationStore>,
    alloc_id: AllocationId,
    probe_idx: ProbeIdx,
    descriptor: ProbeDescriptor,
    child_token: CancellationToken,
) {
    let interval = Duration::from_secs(u64::from(descriptor.interval_seconds));
    loop {
        tokio::select! {
            biased;
            () = child_token.cancelled() => return,
            () = clock.sleep(interval) => {
                if let Err(err) = probe_tick(
                    tcp_prober.as_ref(),
                    clock.as_ref(),
                    observation_store.as_ref(),
                    &alloc_id,
                    probe_idx,
                    &descriptor,
                ).await {
                    tracing::warn!(
                        name: "probe_tick.error",
                        alloc_id = %alloc_id,
                        probe_idx = probe_idx.0,
                        role = ?descriptor.role,
                        error = %err,
                        "supervised probe tick failed; loop continues until cancelled",
                    );
                }
            }
        }
    }
}

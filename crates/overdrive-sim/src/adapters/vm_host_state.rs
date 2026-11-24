//! `SimVmHostState` — test binding of the [`VmHostState`] port trait
//! (ADR-0083 §D7, `brief.md` §105a.2).
//!
//! In-memory observation state (three `BTreeMap`/`BTreeSet`s mirroring
//! [`VmHostObservation`]'s own shape), mutated directly by
//! [`VmHostState::kill_scope`] / [`VmHostState::discard_artifacts`], plus
//! an injectable one-shot [`VmHostState::probe`] fault. Makes "a VM
//! allocation's host state, and its removal" a DST-controllable scenario
//! for Tier 1.
//!
//! # Concurrency
//!
//! Every method body acquires `parking_lot::Mutex`, mutates the state,
//! and releases — no `.await` while holding a guard, per
//! `.claude/rules/development.md` § "Concurrency & async". The `async fn`
//! surface exists only to satisfy the trait signature.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use overdrive_core::AllocationId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::traits::vm_host_state::{
    ScopeFacts, VmHostObservation, VmHostState, VmHostStateProbeError,
};
use parking_lot::Mutex;
use tokio::sync::Notify;

/// The fixed relative path segment every workload cgroup scope lives
/// under — mirrors `overdrive_core::cgroup::CgroupPath::for_alloc`'s own
/// literal, needed here to parse a [`CgroupPath`] back into the
/// [`AllocationId`] it names (the in-memory store's key).
const WORKLOADS_SLICE_PREFIX: &str = "overdrive.slice/workloads.slice/";
const SCOPE_SUFFIX: &str = ".scope";

fn alloc_from_scope(scope: &CgroupPath) -> Option<AllocationId> {
    scope
        .as_str()
        .strip_prefix(WORKLOADS_SLICE_PREFIX)
        .and_then(|s| s.strip_suffix(SCOPE_SUFFIX))
        .and_then(|s| AllocationId::from_str(s).ok())
}

/// A scope hold predicate: while it returns `true`, `kill_scope` on the scope
/// is refused with `ResourceBusy`. Wrapped so `Inner` keeps its `Debug`.
#[derive(Clone)]
struct ScopeHold(Arc<dyn Fn() -> bool + Send + Sync>);

impl std::fmt::Debug for ScopeHold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScopeHold(..)")
    }
}

#[derive(Debug, Default)]
struct Inner {
    scopes: BTreeMap<AllocationId, ScopeFacts>,
    run_dirs: BTreeSet<AllocationId>,
    clones: BTreeMap<AllocationId, PathBuf>,
    probe_fault: Option<io::ErrorKind>,
    /// FIFO of failures for [`VmHostState::kill_scope`]; see
    /// [`SimVmHostState::script_kill_scope_failure`].
    kill_scope_failures: VecDeque<io::ErrorKind>,
    /// FIFO of failures for [`VmHostState::discard_artifacts`]; see
    /// [`SimVmHostState::script_discard_artifacts_failure`].
    discard_artifacts_failures: VecDeque<io::ErrorKind>,
    /// FIFO of failures for [`VmHostState::observe`]; see
    /// [`SimVmHostState::script_observe_failure`].
    observe_failures: VecDeque<io::ErrorKind>,
    /// Per-allocation scope holds; see [`SimVmHostState::hold_scope_while`].
    scope_holds: BTreeMap<AllocationId, ScopeHold>,
}

/// **TEST-HOOK-ONLY** interleaving seam for [`VmHostState::observe`]
/// (S-VM-78, ADR-0083 §D7 / `brief.md` §105a.2 — the `observe()`-first,
/// supervision-LAST hydration read order). Armed via
/// [`SimVmHostState::arm_observe_barrier`]; the NEXT `observe()` call
/// signals `entered` the instant its body starts running, then blocks
/// until the test calls [`ObserveBarrierHandle::release_observe`]. This
/// is what lets a test prove a mutation happened strictly BETWEEN
/// `observe()`'s start and its return — the only shape that can
/// distinguish `observe()`-first from supervision-first hydration
/// ordering (a claim taken merely "before `hydrate_actual` is called"
/// is observed identically under either order).
#[derive(Debug, Default)]
struct ObserveBarrier {
    entered: Notify,
    release: Notify,
}

/// Handle returned by [`SimVmHostState::arm_observe_barrier`].
#[derive(Debug)]
pub struct ObserveBarrierHandle {
    barrier: Arc<ObserveBarrier>,
}

impl ObserveBarrierHandle {
    /// Resolves once the armed `observe()` call has genuinely STARTED —
    /// its body has begun executing — not merely been scheduled as a
    /// task.
    pub async fn wait_for_observe_started(&self) {
        self.barrier.entered.notified().await;
    }

    /// Releases the paused `observe()` call so it proceeds to build and
    /// return its snapshot.
    pub fn release_observe(&self) {
        self.barrier.release.notify_one();
    }
}

/// Sim binding of the [`VmHostState`] port trait.
///
/// # Construction
///
/// ```
/// use overdrive_sim::adapters::vm_host_state::SimVmHostState;
/// let sim = SimVmHostState::new();
/// ```
///
/// # Clone semantics
///
/// Cloning shares the underlying `Arc<Mutex<...>>` state. Mirrors
/// `SimCgroupAccounting` / `SimCgroupFs` so callers can hand one clone
/// to the harness and another to the system under test and have both
/// observe the same mutations.
#[derive(Clone, Debug, Default)]
pub struct SimVmHostState {
    inner: Arc<Mutex<Inner>>,
    /// **TEST-HOOK-ONLY** — one-shot interleaving seam for `observe()`,
    /// armed via [`Self::arm_observe_barrier`]. `None` (the default)
    /// means `observe()` runs uninterrupted, exactly as before this
    /// seam existed.
    observe_barrier: Arc<Mutex<Option<Arc<ObserveBarrier>>>>,
}

impl SimVmHostState {
    /// Construct an empty `SimVmHostState` — no scopes, run dirs, or
    /// clones seeded; no fault injected.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// **TEST-HOOK-ONLY**. Arms a ONE-SHOT interleaving barrier on the
    /// NEXT `observe()` call — see [`ObserveBarrier`]'s doc comment for
    /// why this exists (S-VM-78).
    #[must_use]
    pub fn arm_observe_barrier(&self) -> ObserveBarrierHandle {
        let barrier = Arc::new(ObserveBarrier::default());
        *self.observe_barrier.lock() = Some(barrier.clone());
        ObserveBarrierHandle { barrier }
    }

    /// Seed a cgroup scope with the given live PIDs (may be empty).
    pub fn set_scope(&self, alloc: AllocationId, pids: BTreeSet<u32>) {
        self.inner.lock().scopes.insert(alloc, ScopeFacts { pids });
    }

    /// Seed the VM run root with an entry for `alloc`.
    pub fn set_run_dir(&self, alloc: AllocationId) {
        self.inner.lock().run_dirs.insert(alloc);
    }

    /// Seed the staging directory with a per-launch clone for `alloc` at
    /// `path`.
    pub fn set_clone(&self, alloc: AllocationId, path: PathBuf) {
        self.inner.lock().clones.insert(alloc, path);
    }

    /// Inject a ONE-SHOT [`VmHostStateProbeError::Substrate`] for the
    /// next [`VmHostState::probe`] call.
    pub fn inject_probe_fault(&self, kind: io::ErrorKind) {
        self.inner.lock().probe_fault = Some(kind);
    }

    /// Test-only: queue a failure for the next [`VmHostState::kill_scope`]
    /// call (the sim counterpart of a cgroup kill that fails). FIFO: each call
    /// pops the front entry BEFORE touching the scope table and returns it as
    /// `Err(..)`, leaving the host exactly as it was; once the queue is empty
    /// calls proceed normally. Stack several to model consecutive failures.
    pub fn script_kill_scope_failure(&self, kind: io::ErrorKind) {
        self.inner.lock().kill_scope_failures.push_back(kind);
    }

    /// Test-only: queue a failure for the next
    /// [`VmHostState::discard_artifacts`] call. Same FIFO semantics as
    /// [`Self::script_kill_scope_failure`]: the failing call changes nothing.
    pub fn script_discard_artifacts_failure(&self, kind: io::ErrorKind) {
        self.inner.lock().discard_artifacts_failures.push_back(kind);
    }

    /// **TEST-HOOK-ONLY**. Queue a failure for the next
    /// [`VmHostState::observe`] call. FIFO: each call pops the front entry
    /// and returns it as `Err(..)`; once the queue is empty calls proceed
    /// normally.
    pub fn script_observe_failure(&self, kind: io::ErrorKind) {
        self.inner.lock().observe_failures.push_back(kind);
    }

    /// **TEST-HOOK-ONLY**. While `held()` is true, `kill_scope` on `alloc`'s
    /// scope returns `ResourceBusy` at once, changes nothing and does not pop
    /// the [`Self::script_kill_scope_failure`] queue. `held()` is read on
    /// every call; the hold never removes the scope.
    ///
    /// # Panics
    ///
    /// If `alloc`'s scope was not seeded with [`Self::set_scope`].
    pub fn hold_scope_while(
        &self,
        alloc: &AllocationId,
        held: impl Fn() -> bool + Send + Sync + 'static,
    ) {
        let mut inner = self.inner.lock();
        assert!(
            inner.scopes.contains_key(alloc),
            "hold_scope_while: scope for {alloc} was not seeded with set_scope"
        );
        inner.scope_holds.insert(alloc.clone(), ScopeHold(Arc::new(held)));
    }

    /// **TEST-HOOK-ONLY**. Drops the hold [`Self::hold_scope_while`] set on
    /// `alloc`'s scope. A no-op when none is set.
    pub fn release_scope_hold(&self, alloc: &AllocationId) {
        self.inner.lock().scope_holds.remove(alloc);
    }

    /// **TEST-HOOK-ONLY**. Whether `alloc`'s cgroup scope is still
    /// present in this sim's in-memory state.
    #[must_use]
    pub fn has_scope(&self, alloc: &AllocationId) -> bool {
        self.inner.lock().scopes.contains_key(alloc)
    }

    /// **TEST-HOOK-ONLY**. Whether `alloc`'s run directory and clone
    /// are BOTH absent from this sim's in-memory state.
    #[must_use]
    pub fn artifacts_absent(&self, alloc: &AllocationId) -> bool {
        let inner = self.inner.lock();
        !inner.run_dirs.contains(alloc) && !inner.clones.contains_key(alloc)
    }
}

#[async_trait]
impl VmHostState for SimVmHostState {
    fn kind(&self) -> &'static str {
        "overdrive_sim::SimVmHostState"
    }

    async fn probe(&self) -> Result<(), VmHostStateProbeError> {
        let pending_fault = self.inner.lock().probe_fault.take();
        if let Some(kind) = pending_fault {
            return Err(VmHostStateProbeError::substrate(
                PathBuf::from("sim-vm-host-state"),
                io::Error::from(kind),
            ));
        }
        Ok(())
    }

    async fn observe(&self) -> std::io::Result<VmHostObservation> {
        // Interleaving seam (S-VM-78): if a barrier is armed, signal
        // that this call has genuinely STARTED, then pause until the
        // test releases it. The guard is a temporary and is dropped at
        // the end of this statement -- never held across the `.await`
        // below, per this module's own concurrency discipline (see the
        // crate doc comment at the top of this file).
        let armed = self.observe_barrier.lock().take();
        if let Some(barrier) = &armed {
            barrier.entered.notify_one();
            barrier.release.notified().await;
        }
        let mut inner = self.inner.lock();
        if let Some(kind) = inner.observe_failures.pop_front() {
            return Err(io::Error::from(kind));
        }
        Ok(VmHostObservation {
            scopes: inner.scopes.clone(),
            run_dirs: inner.run_dirs.clone(),
            clones: inner.clones.clone(),
        })
    }

    async fn kill_scope(&self, scope: &CgroupPath) -> std::io::Result<()> {
        // A held scope refuses before anything else: nothing changes and the
        // scripted-failure queue is not consumed. The predicate runs outside
        // the lock.
        let hold = alloc_from_scope(scope)
            .and_then(|alloc| self.inner.lock().scope_holds.get(&alloc).cloned());
        if hold.is_some_and(|hold| (hold.0)()) {
            return Err(io::Error::from(io::ErrorKind::ResourceBusy));
        }
        let scripted = self.inner.lock().kill_scope_failures.pop_front();
        if let Some(kind) = scripted {
            return Err(io::Error::from(kind));
        }
        // Idempotent: an unknown scope (already absent) is a no-op, not
        // an error -- mirrors the real adapter's NotFound-as-Ok
        // tolerance. In-memory removal settles synchronously, so the
        // settle postcondition holds trivially.
        if let Some(alloc) = alloc_from_scope(scope) {
            self.inner.lock().scopes.remove(&alloc);
        }
        Ok(())
    }

    async fn discard_artifacts(&self, alloc: &AllocationId) -> std::io::Result<()> {
        let scripted = self.inner.lock().discard_artifacts_failures.pop_front();
        if let Some(kind) = scripted {
            return Err(io::Error::from(kind));
        }
        {
            let mut inner = self.inner.lock();
            inner.run_dirs.remove(alloc);
            inner.clones.remove(alloc);
        }
        Ok(())
    }
}

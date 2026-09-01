//! Socket-free simulation of the action-shim mTLS lifecycle boundary.
//!
//! This adapter models only the lifecycle completion contract. It does not
//! bind listeners, install rules, or reproduce the worker's lower-level
//! interception machinery; the real-worker integration evidence remains at
//! that boundary.

use std::collections::{BTreeMap, VecDeque};

use async_trait::async_trait;
use overdrive_control_plane::action_shim::MtlsInterceptLifecycle;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::driver::AllocationSpec;
use overdrive_worker::mtls_intercept_worker::{MtlsInterceptInstallError, MtlsInterceptStopError};
use parking_lot::Mutex;

/// Observable lifecycle ownership state for one allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SimMtlsInterceptLifecycleState {
    /// The allocation has a complete, live lifecycle owner.
    Live,
    /// Stop started but a transient failure retained the old owner for retry.
    TeardownPending,
}

/// One completed lifecycle-call outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SimMtlsInterceptLifecycleEvent {
    /// A start completed and made the allocation live.
    StartCompleted {
        /// Allocation whose lifecycle owner is live.
        alloc_id: AllocationId,
    },
    /// A replacement start could not retire its prior owner.
    StartPriorTeardownFailed {
        /// Allocation whose prior owner remains retained.
        alloc_id: AllocationId,
        /// Stable transient teardown diagnostics.
        failures: Vec<String>,
    },
    /// A stop completed, including the state observed at entry.
    StopCompleted {
        /// Allocation whose lifecycle owner is absent after completion.
        alloc_id: AllocationId,
        /// State retained by the allocation at stop entry, if any.
        prior: Option<SimMtlsInterceptLifecycleState>,
    },
    /// A stop fault retained lifecycle ownership for retry.
    StopFailed {
        /// Allocation whose lifecycle owner remains teardown-pending.
        alloc_id: AllocationId,
        /// Stable transient teardown diagnostics.
        failures: Vec<String>,
    },
}

/// Atomic simulation observation of lifecycle state and completed outcomes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimMtlsInterceptLifecycleSnapshot {
    /// Current ownership state by allocation.
    pub allocations: BTreeMap<AllocationId, SimMtlsInterceptLifecycleState>,
    /// Completed outcomes in call order.
    pub events: Vec<SimMtlsInterceptLifecycleEvent>,
}

#[derive(Default)]
struct State {
    allocations: BTreeMap<AllocationId, SimMtlsInterceptLifecycleState>,
    stop_faults: BTreeMap<AllocationId, VecDeque<String>>,
    events: Vec<SimMtlsInterceptLifecycleEvent>,
}

/// In-memory implementation of [`MtlsInterceptLifecycle`].
pub struct SimMtlsInterceptLifecycle {
    state: Mutex<State>,
}

impl SimMtlsInterceptLifecycle {
    /// Construct an empty lifecycle simulation with no scripted failures.
    #[must_use]
    pub fn new() -> Self {
        Self { state: Mutex::new(State::default()) }
    }

    /// Queue one transient stop failure for `alloc_id`.
    ///
    /// Queued failures are FIFO per allocation. An absent stop is idempotent
    /// and deliberately does not consume a queued fault.
    pub fn inject_stop_failure_once(&self, alloc_id: AllocationId, detail: impl Into<String>) {
        self.state.lock().stop_faults.entry(alloc_id).or_default().push_back(detail.into());
    }

    /// Return one atomic state-and-outcome snapshot.
    #[must_use]
    pub fn snapshot(&self) -> SimMtlsInterceptLifecycleSnapshot {
        let state = self.state.lock();
        SimMtlsInterceptLifecycleSnapshot {
            allocations: state.allocations.clone(),
            events: state.events.clone(),
        }
    }

    fn stop_locked(
        state: &mut State,
        alloc_id: &AllocationId,
    ) -> Result<Option<SimMtlsInterceptLifecycleState>, MtlsInterceptStopError> {
        let prior = state.allocations.get(alloc_id).copied();
        let Some(prior) = prior else {
            state.events.push(SimMtlsInterceptLifecycleEvent::StopCompleted {
                alloc_id: alloc_id.clone(),
                prior: None,
            });
            return Ok(None);
        };

        state.allocations.insert(alloc_id.clone(), SimMtlsInterceptLifecycleState::TeardownPending);
        if let Some(detail) = state.stop_faults.get_mut(alloc_id).and_then(VecDeque::pop_front) {
            let failures = vec![detail];
            state.events.push(SimMtlsInterceptLifecycleEvent::StopFailed {
                alloc_id: alloc_id.clone(),
                failures: failures.clone(),
            });
            return Err(MtlsInterceptStopError { alloc_id: alloc_id.clone(), failures });
        }

        state.allocations.remove(alloc_id);
        state.events.push(SimMtlsInterceptLifecycleEvent::StopCompleted {
            alloc_id: alloc_id.clone(),
            prior: Some(prior),
        });
        Ok(Some(prior))
    }
}

impl Default for SimMtlsInterceptLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MtlsInterceptLifecycle for SimMtlsInterceptLifecycle {
    async fn start_alloc(&self, spec: &AllocationSpec) -> Result<(), MtlsInterceptInstallError> {
        let mut state = self.state.lock();
        if state.allocations.contains_key(&spec.alloc) {
            let prior_events = state.events.len();
            if let Err(source) = Self::stop_locked(&mut state, &spec.alloc) {
                state.events.truncate(prior_events);
                state.events.push(SimMtlsInterceptLifecycleEvent::StartPriorTeardownFailed {
                    alloc_id: spec.alloc.clone(),
                    failures: source.failures.clone(),
                });
                return Err(MtlsInterceptInstallError::PriorTeardown { source });
            }
        }
        state.allocations.insert(spec.alloc.clone(), SimMtlsInterceptLifecycleState::Live);
        state
            .events
            .push(SimMtlsInterceptLifecycleEvent::StartCompleted { alloc_id: spec.alloc.clone() });
        drop(state);
        Ok(())
    }

    async fn stop_alloc(&self, alloc_id: &AllocationId) -> Result<(), MtlsInterceptStopError> {
        let mut state = self.state.lock();
        Self::stop_locked(&mut state, alloc_id).map(|_| ())
    }
}

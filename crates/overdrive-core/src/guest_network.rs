//! Shared guest-network admission capabilities.
//!
//! RED-ready API scaffold for GH #295. The public surface is copied exactly
//! from the accepted DESIGN contract in
//! `docs/feature/netns-density-295/feature-delta.md` under
//! "EXEC-close linearization". DELIVER replaces the scaffold bodies; DISTILL
//! intentionally supplies no production behaviour.

// SCAFFOLD: true — netns-density-295 DISTILL.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::Clock;
use parking_lot::Mutex;
use tokio::sync::Notify;

/// Paired construction authority for the release and recovery capabilities.
pub struct GuestNetworkExecWiring {
    gate: Arc<GuestNetworkExecGate>,
    supervisor: Arc<GuestNetworkExecSupervisor>,
}

/// Read/claim capability retained by the VM driver.
pub struct GuestNetworkExecGate {
    shared: Arc<GuestNetworkExecShared>,
}

/// Recovery/reopen/fail-stop capability retained by the control plane.
pub struct GuestNetworkExecSupervisor {
    shared: Arc<GuestNetworkExecShared>,
}

/// RAII claim held across one deferred-EXEC writer acknowledgement.
#[must_use]
pub struct GuestNetworkExecClaim {
    shared: Arc<GuestNetworkExecShared>,
}

struct GuestNetworkExecShared {
    state: Mutex<GuestNetworkExecState>,
    notify: Notify,
    clock: Arc<dyn Clock>,
}

struct GuestNetworkExecState {
    state: GuestNetworkExecGateState,
    active_claims: usize,
}

struct RecoverySnapshot {
    component: SharedGuestNetworkComponent,
    started_at: Instant,
    completed_attempts: u32,
}

enum GuestNetworkExecGateState {
    BootClosed,
    Open,
    Recovering(RecoverySnapshot),
    FailStop,
}

/// Closed vocabulary for the first unhealthy shared-network component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedGuestNetworkComponent {
    /// Shared bridge identity or membership.
    Bridge,
    /// Workload-facing plaintext listener.
    LegF,
    /// Peer-facing inbound TLS listener.
    LegC,
    /// Shared-gateway DNS owner.
    Dns,
    /// Per-TAP TCX link identity.
    TcxLink,
    /// Endpoint map identity or content.
    EndpointMap,
    /// Counter map identity.
    CounterMap,
    /// Owned bpffs pin identity.
    BpffsPin,
    /// Bridge proof-mark guard.
    BridgeGuard,
    /// Constant IP rule program.
    IpRules,
    /// Shared IP sets and their schemas.
    IpSets,
    /// Shared-owner supervisor itself.
    Supervisor,
}

/// Closed vocabulary for terminal shared-network supervisor outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedGuestNetworkFailStopCause {
    /// The bounded recovery deadline elapsed.
    RecoveryDeadlineExceeded,
    /// The supervisor returned without a fail-stop request.
    SupervisorReturned,
    /// The supervisor returned a typed failure.
    SupervisorFailed,
    /// The supervisor task panicked.
    SupervisorPanicked,
    /// The supervisor task was cancelled unexpectedly.
    SupervisorCancelled,
    /// The fail-stop request channel closed unexpectedly.
    RequestChannelClosed,
}

/// Read-only recovery progress while the EXEC gate is recovering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedGuestNetworkRecovery {
    /// First component still failing in the accepted audit order.
    pub component: SharedGuestNetworkComponent,
    /// Completed converge-plus-full-read-back attempts.
    pub attempts: u32,
    /// Monotonic time elapsed since detection.
    pub elapsed: Duration,
}

/// Public fail-stop receipt sent to the CLI-owned serve lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedGuestNetworkFailStop {
    /// Latest failing component, or [`SharedGuestNetworkComponent::Supervisor`].
    pub component: SharedGuestNetworkComponent,
    /// Terminal cause observed by the sole supervisor owner.
    pub cause: SharedGuestNetworkFailStopCause,
    /// Completed recovery attempts.
    pub attempts: u32,
    /// Monotonic time elapsed since detection.
    pub elapsed: Duration,
}

/// Internal request crossing from the server handle to the CLI lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServeShutdownRequest {
    /// Shared guest-network ownership could not be recovered.
    SharedGuestNetwork(SharedGuestNetworkFailStop),
}

impl GuestNetworkExecWiring {
    /// Construct one BootClosed state and its paired opaque capabilities.
    /// CONTRACT_SHAPE: pure-function.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        let shared = Arc::new(GuestNetworkExecShared {
            state: Mutex::new(GuestNetworkExecState {
                state: GuestNetworkExecGateState::BootClosed,
                active_claims: 0,
            }),
            notify: Notify::new(),
            clock,
        });
        Self {
            gate: Arc::new(GuestNetworkExecGate { shared: Arc::clone(&shared) }),
            supervisor: Arc::new(GuestNetworkExecSupervisor { shared }),
        }
    }

    /// Return the VM driver's claim-only capability.
    #[must_use]
    pub fn gate(&self) -> Arc<GuestNetworkExecGate> {
        Arc::clone(&self.gate)
    }

    /// Return the control-plane recovery capability.
    #[must_use]
    pub fn supervisor(&self) -> Arc<GuestNetworkExecSupervisor> {
        Arc::clone(&self.supervisor)
    }
}

impl GuestNetworkExecGate {
    /// Wait while closed/recovering, claim when open, or return `None` at fail-stop.
    #[must_use]
    pub async fn claim_release(&self) -> Option<GuestNetworkExecClaim> {
        loop {
            let mut notified = Box::pin(self.shared.notify.notified());
            let claim = {
                let mut shared = self.shared.state.lock();
                match &shared.state {
                    GuestNetworkExecGateState::Open => {
                        shared.active_claims = shared.active_claims.saturating_add(1);
                        drop(shared);
                        Some(GuestNetworkExecClaim { shared: Arc::clone(&self.shared) })
                    }
                    GuestNetworkExecGateState::FailStop => return None,
                    GuestNetworkExecGateState::BootClosed
                    | GuestNetworkExecGateState::Recovering(_) => {
                        notified.as_mut().enable();
                        None
                    }
                }
            };
            if claim.is_some() {
                return claim;
            }
            notified.await;
        }
    }
}

impl GuestNetworkExecSupervisor {
    /// Report whether fresh-process target recovery may mutate owned targets.
    #[must_use]
    pub fn is_boot_closed(&self) -> bool {
        matches!(self.shared.state.lock().state, GuestNetworkExecGateState::BootClosed)
    }

    /// Perform the sole BootClosed-to-Open transition.
    #[must_use]
    pub fn open_after_boot(&self) -> bool {
        let opened = {
            let mut shared = self.shared.state.lock();
            if matches!(shared.state, GuestNetworkExecGateState::BootClosed) {
                shared.state = GuestNetworkExecGateState::Open;
                true
            } else {
                false
            }
        };
        if opened {
            self.shared.notify.notify_waiters();
        }
        opened
    }

    /// Perform the sole Open-to-Recovering transition.
    #[must_use]
    pub fn begin_recovery(&self, component: SharedGuestNetworkComponent) -> bool {
        let began = {
            let mut shared = self.shared.state.lock();
            if matches!(shared.state, GuestNetworkExecGateState::Open) {
                shared.state = GuestNetworkExecGateState::Recovering(RecoverySnapshot {
                    component,
                    started_at: self.shared.clock.now(),
                    completed_attempts: 0,
                });
                true
            } else {
                false
            }
        };
        if began {
            self.shared.notify.notify_waiters();
        }
        began
    }

    /// Record one completed attempt and either remain recovering or reopen.
    #[must_use]
    pub fn complete_attempt(&self, first_remaining: Option<SharedGuestNetworkComponent>) -> bool {
        let completed = {
            let mut shared = self.shared.state.lock();
            let GuestNetworkExecGateState::Recovering(snapshot) = &mut shared.state else {
                return false;
            };
            let attempts = snapshot.completed_attempts.saturating_add(1);
            match first_remaining {
                Some(component) => {
                    snapshot.component = component;
                    snapshot.completed_attempts = attempts;
                }
                None => shared.state = GuestNetworkExecGateState::Open,
            }
            true
        };
        if completed {
            self.shared.notify.notify_waiters();
        }
        completed
    }

    /// Project the latest immutable recovery snapshot.
    #[must_use]
    pub fn recovery_progress(&self) -> Option<SharedGuestNetworkRecovery> {
        let shared = self.shared.state.lock();
        let GuestNetworkExecGateState::Recovering(snapshot) = &shared.state else {
            return None;
        };
        let progress = SharedGuestNetworkRecovery {
            component: snapshot.component,
            attempts: snapshot.completed_attempts,
            elapsed: self.shared.clock.now().saturating_duration_since(snapshot.started_at),
        };
        drop(shared);
        Some(progress)
    }

    /// Enter FailStop once and return the first public request receipt.
    /// CONTRACT_SHAPE: bounded-change.
    #[must_use]
    pub fn fail_stop(
        &self,
        cause: SharedGuestNetworkFailStopCause,
    ) -> Option<SharedGuestNetworkFailStop> {
        let request = {
            let mut shared = self.shared.state.lock();
            let (component, attempts, elapsed) = match &shared.state {
                GuestNetworkExecGateState::FailStop => return None,
                GuestNetworkExecGateState::Recovering(snapshot) => (
                    snapshot.component,
                    snapshot.completed_attempts,
                    self.shared.clock.now().saturating_duration_since(snapshot.started_at),
                ),
                GuestNetworkExecGateState::BootClosed | GuestNetworkExecGateState::Open => {
                    (SharedGuestNetworkComponent::Supervisor, 0, Duration::ZERO)
                }
            };
            shared.state = GuestNetworkExecGateState::FailStop;
            let request = SharedGuestNetworkFailStop { component, cause, attempts, elapsed };
            drop(shared);
            request
        };
        self.shared.notify.notify_waiters();
        Some(request)
    }
}

impl Drop for GuestNetworkExecClaim {
    fn drop(&mut self) {
        let mut shared = self.shared.state.lock();
        debug_assert!(shared.active_claims > 0, "an EXEC claim must be accounted before Drop");
        shared.active_claims = shared.active_claims.saturating_sub(1);
        drop(shared);
        self.shared.notify.notify_waiters();
    }
}

//! Shared guest-network admission capabilities.
//!
//! RED-ready API scaffold for GH #295. The public surface is copied exactly
//! from the accepted DESIGN contract in
//! `docs/feature/netns-density-295/feature-delta.md` under
//! "EXEC-close linearization". DELIVER replaces the scaffold bodies; DISTILL
//! intentionally supplies no production behaviour.

// SCAFFOLD: true — netns-density-295 DISTILL.

#![expect(
    clippy::unused_async,
    clippy::unused_self,
    reason = "the accepted instance-method API is scaffolded before its private state machine"
)]

use std::sync::Arc;
use std::time::Duration;

use crate::Clock;

/// Paired construction authority for the release and recovery capabilities.
pub struct GuestNetworkExecWiring {
    gate: Arc<GuestNetworkExecGate>,
    supervisor: Arc<GuestNetworkExecSupervisor>,
}

/// Read/claim capability retained by the VM driver.
pub struct GuestNetworkExecGate {
    _private: (),
}

/// Recovery/reopen/fail-stop capability retained by the control plane.
pub struct GuestNetworkExecSupervisor {
    _private: (),
}

/// RAII claim held across one deferred-EXEC writer acknowledgement.
#[must_use]
pub struct GuestNetworkExecClaim {
    _private: (),
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
    pub fn new(_clock: Arc<dyn Clock>) -> Self {
        Self {
            gate: Arc::new(GuestNetworkExecGate { _private: () }),
            supervisor: Arc::new(GuestNetworkExecSupervisor { _private: () }),
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
    #[expect(clippy::panic, reason = "RED scaffold; DELIVER implements claim linearization")]
    pub async fn claim_release(&self) -> Option<GuestNetworkExecClaim> {
        panic!("Not yet implemented -- RED scaffold (netns-density-295 EXEC claim)")
    }
}

impl GuestNetworkExecSupervisor {
    /// Report whether fresh-process target recovery may mutate owned targets.
    #[must_use]
    #[expect(clippy::panic, reason = "RED scaffold; DELIVER implements BootClosed observation")]
    pub fn is_boot_closed(&self) -> bool {
        panic!("Not yet implemented -- RED scaffold (netns-density-295 BootClosed)")
    }

    /// Perform the sole BootClosed-to-Open transition.
    #[must_use]
    #[expect(clippy::panic, reason = "RED scaffold; DELIVER implements boot opening")]
    pub fn open_after_boot(&self) -> bool {
        panic!("Not yet implemented -- RED scaffold (netns-density-295 boot open)")
    }

    /// Perform the sole Open-to-Recovering transition.
    #[must_use]
    #[expect(clippy::panic, reason = "RED scaffold; DELIVER implements recovery detection")]
    pub fn begin_recovery(&self, _component: SharedGuestNetworkComponent) -> bool {
        panic!("Not yet implemented -- RED scaffold (netns-density-295 recovery begin)")
    }

    /// Record one completed attempt and either remain recovering or reopen.
    #[must_use]
    #[expect(clippy::panic, reason = "RED scaffold; DELIVER implements recovery completion")]
    pub fn complete_attempt(&self, _first_remaining: Option<SharedGuestNetworkComponent>) -> bool {
        panic!("Not yet implemented -- RED scaffold (netns-density-295 recovery attempt)")
    }

    /// Project the latest immutable recovery snapshot.
    #[must_use]
    #[expect(clippy::panic, reason = "RED scaffold; DELIVER implements recovery projection")]
    pub fn recovery_progress(&self) -> Option<SharedGuestNetworkRecovery> {
        panic!("Not yet implemented -- RED scaffold (netns-density-295 recovery progress)")
    }

    /// Enter FailStop once and return the first public request receipt.
    /// CONTRACT_SHAPE: bounded-change.
    #[must_use]
    #[expect(
        clippy::panic,
        reason = "RED scaffold; DELIVER implements first-request-wins fail-stop"
    )]
    pub fn fail_stop(
        &self,
        _cause: SharedGuestNetworkFailStopCause,
    ) -> Option<SharedGuestNetworkFailStop> {
        panic!("Not yet implemented -- RED scaffold (netns-density-295 fail-stop)")
    }
}

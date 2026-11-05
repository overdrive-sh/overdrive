//! The dedicated gateway SVID slot (public-ingress-gateway, ADR-0225).
//!
//! `GatewayIdentitySlot` holds exactly one non-allocation SVID generation for
//! the node's gateway subject, under the existing dataplane mTLS owner. It
//! never enters `IdentityMgr` or the generic `IdentityRead`. Its lock also owns
//! a process-local desired epoch: the constructor's epoch one means
//! enabled; `begin_disable` increments the epoch (a private checked add whose
//! overflow is `unreachable!()`, R-30) and empties the slot in the same call
//! (R-4: disable is a synchronous clear); `hold_audited` rejects an
//! action carrying a stale epoch, so an in-flight issue cannot re-hold after a
//! disable.
//!
//! `GatewayClientIdentityAccess` and its source trait are defined in
//! `overdrive-gateway::ports` (R-20); this module owns the private source the
//! slot and the injected clock form behind `client_access`.

#![allow(
    clippy::todo,
    clippy::needless_pass_by_value,
    clippy::unused_self,
    dead_code,
    unused_variables,
    reason = "public-ingress DISTILL scaffold: `client_access` and `SlotIdentitySource::with_current` are a later step's scaffolds"
)]

use std::num::NonZeroU64;
use std::sync::Arc;

use overdrive_core::ca::issue_failure::SvidIssueFailure;
use overdrive_core::gateway_identity::{
    GatewayIdentityDesired, GatewayIdentityEpoch, GatewayIdentityFacts, GatewayIdentityIssueRecord,
    GatewayIdentityView,
};
use overdrive_core::id::{NodeId, SpiffeId};
use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::clock::Clock;
use overdrive_gateway::ports::{
    GatewayClientIdentityAccess, GatewayClientIdentitySource, GatewayClientMtlsError,
    GatewayIdentityActionTarget, GatewayIdentityLifecycleTarget, GatewayIdentityMutationOutcome,
};
use parking_lot::Mutex;
use tokio::sync::watch;

/// The mutable state behind the slot's one lock: the desired epoch and the held
/// generation. Enabled is `epoch == first`; `begin_disable` is the only
/// transition away from it.
struct SlotState {
    epoch: GatewayIdentityEpoch,
    held: Option<SvidMaterial>,
    issue: Option<GatewayIdentityIssueRecord>,
}

/// The single gateway SVID holder.
pub struct GatewayIdentitySlot {
    node_id: NodeId,
    trust_bundle: TrustBundle,
    state: Mutex<SlotState>,
    facts: watch::Sender<Option<GatewayIdentityFacts>>,
}

impl GatewayIdentitySlot {
    /// Epoch one, enabled, empty.
    pub fn new(node_id: NodeId, trust_bundle: TrustBundle) -> Self {
        let (facts, _initial_receiver) = watch::channel(None);
        Self {
            node_id,
            trust_bundle,
            state: Mutex::new(SlotState {
                epoch: GatewayIdentityEpoch::first(),
                held: None,
                issue: None,
            }),
            facts,
        }
    }

    /// The checked desired epoch and subject.
    pub fn desired(&self) -> GatewayIdentityDesired {
        let epoch = self.state.lock().epoch;
        let enabled = epoch == GatewayIdentityEpoch::first();
        GatewayIdentityDesired {
            epoch,
            spiffe_id: enabled.then(|| SpiffeId::for_gateway(&self.node_id)),
        }
    }

    /// Non-secret facts of the held SVID, `None` while empty.
    pub fn current(&self) -> Option<GatewayIdentityFacts> {
        self.facts.borrow().clone()
    }

    /// Currentness stream of the non-secret facts.
    pub fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        self.facts.subscribe()
    }

    /// The capability the exact-peer client borrows held material through.
    pub fn client_access(self: &Arc<Self>, clock: Arc<dyn Clock>) -> GatewayClientIdentityAccess {
        todo!("SCAFFOLD: GatewayIdentitySlot::client_access")
    }

    /// The write-only action capability installed in the application state.
    pub fn action_target(self: &Arc<Self>) -> GatewayIdentityActionHandle {
        GatewayIdentityActionHandle { slot: Arc::clone(self) }
    }

    /// The lifecycle capability the control-plane adapter calls to disable.
    pub fn lifecycle_target(self: &Arc<Self>) -> GatewayIdentityLifecycleHandle {
        GatewayIdentityLifecycleHandle { slot: Arc::clone(self) }
    }

    /// Bump the epoch and empty the slot in the same call. Infallible (R-30):
    /// the epoch is a u64; the private checked add overflowing is
    /// `unreachable!()`.
    pub(crate) fn begin_disable(&self) -> GatewayIdentityEpoch {
        let mut state = self.state.lock();
        let next = state
            .epoch
            .get()
            .checked_add(1)
            .unwrap_or_else(|| unreachable!("a u64 epoch cannot be exhausted by disables"));
        state.epoch = GatewayIdentityEpoch::new(next.get())
            .unwrap_or_else(|_| unreachable!("a checked add of a nonzero epoch is nonzero"));
        state.held = None;
        self.facts.send_replace(None);
        let epoch = state.epoch;
        drop(state);
        epoch
    }

    /// Count one confirmed issue failure and keep the latest cause. The held
    /// identity, `desired()` and `current()` are untouched; subscribers are
    /// notified.
    pub(crate) fn record_issue_failure(&self, failure: SvidIssueFailure) {
        let mut state = self.state.lock();
        let count = state.issue.map_or(NonZeroU64::MIN, |record| record.count.saturating_add(1));
        state.issue = Some(GatewayIdentityIssueRecord { count, last: failure });
        self.facts.send_modify(|_| {});
        drop(state);
    }

    /// The process-local issue-failure record, `None` before any failure.
    pub fn issue_record(&self) -> Option<GatewayIdentityIssueRecord> {
        self.state.lock().issue
    }

    /// Hold `material` for `epoch` after the audit row is durable; a stale
    /// epoch is a successful no-op.
    pub(crate) fn hold_audited(
        &self,
        epoch: GatewayIdentityEpoch,
        material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome {
        let mut state = self.state.lock();
        if epoch != state.epoch {
            return GatewayIdentityMutationOutcome::StaleEpoch;
        }
        let facts = GatewayIdentityFacts {
            epoch,
            spiffe_id: material.spiffe_id().clone(),
            serial: material.serial().clone(),
            not_after: material.not_after(),
        };
        state.held = Some(material);
        self.facts.send_replace(Some(facts));
        drop(state);
        GatewayIdentityMutationOutcome::Applied
    }
}

impl GatewayIdentityView for GatewayIdentitySlot {
    fn desired(&self) -> GatewayIdentityDesired {
        Self::desired(self)
    }

    fn current(&self) -> Option<GatewayIdentityFacts> {
        Self::current(self)
    }
}

/// Write-only capability: the only implementer of `GatewayIdentityActionTarget`.
pub struct GatewayIdentityActionHandle {
    slot: Arc<GatewayIdentitySlot>,
}

/// Lifecycle capability: the only implementer of `GatewayIdentityLifecycleTarget`.
pub struct GatewayIdentityLifecycleHandle {
    slot: Arc<GatewayIdentitySlot>,
}

impl GatewayIdentityActionTarget for GatewayIdentityActionHandle {
    fn hold_audited(
        &self,
        epoch: GatewayIdentityEpoch,
        material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome {
        self.slot.hold_audited(epoch, material)
    }

    fn record_issue_failure(&self, failure: SvidIssueFailure) {
        self.slot.record_issue_failure(failure);
    }
}

impl GatewayIdentityLifecycleTarget for GatewayIdentityLifecycleHandle {
    fn current(&self) -> Option<GatewayIdentityFacts> {
        self.slot.current()
    }

    fn issue_record(&self) -> Option<GatewayIdentityIssueRecord> {
        self.slot.issue_record()
    }

    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        self.slot.subscribe()
    }

    fn begin_disable(&self) -> GatewayIdentityEpoch {
        self.slot.begin_disable()
    }
}

/// The dataplane-private source `client_access` wraps: the slot and the
/// injected clock.
///
/// Runs the closure while the slot keeps the held SVID borrowed. It reads the
/// injected clock, rejects absent or expired material, and cannot return or
/// clone the SVID, key or trust bundle.
struct SlotIdentitySource {
    slot: Arc<GatewayIdentitySlot>,
    clock: Arc<dyn Clock>,
}

impl GatewayClientIdentitySource for SlotIdentitySource {
    fn with_current(
        &self,
        use_material: &mut dyn FnMut(
            &SvidMaterial,
            &TrustBundle,
        ) -> Result<(), GatewayClientMtlsError>,
    ) -> Result<(), GatewayClientMtlsError> {
        todo!("SCAFFOLD: SlotIdentitySource::with_current")
    }
}

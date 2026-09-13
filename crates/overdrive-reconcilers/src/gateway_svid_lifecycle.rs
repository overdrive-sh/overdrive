//! Dedicated gateway-SVID lifecycle reconciler.
//!
//! SCAFFOLD: true. The canonical registration and exhaustive dispatch surface
//! are present; DELIVER replaces the marked reconciliation/hydration behavior.

#![expect(clippy::todo, reason = "public-ingress DISTILL RED scaffolds")]

use overdrive_core::gateway_identity::{GatewayIdentityDesired, GatewayIdentityFacts};
use overdrive_core::reconcilers::{
    Action, HydrateError, HydrationContext, Reconciler, ReconcilerName, ResyncSchedule,
    TargetResource, TickContext,
};
use overdrive_core::traits::observation_store::ObservationRowKind;
use overdrive_core::wall_clock::UnixInstant;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewaySvidLifecycleState {
    pub desired: GatewayIdentityDesired,
    pub actual: Option<GatewayIdentityFacts>,
    pub ever_issued: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewaySvidLifecycleView {
    pub retry: Option<crate::svid_lifecycle::IssueRetry>,
}

pub struct GatewaySvidLifecycle {
    name: ReconcilerName,
}

impl GatewaySvidLifecycle {
    #[must_use]
    pub fn canonical() -> Self {
        let name = ReconcilerName::new(Self::NAME)
            .unwrap_or_else(|_| unreachable!("canonical gateway reconciler name is valid"));
        Self { name }
    }
}

#[async_trait::async_trait]
impl Reconciler for GatewaySvidLifecycle {
    const NAME: &'static str = "gateway-svid-lifecycle";
    type State = GatewaySvidLifecycleState;
    type View = GatewaySvidLifecycleView;

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    fn reconcile(
        &self,
        _desired: &Self::State,
        _actual: &Self::State,
        _view: &Self::View,
        _tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        todo!("SCAFFOLD: GatewaySvidLifecycle::reconcile")
    }

    fn next_evaluation_at(
        &self,
        _desired: &Self::State,
        _actual: &Self::State,
        _next_view: &Self::View,
        _tick: &TickContext,
    ) -> Option<UnixInstant> {
        todo!("SCAFFOLD: GatewaySvidLifecycle::next_evaluation_at")
    }

    async fn hydrate_desired(
        &self,
        _ctx: &HydrationContext<'_>,
        _target: &TargetResource,
    ) -> Result<Self::State, HydrateError> {
        todo!("SCAFFOLD: GatewaySvidLifecycle::hydrate_desired")
    }

    async fn hydrate_actual(
        &self,
        _ctx: &HydrationContext<'_>,
        _target: &TargetResource,
    ) -> Result<Self::State, HydrateError> {
        todo!("SCAFFOLD: GatewaySvidLifecycle::hydrate_actual")
    }

    fn resync_schedule(&self) -> Option<ResyncSchedule> {
        None
    }

    fn interests(&self) -> &'static [ObservationRowKind] {
        &[ObservationRowKind::IssuedCertificate]
    }
}

#[cfg(test)]
mod acceptance {
    use std::time::{Duration, Instant};

    use overdrive_core::gateway_identity::GatewayIdentityEpoch;
    use overdrive_core::id::{CertSerial, SpiffeId};

    use super::*;

    fn tick(now: UnixInstant) -> TickContext {
        TickContext {
            now: Instant::now(),
            now_unix: now,
            tick: 1,
            deadline: Instant::now() + Duration::from_secs(1),
        }
    }

    fn desired(enabled: bool) -> GatewayIdentityDesired {
        GatewayIdentityDesired {
            epoch: GatewayIdentityEpoch::new(1).expect("epoch"),
            spiffe_id: enabled
                .then(|| SpiffeId::new("spiffe://overdrive.local/gateway/node-a").expect("SPIFFE")),
        }
    }

    fn facts(not_after: u64) -> GatewayIdentityFacts {
        GatewayIdentityFacts {
            epoch: GatewayIdentityEpoch::new(1).expect("epoch"),
            spiffe_id: SpiffeId::new("spiffe://overdrive.local/gateway/node-a").expect("SPIFFE"),
            serial: CertSerial::new("01").expect("serial"),
            not_after: UnixInstant::from_unix_duration(Duration::from_secs(not_after)),
        }
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER GatewaySvidLifecycle transition and retry policy"]
    fn absent_current_expiring_replay_and_disable_cover_every_identity_transition() {
        let reconciler = GatewaySvidLifecycle::canonical();
        let now = UnixInstant::from_unix_duration(Duration::from_secs(1_000));
        let view = GatewaySvidLifecycleView::default();
        let enabled_empty =
            GatewaySvidLifecycleState { desired: desired(true), actual: None, ever_issued: false };
        let (issue, _) = reconciler.reconcile(&enabled_empty, &enabled_empty, &view, &tick(now));
        assert!(matches!(issue.as_slice(), [Action::IssueGatewaySvid { .. }]));

        let current = GatewaySvidLifecycleState {
            desired: desired(true),
            actual: Some(facts(10_000)),
            ever_issued: true,
        };
        let (replay, _) = reconciler.reconcile(&current, &current, &view, &tick(now));
        assert!(replay.is_empty(), "apply-twice/current replay is idempotent");

        let expiring = GatewaySvidLifecycleState {
            desired: desired(true),
            actual: Some(facts(1_001)),
            ever_issued: true,
        };
        let (reissue, _) = reconciler.reconcile(&expiring, &expiring, &view, &tick(now));
        assert!(matches!(reissue.as_slice(), [Action::IssueGatewaySvid { .. }]));

        let disabled = GatewaySvidLifecycleState {
            desired: desired(false),
            actual: Some(facts(10_000)),
            ever_issued: true,
        };
        let (drop, _) = reconciler.reconcile(&disabled, &disabled, &view, &tick(now));
        assert!(matches!(drop.as_slice(), [Action::DropGatewaySvid { .. }]));

        let mismatched = GatewaySvidLifecycleState {
            desired: desired(true),
            actual: Some(GatewayIdentityFacts {
                epoch: GatewayIdentityEpoch::new(1).expect("epoch"),
                spiffe_id: SpiffeId::new("spiffe://overdrive.local/gateway/other-node")
                    .expect("mismatched SPIFFE"),
                serial: CertSerial::new("02").expect("serial"),
                not_after: UnixInstant::from_unix_duration(Duration::from_secs(10_000)),
            }),
            ever_issued: true,
        };
        let (replace_malformed, _) =
            reconciler.reconcile(&mismatched, &mismatched, &view, &tick(now));
        assert!(matches!(replace_malformed.as_slice(), [Action::IssueGatewaySvid { .. }]));
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER GatewaySvidLifecycle transition and retry policy"]
    fn retry_memory_suppresses_early_issue_and_reemits_at_the_exact_deadline() {
        let reconciler = GatewaySvidLifecycle::canonical();
        let last_failure = UnixInstant::from_unix_duration(Duration::from_secs(1_000));
        let retry =
            crate::svid_lifecycle::IssueRetry { attempts: 2, last_failure_seen_at: last_failure };
        let view = GatewaySvidLifecycleView { retry: Some(retry.clone()) };
        let state =
            GatewaySvidLifecycleState { desired: desired(true), actual: None, ever_issued: false };
        let (early, early_view) = reconciler.reconcile(&state, &state, &view, &tick(last_failure));
        assert!(early.is_empty());
        assert_eq!(early_view.retry, Some(retry));

        let deadline = last_failure + crate::backoff_for_attempt(2);
        let (due, due_view) = reconciler.reconcile(&state, &state, &view, &tick(deadline));
        assert!(matches!(due.as_slice(), [Action::IssueGatewaySvid { .. }]));
        let next_retry = due_view.retry.expect("issue attempt retains retry input");
        assert_eq!(next_retry.attempts, 3);
        assert_eq!(next_retry.last_failure_seen_at, deadline);
    }
}

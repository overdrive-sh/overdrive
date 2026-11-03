//! `gateway-svid-lifecycle` — the dedicated gateway-SVID reconciler (ADR-0225).
//!
//! It converges the one non-allocation gateway identity: `desired` is the
//! checked epoch and optional subject read through the single
//! [`GatewayIdentityView`](overdrive_core::gateway_identity::GatewayIdentityView)
//! port, `actual` is the held non-secret facts of the dedicated slot. It emits
//! [`Action::IssueGatewaySvid`] only; there is no gateway drop action (R-4):
//! disabling the gateway clears the volatile slot synchronously in the process.
//!
//! SCAFFOLD: true. The canonical constructor, the declared wakeup model and the
//! `State` / `View` shapes are the DESIGN-pinned contract. `reconcile`,
//! `next_evaluation_at` and both hydration bodies are DELIVER's and fail with
//! SCAFFOLD text. The source-local bodies below are the S-PIG-65 specification
//! of the pure half.

#![allow(
    clippy::todo,
    clippy::unused_async,
    clippy::unused_self,
    clippy::missing_const_for_fn,
    reason = "public-ingress DISTILL scaffold: reconcile, scheduling and hydration bodies are DELIVER's"
)]

use std::time::Duration;

use overdrive_core::gateway_identity::{GatewayIdentityDesired, GatewayIdentityFacts};
use overdrive_core::reconcilers::{
    Action, HydrateError, HydrationContext, Reconciler, ReconcilerName, ResyncSchedule,
    ResyncScope, TargetResource, TickContext,
};
use overdrive_core::traits::observation_store::ObservationRowKind;
use overdrive_core::wall_clock::UnixInstant;
use serde::{Deserialize, Serialize};

use crate::svid_lifecycle::IssueRetry;

/// Level-triggered backstop cadence of the host-backed gateway reconciler.
///
/// A DELIVER tuning constant, bounded above by the SVID reissue margin so a lost
/// wake cannot let a held gateway SVID reach expiry unreissued.
pub(crate) const GATEWAY_SVID_RESYNC_PERIOD: Duration = Duration::from_secs(30);

/// `desired` / `actual` projection of the gateway-SVID reconciler. One `State`
/// type serves both roles, exactly as for the workload-SVID reconciler:
/// `desired` carries the checked epoch and optional subject, `actual` carries
/// the held non-secret facts and the durable audit signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewaySvidLifecycleState {
    /// The desired epoch and optional subject.
    pub desired: GatewayIdentityDesired,
    /// The held non-secret facts, `None` while no SVID is held.
    pub actual: Option<GatewayIdentityFacts>,
    /// Whether the issued-certificate audit rows already name the gateway
    /// subject (the restart marker; recomputed each tick, never persisted).
    pub ever_issued: bool,
}

/// Retry-memory `View` of the gateway-SVID reconciler: the inputs the shared
/// backoff schedule consumes, no derived deadline.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewaySvidLifecycleView {
    /// Failed-issue retry inputs; `None` while no attempt is recorded.
    pub retry: Option<IssueRetry>,
}

/// The gateway-SVID reconciler.
pub struct GatewaySvidLifecycle {
    name: ReconcilerName,
}

impl GatewaySvidLifecycle {
    /// The canonical instance; the name is never caller-supplied.
    #[must_use]
    pub fn canonical() -> Self {
        let name = ReconcilerName::new(<Self as Reconciler>::NAME)
            .unwrap_or_else(|_| unreachable!("'gateway-svid-lifecycle' is a valid ReconcilerName"));
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

    /// Host-backed: `actual` is the in-process slot, not an observation row,
    /// and this reconciler's own executor authors the issued-certificate rows,
    /// so it declares no interest in them (the no-busy-loop rule). Its own
    /// cadence is the level-triggered backstop.
    fn resync_schedule(&self) -> Option<ResyncSchedule> {
        Some(ResyncSchedule { period: GATEWAY_SVID_RESYNC_PERIOD, scope: ResyncScope::LocalNode })
    }

    fn interests(&self) -> &'static [ObservationRowKind] {
        &[]
    }
}

#[cfg(test)]
mod acceptance {
    #![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required scenario metadata")]

    use std::time::Instant;

    use overdrive_core::gateway_identity::GatewayIdentityEpoch;
    use overdrive_core::id::{CertSerial, SpiffeId};
    use proptest::prelude::*;

    use super::*;
    use crate::svid_lifecycle::NEAR_EXPIRY_THRESHOLD_SECS;

    const GATEWAY: &str = "spiffe://overdrive.local/gateway/node-a";
    const OTHER_GATEWAY: &str = "spiffe://overdrive.local/gateway/other-node";

    fn at(seconds: u64) -> UnixInstant {
        UnixInstant::from_unix_duration(Duration::from_secs(seconds))
    }

    fn tick(now: UnixInstant) -> TickContext {
        TickContext {
            now: Instant::now(),
            now_unix: now,
            tick: 1,
            deadline: Instant::now() + Duration::from_secs(1),
        }
    }

    fn gateway_id(raw: &str) -> SpiffeId {
        SpiffeId::new(raw).expect("fixture gateway SPIFFE id")
    }

    fn desired(epoch: GatewayIdentityEpoch, enabled: bool) -> GatewayIdentityDesired {
        GatewayIdentityDesired { epoch, spiffe_id: enabled.then(|| gateway_id(GATEWAY)) }
    }

    fn facts(epoch: GatewayIdentityEpoch, subject: &str, not_after: u64) -> GatewayIdentityFacts {
        GatewayIdentityFacts {
            epoch,
            spiffe_id: gateway_id(subject),
            serial: CertSerial::new("01").expect("fixture serial"),
            not_after: at(not_after),
        }
    }

    fn state(
        desired: GatewayIdentityDesired,
        actual: Option<GatewayIdentityFacts>,
        ever_issued: bool,
    ) -> GatewaySvidLifecycleState {
        GatewaySvidLifecycleState { desired, actual, ever_issued }
    }

    /// The `IssueGatewaySvid` actions of an emitted vector; the converged
    /// no-op marker (`Action::Noop`) is the only other action permitted.
    fn gateway_issues(actions: &[Action]) -> Vec<&Action> {
        for action in actions {
            assert!(
                matches!(action, Action::IssueGatewaySvid { .. } | Action::Noop),
                "the gateway reconciler emits IssueGatewaySvid or the converged marker, got {action:?}",
            );
        }
        actions.iter().filter(|a| matches!(a, Action::IssueGatewaySvid { .. })).collect()
    }

    /// S-PIG-65 (pure half): the canonical instance carries the pinned name and
    /// the pinned host-backed wakeup declaration: empty interests, a
    /// node-scoped resync bounded by the near-expiry window, so a lost wake can
    /// never let a held gateway SVID reach expiry unreissued.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn canonical_instance_declares_the_host_backed_wakeup_model() {
        let reconciler = GatewaySvidLifecycle::canonical();
        assert_eq!(reconciler.name().as_str(), "gateway-svid-lifecycle");
        assert_eq!(<GatewaySvidLifecycle as Reconciler>::NAME, "gateway-svid-lifecycle");
        assert!(reconciler.interests().is_empty(), "no interest in the rows this executor authors");
        let schedule =
            reconciler.resync_schedule().expect("host-backed reconciler declares a cadence");
        assert_eq!(schedule.scope, ResyncScope::LocalNode);
        assert!(
            schedule.period.as_secs() < NEAR_EXPIRY_THRESHOLD_SECS,
            "the resync backstop fires inside the near-expiry window",
        );
    }

    /// S-PIG-65: every identity transition emits exactly the action the design
    /// names. Absent identity issues; a current identity of the desired epoch
    /// and subject replays to nothing; a near-expiry identity reissues; a
    /// held identity of another subject or of an earlier epoch is replaced; a
    /// disabled gateway emits no action at all (disable is the synchronous slot
    /// clear, there is no drop action).
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER gateway-svid-lifecycle-transitions"]
    fn absent_current_expiring_stale_and_disabled_cover_every_identity_transition() {
        let reconciler = GatewaySvidLifecycle::canonical();
        let now = at(1_000);
        let view = GatewaySvidLifecycleView::default();
        let first = GatewayIdentityEpoch::first();
        let second = GatewayIdentityEpoch::new(2).expect("two is a valid epoch");

        let absent = state(desired(first, true), None, false);
        let (actions, _) = reconciler.reconcile(&absent, &absent, &view, &tick(now));
        match gateway_issues(&actions).as_slice() {
            [Action::IssueGatewaySvid { epoch, spiffe_id, .. }] => {
                assert_eq!(*epoch, first, "the action carries the desired epoch");
                assert_eq!(spiffe_id.as_str(), GATEWAY, "the action carries the desired subject");
            }
            other => panic!("absent identity must emit one IssueGatewaySvid, got {other:?}"),
        }

        let current = state(desired(first, true), Some(facts(first, GATEWAY, 10_000)), true);
        let (actions, _) = reconciler.reconcile(&current, &current, &view, &tick(now));
        assert!(gateway_issues(&actions).is_empty(), "a current identity replays to nothing");

        let near = now.as_unix_duration().as_secs() + NEAR_EXPIRY_THRESHOLD_SECS;
        let at_boundary = state(desired(first, true), Some(facts(first, GATEWAY, near)), true);
        let (actions, _) = reconciler.reconcile(&at_boundary, &at_boundary, &view, &tick(now));
        assert_eq!(gateway_issues(&actions).len(), 1, "the near-expiry boundary is inclusive");

        let just_outside = state(desired(first, true), Some(facts(first, GATEWAY, near + 1)), true);
        let (actions, _) = reconciler.reconcile(&just_outside, &just_outside, &view, &tick(now));
        assert!(gateway_issues(&actions).is_empty(), "one second past the window is still current");

        let other_subject =
            state(desired(first, true), Some(facts(first, OTHER_GATEWAY, 10_000)), true);
        let (actions, _) = reconciler.reconcile(&other_subject, &other_subject, &view, &tick(now));
        assert_eq!(
            gateway_issues(&actions).len(),
            1,
            "a held identity of another subject is replaced"
        );

        let stale_epoch = state(desired(second, true), Some(facts(first, GATEWAY, 10_000)), true);
        let (actions, _) = reconciler.reconcile(&stale_epoch, &stale_epoch, &view, &tick(now));
        assert_eq!(
            gateway_issues(&actions).len(),
            1,
            "a held identity of an earlier epoch is replaced"
        );

        let disabled = state(desired(second, false), Some(facts(first, GATEWAY, 10_000)), true);
        let (actions, _) = reconciler.reconcile(&disabled, &disabled, &view, &tick(now));
        assert!(
            gateway_issues(&actions).is_empty(),
            "a disabled gateway emits no action: disable is the synchronous slot clear",
        );
    }

    /// S-PIG-65: restart recovery. The durable audit rows name the gateway
    /// subject while the volatile slot is empty: the reconciler reissues
    /// immediately, bypassing the backoff gate that a stale retry entry would
    /// otherwise apply, and clears that entry.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER gateway-svid-lifecycle-transitions"]
    fn restart_with_audited_subject_reissues_immediately_and_clears_the_retry_entry() {
        let reconciler = GatewaySvidLifecycle::canonical();
        let now = at(2_000);
        let first = GatewayIdentityEpoch::first();
        let stale_retry = IssueRetry { attempts: 4, last_failure_seen_at: now };
        let view = GatewaySvidLifecycleView { retry: Some(stale_retry) };
        let restarted = state(desired(first, true), None, true);
        let (actions, next_view) = reconciler.reconcile(&restarted, &restarted, &view, &tick(now));
        assert_eq!(gateway_issues(&actions).len(), 1, "restart recovery is not backoff-gated");
        assert_eq!(next_view.retry, None, "a durable success proves no failure is pending");
    }

    /// S-PIG-65: the retry inputs suppress an early reissue and re-emit at
    /// the exact deadline recomputed from the persisted inputs and the shared
    /// backoff policy; each emission bumps the attempt count.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER gateway-svid-lifecycle-transitions"]
    fn retry_memory_suppresses_an_early_issue_and_reemits_at_the_exact_deadline() {
        let reconciler = GatewaySvidLifecycle::canonical();
        let last_failure = at(1_000);
        let retry = IssueRetry { attempts: 2, last_failure_seen_at: last_failure };
        let view = GatewaySvidLifecycleView { retry: Some(retry.clone()) };
        let never_issued = state(desired(GatewayIdentityEpoch::first(), true), None, false);

        let (early, early_view) =
            reconciler.reconcile(&never_issued, &never_issued, &view, &tick(last_failure));
        assert!(gateway_issues(&early).is_empty(), "inside the backoff window nothing is emitted");
        assert_eq!(early_view.retry, Some(retry), "a suppressed tick leaves the entry unchanged");

        let deadline = last_failure + crate::backoff_for_attempt(2);
        let (due, due_view) =
            reconciler.reconcile(&never_issued, &never_issued, &view, &tick(deadline));
        assert_eq!(gateway_issues(&due).len(), 1, "the deadline tick re-emits");
        let next_retry = due_view.retry.expect("an issue attempt records its retry input");
        assert_eq!(next_retry.attempts, 3);
        assert_eq!(next_retry.last_failure_seen_at, deadline);
    }

    /// S-PIG-65: the emitted action carries the desired epoch and subject and
    /// the node of the issuing target, and the correlation is the same for
    /// the same desired identity on every tick (a content key, never a
    /// per-attempt id).
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER gateway-svid-lifecycle-transitions"]
    fn issue_correlation_is_stable_for_the_same_desired_identity() {
        let reconciler = GatewaySvidLifecycle::canonical();
        let first = GatewayIdentityEpoch::first();
        let absent = state(desired(first, true), None, false);
        let view = GatewaySvidLifecycleView::default();
        let correlation_at = |seconds: u64| {
            let (actions, _) = reconciler.reconcile(&absent, &absent, &view, &tick(at(seconds)));
            match gateway_issues(&actions).as_slice() {
                [Action::IssueGatewaySvid { correlation, .. }] => correlation.clone(),
                other => panic!("expected one IssueGatewaySvid, got {other:?}"),
            }
        };
        assert_eq!(correlation_at(1_000), correlation_at(9_000));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// S-PIG-65 (property): over every desired/actual/clock combination the
        /// reconciler emits an issue if and only if a gateway is desired and no
        /// identity of the desired epoch and subject is held outside the
        /// near-expiry window; it never emits a second action kind.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER gateway-svid-lifecycle-transitions"]
        fn issue_is_emitted_iff_no_usable_identity_of_the_desired_epoch_is_held(
            enabled in any::<bool>(),
            desired_epoch in 1u64..4,
            held in proptest::option::of((1u64..4, any::<bool>(), 0u64..20_000)),
            now in 0u64..10_000,
        ) {
            let reconciler = GatewaySvidLifecycle::canonical();
            let desired_epoch = GatewayIdentityEpoch::new(desired_epoch).expect("nonzero epoch");
            let actual = held.map(|(epoch, own_subject, not_after)| {
                let epoch = GatewayIdentityEpoch::new(epoch).expect("nonzero epoch");
                facts(epoch, if own_subject { GATEWAY } else { OTHER_GATEWAY }, not_after)
            });
            let view = GatewaySvidLifecycleView::default();
            let input = state(desired(desired_epoch, enabled), actual.clone(), actual.is_some());
            let (actions, _) = reconciler.reconcile(&input, &input, &view, &tick(at(now)));

            let usable = actual.as_ref().is_some_and(|held| {
                held.epoch == desired_epoch
                    && held.spiffe_id.as_str() == GATEWAY
                    && held.not_after > at(now + NEAR_EXPIRY_THRESHOLD_SECS)
            });
            let expected = usize::from(enabled && !usable);
            prop_assert_eq!(gateway_issues(&actions).len(), expected);
        }
    }
}

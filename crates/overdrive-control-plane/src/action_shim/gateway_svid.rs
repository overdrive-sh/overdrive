//! Action-shim executor for `Action::IssueGatewaySvid` (ADR-0225).
//!
//! The executor mints the gateway leaf through the existing
//! `ca_issuance::issue_and_audit` (the audit row is durable before anything is
//! held) and then offers the material, with the action's epoch, to the
//! write-only [`GatewayIdentityActionTarget`]. The target holds it only if that
//! epoch is still the desired epoch; a stale epoch is a successful no-op. There
//! is no gateway drop action (R-4): disabling the gateway clears the volatile
//! slot synchronously in the process.

use overdrive_core::reconcilers::Action;
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_gateway::ports::{GatewayIdentityActionTarget, GatewayIdentityMutationOutcome};

use super::ShimError;
use crate::ca_issuance::issue_and_audit;

/// Dispatch one `Action::IssueGatewaySvid`: `issue_and_audit`, then the
/// epoch-checked hold. Both `Applied` and `StaleEpoch` are success; the caller
/// (`dispatch_single`) maps both to `Ok(())`.
///
/// # Errors
///
/// [`ShimError::GatewaySvidIssue`] when issuance or its audit write fails; the
/// target is offered no material.
///
/// # Panics
///
/// Panics if `action` is not [`Action::IssueGatewaySvid`]; the shim's match arm
/// is the sole caller.
pub async fn dispatch_issue(
    action: &Action,
    ca: &dyn Ca,
    observation: &dyn ObservationStore,
    clock: &dyn Clock,
    target: &dyn GatewayIdentityActionTarget,
) -> Result<GatewayIdentityMutationOutcome, ShimError> {
    let Action::IssueGatewaySvid { epoch, spiffe_id, node_id, .. } = action else {
        unreachable!("dispatch_issue is called only from the IssueGatewaySvid arm")
    };
    let material = issue_and_audit(ca, observation, clock, node_id, spiffe_id)
        .await
        .map_err(ShimError::GatewaySvidIssue)?;
    Ok(target.hold_audited(*epoch, material))
}

#[cfg(test)]
mod acceptance {
    #![allow(
        clippy::doc_markdown,
        clippy::expect_used,
        reason = "CONTRACT_SHAPE is required scenario metadata; fixture preconditions panic with their cause"
    )]

    use std::sync::Arc;

    use overdrive_core::ca::issue_failure::{SvidIssueFailure, SvidIssueFailureCause};
    use overdrive_core::gateway_identity::GatewayIdentityEpoch;
    use overdrive_core::id::{CorrelationKey, NodeId, SpiffeId};
    use overdrive_core::traits::ca::SvidMaterial;
    use overdrive_core::traits::observation_store::ObservationStoreError;
    use overdrive_sim::adapters::ca::SimCa;
    use overdrive_sim::adapters::clock::SimClock;
    use overdrive_sim::adapters::entropy::SimEntropy;
    use overdrive_sim::adapters::observation_store::SimObservationStore;
    use parking_lot::Mutex;

    use super::*;
    use crate::gateway_composition::{GatewayIdentityActionComposition, GatewayIdentityDisabled};

    /// A write-only target that records what it was offered and holds only the
    /// desired epoch, as the slot does.
    struct EpochCheckingTarget {
        desired: GatewayIdentityEpoch,
        held: Mutex<Vec<(GatewayIdentityEpoch, SvidMaterial)>>,
        failures: Mutex<Vec<SvidIssueFailure>>,
    }

    impl EpochCheckingTarget {
        fn desiring(desired: GatewayIdentityEpoch) -> Self {
            Self { desired, held: Mutex::new(Vec::new()), failures: Mutex::new(Vec::new()) }
        }
    }

    impl GatewayIdentityActionTarget for EpochCheckingTarget {
        fn hold_audited(
            &self,
            epoch: GatewayIdentityEpoch,
            material: SvidMaterial,
        ) -> GatewayIdentityMutationOutcome {
            if epoch == self.desired {
                *self.held.lock() = vec![(epoch, material)];
                GatewayIdentityMutationOutcome::Applied
            } else {
                GatewayIdentityMutationOutcome::StaleEpoch
            }
        }

        fn record_issue_failure(&self, failure: SvidIssueFailure) {
            self.failures.lock().push(failure);
        }
    }

    fn node() -> NodeId {
        NodeId::new("node-a").expect("fixture node id")
    }

    fn action(epoch: GatewayIdentityEpoch) -> Action {
        Action::IssueGatewaySvid {
            epoch,
            spiffe_id: SpiffeId::new("spiffe://overdrive.local/gateway/node-a")
                .expect("fixture gateway SPIFFE id"),
            node_id: node(),
            correlation: CorrelationKey::new("gateway-svid/node-a:issue")
                .expect("fixture correlation"),
        }
    }

    fn ca(seed: u64) -> SimCa {
        SimCa::new(Arc::new(SimEntropy::new(seed)))
    }

    fn second_epoch() -> GatewayIdentityEpoch {
        GatewayIdentityEpoch::new(2).expect("two is a valid epoch")
    }

    /// S-PIG-65 (executor half): a successful issue leaves exactly one audit
    /// row for the gateway subject, bound to the material the target was
    /// offered (serial, subject and validity end), and the target holds it.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn successful_issue_audits_the_material_the_slot_then_holds() {
        let ca = ca(0x51D);
        let observation = SimObservationStore::single_peer(node(), 0x51D);
        let epoch = GatewayIdentityEpoch::first();
        let target = EpochCheckingTarget::desiring(epoch);

        let before = observation.issued_certificate_rows().await.expect("rows before");
        let outcome = dispatch_issue(&action(epoch), &ca, &observation, &SimClock::new(), &target)
            .await
            .expect("issue, audit and hold");
        let after = observation.issued_certificate_rows().await.expect("rows after");

        assert_eq!(outcome, GatewayIdentityMutationOutcome::Applied);
        assert_eq!(before.len(), 0);
        assert_eq!(after.len(), 1, "exactly one audit row is the whole observable delta");
        let held = target.held.lock();
        assert_eq!(held.len(), 1);
        let (held_epoch, material) = &held[0];
        assert_eq!(*held_epoch, epoch, "the action's epoch travels to the hold");
        assert_eq!(after[0].spiffe_id, *material.spiffe_id());
        assert_eq!(after[0].serial, *material.serial());
        assert_eq!(after[0].not_after, material.not_after());
        assert_eq!(after[0].spiffe_id.as_str(), "spiffe://overdrive.local/gateway/node-a");
        drop(held);
    }

    /// S-PIG-65 (executor half, error): an audit-write failure refuses the
    /// hold. The target is never offered material, no audit row is left, and
    /// the typed error is the gateway issuance error.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn audit_failure_refuses_the_hold_and_leaves_no_audit_row() {
        let ca = ca(0xBAD);
        let observation = SimObservationStore::single_peer(node(), 0xBAD);
        observation.inject_write_failure(ObservationStoreError::Io(std::io::Error::other(
            "injected gateway audit failure",
        )));
        let epoch = GatewayIdentityEpoch::first();
        let target = EpochCheckingTarget::desiring(epoch);

        let result =
            dispatch_issue(&action(epoch), &ca, &observation, &SimClock::new(), &target).await;

        assert!(
            matches!(result, Err(ShimError::GatewaySvidIssue(_))),
            "an audit failure is the typed gateway issuance error, got {result:?}",
        );
        assert!(target.held.lock().is_empty(), "nothing is held without a durable audit row");
        assert!(
            observation.issued_certificate_rows().await.expect("rows").is_empty(),
            "a failed audit leaves no success occurrence",
        );
    }

    /// S-PIG-65 (executor half, edge): an issue whose epoch is no longer the
    /// desired epoch (a disable raced it) still audits, then the hold is a
    /// successful stale-epoch no-op: `Ok(StaleEpoch)`, nothing held.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn stale_epoch_issue_is_audited_and_never_held() {
        let ca = ca(0x57A1E);
        let observation = SimObservationStore::single_peer(node(), 0x57A1E);
        let target = EpochCheckingTarget::desiring(second_epoch());

        let outcome = dispatch_issue(
            &action(GatewayIdentityEpoch::first()),
            &ca,
            &observation,
            &SimClock::new(),
            &target,
        )
        .await
        .expect("a stale epoch is a success, not an error");

        assert_eq!(outcome, GatewayIdentityMutationOutcome::StaleEpoch);
        assert!(target.held.lock().is_empty(), "an in-flight issue cannot re-hold after disable");
        assert_eq!(
            observation.issued_certificate_rows().await.expect("rows").len(),
            1,
            "audit precedes the hold, so the stale issue left its audit row",
        );
    }

    /// A CA whose leaf signing always fails; everything else delegates to a
    /// real `SimCa`. A failing driven-port adapter, not production surface.
    struct SigningFailsCa {
        delegate: SimCa,
    }

    impl Ca for SigningFailsCa {
        fn root(
            &self,
        ) -> overdrive_core::traits::ca::Result<overdrive_core::traits::ca::RootCaHandle> {
            self.delegate.root()
        }

        fn issue_intermediate(
            &self,
            node: &NodeId,
        ) -> overdrive_core::traits::ca::Result<overdrive_core::traits::ca::IntermediateHandle>
        {
            self.delegate.issue_intermediate(node)
        }

        fn issue_svid(
            &self,
            _req: &overdrive_core::traits::ca::SvidRequest,
        ) -> overdrive_core::traits::ca::Result<SvidMaterial> {
            Err(overdrive_core::traits::ca::CaError::signing_failed("injected signing failure"))
        }

        fn trust_bundle(
            &self,
        ) -> overdrive_core::traits::ca::Result<overdrive_core::traits::ca::TrustBundle> {
            self.delegate.trust_bundle()
        }
    }

    /// O-SIF-G1 (executor half, `Ca`): a CA that cannot sign refuses the
    /// issuance with the unchanged typed error, the target is told of exactly
    /// one confirmed `Ca` failure at the injected clock's instant, and nothing
    /// is held and no audit row exists.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER gateway-svid-issue-failure-recording"]
    async fn a_ca_signing_failure_is_recorded_once_as_a_ca_cause_and_holds_nothing() {
        let ca = SigningFailsCa { delegate: ca(0xCA) };
        let observation = SimObservationStore::single_peer(node(), 0xCA);
        let clock = SimClock::new();
        clock.tick(std::time::Duration::from_secs(90));
        let epoch = GatewayIdentityEpoch::first();
        let target = EpochCheckingTarget::desiring(epoch);

        let result = dispatch_issue(&action(epoch), &ca, &observation, &clock, &target).await;

        assert!(
            matches!(
                &result,
                Err(ShimError::GatewaySvidIssue(crate::ca_issuance::CaIssuanceError::Ca { .. }))
            ),
            "the issuance error is returned exactly as before, got {result:?}",
        );
        assert_eq!(
            *target.failures.lock(),
            vec![SvidIssueFailure {
                cause: SvidIssueFailureCause::Ca,
                at: overdrive_core::UnixInstant::from_clock(&clock),
            }],
            "one confirmed failure, classified Ca, stamped by the injected clock",
        );
        assert!(target.held.lock().is_empty(), "no material is offered or held");
        assert!(observation.issued_certificate_rows().await.expect("rows").is_empty());
    }

    /// O-SIF-G1 (executor half, `Audit`): an audit-write failure is recorded
    /// once as an `Audit` cause, and the executor still returns the typed
    /// gateway issuance error.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER gateway-svid-issue-failure-recording"]
    async fn an_audit_write_failure_is_recorded_once_as_an_audit_cause() {
        let ca = ca(0xA0D1);
        let observation = SimObservationStore::single_peer(node(), 0xA0D1);
        observation.inject_write_failure(ObservationStoreError::Io(std::io::Error::other(
            "injected gateway audit failure",
        )));
        let clock = SimClock::new();
        let epoch = GatewayIdentityEpoch::first();
        let target = EpochCheckingTarget::desiring(epoch);

        let result = dispatch_issue(&action(epoch), &ca, &observation, &clock, &target).await;

        assert!(matches!(result, Err(ShimError::GatewaySvidIssue(_))), "got {result:?}");
        assert_eq!(
            *target.failures.lock(),
            vec![SvidIssueFailure {
                cause: SvidIssueFailureCause::Audit,
                at: overdrive_core::UnixInstant::from_clock(&clock),
            }],
        );
        assert!(target.held.lock().is_empty());
    }

    /// O-SIF-G1 (executor half, success): a successful issue, and a successful
    /// issue whose epoch went stale, record nothing; the failure record is not
    /// touched by a hold.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER gateway-svid-issue-failure-recording"]
    async fn a_successful_or_stale_issue_records_no_failure() {
        let ca = ca(0x600D);
        let observation = SimObservationStore::single_peer(node(), 0x600D);
        let clock = SimClock::new();

        let current = EpochCheckingTarget::desiring(GatewayIdentityEpoch::first());
        dispatch_issue(&action(GatewayIdentityEpoch::first()), &ca, &observation, &clock, &current)
            .await
            .expect("a current-epoch issue succeeds");
        assert!(current.failures.lock().is_empty(), "a success records no failure");

        let stale = EpochCheckingTarget::desiring(second_epoch());
        dispatch_issue(&action(GatewayIdentityEpoch::first()), &ca, &observation, &clock, &stale)
            .await
            .expect("a stale epoch is a success");
        assert!(stale.failures.lock().is_empty(), "a stale success records no failure");
    }

    /// S-PIG-65 (shim half): the two gateway error variants carry fixed outer
    /// text and keep their typed source; no SVID or key bytes render.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn gateway_shim_errors_have_fixed_display_and_keep_their_typed_source() {
        use std::error::Error as _;

        let issue = ShimError::GatewaySvidIssue(crate::ca_issuance::CaIssuanceError::Audit {
            source: ObservationStoreError::Io(std::io::Error::other("injected")),
        });
        assert_eq!(issue.to_string(), "gateway SVID issuance or audit failed");
        assert!(issue.source().is_some(), "the typed issuance cause is retained for matching");

        let disabled = ShimError::from(GatewayIdentityDisabled);
        assert_eq!(
            disabled.to_string(),
            "gateway identity action unavailable: public ingress gateway is disabled",
        );
        assert!(matches!(disabled, ShimError::GatewayIdentityDisabled(GatewayIdentityDisabled)));
    }

    /// S-PIG-65 (shim half): the disabled action composition refuses to hand
    /// out a target, so a dispatch under it can only surface the typed refusal;
    /// an enabled composition hands out its own target.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn disabled_composition_refuses_a_target_and_enabled_hands_out_its_own() {
        let disabled = GatewayIdentityActionComposition::disabled();
        assert!(matches!(disabled.target(), Err(GatewayIdentityDisabled)));

        let enabled = GatewayIdentityActionComposition::enabled(Arc::new(
            EpochCheckingTarget::desiring(GatewayIdentityEpoch::first()),
        ));
        assert!(enabled.target().is_ok());
    }
}

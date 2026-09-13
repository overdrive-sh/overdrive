//! Dedicated gateway-SVID action executors.
//!
//! SCAFFOLD: true. Exact audit-before-hold and epoch-bearing caller surfaces
//! compile now; DELIVER replaces the marked issue effect.

#![expect(clippy::todo, reason = "public-ingress gateway-SVID RED scaffold")]
#![expect(clippy::unused_async, reason = "exact gateway issue executor is async")]
#![allow(clippy::unnecessary_wraps, reason = "exact DESIGN-pinned typed executor signature")]

use overdrive_core::reconcilers::Action;
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_gateway::ports::{GatewayIdentityActionTarget, GatewayIdentityMutationOutcome};

use super::ShimError;

pub(crate) async fn dispatch_issue(
    _action: &Action,
    _ca: &dyn Ca,
    _observation: &dyn ObservationStore,
    _clock: &dyn Clock,
    _target: &dyn GatewayIdentityActionTarget,
) -> Result<GatewayIdentityMutationOutcome, ShimError> {
    todo!("SCAFFOLD: gateway SVID issue-and-audit then epoch-checked hold")
}

pub(crate) fn dispatch_drop(
    action: &Action,
    target: &dyn GatewayIdentityActionTarget,
) -> Result<GatewayIdentityMutationOutcome, ShimError> {
    let Action::DropGatewaySvid { epoch, .. } = action else {
        panic!("gateway_svid::dispatch_drop requires DropGatewaySvid")
    };
    Ok(target.drop_after_drain(*epoch))
}

#[cfg(test)]
mod acceptance {
    #![allow(
        clippy::doc_markdown,
        clippy::expect_used,
        reason = "acceptance fixture preconditions and Contract Shape metadata"
    )]

    use std::sync::Mutex;

    use overdrive_core::gateway_identity::GatewayIdentityEpoch;
    use overdrive_core::id::{CorrelationKey, NodeId, SpiffeId};
    use overdrive_core::traits::ca::SvidMaterial;
    use overdrive_core::traits::observation_store::ObservationStoreError;
    use overdrive_gateway::ports::GatewayIdentityMutationOutcome;
    use overdrive_sim::adapters::ca::SimCa;
    use overdrive_sim::adapters::clock::SimClock;
    use overdrive_sim::adapters::entropy::SimEntropy;
    use overdrive_sim::adapters::observation_store::SimObservationStore;

    use super::*;

    struct CapturingTarget {
        held: Mutex<Vec<(GatewayIdentityEpoch, SvidMaterial)>>,
    }

    impl GatewayIdentityActionTarget for CapturingTarget {
        fn hold_audited(
            &self,
            epoch: GatewayIdentityEpoch,
            material: SvidMaterial,
        ) -> GatewayIdentityMutationOutcome {
            self.held.lock().expect("held lock").push((epoch, material));
            GatewayIdentityMutationOutcome::Applied
        }

        fn drop_after_drain(&self, _epoch: GatewayIdentityEpoch) -> GatewayIdentityMutationOutcome {
            self.held.lock().expect("held lock").clear();
            GatewayIdentityMutationOutcome::Applied
        }
    }

    fn action() -> Action {
        Action::IssueGatewaySvid {
            epoch: GatewayIdentityEpoch::first(),
            spiffe_id: SpiffeId::new("spiffe://overdrive.local/gateway/node-a")
                .expect("gateway SPIFFE ID"),
            node_id: NodeId::new("node-a").expect("node ID"),
            correlation: CorrelationKey::new("gateway-svid/node-a:issue").expect("correlation"),
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER gateway SVID issue-and-audit executor"]
    async fn successful_issue_has_one_matching_audit_row_before_the_slot_is_observable() {
        let ca = SimCa::new(std::sync::Arc::new(SimEntropy::new(0x51D)));
        let observation =
            SimObservationStore::single_peer(NodeId::new("node-a").expect("node ID"), 0x51D);
        let target = CapturingTarget { held: Mutex::new(Vec::new()) };
        assert_eq!(
            dispatch_issue(&action(), &ca, &observation, &SimClock::new(), &target)
                .await
                .expect("issue/audit/hold"),
            GatewayIdentityMutationOutcome::Applied,
        );
        let rows = observation.issued_certificate_rows().await.expect("audit rows");
        let held = target.held.lock().expect("held lock");
        assert_eq!(rows.len(), 1);
        assert_eq!(held.len(), 1);
        assert_eq!(rows[0].spiffe_id, *held[0].1.spiffe_id());
        assert_eq!(rows[0].serial, *held[0].1.serial());
        assert_eq!(rows[0].not_after, held[0].1.not_after());
        drop(held);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER gateway SVID issue-and-audit executor"]
    async fn audit_failure_refuses_the_gateway_identity_slot_mutation() {
        let ca = SimCa::new(std::sync::Arc::new(SimEntropy::new(0xBAD)));
        let observation =
            SimObservationStore::single_peer(NodeId::new("node-a").expect("node ID"), 0xBAD);
        observation.inject_write_failure(ObservationStoreError::Io(std::io::Error::other(
            "injected gateway audit failure",
        )));
        let target = CapturingTarget { held: Mutex::new(Vec::new()) };
        assert!(matches!(
            dispatch_issue(&action(), &ca, &observation, &SimClock::new(), &target).await,
            Err(ShimError::GatewaySvidIssue(_))
        ));
        assert!(target.held.lock().expect("held lock").is_empty());
        assert!(
            observation.issued_certificate_rows().await.expect("audit rows").is_empty(),
            "failed audit does not leave a success occurrence",
        );
    }
}

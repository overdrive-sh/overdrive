//! The convergence property for `gateway-svid-lifecycle` (US-05 S1, S2, S3, S5).
//!
//! The reconciler is registered only when the gateway is enabled, so the node
//! is composed the way `run_server` composes an enabled gateway ([`compose`]):
//! the one `GatewayIdentitySlot`, the production `UnboundGatewayBuilder` over
//! the node's intent store and observation store with a test-local custody KEK
//! (`SimKek`), a `SimServiceVipView` and a throwaway manual key pair on disk,
//! and the slot's identity read and action target in `AppState::gateway`. No
//! route is added and no gateway task is started: the reconciler reads only the
//! identity view, and its executor writes only the slot.
//!
//! From an arbitrary starting `(gateway enabled or disabled, held SVID, audit
//! rows, View)` and up to one dropped issuance (the audit write that precedes
//! the hold failing once), an enabled gateway holds a usable SVID of the desired
//! epoch and subject (not inside the near-expiry window), a disabled gateway
//! holds none, within [`ROUNDS`] rounds; then further rounds change nothing. The
//! View is the retry memory a previous process persisted (or the forgotten
//! View): pacing only, it must never keep the SVID from being issued once its
//! backoff has elapsed.
//!
//! The View is seeded through the production view store before the runtime
//! registers the reconciler, as a restart finds it; the runtime has no
//! gateway-view seed seam and none is added.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::SpiffeId;
use overdrive_core::UnixInstant;
use overdrive_core::ca::kek::KEK_LEN;
use overdrive_core::ca::root_key_envelope::KekId;
use overdrive_core::gateway_identity::{GatewayIdentityEpoch, GatewayIdentityView};
use overdrive_core::id::NodeId;
use overdrive_core::public_ingress::PublicCertifiedKeyId;
use overdrive_core::reconcilers::{Reconciler, ReconcilerName};
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{ObservationStore, ObservationStoreError};
use overdrive_dataplane::mtls::gateway_identity::GatewayIdentitySlot;
use overdrive_gateway::application::UnboundGatewayBuilder;
use overdrive_gateway::ports::{GatewayIdentityActionTarget, GatewayIdentityLifecycleTarget};
use overdrive_gateway::{GatewayConfig, ManualCertifiedKeyConfig};
use overdrive_host::ca::PublicCertifiedKeyAeadCodec;
use overdrive_reconcilers::GatewaySvidLifecycle;
use overdrive_reconcilers::GatewaySvidLifecycleView;
use overdrive_reconcilers::svid_lifecycle::{IssueRetry, NEAR_EXPIRY_THRESHOLD_SECS};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::kek::SimKek;
use overdrive_sim::adapters::read_ports::SimServiceVipView;
use overdrive_testing::placement;
use proptest::prelude::*;

use super::{Composed, ROUNDS, STABLE_ROUNDS, name_of, node, property_config, runtime, target_of};
use crate::gateway_composition::{GatewayAppStateComposition, GatewayIdentityActionComposition};
use crate::view_store::ViewStoreExt;
use crate::view_store::redb::RedbViewStore;

placement!(host_pure);

/// The node id `Composed` gives `AppState`; the gateway reconciler's one
/// target is `node/<this>`.
const NODE: &str = "writer-1";

/// The retry memory a previous process persisted for the gateway reconciler.
#[derive(Debug, Clone, Copy)]
pub struct PersistedRetry {
    pub attempts: u32,
    /// Failure seen at process start (otherwise long ago).
    pub seen_recently: bool,
}

/// The registered reconciler, as `run_server` registers it for an enabled gateway.
pub fn reconciler() -> overdrive_reconcilers::AnyReconciler {
    crate::gateway_identity_lifecycle::gateway_svid_lifecycle_registration(&node(NODE)).0
}

/// Persist the retry memory into the view store at `data_dir` before the
/// runtime opens it.
pub async fn persist_retry(data_dir: &Path, retry: PersistedRetry, clock: &SimClock) {
    let seen = if retry.seen_recently {
        UnixInstant::from_clock(clock)
    } else {
        UnixInstant::from_unix_duration(Duration::ZERO)
    };
    let view = GatewaySvidLifecycleView {
        retry: Some(IssueRetry { attempts: retry.attempts, last_failure_seen_at: seen }),
    };
    let store = RedbViewStore::open(data_dir).expect("open view store");
    store
        .write_through(GatewaySvidLifecycle::NAME, &target_of("node", NODE), &view)
        .await
        .expect("persist gateway view");
}

/// A throwaway manual certified key pair for the gateway's custody to install.
fn write_manual_key(dir: &Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::create_dir_all(dir).expect("gateway key directory");
    let chain = dir.join("api-origin-chain.pem");
    let key_path = dir.join("api-origin-key.pem");
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("test key");
    let mut params = rcgen::CertificateParams::new(vec!["api.example.com".to_owned()])
        .expect("certificate params");
    params.is_ca = rcgen::IsCa::NoCa;
    params.not_before = rcgen::date_time_ymd(2020, 1, 1);
    params.not_after = rcgen::date_time_ymd(2100, 1, 1);
    let cert = params.self_signed(&key).expect("test certificate");
    std::fs::write(&chain, cert.pem()).expect("write certificate");
    std::fs::write(&key_path, key.serialize_pem()).expect("write private key");
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
        .expect("private-key mode");
    (chain, key_path)
}

/// The enabled gateway composition over `Sim*` ports, built from the same
/// constructors `run_server` uses (boot step G1): the identity slot, its action
/// target, and the route and status handles of the unbound builder.
pub async fn compose(
    dir: &Path,
    store: &Arc<dyn IntentStore>,
    obs: &Arc<dyn ObservationStore>,
    clock: &Arc<dyn Clock>,
    ca: &dyn Ca,
    node_id: &NodeId,
) -> (GatewayAppStateComposition, Arc<GatewayIdentitySlot>) {
    let (chain, key) = write_manual_key(&dir.join("gateway"));
    let config = GatewayConfig::new(
        Ipv4Addr::new(127, 151, 0, 1),
        ManualCertifiedKeyConfig::new(
            PublicCertifiedKeyId::new("api-origin").expect("key id"),
            chain,
            key,
        ),
    );
    let bind = config.bind();
    let kek = Arc::new(SimKek::new().with(
        &KekId::new("overdrive-public-certified-key").expect("custody KEK id"),
        [0x5a; KEK_LEN],
    ));
    let slot = Arc::new(GatewayIdentitySlot::new(
        node_id.clone(),
        ca.trust_bundle().expect("trust bundle"),
    ));
    let unbound = UnboundGatewayBuilder::new(
        node_id.clone(),
        config,
        Arc::clone(store),
        Arc::clone(obs),
        Arc::new(SimServiceVipView::new(BTreeMap::new())),
        Arc::new(PublicCertifiedKeyAeadCodec::new(kek)),
        Arc::clone(clock),
    )
    .await
    .expect("gateway builder over sim ports");
    let composition = GatewayAppStateComposition::enabled(
        Arc::clone(&slot) as Arc<dyn GatewayIdentityView>,
        GatewayIdentityActionComposition::enabled(Arc::new(slot.action_target())),
        unbound.route_commands(),
        unbound.status_reads(),
        bind,
    );
    (composition, slot)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    /// Never issued.
    Never,
    /// Issued once (an audit row exists), the hold lost: the restart marker.
    IssuedThenLost,
    /// Issued and held, well inside its validity.
    Fresh,
    /// Held, inside the near-expiry window.
    NearExpiry,
    /// Held, past its expiry.
    Expired,
}

#[derive(Debug, Clone)]
struct World {
    /// The gateway is enabled (otherwise disabled after the hold was set up,
    /// which clears the slot synchronously).
    enabled: bool,
    hold: Hold,
    /// Retry memory the previous process persisted: attempts, seen at start.
    retry: Option<(u32, bool)>,
    /// The next issuance's audit write fails once.
    dropped_issue: bool,
}

fn world() -> impl Strategy<Value = World> {
    (
        prop_oneof![3 => Just(true), 1 => Just(false)],
        prop_oneof![
            Just(Hold::Never),
            Just(Hold::IssuedThenLost),
            Just(Hold::Fresh),
            Just(Hold::NearExpiry),
            Just(Hold::Expired),
        ],
        prop::option::of((0u32..=3, any::<bool>())),
        any::<bool>(),
    )
        .prop_map(|(enabled, hold, retry, dropped_issue)| World {
            enabled,
            hold,
            retry,
            dropped_issue,
        })
}

struct Fixture {
    node: Composed,
    slot: Arc<GatewayIdentitySlot>,
    name: ReconcilerName,
}

async fn fixture(world: &World) -> Fixture {
    let persisted =
        world.retry.map(|(attempts, seen_recently)| PersistedRetry { attempts, seen_recently });
    let node = Composed::new_with_gateway(vec![reconciler()], persisted).await;
    let slot = Arc::clone(node.gateway_slot.as_ref().expect("gateway is composed"));
    let subject = SpiffeId::for_gateway(&node.state.node_id);
    if world.hold != Hold::Never {
        let material = crate::ca_issuance::issue_and_audit(
            node.state.ca.as_ref(),
            node.obs.as_ref(),
            node.clock.as_ref(),
            &node.state.node_id,
            &subject,
        )
        .await
        .expect("pre-issue");
        if world.hold != Hold::IssuedThenLost {
            let not_after = material.not_after();
            slot.action_target().hold_audited(GatewayIdentityEpoch::first(), material);
            let remaining = not_after.as_unix_duration().saturating_sub(node.clock.unix_now());
            match world.hold {
                Hold::NearExpiry => node.clock.tick(
                    remaining.saturating_sub(Duration::from_secs(NEAR_EXPIRY_THRESHOLD_SECS / 2)),
                ),
                Hold::Expired => node.clock.tick(remaining + Duration::from_secs(10)),
                Hold::Never | Hold::IssuedThenLost | Hold::Fresh => {}
            }
        }
    }
    if !world.enabled {
        slot.lifecycle_target().begin_disable();
    }
    if world.dropped_issue {
        node.obs.inject_write_failure(ObservationStoreError::Unreachable {
            peer: "dropped by the property's fault injection".to_owned(),
        });
    }
    Fixture { node, slot, name: name_of("gateway-svid-lifecycle") }
}

impl Fixture {
    /// `actual == desired`: an enabled gateway holds a usable SVID of the
    /// desired epoch and subject, a disabled gateway holds none.
    fn converged(&self) -> bool {
        let desired = self.slot.desired();
        let held = self.slot.current();
        let now = UnixInstant::from_clock(&*self.node.clock);
        match (desired.spiffe_id, held) {
            (None, held) => held.is_none(),
            (Some(subject), Some(facts)) => {
                facts.epoch == desired.epoch
                    && facts.spiffe_id == subject
                    && facts.not_after > now + Duration::from_secs(NEAR_EXPIRY_THRESHOLD_SECS)
            }
            (Some(_), None) => false,
        }
    }

    /// The held facts and the audit trail: what a further action would change.
    async fn observable(&self) -> String {
        let audited = self.node.obs.issued_certificate_rows().await.expect("audit rows").len();
        format!("held {:?}, audit rows {audited}", self.slot.current())
    }
}

/// Outcome anchor: OUT-RCG-GATEWAY-SVID-CONVERGES-FROM-ARBITRARY-STATE
/// US-05 S1, S2, S3, S5 for `gateway-svid-lifecycle`. Pending until the resync
/// pass exists: the reconciler is host-backed (the slot is in-process, no row
/// change can wake it), so an empty slot after a restart, a lost hold or a
/// dropped issuance, and a held SVID inside the near-expiry window, are only
/// re-evaluated by the pass.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn gateway_svid_lifecycle_converges_the_gateway_identity_from_any_state_then_stays_converged() {
    let runtime = runtime();
    proptest!(property_config(48, 351_009), |(world in world())| {
        runtime.block_on(async {
            let fixture = fixture(&world).await;

            let mut round = 0usize;
            while !fixture.converged() && round < ROUNDS {
                round += 1;
                // A dropped issuance returns the gateway issuance error: expected under the fault.
                fixture.node.round(&fixture.name, world.dropped_issue).await;
            }
            assert!(
                fixture.converged(),
                "gateway-svid-lifecycle stalled with actual != desired after {ROUNDS} rounds \
                 (View/actual skew or a dropped issuance): world {world:?}, {}",
                fixture.observable().await
            );

            let settled = fixture.observable().await;
            for _ in 0..STABLE_ROUNDS {
                fixture.node.round(&fixture.name, false).await;
                prop_assert_eq!(
                    &fixture.observable().await,
                    &settled,
                    "a converged gateway-svid-lifecycle emitted an action that changed the identity"
                );
            }
            Ok(())
        })?;
    });
}

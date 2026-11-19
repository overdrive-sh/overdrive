//! The convergence property for `service-map-hydrator` (US-05 S1, S2, S3, S5).
//!
//! From an arbitrary View (including the default View and a View that
//! remembers an attempt that never landed), with up to two dropped dataplane
//! updates, the hydrator reaches `actual == desired` within [`ROUNDS`] rounds,
//! and further rounds change nothing. `actual == desired` is stated on the
//! resource the hydrator manages, the dataplane map, read through
//! `ProgrammedServiceView`: the programmed fingerprint equals the desired
//! set's and the dataplane carries the desired backends. A View field that
//! overrules `actual` (a record-on-emit latch) makes a case stall.
//!
//! Every case starts from a freshly composed node, so the dataplane under test
//! (a fresh `SimDataplane`, which reads `None` for every frontend) carries
//! nothing programmed: the state a serve restart or a reboot leaves, because
//! the maps do not persist. No world loses the maps while the process runs:
//! Overdrive is the only writer of its maps, so no production path does.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;
use std::str::FromStr;
use std::time::Duration;

use overdrive_core::SpiffeId;
use overdrive_core::UnixInstant;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::dataplane::fingerprint::{BackendSetFingerprint, fingerprint};
use overdrive_core::id::{ServiceId, ServiceVip};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::ProgrammedServiceView;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::{Backend, DataplaneError};
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationStore, ObservationWrite, ServiceBackendRow,
};
use overdrive_reconcilers::service_map_hydrator::{RetryMemory, ServiceMapHydratorView};
use overdrive_reconcilers::{AnyReconciler, AnyState};
use overdrive_testing::placement;
use proptest::prelude::*;

use super::{
    Composed, ROUND_ADVANCE, ROUNDS, STABLE_ROUNDS, name_of, node, property_config, runtime,
    target_of,
};
use crate::reconciler_runtime::{hydrate_desired_for_test, run_convergence_tick};

placement!(host_pure);

const MAX_DROPPED_UPDATES: usize = 2;
const SERVICE: &str = "payments";

/// One arbitrary starting world for the hydrator's one service.
#[derive(Debug, Clone)]
struct World {
    /// Remote backend octets `10.<b>.<c>.<d>`: non-mesh, non-host addresses.
    backends: Vec<(u8, u8, u8)>,
    /// The hydrator's View for the service, if any (`None` is `View::default()`).
    retry: Option<RetrySeed>,
    /// Dataplane updates dropped before one succeeds.
    dropped_updates: usize,
}

#[derive(Debug, Clone, Copy)]
struct RetrySeed {
    attempts: u32,
    /// The last attempt was seen just now (inside the backoff) rather than long ago.
    seen_recently: bool,
    attempted: AttemptedFingerprint,
}

#[derive(Debug, Clone, Copy)]
enum AttemptedFingerprint {
    Nothing,
    /// The View remembers attempting exactly the desired set, whatever the world shows.
    DesiredSet,
    OtherSet,
}

fn world() -> impl Strategy<Value = World> {
    (
        prop::collection::vec((1u8..=95, 0u8..=255, 1u8..=254), 1..=3),
        prop::option::of((
            0u32..=4,
            any::<bool>(),
            prop_oneof![
                Just(AttemptedFingerprint::Nothing),
                Just(AttemptedFingerprint::DesiredSet),
                Just(AttemptedFingerprint::OtherSet),
            ],
        )),
        0usize..=MAX_DROPPED_UPDATES,
    )
        .prop_map(|(mut backends, retry, dropped_updates)| {
            backends.sort_unstable();
            backends.dedup();
            World {
                backends,
                retry: retry.map(|(attempts, seen_recently, attempted)| RetrySeed {
                    attempts,
                    seen_recently,
                    attempted,
                }),
                dropped_updates,
            }
        })
}

struct Fixture {
    node: Composed,
    sid: ServiceId,
    vip: ServiceVip,
    target: TargetResource,
    name: ReconcilerName,
}

/// The frontend `put_service` declares: its one TCP listener on port 8080.
fn frontend_of(vip: ServiceVip) -> ServiceFrontend {
    ServiceFrontend::new(vip, NonZeroU16::new(8080).expect("non-zero port"), Proto::Tcp)
        .expect("IPv4 service VIP")
}

fn backends_of(world: &World) -> Vec<Backend> {
    world
        .backends
        .iter()
        .enumerate()
        .map(|(i, (b, c, d))| Backend {
            alloc: SpiffeId::from_str(&format!(
                "spiffe://overdrive.local/workload/{SERVICE}/alloc/a{i}"
            ))
            .expect("spiffe"),
            addr: SocketAddr::new(Ipv4Addr::new(10, *b, *c, *d).into(), 8080),
            weight: 100,
            healthy: true,
        })
        .collect()
}

#[cfg(test)]
async fn fixture(world: &World) -> Fixture {
    let node_ = Composed::new(vec![crate::service_map_hydrator()]).await;
    let (sid, vip) = node_.put_service(SERVICE).await;
    let backends = backends_of(world);
    node_
        .obs
        .write(ObservationWrite::ServiceBackend(ServiceBackendRow {
            service_id: sid,
            vip: vip.try_as_ipv4().expect("allocator issues IPv4"),
            backends: backends.clone(),
            updated_at: LogicalTimestamp { counter: 1, writer: node("writer-1") },
        }))
        .await
        .expect("write service_backends");

    let desired_fp = fingerprint(&vip, &backends);
    let other_fp = fingerprint(&vip, &backends[..backends.len().saturating_sub(1)]);
    let other_fp = if other_fp == desired_fp { fingerprint(&vip, &[]) } else { other_fp };
    let target = target_of("service", &sid.to_string());
    if let Some(seed) = world.retry {
        let now = UnixInstant::from_clock(&*node_.clock);
        let mut retries = BTreeMap::new();
        retries.insert(
            sid,
            RetryMemory {
                attempts: seed.attempts,
                last_failure_seen_at: if seed.seen_recently {
                    now
                } else {
                    UnixInstant::from_unix_duration(Duration::ZERO)
                },
                last_attempted_fingerprint: match seed.attempted {
                    AttemptedFingerprint::Nothing => None,
                    AttemptedFingerprint::DesiredSet => Some(desired_fp),
                    AttemptedFingerprint::OtherSet => Some(other_fp),
                },
            },
        );
        node_
            .state
            .runtime
            .seed_service_map_hydrator_view_for_test(&target, ServiceMapHydratorView { retries });
    }

    for _ in 0..world.dropped_updates {
        node_.dataplane.script_update_service_failure(DataplaneError::LoadFailed(
            "dropped by the property's fault injection".to_owned(),
        ));
    }

    Fixture { node: node_, sid, vip, target, name: name_of("service-map-hydrator") }
}

impl Fixture {
    /// The programmed backends the dataplane carries for the service's VIP.
    fn programmed_backends(&self) -> usize {
        self.node
            .dataplane
            .service_backends(self.vip.try_as_ipv4().expect("v4"))
            .map_or(0, |backends| backends.len())
    }

    /// The programmed fingerprint of the service's frontend, read through
    /// `ProgrammedServiceView` from the dataplane the hydrator drives.
    async fn programmed(&self) -> Option<BackendSetFingerprint> {
        self.node
            .dataplane
            .programmed_fingerprint(&frontend_of(self.vip))
            .await
            .expect("read the programmed service map")
    }

    /// The fingerprint of the desired backend set (read through the
    /// reconciler's own hydration).
    ///
    /// Oracle assumption: this fingerprints the hydrated desired set as is, which
    /// equals the set the hydrator programs only because the generator draws
    /// remote backends alone (`10.1..=95.x.y`, non-mesh, non-host). A world that
    /// drew a backend the hydrator's gate drops would need this derived from the
    /// post-gate survivors instead.
    async fn wanted(&self) -> Option<BackendSetFingerprint> {
        let reconciler: AnyReconciler = crate::service_map_hydrator();
        let desired = hydrate_desired_for_test(&reconciler, &self.target, &self.node.state)
            .await
            .expect("hydrate desired");
        let AnyState::ServiceMapHydrator(desired) = desired else {
            panic!("hydrator state expected");
        };
        desired.desired.get(&self.sid).map(|wanted| fingerprint(&wanted.vip, &wanted.backends))
    }

    /// `actual == desired`, stated on the resource the hydrator manages: the
    /// programmed service map carries the fingerprint of the desired set AND
    /// the dataplane carries the desired backends.
    async fn converged(&self, want: usize) -> bool {
        let Some(wanted) = self.wanted().await else { return false };
        self.programmed().await == Some(wanted) && self.programmed_backends() == want
    }

    /// One evaluation of the hydrator's target, without the resync pass: the
    /// tick the pass would admit. Isolates the hydrator's own read of `actual`.
    async fn evaluate_directly(&self, tick: &mut u64) {
        self.node.clock.tick(ROUND_ADVANCE);
        let now = Clock::now(&*self.node.clock);
        let outcome = run_convergence_tick(
            &self.node.state,
            &self.name,
            &self.target,
            now,
            *tick,
            now + Duration::from_millis(100),
        )
        .await;
        *tick += 1;
        outcome.expect("no fault is armed: the tick must not error");
    }

    /// Everything a further action could change, as the ports show it: the
    /// dataplane's backends and the programmed fingerprint.
    async fn observable(&self) -> String {
        format!(
            "{:?} | {:?}",
            self.node.dataplane.service_backends(self.vip.try_as_ipv4().expect("v4")),
            self.programmed().await,
        )
    }
}

/// Outcome anchor: OUT-RCG-HYDRATOR-CONVERGES-FROM-ARBITRARY-STATE
/// US-05 S1, S2, S3, S5 for `service-map-hydrator`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn the_hydrator_converges_from_any_state_despite_dropped_updates_then_stays_converged() {
    let runtime = runtime();
    proptest!(property_config(48, 351_004), |(world in world())| {
        runtime.block_on(async {
            let fixture = fixture(&world).await;

            // Progress: converge within ROUNDS rounds. The effect must be in
            // place on the dataplane the hydrator drives, not only on the row.
            let want = backends_of(&world).len();
            let mut round = 0usize;
            while !fixture.converged(want).await && round < ROUNDS {
                round += 1;
                // A failed update is logged, not a tick error: no error is expected.
                fixture.node.round(&fixture.name, false).await;
            }
            assert!(
                fixture.converged(want).await,
                "service-map-hydrator stalled with actual != desired after {ROUNDS} rounds \
                 (View/actual skew or dropped updates): world {world:?}, observable {}",
                fixture.observable().await
            );

            // Stability: no further action changes anything.
            let settled = fixture.observable().await;
            for _ in 0..STABLE_ROUNDS {
                fixture.node.round(&fixture.name, false).await;
                prop_assert_eq!(
                    &fixture.observable().await,
                    &settled,
                    "a converged hydrator emitted an action that changed observable state"
                );
            }
            Ok(())
        })?;
    });
}

/// A freshly composed node's dataplane carries nothing programmed. The
/// hydrator's own tick (no resync pass) programs it: its read of `actual` is
/// the programmed service map, so an empty map reads as not in place.
async fn programs_an_empty_dataplane(world: &World) {
    let fixture = fixture(world).await;
    let want = backends_of(world).len();
    assert_eq!(
        fixture.programmed().await,
        None,
        "precondition: the dataplane's programmed map is empty"
    );
    let mut tick = 0u64;
    for _ in 0..ROUNDS {
        if fixture.converged(want).await {
            break;
        }
        fixture.evaluate_directly(&mut tick).await;
    }
    assert!(
        fixture.converged(want).await,
        "service-map-hydrator did not program an empty dataplane: \
         programmed fingerprint {:?}, world {world:?}, observable {}",
        fixture.programmed().await,
        fixture.observable().await
    );
}

fn backend_octets() -> impl Strategy<Value = Vec<(u8, u8, u8)>> {
    prop::collection::vec((1u8..=95, 0u8..=255, 1u8..=254), 1..=3).prop_map(|mut backends| {
        backends.sort_unstable();
        backends.dedup();
        backends
    })
}

/// Outcome anchor: OUT-RCG-HYDRATOR-READS-PROGRAMMED-MAP (D-20, ADR-0324).
/// A node whose dataplane map is empty is programmed by the hydrator's own
/// tick: `hydrate_actual` reads `ProgrammedServiceView` over the instance the
/// action shim writes through.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn an_empty_dataplane_is_programmed_by_the_hydrators_own_tick() {
    let runtime = runtime();
    proptest!(property_config(16, 351_020), |(backends in backend_octets())| {
        runtime.block_on(programs_an_empty_dataplane(
            &World { backends, retry: None, dropped_updates: 0 },
        ));
    });
}

//! The convergence property for `service-lifecycle` (US-05 S1, S2, S3, S5),
//! scoped as feature delta D-6 states.
//!
//! The probe-driven stop obligation is excluded from the drop/skew progress
//! oracle: with a dropped stop, or a View marker equal to the latest crossed
//! row's `run_seq`, `service-lifecycle` emits nothing until a newer probe row
//! arrives (ADR-0191 decisions 5 and 6, bounded by the `specs/quint/probe-count`
//! checks). It is excluded by construction: the generated Service declares no
//! probes and the world holds no probe row, so no stop obligation exists to
//! score. Every other obligation stays under the oracle: backend membership
//! (the Service's `service_backends` row carries exactly the Running
//! allocations' addresses) and, through the Stable commit the opt-out Service
//! takes, the readiness gate. The View generator draws markers only from rows
//! present: `observed` and `terminal_announced` name only allocations that have
//! a row in the world, and no marker carries a `run_seq`.

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;

use overdrive_core::SpiffeId;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, ServiceId, ServiceVip};
use overdrive_core::reconcilers::ReconcilerName;
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    AllocState, LogicalTimestamp, ObservationStore, ObservationStoreError, ObservationWrite,
    ServiceBackendRow,
};
use overdrive_reconcilers::service_lifecycle::ServiceLifecycleView;
use overdrive_testing::placement;
use proptest::prelude::*;

use super::{Composed, ROUNDS, STABLE_ROUNDS, name_of, node, property_config, runtime, target_of};

placement!(host_pure);

const SERVICE: &str = "web";
const ALLOCS: [&str; 3] = ["web-0", "web-1", "web-2"];

fn addr_of(index: usize) -> Ipv4Addr {
    Ipv4Addr::new(100, 95, 0, u8::try_from(2 + index).expect("small index"))
}

#[derive(Debug, Clone, Copy)]
struct AllocWorld {
    /// Running (with its workload address) or Terminated.
    running: bool,
    /// The View already remembers this allocation (`observed`).
    observed: bool,
    /// The View already announced this allocation terminal.
    announced: bool,
}

#[derive(Debug, Clone, Copy)]
enum BackendRow {
    Absent,
    /// A row over an address no allocation has.
    Stale,
    /// A row carrying every allocation's address, Terminated ones included.
    OverBroad,
}

#[derive(Debug, Clone)]
struct World {
    allocs: Vec<AllocWorld>,
    backend_row: BackendRow,
    /// `None` is `View::default()`; `Some` seeds the markers above.
    seed_view: bool,
    /// The next observation write (a Stable commit or a backend row) is dropped.
    dropped_write: bool,
}

fn world() -> impl Strategy<Value = World> {
    (
        prop::collection::vec(
            (any::<bool>(), any::<bool>(), any::<bool>()).prop_map(
                |(running, observed, announced)| AllocWorld { running, observed, announced },
            ),
            1..=ALLOCS.len(),
        ),
        prop_oneof![Just(BackendRow::Absent), Just(BackendRow::Stale), Just(BackendRow::OverBroad)],
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(allocs, backend_row, seed_view, dropped_write)| World {
            allocs,
            backend_row,
            seed_view,
            dropped_write,
        })
}

struct Fixture {
    node: Composed,
    name: ReconcilerName,
    sid: ServiceId,
}

fn backend(address: Ipv4Addr) -> Backend {
    Backend {
        workload_identity: SpiffeId::from_str(&format!(
            "spiffe://overdrive.local/workload/{SERVICE}"
        ))
        .expect("spiffe"),
        addr: SocketAddr::new(address.into(), 8080),
        weight: 1,
        healthy: true,
    }
}

#[cfg(test)]
async fn fixture(world: &World) -> Fixture {
    let node_ = Composed::new(vec![crate::service_lifecycle()]).await;
    let (sid, vip): (ServiceId, ServiceVip) = node_.put_service(SERVICE).await;
    let mut observed = BTreeSet::new();
    let mut announced = BTreeSet::new();
    for (index, (alloc_world, alloc)) in world.allocs.iter().zip(ALLOCS).enumerate() {
        let (state, addr) = if alloc_world.running {
            (AllocState::Running, Some(addr_of(index)))
        } else {
            (AllocState::Terminated, None)
        };
        node_.write_alloc_row(SERVICE, alloc, state, WorkloadKind::Service, addr).await;
        let id = AllocationId::new(alloc).expect("alloc id");
        if alloc_world.observed {
            observed.insert(id.clone());
        }
        if alloc_world.announced {
            announced.insert(id);
        }
    }
    let rows = match world.backend_row {
        BackendRow::Absent => None,
        BackendRow::Stale => Some(vec![backend(Ipv4Addr::new(100, 95, 0, 200))]),
        BackendRow::OverBroad => {
            Some((0..world.allocs.len()).map(|index| backend(addr_of(index))).collect::<Vec<_>>())
        }
    };
    if let Some(backends) = rows {
        node_
            .obs
            .write(ObservationWrite::ServiceBackend(ServiceBackendRow {
                service_id: sid,
                vip: vip.try_as_ipv4().expect("allocator issues IPv4"),
                backends,
                updated_at: LogicalTimestamp { counter: 1, writer: node("writer-1") },
            }))
            .await
            .expect("write service_backends");
    }
    if world.seed_view {
        node_.state.runtime.seed_service_lifecycle_view_for_test(
            &target_of("workload", SERVICE),
            ServiceLifecycleView { observed, terminal_announced: announced, ..Default::default() },
        );
    }
    if world.dropped_write {
        node_.obs.inject_write_failure(ObservationStoreError::Unreachable {
            peer: "dropped by the property's fault injection".to_owned(),
        });
    }
    Fixture { node: node_, name: name_of("service-lifecycle"), sid }
}

impl Fixture {
    /// The published backend set; `None` while the Service has no row at all.
    async fn published(&self) -> Option<BTreeSet<SocketAddr>> {
        self.node
            .obs
            .service_backends_rows(&self.sid)
            .await
            .expect("backend rows")
            .into_iter()
            .next()
            .map(|row| row.backends.into_iter().map(|b| b.addr).collect())
    }

    /// `actual == desired`: a row is published and its backend set is the
    /// Running set (an empty set when nothing runs: the Service's row says so).
    async fn converged(&self, world: &World) -> bool {
        let want: BTreeSet<SocketAddr> = world
            .allocs
            .iter()
            .enumerate()
            .filter(|(_, alloc)| alloc.running)
            .map(|(index, _)| SocketAddr::new(addr_of(index).into(), 8080))
            .collect();
        self.published().await == Some(want)
    }

    /// What a further action could change, as the ports show it: the published
    /// member set with each member's health (never a row's logical timestamp,
    /// which a rewrite legitimately advances) and the allocation rows.
    async fn observable(&self) -> String {
        let members: Vec<(SocketAddr, bool)> = self
            .node
            .obs
            .service_backends_rows(&self.sid)
            .await
            .expect("rows")
            .into_iter()
            .flat_map(|row| row.backends.into_iter().map(|b| (b.addr, b.healthy)))
            .collect();
        format!("{members:?} | {:?}", self.node.alloc_rows().await)
    }
}

/// Outcome anchor: OUT-RCG-SERVICE-CONVERGES-FROM-ARBITRARY-STATE
/// US-05 S1, S2, S3, S5 for `service-lifecycle`, scoped as D-6 (see the module
/// doc). Pending until the resync pass exists: a published row that disagrees
/// with the Running set and a dropped write are only re-evaluated by the pass.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn service_lifecycle_publishes_the_running_backends_from_any_state_then_stays_converged() {
    let runtime = runtime();
    proptest!(property_config(48, 351_007), |(world in world())| {
        runtime.block_on(async {
            let fixture = fixture(&world).await;

            let mut round = 0usize;
            while !fixture.converged(&world).await && round < ROUNDS {
                round += 1;
                // A dropped write returns the shim's observation-write error: expected under the fault.
                fixture.node.round(&fixture.name, world.dropped_write).await;
            }
            assert!(
                fixture.converged(&world).await,
                "service-lifecycle stalled with actual != desired after {ROUNDS} rounds \
                 (View/actual skew or a dropped write): world {world:?}, observable {}",
                fixture.observable().await
            );

            let settled = fixture.observable().await;
            for _ in 0..STABLE_ROUNDS {
                fixture.node.round(&fixture.name, false).await;
                prop_assert_eq!(
                    &fixture.observable().await,
                    &settled,
                    "a converged service-lifecycle emitted an action that changed observable state"
                );
            }
            Ok(())
        })?;
    });
}

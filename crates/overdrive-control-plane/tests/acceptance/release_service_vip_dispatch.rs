//! Acceptance — service-vip-allocator step 03-02.
//!
//! S-VIP-06 PARTIAL — action-shim dispatch layer ONLY (reconciler
//! emission lives in 03-01; end-to-end submit → terminal → reallocate
//! lifecycle lives in 03-03).
//!
//! Per ADR-0049 (amended 2026-05-15): when the runtime dispatches a
//! hand-constructed `Action::ReleaseServiceVip { spec_digest,
//! correlation }`, the action shim MUST invoke
//! `PersistentServiceVipAllocator::release(&spec_digest)`. After the
//! dispatch returns, `allocator.get(&spec_digest)` MUST return `None`
//! (the entry is gone from the in-memory memo AND, by virtue of the
//! `release()` contract, from the `IntentStore` `allocator_entries` row).
//!
//! Test shape: construct a real `PersistentServiceVipAllocator` over a
//! tempdir-backed `LocalIntentStore`, pre-populate it with a single
//! `allocate(&digest)`, hand the dispatcher a hand-constructed
//! `Action::ReleaseServiceVip` carrying the same digest, await dispatch,
//! assert `allocator.get(&digest)` returns `None`.
//!
//! PORT-TO-PORT litmus: deleting the new `release_service_vip` dispatch
//! arm's call into `allocator.release(...)` MUST turn this test RED —
//! the allocator's memo would still carry the entry post-dispatch.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use overdrive_control_plane::action_shim::dispatch;
use overdrive_core::UnixInstant;
use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, Resources,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

/// Inert driver — the `ReleaseServiceVip` arm does not touch the driver,
/// so all methods return either `NotFound` or `Ok` and are never
/// expected to fire under this test.
struct InertDriver;

#[async_trait]
impl Driver for InertDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Exec
    }

    async fn start(&self, _spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        // Unused on the ReleaseServiceVip dispatch path.
        Err(DriverError::StartRejected {
            reason: "InertDriver: start() not expected on ReleaseServiceVip dispatch".to_owned(),
            driver: DriverType::Exec,
        })
    }

    async fn stop(&self, _handle: &AllocationHandle) -> Result<(), DriverError> {
        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        Err(DriverError::NotFound { alloc: handle.alloc.clone() })
    }

    async fn resize(
        &self,
        _handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        Ok(())
    }
}

#[tokio::test]
async fn release_action_dispatch_invokes_allocator_release() {
    // ---- Allocator: real PersistentServiceVipAllocator over a fresh
    // tempdir-backed LocalIntentStore. The pre-populate path goes
    // through `allocate(&digest)` which writes the entry to the
    // IntentStore allocator_entries table AND inserts into the memo,
    // mirroring the production lifecycle precisely.
    let tmp = TempDir::new().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let store: Arc<dyn IntentStore> =
        Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        Arc::clone(&store),
    )));

    // ---- Pre-populate: allocate exactly one VIP under a known digest.
    // SHA-256(b"S-VIP-06 03-02 dispatch fixture") — any 32-byte
    // discriminator works; the choice is arbitrary but stable so the
    // PORT-TO-PORT failure mode names the same digest in a regression.
    let digest = ContentHash::of(b"S-VIP-06 03-02 dispatch fixture");
    {
        let mut guard = allocator.lock().await;
        let _vip = guard.allocate(*digest.as_bytes()).await.expect("seed allocation");
        assert!(
            guard.get(digest.as_bytes()).is_some(),
            "pre-condition: allocator carries the seeded digest"
        );
        drop(guard);
    }

    // ---- Construct the Action::ReleaseServiceVip from the same digest.
    // The CorrelationKey shape mirrors the reconciler-emission site in
    // overdrive-core::reconcilers::release_service_vip — derived from
    // (target = "job-lifecycle/<workload>", spec_hash = digest,
    // purpose = "release-service-vip") so an end-to-end test in 03-03
    // sees the same correlation derivation.
    let correlation =
        CorrelationKey::derive("job-lifecycle/test-workload", &digest, "release-service-vip");
    let action = Action::ReleaseServiceVip { spec_digest: digest, correlation };

    // ---- Wire the remaining ports. ObservationStore + Dataplane +
    // broadcast channel are required by `dispatch`'s signature but the
    // ReleaseServiceVip arm does not touch them — InMemory sim shapes
    // are sufficient and add zero state coupling.
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let dataplane: Arc<dyn overdrive_core::traits::dataplane::Dataplane> =
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new());
    let driver: Arc<dyn Driver> = Arc::new(InertDriver);
    let (lifecycle_tx, _lifecycle_rx) = tokio::sync::broadcast::channel(16);
    let writer_node = NodeId::new("writer-1").expect("NodeId");

    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    // ---- Dispatch — the action-shim arm under test.
    let test_broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    dispatch(
        vec![action],
        driver.as_ref(),
        obs.as_ref(),
        dataplane.as_ref(),
        &lifecycle_tx,
        &tick,
        &writer_node,
        Arc::clone(&allocator),
        &test_broker,
        None,
    )
    .await
    .expect("dispatch must succeed");

    // ---- Post-condition: the allocator no longer carries the digest.
    // This is the load-bearing assertion — failure here means the
    // ReleaseServiceVip arm did NOT call `allocator.release(&digest)`,
    // so the memo still contains the entry the test seeded.
    {
        let guard = allocator.lock().await;
        assert!(
            guard.get(digest.as_bytes()).is_none(),
            "allocator.get(&digest) MUST return None after \
             Action::ReleaseServiceVip dispatch (S-VIP-06 partial)"
        );
        drop(guard);
    }
}

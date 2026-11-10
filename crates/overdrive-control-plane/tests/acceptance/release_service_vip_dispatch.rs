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

use overdrive_core::UnixInstant;
use overdrive_core::id::{ContentHash, CorrelationKey};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_sim::adapters::driver::SimDriver;

use crate::acceptance::vm_dispatch::{VmDispatchFixture, VmDispatchPorts};

#[tokio::test]
async fn release_action_dispatch_invokes_allocator_release() {
    // ---- Allocator: the fixture's real PersistentServiceVipAllocator over
    // its tempdir-backed LocalIntentStore. The pre-populate path goes
    // through `allocate(&digest)` which writes the entry to the
    // IntentStore allocator_entries table AND inserts into the memo,
    // mirroring the production lifecycle precisely.
    let fixture = VmDispatchFixture::boot(VmDispatchPorts::sim(Arc::new(SimDriver::new(
        DriverType::Vm,
    )) as Arc<dyn Driver>))
    .await;
    let allocator = Arc::clone(&fixture.state.allocator);

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
    // (target = "workload-lifecycle/<workload>", spec_hash = digest,
    // purpose = "release-service-vip") so an end-to-end test in 03-03
    // sees the same correlation derivation.
    let correlation =
        CorrelationKey::derive("workload-lifecycle/test-workload", &digest, "release-service-vip");
    let action = Action::ReleaseServiceVip { spec_digest: digest, correlation };

    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    // ---- Dispatch — the action-shim arm under test.
    fixture.dispatch(vec![action], &tick).await.expect("dispatch must succeed");

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

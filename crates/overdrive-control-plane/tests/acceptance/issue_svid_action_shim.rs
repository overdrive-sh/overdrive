//! Acceptance — workload-identity-manager Slice 01, step 01-06.
//!
//! Layer 1/2: the `issue_svid` action-shim executor contract (ADR-0067 D3).
//! Both scenarios drive through the action shim's public `dispatch` — the
//! action shim IS the driving port for the issue/hold-and-audit contract,
//! exactly as `lifecycle_broadcast.rs` drives the row-write-and-broadcast
//! contract through the same port. CA I/O lives entirely in the executor;
//! `reconcile()` never touches it (ADR-0067 D3, the ADR-0023 shim boundary).
//!
//! # S-WIM-02 — `IssueSvid` audits before hold
//!
//! Dispatch `Action::IssueSvid` through the shim with a `SimCa` +
//! `SimObservationStore`. Assert: (a) the `issued_certificates` audit row is
//! observable through the `ObservationStore` read surface, (b) `IdentityMgr`
//! holds the alloc, and (c) the held alloc's projected identity matches the
//! audit row — the cert and its audit fact are bound (ADR-0063 D6).
//!
//! # S-WIM-07 — audit-write failure refuses hold (`@error`)
//!
//! Inject an audit-write failure into the `SimObservationStore` so
//! `ca_issuance::issue_and_audit` returns `CaIssuanceError::Audit`. Assert:
//! dispatch surfaces the failure, `IdentityMgr` does NOT hold the alloc, and
//! NO audit row is left behind — no unaudited SVID escapes (K4 fail-closed).
//!
//! Universe (port-exposed observable surface):
//!   - the action-shim `dispatch` result (`Ok` / `Err(ShimError::...)`),
//!   - the `IdentityMgr` held snapshot (`HeldSvidFacts` projection),
//!   - the `ObservationStore` `issued_certificate_rows` audit surface.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::action_shim::dispatch;
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::test_default_allocator;
use overdrive_core::id::{AllocationId, CorrelationKey, NodeId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::Dataplane;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{ObservationStore, ObservationStoreError};
use overdrive_core::{SpiffeId, UnixInstant};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use tempfile::TempDir;

/// The node the SVID is issued on (also the obs-store writer).
fn issuing_node() -> NodeId {
    NodeId::new("local").expect("valid NodeId")
}

/// The allocation an SVID is issued for.
fn issuing_alloc() -> AllocationId {
    AllocationId::new("alloc-payments-0").expect("valid AllocationId")
}

/// The workload identity the reconciler derives (pure) and the executor mints.
fn workload_spiffe() -> SpiffeId {
    SpiffeId::new("spiffe://overdrive.local/job/payments/alloc/payments-0").expect("valid SpiffeId")
}

/// Cause-to-response linkage (kind-derived, not per-attempt).
fn issue_correlation() -> CorrelationKey {
    CorrelationKey::new("svid-lifecycle/alloc-payments-0:issue-svid").expect("valid CorrelationKey")
}

/// A single `Action::IssueSvid` for the canonical fixture identity.
fn issue_action() -> Action {
    Action::IssueSvid {
        alloc_id: issuing_alloc(),
        spiffe_id: workload_spiffe(),
        node_id: issuing_node(),
        correlation: issue_correlation(),
    }
}

/// A `TickContext` at logical tick `n` — wall-clock fields are unused by the
/// `IssueSvid` executor (the window is the injected `Clock`'s, not the tick's).
fn make_tick(tick_n: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: tick_n,
        deadline: now + Duration::from_secs(1),
    }
}

/// A fresh real `LocalIntentStore` for the allocator boilerplate. The allocator
/// is a mandatory `dispatch` port but is NOT exercised by the `IssueSvid` arm —
/// this mirrors the `lifecycle_broadcast.rs` precedent (real redb in the
/// default acceptance lane, allocator unused by the arm under test).
fn intent_store(dir: &TempDir) -> Arc<dyn IntentStore> {
    Arc::new(
        overdrive_store_local::LocalIntentStore::open(dir.path().join("intent.redb"))
            .expect("open intent store"),
    )
}

/// `@in-memory` `@S-WIM-02` -- `Action::IssueSvid` calls
/// `ca_issuance::issue_and_audit`, observes the `issued_certificates` audit row,
/// then holds the returned `SvidMaterial` in `IdentityMgr`.
#[tokio::test]
async fn issue_svid_executor_audits_before_hold() {
    // GIVEN the action-shim driving port wired with a sim CA, a sim observation
    // store (the audit PORT), a held-SVID store, and the non-CA shim ports.
    let ca: Arc<dyn Ca> = Arc::new(SimCa::new(Arc::new(SimEntropy::new(0xCA_02))));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(issuing_node(), 0xCA_02));
    let identity = IdentityMgr::new(None);
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());

    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let dataplane: Arc<dyn Dataplane> = Arc::new(SimDataplane::new());
    let dir = TempDir::new().expect("intent tempdir");
    let allocator = test_default_allocator(intent_store(&dir));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    let tick = make_tick(0);

    // WHEN the reconciler-emitted IssueSvid is dispatched through the shim.
    dispatch(
        vec![issue_action()],
        driver.as_ref(),
        obs.as_ref(),
        dataplane.as_ref(),
        ca.as_ref(),
        clock.as_ref(),
        &identity,
        &tx,
        &tick,
        &issuing_node(),
        allocator,
        &broker,
        None,
        None,
        // transparent-mtls-enrollment step 04-01: a fresh per-host slot
        // allocator — this fixture exercises no netns provisioning.
        &overdrive_control_plane::veth_provisioner::NetSlotAllocator::new(),
    )
    .await
    .expect("IssueSvid dispatch succeeds");

    // THEN (a) exactly one issued_certificates audit row is observable.
    let rows = obs.issued_certificate_rows().await.expect("read audit rows");
    assert_eq!(rows.len(), 1, "IssueSvid writes exactly one issued_certificates audit row");
    let row = &rows[0];

    // AND the row was written for the workload identity the action carried.
    assert_eq!(
        row.spiffe_id,
        workload_spiffe(),
        "audit row identity matches the action's spiffe_id"
    );

    // AND (b) IdentityMgr holds the alloc, with (c) the projected identity
    // matching the audit row — the held cert and its audit fact are bound.
    let snapshot = identity.held_snapshot();
    let facts = snapshot.get(&issuing_alloc()).expect("alloc is held after IssueSvid");
    assert_eq!(
        facts.spiffe_id, row.spiffe_id,
        "held SVID identity matches the audited identity (audit-before-hold binding)"
    );
    assert_eq!(
        facts.not_after, row.not_after,
        "held SVID validity end matches the audit row not_after (single window SSOT)"
    );
}

/// `@in-memory` `@error` `@S-WIM-07` -- if the audit write fails, issuance is
/// refused and no unaudited SVID is placed in the held map.
#[tokio::test]
async fn audit_write_failure_refuses_hold() {
    // GIVEN a sim CA but an observation store whose NEXT write fails — the
    // audit-write fault injected through the sim adapter's queue (the audit
    // path flows through ObservationStore::write, so the fault lands there).
    let ca: Arc<dyn Ca> = Arc::new(SimCa::new(Arc::new(SimEntropy::new(0xCA_07))));
    let obs = SimObservationStore::single_peer(issuing_node(), 0xCA_07);
    obs.inject_write_failure(ObservationStoreError::Io(std::io::Error::other(
        "audit store unavailable (injected)",
    )));
    let obs: Arc<dyn ObservationStore> = Arc::new(obs);
    let identity = IdentityMgr::new(None);
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());

    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let dataplane: Arc<dyn Dataplane> = Arc::new(SimDataplane::new());
    let dir = TempDir::new().expect("intent tempdir");
    let allocator = test_default_allocator(intent_store(&dir));
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    let tick = make_tick(0);

    // WHEN IssueSvid is dispatched against the failing audit store.
    let result = dispatch(
        vec![issue_action()],
        driver.as_ref(),
        obs.as_ref(),
        dataplane.as_ref(),
        ca.as_ref(),
        clock.as_ref(),
        &identity,
        &tx,
        &tick,
        &issuing_node(),
        allocator,
        &broker,
        None,
        None,
        // transparent-mtls-enrollment step 04-01: a fresh per-host slot
        // allocator — this fixture exercises no netns provisioning.
        &overdrive_control_plane::veth_provisioner::NetSlotAllocator::new(),
    )
    .await;

    // THEN dispatch surfaces the failure (no unaudited SVID escaped silently).
    assert!(result.is_err(), "an audit-write failure must surface from dispatch, got {result:?}");

    // AND IdentityMgr does NOT hold the alloc — fail-closed (K4): no unaudited
    // SvidMaterial ever enters the held map.
    assert!(
        identity.held_snapshot().is_empty(),
        "a refused issuance must leave the held map empty — no unaudited SVID held"
    );

    // AND no audit row was recorded — the cert and its row are observable
    // together or not at all.
    let rows = obs.issued_certificate_rows().await.expect("read audit rows");
    assert!(rows.is_empty(), "a refused issuance must leave NO audit row behind");
}

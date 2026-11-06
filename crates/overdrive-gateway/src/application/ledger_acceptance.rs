//! S-PIG-37 cleanup-ledger acceptance (R-5, DG-11: reserve before
//! `open_upstream`, bind after).
//!
//! The ledger is pure in-process state, so these bodies run in the default
//! lane once DELIVER lands. The only collaborator is a recording
//! `GatewayConnectDataplane` double used by `retry_all`.
#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use std::collections::BTreeMap;
use std::num::{NonZeroU16, NonZeroU32};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use overdrive_core::dataplane::{Proto, ServiceFrontend};
use overdrive_core::id::{BackendId, ServiceVip, SpiffeId};
use overdrive_core::public_ingress::{ServiceKey, SocketCookie};
use overdrive_core::traits::clock::Clock;

use super::*;
use crate::ports::{
    GatewayConnectCleanupSweep, GatewayConnectDataplane, GatewayConnectDataplaneError,
    GatewayConnectIntent, GatewaySelectionReceipt, PreparedUpstream,
};

/// Recording `GatewayConnectDataplane` double: only `cleanup` is exercised by
/// the ledger; it can be made to fail.
struct RecordingDataplane {
    cleanup_failing: AtomicBool,
    cleaned: Mutex<Vec<GatewayConnectIntent>>,
}

impl RecordingDataplane {
    fn new() -> Self {
        Self { cleanup_failing: AtomicBool::new(false), cleaned: Mutex::new(Vec::new()) }
    }
    fn cleaned(&self) -> Vec<GatewayConnectIntent> {
        self.cleaned.lock().expect("cleanup log").clone()
    }
}

impl GatewayConnectCleanup for RecordingDataplane {
    fn cleanup(&self, intent: &GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        self.cleaned.lock().expect("cleanup log").push(intent.clone());
        if self.cleanup_failing.load(Ordering::SeqCst) {
            return Err(GatewayConnectDataplaneError::Cleanup);
        }
        Ok(())
    }

    fn cleanup_all_gateway_intents(
        &self,
    ) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError> {
        Ok(GatewayConnectCleanupSweep { removed_intents: 0, removed_receipts: 0 })
    }

    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        Ok(0)
    }

    fn live_receipts(
        &self,
    ) -> Result<BTreeMap<SocketCookie, GatewaySelectionReceipt>, GatewayConnectDataplaneError> {
        Ok(BTreeMap::new())
    }
}

#[async_trait]
impl GatewayConnectDataplane for RecordingDataplane {
    async fn probe(
        &self,
        _deadline: Instant,
        _clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError> {
        Ok(())
    }

    fn open_upstream(
        &self,
        _key: ServiceKey,
    ) -> Result<PreparedUpstream, GatewayConnectDataplaneError> {
        Err(GatewayConnectDataplaneError::Unavailable)
    }

    fn take_receipt(
        &self,
        _intent: &GatewayConnectIntent,
    ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
        Err(GatewayConnectDataplaneError::ReceiptMissing)
    }

    fn selected_backend_identity(
        &self,
        backend_id: BackendId,
    ) -> Result<SpiffeId, GatewayConnectDataplaneError> {
        Err(GatewayConnectDataplaneError::BackendIdentityMissing { backend_id })
    }
}

fn capacity(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("nonzero capacity")
}

fn intent(cookie: u64) -> GatewayConnectIntent {
    let frontend = ServiceFrontend::new(
        ServiceVip::new("10.96.0.1".parse().expect("IP")).expect("VIP"),
        NonZeroU16::new(8080).expect("port"),
        Proto::Tcp,
    )
    .expect("frontend");
    GatewayConnectIntent {
        socket_cookie: SocketCookie::new(cookie).expect("nonzero cookie"),
        service_key: ServiceKey::from_frontend(frontend),
    }
}

fn reserve_one(ledger: &Arc<GatewayCleanupLedger>) -> GatewayCleanupReservation {
    match ledger.reserve() {
        Ok(reservation) => reservation,
        Err(full) => panic!("ledger refused below capacity (capacity {})", full.capacity),
    }
}

fn refused_capacity(ledger: &Arc<GatewayCleanupLedger>) -> NonZeroU32 {
    match ledger.reserve() {
        Ok(_) => panic!("the ledger accepted a reservation beyond capacity"),
        Err(full) => full.capacity,
    }
}

/// S-PIG-37: 0, 1 and 128 reservations are accepted, each holding exactly one
/// slot, and the 129th is refused with the ledger capacity.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn ledger_accepts_zero_one_and_128_reservations_and_refuses_the_129th() {
    let dataplane = RecordingDataplane::new();
    for held in [0_u32, 1, 128] {
        let ledger = GatewayCleanupLedger::new(capacity(128));
        let reservations: Vec<_> = (0..held).map(|_| reserve_one(&ledger)).collect();
        assert_eq!(ledger.len(), held, "each reservation holds exactly one slot");
        assert!(
            ledger.retry_all(&dataplane).is_empty(),
            "{held} unbound reservations are not dirty and never saturated",
        );
        drop(reservations);
        assert_eq!(ledger.len(), 0, "dropping unbound reservations releases every slot");
    }

    let ledger = GatewayCleanupLedger::new(capacity(128));
    let full: Vec<_> = (0..128).map(|_| reserve_one(&ledger)).collect();
    assert_eq!(refused_capacity(&ledger), capacity(128), "the 129th names the capacity");
    assert_eq!(ledger.len(), 128, "a refusal consumes no slot");
    assert!(dataplane.cleaned().is_empty(), "a refusal performs no cleanup");
    drop(full);
}

/// S-PIG-37: a released slot is reusable at capacity, and the refusal that
/// preceded it stays recorded as saturation.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn released_slot_is_reusable_at_capacity() {
    let ledger = GatewayCleanupLedger::new(capacity(128));
    let mut held: Vec<_> = (0..128).map(|_| reserve_one(&ledger)).collect();
    assert_eq!(refused_capacity(&ledger), capacity(128));
    drop(held.pop());
    assert_eq!(ledger.len(), 127, "dropping one unbound reservation releases one slot");
    let again = reserve_one(&ledger);
    assert_eq!(ledger.len(), 128);
    drop(again);
    drop(held);
    assert_eq!(ledger.len(), 0);
}

/// S-PIG-37: binding the registered intent keeps the slot; completing the
/// reservation removes the record.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn bound_reservation_holds_its_slot_until_complete_removes_the_record() {
    let ledger = GatewayCleanupLedger::new(capacity(128));
    let dataplane = RecordingDataplane::new();
    let mut reservation = reserve_one(&ledger);
    assert_eq!(ledger.len(), 1);
    reservation.bind(intent(1));
    assert_eq!(ledger.len(), 1, "binding does not take a second slot");
    reservation.complete();
    assert_eq!(ledger.len(), 0, "a completed record is removed");
    assert!(ledger.retry_all(&dataplane).is_empty());
    assert!(dataplane.cleaned().is_empty(), "a clean record needs no further cleanup");
}

/// S-PIG-37: every refusal increments the saturation count, which
/// `retry_all` reports as `LedgerSaturated` even after later cleanup succeeds.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn saturation_stays_reported_after_the_ledger_drains() {
    let ledger = GatewayCleanupLedger::new(capacity(128));
    let dataplane = RecordingDataplane::new();
    let held: Vec<_> = (0..128).map(|_| reserve_one(&ledger)).collect();
    assert_eq!(refused_capacity(&ledger), capacity(128));
    assert_eq!(refused_capacity(&ledger), capacity(128));
    drop(held);
    assert_eq!(ledger.len(), 0, "the ledger is empty again");
    assert_eq!(
        ledger.retry_all(&dataplane),
        vec![GatewayCleanupFailure::LedgerSaturated { attempts: 2, capacity: 128 }],
        "two refusals stay typed as non-clean after the drain",
    );
}

/// S-PIG-37: records marked failed are retained until `retry_all` cleans their
/// exact intent; failures are grouped as closed-category counts and a failing
/// cleanup keeps the record for the next sweep.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn retry_all_cleans_exact_intents_and_keeps_records_whose_cleanup_fails() {
    let ledger = GatewayCleanupLedger::new(capacity(128));
    let dataplane = RecordingDataplane::new();

    let mut intent_dirty = reserve_one(&ledger);
    intent_dirty.bind(intent(1));
    intent_dirty.record_intent_failure();
    drop(intent_dirty);
    let mut receipt_dirty = reserve_one(&ledger);
    receipt_dirty.bind(intent(2));
    receipt_dirty.record_receipt_failure();
    drop(receipt_dirty);
    assert_eq!(ledger.len(), 2, "both failed records are retained");

    dataplane.cleanup_failing.store(true, Ordering::SeqCst);
    let failures = ledger.retry_all(&dataplane);
    assert_eq!(failures.len(), 2, "one failure category per retained bit: {failures:?}");
    assert!(failures.contains(&GatewayCleanupFailure::ConnectIntent { count: 1 }));
    assert!(failures.contains(&GatewayCleanupFailure::Receipt { count: 1 }));
    assert_eq!(ledger.len(), 2, "records whose cleanup failed remain");
    assert_eq!(dataplane.cleaned().len(), 2);

    dataplane.cleanup_failing.store(false, Ordering::SeqCst);
    assert!(ledger.retry_all(&dataplane).is_empty());
    assert_eq!(ledger.len(), 0);
    let cleaned = dataplane.cleaned();
    assert_eq!(cleaned.len(), 4, "two failed attempts and two successful retries");
    assert!(cleaned[2..].contains(&intent(1)) && cleaned[2..].contains(&intent(2)));
}

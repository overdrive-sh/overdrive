//! `ServiceLifecycleView` CBOR evolution for the startup-stop retry memory
//! (feature `service-allocation-readiness-and-ending`, S-38).
//!
//! - D8 item 1: `startup_last_acted_run` is an additive `#[serde(default)]`
//!   field (ADR-0035 / ADR-0036): a blob written without it decodes with an
//!   empty map and every other field unchanged; a blob with it round-trips.
//! - R2-2: `stable_announced` is deleted; an old blob that still carries the
//!   key decodes and the key is ignored.
//!
//! Port: the runtime's `ViewStore` codec is CBOR (`ciborium`) over the view's
//! serde shape; the oracle is the decoded `ServiceLifecycleView`.

#![allow(clippy::doc_markdown, reason = "per-test CONTRACT_SHAPE declarations")]

use std::collections::{BTreeMap, BTreeSet};

use overdrive_core::id::AllocationId;
use overdrive_core::observation::{ProbeIdx, ProbeRunSeq};
use overdrive_reconcilers::service_lifecycle::ServiceLifecycleView;
use serde::Serialize;

fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}

fn cbor<T: Serialize>(value: &T) -> Vec<u8> {
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes).expect("CBOR encode");
    bytes
}

fn decode(bytes: &[u8]) -> ServiceLifecycleView {
    ciborium::from_reader(bytes).expect("CBOR decode of a persisted view")
}

/// The persisted shape before the startup-stop retry memory landed.
#[derive(Serialize)]
struct PersistedViewBeforeStartupRetry {
    stable_announced: BTreeSet<AllocationId>,
    observed: BTreeSet<AllocationId>,
    terminal_announced: BTreeSet<AllocationId>,
    liveness_last_acted_run: BTreeMap<(AllocationId, ProbeIdx), ProbeRunSeq>,
}

/// A blob written without `startup_last_acted_run` decodes with an empty map and
/// every other field unchanged.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn blob_without_startup_last_acted_run_decodes_with_an_empty_map() {
    let alloc = aid("svc-0");
    let live = BTreeMap::from([(
        (alloc.clone(), ProbeIdx::new(0)),
        ProbeRunSeq::new(3).expect("non-zero"),
    )]);
    let persisted = PersistedViewBeforeStartupRetry {
        stable_announced: BTreeSet::new(),
        observed: BTreeSet::from([alloc.clone()]),
        terminal_announced: BTreeSet::from([alloc]),
        liveness_last_acted_run: live.clone(),
    };

    let decoded = decode(&cbor(&persisted));

    assert!(decoded.startup_last_acted_run.is_empty());
    assert_eq!(decoded.observed, persisted.observed);
    assert_eq!(decoded.terminal_announced, persisted.terminal_announced);
    assert_eq!(decoded.liveness_last_acted_run, live);
}

/// A blob that still carries the removed `stable_announced` key decodes and the
/// key is ignored: every other field is kept, nothing remains of the key.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER: R2-2 the stable_announced view field is deleted"]
fn blob_carrying_the_removed_stable_announced_key_is_ignored() {
    let alloc = aid("svc-0");
    let persisted = PersistedViewBeforeStartupRetry {
        stable_announced: BTreeSet::from([alloc.clone()]),
        observed: BTreeSet::from([alloc.clone()]),
        terminal_announced: BTreeSet::new(),
        liveness_last_acted_run: BTreeMap::new(),
    };

    let decoded = decode(&cbor(&persisted));

    // The expected view names no `stable_announced`: after the field is deleted
    // the key is unknown to the decoder and ignored.
    let expected = ServiceLifecycleView {
        observed: BTreeSet::from([alloc]),
        ..ServiceLifecycleView::default()
    };
    assert_eq!(decoded, expected);
}

/// The acted run survives the runtime's CBOR write-through: a restarted control
/// plane keeps which startup row each allocation was last stopped on.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn startup_last_acted_run_survives_persistence() {
    let view = ServiceLifecycleView {
        startup_last_acted_run: BTreeMap::from([
            ((aid("svc-0"), ProbeIdx::new(0)), ProbeRunSeq::new(2).expect("non-zero")),
            ((aid("svc-1"), ProbeIdx::new(0)), ProbeRunSeq::new(5).expect("non-zero")),
        ]),
        ..ServiceLifecycleView::default()
    };

    assert_eq!(decode(&cbor(&view)), view);
}

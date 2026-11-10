//! Feature `service-allocation-readiness-and-ending`, decision D10 item 5, obligation
//! S-34 (golden half) — `service_lifecycle::startup_deadline`, the derivation
//! extracted from `spec_facts_for_service`: `interval x max_attempts` of startup
//! probe index 0, or `DEFAULT_STARTUP_DEADLINE` for an empty list or on overflow.
//!
//! The golden cases are the values the derivation produced before the extraction, so
//! the extraction is behaviour-preserving.
//!
//! CONTRACT_SHAPE: pure-function (a `Duration` for descriptors; no state, no I/O).

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::doc_markdown)]

use std::time::Duration;

use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_reconcilers::service_lifecycle::{DEFAULT_STARTUP_DEADLINE, startup_deadline};
use proptest::prelude::*;

fn startup_probe(idx: u32, interval_seconds: u32, max_attempts: u32) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(idx),
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port: 8080 },
        timeout_seconds: 1,
        interval_seconds,
        max_attempts,
        failure_threshold: None,
        success_threshold: None,
    }
}

/// The derivation as `spec_facts_for_service` computed it before the extraction.
fn reference(probes: &[ProbeDescriptor]) -> Duration {
    let Some(first) = probes.first() else { return DEFAULT_STARTUP_DEADLINE };
    Duration::from_secs(u64::from(first.interval_seconds))
        .checked_mul(first.max_attempts)
        .unwrap_or(DEFAULT_STARTUP_DEADLINE)
}

/// Golden cases: the empty list (the platform default), one probe, several probes
/// (only index 0 counts), zero interval, the largest representable product.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_startup_deadline_matches_the_values_the_derivation_produced_before_extraction() {
    assert_eq!(startup_deadline(&[]), Duration::from_secs(60));
    assert_eq!(startup_deadline(&[startup_probe(0, 5, 12)]), Duration::from_secs(60));
    assert_eq!(startup_deadline(&[startup_probe(0, 10, 12)]), Duration::from_secs(120));
    assert_eq!(startup_deadline(&[startup_probe(0, 1, 3)]), Duration::from_secs(3));
    assert_eq!(startup_deadline(&[startup_probe(0, 0, 9)]), Duration::ZERO);
    assert_eq!(
        startup_deadline(&[startup_probe(0, 2, 3), startup_probe(1, 100, 100)]),
        Duration::from_secs(6),
        "only startup probe index 0 counts",
    );
    assert_eq!(
        startup_deadline(&[startup_probe(0, u32::MAX, u32::MAX)]),
        reference(&[startup_probe(0, u32::MAX, u32::MAX)]),
        "the extreme product agrees with the derivation, with no panic",
    );
}

proptest! {
    /// S-34 — for every descriptor list the extracted function equals the derivation it
    /// was extracted from.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn the_startup_deadline_equals_the_original_derivation_for_every_descriptor_list(
        specs in prop::collection::vec((any::<u32>(), any::<u32>()), 0..4),
    ) {
        let probes: Vec<ProbeDescriptor> = specs
            .into_iter()
            .enumerate()
            .map(|(idx, (interval, attempts))| {
                startup_probe(u32::try_from(idx).expect("small"), interval, attempts)
            })
            .collect();
        prop_assert_eq!(startup_deadline(&probes), reference(&probes));
    }
}

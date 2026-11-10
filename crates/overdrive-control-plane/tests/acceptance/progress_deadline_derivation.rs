//! Feature `service-allocation-readiness-and-ending`, decision D10, obligation S-34
//! (gate G-7; Quint module `progress_deadline`) — the Service stream's progress
//! deadline exceeds the startup deadline by construction, tested on the real
//! functions.
//!
//! `progress_deadline(startup_probes, floor) = max(floor, startup_deadline(
//! startup_probes) + PROGRESS_DEADLINE_MARGIN)`: no key, flag or server setting; at
//! the default spec the value is 90 s. The Job stream keeps `streaming_cap`.
//!
//! CONTRACT_SHAPE: pure-function (a `Duration` for descriptors and a floor; no
//! state, no I/O).

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::doc_markdown)]

use std::time::Duration;

use overdrive_control_plane::DEFAULT_STREAMING_CAP;
use overdrive_control_plane::streaming::{PROGRESS_DEADLINE_MARGIN, progress_deadline};
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
        inferred: false,
    }
}

/// Descriptor lists: empty, one, many; intervals and attempts across the whole `u32`
/// range, so the product reaches its overflow edge.
fn descriptors() -> impl Strategy<Value = Vec<ProbeDescriptor>> {
    prop::collection::vec((any::<u32>(), any::<u32>()).prop_map(|(i, m)| (i, m)), 0..4).prop_map(
        |pairs| {
            pairs
                .into_iter()
                .enumerate()
                .map(|(idx, (interval, attempts))| {
                    startup_probe(u32::try_from(idx).expect("small"), interval, attempts)
                })
                .collect()
        },
    )
}

proptest! {
    /// S-34 — for every descriptor list and floor, the progress deadline exceeds the
    /// startup deadline, keeps the floor, and is at least the startup deadline plus
    /// the margin.
    #[test]
    /// CONTRACT_SHAPE: pure-function.
    fn the_progress_deadline_exceeds_the_startup_deadline_and_keeps_the_floor(
        probes in descriptors(),
        floor_seconds in 0_u64..=3_600,
    ) {
        let floor = Duration::from_secs(floor_seconds);
        let deadline = progress_deadline(&probes, floor);
        let startup = startup_deadline(&probes);
        prop_assert!(deadline > startup, "deadline {deadline:?} <= startup {startup:?}");
        prop_assert!(deadline >= floor, "deadline {deadline:?} below floor {floor:?}");
        prop_assert!(deadline >= startup + PROGRESS_DEADLINE_MARGIN);
        prop_assert!(deadline == floor || deadline == startup + PROGRESS_DEADLINE_MARGIN);
    }
}

/// S-34 — the default spec (no declared startup probe, the platform default startup
/// deadline of 60 s) yields exactly the 90 s streaming cap.
#[test]
/// CONTRACT_SHAPE: pure-function.
fn the_default_spec_yields_ninety_seconds() {
    assert_eq!(DEFAULT_STARTUP_DEADLINE, Duration::from_secs(60));
    assert_eq!(PROGRESS_DEADLINE_MARGIN, Duration::from_secs(30));
    assert_eq!(progress_deadline(&[], DEFAULT_STREAMING_CAP), Duration::from_secs(90));
}

/// S-34 golden — a startup deadline above the floor lifts the progress deadline to
/// `startup_deadline + margin`; one below it keeps the floor; only startup probe
/// index 0 counts.
#[test]
/// CONTRACT_SHAPE: pure-function.
fn a_long_startup_deadline_lifts_the_progress_deadline_and_a_short_one_keeps_the_floor() {
    let long = [startup_probe(0, 10, 12)];
    assert_eq!(progress_deadline(&long, DEFAULT_STREAMING_CAP), Duration::from_secs(150));
    let short = [startup_probe(0, 1, 2)];
    assert_eq!(progress_deadline(&short, DEFAULT_STREAMING_CAP), Duration::from_secs(90));
    let only_first_counts = [startup_probe(0, 1, 2), startup_probe(1, 100, 100)];
    assert_eq!(
        progress_deadline(&only_first_counts, DEFAULT_STREAMING_CAP),
        Duration::from_secs(90),
    );
}

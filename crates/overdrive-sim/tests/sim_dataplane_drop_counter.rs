//! `SimDataplane` drop-counter mirror — Slice 06 / S-2.2-23.
//!
//! Mirrors the production kernel-side `DROP_COUNTER`
//! `BPF_MAP_TYPE_PERCPU_ARRAY`: one slot per `DropClass` variant,
//! incremented when the kernel-side sanity prologue drops a frame.
//! DST tests assert on per-class increments through this surface
//! without loading a kernel.
//!
//! The acceptance test exercises the full path through the public
//! `SimDataplane` driving port: construct a fresh dataplane, record
//! drops via `record_drop(class)`, read counts via
//! `read_drop_counter(class)` and `snapshot_drop_counter()`. Slot
//! indices are stable per ADR-0040 D8 (Q7=B locks 6 slots).

#![allow(clippy::expect_used)]

use overdrive_core::dataplane::DropClass;
use overdrive_sim::adapters::dataplane::SimDataplane;

/// Acceptance test — Slice 06 mirror works end-to-end through the
/// public `SimDataplane` API. Records one drop per class, verifies
/// the per-class counter is exactly 1 and the others are zero,
/// then verifies the snapshot reflects the mirror in canonical
/// slot order.
#[test]
fn sim_dataplane_drop_counter_records_per_class_increments() {
    let sim = SimDataplane::new();

    // Fresh dataplane has zero counts everywhere.
    for class in [
        DropClass::MalformedHeader,
        DropClass::UnknownVip,
        DropClass::NoHealthyBackend,
        DropClass::SanityPrologue,
        DropClass::ReverseNatMiss,
        DropClass::OversizePacket,
    ] {
        assert_eq!(sim.read_drop_counter(class), 0, "fresh dataplane has zero {class:?}");
    }

    // Record a single drop for each class. Each `record_drop` is a
    // single increment of the matching slot.
    sim.record_drop(DropClass::MalformedHeader);
    sim.record_drop(DropClass::UnknownVip);
    sim.record_drop(DropClass::NoHealthyBackend);
    sim.record_drop(DropClass::SanityPrologue);
    sim.record_drop(DropClass::ReverseNatMiss);
    sim.record_drop(DropClass::OversizePacket);

    // Each slot now reads as 1.
    assert_eq!(sim.read_drop_counter(DropClass::MalformedHeader), 1);
    assert_eq!(sim.read_drop_counter(DropClass::UnknownVip), 1);
    assert_eq!(sim.read_drop_counter(DropClass::NoHealthyBackend), 1);
    assert_eq!(sim.read_drop_counter(DropClass::SanityPrologue), 1);
    assert_eq!(sim.read_drop_counter(DropClass::ReverseNatMiss), 1);
    assert_eq!(sim.read_drop_counter(DropClass::OversizePacket), 1);

    // Snapshot reflects the mirror in canonical slot order
    // (index = `DropClass::as_index()`).
    let snapshot = sim.snapshot_drop_counter();
    assert_eq!(snapshot, [1_u64; 6]);
}

/// Multiple increments accumulate. The 20× `MalformedHeader`
/// pattern matches the S-2.2-22 mixed-batch scenario expectation.
#[test]
fn sim_dataplane_drop_counter_accumulates_under_repeated_record_drop() {
    let sim = SimDataplane::new();

    for _ in 0..20 {
        sim.record_drop(DropClass::MalformedHeader);
    }
    sim.record_drop(DropClass::UnknownVip);
    sim.record_drop(DropClass::UnknownVip);
    sim.record_drop(DropClass::UnknownVip);

    assert_eq!(sim.read_drop_counter(DropClass::MalformedHeader), 20);
    assert_eq!(sim.read_drop_counter(DropClass::UnknownVip), 3);
    // Other slots untouched.
    assert_eq!(sim.read_drop_counter(DropClass::NoHealthyBackend), 0);
    assert_eq!(sim.read_drop_counter(DropClass::SanityPrologue), 0);
    assert_eq!(sim.read_drop_counter(DropClass::ReverseNatMiss), 0);
    assert_eq!(sim.read_drop_counter(DropClass::OversizePacket), 0);
}

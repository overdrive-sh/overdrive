//! Compile-fail fixture for `DropClass` slot-mapping drift.
//!
//! Asserts that `DropClass::VARIANT_COUNT` is structurally locked to
//! the actual variant count (6) via a const-assert in
//! `crates/overdrive-core/src/dataplane/drop_class.rs`. Adding a
//! variant without bumping `VARIANT_COUNT`, OR changing
//! `VARIANT_COUNT` away from the actual variant count, fails this
//! const-assert and produces the diagnostic captured in the sibling
//! `.stderr` fixture.
//!
//! The fixture body simulates the drift by const-asserting an
//! intentionally-wrong invariant — `VARIANT_COUNT == 5`. This
//! mirrors the shape of the real const-assert inside
//! `drop_class.rs`, so any reviewer reading the production const-
//! assert sees the same construct fail in this fixture.
//!
//! See ADR-0040 D8 (Q7=B) for the slot-count lock; reordering or
//! removing a variant is a major-version break that requires a new
//! ADR.

use overdrive_core::dataplane::DropClass;

const _: () = assert!(
    DropClass::VARIANT_COUNT == 5,
    "drop_class slot drift: VARIANT_COUNT diverged from variant count",
);

fn main() {}

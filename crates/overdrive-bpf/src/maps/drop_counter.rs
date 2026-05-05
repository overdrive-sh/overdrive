//! `DROP_COUNTER` — kernel-side `BPF_MAP_TYPE_PERCPU_ARRAY` keyed
//! on `u32 = DropClass as u32`, value `u64` (count). Slot count =
//! `DropClass::VARIANT_COUNT` (= 6) per Q7=B.
//!
//! Userspace sums across CPUs at read time per architecture.md
//! § 10. Slots locked per Q7=B in
//! `crates/overdrive-core/src/dataplane/drop_class.rs`.
//!
//! **RED scaffold** — `#[map]` declaration not yet emitted.
//! DELIVER lands it per Slice 06 (US-06; S-2.2-19..23).

#![allow(dead_code)]

pub const SCAFFOLD: bool = true;

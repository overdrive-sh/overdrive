//! `DROP_COUNTER` — kernel-side `BPF_MAP_TYPE_PERCPU_ARRAY` keyed
//! on `u32 = DropClass as u32`, value `u64` (count). Slot count =
//! `DropClass::VARIANT_COUNT` (= 6) per Q7=B / ADR-0040 D8.
//!
//! Userspace sums across CPUs at read time per architecture.md
//! § 10. Slots locked per Q7=B in
//! `crates/overdrive-core/src/dataplane/drop_class.rs` —
//! `MalformedHeader=0, UnknownVip=1, NoHealthyBackend=2,
//! SanityPrologue=3, ReverseNatMiss=4, OversizePacket=5`.
//!
//! Kernel-side write pattern (per research § 7.1 / aya-rs research
//! § D.4):
//!
//! ```ignore
//! if let Some(counter) = unsafe { DROP_COUNTER.get_ptr_mut(class as u32) } {
//!     unsafe { *counter += 1; }
//! }
//! ```
//!
//! `get_ptr_mut` returns a per-CPU pointer; the increment is
//! per-CPU-local and lock-free. Single-writer per CPU within an
//! XDP program context, so the unsynchronised `+=` is safe (no
//! re-entry; no tail-calls in this path).

use aya_ebpf::{macros::map, maps::PerCpuArray};

/// `DROP_COUNTER` slot count. Lockstep with
/// `overdrive_core::dataplane::DropClass::VARIANT_COUNT` (Q7=B /
/// ADR-0040 D8). `overdrive-bpf` is `#![no_std]` and cannot import
/// `overdrive-core` directly, so the constant is mirrored here; the
/// const-assert in `drop_class.rs` (compile-fail-fixture-tested via
/// `tests/compile_fail/drop_class_slot_drift.rs`) is the structural
/// gate against drift.
pub const SLOT_COUNT: u32 = 6;

/// Per-CPU drop counter, indexed by `DropClass::as_index()`.
/// Userspace reads via `aya::maps::PerCpuArray::get(&idx, 0)` and
/// sums across the returned per-CPU values (see
/// `overdrive_core::dataplane::aggregate_per_cpu`).
#[map]
pub static DROP_COUNTER: PerCpuArray<u64> = PerCpuArray::with_max_entries(SLOT_COUNT, 0);

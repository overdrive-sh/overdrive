//! `REVERSE_LOCAL_MISS_COUNTER` — kernel-side `BPF_MAP_TYPE_PERCPU_ARRAY`
//! of one `u64` slot counting `REVERSE_LOCAL_MAP` misses on the
//! `cgroup_recvmsg4_service` reply path (ADR-0053 rev 2026-06-05, GH
//! #200).
//!
//! A reverse miss is a should-never-happen path under the ordered
//! (reverse-first) dual-write, so this counter measures
//! corruption/eviction, not a routine branch. It is NOT a `DropClass`
//! variant — recvmsg4 does not drop (verifier `[1,1]`, DDD-3); the miss
//! is handled by a source rewrite to the sentinel `192.0.2.1` plus this
//! observable count (DDD-3, US-03 / K5).
//!
//! Per-CPU avoids contention on the recvmsg hot path; userspace sums
//! across CPUs at read time (the `aggregate_per_cpu` precedent that
//! `DROP_COUNTER` uses).
//!
//! # RED scaffold (Slice 03 / S-03-01)
//!
//! `#[map]` attribute lands in DELIVER GREEN (Slice 03). The absent
//! attribute IS the kernel-side RED signal per `maps/mod.rs`.

#![allow(dead_code)]

use aya_ebpf::maps::PerCpuArray;

/// Single slot — the reverse-miss count. Indexed by `0`. A future
/// per-reason split would widen this; Phase-1 has one reason ("no
/// reverse entry for this backend identity").
pub const MAX_ENTRIES: u32 = 1;

/// Slot index for the reverse-miss count.
pub const SLOT_REVERSE_MISS: u32 = 0;

/// `REVERSE_LOCAL_MISS_COUNTER` — one `u64` per CPU.
///
/// RED scaffold: `#[map]` attribute lands in DELIVER GREEN (Slice 03).
// __SCAFFOLD__ — add `#[map]` in DELIVER (Slice 03 GREEN).
pub static REVERSE_LOCAL_MISS_COUNTER: PerCpuArray<u64> =
    PerCpuArray::with_max_entries(MAX_ENTRIES, 0);

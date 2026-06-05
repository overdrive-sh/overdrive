//! `REVERSE_LOCAL_MISS_COUNTER` — kernel-side `BPF_MAP_TYPE_PERCPU_ARRAY`
//! of one `u64` slot counting `REVERSE_LOCAL_MAP` misses on the
//! `cgroup_recvmsg4_service` reply path (ADR-0053 rev 2026-06-05, GH
//! #200).
//!
//! recvmsg4 fires on EVERY unconnected-UDP recv from a cgroup descendant
//! — service replies AND all unrelated same-host UDP — so a `REVERSE_LOCAL_MAP`
//! miss is the common non-service-reply case. The miss path is a pure
//! no-op (ADR-0053 § D3 rev 2026-06-05b / UI-1): it leaves the real source
//! untouched and bumps this counter for observability only. It is NOT a
//! `DropClass` variant — recvmsg4 does not drop (verifier `[1,1]`, DDD-3) —
//! and the counter is behaviorally inert (its incrementing has no effect on
//! the source the app reads). The K5 no-leak guarantee is preserved by the
//! reverse-first dual-write's always-hit property, NOT by a miss-path
//! sentinel.
//!
//! Per-CPU avoids contention on the recvmsg hot path; userspace sums
//! across CPUs at read time (the `aggregate_per_cpu` precedent that
//! `DROP_COUNTER` uses).

#![allow(dead_code)]

use aya_ebpf::{macros::map, maps::PerCpuArray};

/// Single slot — the reverse-miss count. Indexed by `0`. A future
/// per-reason split would widen this; Phase-1 has one reason ("no
/// reverse entry for this backend identity").
pub const MAX_ENTRIES: u32 = 1;

/// Slot index for the reverse-miss count.
pub const SLOT_REVERSE_MISS: u32 = 0;

/// `REVERSE_LOCAL_MISS_COUNTER` — one `u64` per CPU.
#[map]
pub static REVERSE_LOCAL_MISS_COUNTER: PerCpuArray<u64> =
    PerCpuArray::with_max_entries(MAX_ENTRIES, 0);

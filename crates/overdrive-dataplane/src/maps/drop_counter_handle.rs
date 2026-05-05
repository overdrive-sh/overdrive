//! `DropCounterHandle` — typed userspace wrapper around the
//! `DROP_COUNTER` `BPF_MAP_TYPE_PERCPU_ARRAY` per architecture.md
//! § 10.
//!
//! Key = `DropClass as u32`; value = per-CPU `u64`. The
//! `read(class)` operation sums across CPUs at read time.
//!
//! **RED scaffold** — bodies panic via `todo!()` until DELIVER
//! fills them per Slice 06.

#![allow(dead_code)]

pub const SCAFFOLD: bool = true;

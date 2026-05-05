//! `MaglevMapHandle` — typed userspace wrapper around the
//! `MAGLEV_MAP` outer `BPF_MAP_TYPE_HASH_OF_MAPS` per
//! architecture.md § 10.
//!
//! Outer key = `ServiceId`; inner = `BPF_MAP_TYPE_ARRAY` of
//! `BackendId` slots, size = `MaglevTableSize::DEFAULT.get()`.
//!
//! `swap_inner(service_id, new_table)` — invoked from the
//! `EbpfDataplane::update_service` body after
//! `maglev::generate(...)` produces the new permutation.
//!
//! **RED scaffold** — bodies panic via `todo!()` until DELIVER
//! fills them per Slice 04.

#![allow(dead_code)]

pub const SCAFFOLD: bool = true;

//! `ServiceMapHandle` — typed userspace wrapper around the
//! `SERVICE_MAP` outer `BPF_MAP_TYPE_HASH_OF_MAPS` per
//! architecture.md § 10.
//!
//! Outer key = `(ServiceVip, u16 port)` (host-order in the
//! map; converted at the kernel boundary). Inner = per-service
//! `BPF_MAP_TYPE_HASH` of `BackendId` → `BackendEntry`.
//!
//! Atomic-swap surface: `swap_inner(service_id, vip, new_inner)`
//! invokes the Slice 03 swap primitive and returns once the
//! single outer-map pointer write has committed.
//!
//! **RED scaffold** — bodies panic via `todo!()` until DELIVER
//! fills them per Slice 02 / 03.
//!
//! See test-scenarios.md S-2.2-04..06 (Slice 02), S-2.2-09..11
//! (Slice 03).

#![allow(dead_code)]

pub const SCAFFOLD: bool = true;

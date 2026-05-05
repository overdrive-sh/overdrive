//! `BackendMapHandle` — typed userspace wrapper around the
//! `BACKEND_MAP` `BPF_MAP_TYPE_HASH` per architecture.md § 10.
//!
//! Key = `BackendId`; value = `BackendEntry { ipv4, port,
//! weight, healthy, _pad }`.
//!
//! **RED scaffold** — bodies panic via `todo!()` until DELIVER
//! fills them per Slice 03 (S-2.2-09..11).

#![allow(dead_code)]

pub const SCAFFOLD: bool = true;

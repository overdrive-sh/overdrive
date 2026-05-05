//! Shared kernel-side helpers used by `xdp_service_map` and
//! `tc_reverse_nat`.
//!
//! Per architecture.md § 11 (endianness lockstep) + Q3=C (sanity
//! prologue strategy = shared `#[inline(always)]` Rust helper):
//!
//! - `sanity::*` — packet-shape sanity prologue and the wire/host
//!   byte-order conversion site (`reverse_key_from_packet`,
//!   `original_dest_to_wire`).
//!
//! **RED scaffold** — module declarations exist; helper bodies
//! panic via `todo!()` until DELIVER fills them per Slice 05 / 06.

pub mod sanity;

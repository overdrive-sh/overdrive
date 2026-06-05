//! Shared kernel-side helpers used by `xdp_service_map` and
//! `xdp_reverse_nat` (per ADR-0045 § Decision; replaces the
//! pre-pivot `tc_reverse_nat` consumer).
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

// unconnected-udp-sendmsg4 (GH #200, ADR-0053 rev 2026-06-05) — the
// single shared key-build + low-16-NBO site for connect4 + sendmsg4 +
// recvmsg4 (Option 3 / D4). RED scaffold: body is `todo!()` until
// DELIVER Slice 01. Does key-build ONLY — no lookup, no rewrite.
pub mod build_local_service_key;
pub mod csum;
pub mod sanity;

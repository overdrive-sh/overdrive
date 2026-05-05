//! Userspace BPF loader for Phase 2.2.
//!
//! Wraps `aya::Ebpf::load_file` + program attach for the new
//! `xdp_service_map` and `tc_reverse_nat` programs. Resolves
//! interface name → ifindex via `nix::net::if_::if_nametoindex`
//! per US-01 / S-2.2-03.
//!
//! Native-XDP attach is the default; on failure logs a structured
//! `xdp.attach.fallback_generic` warning and falls back to
//! `XDP_SKB` per US-01 / S-2.2-02.
//!
//! **RED scaffold** — bodies panic via `todo!()` until DELIVER
//! fills them per Slice 01 / 02 / 05.
//!
//! See test-scenarios.md S-2.2-01..03 (Slice 01).

#![allow(dead_code)]

/// Marker for DELIVER — the loader scaffold is not yet wired.
pub const SCAFFOLD: bool = true;

//! Userspace BPF loader for Phase 2.2.
//!
//! Wraps `aya::Ebpf::load_file` + program attach for the
//! `xdp_service_map_lookup` (forward path) and
//! `xdp_reverse_nat_lookup` (reverse path) programs. Resolves
//! interface name → ifindex via `nix::net::if_::if_nametoindex`
//! per US-01 / S-2.2-03. Per ADR-0045 § Operational, two ifaces
//! are required: client-facing (forward) and backend-facing
//! (reverse).
//!
//! Native-XDP attach is the default; on failure logs a structured
//! `xdp.attach.fallback_generic` warning and falls back to
//! `XDP_SKB` per US-01 / S-2.2-02. The fallback shape applies to
//! both XDP attach call sites.
//!
//! See test-scenarios.md S-2.2-01..03 (Slice 01) and S-2.2-33
//! (Slice 09 step 09-03).

#![allow(dead_code)]

/// Marker for DELIVER — the loader scaffold is not yet wired.
pub const SCAFFOLD: bool = true;

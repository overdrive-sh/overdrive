//! `tc_reverse_nat` — kernel-side TC egress program for Phase 2.2
//! REVERSE_NAT path (US-05; ADR-0041 Q2=A locked TC egress over
//! XDP-egress).
//!
//! Lookup pipeline per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md`
//! § 10:
//!
//! 1. Sanity prologue shared with `xdp_service_map` per Q3=C.
//! 2. Parse Eth + IPv4 + TCP/UDP headers.
//! 3. Build `ReverseKey { client_ip, client_port, backend_ip,
//!    backend_port, proto }` host-order at the kernel boundary
//!    via the shared `reverse_key_from_packet` helper (architecture.md
//!    § 11 endianness lockstep).
//! 4. REVERSE_NAT_MAP lookup → `OriginalDest { vip, vip_port }`
//!    (host-order).
//! 5. Convert `OriginalDest` to wire-order via
//!    `original_dest_to_wire`.
//! 6. Rewrite source IP / source port back to the VIP, recompute
//!    checksums via `bpf_l3_csum_replace` / `bpf_l4_csum_replace`.
//! 7. Return `TC_ACT_OK`.
//!
//! Miss → `TC_ACT_OK` (pass-through, not LB traffic). Drop classes
//! routed through `DROP_COUNTER[ReverseNatMiss]` and
//! `DROP_COUNTER[MalformedHeader]`.
//!
//! **RED scaffold** — the `#[classifier]` attribute is NOT yet
//! present; DELIVER lands it per Slice 05.
//!
//! See test-scenarios.md S-2.2-15..18 (Slice 05).

#![allow(dead_code)]

// `#[classifier]` attribute will be added by DELIVER per Slice 05.

// RED scaffold marker — DELIVER's first GREEN commit MUST flip this
// to false (or remove it entirely) per the carpaccio slice plan.
pub const SCAFFOLD: bool = true;

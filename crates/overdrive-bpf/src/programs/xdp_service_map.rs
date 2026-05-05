//! `xdp_service_map` — kernel-side XDP program for Phase 2.2
//! SERVICE_MAP forward path (US-02, US-03, US-04, US-06).
//!
//! Lookup pipeline per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md`
//! § 10:
//!
//! 1. Sanity prologue (Slice 06; Q3=C inline helper) drops
//!    pathological frames before the SERVICE_MAP lookup.
//! 2. Parse Eth + IPv4 + TCP/UDP headers.
//! 3. SERVICE_MAP outer-key lookup `(ServiceVip, u16 port)` returns
//!    inner-map fd (HASH_OF_MAPS).
//! 4. MAGLEV_MAP outer-key `ServiceId` returns inner-map of
//!    `BackendId` slots; hash 5-tuple → modulo M → slot.
//! 5. BACKEND_MAP `BackendId` → `BackendEntry { ipv4, port, ...}`.
//! 6. Rewrite IP / TCP-or-UDP destination, recompute checksum via
//!    `bpf_l3_csum_replace` / `bpf_l4_csum_replace` (Q1=A).
//! 7. Return `XDP_TX`.
//!
//! Miss / non-routable / sanity-failed frames return `XDP_PASS` or
//! `XDP_DROP` per the slice's policy; `DROP_COUNTER` increments
//! per `DropClass`.
//!
//! **RED scaffold** — the program is NOT yet declared with
//! `#[xdp]`. DELIVER turns the scaffold GREEN slice by slice. Per
//! `.claude/rules/testing.md` § "RED scaffolds" the absence of the
//! attribute IS the RED signal — the loader cannot find the
//! program until the attribute lands.
//!
//! See test-scenarios.md S-2.2-04..08 (Slice 02), S-2.2-09..11
//! (Slice 03), S-2.2-12..14 (Slice 04), S-2.2-19..23 (Slice 06).

#![allow(dead_code)]

// `#[xdp]` attribute will be added by DELIVER per Slice 02. Keeping
// the function signature in shape-form so the module compiles and
// other modules can reference it once DELIVER lands the attribute.
//
// Signature mirrors the existing `xdp_pass` in `main.rs` —
// `fn(XdpContext) -> u32` — per aya's `#[xdp]` ABI.

// RED scaffold marker — DELIVER's first GREEN commit MUST flip this
// to false (or remove it entirely) per the carpaccio slice plan.
pub const SCAFFOLD: bool = true;

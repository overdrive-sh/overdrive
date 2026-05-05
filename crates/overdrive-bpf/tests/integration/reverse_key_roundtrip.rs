//! S-2.2-17 — Endianness lockstep wire/host roundtrip per
//! architecture.md § 11.
//!
//! Tags: `@US-05` `@K5` `@slice-05` `@real-io @adapter-integration`
//! `@property` `@pending`.
//! Tier: Tier 2 (`BPF_PROG_TEST_RUN` to invoke the kernel-side
//! `reverse_key_from_packet` helper).
//!
//! Sibling: userspace mod-tests proptest in
//! `crates/overdrive-dataplane/src/maps/reverse_nat_map_handle.rs`
//! covers host-order writes against host-order reads.

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc)]

/// S-2.2-17 — A synthetic packet with known wire-order bytes
/// through `reverse_key_from_packet` produces the host-order
/// `ReverseKey` that the userspace test seeded into the map.
#[test]
#[ignore = "RED scaffold S-2.2-17 — DELIVER fills the body per Slice 05"]
fn wire_order_packet_produces_host_order_reverse_key() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-17 — \
         synthesise IPv4+TCP packet with known wire-order bytes; \
         seed REVERSE_NAT_MAP with the equivalent host-order ReverseKey \
         from userspace; assert kernel-side reverse_key_from_packet \
         produces the host-order key bit-for-bit and the lookup hits"
    );
}

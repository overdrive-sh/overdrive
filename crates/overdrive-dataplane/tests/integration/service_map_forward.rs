//! S-2.2-06 — Single-VIP TCP forwarding through real veth.
//!
//! Tags: `@US-02` `@K2` `@slice-02` `@real-io @adapter-integration`
//! `@pending`.
//! Tier: Tier 3.

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc)]

/// S-2.2-06 — Ten TCP SYNs to a registered VIP all rewrite and
/// forward via veth.
#[test]
#[ignore = "RED scaffold S-2.2-06 — DELIVER fills the body per Slice 02"]
fn ten_tcp_syns_to_vip_are_rewritten_and_forwarded_via_veth() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-06 — \
         SERVICE_MAP entry maps 10.0.0.1:8080 (TCP) -> 10.1.0.5:9000; \
         10 TCP SYNs on veth0 are rewritten, valid checksums, XDP_TX, \
         observed via tcpdump on veth1"
    );
}

//! S-2.2-19, S-2.2-20, S-2.2-21 — Sanity prologue per-class drop
//! assertions.
//!
//! Tags: `@US-06` `@K6` `@slice-06` `@real-io @adapter-integration`
//! `@pending`.
//! Tier: Tier 2 (`BPF_PROG_TEST_RUN`).

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc)]

/// S-2.2-19 — Truncated IPv4 (IHL=4) drops with
/// `MalformedHeader` counter increment.
#[test]
#[ignore = "RED scaffold S-2.2-19 — DELIVER fills the body per Slice 06"]
fn truncated_ipv4_header_drops_with_malformed_header_counter() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-19 — \
         frame with IPv4 IHL=4 (would imply 16 bytes of IP header, \
         malformed); BPF_PROG_TEST_RUN returns XDP_DROP; \
         DROP_COUNTER[MalformedHeader] increments by 1; \
         SERVICE_MAP not consulted"
    );
}

/// S-2.2-20 — Pathological TCP flag combination (SYN+RST) drops.
#[test]
#[ignore = "RED scaffold S-2.2-20 — DELIVER fills the body per Slice 06"]
fn tcp_syn_plus_rst_flags_drops_with_malformed_header_counter() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-20 — \
         TCP frame with SYN+RST flags both set; BPF_PROG_TEST_RUN \
         returns XDP_DROP; DROP_COUNTER[MalformedHeader] increments by 1"
    );
}

/// S-2.2-21 — IPv6 frame falls through (NOT dropped, no counter
/// increment).
#[test]
#[ignore = "RED scaffold S-2.2-21 — DELIVER fills the body per Slice 06"]
fn ipv6_ethertype_returns_xdp_pass_no_drop_counter_increment() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-21 — \
         IPv6 frame (EtherType 0x86DD); BPF_PROG_TEST_RUN returns \
         XDP_PASS; DROP_COUNTER does NOT increment for any drop class"
    );
}

//! S-2.2-04, S-2.2-05, S-2.2-08 — `xdp_service_map_lookup`
//! PKTGEN/SETUP/CHECK triptychs.
//!
//! Tags: `@US-02` `@K2` `@slice-02` `@real-io @adapter-integration`
//! `@pending`.
//! Tier: Tier 2 (`BPF_PROG_TEST_RUN`).

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc)]

/// S-2.2-04 — SERVICE_MAP hit returns `XDP_TX` with rewritten
/// headers.
#[test]
#[ignore = "RED scaffold S-2.2-04 — DELIVER fills the body per Slice 02"]
fn service_map_hit_returns_xdp_tx_with_rewritten_headers() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-04 — \
         SETUP populates SERVICE_MAP with VIP 10.0.0.1:8080 (TCP) -> \
         backend 10.1.0.5:9000; PKTGEN builds TCP SYN; CHECK asserts \
         BPF_PROG_TEST_RUN returns XDP_TX with rewritten dest IP/port \
         and recomputed checksums"
    );
}

/// S-2.2-05 — SERVICE_MAP miss returns `XDP_PASS`, no rewrite.
#[test]
#[ignore = "RED scaffold S-2.2-05 — DELIVER fills the body per Slice 02"]
fn service_map_miss_returns_xdp_pass_no_rewrite() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-05 — \
         empty SERVICE_MAP; TCP SYN to 10.0.0.1:8080; \
         BPF_PROG_TEST_RUN returns XDP_PASS; no header rewrites"
    );
}

/// S-2.2-08 — Truncated frame returns `XDP_PASS`, no crash, no
/// SERVICE_MAP lookup.
#[test]
#[ignore = "RED scaffold S-2.2-08 — DELIVER fills the body per Slice 02"]
fn truncated_ipv4_frame_returns_xdp_pass_no_lookup_no_crash() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-08 — \
         Ethernet header + 10-byte truncated IPv4; bounds check fails; \
         BPF_PROG_TEST_RUN returns XDP_PASS; no crash; SERVICE_MAP \
         not consulted"
    );
}

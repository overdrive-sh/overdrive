//! S-2.2-15, S-2.2-18 — REVERSE_NAT_MAP real-TCP end-to-end.
//!
//! Tags: `@US-05` `@K5` `@slice-05` `@real-io @adapter-integration`
//! `@pending`.
//! Tier: Tier 3.

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc)]

/// S-2.2-15 — Real TCP connection completes through forward and
/// reverse paths.
#[test]
#[should_panic(expected = "RED scaffold")]
fn real_tcp_connection_completes_through_vip_with_payload_echo() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-15 — \
         nc -l 9000 listener on veth1's namespace; \
         nc 10.0.0.1 8080 from veth0's namespace; \
         payload echoes; nc exits 0"
    );
}

/// S-2.2-18 — Removed backend's `REVERSE_NAT` entry purged on
/// service update; no stale rewrite leak.
#[test]
#[should_panic(expected = "RED scaffold")]
fn removing_backend_purges_reverse_nat_entry_no_stale_rewrite() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-18 — \
         remove backend B1; REVERSE_NAT_MAP no longer contains \
         B1's entry; late response from B1 falls through with \
         no rewrite"
    );
}

//! S-2.2-16 — `tc_reverse_nat` PKTGEN/SETUP/CHECK triptych.
//!
//! Tags: `@US-05` `@K5` `@slice-05` `@real-io @adapter-integration`
//! `@pending`.
//! Tier: Tier 2 (`BPF_PROG_TEST_RUN` for TC programs).

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc)]

/// S-2.2-16 — `REVERSE_NAT_MAP` lookup hit rewrites source IP/port
/// back to VIP and returns `TC_ACT_OK` with valid checksums.
#[test]
#[should_panic(expected = "RED scaffold")]
fn reverse_nat_lookup_hit_rewrites_source_to_vip() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-16 — \
         SETUP populates REVERSE_NAT_MAP with key (10.1.0.5, 9000, TCP) \
         -> value (10.0.0.1, 8080); PKTGEN builds backend response; \
         CHECK asserts BPF_PROG_TEST_RUN returns TC_ACT_OK with rewritten \
         source IP/port and recomputed checksums"
    );
}

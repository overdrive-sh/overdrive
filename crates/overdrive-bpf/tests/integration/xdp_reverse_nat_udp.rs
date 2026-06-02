//! Tier 2 — `xdp_reverse_nat_lookup` UDP (proto=17) reverse-NAT
//! `BPF_PROG_TEST_RUN` triptych (udp-service-support US-03; ADR-0060
//! § Enforcement Tier 2; K3).
//!
//! **RED scaffolds** (`#[should_panic(expected = "RED scaffold")]`).
//! GREEN: DELIVER mirrors the TCP triptych at
//! `xdp_reverse_nat_redirect_neigh.rs` (PKTGEN `synthesise_backend_
//! response`, the `bpf_prog_test_run` helper, the header-rewrite
//! assertion on `data_out`) with the IPv4 proto byte = 17 (UDP) and a
//! UDP header in place of TCP, drops `#[should_panic]`, and fills the
//! real PKTGEN/SETUP/CHECK bodies.
//!
//! Scenario SSOT:
//! `docs/feature/udp-service-support/distill/test-scenarios.md`
//! - S-03-E: a populated REVERSE_NAT_MAP (ip,port,udp)→vip rewrites a
//!   proto=17 response's source 5-tuple to the VIP.
//! - S-03-F: a REVERSE_NAT_MAP miss for a udp packet returns XDP_PASS
//!   with the frame byte-identical (no rewrite, no DROP_COUNTER slot).
//!
//! Tier 2 (layer 3) — example-only per Mandate 11; no PBT machinery.
//! Linux-only — `BPF_PROG_TEST_RUN` is a Linux syscall (the whole
//! `tests/integration` binary is gated behind `integration-tests` in
//! `integration.rs`).

// S-03-E — SETUP: populate REVERSE_NAT_MAP with (backend_ip,
// backend_port, udp) -> vip. PKTGEN: a UDP response (IPv4 proto=17)
// sourced from (backend_ip, backend_port). CHECK: after
// BPF_PROG_TEST_RUN, data_out's source 5-tuple is rewritten to
// (vip, vip_port); verdict is the reverse-NAT egress verdict.
#[test]
#[should_panic(expected = "RED scaffold")]
fn udp_response_source_rewritten_to_vip_on_reverse_nat_hit() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03-E / xdp_reverse_nat_lookup \
         rewrites a proto=17 UDP response source to the VIP on a REVERSE_NAT_MAP hit)"
    );
}

// S-03-F — boundary: an empty REVERSE_NAT_MAP + a proto=17 UDP response.
// CHECK: verdict is XDP_PASS and data_out is byte-identical to data_in
// (no rewrite, no DROP_COUNTER slot consumed).
#[test]
#[should_panic(expected = "RED scaffold")]
fn udp_response_passes_unmodified_on_reverse_nat_miss() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03-F / a proto=17 UDP response with \
         no REVERSE_NAT_MAP entry returns XDP_PASS, frame byte-identical, no drop slot)"
    );
}

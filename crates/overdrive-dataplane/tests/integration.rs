//! Integration-test entrypoint per `.claude/rules/testing.md` § Layout.
//!
//! Phase 2.1 step 01-03 wires the first scenario:
//! `build_rs_artifact_check` — asserts the `build.rs` artifact-check
//! diagnostic shape on Linux. Tier 3 smoke for the full
//! `EbpfDataplane` (load → attach → counter > 0 → detach) lives in
//! `cargo xtask integration-test vm latest` (step 03-02), not here.
//!
//! Submodules MUST be declared inside the inline `mod integration { … }`
//! block — Cargo treats each `tests/*.rs` file as a crate root, so a
//! bare `mod foo;` resolves to `tests/foo.rs`, not
//! `tests/integration/foo.rs`. The inline wrapper shifts the lookup
//! base into the subdirectory. See `testing.md` § Layout.

#![cfg(feature = "integration-tests")]

mod integration {
    /// phase-2-xdp-service-map Slice 03 (US-03; S-2.2-09..11;
    /// ASR-2.2-01) — atomic HASH_OF_MAPS swap zero-drop test.
    /// RED scaffold.
    mod atomic_swap;
    mod build_rs_artifact_check;
    /// public-ingress-gateway (S-PIG-67) — `GatewayConnectDataplane` Host and Sim
    /// agree through the port over the same `update_service` and socket call
    /// sequences (identity readable only after commit, immutable per id, no
    /// residue, a rejected conflicting backend set).
    mod gateway_connect_dataplane_equivalence;
    /// public-ingress-gateway (G1, ADR-0239/0244/0246/0247; S-PIG-105..118, 123) —
    /// the `HostGatewayRelay` adapter contract on a real kernel: real rustls/kTLS
    /// peers, `/proc/net/tls_stat` and descriptor oracles. Native metal only.
    mod gateway_relay;
    /// public-ingress-gateway (S-PIG-104) — `HostGatewayRelay` and `SimGatewayRelay`
    /// agree through the `GatewayRelay` port over the faults both can produce.
    mod gateway_relay_equivalence;
    /// Shared fixtures (RAII veth-pair, capability gating). Declared at
    /// module scope so siblings reach it via `super::helpers::…`.
    mod helpers;
    /// phase-2-xdp-service-map Slice 04 (US-04; S-2.2-15) —
    /// Maglev real-distribution under XDP traffic on real veth.
    mod maglev_real;
    /// transparent-mtls-host-socket (ADR-0069, GH #26; step 04-01, F1/F4/F5/F6/F7) —
    /// the guardrails AT: fail-closed cause-distinct (inbound nocert/wrongca DISTINCT
    /// reasons before any splice, server gets 0 bytes), the F4/F7 limits at their
    /// CONCRETE values (256 KiB+1 → BufferLimitExceeded; 5 s → HandshakeTimeout; the
    /// 129th concurrent pre-arm → InFlightLimitExceeded), F6 pump supervision
    /// (Stalled → worker teardown → Gone, no leak), the F5 intercept-exemption
    /// negatives (agent dial not re-intercepted; workload cannot self-exempt), and
    /// the honest v1 authn boundary (chain-to-bundle ONLY; the wrong-but-valid-peer
    /// PeerIdentityMismatch case is #[ignore]-gated on #242). Drives the
    /// `MtlsEnforcement` driving port; observables are REAL kernel/subprocess (0
    /// cleartext bytes at the server via a real capture, the distinct reason strings,
    /// the concrete limit values, real teardown → Gone).
    mod mtls_guardrails;
    /// transparent-mtls-host-socket (ADR-0069, GH #26; step 03-01, F3/F5/SD-1/SD-2) —
    /// INBOUND-isolated per-direction wire/syscall observables: the deliver
    /// `splice(legC → legS)` zero-copy out of leg C's kTLS-RX (the request-carrying
    /// INBOUND primary) + the orig-dst → server-SVID selection via the identity port.
    /// Drives `HostMtlsEnforcement::enforce(Inbound)` and asserts kTLS-RX armed
    /// (`ss -tie rxconf:sw`), byte-exact plaintext at the server S, client-leg
    /// 0x17-only (cleartext-hits=0), and splice-only deliver (strace), through the
    /// `MtlsEnforcement` driving port.
    /// GH #295 R5-54 Tier-3 half (LEGS-DIAL, D-14): the real
    /// `HostMtlsEnforcement`'s inbound path to a guest-prefix target —
    /// handshake, awaited dial, relay with the 0.5-RTT plaintext first, no
    /// dial for a client failing peer verification, a dial error resetting
    /// leg C, leg S ended only by the `GuestStream`'s drop, and `teardown`
    /// awaiting the relay.
    mod mtls_inbound_guest_relay;
    /// udp-service-support US-05 / S-05-A..C (ADR-0060 Tier 3; K4) —
    /// multi-listener (TCP + UDP) forward+reverse e2e. RED scaffolds.
    mod multi_listener_tcp_udp_e2e;
    /// reconciler-convergence-guarantees (D-20, ADR-0299) — `ProgrammedServiceView`
    /// Host and Sim agree through the port over one `update_service` sequence, and a
    /// restarted host dataplane reads no frontend.
    mod programmed_service_view_equivalence;
    /// public-ingress-gateway (S-PIG-41, 56 T3, 57 T3, 63, 71 instance half, 72) — the
    /// gateway selection hook and `GatewayConnectDataplane` on a real kernel: the
    /// unregistered-socket preservation row, receipt and applied identity, the
    /// prepared socket, instance restart, and updates racing identity reads.
    mod public_ingress_gateway_hook;
    /// phase-2-xdp-service-map Slice 09 step 09-03 (S-2.2-33;
    /// ADR-0045 § Operational) — loader attach topology under
    /// `bpf_redirect`-on-XDP datapath. Verifies dual-XDP attach
    /// on `lb_veth_a` (forward) + `lb_veth_b` (reverse) and
    /// retirement of TC-egress reverse-NAT.
    mod redirect_neigh_attach;
    /// phase-2-xdp-service-map Slice 05 (US-05; S-2.2-15, S-2.2-18) —
    /// REVERSE_NAT_MAP real-TCP `nc` end-to-end. RED scaffolds.
    mod reverse_nat_e2e;
    /// udp-service-support US-04 / S-04-A..C (ADR-0060 Tier 3; K1) —
    /// single-UDP-listener forward+reverse e2e (walking skeleton).
    /// RED scaffolds.
    mod reverse_nat_udp_e2e;
    /// reconciler-convergence-guarantees Slice 02 AC2 (ADR-0297) — the boot
    /// attaches none of the deleted same-host cgroup socket programs.
    mod same_host_programs_not_loaded_at_boot;
    /// phase-2-xdp-service-map Slice 06 (US-06; S-2.2-22) —
    /// sanity prologue mixed-batch counter assertions. RED scaffold.
    mod sanity_mixed_batch;
    /// phase-2-xdp-service-map Slice 02 (US-02; S-2.2-06) —
    /// SERVICE_MAP forward path through real veth. RED scaffold.
    mod service_map_forward;
    /// udp-service-support — regression guard: SERVICE_MAP outer slot is
    /// keyed on the declared VIP port, not the backend listener port
    /// (VIP:53 → backend:5353).
    mod service_map_vip_port;
    /// phase-2-xdp-service-map Slice 01 (US-01; S-2.2-01..03) —
    /// real-iface XDP attach. RED scaffolds.
    mod veth_attach;
}

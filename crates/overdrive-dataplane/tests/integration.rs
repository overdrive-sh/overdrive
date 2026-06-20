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
    /// unconnected-udp-sendmsg4 follow-up (GH #211; ADR-0053 rev
    /// Decisions 2 & 3 reversal) — `deregister_local_backend` retry-safety:
    /// a retry after a partial failure (forward removed, reverse survived)
    /// purges the stale reverse entry. Caller-supplied backend.
    mod deregister_retry_safety;
    /// Shared fixtures (RAII veth-pair, capability gating). Declared at
    /// module scope so siblings reach it via `super::helpers::…`.
    mod helpers;
    /// udp-service-support step 02-02 (S-02-02; ADR-0053 rev) —
    /// `LOCAL_BACKEND_MAP` keys on `(vip, vip_port, proto)`: TCP +
    /// UDP connect to the same `(vip, port)` reach proto-correct
    /// backends via the cgroup_connect4 path.
    mod local_backend_proto_connect;
    /// phase-2-xdp-service-map Slice 04 (US-04; S-2.2-15) —
    /// Maglev real-distribution under XDP traffic on real veth.
    mod maglev_real;
    /// transparent-mtls-host-socket (ADR-0069, GH #26; step 02-02, F1/F3) — the
    /// focused agent mutual-TLS HANDSHAKE-IDENTITY acceptance test (client leg B +
    /// server leg C). Drives `HostMtlsEnforcement::enforce` for BOTH directions and
    /// asserts the presented leaf chains to the root + its URI SAN is the workload
    /// SPIFFE id, from the captured handshake at the test tier.
    mod mtls_agent_handshake;
    /// transparent-mtls-host-socket (ADR-0069, GH #26; step 04-01, F1/F4/F5/F6/F7) —
    /// the guardrails AT: fail-closed cause-distinct (inbound nocert/wrongca DISTINCT
    /// reasons before any splice, server gets 0 bytes), the F4/F7 limits at their
    /// CONCRETE values (256 KiB+1 → BufferLimitExceeded; 5 s → HandshakeTimeout; the
    /// 129th concurrent pre-arm → InFlightLimitExceeded), F6 pump supervision
    /// (Stalled → worker teardown → Gone, no leak), the F5 intercept-exemption
    /// negatives (agent dial not re-intercepted; workload cannot self-exempt), and
    /// the honest v1 authn boundary (chain-to-bundle ONLY; the wrong-but-valid-peer
    /// PeerIdentityMismatch case is #[ignore]-gated on #178). Drives the
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
    mod mtls_inbound_enforce;
    /// udp-service-support US-05 / S-05-A..C (ADR-0060 Tier 3; K4) —
    /// multi-listener (TCP + UDP) forward+reverse e2e. RED scaffolds.
    mod multi_listener_tcp_udp_e2e;
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
    /// unconnected-udp-sendmsg4 Slice 03 (US-03; S-03-01..03) — GH #200,
    /// ADR-0053 rev 2026-06-05. Reply-path error hardening: sentinel-on-
    /// miss (no backend-IP leak to the app), below-floor attach refusal,
    /// fixture-collision discipline. RED scaffolds (#[should_panic]).
    mod unconnected_udp_reply_hardening;
    /// unconnected-udp-sendmsg4 Slice 01 + Slice 02 Tier-3 prong (US-01
    /// WS, US-02; S-01-01..03, S-02-03) — GH #200, ADR-0053 rev
    /// 2026-06-05. Unconnected sendto/recvfrom round-trip through the real
    /// cgroup sendmsg4+recvmsg4 hooks; recvfrom source == VIP at the app
    /// sockaddr layer; both maps present after one register. THE GATE (no
    /// Tier-2 backstop). RED scaffolds (#[should_panic]).
    mod unconnected_udp_roundtrip;
    /// phase-2-xdp-service-map Slice 01 (US-01; S-2.2-01..03) —
    /// real-iface XDP attach. RED scaffolds.
    mod veth_attach;
}

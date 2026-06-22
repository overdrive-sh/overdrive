//! S-WS — KEYSTONE WALKING SKELETON (DISTILL RED scaffold, GH #241).
//!
//! The mandatory #241 acceptance gate: a real `overdrive serve` + two
//! `overdrive deploy`-ed mesh workloads, where the client workload dials the
//! server workload at its canonical `workload_addr:service_port` **directly**
//! (no DNS), and the **production-installed** inbound nft-TPROXY rule (installed
//! by `start_alloc` from `spec.{workload_addr, service_ports}`, NOT by the test)
//! captures the dial, mTLS terminates, and the client's bytes reach the server.
//!
//! This is THE vertical-slice gate (CLAUDE.md § "Build vertical slices through
//! production entry points"): **NO test-installed `install_inbound_tproxy`, NO
//! synthetic loopback `INBOUND_VIRT_IP`/`INBOUND_VIRT_PORT`** — both are REMOVED
//! from the existing `bidirectional_walking_skeleton.rs` (which this file
//! replaces in #241; DELIVER folds the synthetic-virt skeleton into this
//! canonical-address one). No integration test installs the inbound rule,
//! supplies the address, or stands in for the production call site.
//!
//! MERGE-BLOCKING on the **pinned-6.18 appliance-kernel Tier-3 matrix**
//! (ADR-0068, DELIVER obligation #3): the spike verdicts are dev-Lima 7.0, which
//! is necessary-but-not-sufficient. The DELIVER roadmap AC must name the
//! pinned-6.18 matrix, not merely "tests pass."
//!
//! `E`-surface note (`.claude/rules/verification.md`): S-WS is the
//! operator-observable end-to-end expectation that should GRADUATE into
//! `verification/expectations/` at DELIVER/DEVOPS. Do NOT build the catalogue
//! entry in DISTILL.
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-WS.
//!
//! DELIVER replaces the panic body with the real assertions: spawn real
//! `overdrive serve`, `overdrive deploy` the server + client mesh workloads,
//! drive the client's direct canonical-address dial, and assert the
//! production-installed capture diverts it to the transparent listener, mTLS
//! terminates, and the application round-trip completes byte-exact. Requires
//! root + `CAP_NET_ADMIN`/`CAP_SYS_ADMIN`; a non-root run SKIPs; `uname -r` recorded.

#[test]
#[should_panic(expected = "RED scaffold")]
fn workload_reached_at_canonical_address_terminates_mtls_end_to_end() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WS / real serve+deploy: \
         client dials server workload_addr:service_port directly, \
         production-installed inbound nft-TPROXY captures it (no test-installed \
         rule, no synthetic virt), mTLS terminates, bytes round-trip; \
         MERGE-BLOCKING on the pinned-6.18 Tier-3 matrix)"
    );
}

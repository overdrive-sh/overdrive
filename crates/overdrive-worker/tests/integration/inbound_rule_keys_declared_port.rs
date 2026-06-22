//! S-DPORT — the inbound capture rule keys on the DECLARED service port, not the
//! ephemeral leg-C port (DISTILL RED scaffold, GH #241, Tier-3, error/edge).
//!
//! D-BLOCKER1 / D-TME-10 one-source/two-readers: the installed nft rule's match
//! `dport` is the **declared Service listener port** — the same value
//! `service_backends` advertises and the egress `MtlsResolve` keys on — NOT the
//! ephemeral `leg_c_addr.port()` (the inert self-referential shape the design
//! rejected: a rule matching the agent's own leg-C port, which no real inbound
//! connection targets). The rule's `tproxy to` TARGET is the ephemeral leg-C
//! port (the redirect destination), but the match KEY is the declared port. A
//! dial to `workload_addr:service_port` is captured.
//!
//! Error/edge guard: pins the negative — a mutant that keys the rule on
//! `leg_c_addr.port()` passes a naive "a rule was installed" check but fails
//! this scenario.
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-DPORT.
//!
//! DELIVER replaces the panic body: deploy a 1-listener Service, observe the
//! installed rule's match dport == the declared service port (and != the
//! ephemeral transparent-listener port), and confirm a dial to
//! `workload_addr:service_port` is captured. Requires root; non-root SKIPs.

#[test]
#[should_panic(expected = "RED scaffold")]
fn inbound_capture_rule_matches_declared_service_port_not_ephemeral_leg_c_port() {
    panic!(
        "Not yet implemented -- RED scaffold (S-DPORT / the inbound rule's match \
         dport is the DECLARED service port per D-BLOCKER1 one-source/two-readers, \
         NOT the ephemeral leg_c_addr.port(); a peer dial to \
         workload_addr:service_port is captured)"
    );
}

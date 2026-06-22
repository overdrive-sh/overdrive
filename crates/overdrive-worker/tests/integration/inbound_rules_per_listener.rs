//! S-NRULES — N listeners install exactly N inbound rules (DISTILL RED scaffold,
//! GH #241, Tier-3 real nft observation).
//!
//! A Service with N=2 declared listeners → `start_alloc` installs exactly 2
//! inbound capture rules, each keyed `ip daddr <workload_addr> tcp dport
//! <port_i>` per listener; 2 RAII guards retained, both released on alloc
//! teardown (no leftover nft state). Maps D-A1 (N listeners → N rules via the
//! per-port `install_inbound_tproxy` loop).
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-NRULES.
//!
//! DELIVER replaces the panic body: deploy a 2-listener Service through the
//! production path, dump the live `overdrive-mtls` nft ruleset, assert exactly 2
//! per-virt rules keyed on the `workload_addr` + each declared port, and assert
//! both rules are gone after teardown. The rule install MUST be the production
//! `start_alloc` call site, never a test-installed `install_inbound_tproxy`.
//! Requires root + `CAP_NET_ADMIN`/`CAP_SYS_ADMIN`; a non-root run SKIPs.

#[test]
#[should_panic(expected = "RED scaffold")]
fn two_declared_listeners_install_exactly_two_inbound_capture_rules() {
    panic!(
        "Not yet implemented -- RED scaffold (S-NRULES / a 2-listener Service \
         installs exactly 2 inbound rules keyed ip daddr <workload_addr> tcp \
         dport <port_i> per listener; 2 RAII guards released on teardown)"
    );
}

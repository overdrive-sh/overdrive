//! S-DPORT — the inbound capture rule keys on the DECLARED service port, not the
//! ephemeral leg-C port (GH #241, step 03-01, Tier-3, error/edge).
//!
//! D-BLOCKER1 / D-TME-10 one-source/two-readers: the installed nft rule's match
//! `dport` is the **declared Service listener port** — the same value
//! `service_backends` advertises and the egress `MtlsResolve` keys on — NOT the
//! ephemeral `leg_c_addr.port()` (the inert self-referential shape the design
//! rejected: a rule matching the agent's own leg-C port, which no real inbound
//! connection targets). The rule's `tproxy to` TARGET is the ephemeral leg-C
//! port (the redirect destination), but the match KEY is the declared port.
//!
//! Error/edge guard (the NEGATIVE pin): a mutant that keys the rule's match on
//! `leg_c_addr.port()` passes a naive "a rule was installed" check but fails
//! this scenario — the match dport would no longer equal the declared service
//! port, and the inert self-referential `match == tproxy-target` shape would be
//! present (this test asserts they DIFFER).
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-DPORT.
//!
//! Litmus: revert `start_alloc` to `tproxy_guard = None` → the rule is absent
//! → RED. Key the match on `leg_c_addr.port()` instead of the declared port →
//! `count_inbound_rules(declared_port)` is 0 → RED. The install MUST be the
//! production `start_alloc` call site, never a test-installed
//! `install_inbound_tproxy`.
//!
//! Requires root; non-root SKIPs. Run via `cargo xtask lima run -- cargo nextest
//! run -p overdrive-worker --features integration-tests`. NEVER `--no-run`.

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    reason = "Test body; skip messages + evidence go to stderr; failures must panic with informative messages"
)]

use std::net::Ipv4Addr;
use std::num::NonZeroU16;

use overdrive_core::AllocationId;

use super::inbound_tproxy_harness::{
    KernelStateLock, build_inbound_spec, build_worker, clean_shared_infra, count_inbound_rules,
    inbound_rule_tproxy_target_port, is_root, nft_list_chain, record_uname,
};

/// The canonical per-workload address the Service was provisioned into.
const WORKLOAD_ADDR: &str = "10.99.1.2";
/// The single declared Service listener port — the match KEY the rule must use.
const SERVICE_PORT: u16 = 18777;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inbound_capture_rule_matches_declared_service_port_not_ephemeral_leg_c_port() {
    if !is_root() {
        eprintln!(
            "SKIP inbound_capture_rule_matches_declared_service_port_not_ephemeral_leg_c_port: \
             not root"
        );
        return;
    }
    record_uname("03-01-dport");
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();

    let workload_addr: Ipv4Addr = WORKLOAD_ADDR.parse().expect("workload addr");
    let ports = vec![NonZeroU16::new(SERVICE_PORT).expect("service port")];

    let worker = build_worker();
    let alloc = AllocationId::new("alloc-sa-0301-dport").expect("valid alloc id");
    let spec = build_inbound_spec(&alloc, Some(workload_addr), ports);

    // PORT-TO-PORT: drive production `start_alloc`. The fixture supplied the
    // declared service port; the install must key the rule's match on it.
    worker
        .start_alloc(&spec)
        .expect("start_alloc must install the inbound rule keyed on the declared service port");

    let dump = nft_list_chain()
        .expect("start_alloc must have ensured the shared overdrive-mtls prerouting chain");

    // (1) The rule's MATCH dport is the DECLARED service port. A mutant keying on
    // `leg_c_addr.port()` (ephemeral) makes this 0 → RED.
    let matched = count_inbound_rules(&dump, workload_addr, SERVICE_PORT);
    assert_eq!(
        matched, 1,
        "S-DPORT: the inbound rule's match dport must be the DECLARED service port \
         {SERVICE_PORT} (D-BLOCKER1 one-source/two-readers), got {matched} rules keyed on it:\n\
         {dump}"
    );

    // (2) The rule's `tproxy to` TARGET is the ephemeral leg-C port — the
    // REDIRECT DESTINATION, distinct from the match key. Pin that it is NOT the
    // declared service port: the inert self-referential `match == target` shape
    // the design rejected (a rule that tproxy-redirects daddr:service_port to
    // 127.0.0.1:service_port would be a self-loop) must be structurally absent.
    let target_port = inbound_rule_tproxy_target_port(&dump, workload_addr, SERVICE_PORT)
        .expect("the matched inbound rule must carry a `tproxy to 127.0.0.1:<port>` target");
    assert_ne!(
        target_port, SERVICE_PORT,
        "S-DPORT: the `tproxy to` TARGET must be the ephemeral leg-C port (the redirect \
         destination), NOT the declared service port {SERVICE_PORT} — a rule whose match dport \
         equals its tproxy-target is the inert self-referential shape the design rejected; \
         got target 127.0.0.1:{target_port}:\n{dump}"
    );
    eprintln!(
        "[03-01-dport] match dport={SERVICE_PORT} (declared), tproxy-to=127.0.0.1:{target_port} \
         (ephemeral leg-C) — distinct, as required"
    );

    worker.stop_alloc(&alloc);
    let dump_after_stop = nft_list_chain().expect("shared chain survives stop_alloc");
    assert_eq!(
        count_inbound_rules(&dump_after_stop, workload_addr, SERVICE_PORT),
        0,
        "S-DPORT: the inbound rule must be released on teardown (RAII guard dropped):\n\
         {dump_after_stop}"
    );

    clean_shared_infra();
}

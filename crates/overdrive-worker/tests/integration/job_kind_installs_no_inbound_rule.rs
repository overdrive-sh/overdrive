//! S-JOB0 — a Job-kind alloc (0 listeners) installs 0 inbound rules (DISTILL RED
//! scaffold, GH #241, Tier-3, error/edge).
//!
//! `project_service_listen_ports` returns `Vec::new()` for `Job`/`Schedule`
//! (mirroring `project_probe_descriptors`), so a Job alloc carries empty
//! `service_ports` / `None` `workload_addr` → `start_alloc` installs ZERO inbound
//! capture rules (the host-netns/Job path, unchanged) and retains no
//! `TproxyInterceptGuard`. No spurious capture diverts unrelated traffic.
//!
//! Error/edge guard: a mutant that installs an all-TCP or hardcoded-port rule
//! for a Job fails this scenario.
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-JOB0.
//!
//! DELIVER replaces the panic body: deploy a Job-kind workload (no listeners),
//! dump the live nft ruleset, assert zero per-virt capture rules for the alloc.
//! Requires root; non-root SKIPs.

#[test]
#[should_panic(expected = "RED scaffold")]
fn job_kind_workload_with_no_listeners_installs_no_inbound_capture_rule() {
    panic!(
        "Not yet implemented -- RED scaffold (S-JOB0 / a Job-kind alloc has \
         empty service_ports -> start_alloc installs 0 inbound rules, no \
         spurious capture, no TproxyInterceptGuard retained)"
    );
}

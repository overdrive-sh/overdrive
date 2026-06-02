//! Tier-3 idempotent-provision scenarios for the single-node veth
//! provisioner (step 01-02 of `single-node-dataplane-wiring`, ADR-0061
//! § 3.1).
//!
//! Drives the real `ip(8)` shell-out through
//! [`overdrive_control_plane::veth_provisioner::provision`] against the
//! host netns. `integration-tests`-gated (real network I/O, needs
//! `CAP_NET_ADMIN`) and `#[cfg(target_os = "linux")]`. The unprivileged
//! Lima `lima` user lacks `CAP_NET_ADMIN`; the canonical inner-loop path
//! is `cargo xtask lima run --` (runs as root). When `ip` returns
//! EPERM the test SKIPS rather than fails.
//!
//! Cleanup: each test deletes its veth pair on entry and exit. A stale
//! `ovd-veth-*` from a crashed prior run would otherwise poison the
//! "adopt pre-existing" assertion (per `.claude/rules/debugging.md`
//! § "Leftover XDP attachments" / veth-sweep discipline).

#![cfg(target_os = "linux")]
// Skip-on-no-privilege messages are the legitimate way these Tier-3
// tests communicate "CAP_NET_ADMIN absent, scenario skipped" on an
// unprivileged runner — `eprintln!` to the test log is exactly right.
#![allow(clippy::print_stderr)]

use ipnet::Ipv4Net;
use overdrive_control_plane::veth_provisioner::{
    VethProvisionError, VethProvisionPlan, derive_veth_plan, provision,
};
use std::process::Command;

/// Per-test iface names — suffixed with the PID (so two parallel test
/// binaries do not collide on the global host-netns iface namespace)
/// AND a per-test `tag` (so the two scenarios in this file, which run in
/// the same binary, do not share a veth pair or a global route). Linux
/// IFNAMSIZ = 16 (15 usable); `vcli`/`vbk` + 1 tag + 4 hex ≤ 12 chars.
fn iface_names(tag: char) -> (String, String) {
    let suffix = std::process::id() & 0xffff;
    (format!("vcli{tag}{suffix:04x}"), format!("vbk{tag}{suffix:04x}"))
}

fn delete_pair(client: &str) {
    // Deleting one end reaps both ends of a veth pair.
    let _ = Command::new("ip").args(["link", "del", client]).output();
}

fn link_present(iface: &str) -> bool {
    Command::new("ip").args(["link", "show", iface]).output().is_ok_and(|o| o.status.success())
}

/// Returns `true` if the provision skipped due to missing `CAP_NET_ADMIN`
/// (so the test can bail with a skip rather than fail on an unprivileged
/// runner). Distinguishes the EPERM "no privilege" shape from a genuine
/// provisioning bug.
fn is_cap_skip(err: &VethProvisionError) -> bool {
    let msg = err.to_string();
    msg.contains("Operation not permitted") || msg.contains("Permission denied")
}

fn plan_for(client: &str, backend: &str, cidr: &str) -> VethProvisionPlan {
    let range: Ipv4Net = cidr.parse().expect("valid /24");
    derive_veth_plan(client, backend, range)
}

/// `provision` CREATES the veth pair when it is ABSENT — after a clean
/// provision both ends of the pair exist in the host netns.
#[test]
fn provision_creates_pair_when_absent() {
    let (client, backend) = iface_names('c');
    delete_pair(&client);
    assert!(!link_present(&client), "precondition: pair must be absent");

    let plan = plan_for(&client, &backend, "10.96.0.0/24");
    match provision(&plan) {
        Ok(()) => {
            assert!(link_present(&client), "client veth must exist after provision");
            assert!(link_present(&backend), "backend veth peer must exist after provision");
            delete_pair(&client);
        }
        Err(err) if is_cap_skip(&err) => {
            eprintln!("SKIP provision_creates_pair_when_absent: CAP_NET_ADMIN required ({err})");
        }
        Err(err) => panic!("provision failed for a non-privilege reason: {err}"),
    }
}

/// `provision` ADOPTS a pre-existing pair WITHOUT recreating it —
/// idempotent detect-and-reuse per ADR-0061 § 3.1. A second provision
/// against an already-present pair returns Ok and leaves the same pair
/// in place (the link is still present; no EEXIST failure).
#[test]
fn provision_adopts_preexisting_pair_without_recreating() {
    let (client, backend) = iface_names('a');
    delete_pair(&client);

    let plan = plan_for(&client, &backend, "10.97.0.0/24");
    // First provision creates the pair (or skips on no privilege).
    match provision(&plan) {
        Ok(()) => {}
        Err(err) if is_cap_skip(&err) => {
            eprintln!(
                "SKIP provision_adopts_preexisting_pair_without_recreating: CAP_NET_ADMIN required ({err})"
            );
            return;
        }
        Err(err) => panic!("first provision failed: {err}"),
    }
    assert!(link_present(&client), "pair must exist after first provision");

    // Second provision must ADOPT the existing pair: Ok, no recreate,
    // no EEXIST. The link is still present afterwards.
    provision(&plan).expect("second provision must adopt the pre-existing pair without error");
    assert!(link_present(&client), "pair must still exist after adopt");

    delete_pair(&client);
}

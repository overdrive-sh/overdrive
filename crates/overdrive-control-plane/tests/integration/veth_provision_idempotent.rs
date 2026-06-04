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
use overdrive_control_plane::iface::resolve_iface_ipv4;
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

/// `provision` over an ALREADY-COMPLETE pair converges to a silent
/// idempotent success — the pair is still present and still resolves its
/// gateway IPv4, and no step errors (the route `File exists` collision is
/// swallowed). Guards against the converge falsely erroring on, or
/// destructively re-doing work over, a good pair (ADR-0061 § 3.1).
#[test]
fn provision_complete_pair_converges_to_silent_success() {
    let (client, backend) = iface_names('a');
    delete_pair(&client);

    let plan = plan_for(&client, &backend, "10.97.0.0/24");
    // First provision creates + fully converges the pair (or skips).
    match provision(&plan) {
        Ok(()) => {}
        Err(err) if is_cap_skip(&err) => {
            eprintln!(
                "SKIP provision_complete_pair_converges_to_silent_success: CAP_NET_ADMIN required ({err})"
            );
            return;
        }
        Err(err) => panic!("first provision failed: {err}"),
    }
    assert!(link_present(&client), "pair must exist after first provision");

    // Second provision over the now-complete pair must be an all-noop
    // converge: Ok, no error, pair still present, gateway still resolves.
    provision(&plan).expect("second provision over a complete pair must converge silently");
    assert!(link_present(&client), "pair must still exist after re-converge");
    assert_eq!(
        resolve_iface_ipv4(&client).expect("client iface must still resolve its gateway IPv4"),
        plan.client_gateway,
        "re-converge must not disturb the assigned client gateway address",
    );

    delete_pair(&client);
}

/// REGRESSION (the bug this fix closes): a HALF-PROVISIONED pair — both
/// ends created by `ip link add` but NO address, NO up, NO route (a serve
/// boot that crashed mid-provision) — must be COMPLETED in place by
/// `provision`, so the client iface afterwards resolves its gateway IPv4.
/// The old adopt-untouched branch returned `Ok(())` here and left the
/// pair address-less, surfacing two layers downstream as an
/// `IfaceAddrResolution` error.
#[test]
fn provision_completes_half_provisioned_pair() {
    let (client, backend) = iface_names('h');
    delete_pair(&client);

    // Construct the partial state directly: create the pair and STOP —
    // no addr, no up, no route. This is exactly the crash-mid-provision
    // shape (atomic `ip link add` ran; nothing after it did).
    let add = Command::new("ip")
        .args(["link", "add", &client, "type", "veth", "peer", "name", &backend])
        .output()
        .expect("spawn ip link add");
    if !add.status.success() {
        let stderr = String::from_utf8_lossy(&add.stderr);
        if stderr.contains("Operation not permitted") || stderr.contains("Permission denied") {
            eprintln!("SKIP provision_completes_half_provisioned_pair: CAP_NET_ADMIN required");
            return;
        }
        panic!("could not construct half-provisioned pair: {stderr}");
    }
    assert!(link_present(&client), "precondition: half-provisioned pair created");

    let plan = plan_for(&client, &backend, "10.98.0.0/24");
    match provision(&plan) {
        Ok(()) => {
            // The regression assertion: the address the old path skipped
            // is now assigned, so the iface resolves its gateway IPv4 —
            // NOT the IfaceAddrResolution error the bug produced.
            let resolved = resolve_iface_ipv4(&client)
                .expect("converge must complete the half-provisioned pair's client address");
            assert_eq!(
                resolved, plan.client_gateway,
                "converge must assign the derived client gateway to the half-provisioned iface",
            );
            delete_pair(&client);
        }
        Err(err) if is_cap_skip(&err) => {
            eprintln!(
                "SKIP provision_completes_half_provisioned_pair: CAP_NET_ADMIN required ({err})"
            );
            delete_pair(&client);
        }
        Err(err) => {
            delete_pair(&client);
            panic!("provision failed to complete half-provisioned pair: {err}");
        }
    }
}

/// § 3.2 corrupted edge: client iface present but its declared peer
/// ABSENT (the peer was separately deleted). `provision` must RECREATE
/// the pair from scratch and converge it — afterwards both ends exist and
/// the client resolves its gateway IPv4.
#[test]
fn provision_recreates_pair_when_peer_absent() {
    let (client, backend) = iface_names('p');
    delete_pair(&client);

    let plan = plan_for(&client, &backend, "10.99.0.0/24");
    // Bring up a complete pair first (or skip on no privilege).
    match provision(&plan) {
        Ok(()) => {}
        Err(err) if is_cap_skip(&err) => {
            eprintln!(
                "SKIP provision_recreates_pair_when_peer_absent: CAP_NET_ADMIN required ({err})"
            );
            return;
        }
        Err(err) => panic!("initial provision failed: {err}"),
    }
    // Corrupt it into the § 3.2 shape "client present, declared peer
    // absent". Deleting a veth end reaps BOTH ends, so we cannot just
    // `ip link del <backend>`. Instead we move the peer into a throwaway
    // network namespace: the peer leaves the host netns (so the declared
    // `<backend>` name is absent in the host netns the provisioner reads)
    // while the client end stays present in the host netns. This is the
    // exact "peer separately moved to another netns" reachable shape
    // ADR-0061 § 3.2 names.
    let stash_ns = format!("ovd-stash-{}", std::process::id() & 0xffff);
    let _ = Command::new("ip").args(["netns", "add", &stash_ns]).output();
    let moved = Command::new("ip").args(["link", "set", &backend, "netns", &stash_ns]).output();
    let corrupted = link_present(&client) && !link_present(&backend);
    if !corrupted {
        // Could not construct the corrupted shape on this kernel/runner —
        // skip rather than assert a precondition we cannot establish.
        eprintln!(
            "SKIP provision_recreates_pair_when_peer_absent: could not move peer to netns ({moved:?})"
        );
        let _ = Command::new("ip").args(["netns", "del", &stash_ns]).output();
        delete_pair(&client);
        return;
    }

    // Converge: must recreate the pair and fully provision it.
    provision(&plan).expect("provision must recreate a corrupted client-present/peer-absent pair");
    assert!(link_present(&client), "client end must exist after recreate");
    assert!(link_present(&backend), "peer end must exist after recreate");
    assert_eq!(
        resolve_iface_ipv4(&client).expect("recreated client must resolve its gateway IPv4"),
        plan.client_gateway,
    );

    delete_pair(&client);
    // The stashed orphan peer dies with its netns.
    let _ = Command::new("ip").args(["netns", "del", &stash_ns]).output();
}

/// REGRESSION (inverse corrupted edge): the declared client iface ABSENT
/// but its declared peer (backend) PRESENT in the host netns — e.g. the
/// client end was separately moved/renamed, or an unrelated interface
/// collides on the backend name. The old `(false, _)` wildcard routed
/// this to `CreatePair`, whose `ip link add <client> type veth peer name
/// <backend>` then failed with "File exists" on the surviving peer →
/// boot refusal. `provision` must now RECREATE — dropping the surviving
/// peer (`RecreatePair` dels BOTH ends) before recreate — so afterwards
/// both ends exist and the client resolves its gateway IPv4.
#[test]
fn provision_recreates_pair_when_client_absent_but_peer_present() {
    let (client, backend) = iface_names('i');
    delete_pair(&client);

    let plan = plan_for(&client, &backend, "10.100.0.0/24");
    // Bring up a complete pair first (or skip on no privilege).
    match provision(&plan) {
        Ok(()) => {}
        Err(err) if is_cap_skip(&err) => {
            eprintln!(
                "SKIP provision_recreates_pair_when_client_absent_but_peer_present: CAP_NET_ADMIN required ({err})"
            );
            return;
        }
        Err(err) => panic!("initial provision failed: {err}"),
    }
    // Corrupt it into the inverse shape "client absent, declared peer
    // present" by moving the CLIENT end into a throwaway netns: the client
    // leaves the host netns (so `<client>` is absent in the host netns the
    // provisioner reads) while the backend peer stays present. This is the
    // exact "client separately moved" reachable shape the inverse edge of
    // ADR-0061 § 3.2 names — and the surviving `<backend>` is the iface the
    // bare CreatePair would have collided with on "File exists".
    let stash_ns = format!("ovd-stshi-{}", std::process::id() & 0xffff);
    let _ = Command::new("ip").args(["netns", "add", &stash_ns]).output();
    let moved = Command::new("ip").args(["link", "set", &client, "netns", &stash_ns]).output();
    let corrupted = !link_present(&client) && link_present(&backend);
    if !corrupted {
        eprintln!(
            "SKIP provision_recreates_pair_when_client_absent_but_peer_present: could not move client to netns ({moved:?})"
        );
        let _ = Command::new("ip").args(["netns", "del", &stash_ns]).output();
        delete_pair(&backend);
        return;
    }

    // Converge: must recreate the pair (reaping the surviving peer) and
    // fully provision it — NOT fail with "File exists" on link_add.
    provision(&plan).expect("provision must recreate a corrupted client-absent/peer-present pair");
    assert!(link_present(&client), "client end must exist after recreate");
    assert!(link_present(&backend), "peer end must exist after recreate");
    assert_eq!(
        resolve_iface_ipv4(&client).expect("recreated client must resolve its gateway IPv4"),
        plan.client_gateway,
    );

    delete_pair(&client);
    // The stashed orphan client dies with its netns.
    let _ = Command::new("ip").args(["netns", "del", &stash_ns]).output();
}

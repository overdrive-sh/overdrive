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

/// Read `ethtool -k <iface>` and return whether `tx-checksumming` is ON.
/// `None` when `ethtool` is unavailable or the iface does not report the
/// feature line — the caller treats `None` as "cannot assert" (skip the
/// offload assertion) rather than a failure, since some kernels/runners
/// lack `ethtool` or the feature on a veth.
fn tx_checksumming_on(iface: &str) -> Option<bool> {
    let out = Command::new("ethtool").args(["-k", iface]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("tx-checksumming:")?;
        Some(rest.split_whitespace().next() == Some("on"))
    })
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

/// Production-correctness regression for commit 62fa6be2 (incremental L4
/// checksum): after `provision`, BOTH ends of the LB veth pair must have
/// TX-checksum-offload DISABLED (`ethtool -k <iface>` reports
/// `tx-checksumming: off`). These are the SAME ifaces
/// `EbpfDataplane::new_with_pin_dir` attaches `xdp_service_map_lookup` /
/// `xdp_reverse_nat_lookup` to, so disabling offload here gives the
/// receive-side XDP NAT hook a FULL (not `CHECKSUM_PARTIAL`) ingress
/// checksum as the base for its incremental delta. Without it, every
/// NAT'd packet's checksum is corrupted.
///
/// Also asserts the converge-on-boot IDEMPOTENCY guarantee (ADR-0061
/// § 3.1): a SECOND `provision` over the now-offload-off pair re-observes
/// offload off, emits no disable step, and converges to a silent success
/// — offload stays off, no error.
#[test]
fn provision_disables_tx_offload_on_both_ends_and_is_idempotent() {
    let (client, backend) = iface_names('t');
    delete_pair(&client);

    let plan = plan_for(&client, &backend, "10.101.0.0/24");
    match provision(&plan) {
        Ok(()) => {}
        Err(err) if is_cap_skip(&err) => {
            eprintln!(
                "SKIP provision_disables_tx_offload_on_both_ends_and_is_idempotent: CAP_NET_ADMIN required ({err})"
            );
            return;
        }
        Err(err) => panic!("first provision failed: {err}"),
    }
    assert!(link_present(&client), "pair must exist after first provision");

    // After provision, both ends must report offload OFF. When `ethtool`
    // or the feature is unavailable on this runner, `None` → skip the
    // offload assertion (but still exercise the idempotency path below).
    let client_offload = tx_checksumming_on(&client);
    let backend_offload = tx_checksumming_on(&backend);
    if let Some(client_on) = client_offload {
        assert!(!client_on, "client veth must have tx-checksumming OFF after provision");
    }
    if let Some(backend_on) = backend_offload {
        assert!(!backend_on, "backend veth must have tx-checksumming OFF after provision");
    }
    if client_offload.is_none() && backend_offload.is_none() {
        eprintln!(
            "NOTE provision_disables_tx_offload_on_both_ends_and_is_idempotent: ethtool/tx-checksumming unavailable on this runner; offload state not asserted (idempotency still exercised)"
        );
    }

    // Second provision over the offload-off pair must converge silently —
    // re-observe offload off, emit no disable, no error.
    provision(&plan).expect("second provision over an offload-off pair must converge silently");
    if let Some(client_on) = tx_checksumming_on(&client) {
        assert!(!client_on, "client veth must STILL have tx-checksumming OFF after re-converge");
    }
    if let Some(backend_on) = tx_checksumming_on(&backend) {
        assert!(!backend_on, "backend veth must STILL have tx-checksumming OFF after re-converge");
    }

    delete_pair(&client);
}

/// Drift-repair: a COMPLETE pair whose TX-offload has drifted back ON
/// (e.g. a manual `ethtool -K … tx on`, a driver reset, an operator
/// fat-finger) must be converged back to offload-OFF by `provision` —
/// the present-pair completion path (NOT the recreate path) must observe
/// offload-on and emit the disable. This is the production scenario the
/// `observe`→`converge_steps` offload read exists for: without an honest
/// observation of the live offload state, a re-provision over a drifted
/// pair would leave offload ON and silently corrupt every NAT'd packet
/// (commit 62fa6be2). Guards the observer reporting the TRUE per-iface
/// offload bit (not a constant), so converge emits the disable exactly
/// when it is actually needed.
#[test]
fn provision_repairs_tx_offload_drifted_back_on() {
    let (client, backend) = iface_names('d');
    delete_pair(&client);

    let plan = plan_for(&client, &backend, "10.102.0.0/24");
    // Stand up a complete, converged pair (offload off) — or skip.
    match provision(&plan) {
        Ok(()) => {}
        Err(err) if is_cap_skip(&err) => {
            eprintln!(
                "SKIP provision_repairs_tx_offload_drifted_back_on: CAP_NET_ADMIN required ({err})"
            );
            return;
        }
        Err(err) => panic!("initial provision failed: {err}"),
    }

    // Drift offload BACK ON on both ends. If the feature is fixed /
    // unsettable on this runner's veth, `ethtool -K … tx on` is a no-op
    // and the observed state stays off — in which case this scenario
    // cannot be constructed, so skip rather than assert a precondition we
    // cannot establish.
    let _ = Command::new("ethtool").args(["-K", &client, "tx", "on"]).output();
    let _ = Command::new("ethtool").args(["-K", &backend, "tx", "on"]).output();
    let drifted_on =
        tx_checksumming_on(&client) == Some(true) || tx_checksumming_on(&backend) == Some(true);
    if !drifted_on {
        eprintln!(
            "SKIP provision_repairs_tx_offload_drifted_back_on: could not force tx-offload back on (fixed feature / no ethtool); drift scenario not constructible"
        );
        delete_pair(&client);
        return;
    }

    // Re-provision the (present, complete) pair: converge must OBSERVE the
    // drifted-on offload and emit the disable, restoring offload-off.
    provision(&plan).expect("provision must converge a complete pair whose offload drifted on");

    if let Some(client_on) = tx_checksumming_on(&client) {
        assert!(!client_on, "drifted-on client offload must be repaired to OFF by provision");
    }
    if let Some(backend_on) = tx_checksumming_on(&backend) {
        assert!(!backend_on, "drifted-on backend offload must be repaired to OFF by provision");
    }

    delete_pair(&client);
}

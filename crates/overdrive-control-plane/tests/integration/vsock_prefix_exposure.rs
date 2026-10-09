//! P4 — guest-prefix exposure through the production composition (Tier-3, V-19).
//!
//! netns-density-295 vsock Attachment Replacement DESIGN
//! (`docs/feature/netns-density-295/feature-delta.md` § "Composition —
//! production entry points"; § "Owner and provisioner" (`converge_shared`,
//! U-4); Gate G-V1 / G-V8; validation item V-19 (`spike/findings-v26.md` /
//! the production run); ADR-0152/0171/0175 D8a, D8a-LOOKUP, D8a-LOAD-SWAP,
//! PREFIX-REPLY, R5-3, R5-25). The property: no connection to a guest-prefix
//! address reaches a host socket other than a steered intake listener or leg-C,
//! with `serve` up and down, and a graceful stop leaves the route, the constant
//! rules and the pinned steering in place for the next boot to converge.
//!
//! Lane: Tier-3, example-based (Mandate 11). Drives the **production boot
//! composition** (`run_server_with_obs_and_driver`) with real adapters at the
//! steering / nft / route driven ports and real `lo` connects as the oracle;
//! Sim at the external / non-deterministic ports. Node-global kernel state (the
//! root-namespace `sk_lookup` link + pinned map, the guest-prefix nft table,
//! the shared `local` route, `lo`): this MODULE joins the `host-kernel-shared`
//! nextest group (`.config/nextest.toml`, matched by module). The "after the
//! guest listens, the connect reaches the guest application" half needs a real
//! microVM (CH-fork) and runs on the native x86_64 KVM host via
//! `cargo xtask metal run --` (shared-test-infrastructure.md § 5).
//!
//! ## GATED — pending DELIVER step 9/12 (V-19 production composition)
//!
//! The vsock composition surface (`SharedGuestNetworkOwner`, the steering,
//! the owner boot wiring in `run_server_with_obs_and_driver`) does NOT exist
//! yet; none is invented here (CLAUDE.md § "Implement to the design"). DELIVER
//! wires it and only then **registers this module** in
//! `crates/overdrive-control-plane/tests/integration.rs`
//! (`mod vsock_prefix_exposure;`) and adds its `host-kernel-shared` group
//! entry. Until registered Cargo never compiles this file. V-19 is a
//! DELIVER-step confirmation authored RED (`#[ignore]`), not a blocked item.

use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::time::Duration;

// --- fixture vocabulary (DELIVER's production composition replaces these) -----

/// A booted `overdrive serve` composition over the vsock transport, exposing
/// ONLY the observable surface V-19 reads: real kernel-query helpers and the
/// public API. Never private owner fields (Quint-conformance rule).
struct ServeNode;
impl ServeNode {
    /// Boot `run_server_with_obs_and_driver` with the vsock transport wired and
    /// Sim at the external ports.
    fn boot() -> Self {
        unreachable!("DELIVER: run_server_with_obs_and_driver(vsock) real-kernel composition")
    }
    /// SIGKILL / drop the serve process (ungraceful); node-global objects stay.
    fn kill(self) {}
    /// Graceful shutdown; node-global objects stay (U-4).
    fn shutdown_graceful(self) {}
    /// Deploy a VM declaring `port` (does not make the guest listen).
    fn deploy_declaring(&self, _port: u16) -> Ipv4Addr {
        unreachable!("DELIVER: overdrive deploy a VM declaring the port -> its workload_addr")
    }
    /// Drive the guest application to listen on `port` (metal cell).
    fn make_guest_listen(&self, _workload_addr: Ipv4Addr, _port: u16) {}
    /// Is the Overdrive-tagged `local <prefix> dev lo src <gateway>` route present?
    fn local_route_present(&self) -> bool {
        unreachable!("DELIVER: `ip route` query for the tagged local route")
    }
    /// Is a pinned `sk_lookup` steering link present under the pin dir?
    fn pinned_steering_present(&self) -> bool {
        unreachable!("DELIVER: `bpftool net`/link query")
    }
    /// Are the constant guest-prefix nft rules present?
    fn constant_prefix_rules_present(&self) -> bool {
        unreachable!("DELIVER: nft table query")
    }
}

/// A host service bound wildcard on `port` throughout (the B-2 exposure trap).
fn wildcard_host_service(port: u16) -> TcpListener {
    TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port))
        .expect("host service binds wildcard")
}
fn connect(dst: SocketAddrV4) -> std::io::Result<TcpStream> {
    TcpStream::connect_timeout(&dst.into(), Duration::from_secs(1))
}
fn expect_reset(dst: SocketAddrV4) {
    assert!(
        matches!(connect(dst), Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused),
        "connect to {dst} must be reset and reach no host socket",
    );
}

// --- P4-S16 / V-19, R5-3, R5-25, U-4 ------------------------------------------

#[test]
#[ignore = "pending DELIVER step 9/12 (V-19 guest-prefix exposure through serve)"]
fn no_connection_to_a_guest_prefix_address_reaches_a_host_socket_other_than_a_steered_intake_or_leg_c()
 {
    let node = ServeNode::boot();
    let declared_port: u16 = 22;
    let _sshd = wildcard_host_service(declared_port); // `sshd`-shaped trap on 0.0.0.0:22

    // GIVEN a VM declaring port 22 whose guest is NOT yet listening:
    let workload_addr = node.deploy_declaring(declared_port);
    let intake_dst = SocketAddrV4::new(workload_addr, declared_port);
    // THEN a marked probe and an unmarked host-local connect are both reset and
    // the host service accepts nothing; a remote connect to an unassigned prefix
    // address is dropped (R5-3).
    expect_reset(intake_dst);
    expect_reset(SocketAddrV4::new(Ipv4Addr::new(100, 95, 240, 240), declared_port));

    // WHEN the guest listens and the owner steers its intake (metal cell):
    node.make_guest_listen(workload_addr, declared_port);
    // THEN the connect reaches the guest application (never `sshd`) and
    // exchanges both ways.
    let mut client = connect(intake_dst).expect("steered intake reachable after the guest listens");
    client.write_all(b"v19-req").expect("write");
    let mut buf = [0u8; 7];
    let _ = client.read(&mut buf); // the guest app echoes; the oracle is "not sshd"

    // AND while serve is DOWN (killed) every connect to the prefix is still
    // reset and reaches no host socket (the pinned steering keeps the prefix
    // closed with no process, K-L4).
    node.kill();
    expect_reset(intake_dst);
}

#[test]
#[ignore = "pending DELIVER step 9/12 (V-19 graceful-stop convergence)"]
fn a_graceful_stop_leaves_route_rules_and_pinned_steering_and_the_next_boot_converges_all_three() {
    let node = ServeNode::boot();
    assert!(
        node.local_route_present()
            && node.pinned_steering_present()
            && node.constant_prefix_rules_present()
    );

    // WHEN serve stops gracefully (U-4): the three node-global objects stay.
    node.shutdown_graceful();
    let down = ServeNode; // observe node-global state with serve down
    assert!(down.local_route_present(), "U-4: the local route stays");
    assert!(down.pinned_steering_present(), "U-4: the pinned steering stays");
    assert!(down.constant_prefix_rules_present(), "U-4: the constant rules stay");
    // AND every connect to the prefix while serve is down reaches no host socket.
    expect_reset(SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, 2), 22));

    // AND the next boot converges all three (keeps/updates the steering, keeps
    // the route, re-adds the rules).
    let node2 = ServeNode::boot();
    assert!(
        node2.local_route_present()
            && node2.pinned_steering_present()
            && node2.constant_prefix_rules_present()
    );
}

//! P4 — guest-prefix steering kernel facts (Tier-3, `sk_lookup`).
//!
//! netns-density-295 vsock Attachment Replacement DESIGN
//! (`docs/feature/netns-density-295/feature-delta.md` § "Driven port —
//! guest-prefix steering `overdrive-dataplane::guest_steering`",
//! ADR-0171 D8a-LOOKUP, ADR-0175 D8a-LOAD-SWAP, ADR-0176 D8a-CAP) and
//! validation item V-26 (`spike/findings-v26.md`: all seven kernel claims
//! HOLD on 7.0.0-29/7.0.0-34).
//!
//! This is an **adapter-integration** file (`.claude/rules/tdd-methodology`
//! § "Infrastructure Layer"): it drives the real `GuestPrefixSteering`
//! adapter against the real host kernel (`sk_lookup` link under
//! `GUEST_PREFIX_STEERING_PIN_DIR`, a listener-keyed socket map, `lo`), with
//! hand-bound listening TCPv4 sockets standing in for the owner's intake
//! listeners. The full composition fact — "a steered connection reaches the
//! guest application" — is the production exposure test
//! (`vsock_prefix_exposure.rs`, V-19) and the P6 walking skeleton; here the
//! oracle is the *kernel's* landing decision for a real `lo` connect.
//!
//! Lane: Tier-3, example-based (Mandate 11 — layer-3 sad paths are enumerated,
//! never PBT-generated). Node-global kernel state (the root-namespace
//! `sk_lookup` link + pinned map, `lo`, the guest-prefix nft table): this
//! MODULE joins the `host-kernel-shared` nextest group
//! (`.config/nextest.toml`, `max-threads = 1`, matched by module —
//! `.claude/rules/testing.md` § "Node-global kernel state"). DELIVER adds the
//! group filter entry when it registers this module.
//!
//! ## GATED — pending DELIVER roadmap reconciliation + steering adapter
//!
//! The production surface this file calls does NOT exist yet
//! (`GuestPrefixSteering`, `GuestSteeringCandidate`, `GuestSteeringError`,
//! `GUEST_PREFIX_STEERING_PIN_DIR`, `guest_intake_host_port`,
//! `GUEST_INTAKE_STEERING_MAX`). Per shared-test-infrastructure.md § 0 and
//! CLAUDE.md § "Implement to the design — never invent API surface", these
//! scaffolds are NOT invented here; DELIVER creates them at the roadmap
//! reconciliation + step 9/12 and only then **registers this module** in
//! `crates/overdrive-dataplane/tests/integration.rs`
//! (`mod vsock_guest_steering;`). Until registered, Cargo never compiles this
//! file, so the authored bodies below are the pre-authored RED scaffolds the
//! crafter lifts (the complete executable bodies the testing.md
//! § "RED acceptance scaffolds" convention wants, parked build-safe).
//!
//! Each test carries a reasoned `#[ignore]` naming its activating step; the
//! crafter removes only the marker and records a semantic RED before touching
//! production code.

use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::AsFd as _;
use std::time::Duration;

use ipnet::Ipv4Net;
use overdrive_core::guest_transport::{GUEST_INTAKE_STEERING_MAX, guest_intake_host_port};
use overdrive_dataplane::guest_steering::{
    GUEST_PREFIX_STEERING_PIN_DIR, GuestPrefixSteering, GuestSteeringError,
};

// --- fixture vocabulary (DELIVER's real-kernel fixtures replace these) ------

/// The constant node guest prefix (D8a; `100.95.0.0/16`). A `/16` with its
/// gateway at the first host address, matching `GuestAddressPool::new`.
fn guest_prefix() -> Ipv4Net {
    "100.95.0.0/16".parse().expect("constant prefix parses")
}
fn gateway() -> Ipv4Addr {
    Ipv4Addr::new(100, 95, 0, 1)
}
/// A leased workload address inside the prefix (not the gateway).
fn workload_addr() -> Ipv4Addr {
    Ipv4Addr::new(100, 95, 0, 2)
}

/// Bind + listen a real TCPv4 socket on `addr`, the intake-block port for the
/// declared port at `index` (INTAKE-BIND: `workload_addr:guest_intake_host_port(i)`,
/// never the declared port). Returns the listener; its fd is handed to `steer`.
fn bind_intake_listener(addr: Ipv4Addr, index: u16) -> TcpListener {
    let host_port = guest_intake_host_port(index).expect("index within the intake block");
    TcpListener::bind(SocketAddrV4::new(addr, host_port)).expect("intake bind succeeds")
}

/// Drive a real host-local TCP connect to `dst` and classify the kernel's
/// landing decision — the V-26 oracle. `Ok` iff the connect completes.
fn connect_outcome(dst: SocketAddrV4) -> std::io::Result<TcpStream> {
    TcpStream::connect_timeout(&dst.into(), Duration::from_secs(1))
}

/// Byte-distinct request/response litmus so the assertion proves the real
/// server→client pipe, not an echo.
const REQUEST: &[u8] = b"p4-steer-req";
const RESPONSE: &[u8] = b"p4-steer-rsp";

fn exchange_both_ways(mut client: TcpStream, listener: &TcpListener) {
    let (mut server, _peer) = listener.accept().expect("steered listener accepts");
    client.write_all(REQUEST).expect("client writes");
    let mut got = [0u8; REQUEST.len()];
    server.read_exact(&mut got).expect("server reads request");
    assert_eq!(&got, REQUEST, "server sees the real request bytes");
    server.write_all(RESPONSE).expect("server writes");
    let mut back = [0u8; RESPONSE.len()];
    client.read_exact(&mut back).expect("client reads response");
    assert_eq!(&back, RESPONSE, "client sees the real response bytes");
}

/// Boot the steering at the prefix the way `converge_shared` boot step 9 does:
/// `load` → `attach` (a first attach pins a new link under
/// `GUEST_PREFIX_STEERING_PIN_DIR`).
fn boot_steering() -> GuestPrefixSteering {
    GuestPrefixSteering::load(guest_prefix(), gateway())
        .expect("load: the verifier accepts the shipped program")
        .attach()
        .expect("attach: a first link is attached and pinned")
}

// --- P4-S1 / R5-1, R5-2 (steering-layer half; G-V8, V-26 K-L1/K-L5/K-L10) ---

#[test]
#[ignore = "pending DELIVER step 09/12 (GuestPrefixSteering adapter; roadmap reconciliation)"]
fn only_a_steered_listener_is_reachable_at_the_prefix_declared_port() {
    // GIVEN the boot steering is attached and no entry names the declared port,
    // a host service bound wildcard on the declared port, and nothing steered.
    let steering = boot_steering();
    let declared_port: u16 = 2222;
    let _wildcard_host_service =
        TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, declared_port))
            .expect("a host service may hold 0.0.0.0:declared_port");
    let intake_dst = SocketAddrV4::new(workload_addr(), declared_port);

    // WHEN a host-local connect reaches the prefix with no steering entry:
    // THEN the kernel drops the lookup; TCP is answered with a reset (K-L1,
    // never the wildcard host service — B-2 defect closed).
    assert!(
        matches!(connect_outcome(intake_dst), Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused),
        "no entry -> reset, never the wildcard host service",
    );

    // WHEN the owner binds an intake listener on the intake-block port and
    // steers (workload_addr, declared_port) to it (K-L10: the entry assigns
    // the listener whatever port it is bound to).
    let intake = bind_intake_listener(workload_addr(), 0);
    steering
        .steer(intake_dst, intake.as_fd())
        .expect("steer succeeds: admission reserved the port, map has room");

    // THEN a connect to workload_addr:declared_port reaches the intake listener,
    let client = connect_outcome(intake_dst).expect("steered declared port reaches the intake");
    exchange_both_ways(client, &intake);

    // AND a connect to the listener's own bound block port (not a declared
    // served port) is reset — it is decided by its own (absent) entry, not by
    // the bound address (K-L10).
    let bound_block = SocketAddrV4::new(workload_addr(), guest_intake_host_port(0).unwrap());
    assert!(
        matches!(connect_outcome(bound_block), Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused),
        "the bound block port has no entry of its own -> reset",
    );
}

// --- P4-S2 / R5-3, K-L1: unassigned/released prefix address reaches nothing ---

#[test]
#[ignore = "pending DELIVER step 09/12 (GuestPrefixSteering adapter; roadmap reconciliation)"]
fn an_unassigned_prefix_address_reaches_no_host_socket() {
    // GIVEN the boot steering is attached and a wildcard host service on a port.
    let _steering = boot_steering();
    let port: u16 = 2223;
    let _wildcard_host_service = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port))
        .expect("host service binds");
    // WHEN a host-local connect targets a prefix address with no lease / entry:
    let unassigned = SocketAddrV4::new(Ipv4Addr::new(100, 95, 200, 200), port);
    // THEN the lookup is dropped and no host socket is reached (with the nft
    // constant rules both present and — the DELIVER Tier-3 cell — deleted; the
    // steering alone keeps the prefix closed, K-L1).
    assert!(
        matches!(connect_outcome(unassigned), Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused),
        "unassigned prefix address -> reset, reaches no host socket",
    );
}

// --- P4-S3 / R5-22 close, D8a-CAP, K-L3: close removes the entry + cap cycle --

#[test]
#[ignore = "pending DELIVER step 09/12 (GuestPrefixSteering adapter; roadmap reconciliation)"]
fn closing_a_listener_frees_its_steering_slot_in_the_same_step() {
    let steering = boot_steering();
    let port: u16 = 2300;
    let dst = SocketAddrV4::new(workload_addr(), port);

    // GIVEN a steered listener, a connect reaches it.
    let intake = bind_intake_listener(workload_addr(), 0);
    steering.steer(dst, intake.as_fd()).expect("steer");
    connect_outcome(dst).expect("steered -> reachable").shutdown(std::net::Shutdown::Both).ok();

    // WHEN the listener closes (dropped) the kernel removes the entry in the
    // same step (K-L3): the slot is free and the next connect is reset.
    drop(intake);
    assert!(
        matches!(connect_outcome(dst), Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused),
        "close removes the entry in the same kernel step -> reset",
    );
    // AND a fresh listener steered on the same port is reachable at once.
    let intake2 = bind_intake_listener(workload_addr(), 0);
    steering.steer(dst, intake2.as_fd()).expect("re-steer at once, the slot was freed");
    exchange_both_ways(connect_outcome(dst).expect("re-steered reachable"), &intake2);
}

#[test]
#[ignore = "pending DELIVER step 09/12 (GuestPrefixSteering adapter; roadmap reconciliation)"]
fn steering_map_at_capacity_refuses_one_more_then_a_close_frees_a_slot() {
    // GIVEN the map at GUEST_INTAKE_STEERING_MAX entries (DELIVER fills it with
    // distinct (address, port) entries over the prefix — P-40 filled 262,144 as
    // 5 ports x 52,429 prefix addresses). This fixture is Tier-3-heavy; DELIVER
    // supplies the real fill helper. Here the oracle is the kernel's E2BIG.
    let steering = boot_steering();
    let filled = fill_steering_to_capacity(&steering); // DELIVER fixture
    assert_eq!(filled.len() as u32, GUEST_INTAKE_STEERING_MAX, "the map holds its max_entries");

    // WHEN one more insert is attempted on a free (address, port):
    let overflow_port: u16 = 2399;
    let overflow = bind_intake_listener(workload_addr(), 1);
    let err = steering
        .steer(SocketAddrV4::new(workload_addr(), overflow_port), overflow.as_fd())
        .expect_err("a full map refuses the insert");
    // THEN the kernel refuses with the map-full path (E2BIG surfaces as a
    // MapUpdate on this adapter; D8a-CAP guarantees the owner never hits it,
    // but the kernel fact is asserted here).
    assert!(matches!(err, GuestSteeringError::MapUpdate { .. }), "full map -> MapUpdate");

    // AND after one earlier listener closes, its slot frees and the insert
    // succeeds.
    drop(filled.into_iter().next().expect("at least one entry"));
    steering
        .steer(SocketAddrV4::new(workload_addr(), overflow_port), overflow.as_fd())
        .expect("a freed slot lets the insert succeed");
}

/// DELIVER-provided Tier-3 fixture: fill the steering map to its `max_entries`
/// with distinct `(address, port)` entries over the prefix. Named here so the
/// scaffold reads; the crafter supplies the body co-sequenced with the adapter.
fn fill_steering_to_capacity(_steering: &GuestPrefixSteering) -> Vec<TcpListener> {
    unreachable!("DELIVER real-kernel fixture: fill to GUEST_INTAKE_STEERING_MAX")
}

// --- P4-S4 / R5-24, R5-34, D8a-LOAD-SWAP, K-L9: pinned link + atomic swap -----

#[test]
#[ignore = "pending DELIVER step 09/12 (GuestPrefixSteering adapter; roadmap reconciliation)"]
fn boot_over_a_left_behind_pinned_link_swaps_atomically_and_never_detaches() {
    // GIVEN an earlier boot left a pinned link running its program (DELIVER
    // leaves one, then drops the owner handle without unpinning — U-4). A
    // connect loop runs across the load and the swap; `sshd` is on 0.0.0.0:port.
    let port: u16 = 22;
    let _sshd =
        TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)).expect("stand-in sshd");

    // WHEN a new binary boots over the adopted pinned link: `load` a new
    // program+map, `attach` swaps it in with BPF_LINK_UPDATE (never a detach).
    let candidate = GuestPrefixSteering::load(guest_prefix(), gateway()).expect("load new program");
    // A connect-loop oracle runs concurrently across the next line; DELIVER's
    // Tier-3 harness spawns it and asserts every connect across the update is
    // decided only by the earlier program or the new one — never `sshd`.
    let steering = candidate.attach().expect("attach: BPF_LINK_UPDATE swaps atomically, no detach");

    // THEN the pinned link still exists (never detached) and runs this boot's
    // program; a connect to a workload address with no entry is reset, never
    // reaching `sshd` at any instant of the loop.
    assert!(
        pinned_link_exists(GUEST_PREFIX_STEERING_PIN_DIR),
        "the pinned link is never detached across the swap (K-L9)",
    );
    let dst = SocketAddrV4::new(workload_addr(), port);
    assert!(
        matches!(connect_outcome(dst), Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused),
        "no entry -> reset, never `sshd`, across and after the swap",
    );
    drop(steering);
}

#[test]
#[ignore = "pending DELIVER step 09/12 (GuestPrefixSteering adapter; roadmap reconciliation)"]
fn a_failed_swap_leaves_the_earlier_program_attached() {
    // GIVEN a pinned link running an earlier program and a scripted
    // BPF_LINK_UPDATE failure (DELIVER injects it on the real kernel path; the
    // sim path is vsock_prefix_boot_order.rs).
    let candidate = GuestPrefixSteering::load(guest_prefix(), gateway()).expect("load");
    // WHEN the swap fails:
    let err = candidate.attach().expect_err("the scripted BPF_LINK_UPDATE fails");
    // THEN the link still runs the earlier program, unchanged (K-L9), and the
    // error is `Update`.
    assert!(matches!(err, GuestSteeringError::Update { .. }), "swap failure -> Update");
    assert!(
        pinned_link_exists(GUEST_PREFIX_STEERING_PIN_DIR),
        "a failed swap leaves the earlier pinned link and its program",
    );
}

/// DELIVER-provided Tier-3 helper: does a pinned `sk_lookup` link exist under
/// the pin dir? Observed by a kernel link query (`bpftool net` / the link pin),
/// never through the owner's private fields.
fn pinned_link_exists(_pin_dir: &str) -> bool {
    unreachable!("DELIVER real-kernel fixture: query the pinned sk_lookup link")
}

// --- P4-S5 / R5-43, PREFIX-REPLY: reply path for an in-prefix source ----------

#[test]
#[ignore = "pending DELIVER step 09/12 (GuestPrefixSteering adapter; roadmap reconciliation)"]
fn a_host_local_client_whose_source_lies_in_the_prefix_completes_to_a_steered_intake() {
    // GIVEN the production constant guest-prefix rules are present (PREFIX-REPLY:
    // `ct state established,related accept` sits before each reject/drop, after
    // the leg-S exemption and the leg-C divert), the steering is attached, and a
    // declared port is steered to an intake listener.
    let steering = boot_steering();
    let port: u16 = 2500;
    let dst = SocketAddrV4::new(workload_addr(), port);
    let intake = bind_intake_listener(workload_addr(), 0);
    steering.steer(dst, intake.as_fd()).expect("steer");

    // WHEN a host-local client whose SOURCE address lies in the prefix (the
    // gateway) connects to the steered intake:
    // DELIVER binds the client socket to the gateway source; here the oracle is
    // that the connection completes AND exchanges both ways (replies pass the
    // guest-prefix rules whatever their address, PREFIX-REPLY).
    let client = connect_from_source(gateway(), dst).expect("admitted connection completes");
    exchange_both_ways(client, &intake);

    // AND a NEW unmarked host-local connect to the prefix with no entry is still
    // rejected at once (the accept decides only established/related — rule 5/8
    // unchanged for connection-opening packets).
    let fresh = SocketAddrV4::new(workload_addr(), 2501);
    assert!(
        matches!(connect_outcome(fresh), Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused),
        "a new connection-opening packet is still rejected; only replies pass",
    );
    // (leg-C-diverted and leg-S connections keeping their paths are the P5
    // enforcement cells and the exposure test, not asserted at this layer.)
}

/// DELIVER-provided helper: connect with the client socket bound to `src` as
/// its source address (a `local`-route preferred source inside the prefix).
fn connect_from_source(_src: Ipv4Addr, _dst: SocketAddrV4) -> std::io::Result<TcpStream> {
    unreachable!("DELIVER real-kernel fixture: bind the client source, then connect")
}

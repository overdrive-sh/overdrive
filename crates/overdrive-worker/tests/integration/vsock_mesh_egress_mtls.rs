//! P5 — non-mesh egress kernel-forwarded directly (D14) and mesh egress via
//! leg-F enforced with mTLS (D9). Tier-3, `host-kernel-shared`.
//!
//! DISTILL (netns-density-295 vsock replacement), piece P5. Scenarios P5-S9,
//! P5-S10; obligation R5-8 (mesh/non-mesh half); gate G-V6. Decisions D14
//! (ADR-0162) and D9 (ADR-0153). Spec:
//! `docs/feature/netns-density-295/distill/test-scenarios-p5-egress-unframe-hostinternal.md`.
//!
//! ## GATED — authored RED, NOT YET REGISTERED
//! `MtlsInterceptWorker::register_forwarded_outbound` / `ForwardedOutbound` /
//! `leg_f_addr` and the deletion of `spawn_cleartext_passthrough` do not exist
//! yet (confirmed 2026-10-09). Not added to `tests/integration.rs`
//! `mod integration { … }` nor to the `host-kernel-shared` nextest group until
//! the DELIVER scaffold lands (shared-infra §6). Drives `MtlsInterceptWorker`
//! leg-F directly + a route-disjoint netns peer — NOT `boot_vsock_node` (an
//! `overdrive-worker` test depending on `tests/conformance` is a cycle); the
//! full `serve` + `deploy` mesh cell is P6's walking skeleton (V-6).
//!
//! ## Design-pinned symbols (DELIVER step ~9 scaffold)
//! `overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker::{register_forwarded_outbound,
//! leg_f_addr}`, `ForwardedOutbound`, `MtlsInterceptInstallError::{NotInterceptLive,
//! DuplicateForwardedRegistration}`, `ResolvedBackend`.
//! `overdrive_testing::cidr_lease::TestCidrLease` (route-disjoint off-host dest).
//!
//! ## Egress-dialer discipline (CLAUDE.md §"East-west mTLS tests") — LOAD-BEARING
//! The guest application is the DIALER and speaks PLAINTEXT over an ordinary
//! socket (it is identity-unaware; the agent originates mTLS transparently).
//! A test driving the guest side MUST NOT present client rustls TLS toward the
//! dialed address — that opens a peerless session the agent leg never
//! terminates: the ClientHello tunnels as plaintext, no ServerHello returns,
//! the handshake stalls → RST. The mTLS proof is observed on the INTER-AGENT
//! wire (leg-F → remote peer), NOT the client handshake. Keep a byte-distinct
//! REQUEST/RESPONSE litmus. The SERVER side still holds SVIDs.

use std::net::SocketAddrV4;

use overdrive_testing::cidr_lease::TestCidrLease;

/// Byte-distinct litmus so the oracle proves the real server→client reply pipe,
/// not an echo.
const REQUEST: &[u8] = b"P5-EGRESS-REQUEST\n";
const RESPONSE: &[u8] = b"P5-EGRESS-RESPONSE\n";

// ---------------------------------------------------------------------------
// P5-S9 — non-mesh egress kernel-forwarded directly to its destination (D14).
// @contract-shape:unbounded-preservation.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "pending DELIVER: non-mesh direct egress, cleartext relay removed (D14, ~step 9)"]
async fn non_mesh_guest_tcp_is_forwarded_directly_with_no_cleartext_relay() {
    // GIVEN a route-disjoint off-host destination (never a CIDR literal — the
    // shared allocator draws from 10.250.0.0/16, disjoint from the production
    // workload subnet).
    let lease = TestCidrLease::acquire("p5-nonmesh-egress").expect("route-disjoint cidr");
    let dest = support::OffHostTcpServer::spawn(&lease, RESPONSE).await.expect("off-host server");
    let node = support::MeshEgressNode::compose(&lease).await.expect("worker + forwarder");

    // WHEN a guest plaintext TCP flow dials the non-mesh destination.
    // The owner's host socket connects `dst` DIRECTLY (marked, SO_LINGER{1,0});
    // leg-F is NOT involved for a NonMesh destination.
    let mut dialer = node.open_guest_tcp_flow_plaintext(dest.addr()).await.expect("flow opens");
    dialer.write_all(REQUEST).await.expect("request");
    let got = dialer.read_response(RESPONSE.len()).await.expect("response");

    // THEN byte-exact round-trip, and NO leg-F registration / cleartext relay.
    assert_eq!(got, RESPONSE, "non-mesh destination round-trips byte-exact");
    assert_eq!(dest.request_received(), REQUEST, "destination received the request byte-exact");
    assert_eq!(
        node.forwarded_outbound_registrations(),
        0,
        "no leg-F registration for a NonMesh flow (D14)"
    );
    assert!(node.cleartext_relay_sockets().is_empty(), "no cleartext passthrough socket (D14)");
}

#[tokio::test]
#[ignore = "pending DELIVER: non-mesh direct UDP egress (D14, ~step 9)"]
async fn non_mesh_guest_udp_is_forwarded_directly() {
    let lease = TestCidrLease::acquire("p5-nonmesh-udp").expect("route-disjoint cidr");
    let dest = support::OffHostUdpServer::spawn(&lease, RESPONSE).await.expect("off-host udp");
    let node = support::MeshEgressNode::compose(&lease).await.expect("node");

    let reply = node
        .send_guest_udp_datagram(dest.addr(), REQUEST)
        .await
        .expect("udp datagram forwarded directly");
    assert_eq!(reply.source(), dest.addr(), "reply source = original destination");
    assert_eq!(reply.payload(), RESPONSE, "udp reply byte-exact");
    assert_eq!(node.forwarded_outbound_registrations(), 0, "no leg-F for NonMesh UDP");
}

// ---------------------------------------------------------------------------
// P5-S10 — mesh egress resolved once at pairing, enforced with mTLS on the
// inter-agent wire (D9). @contract-shape:unbounded-preservation.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "pending DELIVER: mesh leg-F registration + mTLS (D9, ~step 9)"]
async fn mesh_guest_tcp_reaches_leg_f_once_and_the_inter_agent_wire_is_mtls() {
    let lease = TestCidrLease::acquire("p5-mesh-egress").expect("route-disjoint cidr");
    // The remote mesh peer HOLDS a server identity (SVID); the test client does
    // not. Egress-dialer discipline: the guest dials PLAINTEXT.
    let peer =
        support::MeshPeer::spawn_with_server_identity(&lease, RESPONSE).await.expect("mesh peer");
    let node = support::MeshEgressNode::compose(&lease).await.expect("node");

    // The owner resolves the destination to a mesh backend ONCE and registers
    // the forwarded outbound; leg-F claims it at accept and does NOT re-resolve.
    let mut dialer = node
        .open_guest_tcp_flow_plaintext(peer.frontend_addr()) // plaintext ordinary socket
        .await
        .expect("mesh flow opens");
    dialer.write_all(REQUEST).await.expect("request");
    let got = dialer.read_response(RESPONSE.len()).await.expect("response");

    // THEN: byte-exact plaintext round-trip at the guest app.
    assert_eq!(got, RESPONSE, "mesh round-trip byte-exact (plaintext at the dialer)");
    // Exactly one forwarded-outbound registration (resolved once at pairing).
    assert_eq!(node.forwarded_outbound_registrations(), 1, "leg-F registered once (D9)");
    assert!(!node.leg_f_re_resolved(), "leg-F does not re-resolve (ADR-0123)");

    // AND: the INTER-AGENT wire (leg-F → remote peer) carries TLS 1.3
    // application_data records (0x17), zero cleartext — observed on the leg,
    // NOT the client handshake (the dialer is plaintext by design).
    let scan = node.scan_inter_agent_wire(peer.wire_addr()).await.expect("wire scan");
    assert_eq!(scan.cleartext_bytes, 0, "no cleartext on the inter-agent leg");
    assert!(scan.tls13_application_data_records > 0, "leg-F ↔ peer carries 0x17 TLS records");
}

/// P5-S10 sad paths — duplicate registration rejected; a leg-F accept with no
/// registration is closed; an accept not intercept-live is refused.
#[tokio::test]
#[ignore = "pending DELIVER: leg-F registration claim atomicity (D9, ~step 9)"]
async fn leg_f_registration_is_claimed_once_and_duplicates_are_refused() {
    use overdrive_worker::mtls_intercept_worker::MtlsInterceptInstallError;

    let lease = TestCidrLease::acquire("p5-mesh-dup").expect("route-disjoint cidr");
    let node = support::MeshEgressNode::compose(&lease).await.expect("node");
    let forwarder_local = node.leg_f_forwarder_local().await.expect("leg-F local addr");
    let backend = node.resolved_backend_for_test();
    let original = SocketAddrV4::new(std::net::Ipv4Addr::new(10, 96, 0, 7), 443);

    let alloc = node.intercept_live_alloc();
    let _held = node
        .worker()
        .register_forwarded_outbound(&alloc, forwarder_local, original, backend.clone())
        .expect("first registration");

    // A second registration for the same forwarder_local is refused.
    let dup = node.worker().register_forwarded_outbound(
        &alloc,
        forwarder_local,
        original,
        backend.clone(),
    );
    assert!(
        matches!(dup, Err(MtlsInterceptInstallError::DuplicateForwardedRegistration { .. })),
        "duplicate forwarded registration refused"
    );

    // An allocation that is not intercept-live is refused NotInterceptLive.
    let dead = node.not_intercept_live_alloc();
    let not_live = node.worker().register_forwarded_outbound(
        &dead,
        node.spare_forwarder_local().await,
        original,
        backend,
    );
    assert!(
        matches!(not_live, Err(MtlsInterceptInstallError::NotInterceptLive { .. })),
        "registration for a non-intercept-live alloc refused"
    );

    // A leg-F accept with no registration is closed without resolution.
    let orphan = node.accept_leg_f_without_registration().await;
    assert!(orphan.closed_without_resolution(), "unregistered leg-F accept closed");
}

/// Worker-local helpers DELIVER shapes (production-composed leg-F + forwarder +
/// a route-disjoint mesh peer / off-host destination). Declared here so the
/// authored bodies name a stable surface; the module body is a DELIVER scaffold.
mod support {
    // DELIVER fills this with the production `MtlsInterceptWorker` composition,
    // the plaintext guest-flow dialer, the off-host netns servers (via the
    // caller's `TestCidrLease`), and the inter-agent wire scan. Referenced by
    // the tests above; it is NOT a hand-installed production effect — it drives
    // the worker's own production objects (shared-infra §2, "no test
    // hand-installs a production effect").
}

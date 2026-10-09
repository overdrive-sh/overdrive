//! P5 — host-internal rule construction (D26-PINS) enforced by the production
//! shared firewall table. Tier-3, `host-kernel-shared`.
//!
//! DISTILL (netns-density-295 vsock replacement), piece P5. Scenario P5-S8;
//! obligations R5-48, R5-7 (kernel-rule half); gate G-V6. Decision D26-PINS
//! (ADR-0167). Anchored on P-47 (V-21). Spec:
//! `docs/feature/netns-density-295/distill/test-scenarios-p5-egress-unframe-hostinternal.md`.
//!
//! ## GATED — authored RED, NOT YET REGISTERED
//! The `OWNER_EGRESS_MARK` rule and the reshaped shared-table construction do
//! not exist yet (confirmed 2026-10-09). Not added to `tests/integration.rs`
//! `mod integration { … }` nor to the `host-kernel-shared` nextest group until
//! the DELIVER scaffold lands (shared-infra §6). Node-global kernel state (the
//! `overdrive-mtls` nft table, marked sockets) → `host-kernel-shared`,
//! `max-threads = 1`, matched by MODULE.
//!
//! ## Design-pinned symbols (DELIVER step ~9 scaffold)
//! the production shared-table composition (`run_server` rules; D26 output
//! rules in the `overdrive-mtls`-shaped table), `OWNER_EGRESS_MARK`,
//! `MTLS_LEG_S_DIAL_MARK`, `MTLS_TPROXY_MARK`. The marked dialer stands in for
//! the owner's connected host socket H.
//!
//! ## Oracle note (R5-7 / F-4)
//! V-21 (P-47) used userspace sends. The production **forwarder redirect**
//! reporting the reject on owner socket H (`sk_psock_report_error`) is
//! end-to-end and is discharged in P6's walking skeleton through `serve` +
//! `deploy`; this file pins the RULE behaviour (marked transmit rejected,
//! others reachable) with a marked userspace dialer standing in for H.

use std::net::{Ipv4Addr, SocketAddrV4};

use super::integration_support as support; // production shared-table composition + marked-dialer helpers

/// The guest prefix (feature-delta): `100.95.0.0/16`.
fn guest_prefix_broadcast() -> Ipv4Addr {
    Ipv4Addr::new(100, 95, 255, 255)
}

// ---------------------------------------------------------------------------
// P5-S8 — the host-internal output rule rejects marked owner sockets,
// structurally. Each F-case is a named example cell (Mandate 11).
// @contract-shape:unbounded-preservation (marked dials reach no host service;
// everything else stays reachable).
// ---------------------------------------------------------------------------

/// F-1 — a marked socket reusing an earlier UNMARKED flow's source port to a
/// local destination is still rejected (the D26 rules precede every accept,
/// PREFIX-REPLY's `ct established,related accept` included).
#[tokio::test]
#[ignore = "pending DELIVER: D26 output rule (D26-PINS F-1, ~step 9)"]
async fn marked_socket_reusing_an_unmarked_source_port_is_still_rejected() {
    let table = support::ProductionSharedTable::compose().await.expect("shared table");
    let host_svc = support::LocalTcpService::bind(Ipv4Addr::LOCALHOST, 0).await; // a host service

    // An earlier UNMARKED connection on a source port establishes (and may leave
    // a conntrack entry); then a MARKED socket reuses that source port.
    let src_port = support::establish_then_release_unmarked_flow(&host_svc).await;
    let marked = support::marked_connect(
        overdrive_control_plane::guest_network::OWNER_EGRESS_MARK,
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, host_svc.port()),
        Some(src_port),
    )
    .await;

    assert!(marked.is_rejected(), "marked reuse of an unmarked source port rejected (F-1)");
    assert_eq!(host_svc.bytes_received(), 0, "host service received nothing");
}

/// F-2 — a marked-connect storm during repeated table replacement reaches no
/// host service (the table is replaced in ONE nft transaction, never separate
/// delete/add which P-47 showed leaks 559–1,446 connects).
#[tokio::test]
#[ignore = "pending DELIVER: D26 atomic table replace (D26-PINS F-2, ~step 9)"]
async fn marked_connect_storm_during_table_replacement_reaches_no_service() {
    let table = support::ProductionSharedTable::compose().await.expect("shared table");
    let host_svc = support::LocalTcpService::bind(Ipv4Addr::LOCALHOST, 0).await;

    let storm = support::spawn_marked_connect_storm(
        overdrive_control_plane::guest_network::OWNER_EGRESS_MARK,
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, host_svc.port()),
        /* duration */ std::time::Duration::from_secs(2),
    );
    for _ in 0..200 {
        table.replace_atomically().await.expect("one nft transaction (delete+add batched)");
    }
    let report = storm.join().await;
    assert_eq!(
        report.connects_that_reached_service, 0,
        "no connect leaked during replacement (F-2)"
    );
    assert_eq!(host_svc.bytes_received(), 0, "host service untouched");
}

/// F-3 — TCP `ECONNREFUSED` and UDP `EPERM`, never a hang (plain `reject`).
#[tokio::test]
#[ignore = "pending DELIVER: D26 plain reject verdict (D26-PINS F-3, ~step 9)"]
async fn marked_dials_fail_fast_tcp_econnrefused_udp_eperm_never_hang() {
    let table = support::ProductionSharedTable::compose().await.expect("shared table");
    let mark = overdrive_control_plane::guest_network::OWNER_EGRESS_MARK;
    let local = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 9);

    let tcp = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        support::marked_tcp_connect(mark, local),
    )
    .await
    .expect("TCP must not hang (F-3)");
    assert_eq!(tcp.io_error_kind(), std::io::ErrorKind::ConnectionRefused, "TCP ECONNREFUSED");

    let udp = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        support::marked_udp_first_send(mark, local),
    )
    .await
    .expect("UDP must not hang (F-3)");
    assert_eq!(udp.io_error_raw(), libc::EPERM, "UDP first send EPERM");
}

/// F-5 — marked UDP to a subnet-directed broadcast, the prefix broadcast
/// address and `224.0.0.1` reaches no host service — with and without
/// `SO_BROADCAST` — while `gateway:53`, an off-host address and a neighbour
/// stay reachable (`fib daddr type != unicast` closes broadcast/multicast
/// structurally, no dependence on the socket leaving `SO_BROADCAST` unset).
#[tokio::test]
#[ignore = "pending DELIVER: D26 non-unicast match (D26-PINS F-5, ~step 9)"]
async fn marked_udp_to_non_unicast_reaches_no_service_unicast_stays_reachable() {
    let table = support::ProductionSharedTable::compose().await.expect("shared table");
    let mark = overdrive_control_plane::guest_network::OWNER_EGRESS_MARK;

    let non_unicast =
        [Ipv4Addr::new(255, 255, 255, 255), guest_prefix_broadcast(), Ipv4Addr::new(224, 0, 0, 1)];
    for dst in non_unicast {
        for broadcast_opt in [false, true] {
            let r =
                support::marked_udp_first_send_opt(mark, SocketAddrV4::new(dst, 7), broadcast_opt)
                    .await;
            assert!(r.is_rejected(), "{dst} (SO_BROADCAST={broadcast_opt}) rejected (F-5)");
        }
    }
    // gateway:53, an off-host unicast address, and a neighbour stay reachable.
    assert!(support::reachable(mark, SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, 1), 53)).await);
    assert!(support::reachable(mark, SocketAddrV4::new(Ipv4Addr::new(93, 184, 216, 34), 53)).await);
    assert!(support::reachable(mark, support::neighbour_address().await).await);
}

/// F-6 — a packet carrying the owner bit with OTHER bits set is rejected (the
/// rule matches under the mask); a packet with ONLY the leg-S or TPROXY mark is
/// unaffected by the D26 rules.
#[tokio::test]
#[ignore = "pending DELIVER: D26 masked mark match (D26-PINS F-6, ~step 9)"]
async fn owner_bit_with_extra_bits_rejected_leg_s_and_tproxy_only_unaffected() {
    let table = support::ProductionSharedTable::compose().await.expect("shared table");
    let owner = overdrive_control_plane::guest_network::OWNER_EGRESS_MARK;
    let leg_s = overdrive_worker::mtls_intercept::MTLS_LEG_S_DIAL_MARK;
    let tproxy = overdrive_worker::mtls_intercept::MTLS_TPROXY_MARK;
    let local = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 9);

    // owner bit with extra bits set => matched under the mask => rejected.
    let with_extra = support::marked_tcp_connect(owner | 0x0F00_0000, local).await;
    assert!(with_extra.is_rejected(), "owner bit + extra bits rejected (F-6)");

    // only leg-S / only TPROXY => the D26 rule does not fire (those marks take
    // their own exempt/divert paths; here, to a local dst, they are NOT the D26
    // concern — the owner rule must not match them).
    assert!(!support::d26_rule_matches(leg_s), "leg-S-only not matched by D26 owner rule (F-6)");
    assert!(!support::d26_rule_matches(tproxy), "TPROXY-only not matched by D26 owner rule (F-6)");
}

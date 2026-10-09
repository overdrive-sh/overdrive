//! P3 guest-side UDP slot model (D25, D25-BIND, CELL-RECREATE, U-2). Covers the
//! HOST-RUNNABLE parts of P3-S2 (slot identity), P3-S3 (reserved-port predicate),
//! P3-S4 (idle + audit), P3-S5 (cell recreate + generation dispatch), P3-S10
//! (exhaustion), plus the `overdrive-init` transport-token parse robustness (C6).
//! Spec: `test-scenarios-p3-udp-slots-framing.md`; `feature-delta.md` §"Guest
//! adaptation contract" (L19593), §"Per-kind total orders — UDP slot release"
//! (L18340). The real guest-kernel behaviours (EADDRINUSE bind R5-15,
//! exhaustion EPERM, concurrent capture) are Tier-3 cells inside the P6 walking
//! skeleton (shared-infra §6, gap G-5).
//!
//! GATED (shared-infra §0): doubly. (1) `overdrive_core::guest_transport` does
//! not exist yet. (2) `overdrive-init` is a single binary with no library
//! surface today — TESTABILITY GAP G-2: D25 describes the slot-ownership table,
//! readiness dispatch and pool as `overdrive-init`-internal state read from the
//! kernel slot map; it pins no pure, host-testable component. This file is
//! authored against the design-implied pure shapes (`overdrive_init::slots`,
//! `overdrive_init::transport`) with an in-memory kernel-map double; the crafter
//! reconciles signatures at activation. If DELIVER does not expose such a
//! component, P3-S2/S4/S5/S10's slot-machine coverage is discharged by the Quint
//! `udp_slots` conformance (P3-S8) + the P6 Tier-3 cells, and this file keeps
//! only the fully-pinned reserved-port predicate + token-parse tests.
//!
//! This file is NOT registered yet. P3 owns creating
//! `crates/overdrive-init/tests/acceptance.rs` (distinct from P6's
//! `tests/integration.rs`); the `mod vsock_guest_udp_slots;` line co-arrives
//! with the DELIVER production scaffold (shared-infra §6).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use overdrive_core::guest_transport::{
    GUEST_DATAGRAM_SLOT_IDLE, GUEST_TCP_INTAKE_PORT, GUEST_UDP_SLOT_BASE, GUEST_UDP_SLOTS,
    guest_reserved_ports,
};
use proptest::prelude::*;

// G-2 design-implied pure guest slot component + transport-token parser.
use overdrive_init::slots::{SlotOutcome, SlotTable};
use overdrive_init::transport::{
    GuestTransportSetupError, TransportTokenField, parse_transport_token,
};

const WL: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 7); // a workload_addr in the guest prefix

fn dest(last: u8, port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(93, 184, 216, last), port)
}

// ---------------------------------------------------------------- P3-S3 (pure)
// D25-BIND / D15-R2: the reserved guest port range is `61_000..=62_024`; a port
// is reserved for a protocol only when it holds an intake of THAT protocol
// (TCP 61_000; UDP 61_001..=62_024). The host-runnable predicate; the real
// `EADDRINUSE` bind on a live guest kernel is the Tier-3 R5-15 cell in P6.

/// The reserved range is exactly the TCP intake plus the 1_024 UDP slot ports.
#[test]
#[ignore = "pending DELIVER: guest_reserved_ports (D25, D25-BIND)"]
fn reserved_guest_ports_cover_tcp_intake_and_every_udp_slot() {
    let range = guest_reserved_ports();
    assert_eq!(*range.start(), GUEST_TCP_INTAKE_PORT); // 61_000
    assert_eq!(*range.end(), GUEST_UDP_SLOT_BASE + GUEST_UDP_SLOTS - 1); // 62_024
    assert_eq!(*range.start(), 61_000);
    assert_eq!(*range.end(), 62_024);
}

/// C6 robustness over the whole `u16` domain: a bind is refused only on a port
/// holding an intake of the SAME protocol — TCP on 61_000, UDP on 61_001..=62_024
/// — and succeeds otherwise (TCP 61_001, UDP 61_000 both succeed).
#[test]
#[ignore = "pending DELIVER: reserved-port bind decision (D25-BIND); Tier-3 EADDRINUSE in P6 WS (R5-15)"]
fn reserved_port_bind_refusal_is_protocol_exact() {
    // G-2: `overdrive_init::slots::reserves(port, proto)` — the pure decision the
    // real bind enforces as EADDRINUSE. Authored shape; crafter reconciles.
    use overdrive_init::slots::{Proto, reserves};
    proptest!(|(port in any::<u16>())| {
        let tcp_reserved = reserves(port, Proto::Tcp);
        let udp_reserved = reserves(port, Proto::Udp);
        prop_assert_eq!(tcp_reserved, port == GUEST_TCP_INTAKE_PORT);
        prop_assert_eq!(
            udp_reserved,
            (GUEST_UDP_SLOT_BASE..=GUEST_UDP_SLOT_BASE + GUEST_UDP_SLOTS - 1).contains(&port)
        );
        // Cross-protocol: a UDP slot port is free for TCP; the TCP intake port is free for UDP.
        if (61_001..=62_024).contains(&port) {
            prop_assert!(!tcp_reserved);
        }
        if port == 61_000 {
            prop_assert!(!udp_reserved);
        }
    });
}

// ---------------------------------------------------------------- P3-S2
// D25 slot identity: one slot per (application socket cookie, original
// destination); a socket sending to N destinations holds N slots.

/// A socket to N distinct destinations holds exactly N distinct slots; a repeat
/// send to a destination it already owns reuses that slot (claims none).
#[test]
#[ignore = "pending DELIVER: overdrive-init slot-ownership table (D25); testability gap G-2"]
fn one_slot_per_socket_destination_pair() {
    for n in [0usize, 1, 2, 7] {
        let mut table = SlotTable::new(GUEST_UDP_SLOTS); // 1_024 slots
        let cookie = 42u64;
        for i in 0..n {
            let d = dest(i as u8, 53);
            assert!(matches!(table.on_first_datagram(cookie, d), SlotOutcome::Claimed(_)));
        }
        assert_eq!(table.owned_slots_for(cookie).len(), n);
        // distinct (cookie, dest) over owned slots
        let tags = table.owned_tags_for(cookie);
        assert_eq!(tags.len(), n, "every owned slot has a distinct (socket, destination)");
        // A repeat send to an already-owned destination reuses the slot.
        if n > 0 {
            let d = dest(0, 53);
            assert!(matches!(table.on_first_datagram(cookie, d), SlotOutcome::Reused(_)));
            assert_eq!(table.owned_slots_for(cookie).len(), n, "no new slot on reuse");
        }
    }
}

// ---------------------------------------------------------------- P3-S4
// D25 idle release (120 s both directions) + K-B2 lost-report audit recovery.

/// An association idle in both directions for `GUEST_DATAGRAM_SLOT_IDLE` is
/// released and its slot returns to the pool; the host observes the end.
#[test]
#[ignore = "pending DELIVER: overdrive-init idle release (D25, R5-14); testability gap G-2"]
fn idle_association_is_released_after_120s() {
    assert_eq!(GUEST_DATAGRAM_SLOT_IDLE, Duration::from_secs(120));
    let mut table = SlotTable::new(GUEST_UDP_SLOTS);
    let cookie = 7u64;
    let d = dest(1, 53);
    let slot = match table.on_first_datagram(cookie, d) {
        SlotOutcome::Claimed(s) => s,
        other => panic!("expected a fresh claim, got {other:?}"),
    };
    assert_eq!(table.free_count(), GUEST_UDP_SLOTS as usize - 1);

    // Nothing parked, idle elapses.
    let ended = table.release_idle(GUEST_DATAGRAM_SLOT_IDLE);
    assert_eq!(ended, vec![slot], "the idle slot ends");
    assert_eq!(table.free_count(), GUEST_UDP_SLOTS as usize, "slot returned to pool");
    assert!(table.host_observed_end(slot), "the host observes the association's end");
}

/// K-B2: when the `sock_release` report is lost, the level-triggered audit finds
/// slots whose socket cookie no longer exists and releases them.
#[test]
#[ignore = "pending DELIVER: overdrive-init sock_release audit (D25, K-B2, R5-14); testability gap G-2"]
fn lost_release_report_is_recovered_by_audit() {
    let mut table = SlotTable::new(GUEST_UDP_SLOTS);
    let cookie = 9u64;
    table.on_first_datagram(cookie, dest(2, 53));
    table.on_first_datagram(cookie, dest(3, 53));
    assert_eq!(table.owned_slots_for(cookie).len(), 2);

    // Socket closed but the release report was lost (not delivered to overdrive-init).
    table.mark_socket_cookie_gone(cookie); // models sock_diag: the cookie no longer exists
    let reclaimed = table.audit_orphans(); // the level-triggered audit
    assert_eq!(reclaimed.len(), 2, "both slots of the released socket reclaimed");
    assert_eq!(table.owned_slots_for(cookie).len(), 0);
    assert_eq!(table.free_count(), GUEST_UDP_SLOTS as usize);
}

// ---------------------------------------------------------------- P3-S5
// CELL-RECREATE: end of association recreates cells (never empties) and discards
// parked frames; readiness is dispatched by (slot index, cell generation).

/// A burst parked in a slot is discarded by cell recreation; the slot's next
/// association to another destination receives none of the old frames; a
/// readiness event of an earlier cell generation never starts an association.
#[test]
#[ignore = "pending DELIVER: overdrive-init cell recreation + generation dispatch (CELL-RECREATE, R5-14); gap G-2"]
fn ending_an_association_recreates_cells_and_discards_parked_frames() {
    let mut table = SlotTable::new(GUEST_UDP_SLOTS);
    let cookie = 11u64;
    let d = dest(4, 53);
    let slot = match table.on_first_datagram(cookie, d) {
        SlotOutcome::Claimed(s) => s,
        other => panic!("{other:?}"),
    };
    let gen0 = table.cell_generation(slot);
    // Park a burst (200 x 1_000 B against 4 KiB buffers, the R5-14 shape).
    for _ in 0..200 {
        table.park_frame(slot, 1_000);
    }
    assert!(table.parked_frame_count(slot) > 0);

    // End the association (release): cells recreated, generation advances.
    table.end_association_release(slot);
    let gen1 = table.cell_generation(slot);
    assert!(gen1 > gen0, "recreating the cells advances the generation");
    assert_eq!(
        table.parked_frame_count(slot),
        0,
        "parked frames discarded (recreated, not emptied)"
    );

    // A readiness event registered for the OLD generation is discarded, never
    // starting an association (invariant 14, NoStaleGenerationStart).
    assert_eq!(table.dispatch_readiness(slot, gen0), SlotOutcome::StaleGenerationDiscarded,);
    // A current-generation readiness event (after a new datagram) starts one.
    table.on_first_datagram(cookie, d);
    let starts = table.dispatch_readiness(slot, table.cell_generation(slot));
    assert!(matches!(starts, SlotOutcome::AssociationStarted(_)));
}

/// A slot whose new cell cannot be created is quarantined: it carries no
/// association, is not returned to the pool, and returns only after the audit's
/// retry succeeds (counter `guest_udp_slot_quarantined`).
#[test]
#[ignore = "pending DELIVER: overdrive-init slot quarantine on cell-create failure (CELL-RECREATE, R5-14); gap G-2"]
fn cell_creation_failure_quarantines_the_slot() {
    let mut table = SlotTable::new(GUEST_UDP_SLOTS);
    let cookie = 13u64;
    let d = dest(5, 53);
    let slot = match table.on_first_datagram(cookie, d) {
        SlotOutcome::Claimed(s) => s,
        other => panic!("{other:?}"),
    };
    table.script_next_cell_creation_failure(); // ENOMEM / EMFILE / sockmap insert
    table.end_association_release(slot);
    assert!(table.is_quarantined(slot), "slot quarantined");
    assert!(!table.is_free(slot), "a quarantined slot is not in the pool");
    assert!(table.owned_tags_for(cookie).is_empty(), "detached from its (socket, destination)");

    // The audit retries; on success the slot returns to the pool.
    let reclaimed = table.audit_retry_quarantined();
    assert_eq!(reclaimed, vec![slot]);
    assert!(table.is_free(slot));
}

// ---------------------------------------------------------------- P3-S10
// D25 exhaustion: no free slot → the capture program refuses a new destination
// with EPERM (`guest_udp_slot_exhausted`); existing associations unaffected.

/// The 1_025-th distinct destination of a socket (pool of 1_024) is refused;
/// the 1_024-th succeeds (C1b boundary).
#[test]
#[ignore = "pending DELIVER: guest slot exhaustion (D25); Tier-3 EPERM in P6 WS (R5-15/S10)"]
fn exhausted_slot_pool_refuses_a_new_destination() {
    let mut table = SlotTable::new(GUEST_UDP_SLOTS);
    let cookie = 1u64;
    for i in 0..GUEST_UDP_SLOTS {
        let d = dest((i % 250) as u8, 1000 + i); // distinct (addr, port) tuples
        assert!(
            matches!(table.on_first_datagram(cookie, d), SlotOutcome::Claimed(_)),
            "slot {i} claims"
        );
    }
    assert_eq!(table.free_count(), 0);
    // The 1_025-th distinct destination: refused (the real capture program's EPERM).
    let overflow = table.on_first_datagram(cookie, dest(0, 65_000));
    assert!(
        matches!(overflow, SlotOutcome::Refused { .. }),
        "exhaustion refuses the new destination"
    );
    // Existing associations are unaffected: a repeat send to an owned destination still reuses.
    assert!(matches!(table.on_first_datagram(cookie, dest(0, 1000)), SlotOutcome::Reused(_)));
}

// ---------------------------------------------------------------- token parse
// `overdrive-init` kernel command-line token (replaces overdrive.net=): exactly
// one `overdrive.transport=vsock2,addr=<workload_addr>,dns=<gateway>,launch=<LaunchToken>`,
// fields in that order, no extras. C6 robustness: every malformed field maps to
// a distinct `GuestTransportSetupError::TokenMalformed { field }`.

/// A well-formed token parses to the workload address, gateway and launch token.
#[test]
#[ignore = "pending DELIVER: overdrive-init transport token parse (D15, ATTR-TOKEN); gap G-2"]
fn transport_token_parses_well_formed_cmdline() {
    let token = "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,\
                 launch=0123456789abcdef0123456789abcdef";
    let parsed = parse_transport_token(token).expect("well-formed token parses");
    assert_eq!(parsed.workload_addr(), WL);
    assert_eq!(parsed.gateway(), Ipv4Addr::new(100, 95, 0, 1));
    assert_eq!(parsed.launch().to_string(), "0123456789abcdef0123456789abcdef");
}

/// Each malformed field is a distinct typed error (C6b closed set): a missing
/// token, a duplicate, a bad version/addr/dns/launch, and an extra field.
#[test]
#[ignore = "pending DELIVER: overdrive-init transport token robustness (D15); gap G-2"]
fn transport_token_malformed_fields_map_to_distinct_errors() {
    // Missing entirely.
    assert!(matches!(parse_transport_token(""), Err(GuestTransportSetupError::TokenMissing)));
    // Duplicate token.
    let dup = "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef \
               overdrive.transport=vsock2,addr=100.95.0.8,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef";
    assert!(matches!(parse_transport_token(dup), Err(GuestTransportSetupError::TokenDuplicate)));
    // Bad per-field values → TokenMalformed { field }.
    let cases = [
        (
            "overdrive.transport=vsock9,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef",
            TransportTokenField::Version,
        ),
        (
            "overdrive.transport=vsock2,addr=not-an-ip,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef",
            TransportTokenField::Addr,
        ),
        (
            "overdrive.transport=vsock2,addr=100.95.0.7,dns=not-an-ip,launch=0123456789abcdef0123456789abcdef",
            TransportTokenField::Dns,
        ),
        (
            "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=NOTHEX",
            TransportTokenField::Launch,
        ),
        (
            "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=00000000000000000000000000000000",
            TransportTokenField::Launch,
        ), // all-zero token invalid
        (
            "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef,extra=1",
            TransportTokenField::Extra,
        ),
    ];
    for (input, field) in cases {
        match parse_transport_token(input) {
            Err(GuestTransportSetupError::TokenMalformed { field: f }) => assert_eq!(f, field),
            other => panic!("expected TokenMalformed {{ field: {field:?} }}, got {other:?}"),
        }
    }
}

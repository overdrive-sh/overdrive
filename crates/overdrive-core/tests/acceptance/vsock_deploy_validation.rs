//! P1 — deploy / API validation of guest-transport listener constraints.
//!
//! DISTILL acceptance scaffolds for feature #295. Spec:
//! `docs/feature/netns-density-295/distill/test-scenarios-p1-admission-cid-lease.md`
//! (P1-S10, P1-S11); design pins feature-delta.md §"[REF] Core vocabulary"
//! (`ParseError::ListenerPortReserved`, `ParseError::TooManyTcpListenerPorts`,
//! `guest_reserved_ports()`, `GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX`), decisions
//! D25 / D15-R2 (reserved range) and INTAKE-PORTS, boundary rows R5-21 / R5-44.
//!
//! Both driving ports are overdrive-core (so this file belongs here):
//!   * TOML ingress `overdrive deploy <SPEC>` →
//!     `overdrive_core::aggregate::WorkloadSpecInput::from_toml_str` →
//!     `ParseError::{ListenerPortReserved, TooManyTcpListenerPorts}`;
//!   * API ingress (operator submit) →
//!     `overdrive_core::aggregate::Service::from_submit(ServiceSpecInput)` →
//!     `AggregateError::Validation { field, message }` carrying the same text.
//!
//! RED / gating: the two new `ParseError` variants, the API-ingress checks,
//! and `overdrive_core::guest_transport::{guest_reserved_ports,
//! GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX}` are pinned by the design but do not
//! exist yet; they land with the DELIVER admission/validation step. This module
//! is NOT registered in `tests/acceptance.rs`; the crafter adds
//! `mod vsock_deploy_validation;` when the body co-arrives with that surface.
//! No production API is invented to make it compile.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use overdrive_core::aggregate::{AggregateError, ParseError, Service, VmInput, WorkloadSpecInput};
use overdrive_core::api::submit::{DriverInput, ListenerInput, ResourcesInput, ServiceSpecInput};
use overdrive_core::guest_transport::{
    GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX, guest_reserved_ports,
};
use proptest::prelude::*;

const DELIVER_STEP: &str = "pending DELIVER: vsock admission/validation step (ParseError::{ListenerPortReserved, \
     TooManyTcpListenerPorts} + guest_reserved_ports)";

// ---------------------------------------------------------------------------
// Helpers — minimal canonical VM `[service]` spec (mirrors the existing
// parser's required sections; see tests/acceptance/listener_parser.rs).
// ---------------------------------------------------------------------------

/// A TOML spec whose `[[listener]]` set is the supplied `(port, protocol)`
/// pairs; every other section is canonical-minimal.
fn service_toml_with_listeners(ports: &[(u16, &str)]) -> String {
    let listeners: String = ports
        .iter()
        .map(|(p, proto)| format!("[[listener]]\nport = {p}\nprotocol = \"{proto}\"\n"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "[service]\nid = \"frontend\"\nreplicas = 1\n\n{listeners}\n\
         [vm]\ncommand = \"/opt/frontend/bin/server\"\nargs = []\n\
         kernel = \"/kernel\"\nrootfs = \"/rootfs\"\n\n\
         [resources]\ncpu_milli = 500\nmemory_bytes = 134217728\n"
    )
}

/// The API-ingress submit input mirroring `service_toml_with_listeners`.
fn service_submit_with_listeners(ports: &[(u16, &str)]) -> ServiceSpecInput {
    ServiceSpecInput {
        id: "frontend".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 134_217_728 },
        driver: DriverInput::Vm(VmInput {
            command: "/opt/frontend/bin/server".to_string(),
            args: vec![],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
        listeners: ports
            .iter()
            .map(|(p, proto)| ListenerInput { port: *p, protocol: (*proto).to_string() })
            .collect(),
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

// ===========================================================================
// P1-S10 / R5-21 — reserved-range listener refused (D25).
// ===========================================================================

#[test]
#[ignore = "pending DELIVER: vsock admission/validation step (ParseError::{ListenerPortReserved, TooManyTcpListenerPorts} + guest_reserved_ports)"]
fn deploy_refuses_reserved_listener_port_naming_the_range() {
    let range = guest_reserved_ports();
    let (first, last) = (*range.start(), *range.end());
    // Interior and both reserved endpoints, both protocols, are refused.
    for (port, proto) in
        [(61_000u16, "tcp"), (61_500, "tcp"), (62_024, "udp"), (61_000, "udp"), (62_024, "tcp")]
    {
        let toml = service_toml_with_listeners(&[(port, proto)]);
        match WorkloadSpecInput::from_toml_str(&toml) {
            Err(ParseError::ListenerPortReserved { port: p, first: f, last: l }) => {
                assert_eq!((p, f, l), (port, first, last), "names the offending port + range");
            }
            other => panic!("expected ListenerPortReserved for {proto} {port}, got {other:?}"),
        }
    }
}

#[test]
#[ignore = "pending DELIVER: vsock admission/validation step (ParseError::{ListenerPortReserved, TooManyTcpListenerPorts} + guest_reserved_ports)"]
fn deploy_accepts_ports_adjacent_to_the_reserved_range() {
    // Boundary: 60,999 (just below) and 62,025 (just above) are accepted.
    for (port, proto) in [(60_999u16, "tcp"), (62_025, "udp"), (62_025, "tcp")] {
        let toml = service_toml_with_listeners(&[(port, proto)]);
        assert!(
            WorkloadSpecInput::from_toml_str(&toml).is_ok(),
            "{proto} {port} is outside the reserved range and must be accepted",
        );
    }
}

#[test]
#[ignore = "pending DELIVER: vsock admission/validation step (ParseError::{ListenerPortReserved, TooManyTcpListenerPorts} + guest_reserved_ports)"]
fn api_ingress_refuses_reserved_listener_port_with_validation() {
    // Same rule at the operator submit ingress → AggregateError::Validation
    // keyed on the listener-port field, carrying the ParseError Display text.
    let input = service_submit_with_listeners(&[(61_000, "tcp")]);
    match Service::from_submit(input) {
        Err(AggregateError::Validation { field, message }) => {
            assert_eq!(field, "listeners[].port");
            assert!(message.contains("reserved"), "message names the reservation: {message}");
        }
        other => panic!("expected Validation for reserved port, got {other:?}"),
    }
}

proptest! {
    // Property (R5-21 boundaries): a single-listener spec is refused iff its
    // port lies inside guest_reserved_ports(), for either protocol.
    #[test]
    #[ignore = "pending DELIVER: vsock admission/validation step (ParseError::{ListenerPortReserved, TooManyTcpListenerPorts} + guest_reserved_ports)"]
    fn deploy_refusal_iff_port_in_reserved_range(
        port in 60_000u16..=63_000,
        tcp in any::<bool>(),
    ) {
        let proto = if tcp { "tcp" } else { "udp" };
        let in_range = guest_reserved_ports().contains(&port);
        let toml = service_toml_with_listeners(&[(port, proto)]);
        let refused = matches!(
            WorkloadSpecInput::from_toml_str(&toml),
            Err(ParseError::ListenerPortReserved { .. })
        );
        prop_assert_eq!(refused, in_range, "refused iff inside the reserved range");
    }
}

// ===========================================================================
// P1-S11 / R5-44 — too many declared TCP listener ports (INTAKE-PORTS).
// ===========================================================================

#[test]
#[ignore = "pending DELIVER: vsock admission/validation step (ParseError::{ListenerPortReserved, TooManyTcpListenerPorts} + guest_reserved_ports)"]
fn deploy_accepts_exactly_the_block_size_and_refuses_one_more() {
    let max = GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX; // 4,096
    // Distinct TCP ports drawn from a band disjoint from the reserved range.
    let at_cap: Vec<(u16, &str)> = (0..max).map(|i| (1_000 + i, "tcp")).collect();
    assert!(
        WorkloadSpecInput::from_toml_str(&service_toml_with_listeners(&at_cap)).is_ok(),
        "exactly {max} distinct TCP listener ports is accepted",
    );

    let over_cap: Vec<(u16, &str)> = (0..=max).map(|i| (1_000 + i, "tcp")).collect();
    match WorkloadSpecInput::from_toml_str(&service_toml_with_listeners(&over_cap)) {
        Err(ParseError::TooManyTcpListenerPorts { declared, max: m }) => {
            assert_eq!((declared, m), (u32::from(max) + 1, max));
        }
        other => panic!("expected TooManyTcpListenerPorts at {}, got {other:?}", max + 1),
    }
}

#[test]
#[ignore = "pending DELIVER: vsock admission/validation step (ParseError::{ListenerPortReserved, TooManyTcpListenerPorts} + guest_reserved_ports)"]
fn api_ingress_refuses_too_many_tcp_listener_ports_with_validation() {
    let max = GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX;
    let over_cap: Vec<(u16, &str)> = (0..=max).map(|i| (1_000 + i, "tcp")).collect();
    match Service::from_submit(service_submit_with_listeners(&over_cap)) {
        Err(AggregateError::Validation { field, .. }) => assert_eq!(field, "listeners"),
        other => panic!("expected Validation {{ field: listeners }}, got {other:?}"),
    }
}

proptest! {
    // Property (R5-44): refusal iff the count of DISTINCT TCP listener ports
    // exceeds the constant — UDP ports do not count toward the TCP limit.
    #[test]
    #[ignore = "pending DELIVER: vsock admission/validation step (ParseError::{ListenerPortReserved, TooManyTcpListenerPorts} + guest_reserved_ports)"]
    fn deploy_refusal_iff_distinct_tcp_ports_over_the_constant(
        tcp_count in (GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX - 2)..=(GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX + 2),
    ) {
        let max = GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX;
        let ports: Vec<(u16, &str)> = (0..tcp_count).map(|i| (1_000 + i, "tcp")).collect();
        let refused = matches!(
            WorkloadSpecInput::from_toml_str(&service_toml_with_listeners(&ports)),
            Err(ParseError::TooManyTcpListenerPorts { .. })
        );
        prop_assert_eq!(refused, tcp_count > max, "refused iff distinct TCP ports > the constant");
    }
}

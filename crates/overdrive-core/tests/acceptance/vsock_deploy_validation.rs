//! P1-S10 — deploy / API validation refuses a listener port the guest
//! reserves for its intakes (GH #295, D25, boundary row R5-21), and a spec
//! whose probes across startup, readiness and liveness exceed the
//! per-allocation cap `GUEST_PROBE_SLOTS` (D-UP-12 / PD-3 / EVMOD-AB,
//! boundary row R5-60).
//!
//! Contract: `docs/feature/netns-density-295/feature-delta.md` § "[REF] Core
//! vocabulary — `overdrive-core`" — **Deploy refusal of a reserved port (D25)**
//! (`ParseError::ListenerPortReserved`, `guest_reserved_ports()`; the API
//! ingress returns `AggregateError::Validation { field: "listeners[].port",
//! message }` with the `ParseError` `Display` text as `message`).
//!
//! Both driving ports are `overdrive-core`:
//!   * TOML ingress `overdrive deploy <SPEC>` →
//!     `overdrive_core::aggregate::WorkloadSpecInput::from_toml_str`;
//!   * API ingress (operator submit) →
//!     `overdrive_core::aggregate::Service::from_submit(ServiceSpecInput)`.
//!
//! The probe cap (§ "Per-allocation probe cap"): the TOML ingress returns
//! `ParseError::ProbeCountExceeded { count, max }` after the three role
//! arrays are parsed and ADR-0058's default startup probe is inferred (so an
//! inferred probe counts); the API ingress returns `AggregateError::Validation
//! { field: "probes", message }` with that `Display` text. Both read
//! `GUEST_PROBE_SLOTS`; neither spells 8.
//!
//! RED: `guest_reserved_ports()` panics and neither ingress checks the range
//! or the probe count until DELIVER step 2 lands them. The removed
//! INTAKE-PORTS limit (ADR-0179, removed by INB-1) has no cell here. The
//! seeded-sim half of R5-60 (eight concurrent probe dials, each taking a node
//! flow slot) is specified against the production owner in
//! `overdrive-control-plane`.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    reason = "test bodies unwrap fixtures they construct; the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use overdrive_core::aggregate::{
    AggregateError, DriverInput, ParseError, ProbeDescriptor, ProbeMechanic, ResourcesInput,
    Service, VmInput, WorkloadSpecInput,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::guest_transport::{GUEST_PROBE_SLOTS, guest_reserved_ports};
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use proptest::prelude::*;

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

/// CONTRACT_SHAPE: pure-function.
#[test]
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
    // Boundary: 60,999 (just below) and 62,025 (just above) stay accepted.
    for (port, proto) in [(60_999u16, "tcp"), (60_999, "udp"), (62_025, "udp"), (62_025, "tcp")] {
        let toml = service_toml_with_listeners(&[(port, proto)]);
        assert!(
            WorkloadSpecInput::from_toml_str(&toml).is_ok(),
            "{proto} {port} is outside the reserved range and must be accepted",
        );
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn api_ingress_refuses_reserved_listener_port_with_validation() {
    // Same rule at the operator submit ingress → AggregateError::Validation
    // keyed on the listener-port field, carrying the ParseError Display text.
    let range = guest_reserved_ports();
    for (port, proto) in [(61_000u16, "tcp"), (61_500, "udp"), (62_024, "tcp")] {
        let input = service_submit_with_listeners(&[(8080, "tcp"), (port, proto)]);
        match Service::from_submit(input) {
            Err(AggregateError::Validation { field, message }) => {
                assert_eq!(field, "listeners[].port");
                let expected = ParseError::ListenerPortReserved {
                    port,
                    first: *range.start(),
                    last: *range.end(),
                }
                .to_string();
                assert_eq!(message, expected, "the ParseError Display text, verbatim");
            }
            other => panic!("expected Validation for reserved {proto} {port}, got {other:?}"),
        }
    }
    // Boundary: the ports just outside the range submit.
    for port in [60_999u16, 62_025] {
        assert!(
            Service::from_submit(service_submit_with_listeners(&[(port, "tcp")])).is_ok(),
            "{port} is outside the reserved range and must be accepted",
        );
    }
}

/// A spec mixing an ordinary listener and a reserved one is refused, naming
/// the reserved port — the ordinary listener does not mask it.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn deploy_refuses_a_reserved_port_among_ordinary_listeners() {
    let toml = service_toml_with_listeners(&[(8080, "tcp"), (53, "udp"), (61_001, "udp")]);
    match WorkloadSpecInput::from_toml_str(&toml) {
        Err(ParseError::ListenerPortReserved { port, .. }) => assert_eq!(port, 61_001),
        other => panic!("expected ListenerPortReserved for 61001, got {other:?}"),
    }
}

/// The refusal text names the port and the range and tells the operator what
/// to do (D25).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_reserved_port_refusal_renders_its_pinned_message() {
    assert_eq!(
        ParseError::ListenerPortReserved { port: 61_000, first: 61_000, last: 62_024 }.to_string(),
        "listener port 61000 is reserved for the guest transport (61000..=62024); choose a port outside this range"
    );
}

proptest! {
    // Property (R5-21 boundaries): a single-listener spec is refused iff its
    // port lies inside guest_reserved_ports(), for either protocol.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
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
// R5-60 — the per-allocation probe cap (D-UP-12), at both ingresses.
// ===========================================================================

/// A TOML spec with one TCP listener on 8080 and `startup` / `readiness` /
/// `liveness` TCP probes against it. `startup == 0` omits the startup
/// section, so ADR-0058 infers one default startup probe.
fn service_toml_with_probes(startup: usize, readiness: usize, liveness: usize) -> String {
    let section = |role: &str, n: usize| -> String {
        format!("[[health_check.{role}]]\ntype = \"tcp\"\nport = 8080\n\n").repeat(n)
    };
    format!(
        "{}\n{}{}{}",
        service_toml_with_listeners(&[(8080, "tcp")]),
        section("startup", startup),
        section("readiness", readiness),
        section("liveness", liveness),
    )
}

/// One operator-declared TCP probe of `role` at position `idx`, with the
/// role's ADR-0057 defaults.
fn tcp_probe(role: ProbeRole, idx: u32) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(idx),
        role,
        mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port: 8080 },
        timeout_seconds: 5,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: matches!(role, ProbeRole::Liveness).then_some(3),
        success_threshold: matches!(role, ProbeRole::Readiness).then_some(1),
        inferred: false,
    }
}

/// The API-ingress submit input with the given probe counts per role (no
/// inference at this ingress: the vectors are the intent's).
fn service_submit_with_probes(startup: u32, readiness: u32, liveness: u32) -> ServiceSpecInput {
    let mut input = service_submit_with_listeners(&[(8080, "tcp")]);
    input.startup_probes = (0..startup).map(|i| tcp_probe(ProbeRole::Startup, i)).collect();
    input.readiness_probes = (0..readiness).map(|i| tcp_probe(ProbeRole::Readiness, i)).collect();
    input.liveness_probes = (0..liveness).map(|i| tcp_probe(ProbeRole::Liveness, i)).collect();
    input
}

/// R5-60: eight probes in total deploy — spread over the three roles, or all
/// in one role (several probes of one role stay supported) — and every probe
/// survives into the intent.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn deploy_accepts_eight_probes_in_total_in_any_shape() {
    for (startup, readiness, liveness) in [(3usize, 3usize, 2usize), (8, 0, 0), (1, 0, 7)] {
        let toml = service_toml_with_probes(startup, readiness, liveness);
        match WorkloadSpecInput::from_toml_str(&toml) {
            Ok(WorkloadSpecInput::Service(spec)) => {
                assert_eq!(
                    (
                        spec.startup_probes.len(),
                        spec.readiness_probes.len(),
                        spec.liveness_probes.len()
                    ),
                    (startup, readiness, liveness),
                    "every declared probe survives"
                );
            }
            other => panic!(
                "expected an accepted service for {startup}/{readiness}/{liveness}, got {other:?}"
            ),
        }
    }
}

/// R5-60: nine probes are refused before any intent exists — eight declared
/// readiness and liveness probes with no startup section, whose inferred
/// default startup probe is the ninth, and nine declared probes.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn deploy_refuses_nine_probes_counting_the_inferred_startup_probe() {
    for (startup, readiness, liveness) in [(0usize, 4usize, 4usize), (9, 0, 0), (3, 3, 3)] {
        let toml = service_toml_with_probes(startup, readiness, liveness);
        match WorkloadSpecInput::from_toml_str(&toml) {
            Err(ParseError::ProbeCountExceeded { count, max }) => {
                assert_eq!(
                    (count, max),
                    (9, GUEST_PROBE_SLOTS),
                    "{startup}/{readiness}/{liveness}"
                );
            }
            other => panic!(
                "expected ProbeCountExceeded for {startup}/{readiness}/{liveness}, got {other:?}"
            ),
        }
    }
}

/// R5-60: the API ingress refuses the same counts with `Validation { field:
/// "probes" }` carrying the `ProbeCountExceeded` text, and accepts eight.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn api_ingress_refuses_nine_probes_with_validation() {
    let expected = ParseError::ProbeCountExceeded { count: 9, max: GUEST_PROBE_SLOTS }.to_string();
    for (startup, readiness, liveness) in [(1u32, 4u32, 4u32), (9, 0, 0), (0, 4, 5)] {
        match Service::from_submit(service_submit_with_probes(startup, readiness, liveness)) {
            Err(AggregateError::Validation { field, message }) => {
                assert_eq!(field, "probes");
                assert_eq!(message, expected, "the ParseError Display text, verbatim");
            }
            other => {
                panic!("expected Validation for {startup}/{readiness}/{liveness}, got {other:?}")
            }
        }
    }
    for (startup, readiness, liveness) in [(3u32, 3u32, 2u32), (0, 0, 8)] {
        assert!(
            Service::from_submit(service_submit_with_probes(startup, readiness, liveness)).is_ok(),
            "{startup}/{readiness}/{liveness}: eight probes are within the cap"
        );
    }
}

/// The probe-cap refusal names the count, the inferred probe and the cap.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_probe_cap_refusal_renders_its_pinned_message() {
    assert_eq!(
        ParseError::ProbeCountExceeded { count: 9, max: 8 }.to_string(),
        "[[health_check.*]] declares 9 probes across startup, readiness and liveness (including any inferred default startup probe); at most 8 probes per workload are supported"
    );
}

proptest! {
    // Property (R5-60 count boundary): a spec is refused iff its probe count
    // — the declared probes plus an inferred default startup probe when no
    // startup section is written — exceeds GUEST_PROBE_SLOTS.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn deploy_refusal_iff_the_probe_count_exceeds_the_cap(
        startup in 0usize..=6,
        readiness in 0usize..=6,
        liveness in 0usize..=6,
    ) {
        let inferred = usize::from(startup == 0);
        let count = startup + readiness + liveness + inferred;
        let max = usize::try_from(GUEST_PROBE_SLOTS).expect("fits usize");
        let parsed = WorkloadSpecInput::from_toml_str(&service_toml_with_probes(startup, readiness, liveness));
        if count > max {
            let refused_with_the_count = matches!(
                parsed,
                Err(ParseError::ProbeCountExceeded { count: c, max: m })
                    if c == count && m == GUEST_PROBE_SLOTS
            );
            prop_assert!(refused_with_the_count, "{} probes: expected ProbeCountExceeded, got {:?}", count, parsed);
        } else {
            prop_assert!(parsed.is_ok(), "{} probes are within the cap", count);
        }
    }
}

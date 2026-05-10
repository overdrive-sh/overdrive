//! S-08-10 — Service `[[listener]]` round-trip property tests.
//!
//! Driving port: `WorkloadSpecInput::from_toml_str` round-trips through
//! TOML serialisation — render the listeners as a TOML body, parse them
//! back, the resulting `ServiceSpec` listeners equal the original.
//!
//! Per `.claude/rules/testing.md` § "Property-based testing (proptest)"
//! — the TOML-render → parse round-trip is a mandatory call site for
//! the listener type, since [`Listener`] is a Phase-1 newtype shape
//! that crosses the operator-facing serialisation boundary.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::net::Ipv4Addr;

use overdrive_core::aggregate::{Listener, ServiceVip, WorkloadSpecInput};
use overdrive_core::dataplane::backend_key::Proto;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// `Proto` is the closed `tcp` / `udp` set per Phase 2.2 dataplane
/// support; the listener parser admits exactly those two.
fn arb_proto() -> impl Strategy<Value = Proto> {
    prop_oneof![Just(Proto::Tcp), Just(Proto::Udp)]
}

/// Generate a `ServiceVip`. Constrained to the 10.0.0.0/24 RFC1918
/// space — the absolute octet values do not matter for round-trip
/// purposes; only that they survive TOML render → parse.
fn arb_service_vip() -> impl Strategy<Value = ServiceVip> {
    (1u8..=254u8).prop_map(|d| ServiceVip(Ipv4Addr::new(10, 0, 0, d)))
}

/// Generate a `Listener`. The (vip, port, protocol) triple is the
/// shape per ADR-0047 §1 — pinned-VIP and pending-VIP listeners are
/// generated in roughly equal proportion.
fn arb_listener() -> impl Strategy<Value = Listener> {
    (1u16..=65535u16, arb_proto(), prop_oneof![Just(None), arb_service_vip().prop_map(Some)])
        .prop_map(|(port, protocol, vip)| Listener {
            port: std::num::NonZeroU16::new(port).expect("port > 0 by generator constraint"),
            protocol,
            vip,
        })
}

/// Generate 1..=8 listeners with distinct `(vip, port, protocol)`
/// triples per S-08-10. Uniqueness is enforced by post-filtering the
/// raw vector; a small case-budget rejection is tolerated by proptest.
fn arb_distinct_listeners() -> impl Strategy<Value = Vec<Listener>> {
    prop::collection::vec(arb_listener(), 1..=8).prop_filter(
        "listeners must be triple-distinct",
        |ls| {
            let mut seen = std::collections::HashSet::with_capacity(ls.len());
            for l in ls {
                if !seen.insert((l.vip, l.port, l.protocol)) {
                    return false;
                }
            }
            true
        },
    )
}

// ---------------------------------------------------------------------------
// Helpers — render listeners to a TOML body the parser can re-read.
// ---------------------------------------------------------------------------

fn render_service_toml(listeners: &[Listener]) -> String {
    use std::fmt::Write as _;
    let mut s = String::from(
        r#"
[service]
id = "frontend"
replicas = 1

"#,
    );
    for l in listeners {
        s.push_str("[[listener]]\n");
        let _ = writeln!(s, "port = {}", l.port.get());
        let _ = writeln!(s, "protocol = \"{}\"", l.protocol);
        if let Some(v) = l.vip {
            let _ = writeln!(s, "vip = \"{v}\"");
        }
        s.push('\n');
    }
    s.push_str(
        r#"
[exec]
command = "/opt/frontend/bin/server"
args = []

[resources]
cpu_milli = 500
memory_bytes = 134217728
"#,
    );
    s
}

// ---------------------------------------------------------------------------
// Property — declaration order is preserved AND every triple is
// byte-equivalent across TOML render → parse.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1024,
        ..ProptestConfig::default()
    })]

    /// S-08-10 — Listener round-trip byte-equivalence: render to TOML,
    /// parse back, every (vip, port, protocol) triple equals the
    /// original in declaration order.
    #[test]
    fn s_08_10_listeners_round_trip_byte_equivalent(
        listeners in arb_distinct_listeners(),
    ) {
        let toml = render_service_toml(&listeners);
        let parsed = WorkloadSpecInput::from_toml_str(&toml)
            .expect("rendered service TOML must parse back");

        let svc = match parsed {
            WorkloadSpecInput::Service(s) => s,
            other => panic!("expected Service variant, got {other:?}"),
        };

        prop_assert_eq!(
            svc.listeners.len(),
            listeners.len(),
            "listener count must round-trip"
        );
        for (i, (a, b)) in listeners.iter().zip(svc.listeners.iter()).enumerate() {
            prop_assert_eq!(a.port, b.port, "listener[{}] port must round-trip", i);
            prop_assert_eq!(a.protocol, b.protocol, "listener[{}] protocol must round-trip", i);
            prop_assert_eq!(a.vip, b.vip, "listener[{}] vip must round-trip", i);
        }
    }
}

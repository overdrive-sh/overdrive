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

use overdrive_core::aggregate::{Listener, WorkloadSpecInput};
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

/// Generate a `Listener`. Per ADR-0049 § 5 / service-vip-allocator
/// step 02-01 the per-listener `vip` axis was removed — VIPs are
/// platform-issued service-wide. The generator carries only the
/// `(port, protocol)` pair.
fn arb_listener() -> impl Strategy<Value = Listener> {
    (1u16..=65535u16, arb_proto()).prop_map(|(port, protocol)| Listener {
        port: std::num::NonZeroU16::new(port).expect("port > 0 by generator constraint"),
        protocol,
    })
}

/// Generate 1..=8 listeners with distinct `(port, protocol)` pairs.
/// Per ADR-0049 § 5 the `vip` axis is gone from the uniqueness rule.
/// Uniqueness is enforced by post-filtering the raw vector; a small
/// case-budget rejection is tolerated by proptest.
fn arb_distinct_listeners() -> impl Strategy<Value = Vec<Listener>> {
    prop::collection::vec(arb_listener(), 1..=8).prop_filter(
        "listeners must be (port, protocol)-distinct",
        |ls| {
            let mut seen = std::collections::HashSet::with_capacity(ls.len());
            for l in ls {
                if !seen.insert((l.port, l.protocol)) {
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
    /// parse back, every (port, protocol) pair equals the original in
    /// declaration order. Per ADR-0049 § 5 / service-vip-allocator
    /// step 02-01 the `vip` axis is gone from the per-listener shape.
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
        }
    }
}

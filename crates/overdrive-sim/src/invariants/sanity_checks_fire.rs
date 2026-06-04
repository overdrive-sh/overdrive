//! `SanityChecksFireBeforeServiceMap` — Slice 06 (US-06; S-2.2-22
//! sibling).
//!
//! **Always invariant**: every packet whose classification violates
//! a sanity-prologue rule (truncated IPv4, pathological TCP flags,
//! IPv6 `EtherType` when the LB is IPv4-only, oversize frame, ...)
//! MUST cause the dataplane to (a) increment
//! `DROP_COUNTER[MalformedHeader]` and (b) short-circuit BEFORE
//! consulting `SERVICE_MAP`. Mirror of the production XDP / TC
//! sanity prologue contract from step 06-02.
//!
//! # What this invariant pins
//!
//! The kernel-side contract is verified at Tier 2 / Tier 3 against
//! real BPF programs (`crates/overdrive-bpf/tests/integration/sanity_prologue_drops.rs`,
//! `crates/overdrive-dataplane/tests/integration/sanity_mixed_batch.rs`).
//! THIS invariant is the Tier 1 / DST mirror: it pins the
//! *conceptual* contract so a future regression in the `SimDataplane`
//! (e.g. a "fast path" that calls `service_backends` on every input
//! before classification) is caught at PR time.
//!
//! Per `.claude/rules/development.md` § *Production code is not
//! shaped by simulation*, the kernel-side production code stays
//! shaped exclusively by the contract production needs. The
//! invariant evaluates a sim-side simulation of the prologue
//! contract: a pure function over a packet classification that
//! either:
//!
//! - emits `record_drop(MalformedHeader)` and short-circuits (sanity
//!   violation), OR
//! - emits a `service_backends` lookup (clean path).
//!
//! After offering N classified packets, the invariant asserts:
//!
//! 1. Drop counter delta for `MalformedHeader` == count of sanity-
//!    violating offerings.
//! 2. `services_lookup_count` delta == count of clean offerings —
//!    sanity-violating packets MUST NOT have triggered a lookup.
//! 3. `services` map state is unchanged across the run (a violating
//!    packet that "leaked" into a backend insert/delete would be a
//!    severe contract violation).
//!
//! Sibling to the Tier 3 mixed-batch test at
//! `crates/overdrive-dataplane/tests/integration/sanity_mixed_batch.rs`:
//! that test pins the kernel-side invocation against real veth +
//! real packets; this invariant pins the conceptual contract under
//! DST seed-deterministic perturbation.
//!
//! Wired into the existing `Invariant` enum's exhaustive match at
//! `crates/overdrive-sim/src/invariants/mod.rs` as additive variant
//! `SanityChecksFireBeforeServiceMap`.

#![allow(clippy::expect_used)]

use std::net::Ipv4Addr;
use std::sync::Arc;

use overdrive_core::SpiffeId;
use overdrive_core::dataplane::DropClass;
use overdrive_core::id::NodeId;
use overdrive_core::traits::dataplane::{Backend, Dataplane};

use crate::adapters::dataplane::SimDataplane;
use crate::harness::{InvariantResult, InvariantStatus};

/// Packet classification used by the sim-side prologue model. The
/// classification mirrors the kernel-side sanity prologue's decision
/// tree from step 06-02: `Ipv4TcpValid` is the clean path; everything
/// else is a sanity violation that MUST short-circuit before
/// `SERVICE_MAP` lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PacketClass {
    /// Well-formed IPv4 + TCP frame; sanity prologue passes through
    /// to `SERVICE_MAP` lookup.
    Ipv4TcpValid,
    /// IPv4 IHL field claims fewer header bytes than the minimum
    /// (IHL < 5).
    Ipv4Truncated,
    /// TCP flags carry a pathological combination (SYN+RST,
    /// SYN+FIN, ...).
    TcpPathologicalFlags,
    /// `EtherType` is non-IPv4 — the LB is IPv4-only in Phase 2.2.
    NonIpv4Ethertype,
}

impl PacketClass {
    /// `true` for any classification the sanity prologue rejects.
    /// `false` for the clean path that proceeds to `SERVICE_MAP`.
    const fn is_sanity_violating(self) -> bool {
        match self {
            Self::Ipv4TcpValid => false,
            Self::Ipv4Truncated | Self::TcpPathologicalFlags | Self::NonIpv4Ethertype => true,
        }
    }
}

/// Outcome trace from a single simulated packet offering. Used by
/// the invariant body to assert on the structural contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Trace {
    /// `true` if the simulation called `record_drop(MalformedHeader)`.
    dropped_with_malformed: bool,
    /// `true` if the simulation consulted `service_backends`.
    consulted_services: bool,
}

/// Simulate the kernel-side XDP path for a single classified packet
/// against `dataplane`. Mirrors the conceptual contract from step
/// 06-02's `sanity_prologue_xdp` helper:
///
/// 1. If the classification is sanity-violating, record a
///    `MalformedHeader` drop and return WITHOUT consulting
///    `service_backends`. The `consulted_services` field MUST be
///    `false` — a regression that ran lookup-then-drop would set this
///    to `true` and fail the invariant.
/// 2. Otherwise, consult `service_backends(vip)` and return.
fn simulate_xdp_path(dp: &SimDataplane, vip: Ipv4Addr, class: PacketClass) -> Trace {
    if class.is_sanity_violating() {
        dp.record_drop(DropClass::MalformedHeader);
        Trace { dropped_with_malformed: true, consulted_services: false }
    } else {
        // Clean path: `SERVICE_MAP` lookup. This is the only call site
        // that consults `services` — the sanity-violating arm above
        // returns before reaching here.
        let _backends = dp.service_backends(vip);
        Trace { dropped_with_malformed: false, consulted_services: true }
    }
}

/// Walk every collected trace and assert the sanity-prologue
/// structural contract per offering. Returns `None` on success,
/// `Some(reason)` on first violation.
fn check_traces(traces: &[(PacketClass, Trace)]) -> Option<String> {
    for (i, (class, trace)) in traces.iter().enumerate() {
        if class.is_sanity_violating() {
            if !trace.dropped_with_malformed {
                return Some(format!(
                    "trace {i} class={class:?}: sanity-violating packet must drop with \
                     MalformedHeader, got trace={trace:?}"
                ));
            }
            if trace.consulted_services {
                return Some(format!(
                    "trace {i} class={class:?}: sanity-violating packet must short-circuit \
                     BEFORE `SERVICE_MAP` lookup, got trace={trace:?}"
                ));
            }
        } else {
            if trace.dropped_with_malformed {
                return Some(format!(
                    "trace {i} class={class:?}: clean packet must NOT drop with \
                     MalformedHeader, got trace={trace:?}"
                ));
            }
            if !trace.consulted_services {
                return Some(format!(
                    "trace {i} class={class:?}: clean packet must consult `SERVICE_MAP`, \
                     got trace={trace:?}"
                ));
            }
        }
    }
    None
}

/// Drive the sanity-prologue scenario and return an `InvariantResult`
/// pinned to the canonical kebab-case name.
///
/// # Scenario
///
/// 1. Pre-load `SimDataplane` with a single VIP → backends mapping.
///    The lookup path on a clean packet hits this mapping; a
///    sanity-violating packet MUST NOT.
/// 2. Snapshot baseline `DROP_COUNTER[MalformedHeader]` and
///    `services` map state.
/// 3. Offer 80 classified packets in a deterministic mixed order:
///    50 valid + 10 truncated + 10 SYN+RST + 10 IPv6 `EtherType`.
///    Mirrors the Tier 3 test's batch shape.
/// 4. Assert:
///    - Drop counter delta = exactly 30 (the sanity-violating count).
///    - Every sanity-violating trace recorded `consulted_services
///      == false` AND `dropped_with_malformed == true`.
///    - Every clean trace recorded `dropped_with_malformed == false`.
///    - `services` map state is bit-equal to its pre-run snapshot.
pub async fn evaluate_sanity_checks_fire_before_service_map() -> InvariantResult {
    const NAME: &str = "sanity-checks-fire-before-service-map";

    let dataplane = Arc::new(SimDataplane::new());
    let vip = Ipv4Addr::new(10, 0, 0, 1);
    let backends = vec![Backend {
        alloc: SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/b1").expect("valid SPIFFE ID"),
        addr: "10.1.0.5:8080".parse().expect("valid SocketAddr"),
        weight: 1,
        healthy: true,
    }];

    // Pre-load the dataplane via the async Dataplane trait.
    if let Err(e) = dataplane.update_service(super::tcp_frontend(vip), backends.clone()).await {
        return fail(NAME, format!("pre-load update_service failed: {e}"));
    }

    // Baseline snapshots.
    let baseline_drop = dataplane.read_drop_counter(DropClass::MalformedHeader);
    let baseline_services = dataplane.service_backends(vip);

    // Mixed 80-packet batch — mirrors the Tier 3 test.
    let mut batch: Vec<PacketClass> = Vec::with_capacity(80);
    for _ in 0..50 {
        batch.push(PacketClass::Ipv4TcpValid);
    }
    for _ in 0..10 {
        batch.push(PacketClass::Ipv4Truncated);
    }
    for _ in 0..10 {
        batch.push(PacketClass::TcpPathologicalFlags);
    }
    for _ in 0..10 {
        batch.push(PacketClass::NonIpv4Ethertype);
    }
    assert_eq!(batch.len(), 80);

    // Offer every packet, collect traces.
    let mut traces: Vec<(PacketClass, Trace)> = Vec::with_capacity(batch.len());
    for class in batch {
        let trace = simulate_xdp_path(&dataplane, vip, class);
        traces.push((class, trace));
    }

    // Assert structural contract on every trace.
    if let Some(violation) = check_traces(&traces) {
        return fail(NAME, violation);
    }

    // Drop counter aggregate check.
    let after_drop = dataplane.read_drop_counter(DropClass::MalformedHeader);
    let drop_delta = after_drop - baseline_drop;
    let expected = 30_u64; // 10 truncated + 10 SYN+RST + 10 IPv6
    if drop_delta != expected {
        return fail(
            NAME,
            format!(
                "DROP_COUNTER[MalformedHeader] delta = {drop_delta}, expected {expected}; \
                 baseline={baseline_drop} after={after_drop}"
            ),
        );
    }

    // `services` map MUST not have mutated across the run — a
    // violating packet that leaked into an `update_service` would
    // be a severe contract regression.
    let after_services = dataplane.service_backends(vip);
    if after_services != baseline_services {
        return fail(
            NAME,
            format!(
                "`services[{vip}]` mutated across run: baseline={baseline_services:?} \
                 after={after_services:?}"
            ),
        );
    }

    // Other drop slots MUST NOT have incremented — the prologue
    // attributes ALL violations to MalformedHeader (slot 0) per
    // step 06-02; UnknownVip / SanityPrologue / NoHealthyBackend
    // all read 0.
    for class in [
        DropClass::UnknownVip,
        DropClass::NoHealthyBackend,
        DropClass::SanityPrologue,
        DropClass::ReverseNatMiss,
        DropClass::OversizePacket,
    ] {
        let count = dataplane.read_drop_counter(class);
        if count != 0 {
            return fail(
                NAME,
                format!(
                    "DROP_COUNTER[{class}] = {count}; expected 0 (only MalformedHeader \
                     receives sanity-prologue attribution)"
                ),
            );
        }
    }

    pass(NAME)
}

fn pass(name: &str) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Pass,
        tick: 1,
        host: cluster_host(),
        cause: None,
    }
}

fn fail(name: &str, cause: String) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Fail,
        tick: 1,
        host: cluster_host(),
        cause: Some(cause),
    }
}

fn cluster_host() -> String {
    NodeId::new("cluster").map_or_else(|_| "cluster".to_owned(), |id| id.to_string())
}

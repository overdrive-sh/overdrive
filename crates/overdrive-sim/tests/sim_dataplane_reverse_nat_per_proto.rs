//! Tier-1 (DST / in-memory) acceptance scaffolds for the per-proto
//! `REVERSE_NAT` key set + the lockstep set-equality gate
//! (udp-service-support US-02 / US-03; ADR-0060 D4 + § Enforcement).
//!
//! **RED scaffolds.** Each test is `#[should_panic(expected = "RED
//! scaffold")]` with a body that `panic!`s naming the scenario, per
//! `.claude/rules/testing.md` § "Test-side scaffolds". GREEN: DELIVER
//! narrows `SimDataplane::reverse_nat_keys_for`'s `[Tcp, Udp]` hardcode
//! (`crates/overdrive-sim/src/adapters/dataplane.rs:277`) and the
//! `ReverseNatLockstep` invariant's `[Tcp, Udp]` walk
//! (`reverse_nat_lockstep.rs:123,161`) to the declared `frontend.proto`,
//! drops `#[should_panic]`, and fills the real assertions — in the same
//! single-cut US-01/US-02 PR.
//!
//! Scenario SSOT:
//! `docs/feature/udp-service-support/distill/test-scenarios.md`
//! - S-03-A: Sim installs EXACTLY the declared-proto key set (property)
//! - S-03-B: NEGATIVE — a dropped fan-out key fails the lockstep
//! - S-03-C: NEGATIVE — an extra (phantom) key fails the lockstep (orphan check)
//! - S-03-D: the #163 shape (tcp-only for a udp service) is caught
//! - S-02-A: empty backends purge only frontend.proto's keys (property)
//! - S-02-B: cross-service shared key survives a per-proto purge
//! - S-02-C: idempotent re-apply (property)
//! - S-02-D: non-IPv4 backend contributes no key (boundary)
//!
//! Mandate 8 (Universe-bound assertion). The universe is the
//! port-observable `BTreeSet<BackendKey>` `REVERSE_NAT` key set (+ the
//! forward service map). The expected assertion is exact set-equality
//! against `{ BackendKey{ ip, port, frontend.proto } : backend }`; an
//! unexpected extra key fails the orphan-direction check (fail-closed) —
//! see S-03-C. This native set-equality IS the Rust equivalent of
//! `assert_state_delta(.., strict=True)`; see
//! `docs/architecture/atdd-infrastructure-policy.md` § "Mandate 8 mapping".
//!
//! Mandate 9. S-03-A, S-02-A, S-02-C are `@property` and live at Tier 1
//! (layer 1) — GREEN versions SHOULD use `proptest` over backend sets ×
//! `{Tcp, Udp}`. S-03-B/C/D and S-02-B/D are example-pinned negatives /
//! boundaries.

#![allow(clippy::expect_used, clippy::unwrap_used)]

// S-03-A — Property: the SimDataplane `REVERSE_NAT` key set for a service
// equals exactly the keys derived from (frontend.proto, backends), with
// NO key for any other protocol.
//
// GREEN: proptest over (IPv4 backends, proto in {Tcp, Udp}); apply
// update_service(frontend_P, backends); assert the reverse_nat key set
// for the VIP == { BackendKey{ip, port, P} } exactly.
#[test]
#[should_panic(expected = "RED scaffold")]
fn sim_installs_exactly_the_declared_proto_key_set() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03-A / Sim installs exactly the \
         declared-frontend.proto BTreeSet(BackendKey), no other-proto key)"
    );
}

// S-03-B — NEGATIVE: a SimDataplane fan-out that DROPS the declared-proto
// key fails the lockstep, naming the missing (ip, port, udp) key.
#[test]
#[should_panic(expected = "RED scaffold")]
fn dropped_sim_fan_out_key_fails_lockstep() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03-B / dropped udp fan-out key \
         fails ReverseNatLockstep, names the missing (ip,port,udp) key)"
    );
}

// S-03-C — NEGATIVE: a SimDataplane fan-out that installs a PHANTOM
// extra-proto key (the pre-US-01 over-broad [Tcp,Udp] behaviour for a
// tcp-only service) fails the lockstep via the orphan-direction check.
#[test]
#[should_panic(expected = "RED scaffold")]
fn phantom_extra_proto_key_fails_lockstep_orphan_check() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03-C / phantom (ip,port,udp) key \
         on a tcp-only service fails the orphan-direction lockstep check)"
    );
}

// S-03-D — the exact #163 shape: a production-mirroring fan-out that
// installs only the tcp key for a udp service. The Tier-1 set-equality
// against the declared udp frontend FAILS, proving #163 cannot recur
// silently at Tier 1.
#[test]
#[should_panic(expected = "RED scaffold")]
fn issue_163_tcp_only_for_udp_service_is_caught() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03-D / #163 regression: tcp-only \
         fan-out for a udp service fails the Tier-1 lockstep set-equality)"
    );
}

// S-02-A — Property: update_service(frontend_P, []) purges ONLY protocol
// P's REVERSE_NAT keys for the VIP; a co-resident other-proto frontend
// on the same VIP (separate update_service call) survives.
//
// GREEN: proptest over backend sets × the two protos. Universe = the
// full BTreeSet(BackendKey) for the VIP; expected = P's keys set_to
// empty, other-proto keys unchanged.
#[test]
#[should_panic(expected = "RED scaffold")]
fn empty_backends_purges_only_this_protos_keys() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02-A / per-proto purge: \
         update_service(frontend_udp, []) removes only udp keys, co-resident tcp survives)"
    );
}

// S-02-B — a REVERSE_NAT key shared with another live service survives a
// per-proto empty-backends purge (the existing live_keys difference
// check, extended to the per-proto frontend shape).
#[test]
#[should_panic(expected = "RED scaffold")]
fn cross_service_shared_key_survives_per_proto_purge() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02-B / cross-service shared \
         (ip,port,udp) key survives when only one service scales to zero)"
    );
}

// S-02-C — Property: update_service(frontend, backends) is idempotent —
// applying it twice with identical args yields the same key set.
#[test]
#[should_panic(expected = "RED scaffold")]
fn idempotent_re_apply_yields_same_key_set() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02-C / idempotent re-apply: \
         second identical update_service yields the same `REVERSE_NAT` key set)"
    );
}

// S-02-D — boundary: a backend with an IPv6 addr contributes no
// REVERSE_NAT key (parity with reverse_nat_keys_for's IPv4-only filter,
// GH #155 deferral).
#[test]
#[should_panic(expected = "RED scaffold")]
fn ipv6_backend_contributes_no_reverse_nat_key() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02-D / IPv6 backend silently \
         skipped from the `REVERSE_NAT` key set, IPv4 backend's key present)"
    );
}

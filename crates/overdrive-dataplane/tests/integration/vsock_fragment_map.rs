//! P5 — fragment-tracking lifecycle of the egress unframe program.
//!
//! DISTILL (netns-density-295 vsock replacement), piece P5. Scenario P5-S4;
//! obligation R5-47; gate G-V6. Decision FRAG-MAP-LIFECYCLE (ADR-0165).
//! Anchored on P-45 (V-18). Spec:
//! `docs/feature/netns-density-295/distill/test-scenarios-p5-egress-unframe-hostinternal.md`.
//!
//! ## GATED — authored RED, NOT YET REGISTERED (see `vsock_egress_unframe.rs`
//! header; same sequencing — shared-infra §0/§6). DELIVER adds
//! `mod vsock_fragment_map;`, the `host-kernel-shared` entry (Tier-3 cells
//! only), and relocates the `// LAYER-1` decision-fn proptest to the crate
//! default unit lane (G-4).
//!
//! ## Design-pinned symbols (DELIVER step ~4 scaffold)
//! `overdrive_core::guest_flow::GUEST_UNFRAME_FRAGMENTS_MAX` (= 4_096);
//! `overdrive_dataplane::guest_flow::{GuestFlowForwarder, GuestFlowCounters}`
//! (`unframe_fragment_shifted`, `unframe_fragment_orphan`).
//! CRAFTER-EXTRACTION (G-1): the fragment-shift decision function modelling the
//! `Fragments` LRU lifecycle (internal structure; the sim forwarder model hosts
//! it). The design pins the invariant, not a public fn signature.

use super::helpers;

/// The IPv4 reassembly key the `Fragments` map is keyed by (D5a / FRAG-MAP).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ReassemblyKey {
    src: std::net::Ipv4Addr,
    dst: std::net::Ipv4Addr,
    protocol: u8,
    id: u16,
}

// ---------------------------------------------------------------------------
// LAYER-1 (relocate to crate default unit lane; G-4) — the decision function.
// Invariant (FRAG-MAP-LIFECYCLE): a later fragment is shifted back by one 8-byte
// unit IFF its own train's first fragment was stripped AND its record was not
// evicted. A stripped first fragment inserts/overwrites; an unstripped first
// fragment removes; the last fragment (MF clear) removes.
// ---------------------------------------------------------------------------
mod decision_fn {
    use super::ReassemblyKey;
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    // CRAFTER-EXTRACTION recommended shape: a pure model of the map lifecycle.
    // DELIVER places the real fn in the sim forwarder model; this proptest pins
    // the PROPERTY. We model the map here as the oracle.
    use overdrive_dataplane::guest_flow::fragment_model::{FragmentEvent, FragmentMap, Shift};

    // Reference oracle: a plain map the proptest compares the production model
    // against (oracle property pattern). Eviction is injected explicitly so the
    // property is deterministic (the kernel LRU is approximate; the only claim
    // is "an eviction loses at most its own record, never corrupts another").
    #[derive(Default)]
    struct OracleMap {
        stripped: BTreeMap<ReassemblyKey, bool>,
    }
    impl OracleMap {
        fn first_stripped(&mut self, k: ReassemblyKey) {
            self.stripped.insert(k, true);
        }
        fn first_unstripped(&mut self, k: ReassemblyKey) {
            self.stripped.remove(&k);
        }
        fn last_fragment(&mut self, k: ReassemblyKey) {
            self.stripped.remove(&k);
        }
        fn evict(&mut self, k: ReassemblyKey) {
            self.stripped.remove(&k);
        }
        fn later_fragment_shifts(&self, k: ReassemblyKey) -> bool {
            self.stripped.get(&k).copied().unwrap_or(false)
        }
    }

    fn key() -> impl Strategy<Value = ReassemblyKey> {
        (0u8..4, 0u16..4).prop_map(|(a, id)| ReassemblyKey {
            src: std::net::Ipv4Addr::new(10, 0, 0, a + 1),
            dst: std::net::Ipv4Addr::new(10, 0, 1, 1),
            protocol: 17, // UDP
            id,
        })
    }

    fn event() -> impl Strategy<Value = FragmentEvent> {
        prop_oneof![
            key().prop_map(FragmentEvent::FirstStripped),
            key().prop_map(FragmentEvent::FirstUnstripped),
            key().prop_map(FragmentEvent::LaterFragment),
            key().prop_map(FragmentEvent::LastFragment),
            key().prop_map(FragmentEvent::Evict),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        #[ignore = "pending DELIVER: fragment_model::FragmentMap (FRAG-MAP-LIFECYCLE, ~step 4)"]
        fn later_fragment_shifts_iff_own_train_stripped_and_not_evicted(
            events in proptest::collection::vec(event(), 0..64)
        ) {
            let mut map = FragmentMap::with_capacity(
                overdrive_core::guest_flow::GUEST_UNFRAME_FRAGMENTS_MAX as usize,
            );
            let mut oracle = OracleMap::default();

            for ev in events {
                match ev {
                    FragmentEvent::FirstStripped(k) => {
                        map.on_event(ev);
                        oracle.first_stripped(k);
                    }
                    FragmentEvent::FirstUnstripped(k) => {
                        map.on_event(ev);
                        oracle.first_unstripped(k);
                    }
                    FragmentEvent::LastFragment(k) => {
                        map.on_event(ev);
                        oracle.last_fragment(k);
                    }
                    FragmentEvent::Evict(k) => {
                        map.on_event(ev);
                        oracle.evict(k);
                    }
                    FragmentEvent::LaterFragment(k) => {
                        // Invariant: the production model shifts this later
                        // fragment IFF the oracle says its own train's first
                        // fragment is stripped and its record present.
                        let shift = map.later_fragment(k);
                        let expect = oracle.later_fragment_shifts(k);
                        prop_assert_eq!(
                            matches!(shift, Shift::BackOneUnit),
                            expect,
                            "shift decision must match oracle for {:?}", k
                        );
                        // A fragment of ANOTHER datagram is never changed by k's
                        // record: nothing cross-key leaks.
                        for other in 0u8..4 {
                            let ok = ReassemblyKey { src: std::net::Ipv4Addr::new(10,0,0,other+1), ..k };
                            if ok != k {
                                prop_assert_eq!(
                                    matches!(map.later_fragment(ok), Shift::BackOneUnit),
                                    oracle.later_fragment_shifts(ok),
                                    "cross-key invariance for {:?}", ok
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tier-3 (host-kernel-shared) — the real egress program with packet injection;
// `GuestFlowCounters` entry counts as the map-state oracle. Example-based
// (Mandate 11). Drives the real forwarder over the veth fixture.
// ---------------------------------------------------------------------------

/// S1 — a stripped first fragment whose train is cut (no later fragments), then
/// an unrelated bare fragmented datagram with the same reassembly key — arrives
/// byte-exact and UNSHIFTED. @contract-shape:unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER: egress fragment lifecycle (FRAG-MAP-LIFECYCLE, ~step 4)"]
async fn cut_train_then_unrelated_bare_fragmented_datagram_arrives_unshifted() {
    let caps = helpers::require_net_admin();
    let fix = helpers::FragmentInjectionFixture::new(&caps).await.expect("fixture");
    let key = fix.reassembly_key();

    // A stripped first fragment whose train is cut: the program strips it and
    // inserts the record; no later fragment of this train ever passes.
    fix.inject_stripped_first_fragment(key).await;

    // Then an UNRELATED bare fragmented datagram reusing (src,dst,proto,id):
    // its own first fragment is unstripped (bare), which REMOVES the stale
    // record, so its later fragments are not shifted — it arrives byte-exact.
    let payload = helpers::fragmented_payload(/* bytes */ 4_096);
    let received = fix.inject_bare_fragmented_datagram(key, &payload).await.expect("send");

    assert_eq!(received, payload, "bare datagram arrives byte-exact, unshifted");
    let counters = fix.forwarder_counters();
    assert_eq!(counters.unframe_fragment_shifted, 1, "only the first (stripped) train shifted");
    assert_eq!(counters.unframe_fragment_orphan, 0, "no orphaned later fragment");
}

/// S2 — the map filled beyond capacity with cut trains from several CPUs: every
/// later datagram arrives byte-exact or (where a live record was evicted) is
/// lost WHOLE, counted, never corrupted; inserts never fail.
#[tokio::test]
#[ignore = "pending DELIVER: egress fragment lifecycle eviction (FRAG-MAP-LIFECYCLE, ~step 4)"]
async fn map_filled_beyond_capacity_loses_whole_datagrams_never_corrupts() {
    let caps = helpers::require_net_admin();
    let fix = helpers::FragmentInjectionFixture::new(&caps).await.expect("fixture");
    let cap = overdrive_core::guest_flow::GUEST_UNFRAME_FRAGMENTS_MAX;

    // Fill well beyond capacity with cut trains from all online CPUs at once.
    let outcomes = fix
        .inject_cut_trains_from_all_cpus(/* count */ (cap as u64) * 4)
        .await
        .expect("inserts never fail");
    assert!(outcomes.insert_failures == 0, "inserts never fail under capacity pressure");

    // Every later frame-shaped / bare datagram either arrives byte-exact or is
    // lost whole (its evicted record counted as orphan); none corrupted.
    let report = fix.replay_later_fragments_and_classify().await.expect("replay");
    assert_eq!(report.corrupted, 0, "no datagram delivered corrupted");
    assert_eq!(
        report.lost_whole, report.orphan_counter_delta,
        "each lost datagram matches one orphan count (lost whole, not partial)"
    );
}

/// S3 — an ordinary congested qdisc (`netem delay 20ms limit 1`) dropping
/// fragments after the program for three runs: afterwards every datagram
/// byte-exact. @contract-shape:unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER: egress fragment lifecycle under qdisc congestion (FRAG-MAP-LIFECYCLE, ~step 4)"]
async fn congested_qdisc_dropping_fragments_leaves_later_datagrams_byte_exact() {
    let caps = helpers::require_net_admin();
    let fix = helpers::FragmentInjectionFixture::new(&caps).await.expect("fixture");
    fix.apply_egress_qdisc("netem delay 20ms limit 1").await.expect("netem");

    for run in 0..3 {
        // The qdisc drops some fragments AFTER the program (so stale records may
        // linger); the production map's lifecycle must still leave the next
        // clean datagram byte-exact.
        let payload = helpers::fragmented_payload(4_097);
        let _ = fix.inject_bare_fragmented_datagram(fix.reassembly_key(), &payload).await; // may drop
        let _ = run;
    }
    fix.clear_egress_qdisc().await.expect("clear netem");

    let payload = helpers::fragmented_payload(4_096);
    let received = fix
        .inject_bare_fragmented_datagram(fix.reassembly_key(), &payload)
        .await
        .expect("send after congestion cleared");
    assert_eq!(received, payload, "after congestion, every datagram byte-exact");
}

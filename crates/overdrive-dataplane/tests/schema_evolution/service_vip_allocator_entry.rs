//! Schema-evolution golden-bytes test — `ServiceVipAllocatorEntryEnvelope`.
//!
//! S-VIP-P02. Pins the V1 archived layout of the
//! `ServiceVipAllocatorEntry` envelope so that any future commit which
//! appends a field to the V1 payload rather than minting a `V2` breaks
//! this test and signals the schema-evolution violation per ADR-0048
//! § 1 and `.claude/rules/testing.md` § "Archive schema-evolution
//! roundtrip".
//!
//! **`FIXTURE_V1` is never touched.** Bumping the envelope to `V2`
//! adds a new `FIXTURE_V2` constant + a new assertion in the same
//! commit; existing constants stay verbatim. See `development.md`
//! § "Version-bump procedure".

use std::net::{IpAddr, Ipv4Addr};

use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::ServiceVip;
use overdrive_dataplane::allocators::entry::{
    ServiceVipAllocatorEntryEnvelope, ServiceVipAllocatorEntryLatest, ServiceVipAllocatorEntryV1,
};

/// Canonical V1 payload pinned by `FIXTURE_V1` below. The expected
/// projection is built from these values verbatim — change any one of
/// them and the test fails until `FIXTURE_V1` is regenerated.
fn canonical_v1_payload() -> ServiceVipAllocatorEntryLatest {
    ServiceVipAllocatorEntryV1 {
        spec_digest: [0xAB; 32],
        vip: ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 96, 0, 1))).expect("valid VIP"),
        counter_idx: 7,
    }
}

/// Hex-encoded rkyv-archived bytes of
/// `ServiceVipAllocatorEntryEnvelope::V1(canonical_v1_payload())`.
/// Generated once at the GREEN landing of step 01-03 by the
/// `print_service_vip_allocator_entry_fixture_v1_bytes` regeneration
/// test below and pinned verbatim from that moment onward. NEVER
/// touched on subsequent commits.
const FIXTURE_V1: &str = include_str!("service_vip_allocator_entry_fixture_v1.hex");

#[test]
fn service_vip_allocator_entry_golden_bytes_roundtrip() {
    let expected = canonical_v1_payload();
    let fixture_hex = FIXTURE_V1.trim();
    assert!(
        !fixture_hex.is_empty(),
        "FIXTURE_V1 must be populated — regenerate via the ignored \
         `print_service_vip_allocator_entry_fixture_v1_bytes` test"
    );
    let bytes = hex::decode(fixture_hex).expect("FIXTURE_V1 must hex-decode cleanly");

    // redb / on-disk reads land at unknown alignment; rkyv requires
    // 8-byte alignment. Copy into AlignedVec before deserialising.
    let mut aligned = rkyv::util::AlignedVec::<8>::new();
    aligned.extend_from_slice(&bytes);

    let envelope: ServiceVipAllocatorEntryEnvelope =
        rkyv::from_bytes::<ServiceVipAllocatorEntryEnvelope, rkyv::rancor::Error>(&aligned)
            .expect("fixture bytes must deserialise as the envelope");

    let latest = envelope.into_latest().expect("envelope must project to Latest");

    assert_eq!(
        latest, expected,
        "FIXTURE_V1 must project to the canonical V1 payload bit-equivalent",
    );
}

// ---------------------------------------------------------------------
// Bootstrap helper — regenerates the canonical V1 bytes on demand for
// the crafter to paste into `service_vip_allocator_entry_fixture_v1.hex`.
// Run via:
//
//   cargo nextest run -p overdrive-dataplane --test schema_evolution \
//       -E 'test(/print_service_vip_allocator_entry_fixture_v1_bytes/)' \
//       --no-capture --run-ignored=ignored-only
//
// Marked `#[ignore]` so it never runs in normal test execution; the
// pinned `FIXTURE_V1` constant is the load-bearing artifact, this is a
// one-shot regeneration aid.
// ---------------------------------------------------------------------

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping a payload variant; the pinned FIXTURE_V<N> constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V1"
)]
fn print_service_vip_allocator_entry_fixture_v1_bytes() {
    let envelope = ServiceVipAllocatorEntryEnvelope::latest(canonical_v1_payload());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_V1 = \"{}\"", hex::encode(bytes.as_ref()));
}

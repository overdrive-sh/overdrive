//! Schema-evolution golden-bytes test — `ServiceVipAllocatorEntryEnvelope`.
//!
//! S-VIP-P02. Pins the V1 + V2 archived layouts of the
//! `ServiceVipAllocatorEntry` envelope so that any future commit which
//! appends a field to an existing payload rather than minting a new
//! variant breaks this test and signals the schema-evolution violation
//! per ADR-0048 § 1 and `.claude/rules/testing.md` § "Archive
//! schema-evolution roundtrip".
//!
//! **`FIXTURE_V1` is never touched.** It remains the load-bearing
//! assertion that V1 bytes persisted by prior binaries still decode
//! through the current envelope (and up-convert via
//! `From<V1> for V2` to drop the obsolete `counter_idx` field).
//! Bumping the envelope to `V3` adds a new `FIXTURE_V3` constant + a
//! new assertion in the same commit; existing constants stay verbatim.
//! See `development.md` § "Version-bump procedure".

use std::net::{IpAddr, Ipv4Addr};

use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::ServiceVip;
use overdrive_dataplane::allocators::entry::{
    ServiceVipAllocatorEntryEnvelope, ServiceVipAllocatorEntryLatest, ServiceVipAllocatorEntryV1,
    ServiceVipAllocatorEntryV2,
};

/// Canonical V1 payload pinned by `FIXTURE_V1` below. The V1 → V2
/// projection drops the `counter_idx` field; the test asserts the
/// up-converted V2 matches the canonical V2 projection (same spec
/// digest + VIP, no counter).
fn canonical_v1_payload() -> ServiceVipAllocatorEntryV1 {
    ServiceVipAllocatorEntryV1 {
        spec_digest: [0xAB; 32],
        vip: ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 96, 0, 1))).expect("valid VIP"),
        counter_idx: 7,
    }
}

/// Canonical V2 payload pinned by `FIXTURE_V2` below.
fn canonical_v2_payload() -> ServiceVipAllocatorEntryLatest {
    ServiceVipAllocatorEntryV2 {
        spec_digest: [0xAB; 32],
        vip: ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 96, 0, 1))).expect("valid VIP"),
    }
}

/// Hex-encoded rkyv-archived bytes of
/// `ServiceVipAllocatorEntryEnvelope::V1(canonical_v1_payload())`.
/// Generated once at the GREEN landing of step 01-03 by the
/// `print_service_vip_allocator_entry_fixture_v1_bytes` regeneration
/// test below and pinned verbatim from that moment onward. NEVER
/// touched on subsequent commits.
const FIXTURE_V1: &str = include_str!("service_vip_allocator_entry_fixture_v1.hex");

/// Hex-encoded rkyv-archived bytes of
/// `ServiceVipAllocatorEntryEnvelope::V2(canonical_v2_payload())`.
/// Generated at the GREEN landing of step 03-03 (ADR-0049 § Amendments
/// → 2026-05-19; V1 → V2 envelope bump). NEVER touched on subsequent
/// commits.
const FIXTURE_V2: &str = include_str!("service_vip_allocator_entry_fixture_v2.hex");

#[test]
fn service_vip_allocator_entry_v1_golden_bytes_up_convert_to_v2() {
    // V1 fixture bytes decode through the current envelope (which now
    // carries both V1 and V2 variants) and up-convert to a V2 payload
    // via `From<V1> for V2`. The `counter_idx` field is discarded by
    // the conversion — it was never consumed by the scan-based V2
    // allocator (ADR-0049 § Amendments → 2026-05-19).
    let v1 = canonical_v1_payload();
    let expected_v2 = ServiceVipAllocatorEntryV2 { spec_digest: v1.spec_digest, vip: v1.vip };

    let fixture_hex = FIXTURE_V1.trim();
    assert!(
        !fixture_hex.is_empty(),
        "FIXTURE_V1 must be populated — regenerate via the ignored \
         `print_service_vip_allocator_entry_fixture_v1_bytes` test"
    );
    let bytes = hex::decode(fixture_hex).expect("FIXTURE_V1 must hex-decode cleanly");

    let mut aligned = rkyv::util::AlignedVec::<8>::new();
    aligned.extend_from_slice(&bytes);

    let envelope: ServiceVipAllocatorEntryEnvelope =
        rkyv::from_bytes::<ServiceVipAllocatorEntryEnvelope, rkyv::rancor::Error>(&aligned)
            .expect("V1 fixture bytes must deserialise as the envelope");

    let latest = envelope.into_latest().expect("envelope must project to Latest");

    assert_eq!(
        latest, expected_v2,
        "FIXTURE_V1 must project to the canonical V2 payload (counter_idx dropped via \
         `From<V1> for V2`)",
    );
}

#[test]
fn service_vip_allocator_entry_v2_golden_bytes_roundtrip() {
    let expected = canonical_v2_payload();
    let fixture_hex = FIXTURE_V2.trim();
    assert!(
        !fixture_hex.is_empty(),
        "FIXTURE_V2 must be populated — regenerate via the ignored \
         `print_service_vip_allocator_entry_fixture_v2_bytes` test"
    );
    let bytes = hex::decode(fixture_hex).expect("FIXTURE_V2 must hex-decode cleanly");

    let mut aligned = rkyv::util::AlignedVec::<8>::new();
    aligned.extend_from_slice(&bytes);

    let envelope: ServiceVipAllocatorEntryEnvelope =
        rkyv::from_bytes::<ServiceVipAllocatorEntryEnvelope, rkyv::rancor::Error>(&aligned)
            .expect("V2 fixture bytes must deserialise as the envelope");

    let latest = envelope.into_latest().expect("envelope must project to Latest");

    assert_eq!(
        latest, expected,
        "FIXTURE_V2 must project to the canonical V2 payload bit-equivalent",
    );
}

// ---------------------------------------------------------------------
// Bootstrap helpers — regenerate the canonical V1 / V2 bytes on demand
// for the crafter to paste into the .hex fixture files. Run via:
//
//   cargo nextest run -p overdrive-dataplane --test schema_evolution \
//       -E 'test(/print_service_vip_allocator_entry_fixture_v._bytes/)' \
//       --no-capture --run-ignored=ignored-only
//
// Marked `#[ignore]` so they never run in normal test execution; the
// pinned `FIXTURE_V<N>` constants are the load-bearing artifacts.
// ---------------------------------------------------------------------

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping a payload variant; the pinned FIXTURE_V<N> constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V1"
)]
fn print_service_vip_allocator_entry_fixture_v1_bytes() {
    // V1 envelope variant constructed directly via the `From<V1> for V2`
    // path is impossible (V1 → V2 collapses); to regenerate V1 bytes we
    // need to construct `Envelope::V1(...)` explicitly. This is a
    // regeneration-only path — production writes never produce V1.
    #[allow(
        clippy::allow_attributes,
        reason = "regeneration tool needs to bypass the alias-to-payload writer discipline"
    )]
    let envelope = ServiceVipAllocatorEntryEnvelope::V1(canonical_v1_payload());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_V1 = \"{}\"", hex::encode(bytes.as_ref()));
}

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping a payload variant; the pinned FIXTURE_V<N> constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V2"
)]
fn print_service_vip_allocator_entry_fixture_v2_bytes() {
    let envelope = ServiceVipAllocatorEntryEnvelope::latest(canonical_v2_payload());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_V2 = \"{}\"", hex::encode(bytes.as_ref()));
}

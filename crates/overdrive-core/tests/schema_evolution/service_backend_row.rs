//! Schema-evolution golden-bytes test — `ServiceBackendRowEnvelope`.
//!
//! S-EV-01.4. Pins the V1 archived layout of the `ServiceBackendRow`
//! envelope so that any future commit which appends a field to the V1
//! payload rather than minting a `V2` breaks this test and signals the
//! schema-evolution violation per ADR-0048 § 1 and
//! `.claude/rules/testing.md` § "Archive schema-evolution roundtrip".
//!
//! **`FIXTURE_V1` is never touched.** Bumping the envelope to `V2` adds
//! a new `FIXTURE_V2` constant + a new assertion in the same commit;
//! existing constants stay verbatim. See `development.md`
//! § "Version-bump procedure".

use std::net::Ipv4Addr;

use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::{NodeId, ServiceId, SpiffeId};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ServiceBackendRowEnvelope, ServiceBackendRowLatest, ServiceBackendRowV1,
};

use super::harness::assert_envelope_v_roundtrip;

/// Canonical V1 payload pinned by `FIXTURE_V1` below. The expected
/// projection is built from these values verbatim — change any one of
/// them and the test fails until `FIXTURE_V1` is regenerated.
fn canonical_v1_payload() -> ServiceBackendRowLatest {
    let writer = NodeId::new("node-001").expect("valid writer node id");
    let alloc =
        SpiffeId::new("spiffe://overdrive.sh/svc/payments/alloc-1").expect("valid spiffe id");
    ServiceBackendRowV1 {
        service_id: ServiceId::new(42).expect("valid service id"),
        vip: Ipv4Addr::new(10, 0, 0, 1),
        backends: vec![Backend {
            alloc,
            addr: "10.0.1.1:8080".parse().expect("valid socket addr"),
            weight: 1,
            healthy: true,
        }],
        updated_at: LogicalTimestamp { counter: 1, writer },
    }
}

/// Hex-encoded rkyv-archived bytes of
/// `ServiceBackendRowEnvelope::V1(canonical_v1_payload())`. Generated
/// once at the GREEN landing of step 02-03 and pinned verbatim from
/// that moment onward. NEVER touched on subsequent commits.
const FIXTURE_V1: &str = "7370696666653a2f2f6f76657264726976652e73682f7376632f7061796d656e74732f616c6c6f632d310000aa000000d4ffffff1500000000000a000101901f000000000000000000000000000000000000000000000000010001000000000000000000000000002a000000000000000a000001b8ffffff010000000000000001000000000000006e6f64652d303031";

#[test]
fn service_backend_row_v1_decodes_through_current_envelope() {
    let expected = canonical_v1_payload();
    assert_envelope_v_roundtrip::<ServiceBackendRowEnvelope>(FIXTURE_V1, &expected);
}

// ---------------------------------------------------------------------
// Bootstrap helper — generates the canonical V1 bytes on demand for the
// crafter to paste into `FIXTURE_V1` above. Run via:
//
//   cargo nextest run -p overdrive-core --test schema_evolution \
//       -E 'test(/print_service_backend_fixture_v1_bytes/)' --no-capture
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
fn print_service_backend_fixture_v1_bytes() {
    let envelope = ServiceBackendRowEnvelope::latest(canonical_v1_payload());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_V1 = \"{}\"", hex::encode(bytes.as_ref()));
}

#[test]
#[ignore = "discriminant-offset stability check — flips byte at offset 48 from end across payload sizes; confirms it always fires as discriminant"]
#[allow(
    clippy::print_stdout,
    reason = "probe tool emits offsets to stdout for the human to inspect"
)]
fn discriminant_offset_48_is_stable_across_payload_sizes() {
    let mk = |writer_name: &str, backends: Vec<Backend>, counter: u64| ServiceBackendRowV1 {
        service_id: ServiceId::new(42).expect("valid"),
        vip: Ipv4Addr::new(10, 0, 0, 1),
        backends,
        updated_at: LogicalTimestamp { counter, writer: NodeId::new(writer_name).expect("valid") },
    };
    let alloc_s = SpiffeId::new("spiffe://overdrive.sh/a").expect("valid");
    let alloc_m = SpiffeId::new("spiffe://overdrive.sh/svc/payments/alloc-1").expect("valid");
    let b1 = vec![Backend {
        alloc: alloc_m.clone(),
        addr: "10.0.1.1:8080".parse().expect("addr"),
        weight: 1,
        healthy: true,
    }];
    let b2 = vec![
        Backend {
            alloc: alloc_s,
            addr: "10.0.1.1:8080".parse().expect("addr"),
            weight: 1,
            healthy: true,
        },
        Backend {
            alloc: alloc_m,
            addr: "10.0.1.2:9090".parse().expect("addr"),
            weight: 2,
            healthy: false,
        },
    ];

    let cases = vec![
        ("one_backend/short_writer", mk("a", b1.clone(), 1)),
        ("one_backend/med_writer", mk("node-001", b1.clone(), 999_999)),
        ("one_backend/long_writer", mk("node-with-much-longer-identifier-string", b1, u64::MAX)),
        ("two_backends/med_writer", mk("node-001", b2.clone(), 1)),
        ("two_backends/long_writer", mk("node-with-much-longer-identifier-string", b2, 1)),
    ];

    for (label, payload) in cases {
        let envelope = ServiceBackendRowEnvelope::latest(payload);
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
        let n = bytes.len();
        // Flip byte at offset 48 from end
        let mut modified = bytes.to_vec();
        let idx = n - 48;
        modified[idx] = 0x99;
        let result = rkyv::from_bytes::<ServiceBackendRowEnvelope, rkyv::rancor::Error>(&modified);
        match result {
            Ok(_) => panic!("{label}: flipping offset 48 must error, but decoded OK"),
            Err(e) => {
                let s = format!("{e}");
                assert!(
                    s.contains("invalid discriminant"),
                    "{label}: offset 48 must be the discriminant byte; got: {s}"
                );
                println!("{label}: len={n}, offset 48 confirmed as discriminant");
            }
        }
    }
}

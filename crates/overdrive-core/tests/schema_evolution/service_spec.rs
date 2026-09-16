//! Current VM-only archive fixture for `ServiceSpecEnvelope`.

#![allow(
    clippy::doc_markdown,
    reason = "the required per-test CONTRACT_SHAPE declaration is a literal protocol marker"
)]

use std::num::NonZeroU16;

use overdrive_core::aggregate::{
    Listener, ParserDriverInput, ParserResourcesInput as ResourcesInput, ParserVmInput,
    ServiceSpecEnvelope, ServiceSpecLatest, ServiceSpecV1,
};
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::dataplane::backend_key::Proto;

use super::harness::assert_envelope_v_roundtrip;

fn canonical_v1_payload() -> ServiceSpecLatest {
    ServiceSpecV1 {
        id: "svc-v1-vm".to_string(),
        replicas: 1,
        driver: ParserDriverInput::Vm(ParserVmInput {
            command: "/usr/bin/server".to_string(),
            args: vec![],
            kernel: "/kernel".to_string(),
            rootfs: "/rootfs".to_string(),
        }),
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 134_217_728 },
        listeners: vec![Listener {
            port: NonZeroU16::new(9090).expect("non-zero port"),
            protocol: Proto::Tcp,
        }],
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

const FIXTURE_V1: &str = "7376632d76312d766d2f7573722f62696e2f7365727665728223000000000000000000000000000089000000d8ffffff01000000000000008f000000d1ffffffd8ffffff000000002f6b65726e656cff2f726f6f746673ff64000000000000000000000800000000b0ffffff01000000acffffff00000000a4ffffff000000009cffffff00000000";

fn archive_hex(envelope: &ServiceSpecEnvelope) -> String {
    let bytes =
        rkyv::to_bytes::<rkyv::rancor::Error>(envelope).expect("ServiceSpec envelope must archive");
    hex::encode(bytes.as_ref())
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn service_spec_v1_direct_fixture_round_trips_with_sole_tag_zero() {
    let expected = canonical_v1_payload();
    assert_eq!(archive_hex(&ServiceSpecEnvelope::latest(expected.clone())), FIXTURE_V1);
    assert_envelope_v_roundtrip::<ServiceSpecEnvelope>(FIXTURE_V1, &expected);
    assert_eq!(ServiceSpecEnvelope::known_discriminants(), &[0]);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn service_spec_envelope_type_name_is_exact_string() {
    assert_eq!(ServiceSpecEnvelope::type_name(), "ServiceSpecEnvelope");
}

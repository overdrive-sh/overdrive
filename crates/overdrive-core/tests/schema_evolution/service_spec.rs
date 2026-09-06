//! Frozen archive fixtures for `ServiceSpecEnvelope`.

use std::num::NonZeroU16;

use overdrive_core::aggregate::{
    Listener, ParserDriverInput, ParserExecInput as ExecInput,
    ParserResourcesInput as ResourcesInput, ParserVmInput, ProbeDescriptor, ProbeMechanic,
    ServiceSpecEnvelope, ServiceSpecLatest, ServiceSpecV1, ServiceSpecV2, ServiceSpecV3,
};
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::observation::{ProbeIdx, ProbeRole};

use super::harness::assert_envelope_v_roundtrip;

fn canonical_v1_payload() -> ServiceSpecV1 {
    ServiceSpecV1 {
        id: "svc-pre-probes".to_string(),
        replicas: 1,
        exec: ExecInput { command: "/usr/bin/server".to_string(), args: vec![] },
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 134_217_728 },
        listeners: vec![Listener {
            port: NonZeroU16::new(8080).expect("non-zero port"),
            protocol: Proto::Tcp,
        }],
    }
}

fn canonical_v2_payload() -> ServiceSpecV2 {
    ServiceSpecV2 {
        id: "svc-with-probe".to_string(),
        replicas: 1,
        exec: ExecInput { command: "/usr/bin/server".to_string(), args: vec![] },
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 134_217_728 },
        listeners: vec![Listener {
            port: NonZeroU16::new(9090).expect("non-zero port"),
            protocol: Proto::Tcp,
        }],
        startup_probes: vec![ProbeDescriptor {
            idx: ProbeIdx::new(0),
            role: ProbeRole::Startup,
            mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_string(), port: 9090 },
            timeout_seconds: 5,
            interval_seconds: 2,
            max_attempts: 30,
            failure_threshold: None,
            success_threshold: None,
            inferred: true,
        }],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

fn canonical_v3_payload() -> ServiceSpecLatest {
    ServiceSpecV3 {
        id: "svc-v3-vm".to_string(),
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

const FIXTURE_V1: &str = "7376632d7072652d70726f6265732f7573722f62696e2f736572766572000000901f00000000000000000000000000008e000000d0ffffff010000008f000000d2ffffffdcffffff000000000000000064000000000000000000000800000000c0ffffff01000000000000000000000000000000000000000000000000000000";
const FIXTURE_V2: &str = "7376632d776974682d70726f62652f7573722f62696e2f73657276657200000082230000000000000000000000000000302e302e302e30ff8223000000000000000000000000000005000000020000001e000000000000000000000000000000000000000100000001000000000000008e00000090ffffff010000008f00000092ffffff9cffffff00000000000000006400000000000000000000080000000080ffffff010000007cffffff01000000b8ffffff00000000b0ffffff00000000";
const FIXTURE_V3: &str = "7376632d76332d766d2f7573722f62696e2f736572766572822300000000000089000000e0ffffff01000000010000008f000000d9ffffffe0ffffff000000002f6b65726e656cff2f726f6f746673ff64000000000000000000000800000000b8ffffff01000000b4ffffff00000000acffffff00000000a4ffffff00000000020000009cffffff0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

fn archive_hex(envelope: &ServiceSpecEnvelope) -> String {
    let bytes =
        rkyv::to_bytes::<rkyv::rancor::Error>(envelope).expect("ServiceSpec envelope must archive");
    hex::encode(bytes.as_ref())
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn service_spec_v1_decodes_through_current_envelope() {
    let expected: ServiceSpecLatest = ServiceSpecV2::from(canonical_v1_payload()).into();
    assert_envelope_v_roundtrip::<ServiceSpecEnvelope>(FIXTURE_V1, &expected);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn service_spec_v2_decodes_through_current_envelope() {
    let expected: ServiceSpecLatest = canonical_v2_payload().into();
    assert_envelope_v_roundtrip::<ServiceSpecEnvelope>(FIXTURE_V2, &expected);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn service_spec_v1_and_v2_rearchive_to_their_frozen_bytes() {
    assert_eq!(archive_hex(&ServiceSpecEnvelope::V1(canonical_v1_payload())), FIXTURE_V1);
    assert_eq!(archive_hex(&ServiceSpecEnvelope::V2(canonical_v2_payload())), FIXTURE_V2);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn service_spec_v3_boxed_fixture_decodes_and_rearchives_exactly() {
    let expected = canonical_v3_payload();
    assert_envelope_v_roundtrip::<ServiceSpecEnvelope>(FIXTURE_V3, &expected);
    assert_eq!(archive_hex(&ServiceSpecEnvelope::latest(expected)), FIXTURE_V3);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn service_spec_envelope_known_discriminants_are_exactly_v1_v2_v3() {
    assert_eq!(ServiceSpecEnvelope::known_discriminants(), &[0, 1, 2]);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn service_spec_envelope_type_name_is_exact_string() {
    assert_eq!(ServiceSpecEnvelope::type_name(), "ServiceSpecEnvelope");
}

#[test]
#[ignore = "fixture regeneration tool — run only while bumping the envelope"]
#[allow(clippy::print_stdout, reason = "prints candidate frozen fixture bytes for manual review")]
fn print_service_spec_fixture_bytes() {
    for envelope in [
        ServiceSpecEnvelope::V1(canonical_v1_payload()),
        ServiceSpecEnvelope::V2(canonical_v2_payload()),
        ServiceSpecEnvelope::latest(canonical_v3_payload()),
    ] {
        println!("FIXTURE = \"{}\"", archive_hex(&envelope));
    }
}

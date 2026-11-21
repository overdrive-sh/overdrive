//! Archive fixtures for `ServiceSpecEnvelope`: the frozen V1 layout and the V2
//! layout whose driver table carries `[vm.volume]` (ADR-0048, GH #348).
//!
//! V2 is a larger variant than V1, so the V1 bytes are the explicit
//! `Envelope::V1(<frozen V1 payload>)` archive: built directly, never through
//! `latest()` or the aliases, which now produce V2.

#![allow(
    clippy::doc_markdown,
    reason = "the required per-test CONTRACT_SHAPE declaration is a literal protocol marker"
)]

use overdrive_core::byte_size::ByteSize;
use overdrive_core::cpu_quantity::CpuQuantity;
use std::num::NonZeroU16;

use overdrive_core::aggregate::{
    Listener, ParserDriverInput, ParserDriverInputV1, ParserResourcesInput as ResourcesInput,
    ParserVmInput, ParserVmInputV1, ProbeDescriptor, ProbeMechanic, ServiceSpecEnvelope,
    ServiceSpecLatest, ServiceSpecV1, ServiceSpecV2, VmVolumeInput,
};
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::observation::{ProbeIdx, ProbeRole};

use super::harness::{assert_envelope_v_roundtrip, decode_envelope_v_latest};

fn listeners() -> Vec<Listener> {
    vec![Listener { port: NonZeroU16::new(9090).expect("non-zero port"), protocol: Proto::Tcp }]
}

const fn guest_listeners_descriptor() -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::GuestListeners,
        timeout_seconds: 5,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: None,
        success_threshold: None,
    }
}

/// The frozen V1 payload, hand-built from the V1 types.
fn payload_v1(startup_probes: Vec<ProbeDescriptor>) -> ServiceSpecV1 {
    ServiceSpecV1 {
        id: "svc-v1-vm".to_string(),
        replicas: 1,
        driver: ParserDriverInputV1::Vm(ParserVmInputV1 {
            command: "/usr/bin/server".to_string(),
            args: vec![],
            kernel: "/kernel".to_string(),
            rootfs: "/rootfs".to_string(),
        }),
        resources: ResourcesInput {
            cpu: CpuQuantity::from_millis(100),
            memory: ByteSize::new(134_217_728),
        },
        listeners: listeners(),
        startup_probes,
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

/// The same payload as the V2 value it must up-convert to, written out by hand.
fn payload_v2(
    startup_probes: Vec<ProbeDescriptor>,
    volume: Option<VmVolumeInput>,
) -> ServiceSpecLatest {
    ServiceSpecV2 {
        id: "svc-v1-vm".to_string(),
        replicas: 1,
        driver: ParserDriverInput::Vm(ParserVmInput {
            command: "/usr/bin/server".to_string(),
            args: vec![],
            kernel: "/kernel".to_string(),
            rootfs: "/rootfs".to_string(),
            volume,
        }),
        resources: ResourcesInput {
            cpu: CpuQuantity::from_millis(100),
            memory: ByteSize::new(134_217_728),
        },
        listeners: listeners(),
        startup_probes,
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

const FIXTURE_V1: &str = "7376632d76312d766d2f7573722f62696e2f7365727665728223000000000000000000000000000089000000d8ffffff01000000000000008f000000d1ffffffd8ffffff000000002f6b65726e656cff2f726f6f746673ff64000000000000000000000800000000b0ffffff01000000acffffff00000000a4ffffff000000009cffffff0000000000000000000000000000000000000000";
const FIXTURE_V1_GUEST_LISTENERS: &str = "7376632d76312d766d2f7573722f62696e2f7365727665728223000000000000000000000200000000000000000000000000000000000000000000000000000005000000020000001e000000000000000000000000000000000000000000000000000000000000008900000098ffffff01000000000000008f00000091ffffff98ffffff000000002f6b65726e656cff2f726f6f746673ff6400000000000000000000080000000070ffffff010000006cffffff01000000a4ffffff000000009cffffff0000000000000000000000000000000000000000";
const FIXTURE_V2: &str = "7376632d76312d766d2f7573722f62696e2f7365727665728223000000000000010000000000000089000000d8ffffff01000000000000008f000000d1ffffffd8ffffff000000002f6b65726e656cff2f726f6f746673ff010000003430476942ffffff0000000064000000000000000000000800000000a0ffffff010000009cffffff0000000094ffffff000000008cffffff00000000";

fn archive_hex(envelope: &ServiceSpecEnvelope) -> String {
    let bytes =
        rkyv::to_bytes::<rkyv::rancor::Error>(envelope).expect("ServiceSpec envelope must archive");
    hex::encode(bytes.as_ref())
}

/// The explicit frozen V1 archives are pinned byte for byte.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn frozen_v1_archives_are_pinned() {
    assert_eq!(archive_hex(&ServiceSpecEnvelope::V1(payload_v1(vec![]))), FIXTURE_V1);
    assert_eq!(
        archive_hex(&ServiceSpecEnvelope::V1(payload_v1(vec![guest_listeners_descriptor()]))),
        FIXTURE_V1_GUEST_LISTENERS,
    );
}

/// Each V1 archive decodes through the current envelope and up-converts to the
/// hand-written V2 payload with no volume.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn v1_archives_decode_and_up_convert_with_no_volume() {
    assert_eq!(
        decode_envelope_v_latest::<ServiceSpecEnvelope>(FIXTURE_V1),
        payload_v2(vec![], None)
    );
    assert_eq!(
        decode_envelope_v_latest::<ServiceSpecEnvelope>(FIXTURE_V1_GUEST_LISTENERS),
        payload_v2(vec![guest_listeners_descriptor()], None),
    );
}

/// The V2 layout carrying a volume archives to the pinned bytes and round trips.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn v2_service_spec_with_a_volume_archives_and_roundtrips() {
    let expected = payload_v2(vec![], Some(VmVolumeInput { size: "40GiB".to_string() }));
    let envelope = ServiceSpecEnvelope::latest(expected.clone());
    assert!(matches!(envelope, ServiceSpecEnvelope::V2(_)), "latest() produces V2");
    assert_eq!(archive_hex(&envelope), FIXTURE_V2);
    assert_envelope_v_roundtrip::<ServiceSpecEnvelope>(FIXTURE_V2, &expected);
    assert_eq!(ServiceSpecEnvelope::known_discriminants(), &[0, 1]);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn service_spec_envelope_type_name_is_exact_string() {
    assert_eq!(ServiceSpecEnvelope::type_name(), "ServiceSpecEnvelope");
}

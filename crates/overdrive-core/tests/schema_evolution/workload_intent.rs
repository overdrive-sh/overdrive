//! Archive fixtures for `WorkloadIntentEnvelope`: the frozen V1 layout and the
//! V2 layout that carries `[vm.volume]` (ADR-0048, GH #348).
//!
//! V2 is a larger variant than V1, so the V1 bytes are the explicit
//! `Envelope::V1(<frozen V1 payload>)` archive: built directly, never through
//! `latest()` or the aliases, which now produce V2.

#![allow(
    clippy::doc_markdown,
    reason = "the required per-test CONTRACT_SHAPE declaration is a literal protocol marker"
)]

use std::num::{NonZeroU16, NonZeroU32};

use overdrive_core::aggregate::{
    CronExpr, Job, JobV1, Listener, ProbeDescriptor, ProbeMechanic, Schedule, ScheduleV1, Service,
    ServiceV1, Vm, VmV1, VmVolume, WorkloadDriver, WorkloadDriverV1, WorkloadIntent,
    WorkloadIntentEnvelope, WorkloadIntentLatest, WorkloadIntentV1,
};
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::WorkloadId;
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_core::traits::driver::Resources;
use overdrive_core::volume::VolumeSize;

use super::harness::{assert_envelope_v_roundtrip, decode_envelope_v_latest};

const COMMAND: &str = "/sbin/init";
const ARGS: &str = "--quiet";
const KERNEL: &str = "/var/lib/overdrive/vmlinux";
const ROOTFS: &str = "/var/lib/overdrive/rootfs.img";

// ---------------------------------------------------------------------------
// Frozen V1 payloads, hand-built from the V1 types.
// ---------------------------------------------------------------------------

fn vm_v1(command: &str) -> WorkloadDriverV1 {
    WorkloadDriverV1::Vm(VmV1 {
        command: command.to_string(),
        args: vec![ARGS.to_string()],
        kernel: KERNEL.to_string(),
        rootfs: ROOTFS.to_string(),
    })
}

fn job_v1() -> JobV1 {
    JobV1 {
        id: WorkloadId::new("svc-v1-job").expect("valid workload id"),
        replicas: NonZeroU32::new(1).expect("non-zero replicas"),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: vm_v1(COMMAND),
    }
}

fn service_v1(startup_probes: Vec<ProbeDescriptor>) -> ServiceV1 {
    ServiceV1 {
        id: WorkloadId::new("svc-v1-service").expect("valid workload id"),
        replicas: NonZeroU32::new(2).expect("non-zero replicas"),
        resources: Resources { cpu_milli: 500, memory_bytes: 128 * 1024 * 1024 },
        driver: vm_v1("/usr/bin/frontend"),
        listeners: vec![Listener {
            port: NonZeroU16::new(8080).expect("non-zero port"),
            protocol: Proto::Tcp,
        }],
        startup_probes,
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

fn schedule_v1() -> ScheduleV1 {
    ScheduleV1 {
        id: WorkloadId::new("svc-v1-schedule").expect("valid workload id"),
        job: JobV1 {
            id: WorkloadId::new("svc-v1-schedule").expect("valid workload id"),
            replicas: NonZeroU32::new(1).expect("non-zero replicas"),
            resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: vm_v1("/usr/local/bin/cleanup"),
        },
        cron_expr: CronExpr::new("0 2 * * *").expect("valid cron expr"),
    }
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

// ---------------------------------------------------------------------------
// The same intents as the V2 payloads they must up-convert to, written out by
// hand (never derived from the V1 value with `.into()`).
// ---------------------------------------------------------------------------

fn vm_v2(command: &str, volume: Option<VmVolume>) -> WorkloadDriver {
    WorkloadDriver::Vm(Vm {
        command: command.to_string(),
        args: vec![ARGS.to_string()],
        kernel: KERNEL.to_string(),
        rootfs: ROOTFS.to_string(),
        volume,
    })
}

fn forty_gib() -> VmVolume {
    VmVolume { size: "40GiB".parse::<VolumeSize>().expect("valid size") }
}

fn job_v2(volume: Option<VmVolume>) -> Job {
    Job {
        id: WorkloadId::new("svc-v1-job").expect("valid workload id"),
        replicas: NonZeroU32::new(1).expect("non-zero replicas"),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: vm_v2(COMMAND, volume),
    }
}

fn service_v2(startup_probes: Vec<ProbeDescriptor>) -> Service {
    Service {
        id: WorkloadId::new("svc-v1-service").expect("valid workload id"),
        replicas: NonZeroU32::new(2).expect("non-zero replicas"),
        resources: Resources { cpu_milli: 500, memory_bytes: 128 * 1024 * 1024 },
        driver: vm_v2("/usr/bin/frontend", None),
        listeners: vec![Listener {
            port: NonZeroU16::new(8080).expect("non-zero port"),
            protocol: Proto::Tcp,
        }],
        startup_probes,
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

fn schedule_v2() -> Schedule {
    Schedule {
        id: WorkloadId::new("svc-v1-schedule").expect("valid workload id"),
        job: Job {
            id: WorkloadId::new("svc-v1-schedule").expect("valid workload id"),
            replicas: NonZeroU32::new(1).expect("non-zero replicas"),
            resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: vm_v2("/usr/local/bin/cleanup", None),
        },
        cron_expr: CronExpr::new("0 2 * * *").expect("valid cron expr"),
    }
}

// ---------------------------------------------------------------------------
// Pinned archives.
// ---------------------------------------------------------------------------

const FIXTURE_V1: &str = "7376632d76312d6a6f622f7362696e2f696e69742d2d7175696574ff2f7661722f6c69622f6f76657264726976652f766d6c696e75782f7661722f6c69622f6f76657264726976652f726f6f7466732e696d670000000000000000000000000000000000000000008a00000098ffffff010000000000000064000000000000000000000400000000000000008a0000007effffff80ffffff010000009a00000080ffffff9d00000092ffffff00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
const FIXTURE_V1_SERVICE: &str = "7376632d76312d736572766963652f7573722f62696e2f66726f6e74656e64002d2d7175696574ff2f7661722f6c69622f6f76657264726976652f766d6c696e75782f7661722f6c69622f6f76657264726976652f726f6f7466732e696d6700901f000000000000000000000000000001000000000000008e00000088ffffff0200000000000000f4010000000000000000000800000000000000009100000072ffffff7cffffff010000009a0000007cffffff9d0000008effffffa4ffffff01000000a0ffffff0000000098ffffff0000000090ffffff000000000000000000000000000000000000000000000000";
const FIXTURE_V1_SCHEDULE: &str = "7376632d76312d7363686564756c657376632d76312d7363686564756c652f7573722f6c6f63616c2f62696e2f636c65616e75702d2d7175696574ff2f7661722f6c69622f6f76657264726976652f766d6c696e75782f7661722f6c69622f6f76657264726976652f726f6f7466732e696d67302032202a202a202a00000000000000000000000002000000000000008f00000070ffffff8f00000077ffffff010000000000000064000000000000000000000400000000000000009600000062ffffff70ffffff010000009a00000070ffffff9d00000082ffffff000000008900000093ffffff0000000000000000000000000000000000000000000000000000000000000000";
const FIXTURE_V1_SERVICE_GUEST_LISTENERS: &str = "7376632d76312d736572766963652f7573722f62696e2f66726f6e74656e64002d2d7175696574ff2f7661722f6c69622f6f76657264726976652f766d6c696e75782f7661722f6c69622f6f76657264726976652f726f6f7466732e696d6700901f000000000000000000000200000000000000000000000000000000000000000000000000000005000000020000001e0000000000000000000000000000000000000000000000000000000000000001000000000000008e00000048ffffff0200000000000000f4010000000000000000000800000000000000009100000032ffffff3cffffff010000009a0000003cffffff9d0000004effffff64ffffff0100000060ffffff0100000098ffffff0000000090ffffff000000000000000000000000000000000000000000000000";
const FIXTURE_V2: &str = "7376632d76312d6a6f622f7362696e2f696e69742d2d7175696574ff2f7661722f6c69622f6f76657264726976652f766d6c696e75782f7661722f6c69622f6f76657264726976652f726f6f7466732e696d670000000000010000000000000000000000000000008a00000098ffffff01000000000000006400000000000000000000040000000000000000000000008a0000007affffff7cffffff010000009a0000007cffffff9d0000008effffff0100000000000000000000000a0000000000000000000000000000000000000000000000000000000000000000000000";
const FIXTURE_V2_SERVICE_GUEST_LISTENERS: &str = "7376632d76312d736572766963652f7573722f62696e2f66726f6e74656e64002d2d7175696574ff2f7661722f6c69622f6f76657264726976652f766d6c696e75782f7661722f6c69622f6f76657264726976652f726f6f7466732e696d6700901f000000000000000000000200000000000000000000000000000000000000000000000000000005000000020000001e0000000000000000000000000000000000000000000000010000000000000001000000000000008e00000048ffffff0200000000000000f40100000000000000000008000000000000000000000000910000002effffff38ffffff010000009a00000038ffffff9d0000004affffff0000000000000000000000000000000050ffffff010000004cffffff0100000084ffffff000000007cffffff00000000";

fn archive_hex(envelope: &WorkloadIntentEnvelope) -> String {
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(envelope)
        .expect("WorkloadIntent envelope must archive");
    hex::encode(bytes.as_ref())
}

fn v1_hex(intent: WorkloadIntentV1) -> String {
    archive_hex(&WorkloadIntentEnvelope::V1(intent))
}

/// The explicit frozen V1 archives are pinned byte for byte, and each is the
/// `V1` variant (discriminant 0).
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn frozen_v1_archives_are_pinned() {
    assert_eq!(v1_hex(WorkloadIntentV1::Job(job_v1())), FIXTURE_V1);
    assert_eq!(v1_hex(WorkloadIntentV1::Service(service_v1(vec![]))), FIXTURE_V1_SERVICE);
    assert_eq!(v1_hex(WorkloadIntentV1::Schedule(schedule_v1())), FIXTURE_V1_SCHEDULE);
    assert_eq!(
        v1_hex(WorkloadIntentV1::Service(service_v1(vec![guest_listeners_descriptor()]))),
        FIXTURE_V1_SERVICE_GUEST_LISTENERS,
    );
}

/// Every V1 archive decodes through the current envelope and up-converts to
/// the hand-written V2 intent with no volume.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn v1_archives_decode_and_up_convert_with_no_volume() {
    assert_eq!(
        decode_envelope_v_latest::<WorkloadIntentEnvelope>(FIXTURE_V1),
        WorkloadIntent::Job(job_v2(None)),
    );
    assert_eq!(
        decode_envelope_v_latest::<WorkloadIntentEnvelope>(FIXTURE_V1_SERVICE),
        WorkloadIntent::Service(service_v2(vec![])),
    );
    assert_eq!(
        decode_envelope_v_latest::<WorkloadIntentEnvelope>(FIXTURE_V1_SCHEDULE),
        WorkloadIntent::Schedule(schedule_v2()),
    );
    assert_eq!(
        decode_envelope_v_latest::<WorkloadIntentEnvelope>(FIXTURE_V1_SERVICE_GUEST_LISTENERS),
        WorkloadIntent::Service(service_v2(vec![guest_listeners_descriptor()])),
    );
}

/// The V2 layout carrying a volume archives to the pinned bytes and round
/// trips through the current envelope.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn v2_job_with_a_volume_archives_and_roundtrips() {
    let expected = WorkloadIntent::Job(job_v2(Some(forty_gib())));
    let envelope = WorkloadIntentEnvelope::latest(expected.clone());
    assert!(matches!(envelope, WorkloadIntentEnvelope::V2(_)), "latest() produces V2");
    assert_eq!(archive_hex(&envelope), FIXTURE_V2);
    assert_envelope_v_roundtrip::<WorkloadIntentEnvelope>(FIXTURE_V2, &expected);
}

/// The V2 layout of a service with guest listeners archives to the pinned
/// bytes and round trips.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn v2_service_with_guest_listeners_archives_and_roundtrips() {
    let expected = WorkloadIntent::Service(service_v2(vec![guest_listeners_descriptor()]));
    assert_eq!(
        archive_hex(&WorkloadIntentEnvelope::latest(expected.clone())),
        FIXTURE_V2_SERVICE_GUEST_LISTENERS,
    );
    assert_envelope_v_roundtrip::<WorkloadIntentEnvelope>(
        FIXTURE_V2_SERVICE_GUEST_LISTENERS,
        &expected,
    );
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn workload_intent_codec_roundtrips_each_kind() {
    for expected in [
        WorkloadIntentLatest::Job(job_v2(Some(forty_gib()))),
        WorkloadIntentLatest::Service(service_v2(vec![])),
        WorkloadIntentLatest::Schedule(schedule_v2()),
    ] {
        let bytes = expected.archive_for_store().expect("archive_for_store must succeed");
        let decoded = WorkloadIntent::from_store_bytes(
            bytes.as_ref(),
            std::path::Path::new("schema_evolution.redb"),
            None,
        )
        .expect("current bytes must decode");
        assert_eq!(decoded, expected);
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn workload_intent_envelope_reports_both_discriminants() {
    assert_eq!(WorkloadIntentEnvelope::known_discriminants(), &[0, 1]);
    assert_eq!(WorkloadIntentEnvelope::type_name(), "WorkloadIntentEnvelope");
}

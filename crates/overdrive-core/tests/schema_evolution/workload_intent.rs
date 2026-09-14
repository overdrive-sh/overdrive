//! Current VM-only archive fixtures for `WorkloadIntentEnvelope`.

#![allow(
    clippy::doc_markdown,
    reason = "the required per-test CONTRACT_SHAPE declaration is a literal protocol marker"
)]

use std::num::{NonZeroU16, NonZeroU32};

use overdrive_core::aggregate::{
    CronExpr, Job, Listener, Schedule, Service, Vm, WorkloadDriver, WorkloadIntent,
    WorkloadIntentEnvelope, WorkloadIntentLatest,
};
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::WorkloadId;
use overdrive_core::traits::driver::Resources;

use super::harness::assert_envelope_v_roundtrip;

fn vm(command: &str) -> WorkloadDriver {
    WorkloadDriver::Vm(Vm {
        command: command.to_string(),
        args: vec!["--quiet".to_string()],
        kernel: "/var/lib/overdrive/vmlinux".to_string(),
        rootfs: "/var/lib/overdrive/rootfs.img".to_string(),
    })
}

fn canonical_job() -> Job {
    Job {
        id: WorkloadId::new("svc-v1-job").expect("valid workload id"),
        replicas: NonZeroU32::new(1).expect("non-zero replicas"),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: vm("/sbin/init"),
    }
}

fn canonical_service() -> Service {
    Service {
        id: WorkloadId::new("svc-v1-service").expect("valid workload id"),
        replicas: NonZeroU32::new(2).expect("non-zero replicas"),
        resources: Resources { cpu_milli: 500, memory_bytes: 128 * 1024 * 1024 },
        driver: vm("/usr/bin/frontend"),
        listeners: vec![Listener {
            port: NonZeroU16::new(8080).expect("non-zero port"),
            protocol: Proto::Tcp,
        }],
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

fn canonical_schedule() -> Schedule {
    Schedule {
        id: WorkloadId::new("svc-v1-schedule").expect("valid workload id"),
        job: Job {
            id: WorkloadId::new("svc-v1-schedule").expect("valid workload id"),
            replicas: NonZeroU32::new(1).expect("non-zero replicas"),
            resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: vm("/usr/local/bin/cleanup"),
        },
        cron_expr: CronExpr::new("0 2 * * *").expect("valid cron expr"),
    }
}

const FIXTURE_V1: &str = "7376632d76312d6a6f622f7362696e2f696e69742d2d7175696574ff2f7661722f6c69622f6f76657264726976652f766d6c696e75782f7661722f6c69622f6f76657264726976652f726f6f7466732e696d670000000000000000000000000000000000000000008a00000098ffffff010000000000000064000000000000000000000400000000000000008a0000007effffff80ffffff010000009a00000080ffffff9d00000092ffffff000000000000000000000000000000000000000000000000000000000000000000000000";
const FIXTURE_V1_SERVICE: &str = "7376632d76312d736572766963652f7573722f62696e2f66726f6e74656e64002d2d7175696574ff2f7661722f6c69622f6f76657264726976652f766d6c696e75782f7661722f6c69622f6f76657264726976652f726f6f7466732e696d6700901f000000000000000000000000000001000000000000008e00000088ffffff0200000000000000f4010000000000000000000800000000000000009100000072ffffff7cffffff010000009a0000007cffffff9d0000008effffffa4ffffff01000000a0ffffff0000000098ffffff0000000090ffffff0000000000000000";
const FIXTURE_V1_SCHEDULE: &str = "7376632d76312d7363686564756c657376632d76312d7363686564756c652f7573722f6c6f63616c2f62696e2f636c65616e75702d2d7175696574ff2f7661722f6c69622f6f76657264726976652f766d6c696e75782f7661722f6c69622f6f76657264726976652f726f6f7466732e696d67302032202a202a202a00000000000000000000000002000000000000008f00000070ffffff8f00000077ffffff010000000000000064000000000000000000000400000000000000009600000062ffffff70ffffff010000009a00000070ffffff9d00000082ffffff000000008900000093ffffff00000000000000000000000000000000";

fn archive_hex(intent: &WorkloadIntent) -> String {
    let envelope = WorkloadIntentEnvelope::latest(intent.clone());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope)
        .expect("WorkloadIntent envelope must archive");
    hex::encode(bytes.as_ref())
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn workload_intent_v1_job_decodes_through_current_envelope() {
    let expected = WorkloadIntent::Job(canonical_job());
    assert_eq!(archive_hex(&expected), FIXTURE_V1);
    assert_envelope_v_roundtrip::<WorkloadIntentEnvelope>(FIXTURE_V1, &expected);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn workload_intent_v1_service_decodes_through_current_envelope() {
    let expected = WorkloadIntent::Service(canonical_service());
    assert_eq!(archive_hex(&expected), FIXTURE_V1_SERVICE);
    assert_envelope_v_roundtrip::<WorkloadIntentEnvelope>(FIXTURE_V1_SERVICE, &expected);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn workload_intent_v1_schedule_decodes_through_current_envelope() {
    let expected = WorkloadIntent::Schedule(canonical_schedule());
    assert_eq!(archive_hex(&expected), FIXTURE_V1_SCHEDULE);
    assert_envelope_v_roundtrip::<WorkloadIntentEnvelope>(FIXTURE_V1_SCHEDULE, &expected);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn workload_intent_v1_codec_roundtrips_each_kind() {
    for expected in [
        WorkloadIntentLatest::Job(canonical_job()),
        WorkloadIntentLatest::Service(canonical_service()),
        WorkloadIntentLatest::Schedule(canonical_schedule()),
    ] {
        let bytes = expected.archive_for_store().expect("archive_for_store must succeed");
        let decoded = WorkloadIntent::from_store_bytes(
            bytes.as_ref(),
            std::path::Path::new("schema_evolution.redb"),
            None,
        )
        .expect("current V1 bytes must decode");
        assert_eq!(decoded, expected);
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn workload_intent_v1_envelope_reports_sole_discriminant() {
    assert_eq!(WorkloadIntentEnvelope::known_discriminants(), &[0]);
    assert_eq!(WorkloadIntentEnvelope::type_name(), "WorkloadIntentEnvelope");
}

//! Node-local persistent volume (GH #348, ADR-0310, ADR-0311): the operator's
//! `[vm.volume] size = "40GiB"` declaration, from TOML to the persisted intent.
//!
//! Driven through the real driving ports the spec path already uses:
//! `WorkloadSpecInput::from_toml_str` (the TOML parser), the wire shapes
//! `JobSpecInput` / `ServiceSpecInput` and their validating constructors
//! `Job::from_submit` / `Service::from_submit`, the intent codec
//! (`archive_for_store`, `from_store_bytes`, `spec_digest`), and the echo
//! `JobSpecInput::from(&Job)`. No scaffold is needed on the production side:
//! the tests reach the new `volume` field through text and JSON, so each one
//! fails today at the first place the field is refused.
//!
//! Evidence tier: in-memory acceptance (layer 2). Error and boundary scenarios
//! are the majority. A scenario that holds before and after the feature is
//! marked GUARD and is not pending.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::path::Path;

use overdrive_core::aggregate::{
    AggregateError, Job, JobSpecInput, Service, WorkloadIntent, WorkloadIntentEnvelope,
    WorkloadSpecInput,
};
use overdrive_core::api::submit::ServiceSpecInput;
use overdrive_core::codec::VersionedEnvelope;
use serde_json::{Value, json};

const JOB_BASE: &str = "[job]\nid = \"dev-box\"\n\n[vm]\ncommand = \"/sbin/init\"\nargs = []\nkernel = \"/k\"\nrootfs = \"/r\"\n";
const RESOURCES: &str = "\n[resources]\ncpu_milli = 500\nmemory_bytes = 134217728\n";

fn job_toml(volume: &str) -> String {
    format!("{JOB_BASE}{volume}{RESOURCES}")
}

fn wire_job(replicas: u32, volume: &Value) -> Value {
    let mut vm = json!({"command": "/sbin/init", "args": [], "kernel": "/k", "rootfs": "/r"});
    if !volume.is_null() {
        vm["volume"] = volume.clone();
    }
    json!({"id": "dev-box", "replicas": replicas,
           "resources": {"cpu_milli": 500, "memory_bytes": 134_217_728u64}, "vm": vm})
}

fn admit_job(replicas: u32, volume: &Value) -> Result<Job, AggregateError> {
    let spec: JobSpecInput = serde_json::from_value(wire_job(replicas, volume))
        .expect("a [vm.volume] declaration must be part of the wire spec");
    Job::from_submit(spec)
}

fn admit_service(replicas: u32, volume: &Value) -> Result<Service, AggregateError> {
    let mut wire = wire_job(replicas, volume);
    wire["listeners"] = json!([{"port": 8080, "protocol": "tcp"}]);
    let spec: ServiceSpecInput = serde_json::from_value(wire)
        .expect("a [vm.volume] declaration must be part of the wire spec");
    Service::from_submit(spec)
}

fn validation(err: AggregateError) -> (&'static str, String) {
    match err {
        AggregateError::Validation { field, message } => (field, message),
        other => panic!("expected AggregateError::Validation, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Parser: the TOML an operator writes
// ---------------------------------------------------------------------------

/// A job declaring a 40 GiB volume parses.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "a [vm.volume] size = \"40GiB\" declaration must parse")]
fn a_job_with_a_volume_declaration_parses() {
    WorkloadSpecInput::from_toml_str(&job_toml("\n[vm.volume]\nsize = \"40GiB\"\n"))
        .expect("a [vm.volume] size = \"40GiB\" declaration must parse");
}

/// A service declaring a volume parses.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "a service [vm.volume] declaration must parse")]
fn a_service_with_a_volume_declaration_parses() {
    let toml = format!(
        "[service]\nid = \"dev-box\"\n\n[[listener]]\nport = 8080\nprotocol = \"tcp\"\n\n[vm]\ncommand = \"/sbin/init\"\nargs = []\nkernel = \"/k\"\nrootfs = \"/r\"\n\n[vm.volume]\nsize = \"1TiB\"\n{RESOURCES}"
    );
    WorkloadSpecInput::from_toml_str(&toml).expect("a service [vm.volume] declaration must parse");
}

/// Sad: only one volume, only `size` (limitation L1, ADR-0310: no named volumes).
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(
    expected = "the declared [vm.volume] size = \"40GiB\" must parse before a bad key can be refused"
)]
fn sad_the_volume_table_refuses_unknown_keys_an_array_and_a_missing_size() {
    // The control: the same table with only a valid size must parse, otherwise a refusal below proves nothing.
    WorkloadSpecInput::from_toml_str(&job_toml("\n[vm.volume]\nsize = \"40GiB\"\n")).expect(
        "the declared [vm.volume] size = \"40GiB\" must parse before a bad key can be refused",
    );
    for bad in [
        "\n[vm.volume]\nsize = \"40GiB\"\nname = \"data\"\n",
        "\n[vm.volume]\nsizee = \"40GiB\"\n",
        "\n[[vm.volume]]\nsize = \"40GiB\"\n",
        "\n[vm.volume]\n",
        "\n[vm.volume]\nsize = 40\n",
    ] {
        assert!(
            WorkloadSpecInput::from_toml_str(&job_toml(bad)).is_err(),
            "{bad:?} must be refused"
        );
    }
}

// ---------------------------------------------------------------------------
// Admission: validation at the validating constructors
// ---------------------------------------------------------------------------

/// An admitted job echoes its declaration back on the wire.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "a [vm.volume] declaration must be part of the wire spec")]
fn an_admitted_job_echoes_the_declared_size() {
    let job = admit_job(1, &json!({"size": "40GiB"})).expect("one replica with a volume is valid");
    let echoed = serde_json::to_value(JobSpecInput::from(&job)).expect("echo serialises");
    assert_eq!(echoed["vm"]["volume"]["size"], "40GiB");
}

/// A size in TiB is echoed in its canonical spelling.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "a [vm.volume] declaration must be part of the wire spec")]
fn the_echo_uses_the_canonical_size_spelling() {
    let job = admit_job(1, &json!({"size": "1024gib"})).expect("valid");
    let echoed = serde_json::to_value(JobSpecInput::from(&job)).expect("echo serialises");
    assert_eq!(echoed["vm"]["volume"]["size"], "1TiB");
}

/// Sad: a size outside the grammar is rejected naming `vm.volume.size`.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "a [vm.volume] declaration must be part of the wire spec")]
fn sad_a_malformed_size_is_rejected_naming_the_size_field() {
    for bad in ["0GiB", "40GB", "forty", ""] {
        let err =
            admit_job(1, &json!({"size": bad})).expect_err("a malformed size must be rejected");
        assert_eq!(validation(err).0, "vm.volume.size", "{bad:?}");
    }
}

/// Sad: a volume is a single-writer disk, so it requires one replica (ADR-0310).
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "a [vm.volume] declaration must be part of the wire spec")]
fn sad_a_volume_with_several_replicas_is_rejected_for_job_and_service() {
    for replicas in [2u32, 3, 100] {
        let (field, message) = validation(
            admit_job(replicas, &json!({"size": "40GiB"})).expect_err("job with replicas"),
        );
        assert_eq!(field, "vm.volume");
        assert_eq!(message, "a volume requires replicas = 1");
        let (field, message) = validation(
            admit_service(replicas, &json!({"size": "40GiB"})).expect_err("service with replicas"),
        );
        assert_eq!((field, message.as_str()), ("vm.volume", "a volume requires replicas = 1"));
    }
}

/// A service with one replica and a volume is admitted.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "a [vm.volume] declaration must be part of the wire spec")]
fn a_single_replica_service_with_a_volume_is_admitted() {
    admit_service(1, &json!({"size": "2TiB"})).expect("one replica with a volume is valid");
}

/// GUARD (boundary 6, feature disabled): a spec with no volume echoes no volume
/// key, so the pre-feature path is unchanged.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn guard_a_job_without_a_volume_echoes_no_volume() {
    let job = admit_job(3, &Value::Null).expect("replicas = 3 without a volume is valid");
    let echoed = serde_json::to_value(JobSpecInput::from(&job)).expect("echo serialises");
    assert!(echoed["vm"].get("volume").is_none(), "no volume declared, none echoed: {echoed}");
}

// ---------------------------------------------------------------------------
// Persistence: the volume is part of the immutable spec identity (ADR-0311)
// ---------------------------------------------------------------------------

fn archived(job: Job) -> (Vec<u8>, overdrive_core::id::ContentHash, WorkloadIntent) {
    let intent = WorkloadIntent::Job(job);
    let bytes = intent.archive_for_store().expect("archives").to_vec();
    let digest = intent.spec_digest().expect("digests");
    (bytes, digest, intent)
}

/// A volume declaration survives the intent store round trip.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "a [vm.volume] declaration must be part of the wire spec")]
fn the_volume_declaration_survives_the_intent_store_round_trip() {
    let (bytes, _, intent) = archived(admit_job(1, &json!({"size": "40GiB"})).unwrap());
    let decoded = WorkloadIntent::from_store_bytes(&bytes, Path::new("volume.redb"), None)
        .expect("current bytes decode");
    assert_eq!(decoded, intent);
    let WorkloadIntent::Job(job) = decoded else { panic!("a job stays a job") };
    let echoed = serde_json::to_value(JobSpecInput::from(&job)).unwrap();
    assert_eq!(echoed["vm"]["volume"]["size"], "40GiB");
}

/// The size is identity: the same workload with another size, or with no
/// volume, is a different spec.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "a [vm.volume] declaration must be part of the wire spec")]
fn the_spec_digest_changes_with_the_volume_declaration() {
    let digest = |v: &Value| archived(admit_job(1, v).unwrap()).1;
    let forty = digest(&json!({"size": "40GiB"}));
    assert_eq!(forty, digest(&json!({"size": "40gib"})), "the same size spelled twice is one spec");
    assert_ne!(forty, digest(&json!({"size": "80GiB"})), "size is part of the identity");
    assert_ne!(forty, digest(&Value::Null), "a volume is part of the identity");
}

/// The intent envelope gains a second version for the volume field; version 1
/// stays decodable (ADR-0048, development.md rkyv schema evolution).
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the workload intent envelope must know discriminants [0, 1]")]
fn the_intent_envelope_gains_version_two_and_keeps_version_one() {
    assert_eq!(
        WorkloadIntentEnvelope::known_discriminants(),
        &[0, 1],
        "the workload intent envelope must know discriminants [0, 1]"
    );
}

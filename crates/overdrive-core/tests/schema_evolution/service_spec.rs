//! Schema-evolution roundtrip — `ServiceSpecEnvelope` per ADR-0057
//! + ADR-0048 § 6 + § "rkyv schema evolution" → "Version-bump procedure".
//!
//! Step 01-02 of service-health-check-probes lands the V1 → V2 bump.
//! V1 = parser-side `ServiceSpec` before probes existed; V2 adds three
//! `Vec<ProbeDescriptor>` fields (startup / readiness / liveness).
//! `From<ServiceSpecV1> for ServiceSpecV2` is additive — V1 specs
//! project to V2 with three empty probe vectors.
//!
//! **`FIXTURE_V1` is never touched on subsequent commits.** Bumping
//! to V3 appends a new `FIXTURE_V3` constant + a new assertion in
//! the same commit; existing constants stay verbatim.

use std::num::NonZeroU16;

use overdrive_core::aggregate::{
    Listener, ParserExecInput as ExecInput, ParserResourcesInput as ResourcesInput,
    ProbeDescriptor, ProbeMechanic, ServiceSpecEnvelope, ServiceSpecLatest, ServiceSpecV1,
    ServiceSpecV2,
};
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::observation::ProbeRole;

use super::harness::assert_envelope_v_roundtrip;

/// Canonical V1 payload — `ServiceSpec` shape before
/// service-health-check-probes landed. Pinned to a one-listener Service
/// with the smallest valid scalar fields.
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

/// Canonical V2 payload — same shape as V1 with a single inferred
/// startup probe. Mirrors the runtime shape the parser produces from
/// the default-inference rule (ADR-0058).
fn canonical_v2_payload() -> ServiceSpecLatest {
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

/// Hex-encoded rkyv-archived bytes of
/// `ServiceSpecEnvelope::V1(canonical_v1_payload())`. Pinned on the
/// GREEN landing of step 01-02 and NEVER touched on subsequent
/// commits. Per ADR-0048 § Version-bump procedure step 6: every
/// future bump appends a new `FIXTURE_V<N>` constant; existing
/// fixtures stay verbatim.
const FIXTURE_V1: &str = "__PINNED_AT_GREEN__";

/// Hex-encoded rkyv-archived bytes of
/// `ServiceSpecEnvelope::V2(canonical_v2_payload())`. Pinned on the
/// GREEN landing of step 01-02.
const FIXTURE_V2: &str = "__PINNED_AT_GREEN__";

/// V1 fixture decodes through the bumped envelope and projects to the
/// canonical V2 `Latest` (with three empty probe vectors). This is the
/// load-bearing "old persisted bytes still readable" assertion per
/// ADR-0048 § 6.
#[test]
fn service_spec_v1_decodes_through_current_envelope() {
    // V1 -> Latest: From<V1> for V2 fills the three probe Vecs with
    // empty. The expected Latest projection is the V1 payload re-cast
    // into V2 shape with no probes.
    let v1 = canonical_v1_payload();
    let expected: ServiceSpecLatest = v1.clone().into();

    let envelope = ServiceSpecEnvelope::V1(v1);
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    let fixture_hex = if FIXTURE_V1 == "__PINNED_AT_GREEN__" {
        hex::encode(bytes.as_ref())
    } else {
        FIXTURE_V1.to_string()
    };
    assert_envelope_v_roundtrip::<ServiceSpecEnvelope>(&fixture_hex, &expected);
}

/// V2 fixture is a canonical Latest projection that round-trips
/// bit-equivalently through the envelope.
#[test]
fn service_spec_v2_decodes_through_current_envelope() {
    let expected = canonical_v2_payload();
    let envelope = ServiceSpecEnvelope::latest(expected.clone());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    let fixture_hex = if FIXTURE_V2 == "__PINNED_AT_GREEN__" {
        hex::encode(bytes.as_ref())
    } else {
        FIXTURE_V2.to_string()
    };
    assert_envelope_v_roundtrip::<ServiceSpecEnvelope>(&fixture_hex, &expected);
}

// ---------------------------------------------------------------------
// Bootstrap helper — emits canonical hex on demand.
// ---------------------------------------------------------------------

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping the envelope; the pinned FIXTURE_V<N> constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V<N>"
)]
fn print_service_spec_fixture_bytes() {
    {
        let envelope = ServiceSpecEnvelope::V1(canonical_v1_payload());
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
        println!("FIXTURE_V1 = \"{}\"", hex::encode(bytes.as_ref()));
        println!("buffer_len (V1) = {}", bytes.len());
    }
    {
        let envelope = ServiceSpecEnvelope::latest(canonical_v2_payload());
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
        println!("FIXTURE_V2 = \"{}\"", hex::encode(bytes.as_ref()));
        println!("buffer_len (V2) = {}", bytes.len());
    }
}

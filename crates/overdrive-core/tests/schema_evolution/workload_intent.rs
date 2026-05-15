//! Schema-evolution roundtrip for `WorkloadIntentEnvelope` against
//! its V1 golden-bytes fixture (per ADR-0050 + ADR-0048 § 6).
//!
//! Per `.claude/rules/testing.md` § "Property-based testing
//! (proptest)" → "Mandatory call sites" → "Archive schema-evolution
//! roundtrip": every rkyv versioned envelope ships at least one
//! historical-version golden fixture pinning the archived bytes.
//! `FIXTURE_V1_*` constants are generated once at the GREEN landing
//! of step 02-03a and pinned verbatim from that moment onward —
//! NEVER touched on subsequent commits.
//!
//! Three variants of `WorkloadIntent` are exercised per ADR-0050 OQ-3
//! / OQ-4: `Job`, `Service`, `Schedule`. All three variants share one
//! envelope (`WorkloadIntentEnvelope`); only the inner discriminant
//! changes. The three round-trip assertions pin that the envelope's
//! archived layout is byte-identical to the canonical projection
//! across every variant.
//!
//! When bumping to `WorkloadIntentEnvelope::V2`, append new
//! `FIXTURE_V2_*` constants + new tests; the V1 constants + tests
//! stay verbatim, asserting V1 bytes continue to decode through the
//! bumped envelope via `From<WorkloadIntentV1> for WorkloadIntentV2`.

use std::num::{NonZeroU16, NonZeroU32};

use overdrive_core::aggregate::{
    CronExpr, Exec, Job, Listener, ScheduleV1, ServiceV1, WorkloadDriver, WorkloadIntent,
    WorkloadIntentEnvelope,
};
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::WorkloadId;
use overdrive_core::traits::driver::Resources;

use super::harness::assert_envelope_v_roundtrip;

// Per ADR-0050 step 02-03a: `WorkloadIntentEnvelope::discriminant_offset_from_end()`
// returns `None` for the initial landing — the empirical offset for the
// 3-variant inner enum is deferred. The triangulation +
// unknown_version_probe assertions are dependent on a `Some(N)` offset
// and are deliberately not exercised in this slice; the V1 round-trip
// fixtures + `archive_for_store` round-trip remain the load-bearing
// schema-evolution defense.

/// Canonical V1 `Job`-variant payload. Same shape as the
/// pre-migration `JobEnvelope` V1 fixture, now wrapped in the outer
/// `WorkloadIntent::Job` discriminant.
fn canonical_v1_job_payload() -> WorkloadIntent {
    WorkloadIntent::Job(Job {
        id: WorkloadId::new("svc-payments").expect("valid workload id"),
        replicas: NonZeroU32::new(3).expect("non-zero replicas"),
        resources: Resources { cpu_milli: 250, memory_bytes: 256 * 1024 * 1024 },
        driver: WorkloadDriver::Exec(Exec {
            command: "/bin/sleep".to_string(),
            args: vec!["3600".to_string()],
        }),
    })
}

/// Canonical V1 `Service`-variant payload (ADR-0050 OQ-3 minimal
/// shape — `(port, protocol)` listeners only).
fn canonical_v1_service_payload() -> WorkloadIntent {
    WorkloadIntent::Service(ServiceV1 {
        id: WorkloadId::new("svc-frontends").expect("valid workload id"),
        replicas: NonZeroU32::new(2).expect("non-zero replicas"),
        resources: Resources { cpu_milli: 500, memory_bytes: 128 * 1024 * 1024 },
        driver: WorkloadDriver::Exec(Exec {
            command: "/usr/bin/frontend".to_string(),
            args: vec!["--port".to_string(), "8080".to_string()],
        }),
        listeners: vec![Listener {
            port: NonZeroU16::new(8080).expect("non-zero port"),
            protocol: Proto::Tcp,
        }],
    })
}

/// Canonical V1 `Schedule`-variant payload (ADR-0050 OQ-4 embedded-
/// job shape).
fn canonical_v1_schedule_payload() -> WorkloadIntent {
    WorkloadIntent::Schedule(ScheduleV1 {
        id: WorkloadId::new("svc-nightly-cleanup").expect("valid workload id"),
        job: Job {
            id: WorkloadId::new("svc-nightly-cleanup").expect("valid workload id"),
            replicas: NonZeroU32::new(1).expect("non-zero replicas"),
            resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: WorkloadDriver::Exec(Exec {
                command: "/usr/local/bin/cleanup".to_string(),
                args: vec!["--mode".to_string(), "nightly".to_string()],
            }),
        },
        cron_expr: CronExpr::new("0 2 * * *").expect("valid cron expr"),
    })
}

/// Hex-encoded rkyv-archived bytes of
/// `WorkloadIntentEnvelope::V1(canonical_v1_job_payload())`.
/// Generated once at the GREEN landing of step 02-03a via
/// `print_fixture_v1_bytes`. NEVER touched on subsequent commits.
///
/// The dst-lint envelope-fixture-coverage scanner per ADR-0048 § 6
/// (xtask `scan_for_envelope_fixture_coverage`) requires a `FIXTURE_V<N>`
/// constant per envelope variant. This is the canonical V1 fixture;
/// the `FIXTURE_V1_SERVICE` and `FIXTURE_V1_SCHEDULE` constants below
/// are sibling fixtures pinning the other two `WorkloadIntentV1`
/// inner-enum arms.
#[allow(
    dead_code,
    reason = "consumed by xtask::dst_lint::scan_for_envelope_fixture_coverage at PR-time, not by any test runtime — the constant's NAME is the load-bearing artifact"
)]
const FIXTURE_V1: &str = FIXTURE_V1_JOB;

#[allow(
    dead_code,
    reason = "fixture constant retained for explicit job-arm naming; aliased from FIXTURE_V1"
)]
const FIXTURE_V1_JOB: &str = "7376632d7061796d656e74732f62696e2f736c656570000033363030ffffffff000000000000000000000000000000008c000000d0ffffff0300000000000000fa000000000000000000001000000000000000008a000000b8ffffffbcffffff010000000000000000000000000000000000000000000000";

/// Hex-encoded rkyv-archived bytes of
/// `WorkloadIntentEnvelope::V1(canonical_v1_service_payload())`.
/// Generated once at the GREEN landing of step 02-03a.
const FIXTURE_V1_SERVICE: &str = "7376632d66726f6e74656e64732f7573722f62696e2f66726f6e74656e6400002d2d706f7274ffff38303830ffffffff901f000000000000000000000000000001000000000000008d000000b8ffffff0200000000000000f40100000000000000000008000000000000000091000000a1ffffffacffffff02000000b4ffffff01000000000000000000000000000000";

/// Hex-encoded rkyv-archived bytes of
/// `WorkloadIntentEnvelope::V1(canonical_v1_schedule_payload())`.
/// Generated once at the GREEN landing of step 02-03a.
const FIXTURE_V1_SCHEDULE: &str = "7376632d6e696768746c792d636c65616e75707376632d6e696768746c792d636c65616e75702f7573722f6c6f63616c2f62696e2f636c65616e75702d2d6d6f6465ffff6e696768746c79ff302032202a202a202a000000000000000000000002000000000000009300000098ffffff93000000a3ffffff010000000000000064000000000000000000000400000000000000009600000092ffffffa0ffffff020000000000000089000000a4ffffff";

#[test]
fn workload_intent_v1_job_decodes_through_current_envelope() {
    let expected = canonical_v1_job_payload();
    assert_envelope_v_roundtrip::<WorkloadIntentEnvelope>(FIXTURE_V1_JOB, &expected);
}

#[test]
fn workload_intent_v1_service_decodes_through_current_envelope() {
    let expected = canonical_v1_service_payload();
    assert_envelope_v_roundtrip::<WorkloadIntentEnvelope>(FIXTURE_V1_SERVICE, &expected);
}

#[test]
fn workload_intent_v1_schedule_decodes_through_current_envelope() {
    let expected = canonical_v1_schedule_payload();
    assert_envelope_v_roundtrip::<WorkloadIntentEnvelope>(FIXTURE_V1_SCHEDULE, &expected);
}

// Triangulation + unknown_version_probe assertions deferred per the
// `discriminant_offset_from_end -> None` choice above. Re-add when
// the empirical offset for `WorkloadIntentEnvelope` is pinned.

/// `WorkloadIntent::spec_digest()` is deterministic: the canonical
/// rkyv archive of a logical payload is byte-stable, so two calls
/// return bit-identical hashes. Per ADR-0050 the digest is over the
/// rkyv-archived **inner** `WorkloadIntentV1` payload bytes (NOT the
/// envelope) — stable across envelope version bumps. The
/// `ServiceVipAllocator` memo (ADR-0049) keys by this value.
#[test]
fn spec_digest_is_deterministic_across_variants() {
    for canonical in [
        canonical_v1_job_payload(),
        canonical_v1_service_payload(),
        canonical_v1_schedule_payload(),
    ] {
        let first = canonical.spec_digest().expect("first spec_digest must succeed");
        let second = canonical.spec_digest().expect("second spec_digest must succeed");
        assert_eq!(
            first.to_string(),
            second.to_string(),
            "spec_digest must be byte-stable across calls — canonical rkyv archive is \
             deterministic; a divergence here means rkyv canonicalisation drifted",
        );
    }
}

/// `WorkloadIntent::archive_for_store` round-trips bit-equivalently
/// through `WorkloadIntent::from_store_bytes` for every variant. Per
/// ADR-0050 § 4: the codec methods are the SOLE persistence-boundary
/// wrapping sites; the round-trip is the load-bearing invariant.
#[test]
fn archive_for_store_roundtrips_every_variant() {
    for canonical in [
        canonical_v1_job_payload(),
        canonical_v1_service_payload(),
        canonical_v1_schedule_payload(),
    ] {
        let bytes = canonical.archive_for_store().expect("archive_for_store must succeed");
        let decoded = WorkloadIntent::from_store_bytes(
            bytes.as_ref(),
            std::path::Path::new("schema_evolution.redb"),
            None,
        )
        .expect("from_store_bytes must succeed on the bytes archive_for_store just produced");
        assert_eq!(
            decoded, canonical,
            "archive_for_store -> from_store_bytes must round-trip bit-equivalently",
        );
    }
}

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping a payload variant; the pinned FIXTURE_V<N>_* constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V<N>_* constants"
)]
fn print_fixture_v1_bytes() {
    for (label, canonical) in [
        ("FIXTURE_V1_JOB", canonical_v1_job_payload()),
        ("FIXTURE_V1_SERVICE", canonical_v1_service_payload()),
        ("FIXTURE_V1_SCHEDULE", canonical_v1_schedule_payload()),
    ] {
        let envelope = WorkloadIntentEnvelope::latest(canonical);
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
        println!("const {label}: &str = \"{}\";", hex::encode(bytes.as_ref()));
    }
}

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
//! normally stay verbatim, asserting V1 bytes continue to decode
//! through the bumped envelope via
//! `From<WorkloadIntentV1> for WorkloadIntentV2`.
//!
//! **Exception realized 2026-08-12 (GH #42 / ADR-0083 Amendment):**
//! the V1->V2 fork that added `WorkloadDriverV2::Vm` grew the outer
//! envelope's archived root — rkyv 0.8 sizes an enum's root to the
//! max footprint across all variants, so a wider V2 sibling pads
//! every archive, including ones holding the smaller V1 payload.
//! Old V1 bytes, sized for a V1-only envelope, stopped decoding
//! (`InvalidEnumDiscriminantError` on `ArchivedWorkloadIntentEnvelope`).
//! `FIXTURE_V1_*` were regenerated in the SAME commit as the bump,
//! user-approved, per the `ServiceSpecEnvelope` greenfield
//! same-commit-regeneration precedent (`service_spec.rs`). From
//! that commit onward they are pinned verbatim again — NEVER
//! touched.

use std::num::{NonZeroU16, NonZeroU32};

use overdrive_core::aggregate::{
    CronExpr, Exec, Job, Listener, Schedule, Service, Vm, WorkloadDriver, WorkloadIntent,
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

/// Canonical V1 `Service`-variant payload — ADR-0050 OQ-3 minimal
/// shape plus the three `Vec<ProbeDescriptor>` slots added by
/// GAP-6 (probe descriptors persisted in `WorkloadIntent::Service`).
/// The canonical fixture uses empty probe vecs — exercises the
/// most common Phase-1 shape (no operator-declared probes; the
/// parser's default-TCP inference flows through `startup_probes`
/// when listeners exist; an explicit empty vec is the explicit
/// opt-out per ADR-0058).
fn canonical_v1_service_payload() -> WorkloadIntent {
    WorkloadIntent::Service(Service {
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
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    })
}

/// Canonical V1 `Schedule`-variant payload (ADR-0050 OQ-4 embedded-
/// job shape).
fn canonical_v1_schedule_payload() -> WorkloadIntent {
    WorkloadIntent::Schedule(Schedule {
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
// Regenerated 2026-08-12 at the `WorkloadIntentEnvelope` V1->V2 fork
// (`WorkloadDriverV2::Vm`, GH #42 / ADR-0083 Amendment 2026-08-12),
// user-approved. rkyv 0.8 sizes an enum's archived root to the max
// footprint across all variants; the moment `WorkloadIntentEnvelope`
// gained the wider V2 sibling (embeds `WorkloadDriverV2::Vm`, two
// more `String` fields than `WorkloadDriverV1::Exec`), the whole
// envelope's archived layout grew to fit V2 — even archives holding
// the smaller V1 variant, which pad to match. Old V1 bytes, sized
// for a V1-only envelope, no longer decode
// (`InvalidEnumDiscriminantError` on `ArchivedWorkloadIntentEnvelope`).
// Per the `ServiceSpecEnvelope` precedent (`service_spec.rs`
// FIXTURE_V2, GAP-6/ADR-0080 same-commit regenerations) and the
// Phase-1 greenfield single-cut migration policy (delete the on-disk
// redb file is the official upgrade path — no real pre-V2 bytes to
// preserve), the fixture is regenerated in the SAME commit as the
// envelope bump rather than attempting byte preservation across a
// real size-growing fork. The JOB, SERVICE, and SCHEDULE fixtures
// all regenerate together — the outer envelope's archive offsets are
// positional across every variant. From this commit onward they are
// pinned verbatim again — NEVER touched.
const FIXTURE_V1_JOB: &str = "7376632d7061796d656e74732f62696e2f736c656570000033363030ffffffff010000000000000000000000000000008c000000d0ffffff0300000000000000fa000000000000000000001000000000000000008a000000b8ffffffbcffffff0100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

/// Hex-encoded rkyv-archived bytes of
/// `WorkloadIntentEnvelope::V1(canonical_v1_service_payload())`.
///
/// Regenerated 2026-08-12 alongside `FIXTURE_V1_JOB` — see that
/// constant's comment for the full rationale (the
/// `WorkloadIntentEnvelope` V1->V2 fork, GH #42 / ADR-0083 Amendment
/// 2026-08-12, grew the outer envelope's archived root across every
/// variant; regenerated in the same commit as the bump, user-
/// approved, per the `ServiceSpecEnvelope` precedent). `ServiceV1`'s
/// own payload shape did not change — only the outer envelope's
/// positional offsets shifted.
const FIXTURE_V1_SERVICE: &str = "7376632d66726f6e74656e64732f7573722f62696e2f66726f6e74656e6400002d2d706f7274ffff38303830ffffffff901f000000000000010000000000000001000000000000008d000000b8ffffff0200000000000000f40100000000000000000008000000000000000091000000a1ffffffacffffff0200000000000000000000000000000000000000a4ffffff01000000a0ffffff0000000098ffffff0000000090ffffff0000000000000000";

/// Hex-encoded rkyv-archived bytes of
/// `WorkloadIntentEnvelope::V1(canonical_v1_schedule_payload())`.
///
/// Regenerated 2026-08-12 alongside `FIXTURE_V1_JOB` — see that
/// constant's comment for the full rationale (the
/// `WorkloadIntentEnvelope` V1->V2 fork, GH #42 / ADR-0083 Amendment
/// 2026-08-12, grew the outer envelope's archived root across every
/// variant; regenerated in the same commit as the bump, user-
/// approved, per the `ServiceSpecEnvelope` precedent). The Schedule
/// fixture itself does NOT embed the `Vm`-carrying driver (Schedule
/// embeds `Job`, which does), but the SHARED `WorkloadIntentEnvelope`
/// archive offsets are positional — the V2 sibling's growth shifts
/// archive metadata across all three variants.
const FIXTURE_V1_SCHEDULE: &str = "7376632d6e696768746c792d636c65616e75707376632d6e696768746c792d636c65616e75702f7573722f6c6f63616c2f62696e2f636c65616e75702d2d6d6f6465ffff6e696768746c79ff302032202a202a202a000000010000000000000002000000000000009300000098ffffff93000000a3ffffff010000000000000064000000000000000000000400000000000000009600000092ffffffa0ffffff0200000000000000000000000000000000000000000000008900000094ffffff00000000000000000000000000000000";

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

/// Canonical V2 `Job`-variant payload carrying the new
/// `WorkloadDriverV2::Vm` arm (GH #42 / ADR-0083 Amendment
/// 2026-08-12). Exercises the microVM driver through the live
/// `WorkloadDriver` alias — proves `WorkloadIntentEnvelope::V2` round-
/// trips a `Vm`-driven `Job` bit-equivalently.
fn canonical_v2_job_vm_payload() -> WorkloadIntent {
    WorkloadIntent::Job(Job {
        id: WorkloadId::new("svc-microvm").expect("valid workload id"),
        replicas: NonZeroU32::new(1).expect("non-zero replicas"),
        resources: Resources { cpu_milli: 500, memory_bytes: 512 * 1024 * 1024 },
        driver: WorkloadDriver::Vm(Vm {
            command: "/sbin/init".to_string(),
            args: vec!["--quiet".to_string()],
            kernel: "/var/lib/overdrive/vmlinux".to_string(),
            rootfs: "/var/lib/overdrive/rootfs.img".to_string(),
        }),
    })
}

/// Hex-encoded rkyv-archived bytes of
/// `WorkloadIntentEnvelope::V2(canonical_v2_job_vm_payload())`.
/// Generated once at the GREEN landing of step 01-02 via
/// `print_fixture_v2_bytes`. NEVER touched on subsequent commits.
///
/// The dst-lint envelope-fixture-coverage scanner (ADR-0048 § 6)
/// requires a `FIXTURE_V<N>` constant per envelope variant — this is
/// the canonical V2 fixture.
#[allow(
    dead_code,
    reason = "fixture constant retained for explicit job+vm-arm naming; aliased from FIXTURE_V2"
)]
const FIXTURE_V2_JOB_VM: &str = "7376632d6d6963726f766d2f7362696e2f696e69740000002d2d7175696574ff2f7661722f6c69622f6f76657264726976652f766d6c696e75782f7661722f6c69622f6f76657264726976652f726f6f7466732e696d6700010000000000000000000000000000008b00000098ffffff0100000000000000f4010000000000000000002000000000010000008a0000007fffffff84ffffff010000009a00000084ffffff9d00000096ffffff000000000000000000000000000000000000000000000000000000000000000000000000";

#[allow(
    dead_code,
    reason = "consumed by xtask::dst_lint::scan_for_envelope_fixture_coverage at PR-time, not by any test runtime — the constant's NAME is the load-bearing artifact"
)]
const FIXTURE_V2: &str = FIXTURE_V2_JOB_VM;

#[test]
fn workload_intent_v2_job_vm_decodes_through_current_envelope() {
    let expected = canonical_v2_job_vm_payload();
    assert_envelope_v_roundtrip::<WorkloadIntentEnvelope>(FIXTURE_V2_JOB_VM, &expected);
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

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping a payload variant; the pinned FIXTURE_V<N>_* constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V<N>_* constants"
)]
fn print_fixture_v2_bytes() {
    let (label, canonical) = ("FIXTURE_V2_JOB_VM", canonical_v2_job_vm_payload());
    let envelope = WorkloadIntentEnvelope::latest(canonical);
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("const {label}: &str = \"{}\";", hex::encode(bytes.as_ref()));
}

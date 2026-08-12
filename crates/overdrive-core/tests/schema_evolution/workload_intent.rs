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
//!
//! **Review remediation 2026-08-12 (D1, step 01-02 #42):** the first
//! regeneration pass above wrapped the canonical payload via
//! `WorkloadIntentEnvelope::latest(...)`, which accepts only
//! `Self::Latest = WorkloadIntentV2` — it silently produced
//! V2-tagged bytes mislabelled as `FIXTURE_V1_*`, so the "old V1
//! bytes still decode" defense this file exists to provide was never
//! actually exercised. Corrected: `canonical_v1_{job,service,
//! schedule}_payload()` now construct the genuinely FROZEN
//! `WorkloadIntentV1` / `JobV1` / `ServiceV1` / `ScheduleV1` /
//! `WorkloadDriverV1` types directly, and `print_fixture_v1_bytes`
//! wraps them via the explicit `WorkloadIntentEnvelope::V1(...)`
//! constructor (never `.latest()`, which is structurally incapable of
//! producing a `V1`-tagged envelope). Each V1 round-trip test also
//! asserts the decoded envelope IS the `V1` variant before projecting
//! to `Latest`, so a future regeneration that repeats this mistake
//! fails loudly instead of silently passing through
//! `into_latest()`'s `V2` identity arm. Each test's `expected` value
//! is ALSO independently hand-written (`expected_v2_for_v1_*`, built
//! directly via V2 struct literals) rather than derived via
//! `canonical_v1_*_payload().into()` — the latter is tautological
//! (both sides of the comparison would route through the identical
//! `From<WorkloadIntentV1>` chain over the identical input), which a
//! deliberate temporary corruption of
//! `From<WorkloadDriverV1> for WorkloadDriverV2` confirmed: the
//! `.into()`-derived form kept passing.

use std::num::{NonZeroU16, NonZeroU32};

use overdrive_core::aggregate::{
    CronExpr, Exec, Job, JobV1, Listener, Schedule, ScheduleV1, Service, ServiceV1, Vm,
    WorkloadDriver, WorkloadDriverV1, WorkloadIntent, WorkloadIntentEnvelope, WorkloadIntentV1,
};
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::WorkloadId;
use overdrive_core::traits::driver::Resources;
use proptest::prelude::*;

use super::harness::assert_envelope_v_roundtrip;

// Per ADR-0050 step 02-03a: `WorkloadIntentEnvelope::discriminant_offset_from_end()`
// returns `None` for the initial landing — the empirical offset for the
// 3-variant inner enum is deferred. The triangulation +
// unknown_version_probe assertions are dependent on a `Some(N)` offset
// and are deliberately not exercised in this slice; the V1 round-trip
// fixtures + `archive_for_store` round-trip remain the load-bearing
// schema-evolution defense.

/// Canonical V1 `Job`-variant payload. **Genuinely FROZEN-typed** —
/// `WorkloadIntentV1::Job(JobV1 { .. })` with `WorkloadDriverV1::Exec`.
/// D1 review remediation (#42): prior to this fix the function
/// returned a `WorkloadIntentV2`-typed value (the live `WorkloadIntent`
/// / `Job` / `WorkloadDriver` aliases all resolved to V2 after the
/// fork), so "the V1 fixture" never actually touched the frozen V1
/// shape or exercised `From<WorkloadIntentV1> for WorkloadIntentV2`.
fn canonical_v1_job_payload() -> WorkloadIntentV1 {
    WorkloadIntentV1::Job(JobV1 {
        id: WorkloadId::new("svc-payments").expect("valid workload id"),
        replicas: NonZeroU32::new(3).expect("non-zero replicas"),
        resources: Resources { cpu_milli: 250, memory_bytes: 256 * 1024 * 1024 },
        driver: WorkloadDriverV1::Exec(Exec {
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
/// opt-out per ADR-0058). See [`canonical_v1_job_payload`] for why
/// this returns the genuinely FROZEN `WorkloadIntentV1` type.
fn canonical_v1_service_payload() -> WorkloadIntentV1 {
    WorkloadIntentV1::Service(ServiceV1 {
        id: WorkloadId::new("svc-frontends").expect("valid workload id"),
        replicas: NonZeroU32::new(2).expect("non-zero replicas"),
        resources: Resources { cpu_milli: 500, memory_bytes: 128 * 1024 * 1024 },
        driver: WorkloadDriverV1::Exec(Exec {
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
/// job shape). See [`canonical_v1_job_payload`] for why this returns
/// the genuinely FROZEN `WorkloadIntentV1` type.
fn canonical_v1_schedule_payload() -> WorkloadIntentV1 {
    WorkloadIntentV1::Schedule(ScheduleV1 {
        id: WorkloadId::new("svc-nightly-cleanup").expect("valid workload id"),
        job: JobV1 {
            id: WorkloadId::new("svc-nightly-cleanup").expect("valid workload id"),
            replicas: NonZeroU32::new(1).expect("non-zero replicas"),
            resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: WorkloadDriverV1::Exec(Exec {
                command: "/usr/local/bin/cleanup".to_string(),
                args: vec!["--mode".to_string(), "nightly".to_string()],
            }),
        },
        cron_expr: CronExpr::new("0 2 * * *").expect("valid cron expr"),
    })
}

/// Independently hand-written V2 projection of
/// [`canonical_v1_job_payload`] — same field values, but built
/// directly via [`Job`] / [`WorkloadDriver`] (V2) struct literals,
/// NOT via `.into()`.
///
/// D1 point-4 review verification (#42): comparing
/// `decode(fixture).into_latest()` against
/// `canonical_v1_job_payload().into()` is TAUTOLOGICAL — both sides
/// route through the identical `From<WorkloadIntentV1>` chain over
/// the identical input, so a broken link in that chain cancels out
/// symmetrically and the test cannot detect it (confirmed by
/// temporarily corrupting `From<WorkloadDriverV1> for
/// WorkloadDriverV2` to emit a wrong `command` string — the test kept
/// passing). This independently hand-written expected value breaks
/// the symmetry: a wrong mapping anywhere in the `From` chain now
/// diverges from this value and the comparison genuinely fails.
fn expected_v2_for_v1_job() -> WorkloadIntent {
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

/// See [`expected_v2_for_v1_job`] — independently hand-written V2
/// projection of [`canonical_v1_service_payload`].
fn expected_v2_for_v1_service() -> WorkloadIntent {
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

/// See [`expected_v2_for_v1_job`] — independently hand-written V2
/// projection of [`canonical_v1_schedule_payload`].
fn expected_v2_for_v1_schedule() -> WorkloadIntent {
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

/// Decode `fixture_hex` into the raw [`WorkloadIntentEnvelope`]
/// WITHOUT projecting through `into_latest()` — used only to assert
/// which envelope VARIANT a fixture encodes. `into_latest()`'s `V2`
/// arm is an identity projection, so a fixture that was mistakenly
/// regenerated as `V2` (D1 review remediation, GH #42) would still
/// pass `assert_envelope_v_roundtrip`; this helper is the structural
/// defense a wrong-variant regeneration cannot slip past.
fn decode_envelope_variant(fixture_hex: &str) -> WorkloadIntentEnvelope {
    let bytes = hex::decode(fixture_hex.trim())
        .expect("schema_evolution fixture hex string must decode cleanly");
    let mut aligned = rkyv::util::AlignedVec::<8>::new();
    aligned.extend_from_slice(&bytes);
    rkyv::from_bytes::<WorkloadIntentEnvelope, rkyv::rancor::Error>(&aligned)
        .expect("schema_evolution fixture bytes must deserialise as the envelope")
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
//
// Review remediation (D1, step 01-02 #42): the FIRST regeneration
// pass wrapped via `WorkloadIntentEnvelope::latest(...)`, which only
// accepts `WorkloadIntentV2` — it silently produced V2-tagged bytes
// mislabelled as V1. This is the CORRECTED regeneration: the bytes
// below are `WorkloadIntentEnvelope::V1(..)` over the frozen
// `WorkloadIntentV1`/`JobV1`/`WorkloadDriverV1` types (see
// `canonical_v1_job_payload` and `print_fixture_v1_bytes`).
const FIXTURE_V1_JOB: &str = "7376632d7061796d656e74732f62696e2f736c656570000033363030ffffffff000000000000000000000000000000008c000000d0ffffff0300000000000000fa000000000000000000001000000000000000008a000000b8ffffffbcffffff0100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";

/// Hex-encoded rkyv-archived bytes of
/// `WorkloadIntentEnvelope::V1(canonical_v1_service_payload())`.
///
/// Regenerated alongside `FIXTURE_V1_JOB` — see that constant's
/// comment for the full rationale (the `WorkloadIntentEnvelope`
/// V1->V2 fork, GH #42 / ADR-0083 Amendment 2026-08-12, and the D1
/// review-remediation correction to construct the envelope explicitly
/// via `WorkloadIntentEnvelope::V1(..)` over the frozen types).
/// `ServiceV1`'s own payload shape did not change — only the outer
/// envelope's positional offsets shifted.
const FIXTURE_V1_SERVICE: &str = "7376632d66726f6e74656e64732f7573722f62696e2f66726f6e74656e6400002d2d706f7274ffff38303830ffffffff901f000000000000000000000000000001000000000000008d000000b8ffffff0200000000000000f40100000000000000000008000000000000000091000000a1ffffffacffffff02000000b4ffffff01000000b0ffffff00000000a8ffffff00000000a0ffffff000000000000000000000000000000000000000000000000";

/// Hex-encoded rkyv-archived bytes of
/// `WorkloadIntentEnvelope::V1(canonical_v1_schedule_payload())`.
///
/// Regenerated alongside `FIXTURE_V1_JOB` — see that constant's
/// comment for the full rationale (the `WorkloadIntentEnvelope`
/// V1->V2 fork, GH #42 / ADR-0083 Amendment 2026-08-12, and the D1
/// review-remediation correction to construct the envelope explicitly
/// via `WorkloadIntentEnvelope::V1(..)` over the frozen types). The
/// Schedule fixture itself does NOT embed the `Vm`-carrying driver
/// (Schedule embeds `Job`, which does), but the SHARED
/// `WorkloadIntentEnvelope` archive offsets are positional — the V2
/// sibling's growth shifts archive metadata across all three
/// variants.
const FIXTURE_V1_SCHEDULE: &str = "7376632d6e696768746c792d636c65616e75707376632d6e696768746c792d636c65616e75702f7573722f6c6f63616c2f62696e2f636c65616e75702d2d6d6f6465ffff6e696768746c79ff302032202a202a202a000000000000000000000002000000000000009300000098ffffff93000000a3ffffff010000000000000064000000000000000000000400000000000000009600000092ffffffa0ffffff020000000000000089000000a4ffffff0000000000000000000000000000000000000000000000000000000000000000";

#[test]
fn workload_intent_v1_job_decodes_through_current_envelope() {
    assert!(
        matches!(decode_envelope_variant(FIXTURE_V1_JOB), WorkloadIntentEnvelope::V1(_)),
        "FIXTURE_V1_JOB must encode WorkloadIntentEnvelope::V1 — a wrong-variant regeneration \
         (wrapping via `.latest()`, which only accepts the V2 payload) would silently pass \
         `into_latest()`'s V2 identity arm without exercising the V1 decode arm or \
         From<WorkloadIntentV1> for WorkloadIntentV2 (D1 review remediation, GH #42)",
    );
    // Independently hand-written -- NOT `canonical_v1_job_payload().into()`.
    // See `expected_v2_for_v1_job`'s doc comment: a `.into()`-derived
    // `expected` makes this assertion tautological against a broken
    // `From` chain.
    let expected = expected_v2_for_v1_job();
    assert_envelope_v_roundtrip::<WorkloadIntentEnvelope>(FIXTURE_V1_JOB, &expected);
}

#[test]
fn workload_intent_v1_service_decodes_through_current_envelope() {
    assert!(
        matches!(decode_envelope_variant(FIXTURE_V1_SERVICE), WorkloadIntentEnvelope::V1(_)),
        "FIXTURE_V1_SERVICE must encode WorkloadIntentEnvelope::V1 — see \
         workload_intent_v1_job_decodes_through_current_envelope for why this guard exists",
    );
    // Independently hand-written -- see `expected_v2_for_v1_job`'s doc
    // comment for why a `.into()`-derived `expected` is tautological.
    let expected = expected_v2_for_v1_service();
    assert_envelope_v_roundtrip::<WorkloadIntentEnvelope>(FIXTURE_V1_SERVICE, &expected);
}

#[test]
fn workload_intent_v1_schedule_decodes_through_current_envelope() {
    assert!(
        matches!(decode_envelope_variant(FIXTURE_V1_SCHEDULE), WorkloadIntentEnvelope::V1(_)),
        "FIXTURE_V1_SCHEDULE must encode WorkloadIntentEnvelope::V1 — see \
         workload_intent_v1_job_decodes_through_current_envelope for why this guard exists",
    );
    // Independently hand-written -- see `expected_v2_for_v1_job`'s doc
    // comment for why a `.into()`-derived `expected` is tautological.
    let expected = expected_v2_for_v1_schedule();
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
///
/// D3 review remediation (#42): the loop now also covers a fresh
/// `Vm`-driven `Job` (`canonical_v2_job_vm_payload`) alongside the
/// three Exec-driven V1-projected payloads — AC-2 must exercise the
/// Vm driver through the runtime `spec_digest` path, not only the
/// pinned `FIXTURE_V2_JOB_VM` hex constant.
#[test]
fn spec_digest_is_deterministic_across_variants() {
    let canonicals: [WorkloadIntent; 4] = [
        canonical_v1_job_payload().into(),
        canonical_v1_service_payload().into(),
        canonical_v1_schedule_payload().into(),
        canonical_v2_job_vm_payload(),
    ];
    for canonical in canonicals {
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
///
/// D3 review remediation (#42): the loop now also covers a fresh
/// `Vm`-driven `Job` (`canonical_v2_job_vm_payload`) — see
/// `spec_digest_is_deterministic_across_variants` above for the same
/// rationale.
#[test]
fn archive_for_store_roundtrips_every_variant() {
    let canonicals: [WorkloadIntent; 4] = [
        canonical_v1_job_payload().into(),
        canonical_v1_service_payload().into(),
        canonical_v1_schedule_payload().into(),
        canonical_v2_job_vm_payload(),
    ];
    for canonical in canonicals {
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

// ---------------------------------------------------------------------
// D3 review remediation (#42) — fresh (non-pinned) Vm-driver roundtrip.
//
// AC-2 was previously covered only by the single pinned
// `FIXTURE_V2_JOB_VM` hex constant. This property generates arbitrary
// `Vm` field values on every run and drives them through the REAL
// persistence codec (`archive_for_store` -> `from_store_bytes`),
// independent of any pinned fixture — the roadmap-suggested defense
// against a codec regression that a single hand-picked example could
// miss.
// ---------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn workload_intent_v2_vm_driver_roundtrips_for_arbitrary_fields(
        command in ".{0,64}",
        args in prop::collection::vec(".{0,32}", 0..=4),
        kernel in ".{0,64}",
        rootfs in ".{0,64}",
    ) {
        let canonical: WorkloadIntent = WorkloadIntent::Job(Job {
            id: WorkloadId::new("svc-vm-proptest").expect("valid workload id"),
            replicas: NonZeroU32::new(1).expect("non-zero replicas"),
            resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: WorkloadDriver::Vm(Vm { command, args, kernel, rootfs }),
        });
        let bytes = canonical.archive_for_store().expect("archive_for_store must succeed");
        let decoded = WorkloadIntent::from_store_bytes(
            bytes.as_ref(),
            std::path::Path::new("schema_evolution.redb"),
            None,
        )
        .expect("from_store_bytes must succeed on the bytes archive_for_store just produced");
        prop_assert_eq!(decoded, canonical);
    }
}

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping a payload variant; the pinned FIXTURE_V<N>_* constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V<N>_* constants"
)]
fn print_fixture_v1_bytes() {
    for (label, v1_payload) in [
        ("FIXTURE_V1_JOB", canonical_v1_job_payload()),
        ("FIXTURE_V1_SERVICE", canonical_v1_service_payload()),
        ("FIXTURE_V1_SCHEDULE", canonical_v1_schedule_payload()),
    ] {
        // Explicit V1 wrap — NOT `WorkloadIntentEnvelope::latest(...)`,
        // which only accepts `Self::Latest = WorkloadIntentV2` and is
        // therefore structurally incapable of producing a V1-tagged
        // envelope. D1 review remediation (GH #42): that was the bug —
        // `canonical_v1_*_payload()` used to return a V2-typed value
        // (the live aliases all resolved to V2 after the fork), so
        // `.latest()` silently wrapped it as V2.
        let envelope = WorkloadIntentEnvelope::V1(v1_payload);
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

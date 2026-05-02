//! Round-trip acceptance scenarios for `wire-exec-spec-end-to-end` —
//! `JobSpecInput` ↔ `Job::from_spec` ↔ `From<&Job>` is identity for
//! every valid input carrying the new exec block.
//!
//! Covers `docs/feature/wire-exec-spec-end-to-end/distill/test-scenarios.md`
//! §4 *Round-trip*.
//!
//! The rkyv byte-equality property for the extended `Job` shape
//! (command + args fields contributing to the canonical archive) is
//! pinned by the existing `aggregate_roundtrip::job_rkyv_byte_identical_on_repeated_archival`
//! test once its sample fixture is migrated to the new shape (see
//! DWD-9 in `wave-decisions.md` — the fixture migration lands in the
//! same DELIVER step).

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use overdrive_core::aggregate::{DriverInput, ExecInput, Job, JobSpecInput, ResourcesInput};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Single-case roundtrip — pin the projection identity for the
// canonical-shape happy path.
// ---------------------------------------------------------------------------

#[test]
fn jobspec_input_roundtrips_through_aggregate_with_exec_block() {
    let original = JobSpecInput {
        id: "payments".to_string(),
        replicas: 3,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/opt/x/y".to_string(),
            args: vec!["--port".to_string(), "8080".to_string()],
        }),
    };

    let job = Job::from_spec(original.clone()).expect("canonical spec is valid");
    let back = JobSpecInput::from(&job);

    assert_eq!(back, original, "JobSpecInput → Job → JobSpecInput must be identity");
}

#[test]
fn jobspec_input_roundtrips_with_empty_args_vec() {
    // Companion to the above — exercise the zero-args case (binary
    // takes no argv) per ADR-0031 §4. The empty Vec must survive the
    // round-trip identically.
    let original = JobSpecInput {
        id: "tick".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    };

    let job = Job::from_spec(original.clone()).expect("zero-args spec is valid");
    let back = JobSpecInput::from(&job);

    assert_eq!(back, original);
    assert!(back.matches_empty_args());
}

trait MatchesEmptyArgs {
    fn matches_empty_args(&self) -> bool;
}

impl MatchesEmptyArgs for JobSpecInput {
    fn matches_empty_args(&self) -> bool {
        match &self.driver {
            DriverInput::Exec(exec) => exec.args.is_empty(),
        }
    }
}

// ---------------------------------------------------------------------------
// Property — every valid JobSpecInput round-trips identity.
// ---------------------------------------------------------------------------

const ALPHA: &str = "abcdefghijklmnopqrstuvwxyz";
const ALNUM_DASH: &str = "abcdefghijklmnopqrstuvwxyz0123456789-";
const ALNUM: &str = "abcdefghijklmnopqrstuvwxyz0123456789";

/// Valid label matching the `JobId` newtype's `^[a-z][a-z0-9-]{0,62}$`.
fn valid_label() -> impl Strategy<Value = String> {
    prop_oneof![
        proptest::sample::select(ALPHA.chars().collect::<Vec<_>>()).prop_map(|c| c.to_string()),
        (
            proptest::sample::select(ALPHA.chars().collect::<Vec<_>>()),
            prop::collection::vec(
                proptest::sample::select(ALNUM_DASH.chars().collect::<Vec<_>>()),
                0..=60,
            ),
            proptest::sample::select(ALNUM.chars().collect::<Vec<_>>()),
        )
            .prop_map(|(first, interior, last)| {
                let mut s = String::with_capacity(2 + interior.len());
                s.push(first);
                s.extend(interior);
                s.push(last);
                s
            }),
    ]
}

/// Non-empty command — any non-empty ASCII printable string. The
/// validation rule is "non-empty after trim"; this generator stays
/// strictly above that boundary by including at least one
/// non-whitespace char.
fn non_empty_command() -> impl Strategy<Value = String> {
    "[A-Za-z0-9/_.-]{1,64}"
        .prop_filter("command must contain at least one non-whitespace char", |s| {
            !s.trim().is_empty()
        })
}

/// Generator for the args vector — any vector of any opaque strings,
/// including empty and whitespace-only elements (per ADR-0031 §4 args
/// are opaque to the platform).
fn args_vec() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(any::<String>().prop_map(|s| s.chars().take(32).collect()), 0..=8)
}

fn arb_jobspec_input() -> impl Strategy<Value = JobSpecInput> {
    (
        valid_label(),
        1u32..=1024,
        0u32..=64_000,
        1u64..=(128 * 1024 * 1024 * 1024),
        non_empty_command(),
        args_vec(),
    )
        .prop_map(|(id, replicas, cpu, mem, command, args)| JobSpecInput {
            id,
            replicas,
            resources: ResourcesInput { cpu_milli: cpu, memory_bytes: mem },
            driver: DriverInput::Exec(ExecInput { command, args }),
        })
}

proptest! {
    /// For any valid `JobSpecInput`, `from_spec` succeeds AND the
    /// round-trip back via `From<&Job>` equals the original. Closes the
    /// "newtype roundtrip" mandatory call site for the aggregate input
    /// shape.
    #[test]
    fn jobspec_input_roundtrip_property_with_exec_block(
        original in arb_jobspec_input(),
    ) {
        let job = Job::from_spec(original.clone())
            .expect("generator yields valid JobSpecInput");
        let back = JobSpecInput::from(&job);
        prop_assert_eq!(back, original);
    }
}

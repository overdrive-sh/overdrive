//! Slice 05 — parser-side cron-required scenario.
//!
//! Per `docs/feature/workload-kind-discriminator/distill/test-scenarios.md`
//! §5 / S-05-04: a `[job]+[schedule]` TOML body whose `[schedule]`
//! block omits the `cron` field is rejected by the parser with a
//! structured `ParseError::MissingCron`. The error's `Display` form
//! names `cron` and `[schedule]` so the operator can correct the spec
//! without consulting docs.

use overdrive_core::aggregate::{ParseError, WorkloadSpecInput};

/// S-05-04: missing `cron` field in `[schedule]` is rejected.
///
/// Empty `cron = ""` and missing `cron` line both surface the same
/// `MissingCron` variant — they are observationally indistinguishable
/// once the file reaches the parser, which is the contract the slice
/// 05 spec carries (cron is "non-empty after trim").
#[test]
fn schedule_05_04_schedule_requires_non_empty_cron_field() {
    // Case A: `cron` field absent entirely.
    let absent = r#"
[job]
id = "nightly-backup"

[exec]
command = "/bin/echo"
args = []

[resources]
cpu_milli = 100
memory_bytes = 67108864

[schedule]
"#;
    let err = WorkloadSpecInput::from_toml_str(absent).expect_err("missing cron must be rejected");
    assert!(
        matches!(err, ParseError::MissingCron),
        "S-05-04: missing cron must surface MissingCron; got {err:?}",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("cron") && msg.contains("[schedule]"),
        "S-05-04: error must name `cron` AND `[schedule]`; got: {msg}",
    );

    // Case B: `cron = ""` (empty after trim) — same diagnosis.
    let empty = r#"
[job]
id = "nightly-backup"

[exec]
command = "/bin/echo"
args = []

[resources]
cpu_milli = 100
memory_bytes = 67108864

[schedule]
cron = "   "
"#;
    let err = WorkloadSpecInput::from_toml_str(empty).expect_err("blank cron must be rejected");
    assert!(
        matches!(err, ParseError::MissingCron),
        "S-05-04: empty cron must surface MissingCron; got {err:?}",
    );
}

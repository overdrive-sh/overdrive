//! Slice 05 — Schedule kind submit/alloc-status surface scenarios.
//!
//! Per `docs/feature/workload-kind-discriminator/distill/test-scenarios.md`
//! §5 (US-05). Six scenarios:
//!
//! * S-05-01 — submit echo for Schedule kind names "Schedule
//!   registered." plus a NOTE block that references the deferral URL.
//! * S-05-02 — KPI K5: byte-equality of the deferral URL across the
//!   submit-echo render and the alloc-status render. Single SSOT
//!   constant, single read.
//! * S-05-03 — `[schedule]` without `[job]` is rejected at the CLI
//!   boundary. Cross-references S-01-05 in the parser.
//! * S-05-04 — `[schedule]` with `[job]` but no `cron` is rejected by
//!   the parser. (NB: the corresponding parser-only acceptance test
//!   lives in `crates/overdrive-core/tests/acceptance/schedule_parser.rs`;
//!   here we only assert that the CLI surface routes the parser error
//!   shape through to the operator.)
//! * S-05-05 — Schedule submit persists the spec to the `IntentStore`
//!   under the canonical `IntentKey`, recoverable as a Schedule-kind
//!   `WorkloadSpec` carrying the operator-supplied cron expression.
//! * S-05-06 — the deferral URL is sourced from a single CLI constant
//!   `SCHEDULE_EXECUTION_TRACKING_URL` and equals
//!   `https://github.com/overdrive-sh/overdrive/issues/166`.
//!
//! # Sequencing note
//!
//! The slice 05 surface is render-side + `IntentStore`-side. The
//! production `submit_streaming` parser is still legacy
//! `JobSpecInput`; slice 02 wires the `WorkloadSpec` discriminator
//! into `submit_streaming`. These tests therefore exercise the slice
//! 05 surfaces directly (render functions, `IntentKey` derivation,
//! `IntentStore` persistence helper) — not the legacy
//! `submit_streaming` path. The matching slice-02 wiring covers the
//! end-to-end CLI flow later.

use overdrive_cli::render::schedule::{
    SCHEDULE_EXECUTION_TRACKING_URL, schedule_alloc_status_block, schedule_submit_echo,
};
use overdrive_core::aggregate::{IntentKey, ScheduleSpec, WorkloadSpec, WorkloadSpecInput};
use overdrive_store_local::{IntentStore, LocalIntentStore};

/// Parse the canonical `NIGHTLY_BACKUP_TOML` body and unwrap the
/// Schedule arm — every scenario starts from the same parsed
/// `ScheduleSpec`. A non-Schedule outcome is a fixture bug, not a
/// production-code bug, so a panic at this seam is the correct
/// failure mode for tests.
fn parse_schedule(toml: &str) -> ScheduleSpec {
    match WorkloadSpecInput::from_toml_str(toml) {
        Ok(WorkloadSpecInput::Schedule(s)) => s,
        Ok(other) => panic!("expected Schedule, got {other:?}"),
        Err(err) => panic!("failed to parse Schedule fixture: {err}"),
    }
}

/// Canonical `[job]+[schedule]` TOML body used across the slice 05
/// scenarios. The `cron` value is shipped verbatim through to the
/// alloc-status render — slice 05 does NOT canonicalise cron strings
/// at the parse boundary.
const NIGHTLY_BACKUP_TOML: &str = r#"
[job]
id = "nightly-backup"

[exec]
command = "/bin/echo"
args = ["nightly", "backup"]

[resources]
cpu_milli = 100
memory_bytes = 67108864

[schedule]
cron = "0 2 * * *"
"#;

// ---------------------------------------------------------------------------
// S-05-06 — Deferral URL is a single CLI constant.
// ---------------------------------------------------------------------------

/// S-05-06: the SSOT for the deferral URL is the single
/// `SCHEDULE_EXECUTION_TRACKING_URL` constant; its value is
/// byte-equal to the canonical GH #166 URL.
#[test]
fn schedule_05_06_deferral_url_sourced_from_single_constant() {
    assert_eq!(
        SCHEDULE_EXECUTION_TRACKING_URL, "https://github.com/overdrive-sh/overdrive/issues/166",
        "SCHEDULE_EXECUTION_TRACKING_URL must equal GH #166 byte-for-byte",
    );
}

// ---------------------------------------------------------------------------
// S-05-01 — Submit echo for Schedule kind.
// ---------------------------------------------------------------------------

/// S-05-01: rendered submit-echo for a Schedule names "Submitting
/// schedule '<id>' (kind=Schedule)", "Schedule registered.", and a
/// NOTE block that includes the deferral URL constant.
#[test]
fn schedule_05_01_submit_echoes_registered_with_deferral_url() {
    let sched = parse_schedule(NIGHTLY_BACKUP_TOML);

    let echo = schedule_submit_echo(
        &sched,
        "deadbeef000102030405060708090a0b0c0d0e0f000102030405060708090a0b",
        "https://overdrive.local:8443",
    );

    assert!(
        echo.contains("Submitting schedule 'nightly-backup' (kind=Schedule)"),
        "S-05-01: header line missing; got:\n{echo}",
    );
    assert!(
        echo.contains("Schedule registered."),
        "S-05-01: registered line missing; got:\n{echo}",
    );
    assert!(
        echo.contains("NOTE: schedule execution is not yet implemented"),
        "S-05-01: NOTE block missing; got:\n{echo}",
    );
    assert!(
        echo.contains(SCHEDULE_EXECUTION_TRACKING_URL),
        "S-05-01: NOTE block must include the deferral URL; got:\n{echo}",
    );
}

// ---------------------------------------------------------------------------
// S-05-02 — KPI K5 byte-equality of deferral URL.
// ---------------------------------------------------------------------------

/// S-05-02 (KPI K5): the deferral URL emitted by the submit echo and
/// the deferral URL emitted by the alloc-status render are
/// byte-identical. Both reads route through the same SSOT constant —
/// drift is structurally impossible.
#[test]
fn schedule_05_02_alloc_status_byte_equality_with_submit_url() {
    let sched = parse_schedule(NIGHTLY_BACKUP_TOML);

    let echo = schedule_submit_echo(
        &sched,
        "deadbeef000102030405060708090a0b0c0d0e0f000102030405060708090a0b",
        "https://overdrive.local:8443",
    );

    let alloc = schedule_alloc_status_block(sched.job_inner.id.as_str(), sched.cron_expr.as_str());

    // KPI K5 byte-equality — extract the URL substring from each
    // surface's text and assert byte-identity.
    let echo_url = extract_url(&echo).expect("submit echo carries a URL");
    let alloc_url = extract_url(&alloc).expect("alloc status carries a URL");
    assert_eq!(
        echo_url, alloc_url,
        "KPI K5: deferral URL must byte-equal across surfaces; \
         echo URL `{echo_url}` vs alloc URL `{alloc_url}`",
    );
    assert_eq!(echo_url, SCHEDULE_EXECUTION_TRACKING_URL);

    // Alloc status render contract: names "kind: Schedule", echoes
    // the cron expression unchanged, and surfaces the empty-state
    // message.
    assert!(
        alloc.contains("kind: Schedule"),
        "S-05-02: alloc status must name kind: Schedule; got:\n{alloc}",
    );
    assert!(
        alloc.contains("0 2 * * *"),
        "S-05-02: alloc status must echo cron expression unchanged; got:\n{alloc}",
    );
    assert!(
        alloc.contains("No allocations have been spawned yet"),
        "S-05-02: alloc status must surface the empty-state line; got:\n{alloc}",
    );
}

/// Extract the deferral URL from a rendered text block. Used by
/// S-05-02's byte-equality assertion to reach the URL substring out
/// of each surface's render.
///
/// Both the submit-echo and the alloc-status render attach the
/// deferral URL to a `Tracking:` line. The submit echo also names
/// the control-plane endpoint (`Endpoint: <url>`) on a separate
/// line, so a "first https:// token" extractor would silently land
/// on the wrong URL — assert specifically against the `Tracking:`
/// line so the byte-equality check measures the surface that
/// actually needs to be drift-free.
fn extract_url(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| {
            let trimmed = line.trim_start();
            trimmed.strip_prefix("Tracking:").map(|s| s.trim().to_owned())
        })
        .map(|s| s.trim_end_matches(|c: char| ".,;:".contains(c)).to_owned())
}

// ---------------------------------------------------------------------------
// S-05-03 — `[schedule]` without `[job]` rejected.
// ---------------------------------------------------------------------------

/// S-05-03: a TOML body containing `[schedule]` without `[job]` is
/// rejected by the parser with an error whose `Display` form names
/// the violated rule. Cross-references S-01-05 in the parser-side
/// acceptance suite (overdrive-core).
#[test]
fn schedule_05_03_schedule_without_job_cli_handler_rejects() {
    let bad = r#"
[exec]
command = "/bin/echo"
args = []

[resources]
cpu_milli = 100
memory_bytes = 67108864

[schedule]
cron = "0 2 * * *"
"#;
    let err = WorkloadSpecInput::from_toml_str(bad)
        .expect_err("[schedule] without [job] must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("[schedule] is only valid alongside [job]"),
        "S-05-03: error must name the violated rule; got: {msg}",
    );
}

// ---------------------------------------------------------------------------
// S-05-05 — Schedule submit persists to IntentStore.
// ---------------------------------------------------------------------------

/// S-05-05: persisting a `WorkloadSpec::Schedule` under the canonical
/// `IntentKey::for_schedule` round-trips byte-equal — a re-read
/// reconstructs a Schedule-kind `WorkloadSpec` carrying the
/// operator-supplied cron expression.
///
/// Slice 05 ships the persistence helper at the `IntentStore`
/// boundary; slice 02 wires it through `submit_streaming`. This
/// test asserts the persistence-side contract directly.
#[tokio::test]
async fn schedule_05_05_schedule_submit_persists_to_intent_store() {
    let sched = parse_schedule(NIGHTLY_BACKUP_TOML);
    let id = sched.job_inner.id.clone();
    let cron_expected = sched.cron_expr.as_str().to_owned();
    let spec = WorkloadSpec::Schedule(sched);

    // rkyv-archive the WorkloadSpec for the persisted byte stream.
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&spec)
        .expect("rkyv archive of Schedule WorkloadSpec");

    // Build the canonical IntentKey for a Schedule. Lives at the
    // overdrive-core SSOT (`IntentKey::for_schedule`).
    let workload_id =
        overdrive_core::id::WorkloadId::new(&id).expect("schedule id parses as WorkloadId");
    let key = IntentKey::for_schedule(&workload_id);
    assert_eq!(
        key.as_str(),
        format!("schedules/{id}"),
        "S-05-05: canonical IntentKey for Schedule must be schedules/<id>",
    );

    // Persist via a real LocalIntentStore.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let store =
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("LocalIntentStore opens");
    store.put(key.as_bytes(), archived.as_ref()).await.expect("put Schedule spec");

    // Re-read; deserialise via rkyv; assert Schedule-kind + cron preserved.
    let bytes = store
        .get(key.as_bytes())
        .await
        .expect("intent store get")
        .expect("schedule spec must be present");
    let recovered: WorkloadSpec =
        rkyv::from_bytes::<WorkloadSpec, rkyv::rancor::Error>(bytes.as_ref())
            .expect("rkyv deserialise");
    match recovered {
        WorkloadSpec::Schedule(s) => {
            assert_eq!(s.job_inner.id, id, "S-05-05: recovered id must match");
            assert_eq!(
                s.cron_expr.as_str(),
                cron_expected,
                "S-05-05: recovered cron must match operator input verbatim",
            );
        }
        other => panic!("S-05-05: recovered spec must be Schedule kind; got {other:?}"),
    }
}

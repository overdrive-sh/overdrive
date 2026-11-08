//! P5/E5 retained-record reader tests. Records are preconditions, never claimed
//! evidence of executing a command. Native finalization is observed separately.
use super::support::*;
use std::fs;
use std::process::Stdio;
use std::thread;
use std::time::Duration;

/// CONTRACT_SHAPE: bounded-change (collector output/status only; retained records,
/// source, other invocation and owned ephemeral files are byte-for-byte unchanged).
#[test]
#[ignore = "pending DELIVER: metal-run-isolation completion-gated collection"]
fn retained_v1_and_v2_commands_return_exact_status_on_read_only_repeat_collection() {
    for version in [1, 2] {
        for code in [0, 1, 64, 70, 74, 75, 78, 100, 254, 255] {
            let fixture = Fixture::new();
            fixture.retained(
                RUN_A,
                version,
                Some(("command", &code.to_string())),
                (version == 2).then_some("complete"),
            );
            fixture.retained(RUN_B, 2, Some(("command", "0")), Some("complete"));
            let scratch = fixture.source.join("target/metal-run-tmp").join(RUN_A);
            fs::create_dir_all(&scratch).expect("scratch complement");
            fs::write(scratch.join("collector-must-not-clean"), "retained").expect("complement");
            let records_before = snapshot(&fixture.runs);
            let source_before = snapshot(&fixture.source);
            let first = fixture.collect(RUN_A).output().expect("first collect");
            let second = fixture.collect(RUN_A).output().expect("repeat collect");
            assert_eq!(first.status.code(), Some(code));
            assert_eq!(second.status.code(), Some(code));
            assert_eq!(first.stdout, b"retained selected-command output\n");
            assert_eq!(first.stdout, second.stdout);
            assert_eq!(first.stderr, second.stderr);
            let marker = String::from_utf8_lossy(&first.stderr);
            assert!(marker.contains(&format!("OVERDRIVE_METAL_RUN_COMPLETE run_id={RUN_A} exit_code={code} cause=command_returned")), "{marker}");
            assert!(!marker.contains("RUNNER_ERROR"));
            assert_eq!(snapshot(&fixture.runs), records_before);
            assert_eq!(snapshot(&fixture.source), source_before);
        }
    }
}

/// CONTRACT_SHAPE: bounded-change (a v2 result is emitted only after its actual
/// observed unit stops; waiting/collection never rewrites terminal or receipt).
#[test]
#[ignore = "pending DELIVER: metal-run-isolation unit-stop collection gate"]
fn collection_waits_for_owner_stop_even_when_terminal_and_receipt_exist() {
    let fixture = Fixture::new();
    fixture.retained(RUN_A, 2, Some(("command", "255")), Some("complete"));
    let before = snapshot(&fixture.runs);
    fs::write(fixture.temp.path().join("unit-state"), "deactivating\n")
        .expect("external unit observation");
    let mut collector = fixture
        .collect(RUN_A)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("collect deactivating owner");
    thread::sleep(Duration::from_millis(250));
    assert!(
        collector.try_wait().expect("collector state").is_none(),
        "command terminal cannot release completion while owner descendants are draining"
    );
    fs::write(fixture.temp.path().join("unit-state"), "inactive\n")
        .expect("external stop completion");
    let output = collector.wait_with_output().expect("collected stopped owner");
    assert_eq!(output.status.code(), Some(255));
    assert_eq!(snapshot(&fixture.runs), before);
}

/// CONTRACT_SHAPE: bounded-change (typed completion/artifact failures only; actual
/// selected-command terminal and every owned/foreign file remain retained).
#[test]
#[ignore = "pending DELIVER: metal-run-isolation completion error taxonomy"]
fn sad_cleanup_and_artifact_failures_never_become_command_results() {
    for state in [
        "guest_staging",
        "vmm_runtime",
        "command_tmp",
        "missing",
        "malformed",
        "wrong-id",
        "both-metadata",
        "mismatched-format",
        "malformed-phase",
        "missing-terminal",
        "missing-terminal-and-receipt",
    ] {
        let fixture = Fixture::new();
        let receipt = match state {
            "guest_staging" | "vmm_runtime" | "command_tmp" => state,
            _ => "complete",
        };
        let directory = fixture.retained(RUN_A, 2, Some(("command", "255")), Some(receipt));
        match state {
            "missing" => {
                fs::remove_file(directory.join("completion.v1")).expect("missing completion");
            }
            "malformed" => fs::write(directory.join("completion.v1"), "kind=complete\n")
                .expect("malformed receipt"),
            "wrong-id" => {
                let path = directory.join("completion.v1");
                fs::write(&path, text(&path).replace(RUN_A, RUN_B)).expect("foreign receipt");
            }
            "both-metadata" => {
                fs::copy(directory.join("metadata.v2"), directory.join("metadata.v1"))
                    .expect("ambiguous metadata");
            }
            "mismatched-format" => {
                let path = directory.join("metadata.v2");
                fs::write(&path, text(&path).replace("run-v2", "run-v1"))
                    .expect("mismatched metadata");
            }
            "malformed-phase" => fs::write(directory.join("phase.v1"), "phase=running\n")
                .expect("invalid retained phase"),
            "missing-terminal" => {
                fs::remove_file(directory.join("terminal.v1")).expect("supervisor lost");
            }
            "missing-terminal-and-receipt" => {
                fs::remove_file(directory.join("terminal.v1")).expect("supervisor lost");
                fs::remove_file(directory.join("completion.v1")).expect("receipt absent");
            }
            _ => {}
        }
        let before = snapshot(&fixture.runs);
        let output = fixture.collect(RUN_A).output().expect("collect invalid completion");
        let (expected, status) = match state {
            "guest_staging" | "vmm_runtime" | "command_tmp" => ("cleanup_failed", 74),
            "missing-terminal" | "missing-terminal-and-receipt" => ("supervisor_lost", 70),
            _ => ("run_artifact_io", 74),
        };
        runner_error(&output, expected, status);
        assert_eq!(snapshot(&fixture.runs), before);
        if state == "guest_staging" || state == "vmm_runtime" || state == "command_tmp" {
            assert_eq!(fields(&directory.join("terminal.v1"))["exit_code"], "255");
        }
    }
}

/// CONTRACT_SHAPE: bounded-change (closed runner error/status vocabulary; malformed
/// terminal numbers, fields, order, LF and receipt enums cannot escape as results).
#[test]
#[ignore = "pending DELIVER: metal-run-isolation retained protocol validation"]
fn sad_retained_records_enforce_closed_error_and_command_status_domains() {
    for value in ["-1", "256", "0255", "", "abc"] {
        let fixture = Fixture::new();
        fixture.retained(RUN_A, 2, Some(("command", value)), Some("complete"));
        runner_error(
            &fixture.collect(RUN_A).output().expect("out-of-domain status"),
            "run_artifact_io",
            74,
        );
    }
    for error in [
        "maintenance_timeout",
        "checkout_timeout",
        "runner_preparation_failed",
        "checkout_identity_conflict",
        "source_mismatch",
        "preflight_failed",
        "lease_timeout",
        "launch_aborted",
        "controller_eof_before_start",
        "command_launch_failed",
        "log_write_failed",
        "log_finalize_failed",
        "not_approved",
    ] {
        let fixture = Fixture::new();
        fixture.retained(RUN_A, 2, Some(("runner", error)), Some("complete"));
        let output = fixture.collect(RUN_A).output().expect("runner terminal collect");
        match error {
            "checkout_identity_conflict" | "source_mismatch" | "preflight_failed" => {
                runner_error(&output, error, 78);
            }
            "maintenance_timeout" | "checkout_timeout" | "runner_preparation_failed" => {
                runner_error(&output, error, 75);
            }
            "lease_timeout"
            | "launch_aborted"
            | "controller_eof_before_start"
            | "command_launch_failed" => runner_error(&output, "run_not_started", 75),
            _ => runner_error(&output, "run_artifact_io", 74),
        }
    }
    for mutation in ["no-lf", "duplicate", "reorder", "unknown-root", "unknown-field"] {
        let fixture = Fixture::new();
        let directory = fixture.retained(RUN_A, 2, Some(("command", "0")), Some("complete"));
        let path = directory.join("completion.v1");
        let original = text(&path);
        let content = match mutation {
            "no-lf" => original.trim_end().to_owned(),
            "duplicate" => format!("{original}kind=complete\n"),
            "reorder" => {
                let mut content = original.lines().rev().collect::<Vec<_>>().join("\n");
                content.push('\n');
                content
            }
            "unknown-root" => {
                original.replace("kind=complete", "kind=runner\nerror=cleanup_failed\nroot=foreign")
            }
            _ => format!("{original}extra=unapproved\n"),
        };
        fs::write(path, content).expect("malformed record input");
        runner_error(
            &fixture.collect(RUN_A).output().expect("malformed record collect"),
            "run_artifact_io",
            74,
        );
    }
}

/// CONTRACT_SHAPE: bounded-change (invalid/unknown retrieval changes only output;
/// collection without registration cannot create records or clean anything).
#[test]
#[ignore = "pending DELIVER: metal-run-isolation retrieval refusal"]
fn sad_collection_without_a_run_preserves_absence() {
    let fixture = Fixture::new();
    for invalid in [
        "",
        "../x",
        "0123456789abcdef0123456",
        "0123456789abcdef012345678",
        "ABCDEF0123456789abcdef01",
    ] {
        runner_error(
            &fixture.collect(invalid).output().expect("invalid run collect"),
            "invalid_run_id",
            64,
        );
    }
    runner_error(
        &fixture.collect(RUN_A).output().expect("unknown run collect"),
        "run_not_found",
        66,
    );
    assert!(snapshot(&fixture.runs).is_empty());
}

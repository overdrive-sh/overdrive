//! Actual finalizer entry and owned filesystem effects, with a fresh valid run
//! identity. Real systemd descendant draining remains an independent E2E oracle.
use super::support::*;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct OwnedRoots(Vec<PathBuf>);
impl Drop for OwnedRoots {
    fn drop(&mut self) {
        // Only the exact three unique roots this fixture created; never a glob,
        // shared base, another run, source checkout, module, target or policy.
        for root in &self.0 {
            if root.is_dir() {
                let _ = fs::remove_dir_all(root);
            }
        }
    }
}
fn unique_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
    format!("{:08x}{:016x}", std::process::id(), nanos & u128::from(u64::MAX))
}
fn finalize(fixture: &Fixture, id: &str) -> std::process::Output {
    Command::new("bash")
        .arg(workspace_file("infra/metal/lease-holder.sh"))
        .args(["--finalize", id])
        .arg(&fixture.runs)
        .output()
        .expect("actual finalizer entry")
}

/// CONTRACT_SHAPE: bounded-change (exact invocation-owned staging/runtime/tmp
/// roots and completion receipt; source/target/terminal/foreign-run complement preserved).
#[test]
#[ignore = "pending DELIVER: metal-run-isolation scoped finalization"]
fn finalization_removes_exact_owned_roots_and_repeating_it_preserves_the_complement() {
    let fixture = Fixture::new();
    let id = unique_id();
    let directory = fixture.retained(&id, 2, Some(("command", "255")), None);
    let roots = OwnedRoots(vec![
        PathBuf::from("/srv/vm/metal").join(&id),
        PathBuf::from("/run/ovd-gk").join(&id),
        fixture.source.join("target/metal-run-tmp").join(&id),
    ]);
    for root in &roots.0 {
        assert!(!root.exists(), "fresh test identity cannot adopt an existing root");
        fs::create_dir_all(root).expect("owned finalization input");
        fs::write(root.join("owned-input"), "ephemeral").expect("owned input");
    }
    let foreign_id = unique_id();
    assert_ne!(foreign_id, id, "the complement belongs to a different invocation");
    let foreign_roots = OwnedRoots(vec![
        PathBuf::from("/srv/vm/metal").join(&foreign_id),
        PathBuf::from("/run/ovd-gk").join(&foreign_id),
        fixture.source.join("target/metal-run-tmp").join(&foreign_id),
    ]);
    for root in &foreign_roots.0 {
        assert!(!root.exists(), "fresh foreign fixture cannot adopt an existing root");
        fs::create_dir_all(root).expect("other invocation root complement");
        fs::write(root.join("must-survive"), "other invocation")
            .expect("other invocation file input");
    }
    fs::write(fixture.source.join("target/build-artifact"), "retained artifact")
        .expect("artifact complement");
    let source_before = fs::read(fixture.source.join("source-marker")).expect("source before");
    let terminal_before = fs::read(directory.join("terminal.v1")).expect("terminal before");
    let foreign_before: Vec<_> = foreign_roots.0.iter().map(|root| snapshot(root)).collect();
    for _ in 0..2 {
        success(&finalize(&fixture, &id));
        for root in &roots.0 {
            assert!(
                fs::symlink_metadata(root).is_err(),
                "complete receipt requires actual lstat absence: {}",
                root.display()
            );
        }
        let receipt = fields(&directory.join("completion.v1"));
        assert_eq!(receipt["format"], "overdrive-metal-completion-v1");
        assert_eq!(receipt["run_id"], id);
        assert_eq!(receipt["kind"], "complete");
        assert_eq!(receipt.len(), 4);
        assert_eq!(
            fs::read(directory.join("terminal.v1")).expect("retained terminal"),
            terminal_before
        );
        assert_eq!(fs::read(fixture.source.join("source-marker")).expect("source"), source_before);
        assert_eq!(text(&fixture.source.join("target/build-artifact")), "retained artifact");
        for (root, before) in foreign_roots.0.iter().zip(&foreign_before) {
            assert!(
                fs::symlink_metadata(root).expect("surviving invocation root").is_dir(),
                "finalization cannot replace the other invocation's root: {}",
                root.display()
            );
            assert_eq!(
                &snapshot(root),
                before,
                "finalization cannot change the other invocation's files: {}",
                root.display()
            );
        }
    }
}

struct ImmutableFile(PathBuf);
impl Drop for ImmutableFile {
    fn drop(&mut self) {
        let _ = Command::new("chattr").arg("-i").arg(&self.0).status();
    }
}

/// CONTRACT_SHAPE: bounded-change (actual immutable file in exactly one owned
/// root induces the finalizer's declared cause; terminal and foreign roots persist).
#[test]
#[ignore = "pending DELIVER: metal-run-isolation cleanup failure producers"]
fn sad_each_owned_root_removal_failure_is_produced_and_collected_with_its_exact_root() {
    for (blocked_index, expected_root) in
        [(0, "guest_staging"), (1, "vmm_runtime"), (2, "command_tmp")]
    {
        let fixture = Fixture::new();
        let id = unique_id();
        let directory = fixture.retained(&id, 2, Some(("command", "255")), None);
        let roots = OwnedRoots(vec![
            PathBuf::from("/srv/vm/metal").join(&id),
            PathBuf::from("/run/ovd-gk").join(&id),
            fixture.source.join("target/metal-run-tmp").join(&id),
        ]);
        for root in &roots.0 {
            assert!(!root.exists(), "unique test run cannot adopt a root");
            fs::create_dir_all(root).expect("owned root input");
        }
        let blocker = roots.0[blocked_index].join("undeletable-owned-file");
        fs::write(&blocker, "owned failure input").expect("failure input");
        // This changes one owned file attribute, never a shared mount, host
        // policy, module or sysctl. It is removed by RAII before root cleanup.
        success(
            &Command::new("chattr")
                .arg("+i")
                .arg(&blocker)
                .output()
                .expect("immutable owned file prerequisite"),
        );
        let _immutable = ImmutableFile(blocker.clone());
        let terminal_before =
            fs::read(directory.join("terminal.v1")).expect("retained command input");
        let foreign = fixture.source.join("target/foreign-run-artifact");
        fs::write(&foreign, "other invocation").expect("foreign complement");
        let result = finalize(&fixture, &id);
        fs::write(fixture.temp.path().join(format!("finalizer-{id}.stderr")), &result.stderr)
            .expect("retain first finalizer diagnostics");
        fs::write(
            fixture.temp.path().join(format!("finalizer-{id}.status")),
            format!("{:?}\n", result.status.code()),
        )
        .expect("retain finalizer status");
        let receipt = fields(&directory.join("completion.v1"));
        assert_eq!(receipt["kind"], "runner");
        assert_eq!(receipt["error"], "cleanup_failed");
        assert_eq!(receipt["root"], expected_root);
        assert_eq!(receipt["run_id"], id);
        assert_eq!(receipt.len(), 6);
        assert!(blocker.is_file(), "the reported failing root must actually remain");
        let output = fixture.collect(&id).output().expect("collect actual cleanup failure");
        runner_error(&output, "cleanup_failed", 74);
        assert_eq!(
            fs::read(directory.join("terminal.v1")).expect("actual command result remains"),
            terminal_before
        );
        assert_eq!(text(&foreign), "other invocation");
    }
}

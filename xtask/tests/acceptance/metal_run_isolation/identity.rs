//! P1/E1/E6. Every case drives the copied current bootstrap with real Git,
//! actual local rsync and the external SSH transport fixture.
use super::support::*;
use sha2::{Digest as _, Sha256};
use std::fs;
use std::path::Path;

fn assert_binding(source: &Path, binding: &Path, kind: &str) -> String {
    let record = fields(binding);
    assert_eq!(record.len(), 4);
    assert_eq!(record["format"], "overdrive-metal-checkout-v1");
    assert_eq!(record["kind"], kind);
    let identity = if kind == "worktree" {
        git_text(source, &["rev-parse", "--absolute-git-dir"]).into_bytes()
    } else {
        let mut bytes = fs::canonicalize(source)
            .expect("physical checkout")
            .as_os_str()
            .as_encoded_bytes()
            .to_vec();
        bytes.push(0);
        bytes.extend(git_text(source, &["symbolic-ref", "HEAD"]).bytes());
        bytes
    };
    assert_eq!(record["identity_b64"], b64(&identity));
    let mut hash = Sha256::new();
    hash.update(b"overdrive-metal-checkout-v1\0");
    hash.update(kind.as_bytes());
    hash.update(b"\0");
    hash.update(identity);
    let expected = format!("{:x}", hash.finalize());
    assert_eq!(record["checkout_key"], expected);
    expected
}

/// CONTRACT_SHAPE: bounded-change (each selected checkout's source/binding only;
/// the sibling source, build artifact and retained evidence complement is exact).
#[test]
fn same_branch_worktrees_keep_source_artifacts_and_evidence_disjoint() {
    let fixture = Fixture::new();
    let second = fixture.temp.path().join("second");
    success(&git(
        &fixture.source,
        &["worktree", "add", "--detach", second.to_str().expect("fixture path"), "HEAD"],
    ));
    success(&git(&second, &["symbolic-ref", "HEAD", "refs/heads/metal/isolation"]));
    fs::write(second.join("source-marker"), "second-worktree\n").expect("dirty second source");
    success(&fixture.sync(&fixture.source));
    let first_remote = fixture.checkout();
    fs::create_dir_all(first_remote.join("target")).expect("first artifacts");
    fs::write(first_remote.join("target/owned-artifact"), "first-artifact")
        .expect("first artifact");
    fs::write(first_remote.join("retained-evidence"), "first-result").expect("first evidence");
    let before = snapshot(&first_remote);
    success(&fixture.sync(&second));
    assert_eq!(
        snapshot(&first_remote),
        before,
        "second worktree must preserve first's complete complement"
    );
    let bindings = fixture.bindings();
    assert_eq!(bindings.len(), 2, "same full branch ref cannot merge linked worktrees");
    let second_binding = bindings
        .iter()
        .find(|binding| binding.parent() != Some(first_remote.as_path()))
        .expect("distinct binding");
    assert_eq!(
        text(&second_binding.parent().expect("parent").join("source-marker")),
        "second-worktree\n"
    );
    let first_key = assert_binding(
        &fixture.source,
        &first_remote.join(".overdrive-metal-checkout.v1"),
        "branch",
    );
    let second_key = assert_binding(&second, second_binding, "worktree");
    assert_ne!(first_key, second_key);
    assert_eq!(
        second_binding.parent().expect("parent").file_name().expect("key"),
        second_key.as_str()
    );
}

/// CONTRACT_SHAPE: bounded-change (one worktree binding survives branch changes
/// and detached HEAD; no new remote owner or cold path is created).
#[test]
fn linked_identity_survives_branch_switch_and_detachment() {
    let fixture = Fixture::new();
    let linked = fixture.temp.path().join("linked");
    success(&git(
        &fixture.source,
        &["worktree", "add", "-b", "metal-isolation", linked.to_str().expect("path")],
    ));
    success(&fixture.sync(&linked));
    let remote = fixture.checkout();
    let binding = fs::read(remote.join(".overdrive-metal-checkout.v1")).expect("initial binding");
    assert_binding(&linked, &remote.join(".overdrive-metal-checkout.v1"), "worktree");
    for args in [vec!["checkout", "-b", "metal/other"], vec!["checkout", "--detach", "HEAD"]] {
        success(&git(&linked, &args));
        success(&fixture.sync(&linked));
        assert_eq!(fixture.checkout(), remote);
        assert_eq!(
            fs::read(remote.join(".overdrive-metal-checkout.v1")).expect("binding"),
            binding
        );
    }
}

/// CONTRACT_SHAPE: bounded-change (attached clone fallback is physical-root plus
/// full branch; sanitisation aliases and moved metadata select cold state).
#[test]
fn attached_clone_fallback_and_sanitised_spellings_never_adopt_old_state() {
    let fixture = Fixture::new();
    let legacy = fixture.remote.join("overdrive-branches/metal-isolation");
    fs::create_dir_all(&legacy).expect("legacy cache");
    fs::write(legacy.join("old-artifact"), "keep").expect("legacy contents");
    let old = snapshot(&legacy);
    success(&fixture.sync(&fixture.source));
    let first = fixture.checkout();
    assert_binding(&fixture.source, &first.join(".overdrive-metal-checkout.v1"), "branch");
    success(&git(&fixture.source, &["checkout", "-b", "metal-isolation"]));
    success(&fixture.sync(&fixture.source));
    assert_eq!(
        fixture.bindings().len(),
        2,
        "colliding sanitised spellings need distinct full identities"
    );
    let clone = fixture.temp.path().join("separate-clone");
    let output = std::process::Command::new("git")
        .args(["clone", "--quiet"])
        .arg(&fixture.source)
        .arg(&clone)
        .output()
        .expect("clone");
    success(&output);
    success(&fixture.sync(&clone));
    assert_eq!(fixture.bindings().len(), 3, "separate clone on same branch must be cold");
    assert_eq!(snapshot(&legacy), old, "migration never deletes or adopts legacy cache");
}

/// CONTRACT_SHAPE: bounded-change (invalid local identity/override produces only
/// a typed refusal; remote source/artifact/evidence universe is unchanged).
#[test]
fn sad_invalid_identity_and_override_never_contact_a_mutating_remote() {
    for override_value in ["", "/absolute", "a//b", "a/./b", "a/../b", "a/", "a b", "a;true"] {
        let fixture = Fixture::new();
        let before = snapshot(&fixture.remote);
        let output = fixture
            .bootstrap(&fixture.source, &["--sync-only"])
            .env("OVERDRIVE_METAL_REMOTE_SUBDIR", override_value)
            .output()
            .expect("invalid override");
        // An empty environment value retains the existing 'unset override'
        // meaning; it is the valid default control in this finite partition.
        if override_value.is_empty() {
            success(&output);
        } else {
            runner_error(&output, "remote_subdir_invalid", 64);
            assert_eq!(snapshot(&fixture.remote), before);
        }
    }
    for state in ["detached", "not-git", "bare"] {
        let fixture = Fixture::new();
        match state {
            "detached" => success(&git(&fixture.source, &["checkout", "--detach", "HEAD"])),
            "not-git" => {
                fs::remove_dir_all(fixture.source.join(".git")).expect("missing Git input");
            }
            "bare" => success(&git(&fixture.source, &["config", "core.bare", "true"])),
            _ => unreachable!(),
        }
        let before = snapshot(&fixture.remote);
        runner_error(&fixture.sync(&fixture.source), "checkout_identity_invalid", 64);
        assert_eq!(snapshot(&fixture.remote), before);
    }
}

/// CONTRACT_SHAPE: bounded-change (fresh override binds once; unbound nonempty,
/// malformed/full-owner conflict and symlink aliases cannot change source).
#[test]
fn sad_override_conflicts_preserve_the_full_remote_complement() {
    for state in ["nonempty", "foreign", "malformed", "symlink"] {
        let fixture = Fixture::new();
        let destination = fixture.remote.join("custom/checkout");
        fs::create_dir_all(&destination).expect("override directory");
        if state == "symlink" {
            fs::remove_dir(&destination).expect("replace directory");
            std::os::unix::fs::symlink(&fixture.source, &destination).expect("foreign alias");
        } else if state == "nonempty" {
            fs::write(destination.join("foreign-artifact"), "preserve").expect("foreign input");
        } else {
            let mut command = fixture.bootstrap(&fixture.source, &["--sync-only"]);
            success(
                &command
                    .env("OVERDRIVE_METAL_REMOTE_SUBDIR", "custom/checkout")
                    .output()
                    .expect("fresh override"),
            );
            let path = destination.join(".overdrive-metal-checkout.v1");
            if state == "malformed" {
                fs::write(path, "format=overdrive-metal-checkout-v1\nkind=branch\n")
                    .expect("malformed binding");
            } else {
                let original = text(&path);
                let identity = fields(&path)["identity_b64"].clone();
                fs::write(
                    path,
                    original.replace(
                        &format!("identity_b64={identity}"),
                        &format!("identity_b64={}", b64(b"/foreign\0refs/heads/metal/isolation")),
                    ),
                )
                .expect("different full owner with same hash");
            }
        }
        let before = snapshot(&fixture.remote);
        let output = fixture
            .bootstrap(&fixture.source, &["--sync-only"])
            .env("OVERDRIVE_METAL_REMOTE_SUBDIR", "custom/checkout")
            .output()
            .expect("conflicting override");
        runner_error(
            &output,
            if state == "symlink" { "remote_subdir_invalid" } else { "checkout_identity_conflict" },
            if state == "symlink" { 64 } else { 78 },
        );
        assert_eq!(snapshot(&fixture.remote), before, "refusal cannot change foreign artifacts");
    }
}

/// CONTRACT_SHAPE: bounded-change (pull changes only named local generated paths;
/// dirty local inputs do not rebind or rewrite remote source/build/evidence).
#[test]
fn pull_uses_binding_after_local_source_changes_and_repeated_sync_preserves_target() {
    let fixture = Fixture::new();
    success(&fixture.sync(&fixture.source));
    let remote = fixture.checkout();
    fs::create_dir_all(remote.join("target")).expect("target");
    fs::write(remote.join("target/keep"), "artifact").expect("artifact");
    success(&fixture.sync(&fixture.source));
    assert_eq!(text(&remote.join("target/keep")), "artifact");
    fs::write(remote.join("generated-evidence"), "actual-result\n").expect("remote result");
    fs::write(fixture.source.join("source-marker"), "new-local-edit\n").expect("dirty source");
    let before = snapshot(&remote);
    let output = fixture
        .bootstrap(&fixture.source, &["--pull", "--", "generated-evidence"])
        .output()
        .expect("pull generated evidence");
    success(&output);
    assert_eq!(text(&fixture.source.join("generated-evidence")), "actual-result\n");
    assert_eq!(text(&fixture.source.join("source-marker")), "new-local-edit\n");
    assert_eq!(snapshot(&remote), before);
}

/// CONTRACT_SHAPE: bounded-change (missing transfer prerequisite is a preparation
/// refusal; no host package install or source/retained-run mutation is allowed).
#[test]
fn sad_missing_rsync_is_typed_without_installing_packages() {
    let fixture = Fixture::new();
    success(&fixture.sync(&fixture.source));
    for args in [vec!["--sync-only"], vec!["--pull", "--", "evidence"]] {
        let before = snapshot(&fixture.remote);
        let output = fixture
            .bootstrap(&fixture.source, &args)
            .env("METAL_FIXTURE_MISSING_RSYNC", "1")
            .output()
            .expect("missing remote rsync");
        runner_error(&output, "runner_preparation_failed", 75);
        assert!(String::from_utf8_lossy(&output.stderr).contains("prerequisite=rsync"));
        assert!(!fixture.temp.path().join("forbidden-install").exists());
        assert_eq!(snapshot(&fixture.remote), before);
        assert!(snapshot(&fixture.runs).is_empty(), "sync/pull cannot register a durable run");
    }
}

/// CONTRACT_SHAPE: bounded-change (invalid mode changes parse output only;
/// neither public nor bootstrap entrypoint registers or transfers any source).
#[test]
fn sad_exclusive_is_only_a_run_mode_at_both_existing_entrypoints() {
    let fixture = Fixture::new();
    let output = fixture
        .bootstrap(&fixture.source, &["--sync-only", "--exclusive"])
        .output()
        .expect("invalid bootstrap mode");
    runner_error(&output, "invalid_mode", 64);
    assert!(snapshot(&fixture.remote).is_empty());
    assert!(snapshot(&fixture.runs).is_empty());
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["metal", "sync", "--exclusive"])
        .output()
        .expect("public CLI mode parse");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--exclusive"));
}

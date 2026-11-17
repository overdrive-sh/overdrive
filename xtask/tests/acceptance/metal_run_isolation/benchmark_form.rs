//! The Metal benchmark form: `cargo xtask metal run --benchmark <FAMILY> [--spike <NAME>]`
//! (ADR-0304, DDD-9). Bash is the runner under test; the selected `cargo` is the
//! recording fixture the sibling scenarios use, so nothing here boots a guest or
//! measures anything.
//!
//! Driving ports: `infra/metal/bootstrap.sh --run --benchmark ...` and the `xtask`
//! `metal run` command line. Evidence lane: integration (host-pure).
//! Traces to: US-BENCH-01 AC (c) (exclusive admission, closed command), US-BENCH-04 AC (a), DDD-9.
//! The runner's end-to-end run is not tested here or in nextest: it is a benchmark, evidenced by
//! a retained `--benchmark` run (DDD-9, evidence lane).
use super::native_controller::NativeController;
use super::support::*;
use std::fs;
use std::process::Command;

use overdrive_testing::placement;

placement!(host_pure);

fn selected_argv(controller: &NativeController) -> String {
    text(&controller.body)
}

/// The benchmark form selects the exclusive class and runs exactly the closed
/// `cargo bench` command with an absolute results root under the bound checkout.
/// CONTRACT_SHAPE: bounded-change (the run record and the selected command's argv).
#[test]
fn the_benchmark_form_runs_exactly_the_closed_benchmark_command_under_exclusive_admission() {
    let controller = NativeController::new();
    success(&controller.fixture.sync(&controller.fixture.source));
    let output = controller
        .command_with_args(&[
            "--run",
            "--no-sync",
            "--no-sudo",
            "--benchmark",
            "cgroup-connect-hook",
        ])
        .output()
        .expect("benchmark form");
    success(&output);
    let id = NativeController::run_id(&output);
    let metadata = fields(&controller.fixture.runs.join(&id).join("metadata.v2"));
    assert_eq!(metadata["execution_class"], "exclusive", "a benchmark holds the host exclusively");
    let root = controller.fixture.checkout();
    let expected = format!(
        "selected compiler argv: bench -p overdrive-bench-families --bench benchmarks -- run --family cgroup-connect-hook --results-root {}/benchmarks/results\n",
        root.display()
    );
    assert_eq!(
        selected_argv(&controller),
        expected,
        "exactly the closed benchmark command, nothing appended"
    );
}

/// A spike crate's benchmark runs through its own manifest and writes under its own
/// directory; the maintained results tree is untouched.
/// CONTRACT_SHAPE: bounded-change (the selected command's argv).
#[test]
fn the_spike_form_runs_the_spike_crates_benchmark_and_results_stay_in_its_directory() {
    let controller = NativeController::new();
    let spike = controller.fixture.source.join("spike-scratch/pig-demo");
    fs::create_dir_all(&spike).expect("spike directory");
    fs::write(spike.join("Cargo.toml"), "[package]\nname = \"pig-demo\"\n")
        .expect("spike manifest");
    success(&controller.fixture.sync(&controller.fixture.source));
    let output = controller
        .command_with_args(&[
            "--run",
            "--no-sync",
            "--no-sudo",
            "--benchmark",
            "cgroup-connect-hook",
            "--spike",
            "pig-demo",
        ])
        .output()
        .expect("spike benchmark form");
    success(&output);
    let root = controller.fixture.checkout();
    let expected = format!(
        "selected compiler argv: bench --manifest-path spike-scratch/pig-demo/Cargo.toml --bench benchmarks -- run --family cgroup-connect-hook --results-root {}/spike-scratch/pig-demo/results\n",
        root.display()
    );
    assert_eq!(selected_argv(&controller), expected);
}

/// Drive `bootstrap.sh --run` with each malformed argument list and assert the usage status, the
/// runner-error shape with the pinned `name`, and that the selected command never ran.
fn assert_refused_before_any_command_runs(name: &str, cases: Vec<(&str, Vec<&str>)>) {
    for (label, extra) in cases {
        let controller = NativeController::new();
        success(&controller.fixture.sync(&controller.fixture.source));
        let mut args = vec!["--run", "--no-sync", "--no-sudo"];
        args.extend(extra);
        let output =
            controller.command_with_args(&args).output().expect("refused benchmark request");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.code() == Some(64)
                && stderr.contains(&format!("OVERDRIVE_METAL_RUNNER_ERROR error={name}")),
            "a malformed benchmark request must be refused with the usage status and its pinned name `{name}` (case: {label}); got {:?}: {stderr}",
            output.status.code()
        );
        assert!(!controller.body.exists(), "{label}: no selected command may run");
    }
}

/// A benchmark family that fails `^[a-z][a-z0-9-]{0,62}$` (uppercase, a leading digit, 64
/// characters, empty) is refused as `benchmark_family_invalid` before the selected command runs.
/// CONTRACT_SHAPE: bounded-change (one refusal line per case; no selected command runs).
/// Boundary: 63 characters is accepted, 64 is not. Fault stimulus: each malformed family below.
#[test]
fn a_malformed_benchmark_family_is_refused_as_benchmark_family_invalid() {
    let long_family = "a".repeat(64);
    assert_refused_before_any_command_runs(
        "benchmark_family_invalid",
        vec![
            ("uppercase family", vec!["--benchmark", "Bad_Family"]),
            ("family starting with a digit", vec!["--benchmark", "1family"]),
            ("family over 63 characters", vec!["--benchmark", &long_family]),
            ("empty family", vec!["--benchmark", ""]),
        ],
    );
}

/// A spike name that fails `^[a-z0-9][a-z0-9-]{0,62}$`, or whose `spike-scratch/<name>/Cargo.toml`
/// does not exist after sync, is refused as `benchmark_spike_invalid` before the selected command runs.
/// CONTRACT_SHAPE: bounded-change (one refusal line per case; no selected command runs).
/// Fault stimulus: each malformed spike name, and a spike crate that does not exist.
#[test]
fn a_malformed_or_missing_spike_is_refused_as_benchmark_spike_invalid() {
    assert_refused_before_any_command_runs(
        "benchmark_spike_invalid",
        vec![
            ("spike name escaping the directory", vec!["--benchmark", "fam", "--spike", "../x"]),
            ("spike name with an uppercase letter", vec!["--benchmark", "fam", "--spike", "Pig"]),
            (
                "spike crate that does not exist",
                vec!["--benchmark", "fam", "--spike", "no-such-probe"],
            ),
        ],
    );
}

/// `--spike` without `--benchmark`, and `--benchmark` beside `--exclusive` or a trailing command, are
/// refused as `invalid_mode` before the selected command runs (`xtask`'s own parser rejects the last
/// two first, so only the script is driven here).
/// CONTRACT_SHAPE: bounded-change (one refusal line per case; no selected command runs).
/// Fault stimulus: each conflicting argument list.
#[test]
fn a_benchmark_request_in_conflict_with_another_form_is_refused_as_invalid_mode() {
    assert_refused_before_any_command_runs(
        "invalid_mode",
        vec![
            ("spike without a benchmark", vec!["--spike", "pig-demo"]),
            ("benchmark with exclusive", vec!["--exclusive", "--benchmark", "fam"]),
            (
                "benchmark with a trailing command",
                vec!["--benchmark", "fam", "--", "cargo", "check"],
            ),
        ],
    );
}

/// The `xtask` command line rejects the same combinations itself, naming the conflict,
/// so the refusal never reaches the remote runner.
/// CONTRACT_SHAPE: pure-function (argument list in, parse diagnostic out).
#[test]
fn the_xtask_command_line_names_the_conflict_between_the_benchmark_form_and_other_forms() {
    for (label, args) in [
        ("--exclusive", vec!["metal", "run", "--benchmark", "fam", "--exclusive"]),
        ("a trailing command", vec!["metal", "run", "--benchmark", "fam", "--", "cargo", "check"]),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask")).args(&args).output().expect("xtask");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{label}: refused");
        assert!(
            stderr.contains("cannot be used with"),
            "--benchmark must conflict with {label}: {stderr}"
        );
    }
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["metal", "run", "--spike", "pig-demo"])
        .output()
        .expect("xtask");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("required arguments were not provided"),
        "--spike must conflict with its absence of --benchmark: {stderr}"
    );
}

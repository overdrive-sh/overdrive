//! The smoke variant the benchmark attribute emits (ADR-0297, DDD-2): one replicate
//! per arm, no statistics descriptor held, no timing asserted.
//!
//! Three benchmarks are declared here, each in its own family (a family has exactly one
//! benchmark, DDD-3) and each with `smoke(host_pure)`; the attribute emits `smoke_<name>` for each. A scenario runs that generated test in a child process (the
//! test binary itself, filtered to it) and reads its verdict, so a broken fixture's
//! failure is observed without failing this suite.
//! Driving port: the `overdrive_bench` attribute and `cargo nextest run` (the generated test).
//! Evidence lane: integration (child process of the test binary). Traces: US-BENCH-04 AC (c), (d), K7.
#![allow(
    clippy::print_stdout,
    reason = "the probe bodies announce their replicates on the captured test output so the parent scenario can count them"
)]

use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

use overdrive_bench::{Replicate, ReplicateFailure};
use overdrive_testing_macros::overdrive_bench;

static SEEN: [AtomicU32; 2] = [AtomicU32::new(0), AtomicU32::new(0)];

/// Fails if its body runs more than once for an arm: the generated smoke test passes
/// only when it ran at most one replicate per arm.
#[overdrive_bench(
    family = "smoke-probe-one-per-arm",
    substrate = native,
    arms = ["first", "second"],
    replicates = 9,
    metric(name = "value", unit = count, table = "values"),
    smoke(host_pure),
)]
fn one_per_arm(rep: &mut Replicate) -> Result<(), ReplicateFailure> {
    println!("smoke-probe replicate arm={}", rep.arm());
    let slot = usize::from(rep.arm() == "second");
    if SEEN[slot].fetch_add(1, Ordering::SeqCst) > 0 {
        return Err(ReplicateFailure::measurement(
            "the smoke variant ran more than one replicate for an arm",
        ));
    }
    rep.record("value", 1.0)
}

/// Checks the smoke contract from inside: no statistics descriptor is held, and an
/// undeclared metric is a failure the body can see.
#[overdrive_bench(
    family = "smoke-probe-no-timing",
    substrate = native,
    arms = ["only"],
    replicates = 9,
    bpf_stats,
    metric(name = "hook_ns", unit = ns, table = "hook-time"),
    smoke(host_pure),
)]
fn no_timing(rep: &mut Replicate) -> Result<(), ReplicateFailure> {
    println!("smoke-probe no-timing body ran");
    if rep.bpf_stats().is_some() {
        return Err(ReplicateFailure::measurement(
            "the smoke variant must not hold the statistics descriptor",
        ));
    }
    rep.record_absent("hook_ns", "statistics are not held under the smoke variant")?;
    if rep.record("hook_nz", 1.0).is_ok() {
        return Err(ReplicateFailure::measurement("an undeclared metric was accepted"));
    }
    Ok(())
}

/// Fails its fixture only when the parent scenario asks (a broken dataplane build).
#[overdrive_bench(
    family = "smoke-probe-broken-fixture",
    substrate = native,
    arms = ["only"],
    replicates = 9,
    metric(name = "value", unit = count, table = "values"),
    smoke(host_pure),
)]
fn broken_fixture(rep: &mut Replicate) -> Result<(), ReplicateFailure> {
    if std::env::var_os("OVERDRIVE_BENCH_TEST_BREAK_FIXTURE").is_some() {
        return Err(ReplicateFailure::fixture("the fixture no longer builds a dataplane"));
    }
    rep.record("value", 1.0)
}

fn run_generated(test: &str, break_fixture: bool) -> Output {
    let mut command = Command::new(std::env::current_exe().expect("this test binary"));
    command.args([&format!("smoke_variant::{test}"), "--test-threads", "1", "--nocapture"]);
    if break_fixture {
        command.env("OVERDRIVE_BENCH_TEST_BREAK_FIXTURE", "1");
    }
    command.output().expect("run the generated smoke test")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The panic message of the failed child test: the lines after libtest's
/// `thread '<test path>' panicked at <location>:` line. libtest prints the test path in its own
/// lines, so only the message `overdrive_bench::smoke` produced can satisfy an assertion on this text.
fn panic_message(output: &Output) -> String {
    let text = format!("{}{}", stdout(output), String::from_utf8_lossy(&output.stderr));
    let mut message = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if inside {
            if line.is_empty() || line.starts_with("note:") || line.starts_with("stack backtrace") {
                break;
            }
            message.push(line);
        } else if line.contains("panicked at") {
            inside = true;
        }
    }
    message.join("\n")
}

/// The generated smoke test runs exactly one replicate per arm and passes: the body announces each
/// replicate it runs, and a smoke test that ran none (or two) fails this scenario.
/// CONTRACT_SHAPE: bounded-change (the child's announced replicates).
#[test]
fn the_generated_smoke_test_runs_one_replicate_per_arm_and_passes() {
    let output = run_generated("smoke_one_per_arm", false);
    let text = stdout(&output);
    assert!(
        output.status.success() && text.contains("1 passed"),
        "the generated smoke test must run and pass: {text}"
    );
    for arm in ["first", "second"] {
        assert_eq!(
            text.matches(&format!("smoke-probe replicate arm={arm}")).count(),
            1,
            "the generated smoke test must run exactly one replicate of arm `{arm}`: {text}"
        );
    }
}

/// The smoke variant holds no statistics descriptor and asserts nothing about timing:
/// a benchmark whose only metric needs statistics still passes, recording it absent; the body
/// announces that it ran, so a smoke test that never ran it fails this scenario.
/// CONTRACT_SHAPE: bounded-change (the child's announced body).
#[test]
fn the_smoke_variant_holds_no_statistics_and_asserts_no_timing() {
    let output = run_generated("smoke_no_timing", false);
    assert!(
        output.status.success()
            && stdout(&output).contains("1 passed")
            && stdout(&output).contains("smoke-probe no-timing body ran"),
        "the generated smoke test must hold no statistics descriptor and still pass: {}",
        stdout(&output)
    );
}

/// A benchmark whose fixture no longer builds fails its smoke test, and the failure names the
/// benchmark and carries the fixture's reason. The assertion reads only the panic message that
/// `overdrive_bench::smoke` produced: libtest prints the test path (which contains the benchmark's
/// name) on its own lines, so those cannot satisfy it. Fault stimulus: the fixture returns a fixture failure.
/// CONTRACT_SHAPE: bounded-change (the child's verdict and panic message).
#[test]
fn a_broken_fixture_fails_its_generated_smoke_test_naming_the_benchmark() {
    let output = run_generated("smoke_broken_fixture", true);
    let message = panic_message(&output);
    assert!(
        !output.status.success()
            && message.contains("broken_fixture")
            && message.contains("the fixture no longer builds a dataplane"),
        "a broken fixture must fail its generated smoke test, the panic message naming the benchmark and the reason; message: `{message}`; output: {}{}",
        stdout(&output),
        String::from_utf8_lossy(&output.stderr)
    );
}

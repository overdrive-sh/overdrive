//! Positive-path acceptance scenarios for `wire-exec-spec-end-to-end` —
//! `Job::from_spec` accepts the new `[exec]` block and preserves the
//! operator's command + args field-for-field.
//!
//! Covers `docs/feature/wire-exec-spec-end-to-end/distill/test-scenarios.md`
//! §2 *positive cases*: empty args is valid, casing preserved, args
//! opaqueness.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use overdrive_core::aggregate::{
    DriverInput, Exec, ExecInput, Job, JobSpecInput, ResourcesInput, WorkloadDriver,
};

/// Helper — produce a spec whose `exec.command` is `cmd` and `exec.args`
/// is `argv`, leaving id / replicas / resources at canonical-valid
/// values.
fn spec_with(cmd: &str, argv: Vec<String>) -> JobSpecInput {
    JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 134_217_728 },
        driver: DriverInput::Exec(ExecInput { command: cmd.to_string(), args: argv }),
    }
}

#[test]
fn job_from_spec_accepts_non_empty_command_with_empty_args_vec() {
    // Per ADR-0031 §4: empty `args` is the legitimate zero-args case
    // for binaries that take no arguments (`/bin/true`, `/bin/date`).
    let spec = spec_with("/bin/true", vec![]);
    let job = Job::from_spec(spec).expect("non-empty command + empty args is valid");
    // Per ADR-0031 Amendment 1 the command + args live one level
    // deeper through the tagged-enum `WorkloadDriver` field.
    let WorkloadDriver::Exec(Exec { command, args }) = &job.driver;
    assert_eq!(command, "/bin/true");
    assert!(args.is_empty(), "args must remain empty; got {args:?}");
}

#[test]
fn job_from_spec_preserves_operator_command_casing_verbatim() {
    // Per ADR-0031 §4: validation is a *predicate*, not a
    // *normalisation*. The operator's casing flows through to the
    // driver as-typed. Mixed casing in a path is uncommon but legal
    // (e.g. ext4 case-sensitive).
    let spec = spec_with("/Opt/Payments/Server", vec![]);
    let job = Job::from_spec(spec).expect("mixed-case command is valid");
    let WorkloadDriver::Exec(Exec { command, .. }) = &job.driver;
    assert_eq!(command, "/Opt/Payments/Server", "command must preserve operator casing verbatim",);
}

#[test]
fn job_from_spec_accepts_empty_string_and_whitespace_in_args_vec() {
    // Per ADR-0031 §4: argv is opaque to the platform. Per-element
    // validation is the kernel's job at `execve(2)`. Adding a Phase 1
    // rejection rule on individual args would diverge from the kernel
    // posture for no safety benefit.
    let argv = vec![String::new(), "  ".to_string(), "non-empty".to_string()];
    let spec = spec_with("/bin/echo", argv.clone());
    let job = Job::from_spec(spec).expect("empty / whitespace args elements are valid");
    let WorkloadDriver::Exec(Exec { args, .. }) = &job.driver;
    assert_eq!(
        args, &argv,
        "args vector must be preserved verbatim including empty / whitespace elements",
    );
}

#[test]
fn job_from_spec_carries_command_and_args_through_to_aggregate() {
    // Walking-skeleton-shaped sanity check: the operator's declared
    // command + args appear unchanged on the validated `Job`. This is
    // the minimal happy-path assertion for the projection to the
    // intent surface.
    let argv = vec!["--port".to_string(), "8080".to_string(), "--mode=fast".to_string()];
    let spec = spec_with("/opt/payments/bin/payments-server", argv.clone());
    let job = Job::from_spec(spec).expect("canonical exec spec is valid");
    let WorkloadDriver::Exec(Exec { command, args }) = &job.driver;
    assert_eq!(command, "/opt/payments/bin/payments-server");
    assert_eq!(args, &argv);
}

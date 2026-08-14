//! RED scaffolds for the "exactly one driver table" parse contract —
//! `microvm-driver-cloud-hypervisor` (GH #42), Slice 01.
//!
//! Per `docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md`
//! § Slice 01 AC-02 (S-VM-06, S-VM-07) and ADR-0083 §D4. Replaces
//! `ParseError::MissingExec` (`workload_spec.rs:743-745`) with
//! `MissingDriverSection` / `MultipleDriverSections`, mirroring the
//! existing `MixedServiceAndJob` / `MissingKindSection` pair one axis
//! over. `ParseError::MissingExec` is deleted in the same commit — single
//! cut, no alias, per `feedback_single_cut_greenfield_migrations.md`.
//!
//! Driving port: the TOML spec parser (in-process, no subprocess, no
//! `overdrive serve` needed — a pure parse-boundary rejection). Per
//! `.claude/rules/testing.md` § "RED scaffolds": placeholder bodies only.

#![allow(clippy::missing_panics_doc)]

use overdrive_core::aggregate::{ParseError, WorkloadSpecInput};

/// S-VM-06 — a spec declaring BOTH `[exec]` and `[vm]` is rejected with
/// `MultipleDriverSections` naming both tables; no allocation is created.
#[test]
fn spec_with_both_exec_and_vm_tables_is_rejected() {
    let toml = r#"
        [job]
        id = "batch-render"

        [exec]
        command = "/bin/true"
        args = []

        [vm]
        command = "/usr/bin/render"
        args = []
        kernel = "/srv/vm/kernel"
        rootfs = "/srv/vm/rootfs.ext4"

        [resources]
        cpu_milli = 100
        memory_bytes = 1048576
    "#;

    let err = WorkloadSpecInput::from_toml_str(toml)
        .expect_err("both [exec] and [vm] present must be rejected");

    assert!(
        matches!(err, ParseError::MultipleDriverSections),
        "expected ParseError::MultipleDriverSections, got {err:?}"
    );
    // The Display form names both tables (ADR-0083 §D4).
    let message = err.to_string();
    assert!(message.contains("[exec]"), "message must name [exec]: {message}");
    assert!(message.contains("[vm]"), "message must name [vm]: {message}");
}

/// S-VM-07 — a spec declaring NEITHER `[exec]` nor `[vm]` is rejected
/// with `MissingDriverSection`.
#[test]
fn spec_with_neither_exec_nor_vm_table_is_rejected() {
    let toml = r#"
        [job]
        id = "batch-render"

        [resources]
        cpu_milli = 100
        memory_bytes = 1048576
    "#;

    let err = WorkloadSpecInput::from_toml_str(toml)
        .expect_err("neither [exec] nor [vm] present must be rejected");

    assert!(
        matches!(err, ParseError::MissingDriverSection),
        "expected ParseError::MissingDriverSection, got {err:?}"
    );
}

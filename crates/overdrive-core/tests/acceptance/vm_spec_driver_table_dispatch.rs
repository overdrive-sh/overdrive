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

// ---------------------------------------------------------------------------
// Slice 04 / AC-15 (S-VM-62) — the `[[vm.volume]]` operator surface is closed.
//
// Per test-scenarios.md § Slice 04 AC-15 and feature-delta [D8a]: only
// `source`, `target` and `read_only` are accepted volume keys, and the
// rejection surfaces through the SAME `from_toml_str` parse boundary as
// S-VM-06/07 — a nested `deny_unknown_fields` struct failing during the
// same call. `[[vm.volume]]` rides inside Slice 01's JobEnvelope V2 (no
// second rkyv bump); the parsed volumes are not flowed to the aggregate in
// this step (that is a later Slice-04 step). This file only proves the
// operator surface is closed.
// ---------------------------------------------------------------------------

/// S-VM-62 — a `[[vm.volume]]` table declaring a key other than `source`,
/// `target` or `read_only` (e.g. `cache`) is rejected BY NAME.
#[test]
fn vm_volume_with_unknown_key_is_rejected_naming_the_key() {
    let toml = r#"
        [job]
        id = "batch-render"

        [vm]
        command = "/usr/bin/render"
        args = []
        kernel = "/srv/vm/kernel"
        rootfs = "/srv/vm/rootfs.ext4"

        [[vm.volume]]
        source = "/var/lib/overdrive/outputs/batch-render"
        target = "/output"
        cache = "always"

        [resources]
        cpu_milli = 100
        memory_bytes = 1048576
    "#;

    let err = WorkloadSpecInput::from_toml_str(toml)
        .expect_err("an unknown [[vm.volume]] key must be rejected");
    // The rejection names the unrecognised key (AC-15 — "naming the
    // unrecognised key").
    let message = err.to_string();
    assert!(
        message.contains("cache"),
        "the rejection must name the unrecognised key `cache`, got: {message}"
    );
}

/// S-VM-62 companion — the three documented `[[vm.volume]]` keys
/// (`source`, `target`, `read_only`) are ACCEPTED, `read_only` is optional
/// (defaults false), and a volume-free `[vm]` spec still parses (zero
/// volumes is the default). Without this, the unknown-key test above
/// passes an implementation that rejects EVERY `[[vm.volume]]`.
#[test]
fn vm_volume_with_only_documented_keys_is_accepted() {
    let with_volume = r#"
        [job]
        id = "batch-render"

        [vm]
        command = "/usr/bin/render"
        args = []
        kernel = "/srv/vm/kernel"
        rootfs = "/srv/vm/rootfs.ext4"

        [[vm.volume]]
        source = "/var/lib/overdrive/outputs/batch-render"
        target = "/output"
        read_only = false

        [resources]
        cpu_milli = 100
        memory_bytes = 1048576
    "#;
    WorkloadSpecInput::from_toml_str(with_volume)
        .expect("a [[vm.volume]] with only source/target/read_only must parse");

    // `read_only` is optional (defaults false) and a `[vm]` with no
    // `[[vm.volume]]` at all stays valid — zero volumes is the default.
    let read_only_omitted = r#"
        [job]
        id = "batch-render"

        [vm]
        command = "/usr/bin/render"
        args = []
        kernel = "/srv/vm/kernel"
        rootfs = "/srv/vm/rootfs.ext4"

        [[vm.volume]]
        source = "/data/in"
        target = "/in"

        [resources]
        cpu_milli = 100
        memory_bytes = 1048576
    "#;
    WorkloadSpecInput::from_toml_str(read_only_omitted)
        .expect("read_only is optional and defaults false");

    let no_volume = r#"
        [job]
        id = "batch-render"

        [vm]
        command = "/usr/bin/render"
        args = []
        kernel = "/srv/vm/kernel"
        rootfs = "/srv/vm/rootfs.ext4"

        [resources]
        cpu_milli = 100
        memory_bytes = 1048576
    "#;
    WorkloadSpecInput::from_toml_str(no_volume)
        .expect("a [vm] spec with no [[vm.volume]] must still parse (zero volumes is the default)");
}

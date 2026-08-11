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

/// S-VM-06 — a spec declaring BOTH `[exec]` and `[vm]` is rejected with
/// `MultipleDriverSections` naming both tables; no allocation is created.
#[test]
#[should_panic(expected = "RED scaffold")]
fn spec_with_both_exec_and_vm_tables_is_rejected() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-06 / [exec]+[vm] both \
         present -- ParseError::MultipleDriverSections naming both tables, \
         no intent committed -- ADR-0083 §D4)"
    );
}

/// S-VM-07 — a spec declaring NEITHER `[exec]` nor `[vm]` is rejected
/// with `MissingDriverSection`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn spec_with_neither_exec_nor_vm_table_is_rejected() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-07 / neither [exec] nor \
         [vm] present -- ParseError::MissingDriverSection -- replaces the \
         deleted ParseError::MissingExec, single cut -- ADR-0083 §D4)"
    );
}

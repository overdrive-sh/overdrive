//! 03-05 acceptance for the typed driver-start failure contract (DWD-24).
//!
//! `@contract-shape:pure-function` — every scenario drives the total
//! `DriverStartFailure -> TransitionReason` conversion through the public
//! core contract and asserts only on the returned value. No test reaches
//! into a private classifier, and none of them exists any more: the
//! action-shim text grammar these scenarios replace is deleted.
//!
//! The load-bearing property across the whole file is that **`class`
//! selects the cause and `detail` never does**. Under the retired grammar
//! the diagnostic prose WAS the classification input, so rewording a
//! driver's message silently changed the operator's diagnosis; these
//! scenarios pin that this is now structurally impossible.

#![allow(clippy::missing_panics_doc)]

use overdrive_core::TransitionReason;
use overdrive_core::traits::driver::{
    DriverError, DriverStartClass, DriverStartFailure, DriverType, ExecStartFailure, VmStartFailure,
};
use proptest::prelude::*;

/// Drive the conversion through the same port the action shim uses: a
/// `DriverError::StartRejected` carrying the typed failure.
fn convert(failure: DriverStartFailure) -> (TransitionReason, String) {
    let error = DriverError::StartRejected { failure };
    match &error {
        DriverError::StartRejected { failure } => {
            (TransitionReason::from(failure), failure.detail.clone())
        }
        other => panic!("expected StartRejected, got {other:?}"),
    }
}

/// Non-empty diagnostics that share NO vocabulary with any retired prefix
/// (`spawn `, `cgroup setup failed: `, `No such file or directory`, ...).
/// If any of these could still steer a classification, the grammar would
/// not really be gone.
fn arbitrary_detail() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("fichier introuvable".to_owned()),
        Just("the thing is simply not there".to_owned()),
        Just("spawn /decoy: No such file or directory (os error 2)".to_owned()),
        Just("cgroup setup failed: place_pid: decoy".to_owned()),
        Just("\u{1F4A5} unprintable-ish \u{0}\u{7} tail".to_owned()),
        "[^\u{0}]{1,64}",
    ]
}

// ---------------------------------------------------------------------
// The envelope shape itself.
// ---------------------------------------------------------------------

/// A rejected start exposes exactly one structured cause and one
/// separately preserved, non-empty diagnostic detail.
#[test]
fn driver_start_rejection_exposes_one_typed_cause_and_one_verbatim_detail() {
    let detail = "stat rootfs master /srv/vm/root.ext4: No such file or directory (os error 2)";
    let failure = DriverStartFailure {
        class: DriverStartClass::Vm(VmStartFailure::RootfsNotFound {
            path: "/srv/vm/root.ext4".to_owned(),
        }),
        detail: detail.to_owned(),
    };

    let (reason, preserved) = convert(failure.clone());

    // One structured cause...
    assert_eq!(
        reason,
        TransitionReason::VmRootfsNotFound { path: "/srv/vm/root.ext4".to_owned() },
        "the typed class must select exactly one operator-visible cause",
    );
    // ...and one separately preserved verbatim diagnostic.
    assert_eq!(preserved, detail, "the low-level diagnostic must survive byte-for-byte");
    assert!(!preserved.is_empty(), "the diagnostic channel must be non-empty");

    // The two channels are independent: the cause payload carries the
    // structured path, NOT the diagnostic prose.
    match reason {
        TransitionReason::VmRootfsNotFound { path } => assert_ne!(
            path, preserved,
            "the structured payload must not be the diagnostic text in disguise",
        ),
        other => panic!("expected VmRootfsNotFound, got {other:?}"),
    }

    // The family rides the class, so it cannot contradict the cause.
    assert_eq!(failure.class.driver_type(), DriverType::Vm);
}

// ---------------------------------------------------------------------
// The property the retired grammar could not hold.
// ---------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Holding the structured facts constant while changing ONLY the
    /// diagnostic prose must not change the selected operator cause.
    ///
    /// Quantified over every class family — Exec, VM, and the unknown
    /// fallback — because the fallback is the one arm where `detail` is
    /// legitimately read, and it must still not *select* anything.
    #[test]
    fn typed_cause_selection_is_independent_of_diagnostic_wording(
        first in arbitrary_detail(),
        second in arbitrary_detail(),
    ) {
        let classes = [
            DriverStartClass::Exec(ExecStartFailure::BinaryNotFound {
                path: "/usr/local/bin/payments".to_owned(),
            }),
            DriverStartClass::Exec(ExecStartFailure::CgroupSetupFailed {
                kind: "create_scope".to_owned(),
                source: "EACCES".to_owned(),
            }),
            DriverStartClass::Vm(VmStartFailure::KernelNotFound {
                path: "/srv/vm/vmlinuz".to_owned(),
            }),
            DriverStartClass::Vm(VmStartFailure::BootDeadlineExceeded {
                deadline_ms: 30_000,
                console_tail: Some("panic: no init".to_owned()),
            }),
            DriverStartClass::Unclassified { driver: DriverType::Vm },
        ];

        for class in classes {
            let (reason_a, detail_a) = convert(DriverStartFailure {
                class: class.clone(),
                detail: first.clone(),
            });
            let (reason_b, detail_b) = convert(DriverStartFailure {
                class: class.clone(),
                detail: second.clone(),
            });

            let unclassified = matches!(class, DriverStartClass::Unclassified { .. });
            if unclassified {
                // The ONE arm that legitimately carries the diagnostic:
                // it still does not SELECT a named cause from the text.
                let a_is_internal =
                    matches!(reason_a, TransitionReason::DriverInternalError { .. });
                let b_is_internal =
                    matches!(reason_b, TransitionReason::DriverInternalError { .. });
                prop_assert!(a_is_internal, "unknown must stay the internal-error fallback");
                prop_assert!(b_is_internal, "unknown must stay the internal-error fallback");
            } else {
                prop_assert_eq!(
                    &reason_a,
                    &reason_b,
                    "rewording the diagnostic changed the operator cause for {:?}",
                    class,
                );
            }

            // Whatever the class, the diagnostic is preserved verbatim.
            prop_assert_eq!(&detail_a, &first);
            prop_assert_eq!(&detail_b, &second);
        }
    }
}

// ---------------------------------------------------------------------
// Exec parity — every payload below is the pre-existing live operator
// surface and must not move.
// ---------------------------------------------------------------------

/// Assert one Exec class converts to exactly `expected` and preserves its
/// diagnostic verbatim.
fn assert_exec_parity(class: ExecStartFailure, detail: &str, expected: &TransitionReason) {
    let (reason, preserved) = convert(DriverStartFailure {
        class: DriverStartClass::Exec(class),
        detail: detail.to_owned(),
    });
    assert_eq!(&reason, expected, "Exec operator classification must not change");
    assert_eq!(preserved, detail, "the verbatim Exec diagnostic must be preserved");
}

#[test]
fn exec_binary_not_found_preserves_existing_operator_cause_and_detail() {
    assert_exec_parity(
        ExecStartFailure::BinaryNotFound { path: "/no/such".to_owned() },
        "spawn /no/such: No such file or directory (os error 2)",
        &TransitionReason::ExecBinaryNotFound { path: "/no/such".to_owned() },
    );
}

#[test]
fn exec_permission_denied_preserves_existing_operator_cause_and_detail() {
    assert_exec_parity(
        ExecStartFailure::PermissionDenied { path: "/usr/local/bin/payments".to_owned() },
        "spawn /usr/local/bin/payments: Permission denied (os error 13)",
        &TransitionReason::ExecPermissionDenied { path: "/usr/local/bin/payments".to_owned() },
    );
}

/// ENOEXEC keeps the canonical `kind` token exactly. A renamed token is
/// an operator-visible break even though nothing fails to compile, so it
/// is asserted as a literal rather than via a shared constant.
#[test]
fn exec_format_error_preserves_exec_format_error_kind_and_detail() {
    assert_exec_parity(
        ExecStartFailure::BinaryInvalid {
            path: "/tmp/garbage".to_owned(),
            kind: "exec_format_error".to_owned(),
        },
        "spawn /tmp/garbage: Exec format error (os error 8)",
        &TransitionReason::ExecBinaryInvalid {
            path: "/tmp/garbage".to_owned(),
            kind: "exec_format_error".to_owned(),
        },
    );

    // The retired Phase-1 wording must not reappear anywhere.
    let (reason, _) = convert(DriverStartFailure {
        class: DriverStartClass::Exec(ExecStartFailure::BinaryInvalid {
            path: "/tmp/garbage".to_owned(),
            kind: "exec_format_error".to_owned(),
        }),
        detail: "spawn /tmp/garbage: Exec format error (os error 8)".to_owned(),
    });
    match reason {
        TransitionReason::ExecBinaryInvalid { kind, .. } => {
            assert_eq!(kind, "exec_format_error");
            assert_ne!(kind, "not_executable", "the retired sub-cause token must not return");
            assert_ne!(kind, "bad_elf", "the retired sub-cause token must not return");
            assert_ne!(kind, "wrong_arch", "the retired sub-cause token must not return");
        }
        other => panic!("expected ExecBinaryInvalid, got {other:?}"),
    }
}

#[test]
fn exec_cgroup_create_scope_failure_preserves_existing_kind_and_detail() {
    assert_exec_parity(
        ExecStartFailure::CgroupSetupFailed {
            kind: "create_scope".to_owned(),
            source: "mkdir /sys/fs/cgroup/...: Permission denied".to_owned(),
        },
        "create workload scope: mkdir /sys/fs/cgroup/...: Permission denied",
        &TransitionReason::CgroupSetupFailed {
            kind: "create_scope".to_owned(),
            source: "mkdir /sys/fs/cgroup/...: Permission denied".to_owned(),
        },
    );
}

#[test]
fn exec_cgroup_place_pid_failure_preserves_existing_kind_and_detail() {
    assert_exec_parity(
        ExecStartFailure::CgroupSetupFailed {
            kind: "place_pid".to_owned(),
            source: "write cgroup.procs: Permission denied".to_owned(),
        },
        "place pid in scope: write cgroup.procs: Permission denied",
        &TransitionReason::CgroupSetupFailed {
            kind: "place_pid".to_owned(),
            source: "write cgroup.procs: Permission denied".to_owned(),
        },
    );
}

// ---------------------------------------------------------------------
// The single unknown fallback.
// ---------------------------------------------------------------------

/// The closed contract has ONE unknown fallback: the pre-existing
/// `DriverInternalError`, carrying the diagnostic verbatim. An unknown
/// failure is never guessed into a named Exec or VM cause.
#[test]
fn unclassified_start_failure_maps_only_to_driver_internal_error_with_verbatim_detail() {
    for driver in [DriverType::Exec, DriverType::Vm, DriverType::Unikernel, DriverType::Wasm] {
        // Prose deliberately shaped like the retired grammar's own
        // matches — if any of it still steered a decision, this would
        // resolve to a named Exec cause instead of the fallback.
        let detail = "spawn /no/such: No such file or directory (os error 2)";
        let (reason, preserved) = convert(DriverStartFailure {
            class: DriverStartClass::Unclassified { driver },
            detail: detail.to_owned(),
        });

        assert_eq!(
            reason,
            TransitionReason::DriverInternalError { detail: detail.to_owned() },
            "an unknown {driver} failure must reach the internal-error fallback, never a guess",
        );
        assert_eq!(preserved, detail, "the diagnostic must be preserved verbatim");
    }
}

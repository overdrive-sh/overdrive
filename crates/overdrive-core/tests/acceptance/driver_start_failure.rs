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
    DriverError, DriverStartClass, DriverStartFailure, DriverType, VmStartFailure,
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

/// Non-empty diagnostics that do not encode the structured start cause.
fn arbitrary_detail() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("fichier introuvable".to_owned()),
        Just("the thing is simply not there".to_owned()),
        Just("the substrate returned an unclassified status".to_owned()),
        Just("guest control channel did not answer".to_owned()),
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

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[test]
fn frozen_driver_error_remains_exhaustively_matchable_by_external_callers() {
    fn public_match(error: &DriverError) -> &'static str {
        match error {
            DriverError::StartRejected { .. } => "start_rejected",
            DriverError::NotFound { .. } => "not_found",
            DriverError::Io(_) => "io",
            DriverError::ResizeUnsupported { .. } => "resize_unsupported",
        }
    }

    let error = DriverError::StartRejected {
        failure: DriverStartFailure {
            class: DriverStartClass::Unclassified { driver: DriverType::Vm },
            detail: "compatible public shape".to_owned(),
        },
    };
    assert_eq!(public_match(&error), "start_rejected");
}

// ---------------------------------------------------------------------
// The property the retired grammar could not hold.
// ---------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Holding the structured facts constant while changing ONLY the
    /// diagnostic prose must not change the selected operator cause.
    ///
    /// Quantified over every class family — VM, and the unknown
    /// fallback — because the fallback is the one arm where `detail` is
    /// legitimately read, and it must still not *select* anything.
    #[test]
    fn typed_cause_selection_is_independent_of_diagnostic_wording(
        first in arbitrary_detail(),
        second in arbitrary_detail(),
    ) {
        let classes = [
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
// The single unknown fallback.
// ---------------------------------------------------------------------

/// The closed contract has ONE unknown fallback: the pre-existing
/// `DriverInternalError`, carrying the diagnostic verbatim. An unknown
/// failure is never guessed into a named VM cause.
#[test]
fn unclassified_start_failure_maps_only_to_driver_internal_error_with_verbatim_detail() {
    for driver in [DriverType::Vm, DriverType::Unikernel, DriverType::Wasm] {
        let detail = "unclassified substrate failure";
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

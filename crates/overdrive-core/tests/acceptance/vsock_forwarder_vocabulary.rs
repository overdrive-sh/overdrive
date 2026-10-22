//! EVMOD-PORT — the flow forwarder's core vocabulary (GH #295, the in-kernel
//! event-driven forwarder).
//!
//! Contract: `docs/feature/netns-density-295/feature-delta.md` § "[REF] vsock
//! data path — the in-kernel event-driven forwarder (EVMOD)" → "[REF] Driven
//! port — `FlowForwarder` (EVMOD-PORT)". The port's behaviour — attach,
//! watch, abort, the end records, the inventory — belongs to its adapters and
//! is specified against `SimFlowForwarder` (`overdrive-sim`, step 4) and the
//! module's Tier-3 suite through `KernelFlowForwarder` (`overdrive-fwd`, step
//! 4); this file pins the vocabulary every adapter and both owners share:
//!
//! - `ForwardId` and `ForwarderGeneration` are complete newtypes (validating
//!   constructor, decimal `Display` / `FromStr`, serde matching them; the
//!   mandatory proptest roundtrip, `.claude/rules/rust.md` § Newtypes);
//! - `ForwarderGeneration::next` advances by one and stops at `u64::MAX`
//!   (QPROJ-8: each `Forwarder` recovery's successful open advances it);
//! - `PairSide::as_str` and `AbortCause::as_str` own their labels (the label
//!   names the cause class, never its side or errno);
//! - every `ForwarderError` renders its pinned operator text (C6b), and the
//!   errors that wrap a cause keep it as their `source`.
//!
//! Driving port: the public items of
//! `overdrive_core::guest_transport::forwarder` (layer 1, pure: `proptest`
//! full, Mandate 9).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    reason = "test bodies unwrap fixtures they construct; the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::collections::BTreeSet;
use std::error::Error as _;

use overdrive_core::guest_transport::forwarder::{
    AbortCause, FlowForwarder, ForwardId, ForwardIdError, ForwardRecordField, ForwarderError,
    ForwarderGeneration, ForwarderInventory, ForwarderOp, ForwarderProbeObservation,
    ForwarderProbeStage, PairSide,
};
use overdrive_core::guest_transport::wire::KernelErrno;
use proptest::prelude::*;

/// The kernel adapter implements the port over `dyn AsFd`; the owners hold
/// it behind the port, so the trait must be usable as a trait object.
const fn assert_port_is_object_safe(_: Option<&dyn FlowForwarder<dyn std::os::fd::AsFd>>) {}
const _: () = assert_port_is_object_safe(None);

/// One abort cause of every class, each with both sides where it has one.
fn every_cause() -> Vec<AbortCause> {
    let mut causes = vec![AbortCause::Detached];
    for side in [PairSide::A, PairSide::B] {
        causes.push(AbortCause::PeerAbort { side });
        causes.push(AbortCause::SocketError { side, errno: KernelErrno::ECONNRESET });
        causes.push(AbortCause::FrameInvalid { side });
        causes.push(AbortCause::Unclassified { side });
    }
    causes
}

// ===========================================================================
// ForwardId — complete newtype
// ===========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Every non-zero id is accepted; `get` returns it; `Display` is decimal
    /// and re-parses through `FromStr`; serde carries the same decimal text.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_non_zero_forward_id_round_trips(raw in 1u64..=u64::MAX) {
        let id = ForwardId::new(raw).expect("a non-zero id is accepted");
        prop_assert_eq!(id.get(), raw);
        prop_assert_eq!(id.to_string(), raw.to_string(), "Display is decimal");
        prop_assert_eq!(raw.to_string().parse::<ForwardId>(), Ok(id));
        let json = serde_json::to_string(&id).expect("serialize");
        prop_assert_eq!(&json, &format!("\"{raw}\""), "serde matches Display");
        prop_assert_eq!(serde_json::from_str::<ForwardId>(&json).expect("deserialize"), id);
    }

    /// Ids order as their values (an owner's table iterates them in issue
    /// order).
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn forward_ids_order_by_value(a in 1u64..=u64::MAX, b in 1u64..=u64::MAX) {
        let (ia, ib) = (ForwardId::new(a).expect("non-zero"), ForwardId::new(b).expect("non-zero"));
        prop_assert_eq!(ia.cmp(&ib), a.cmp(&b));
        prop_assert_eq!(ia == ib, a == b);
    }
}

/// Zero is refused `Zero` by the constructor, `FromStr` and serde; text that
/// is not a decimal `u64` is `NotDecimal`.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn a_forward_id_refuses_zero_and_non_decimal_text() {
    assert_eq!(ForwardId::new(0), Err(ForwardIdError::Zero));
    assert_eq!("0".parse::<ForwardId>(), Err(ForwardIdError::Zero));
    assert!(serde_json::from_str::<ForwardId>("\"0\"").is_err(), "serde refuses id 0");
    for text in ["", "-1", "18446744073709551616", "0x1", " 1", "1 ", "one"] {
        assert_eq!(text.parse::<ForwardId>(), Err(ForwardIdError::NotDecimal), "{text:?}");
    }
}

// ===========================================================================
// ForwarderGeneration — complete newtype, advancing by one
// ===========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Every non-zero generation round-trips through `get`, decimal text and
    /// serde.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_non_zero_generation_round_trips(raw in 1u64..=u64::MAX) {
        let generation = ForwarderGeneration::new(raw).expect("a non-zero generation");
        prop_assert_eq!(generation.get(), raw);
        prop_assert_eq!(generation.to_string(), raw.to_string());
        prop_assert_eq!(raw.to_string().parse::<ForwarderGeneration>(), Ok(generation));
        let json = serde_json::to_string(&generation).expect("serialize");
        prop_assert_eq!(&json, &format!("\"{raw}\""));
        prop_assert_eq!(
            serde_json::from_str::<ForwarderGeneration>(&json).expect("deserialize"),
            generation
        );
    }

    /// QPROJ-8: `next` is the generation one above, and later than it, for
    /// every generation below `u64::MAX`.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn next_is_the_generation_one_above(raw in 1u64..u64::MAX) {
        let generation = ForwarderGeneration::new(raw).expect("non-zero");
        let next = generation.next().expect("below u64::MAX there is a next generation");
        prop_assert_eq!(next.get(), raw + 1);
        prop_assert!(next > generation);
    }
}

/// The first context's generation is 1; the second open is 2; the
/// generation at `u64::MAX` has no next; zero is refused.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_first_generation_is_one_and_the_last_has_no_next() {
    assert_eq!(ForwarderGeneration::FIRST.get(), 1);
    assert_eq!(ForwarderGeneration::new(1), Ok(ForwarderGeneration::FIRST));
    assert_eq!(ForwarderGeneration::FIRST.next().map(ForwarderGeneration::get), Some(2));
    let last = ForwarderGeneration::new(u64::MAX).expect("u64::MAX is a generation");
    assert_eq!(last.next(), None);
    assert_eq!(ForwarderGeneration::new(0), Err(ForwardIdError::Zero));
    assert_eq!("0".parse::<ForwarderGeneration>(), Err(ForwardIdError::Zero));
    assert_eq!("two".parse::<ForwarderGeneration>(), Err(ForwardIdError::NotDecimal));
}

// ===========================================================================
// Labels — one SSOT per enum
// ===========================================================================

/// The two sides carry distinct, non-empty labels.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_two_pair_sides_have_distinct_labels() {
    let labels: BTreeSet<&str> =
        [PairSide::A, PairSide::B].map(PairSide::as_str).into_iter().collect();
    assert_eq!(labels.len(), 2);
    assert!(labels.iter().all(|label| !label.is_empty()));
}

/// Each of the five abort-cause classes has its own non-empty label, and
/// the label names the class only: the same class on either side, or with
/// another errno, has the same label.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn every_abort_cause_class_has_one_distinct_label() {
    let labels: BTreeSet<&str> = every_cause().iter().map(AbortCause::as_str).collect();
    assert_eq!(labels.len(), 5, "one label per cause class: {labels:?}");
    assert!(labels.iter().all(|label| !label.is_empty()));
    assert_eq!(
        AbortCause::SocketError { side: PairSide::A, errno: KernelErrno::ECONNRESET }.as_str(),
        AbortCause::SocketError { side: PairSide::B, errno: KernelErrno::ECONNREFUSED }.as_str(),
    );
    assert_eq!(
        AbortCause::PeerAbort { side: PairSide::A }.as_str(),
        AbortCause::PeerAbort { side: PairSide::B }.as_str(),
    );
    assert_ne!(
        AbortCause::Unclassified { side: PairSide::A }.as_str(),
        AbortCause::PeerAbort { side: PairSide::A }.as_str(),
        "a broken invariant keeps a cause no reachable end uses (U-EV-7)"
    );
}

// ===========================================================================
// Plain data
// ===========================================================================

/// A fresh context's inventory is all zero.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_default_inventory_is_empty() {
    let inventory = ForwarderInventory::default();
    assert_eq!((inventory.live_pairs, inventory.live_watches, inventory.queued_ends), (0, 0, 0));
    assert_eq!(
        (
            inventory.attached,
            inventory.completed,
            inventory.aborted,
            inventory.unclassified_eof,
            inventory.dgram_oversize
        ),
        (0, 0, 0, 0, 0)
    );
}

// ===========================================================================
// Operator texts (C6b)
// ===========================================================================

/// The id error texts.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn forward_id_errors_render_their_pinned_messages() {
    assert_eq!(ForwardIdError::Zero.to_string(), "forward id is zero");
    assert_eq!(ForwardIdError::NotDecimal.to_string(), "forward id is not a decimal integer");
}

/// Every forwarder error that carries no errno renders its pinned text.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn forwarder_errors_render_their_pinned_messages() {
    let io = || std::io::Error::from(std::io::ErrorKind::NotFound);
    let cases: [(ForwarderError, &str); 12] = [
        (
            ForwarderError::DeviceAbsent { source: io() },
            "the flow forwarder device cannot be opened",
        ),
        (
            ForwarderError::AbiMismatch { expected: 1, found: 2 },
            "forwarder ABI 2 is not the expected 1",
        ),
        (
            ForwarderError::UnsupportedSocket { side: PairSide::A },
            "socket A is not a supported kind for this pair",
        ),
        (
            ForwarderError::UpperLayerProtocol { side: PairSide::B },
            "socket B carries an upper-layer protocol",
        ),
        (ForwarderError::NotConnected { side: PairSide::A }, "socket A is not connected"),
        (
            ForwarderError::AlreadyHooked { side: PairSide::B },
            "socket B is already attached or watched",
        ),
        (
            ForwarderError::BufferTooSmall { side: PairSide::A },
            "vsock socket A has a buffer smaller than the largest datagram frame",
        ),
        (
            ForwarderError::Io { op: ForwarderOp::Attach, source: io() },
            "forwarder operation Attach failed",
        ),
        (
            ForwarderError::EndDecode { field: ForwardRecordField::Kind },
            "a forwarder end record failed to decode",
        ),
        (
            ForwarderError::Probe {
                stage: ForwarderProbeStage::Version,
                source: Box::new(ForwarderError::AbiMismatch { expected: 1, found: 2 }),
            },
            "forwarder probe stage Version failed",
        ),
        (
            ForwarderError::ProbeMismatch {
                stage: ForwarderProbeStage::Backpressure,
                observed: ForwarderProbeObservation::SenderNotBlocked,
            },
            "forwarder probe stage Backpressure observed SenderNotBlocked",
        ),
        (
            ForwarderError::ProbeMismatch {
                stage: ForwarderProbeStage::Version,
                observed: ForwarderProbeObservation::ContextInheritable,
            },
            "forwarder probe stage Version observed ContextInheritable",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text, "{error:?}");
    }
}

/// EVMOD-ATTACH-END: an ended half names its side and the errno of its end
/// (`KernelErrno`'s decimal `Display`).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn a_half_ended_refusal_names_its_side_and_errno() {
    assert_eq!(
        ForwarderError::HalfEnded { side: PairSide::B, errno: KernelErrno::ECONNRESET }.to_string(),
        "socket B has already ended: errno 104"
    );
}

/// A probe failure keeps the stage's own error as its source, and a device
/// or control failure keeps the I/O error — the chain an operator reads is
/// never flattened to text.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn wrapping_forwarder_errors_keep_their_cause_as_the_source() {
    let probe = ForwarderError::Probe {
        stage: ForwarderProbeStage::DeviceOpen,
        source: Box::new(ForwarderError::DeviceAbsent {
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        }),
    };
    let inner = probe.source().expect("the stage's error is the source");
    assert_eq!(inner.to_string(), "the flow forwarder device cannot be opened");
    assert!(inner.source().is_some(), "the device's I/O error is kept below it");

    let io = ForwarderError::Io {
        op: ForwarderOp::Read,
        source: std::io::Error::from(std::io::ErrorKind::WouldBlock),
    };
    let source = io.source().expect("the I/O error is the source");
    assert_eq!(
        source.downcast_ref::<std::io::Error>().map(std::io::Error::kind),
        Some(std::io::ErrorKind::WouldBlock)
    );
    assert!(ForwarderError::AbiMismatch { expected: 1, found: 2 }.source().is_none());
}

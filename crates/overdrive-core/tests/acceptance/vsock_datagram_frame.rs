//! P3-S1 — the guest datagram frame carries any logical payload and restores
//! it exactly (D5, ADR-0149; DG-8). Layer-1 unit / `proptest` (Mandate 9 full
//! PBT at the pure codec layer).
//!
//! Spec: `docs/feature/netns-density-295/distill/test-scenarios-p3-udp-slots-framing.md`
//! P3-S1. Contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Wire contracts" (Datagram frame) and DG-8 (`DatagramFrame` /
//! `DatagramFrameError` in `overdrive_core::guest_transport::wire`).
//! Contract shape: pure-function (encode/decode round-trip, canonical decode).
//!
//! Driving port: the public codec `DatagramFrame::{new, payload, encode,
//! decode}`. This file is the Rust-codec side of the P3-S1 golden frames; the
//! host and guest programs assert the same bytes at Tier 3.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    reason = "test bodies unwrap fixtures they construct; the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use overdrive_core::guest_transport::wire::{
    DATAGRAM_FRAME_HEADER_LEN, DATAGRAM_FRAME_MAGIC, DatagramFrame, DatagramFrameError,
    MAX_GUEST_DATAGRAM_PAYLOAD,
};
use proptest::prelude::*;

const MAX: usize = MAX_GUEST_DATAGRAM_PAYLOAD as usize; // 59_000

/// The golden empty frame: the header alone, length 0.
const GOLDEN_EMPTY: [u8; DATAGRAM_FRAME_HEADER_LEN] =
    [b'O', b'V', b'D', b'1', 0x00, 0x00, 0x00, 0x00];

/// The golden maximum frame's header: `OVD1`, BE `59_000` (`0x0000_E678`).
const GOLDEN_MAX_HEADER: [u8; DATAGRAM_FRAME_HEADER_LEN] =
    [b'O', b'V', b'D', b'1', 0x00, 0x00, 0xE6, 0x78];

/// The golden maximum frame's payload: byte `i` is `i mod 251` (a prime
/// period, so no byte run aligns with the header or the length).
fn golden_max_payload() -> Vec<u8> {
    (0..MAX).map(|i| u8::try_from(i % 251).expect("< 251")).collect()
}

/// A hand-built frame: `magic`, BE `declared`, then `payload` verbatim.
fn raw_frame(magic: [u8; 4], declared: u32, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(DATAGRAM_FRAME_HEADER_LEN + payload.len());
    bytes.extend_from_slice(&magic);
    bytes.extend_from_slice(&declared.to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn declared_len(len: usize) -> u32 {
    u32::try_from(len).expect("test payloads are below 4 GiB")
}

// ===========================================================================
// Round trip (symmetric property) and canonical decode
// ===========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `decode(encode(p)) == p` for every logical payload in `0..=59_000`; the
    /// encoding is exactly magic, BE `u32` length, payload, pre-sized.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    fn every_payload_up_to_the_maximum_round_trips(
        payload in prop::collection::vec(any::<u8>(), 0..=MAX),
    ) {
        let frame = DatagramFrame::new(&payload).expect("a payload within the bound frames");
        prop_assert_eq!(frame.payload(), &payload[..]);

        let bytes = frame.encode();
        prop_assert_eq!(bytes.len(), DATAGRAM_FRAME_HEADER_LEN + payload.len());
        prop_assert_eq!(bytes.capacity(), bytes.len(), "pre-sized: no slack allocation");
        prop_assert_eq!(&bytes[0..4], &DATAGRAM_FRAME_MAGIC);
        prop_assert_eq!(&bytes[4..8], &declared_len(payload.len()).to_be_bytes());
        prop_assert_eq!(&bytes[8..], &payload[..]);

        let restored = DatagramFrame::decode(&bytes).expect("a self-encoded frame decodes");
        prop_assert_eq!(restored, frame);
        prop_assert_eq!(restored.payload(), &payload[..]);
    }

    /// Canonical decode: any byte string `decode` accepts re-encodes to
    /// exactly itself; every other byte string is refused with a declared
    /// variant (the error set is closed — the match below is exhaustive).
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    fn every_accepted_byte_string_is_canonical(
        bytes in prop_oneof![
            prop::collection::vec(any::<u8>(), 0..=64),
            // Bias towards the right magic so the length checks are reached.
            (prop::collection::vec(any::<u8>(), 0..=64), any::<u32>()).prop_map(|(tail, len)| {
                let mut b = DATAGRAM_FRAME_MAGIC.to_vec();
                b.extend_from_slice(&(len % 80).to_be_bytes());
                b.extend_from_slice(&tail);
                b
            }),
        ],
    ) {
        match DatagramFrame::decode(&bytes) {
            Ok(frame) => prop_assert_eq!(frame.encode(), bytes),
            Err(
                DatagramFrameError::Truncated { .. }
                | DatagramFrameError::BadMagic { .. }
                | DatagramFrameError::Oversize { .. }
                | DatagramFrameError::LengthMismatch { .. },
            ) => {}
        }
    }

    /// Every non-canonical mutation of a valid frame is refused: a changed
    /// magic byte, a declared length that disagrees with the carried payload
    /// (short or long), and trailing bytes after the payload.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    fn every_non_canonical_frame_is_refused(
        payload in prop::collection::vec(any::<u8>(), 0..=256),
        magic_byte in 0usize..4,
        flip in 1u8..=u8::MAX,
        extra in prop::collection::vec(any::<u8>(), 1..=16),
        skew in 1u32..=64,
    ) {
        let good = DatagramFrame::new(&payload).expect("within bound").encode();

        let mut bad_magic = good.clone();
        bad_magic[magic_byte] ^= flip;
        let mut observed = [0u8; 4];
        observed.copy_from_slice(&bad_magic[0..4]);
        prop_assert_eq!(
            DatagramFrame::decode(&bad_magic),
            Err(DatagramFrameError::BadMagic { observed })
        );

        let mut trailing = good;
        trailing.extend_from_slice(&extra);
        prop_assert_eq!(
            DatagramFrame::decode(&trailing),
            Err(DatagramFrameError::LengthMismatch {
                declared: declared_len(payload.len()),
                actual: payload.len() + extra.len(),
            })
        );

        let longer = declared_len(payload.len()) + skew;
        let over_declared = raw_frame(DATAGRAM_FRAME_MAGIC, longer, &payload);
        prop_assert_eq!(
            DatagramFrame::decode(&over_declared),
            Err(DatagramFrameError::LengthMismatch { declared: longer, actual: payload.len() })
        );
    }
}

// ===========================================================================
// Golden frames (the Rust-codec side of P3-S1's cross-codec pin)
// ===========================================================================

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn golden_empty_frame_is_the_header_alone() {
    let empty = DatagramFrame::new(&[]).expect("the empty datagram frames");
    assert_eq!(empty.encode(), GOLDEN_EMPTY.to_vec());
    let decoded = DatagramFrame::decode(&GOLDEN_EMPTY).expect("the golden empty frame decodes");
    assert_eq!(decoded.payload(), &[] as &[u8]);
    assert_eq!(decoded, empty);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn golden_maximum_frame_carries_the_inclusive_bound() {
    let payload = golden_max_payload();
    let mut golden = GOLDEN_MAX_HEADER.to_vec();
    golden.extend_from_slice(&payload);

    let frame = DatagramFrame::new(&payload).expect("59,000 bytes is within the inclusive bound");
    assert_eq!(frame.encode(), golden);
    assert_eq!(DatagramFrame::decode(&golden).expect("the golden max frame decodes"), frame);
}

/// The sizes the spike exercised (P-5 / P-29) round-trip byte-exact.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn the_spike_sizes_round_trip() {
    for len in [0usize, 1, 1_431, MAX] {
        let payload = vec![0xA5u8; len];
        let encoded = DatagramFrame::new(&payload).expect("within bound").encode();
        assert_eq!(
            DatagramFrame::decode(&encoded).expect("decodes").payload(),
            &payload[..],
            "{len} bytes"
        );
    }
}

// ===========================================================================
// Each refusal variant
// ===========================================================================

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn a_payload_over_the_maximum_is_refused_oversize_by_new() {
    let over = vec![0u8; MAX + 1];
    assert_eq!(
        DatagramFrame::new(&over),
        Err(DatagramFrameError::Oversize {
            length: u64::from(MAX_GUEST_DATAGRAM_PAYLOAD) + 1,
            max: MAX_GUEST_DATAGRAM_PAYLOAD,
        })
    );
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn a_declared_length_over_the_maximum_is_refused_oversize_by_decode() {
    // Declared one over the bound, carrying exactly that many bytes: the
    // length check, not the carried-bytes check, refuses it.
    let declared = MAX_GUEST_DATAGRAM_PAYLOAD + 1;
    let frame = raw_frame(DATAGRAM_FRAME_MAGIC, declared, &vec![0u8; MAX + 1]);
    assert_eq!(
        DatagramFrame::decode(&frame),
        Err(DatagramFrameError::Oversize {
            length: u64::from(declared),
            max: MAX_GUEST_DATAGRAM_PAYLOAD,
        })
    );
    // The largest declarable length carrying nothing is still Oversize.
    assert_eq!(
        DatagramFrame::decode(&raw_frame(DATAGRAM_FRAME_MAGIC, u32::MAX, &[])),
        Err(DatagramFrameError::Oversize {
            length: u64::from(u32::MAX),
            max: MAX_GUEST_DATAGRAM_PAYLOAD,
        })
    );
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn fewer_bytes_than_the_header_are_refused_truncated() {
    for len in 0..DATAGRAM_FRAME_HEADER_LEN {
        assert_eq!(
            DatagramFrame::decode(&GOLDEN_EMPTY[..len]),
            Err(DatagramFrameError::Truncated { len }),
            "{len} bytes"
        );
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn a_wrong_magic_is_refused_bad_magic() {
    for magic in [*b"OVD0", *b"ovd1", *b"OF\x02\x01", [0u8; 4]] {
        assert_eq!(
            DatagramFrame::decode(&raw_frame(magic, 2, b"hi")),
            Err(DatagramFrameError::BadMagic { observed: magic }),
            "{magic:02x?}"
        );
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn a_body_shorter_or_longer_than_declared_is_refused_length_mismatch() {
    // Short: never a truncated payload accepted.
    let short = raw_frame(DATAGRAM_FRAME_MAGIC, 100, &[1u8; 10]);
    assert_eq!(
        DatagramFrame::decode(&short),
        Err(DatagramFrameError::LengthMismatch { declared: 100, actual: 10 })
    );
    // Long: a second frame (or garbage) after the first is not one frame.
    let mut two = DatagramFrame::new(b"ab").expect("within bound").encode();
    two.extend_from_slice(&GOLDEN_EMPTY);
    assert_eq!(
        DatagramFrame::decode(&two),
        Err(DatagramFrameError::LengthMismatch {
            declared: 2,
            actual: 2 + DATAGRAM_FRAME_HEADER_LEN
        })
    );
}

/// The refusal texts name the cause (operators read them in owner audits).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn refusals_render_their_pinned_messages() {
    assert_eq!(
        DatagramFrameError::Truncated { len: 3 }.to_string(),
        "datagram frame is 3 bytes, shorter than its header"
    );
    assert_eq!(
        DatagramFrameError::BadMagic { observed: *b"OVD0" }.to_string(),
        "datagram frame magic [4f, 56, 44, 30] is not OVD1"
    );
    assert_eq!(
        DatagramFrameError::Oversize { length: 59_001, max: 59_000 }.to_string(),
        "datagram frame length 59001 exceeds 59000"
    );
    assert_eq!(
        DatagramFrameError::LengthMismatch { declared: 4, actual: 2 }.to_string(),
        "datagram frame declares 4 payload bytes but carries 2"
    );
}

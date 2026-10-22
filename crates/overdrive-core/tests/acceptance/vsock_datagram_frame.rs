//! P3-S1 / R5-51 — the guest datagram frame carries any logical payload and
//! restores it exactly (D5, ADR-0149; DG-8), the empty datagram included: an
//! empty datagram is the frame whose declared length is 0, and a payload that
//! itself parses as a frame is ordinary payload, stripped of exactly one outer
//! frame (EVMOD-DGRAM, ADR-0201: one vsock STREAM per association carries the
//! frames both ways, written and read by the forwarder; no escape rule).
//! Layer-1 unit / `proptest` (Mandate 9 full PBT at the pure codec layer).
//!
//! Contract: `docs/feature/netns-density-295/feature-delta.md` § "[REF] Wire
//! contracts" (Datagram frame) and DG-8 (`DatagramFrame` /
//! `DatagramFrameError` in `overdrive_core::guest_transport::wire`), with
//! boundary row R5-51. Contract shape: pure-function (encode/decode
//! round-trip, canonical decode).
//!
//! Driving port: the public codec `DatagramFrame::{new, payload, encode,
//! decode}`. This file is the Rust-codec side of R5-51's golden frames; the
//! forwarder module's UAPI header carries the same constants, checked by the
//! `overdrive-fwd` mirror, and its Tier-3 suite asserts the same bytes on the
//! wire (P-72). The forwarder's own framing behaviour — empties in order,
//! `dgram_oversize`, `FrameInvalid` on an incomplete frame at a clean peer
//! end — is the forwarder's, specified against `SimFlowForwarder` and the
//! module's Tier-3 suite, not here.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    reason = "test bodies unwrap fixtures they construct; the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use overdrive_core::guest_transport::wire::{
    DATAGRAM_FRAME_HEADER_LEN, DATAGRAM_FRAME_MAGIC, DatagramFrame, DatagramFrameError,
    MAX_GUEST_DATAGRAM_FRAME, MAX_GUEST_DATAGRAM_PAYLOAD,
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
fn golden_empty_frame_is_the_header_alone() {
    let empty = DatagramFrame::new(&[]).expect("the empty datagram frames");
    assert_eq!(empty.encode(), GOLDEN_EMPTY.to_vec());
    let decoded = DatagramFrame::decode(&GOLDEN_EMPTY).expect("the golden empty frame decodes");
    assert_eq!(decoded.payload(), &[] as &[u8]);
    assert_eq!(decoded, empty);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
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
// R5-51 — the zero-length datagram and frame-shaped payloads
// ===========================================================================

/// R5-51: the empty datagram is a frame of its own, never "no frame" — its
/// 8-byte header puts bytes on the stream, it is distinct from a one-byte
/// datagram, and a frame declaring length 0 that carries any byte is not one
/// canonical frame, so the codec never reads a payload into an empty
/// datagram.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_empty_datagram_is_a_header_only_frame_distinct_from_every_payload() {
    let empty = DatagramFrame::new(&[]).expect("the empty datagram frames");
    let encoded = empty.encode();
    assert_eq!(encoded.len(), DATAGRAM_FRAME_HEADER_LEN, "the header makes it non-empty");
    assert_eq!(&encoded[4..8], &[0u8; 4], "declared length 0");

    let one = DatagramFrame::new(&[0]).expect("a one-byte datagram frames");
    let one_bytes = one.encode();
    assert_ne!(one_bytes, encoded, "a one-byte datagram is not the empty datagram");
    assert_ne!(DatagramFrame::decode(&one_bytes).expect("decodes"), empty);

    for tail in [&b"\x00"[..], &b"x"[..], &GOLDEN_EMPTY[..]] {
        let mut smuggled = GOLDEN_EMPTY.to_vec();
        smuggled.extend_from_slice(tail);
        assert_eq!(
            DatagramFrame::decode(&smuggled),
            Err(DatagramFrameError::LengthMismatch { declared: 0, actual: tail.len() }),
            "declared 0 carrying {} bytes",
            tail.len()
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// R5-51 (no escape rule): a payload that is itself a valid frame —
    /// the empty frame included — is ordinary payload. Framing it and
    /// decoding strips exactly one outer frame and returns the inner frame's
    /// bytes unchanged.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn a_frame_shaped_payload_is_stripped_of_exactly_one_outer_frame(
        inner in prop::collection::vec(any::<u8>(), 0..=256),
    ) {
        let inner_frame = DatagramFrame::new(&inner).expect("within bound").encode();
        let outer = DatagramFrame::new(&inner_frame).expect("within bound").encode();
        let stripped = DatagramFrame::decode(&outer).expect("the outer frame decodes");
        prop_assert_eq!(stripped.payload(), &inner_frame[..]);
        prop_assert_eq!(
            DatagramFrame::decode(stripped.payload()).expect("the payload is a frame").payload(),
            &inner[..]
        );
    }
}

/// Splits a stream of back-to-back frames at each header's declared length —
/// the test's oracle for "one frame per datagram, in order" — decoding each
/// slice with the codec under test.
fn split_frames(mut stream: &[u8]) -> Vec<Vec<u8>> {
    let mut datagrams = Vec::new();
    while !stream.is_empty() {
        let mut declared = [0u8; 4];
        declared.copy_from_slice(&stream[4..8]);
        let declared = usize::try_from(u32::from_be_bytes(declared)).expect("fits usize");
        let end = DATAGRAM_FRAME_HEADER_LEN + declared;
        let frame = DatagramFrame::decode(&stream[..end]).expect("each slice is one frame");
        datagrams.push(frame.payload().to_vec());
        stream = &stream[end..];
    }
    datagrams
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// R5-51: a sequence of datagrams — empty ones included, anywhere —
    /// framed back to back is one frame per datagram, in order; an empty
    /// datagram contributes exactly its 8-byte header, so it is delivered
    /// once, between its neighbours.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn framed_datagrams_keep_their_order_and_every_empty_one(
        datagrams in prop::collection::vec(
            prop_oneof![Just(Vec::new()), prop::collection::vec(any::<u8>(), 0..=64)],
            0..=16,
        ),
    ) {
        let mut stream = Vec::new();
        for datagram in &datagrams {
            stream.extend_from_slice(&DatagramFrame::new(datagram).expect("within bound").encode());
        }
        let expected_len: usize =
            datagrams.iter().map(|d| DATAGRAM_FRAME_HEADER_LEN + d.len()).sum();
        prop_assert_eq!(stream.len(), expected_len);
        prop_assert_eq!(split_frames(&stream), datagrams);
    }
}

/// The largest frame is the largest payload plus the header (the datagram-mode
/// attach refuses a vsock half whose buffer is below it, EVMOD-DGRAM).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_largest_frame_is_the_largest_payload_plus_the_header() {
    assert_eq!(MAX_GUEST_DATAGRAM_PAYLOAD, 59_000);
    assert_eq!(MAX_GUEST_DATAGRAM_FRAME, 59_008);
    assert_eq!(
        u64::from(MAX_GUEST_DATAGRAM_FRAME),
        u64::from(MAX_GUEST_DATAGRAM_PAYLOAD) + DATAGRAM_FRAME_HEADER_LEN as u64
    );
    assert_eq!(DATAGRAM_FRAME_MAGIC, *b"OVD1");
}

/// The golden maximum frame is exactly `MAX_GUEST_DATAGRAM_FRAME` bytes.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_golden_maximum_frame_is_max_guest_datagram_frame_bytes() {
    let payload = golden_max_payload();
    let encoded = DatagramFrame::new(&payload).expect("within the inclusive bound").encode();
    assert_eq!(encoded.len() as u64, u64::from(MAX_GUEST_DATAGRAM_FRAME));
}

// ===========================================================================
// Each refusal variant
// ===========================================================================

/// CONTRACT_SHAPE: pure-function.
#[test]
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

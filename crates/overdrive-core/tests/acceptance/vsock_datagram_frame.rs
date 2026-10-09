//! P3-S1 — the guest datagram frame carries any logical payload and restores
//! it exactly (D5, ADR-0149). Layer-1 unit / `proptest` (Mandate 9 full PBT at
//! the pure codec layer). Spec: `test-scenarios-p3-udp-slots-framing.md` P3-S1;
//! `feature-delta.md` §"Wire contracts — datagram frame" (L18177), golden-bytes
//! L18174. Contract shape: pure-function (encode/decode round-trip).
//!
//! GATED (shared-infra §0): the replacement wire surface
//! `overdrive_core::guest_transport::wire` does not exist yet; DELIVER's roadmap
//! reconciliation creates it. This body is the authored RED scaffold; it is NOT
//! registered in `tests/acceptance.rs` and so is not compiled until the
//! production codec + the `mod vsock_datagram_frame;` line co-arrive in DELIVER.
//!
//! TESTABILITY GAP G-1: D5 pins the constants `DATAGRAM_FRAME_MAGIC` (`*b"OVD1"`),
//! `DATAGRAM_FRAME_HEADER_LEN` (8) and `MAX_GUEST_DATAGRAM_PAYLOAD` (59_000) and
//! that a "Rust codec" exists (the golden-bytes pin), but NOT the exact codec
//! type/method — unlike `FlowRequest`/`ControlMessage`, which pin
//! `encode`/`decode`. This scaffold is authored against the design-implied
//! `DatagramFrame { encode -> Vec<u8>, decode(&[u8]) -> Result }` shape mirroring
//! `FlowRequest`; the crafter reconciles the exact signature to the pinned codec
//! at activation (an internal-structure note for the DELIVER reviewer, not a
//! DESIGN blocker — CLAUDE.md §"Implement to the design").

#![allow(clippy::expect_used, clippy::unwrap_used)]

use overdrive_core::guest_transport::wire::{
    DATAGRAM_FRAME_HEADER_LEN, DATAGRAM_FRAME_MAGIC, DatagramFrame, DatagramFrameError,
    MAX_GUEST_DATAGRAM_PAYLOAD,
};
use proptest::prelude::*;

const MAX: usize = MAX_GUEST_DATAGRAM_PAYLOAD as usize; // 59_000

/// Symmetric property (Hebert §3 "symmetric"): `decode(encode(p)) == p` for
/// every logical payload in `0..=59_000`, and the encoding carries the magic +
/// a big-endian u32 logical length. Universe: { decoded payload, decode result }.
#[test]
#[ignore = "pending DELIVER: datagram frame codec in overdrive-core guest_transport::wire (D5)"]
fn datagram_frame_round_trips_every_payload_length() {
    proptest!(|(payload in prop::collection::vec(any::<u8>(), 0..=MAX))| {
        let frame = DatagramFrame::new(&payload).expect("payload within the 59_000 bound");
        let bytes = frame.encode();

        // Wire shape (D5): magic, BE u32 logical length, then payload.
        prop_assert_eq!(&bytes[0..4], &DATAGRAM_FRAME_MAGIC);
        let logical_len = u32::from_be_bytes(bytes[4..8].try_into().unwrap());
        prop_assert_eq!(logical_len as usize, payload.len());
        prop_assert_eq!(bytes.len(), DATAGRAM_FRAME_HEADER_LEN + payload.len());

        // Round-trip restores the payload byte-exact.
        let restored = DatagramFrame::decode(&bytes).expect("a self-encoded frame decodes");
        prop_assert_eq!(restored.payload(), &payload[..]);
    });
}

/// Pinned canonical examples (domain-readable, reviewer-facing) — the golden
/// sizes the spike exercised (P-5 / P-29: 0, 1, 1_431, 59_000) plus the exact
/// empty carrier (header alone) and the inclusive maximum.
#[test]
#[ignore = "pending DELIVER: datagram frame codec in overdrive-core guest_transport::wire (D5)"]
fn datagram_frame_pins_empty_carrier_and_boundary_sizes() {
    // C1a: the empty datagram is exactly the 8-byte header, no payload.
    let empty = DatagramFrame::new(&[]).expect("empty is a valid frame");
    let empty_bytes = empty.encode();
    assert_eq!(empty_bytes.len(), DATAGRAM_FRAME_HEADER_LEN);
    assert_eq!(&empty_bytes[0..4], &DATAGRAM_FRAME_MAGIC);
    assert_eq!(u32::from_be_bytes(empty_bytes[4..8].try_into().unwrap()), 0);
    assert_eq!(DatagramFrame::decode(&empty_bytes).expect("empty decodes").payload(), &[] as &[u8]);

    for len in [0usize, 1, 1_431, MAX] {
        let payload = vec![0xA5u8; len];
        let frame = DatagramFrame::new(&payload).expect("within bound");
        assert_eq!(
            DatagramFrame::decode(&frame.encode()).expect("decodes").payload(),
            &payload[..]
        );
    }
}

/// C1b boundary + C6 robustness (Postel): `59_001` (max+1) is refused at
/// construction, and every malformed byte string fails to decode with a typed
/// error — never a panic, never a truncated-but-accepted payload. The error set
/// is closed (C6c): no variant outside the declared `DatagramFrameError` escapes.
#[test]
#[ignore = "pending DELIVER: datagram frame codec in overdrive-core guest_transport::wire (D5)"]
fn datagram_frame_rejects_oversize_and_malformed_fail_closed() {
    // max+1: refused at construction (the logical bound is inclusive 59_000).
    let over = vec![0u8; MAX + 1];
    assert!(matches!(DatagramFrame::new(&over), Err(DatagramFrameError::PayloadTooLong { .. })));

    // Bad magic.
    let mut bad_magic = DatagramFrame::new(b"hi").unwrap().encode();
    bad_magic[0] = b'X';
    assert!(matches!(DatagramFrame::decode(&bad_magic), Err(DatagramFrameError::BadMagic { .. })));

    // A length prefix declaring more than the inclusive bound.
    let mut overlen = DatagramFrame::new(b"hi").unwrap().encode();
    overlen[4..8].copy_from_slice(&(MAX_GUEST_DATAGRAM_PAYLOAD + 1).to_be_bytes());
    assert!(matches!(
        DatagramFrame::decode(&overlen),
        Err(DatagramFrameError::PayloadTooLong { .. })
    ));

    // A header-only truncation: fewer than DATAGRAM_FRAME_HEADER_LEN bytes.
    assert!(matches!(
        DatagramFrame::decode(&[DATAGRAM_FRAME_MAGIC[0], DATAGRAM_FRAME_MAGIC[1]]),
        Err(DatagramFrameError::Truncated { .. })
    ));

    // A body shorter than the declared logical length — fail closed, never a
    // truncated payload.
    let mut short_body = DatagramFrame::new(&vec![1u8; 100]).unwrap().encode();
    short_body.truncate(DATAGRAM_FRAME_HEADER_LEN + 10);
    assert!(matches!(
        DatagramFrame::decode(&short_body),
        Err(DatagramFrameError::Truncated { .. })
    ));

    // C6c: fuzz arbitrary bytes — decode is total (Ok | one of the declared
    // Err variants), never a panic.
    proptest!(|(bytes in prop::collection::vec(any::<u8>(), 0..=64))| {
        match DatagramFrame::decode(&bytes) {
            Ok(frame) => {
                // An accepted frame re-encodes to the same canonical bytes.
                prop_assert_eq!(DatagramFrame::decode(&frame.encode()).unwrap().payload(),
                                frame.payload());
            }
            Err(
                DatagramFrameError::BadMagic { .. }
                | DatagramFrameError::Truncated { .. }
                | DatagramFrameError::PayloadTooLong { .. },
            ) => {}
        }
    });
}

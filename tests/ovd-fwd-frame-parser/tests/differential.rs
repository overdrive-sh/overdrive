//! FRAME-PARSE differential properties for the exact shipped C source.
//!
//! Driving ports: the test-only FFI `parse` wrapper and the production Rust
//! `DatagramFrame` codec. No parser or frame model is copied into this test.

#![cfg(target_os = "linux")]
#![allow(clippy::panic, reason = "proptest strategy construction has contract-guaranteed inputs")]

use overdrive_core::guest_transport::wire::{
    DATAGRAM_FRAME_HEADER_LEN, DATAGRAM_FRAME_MAGIC, DatagramFrame, DatagramFrameError,
    MAX_GUEST_DATAGRAM_PAYLOAD,
};
use overdrive_fwd_frame_parser_test::{FrameParse, parse};
use proptest::prelude::*;

const MAX: usize = MAX_GUEST_DATAGRAM_PAYLOAD as usize;

fn payload() -> BoxedStrategy<Vec<u8>> {
    prop_oneof![
        16 => prop::collection::vec(any::<u8>(), 0..=2_048),
        1 => Just(Vec::new()),
        1 => Just(vec![0xA5; MAX]),
    ]
    .boxed()
}

fn encoded(payload: &[u8]) -> Vec<u8> {
    DatagramFrame::new(payload)
        .unwrap_or_else(|error| panic!("bounded payload did not frame: {error}"))
        .encode()
}

fn parser_input_and_split() -> impl Strategy<Value = (Vec<u8>, usize)> {
    let declared = prop_oneof![
        Just(0),
        Just(MAX_GUEST_DATAGRAM_PAYLOAD - 1),
        Just(MAX_GUEST_DATAGRAM_PAYLOAD),
        Just(MAX_GUEST_DATAGRAM_PAYLOAD + 1),
        any::<u32>(),
    ];
    let header_shaped = (
        prop_oneof![4 => Just(DATAGRAM_FRAME_MAGIC), 1 => any::<[u8; 4]>()],
        declared,
        prop::collection::vec(any::<u8>(), 0..=1_024),
    )
        .prop_map(|(magic, length, tail)| {
            let mut bytes = Vec::with_capacity(DATAGRAM_FRAME_HEADER_LEN + tail.len());
            bytes.extend_from_slice(&magic);
            bytes.extend_from_slice(&length.to_be_bytes());
            bytes.extend_from_slice(&tail);
            bytes
        });
    let bytes = prop_oneof![
        8 => prop::collection::vec(any::<u8>(), 0..=1_024),
        8 => header_shaped,
        4 => payload().prop_map(|body| encoded(&body)),
    ];

    bytes.prop_flat_map(|bytes| {
        let split_end = bytes.len() + 1;
        (Just(bytes), 0..split_end)
    })
}

fn usize_from_u32(value: u32) -> usize {
    usize::try_from(value).unwrap_or_else(|_| panic!("u32 {value} did not fit usize"))
}

fn u64_from_usize(value: usize) -> u64 {
    u64::try_from(value).unwrap_or_else(|_| panic!("usize {value} did not fit u64"))
}

proptest! {
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn arbitrary_buffers_and_split_points_agree_with_the_rust_codec(
        (bytes, split) in parser_input_and_split(),
    ) {
        let visible = &bytes[..split];
        let parsed = parse(visible, u64_from_usize(split));

        match parsed {
            FrameParse::Invalid { need, length } => {
                prop_assert_eq!((need, length), (0, 0));
                match DatagramFrame::decode(visible) {
                    Err(DatagramFrameError::BadMagic { .. }) => {}
                    other => prop_assert!(false, "C invalid disagreed with Rust: {other:?}"),
                }
            }
            FrameParse::Oversize { need, length } => {
                prop_assert_eq!(need, 0);
                match DatagramFrame::decode(visible) {
                    Err(DatagramFrameError::Oversize { length: rust_length, max }) => {
                        prop_assert_eq!(rust_length, u64::from(length));
                        prop_assert_eq!(max, MAX_GUEST_DATAGRAM_PAYLOAD);
                    }
                    other => prop_assert!(false, "C oversize disagreed with Rust: {other:?}"),
                }
            }
            FrameParse::NeedMore { need, length } if split < DATAGRAM_FRAME_HEADER_LEN => {
                prop_assert_eq!(usize_from_u32(need), DATAGRAM_FRAME_HEADER_LEN);
                prop_assert_eq!(length, 0);
                prop_assert_eq!(
                    DatagramFrame::decode(visible),
                    Err(DatagramFrameError::Truncated { len: split })
                );
            }
            FrameParse::NeedMore { need, length } => {
                let need = usize_from_u32(need);
                prop_assert!(need > split);
                prop_assert_eq!(
                    u64_from_usize(need),
                    u64_from_usize(DATAGRAM_FRAME_HEADER_LEN) + u64::from(length)
                );
                match DatagramFrame::decode(visible) {
                    Err(DatagramFrameError::LengthMismatch { declared, actual }) => {
                        prop_assert_eq!(declared, length);
                        prop_assert_eq!(actual, split - DATAGRAM_FRAME_HEADER_LEN);
                    }
                    other => prop_assert!(false, "C need-more disagreed with Rust: {other:?}"),
                }
            }
            FrameParse::Complete { need, length } => {
                let need = usize_from_u32(need);
                prop_assert!(need <= split);
                prop_assert_eq!(
                    u64_from_usize(need),
                    u64_from_usize(DATAGRAM_FRAME_HEADER_LEN) + u64::from(length)
                );
                let rust = DatagramFrame::decode(&bytes[..need])
                    .unwrap_or_else(|error| panic!("C complete disagreed with Rust: {error}"));
                prop_assert_eq!(rust.payload().len(), usize_from_u32(length));
            }
        }
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn encoded_datagram_sequences_keep_every_boundary_under_arbitrary_chunks(
        datagrams in prop::collection::vec(payload(), 0..=8),
        chunks in prop::collection::vec(1usize..=8_192, 0..=64),
    ) {
        let mut stream = Vec::new();
        let mut expected_boundaries = Vec::with_capacity(datagrams.len());
        for datagram in &datagrams {
            stream.extend_from_slice(&encoded(datagram));
            expected_boundaries.push(stream.len());
        }

        let mut delivery_points = Vec::with_capacity(chunks.len() + 1);
        let mut delivered = 0usize;
        for chunk in chunks {
            delivered = delivered.saturating_add(chunk).min(stream.len());
            delivery_points.push(delivered);
            if delivered == stream.len() {
                break;
            }
        }
        if delivery_points.last().copied() != Some(stream.len()) {
            delivery_points.push(stream.len());
        }

        let mut cursor = 0usize;
        let mut observed = Vec::with_capacity(datagrams.len());
        let mut observed_boundaries = Vec::with_capacity(datagrams.len());
        for queued in delivery_points {
            while cursor < queued {
                let available = queued - cursor;
                let header_len = available.min(DATAGRAM_FRAME_HEADER_LEN);
                match parse(
                    &stream[cursor..cursor + header_len],
                    u64_from_usize(available),
                ) {
                    FrameParse::Complete { need, length } => {
                        let need = usize_from_u32(need);
                        prop_assert!(need <= available);
                        let frame = DatagramFrame::decode(&stream[cursor..cursor + need])
                            .unwrap_or_else(|error| {
                                panic!("C completed a frame the Rust codec refused: {error}")
                            });
                        prop_assert_eq!(frame.payload().len(), usize_from_u32(length));
                        observed.push(frame.payload().to_vec());
                        cursor += need;
                        observed_boundaries.push(cursor);
                    }
                    FrameParse::NeedMore { need, .. } => {
                        prop_assert!(usize_from_u32(need) > available);
                        break;
                    }
                    other => prop_assert!(
                        false,
                        "Rust-encoded sequence produced a C parser refusal: {other:?}"
                    ),
                }
            }
        }

        prop_assert_eq!(cursor, stream.len());
        prop_assert_eq!(&observed, &datagrams);
        prop_assert_eq!(observed_boundaries, expected_boundaries);
    }
}

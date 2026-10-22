//! `#[repr(C)]` mirrors of the `ovd_fwd` control ABI (EVMOD-CTL,
//! EVMOD-REC; ADR-0199, ADR-0202).
//!
//! The C header `kmod/ovd_fwd/include/uapi/linux/ovd_fwd.h` is the SSOT of
//! the ABI; this module mirrors it and asserts every pinned size and offset
//! at compile time. The end record's layout is pinned by the feature delta
//! (§ *Control interface*, "End record"): 32 bytes, native byte order, it
//! never leaves the machine.
//!
use overdrive_core::guest_transport::forwarder::{
    AbortCause, ForwardEnd, ForwardId, ForwardOutcome, ForwardRecordField, ForwarderError, PairSide,
};
use overdrive_core::guest_transport::wire::{
    DATAGRAM_FRAME_HEADER_LEN, DATAGRAM_FRAME_MAGIC, KernelErrno, MAX_GUEST_DATAGRAM_FRAME,
    MAX_GUEST_DATAGRAM_PAYLOAD,
};

nix::ioctl_read!(version_ioctl, 0xB9, 0x00, OvdFwdVersion);
nix::ioctl_readwrite!(attach_ioctl, 0xB9, 0x01, OvdFwdAttach);
nix::ioctl_readwrite!(watch_ioctl, 0xB9, 0x02, OvdFwdWatch);
nix::ioctl_write_ptr!(abort_ioctl, 0xB9, 0x03, u64);
nix::ioctl_none!(abort_all_ioctl, 0xB9, 0x04);
nix::ioctl_read!(inventory_ioctl, 0xB9, 0x05, OvdFwdInventory);
nix::ioctl_readwrite!(pair_state_ioctl, 0xB9, 0x06, OvdFwdPairState);

/// The control device (§ *Control interface*, "Device").
pub const FORWARDER_DEVICE_PATH: &str = "/dev/ovd_fwd";

/// The ABI version this adapter speaks, matching `OVD_FWD_ABI_VERSION`.
pub const OVD_FWD_ABI_VERSION: u32 = 1;

/// `OVD_FWD_VERSION` result: the ABI version only (MODULE-REFCOUNT,
/// 2026-10-20: no permanence flag; the module is unload-safe by reference
/// counting, ADR-0198).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OvdFwdVersion {
    /// `abi_version: u32`.
    pub abi_version: u32,
}

const _: () = assert!(std::mem::size_of::<OvdFwdVersion>() == 4);
const _: () = assert!(std::mem::offset_of!(OvdFwdVersion, abi_version) == 0);

/// `OVD_FWD_ATTACH` request and response.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OvdFwdAttach {
    /// File descriptor for half A.
    pub fd_a: i32,
    /// File descriptor for half B.
    pub fd_b: i32,
    /// Stream `0`, datagram with UDP A `1`, datagram with UDP B `2`.
    pub mode: u32,
    /// Reserved; must be zero.
    pub flags: u32,
    /// The attached pair's id, written on success.
    pub id: u64,
    /// The first offending side, written on refusal.
    pub bad_side: u8,
    /// Reserved.
    pub reserved0: u8,
    /// `ECONNABORTED` only: the ended half's errno.
    pub end_errno: u16,
    /// Reserved.
    pub reserved1: u32,
}

const _: () = assert!(std::mem::size_of::<OvdFwdAttach>() == 32);
const _: () = assert!(std::mem::offset_of!(OvdFwdAttach, fd_a) == 0);
const _: () = assert!(std::mem::offset_of!(OvdFwdAttach, fd_b) == 4);
const _: () = assert!(std::mem::offset_of!(OvdFwdAttach, mode) == 8);
const _: () = assert!(std::mem::offset_of!(OvdFwdAttach, flags) == 12);
const _: () = assert!(std::mem::offset_of!(OvdFwdAttach, id) == 16);
const _: () = assert!(std::mem::offset_of!(OvdFwdAttach, bad_side) == 24);
const _: () = assert!(std::mem::offset_of!(OvdFwdAttach, end_errno) == 26);

/// `OVD_FWD_WATCH` request and response.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OvdFwdWatch {
    /// File descriptor for the watched vsock.
    pub fd: i32,
    /// Reserved; must be zero.
    pub flags: u32,
    /// The watch's id, written on success.
    pub id: u64,
    /// The offending side, written on refusal (always A).
    pub bad_side: u8,
    /// Reserved.
    pub reserved0: u8,
    /// `ECONNABORTED` only: the ended half's errno.
    pub end_errno: u16,
    /// Reserved.
    pub reserved1: u32,
}

const _: () = assert!(std::mem::size_of::<OvdFwdWatch>() == 24);
const _: () = assert!(std::mem::offset_of!(OvdFwdWatch, fd) == 0);
const _: () = assert!(std::mem::offset_of!(OvdFwdWatch, flags) == 4);
const _: () = assert!(std::mem::offset_of!(OvdFwdWatch, id) == 8);
const _: () = assert!(std::mem::offset_of!(OvdFwdWatch, bad_side) == 16);
const _: () = assert!(std::mem::offset_of!(OvdFwdWatch, end_errno) == 18);

/// `OVD_FWD_INVENTORY` request and response.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OvdFwdInventory {
    /// Live attached pairs.
    pub live_pairs: u32,
    /// Live watches.
    pub live_watches: u32,
    /// Queued end records.
    pub queued_ends: u32,
    /// Reserved.
    pub reserved: u32,
    /// Cumulative pairs attached.
    pub attached: u64,
    /// Cumulative pairs completed.
    pub completed: u64,
    /// Cumulative peer-aborted pairs and watches.
    pub aborted_peer: u64,
    /// Cumulative socket-error-aborted pairs and watches.
    pub aborted_socket: u64,
    /// Cumulative frame-invalid-aborted pairs.
    pub aborted_frame: u64,
    /// Cumulative owner-detached pairs and watches.
    pub aborted_detached: u64,
    /// Cumulative unclassified ends.
    pub unclassified_eof: u64,
    /// Cumulative oversized datagrams consumed.
    pub dgram_oversize: u64,
}

const _: () = assert!(std::mem::size_of::<OvdFwdInventory>() == 80);
const _: () = assert!(std::mem::offset_of!(OvdFwdInventory, live_pairs) == 0);
const _: () = assert!(std::mem::offset_of!(OvdFwdInventory, live_watches) == 4);
const _: () = assert!(std::mem::offset_of!(OvdFwdInventory, queued_ends) == 8);
const _: () = assert!(std::mem::offset_of!(OvdFwdInventory, reserved) == 12);
const _: () = assert!(std::mem::offset_of!(OvdFwdInventory, attached) == 16);
const _: () = assert!(std::mem::offset_of!(OvdFwdInventory, dgram_oversize) == 72);

/// `OVD_FWD_PAIR_STATE` request and response.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OvdFwdPairState {
    /// The live pair id.
    pub id: u64,
    /// Per direction: 0 none, 1 clean, 2 abort; A-to-B then B-to-A.
    pub source_end: [u8; 2],
    /// Per direction: target write side has been shut; A-to-B then B-to-A.
    pub target_shut: [u8; 2],
    /// Reserved.
    pub reserved: [u8; 4],
    /// Bytes accepted by each target; A-to-B then B-to-A.
    pub moved: [u64; 2],
}

const _: () = assert!(std::mem::size_of::<OvdFwdPairState>() == 32);
const _: () = assert!(std::mem::offset_of!(OvdFwdPairState, id) == 0);
const _: () = assert!(std::mem::offset_of!(OvdFwdPairState, source_end) == 8);
const _: () = assert!(std::mem::offset_of!(OvdFwdPairState, target_shut) == 10);
const _: () = assert!(std::mem::offset_of!(OvdFwdPairState, moved) == 16);

/// UAPI frame magic, matching the Rust guest transport codec.
pub const OVD_FWD_FRAME_MAGIC: u32 = u32::from_be_bytes(DATAGRAM_FRAME_MAGIC);
/// UAPI frame header length.
pub const OVD_FWD_FRAME_HDR_LEN: u32 = 8;
/// Largest UAPI datagram payload.
pub const OVD_FWD_FRAME_MAX_PAYLOAD: u32 = MAX_GUEST_DATAGRAM_PAYLOAD;
/// Largest UAPI datagram frame.
pub const OVD_FWD_FRAME_MAX: u32 = MAX_GUEST_DATAGRAM_FRAME;

const _: () = assert!(OVD_FWD_FRAME_MAGIC == u32::from_be_bytes(DATAGRAM_FRAME_MAGIC));
const _: () = assert!(OVD_FWD_FRAME_HDR_LEN as usize == DATAGRAM_FRAME_HEADER_LEN);
const _: () = assert!(OVD_FWD_FRAME_MAX_PAYLOAD == MAX_GUEST_DATAGRAM_PAYLOAD);
const _: () = assert!(OVD_FWD_FRAME_MAX == MAX_GUEST_DATAGRAM_FRAME);
const _: () = assert!(OVD_FWD_FRAME_MAX == OVD_FWD_FRAME_MAX_PAYLOAD + OVD_FWD_FRAME_HDR_LEN);

/// Length of one end record; `read()` returns whole records only.
pub const END_RECORD_LEN: usize = 32;

/// Record `kind` values.
pub mod kind {
    /// Pair completed.
    pub const PAIR_COMPLETED: u8 = 1;
    /// Pair aborted.
    pub const PAIR_ABORTED: u8 = 2;
    /// Watch: guest half-close.
    pub const WATCH_HALF_CLOSE: u8 = 3;
    /// Watch: aborted.
    pub const WATCH_ABORTED: u8 = 4;
}

/// Record `cause` values (kinds 2 and 4; otherwise 0).
pub mod cause {
    /// Peer abort.
    pub const PEER_ABORT: u8 = 1;
    /// Socket error (`errno` non-zero).
    pub const SOCKET_ERROR: u8 = 2;
    /// Frame invalid.
    pub const FRAME_INVALID: u8 = 3;
    /// Detached (the owner's abort).
    pub const DETACHED: u8 = 4;
    /// Unclassified end-of-stream (an alarm).
    pub const UNCLASSIFIED: u8 = 5;
}

/// One end record as the module writes it (EVMOD-REC).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OvdFwdEndRecord {
    /// The pair or watch.
    pub id: u64,
    /// [`kind`].
    pub kind: u8,
    /// [`cause`].
    pub cause: u8,
    /// Causes 1–3 and 5: `0` half A, `1` half B; else `0`.
    pub side: u8,
    /// `0`.
    pub reserved0: u8,
    /// Cause 2: the errno, non-zero; else `0`.
    pub errno: u16,
    /// `0`.
    pub reserved1: u16,
    /// Bytes accepted by half B from half A (payload bytes in datagram mode).
    pub moved_ab: u64,
    /// Bytes accepted by half A from half B.
    pub moved_ba: u64,
}

const _: () = assert!(std::mem::size_of::<OvdFwdEndRecord>() == END_RECORD_LEN);
const _: () = assert!(std::mem::offset_of!(OvdFwdEndRecord, id) == 0);
const _: () = assert!(std::mem::offset_of!(OvdFwdEndRecord, kind) == 8);
const _: () = assert!(std::mem::offset_of!(OvdFwdEndRecord, cause) == 9);
const _: () = assert!(std::mem::offset_of!(OvdFwdEndRecord, side) == 10);
const _: () = assert!(std::mem::offset_of!(OvdFwdEndRecord, reserved0) == 11);
const _: () = assert!(std::mem::offset_of!(OvdFwdEndRecord, errno) == 12);
const _: () = assert!(std::mem::offset_of!(OvdFwdEndRecord, reserved1) == 14);
const _: () = assert!(std::mem::offset_of!(OvdFwdEndRecord, moved_ab) == 16);
const _: () = assert!(std::mem::offset_of!(OvdFwdEndRecord, moved_ba) == 24);

/// Decodes one record read from the context into the port's [`ForwardEnd`].
///
/// # Errors
///
/// [`ForwarderError::EndDecode`] naming the first field outside its pinned
/// values: `Id` (zero), `Kind` (not 1–4), `Cause` (non-zero for kinds 1 and
/// 3, or not 1–5 for kinds 2 and 4), `Side` (not 0 or 1 where a side is
/// carried, non-zero elsewhere), `Errno` (zero for cause 2, non-zero
/// elsewhere), `Reserved` (any reserved byte non-zero).
pub fn decode_end_record(bytes: &[u8; END_RECORD_LEN]) -> Result<ForwardEnd, ForwarderError> {
    let field_error = |field| ForwarderError::EndDecode { field };
    let id = u64::from_ne_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]);
    let id = ForwardId::new(id).map_err(|_| field_error(ForwardRecordField::Id))?;
    let record_kind = bytes[8];
    if !(1..=4).contains(&record_kind) {
        return Err(field_error(ForwardRecordField::Kind));
    }
    let record_cause = bytes[9];
    match record_kind {
        kind::PAIR_COMPLETED | kind::WATCH_HALF_CLOSE if record_cause != 0 => {
            return Err(field_error(ForwardRecordField::Cause));
        }
        kind::PAIR_ABORTED | kind::WATCH_ABORTED if !(1..=5).contains(&record_cause) => {
            return Err(field_error(ForwardRecordField::Cause));
        }
        _ => {}
    }
    let side = bytes[10];
    let side_is_used = matches!(
        record_cause,
        cause::PEER_ABORT | cause::SOCKET_ERROR | cause::FRAME_INVALID | cause::UNCLASSIFIED
    );
    if side > 1 || (!side_is_used && side != 0) {
        return Err(field_error(ForwardRecordField::Side));
    }
    let errno_raw = u16::from_ne_bytes([bytes[12], bytes[13]]);
    if (record_cause == cause::SOCKET_ERROR) == (errno_raw == 0) {
        return Err(field_error(ForwardRecordField::Errno));
    }
    if bytes[11] != 0 || bytes[14] != 0 || bytes[15] != 0 {
        return Err(field_error(ForwardRecordField::Reserved));
    }
    let pair_side = match side {
        0 => PairSide::A,
        1 => PairSide::B,
        _ => return Err(field_error(ForwardRecordField::Side)),
    };
    let outcome = match record_kind {
        kind::PAIR_COMPLETED => ForwardOutcome::Completed,
        kind::WATCH_HALF_CLOSE => ForwardOutcome::GuestHalfClose,
        kind::PAIR_ABORTED | kind::WATCH_ABORTED => {
            let abort_cause = match record_cause {
                cause::PEER_ABORT => AbortCause::PeerAbort { side: pair_side },
                cause::SOCKET_ERROR => AbortCause::SocketError {
                    side: pair_side,
                    errno: KernelErrno::new(errno_raw)
                        .map_err(|_| field_error(ForwardRecordField::Errno))?,
                },
                cause::FRAME_INVALID => AbortCause::FrameInvalid { side: pair_side },
                cause::DETACHED => AbortCause::Detached,
                cause::UNCLASSIFIED => AbortCause::Unclassified { side: pair_side },
                _ => return Err(field_error(ForwardRecordField::Cause)),
            };
            ForwardOutcome::Aborted { cause: abort_cause }
        }
        _ => return Err(field_error(ForwardRecordField::Kind)),
    };
    Ok(ForwardEnd {
        id,
        outcome,
        moved_ab: u64::from_ne_bytes([
            bytes[16], bytes[17], bytes[18], bytes[19], bytes[20], bytes[21], bytes[22], bytes[23],
        ]),
        moved_ba: u64::from_ne_bytes([
            bytes[24], bytes[25], bytes[26], bytes[27], bytes[28], bytes[29], bytes[30], bytes[31],
        ]),
    })
}

/// The `Version` stage's verdict over the ABI version `OVD_FWD_VERSION`
/// returned. No permanence check (MODULE-REFCOUNT, 2026-10-20).
///
/// # Errors
///
/// [`ForwarderError::AbiMismatch`] for another ABI.
pub const fn version_verdict(version: OvdFwdVersion) -> Result<(), ForwarderError> {
    if version.abi_version == OVD_FWD_ABI_VERSION {
        Ok(())
    } else {
        Err(ForwarderError::AbiMismatch {
            expected: OVD_FWD_ABI_VERSION,
            found: version.abi_version,
        })
    }
}

#[cfg(test)]
mod tests {
    //! Default-lane: the mirror's decode and the `Version` stage's verdict
    //! over hand-built inputs (no device).

    #![allow(clippy::expect_used, reason = "test assertions")]

    use overdrive_core::guest_transport::forwarder::{
        AbortCause, ForwardId, ForwardOutcome, ForwardRecordField, ForwarderError, PairSide,
    };
    use overdrive_core::guest_transport::wire::KernelErrno;

    use super::{
        END_RECORD_LEN, OVD_FWD_ABI_VERSION, OvdFwdVersion, cause, decode_end_record, kind,
        version_verdict,
    };

    /// Builds the 32 native-order bytes of § *Control interface*, "End
    /// record", field by field at their pinned offsets.
    fn record(
        id: u64,
        k: u8,
        c: u8,
        side: u8,
        errno: u16,
        ab: u64,
        ba: u64,
    ) -> [u8; END_RECORD_LEN] {
        let mut b = [0u8; END_RECORD_LEN];
        b[0..8].copy_from_slice(&id.to_ne_bytes());
        b[8] = k;
        b[9] = c;
        b[10] = side;
        b[12..14].copy_from_slice(&errno.to_ne_bytes());
        b[16..24].copy_from_slice(&ab.to_ne_bytes());
        b[24..32].copy_from_slice(&ba.to_ne_bytes());
        b
    }

    fn id(raw: u64) -> ForwardId {
        ForwardId::new(raw).expect("non-zero id")
    }

    #[test]
    fn every_pinned_record_shape_decodes_to_its_forward_end() {
        let econnreset = KernelErrno::ECONNRESET;
        let cases = [
            (record(1, kind::PAIR_COMPLETED, 0, 0, 0, 10, 20), ForwardOutcome::Completed, 10, 20),
            (
                record(2, kind::PAIR_ABORTED, cause::PEER_ABORT, 1, 0, 3, 0),
                ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { side: PairSide::B } },
                3,
                0,
            ),
            (
                record(3, kind::PAIR_ABORTED, cause::SOCKET_ERROR, 0, econnreset.get(), 0, 0),
                ForwardOutcome::Aborted {
                    cause: AbortCause::SocketError { side: PairSide::A, errno: econnreset },
                },
                0,
                0,
            ),
            (
                record(4, kind::PAIR_ABORTED, cause::FRAME_INVALID, 0, 0, 0, 0),
                ForwardOutcome::Aborted { cause: AbortCause::FrameInvalid { side: PairSide::A } },
                0,
                0,
            ),
            (
                record(5, kind::PAIR_ABORTED, cause::DETACHED, 0, 0, 7, 8),
                ForwardOutcome::Aborted { cause: AbortCause::Detached },
                7,
                8,
            ),
            (
                record(6, kind::PAIR_ABORTED, cause::UNCLASSIFIED, 1, 0, 0, 0),
                ForwardOutcome::Aborted { cause: AbortCause::Unclassified { side: PairSide::B } },
                0,
                0,
            ),
            (
                record(7, kind::WATCH_HALF_CLOSE, 0, 0, 0, 0, 0),
                ForwardOutcome::GuestHalfClose,
                0,
                0,
            ),
            (
                record(u64::MAX, kind::WATCH_ABORTED, cause::DETACHED, 0, 0, 0, 0),
                ForwardOutcome::Aborted { cause: AbortCause::Detached },
                0,
                0,
            ),
        ];
        for (bytes, outcome, ab, ba) in cases {
            let raw_id = u64::from_ne_bytes(bytes[0..8].try_into().expect("8 bytes"));
            let end = decode_end_record(&bytes).expect("a pinned record decodes");
            assert_eq!(end.id, id(raw_id));
            assert_eq!(end.outcome, outcome, "record {bytes:02x?}");
            assert_eq!((end.moved_ab, end.moved_ba), (ab, ba));
        }
    }

    #[test]
    fn a_record_outside_its_pinned_values_names_the_field() {
        let mut reserved0 = record(1, kind::PAIR_COMPLETED, 0, 0, 0, 0, 0);
        reserved0[11] = 1;
        let mut reserved1 = record(1, kind::PAIR_COMPLETED, 0, 0, 0, 0, 0);
        reserved1[15] = 1;
        let cases = [
            (record(0, kind::PAIR_COMPLETED, 0, 0, 0, 0, 0), ForwardRecordField::Id),
            (record(1, 0, 0, 0, 0, 0, 0), ForwardRecordField::Kind),
            (record(1, 5, 0, 0, 0, 0, 0), ForwardRecordField::Kind),
            (
                record(1, kind::PAIR_COMPLETED, cause::PEER_ABORT, 0, 0, 0, 0),
                ForwardRecordField::Cause,
            ),
            (record(1, kind::PAIR_ABORTED, 0, 0, 0, 0, 0), ForwardRecordField::Cause),
            (record(1, kind::PAIR_ABORTED, 6, 0, 0, 0, 0), ForwardRecordField::Cause),
            (
                record(1, kind::PAIR_ABORTED, cause::PEER_ABORT, 2, 0, 0, 0),
                ForwardRecordField::Side,
            ),
            (record(1, kind::PAIR_ABORTED, cause::DETACHED, 1, 0, 0, 0), ForwardRecordField::Side),
            (
                record(1, kind::PAIR_ABORTED, cause::SOCKET_ERROR, 0, 0, 0, 0),
                ForwardRecordField::Errno,
            ),
            (
                record(1, kind::PAIR_ABORTED, cause::PEER_ABORT, 0, 104, 0, 0),
                ForwardRecordField::Errno,
            ),
            (reserved0, ForwardRecordField::Reserved),
            (reserved1, ForwardRecordField::Reserved),
        ];
        for (bytes, field) in cases {
            match decode_end_record(&bytes) {
                Err(ForwarderError::EndDecode { field: got }) => {
                    assert_eq!(got, field, "record {bytes:02x?}");
                }
                other => {
                    panic!("record {bytes:02x?} must be refused naming {field:?}, got {other:?}")
                }
            }
        }
    }

    #[test]
    fn the_version_stage_accepts_only_this_abi() {
        assert!(version_verdict(OvdFwdVersion { abi_version: OVD_FWD_ABI_VERSION }).is_ok());
        for found in [0, OVD_FWD_ABI_VERSION + 1, u32::MAX] {
            match version_verdict(OvdFwdVersion { abi_version: found }) {
                Err(ForwarderError::AbiMismatch { expected, found: got }) => {
                    assert_eq!((expected, got), (OVD_FWD_ABI_VERSION, found));
                }
                other => panic!("ABI {found} must be refused AbiMismatch, got {other:?}"),
            }
        }
    }
}

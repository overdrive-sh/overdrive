//! `#[repr(C)]` mirrors of the `ovd_fwd` control ABI (EVMOD-CTL,
//! EVMOD-REC; ADR-0199, ADR-0202).
//!
//! The C header `kmod/ovd_fwd/include/uapi/linux/ovd_fwd.h` is the SSOT of
//! the ABI; this module mirrors it and asserts every pinned size and offset
//! at compile time. The end record's layout is pinned by the feature delta
//! (§ *Control interface*, "End record"): 32 bytes, native byte order, it
//! never leaves the machine.
//!
//! The `ioctl` argument structures of `OVD_FWD_ATTACH`, `OVD_FWD_WATCH`,
//! `OVD_FWD_ABORT`, `OVD_FWD_INVENTORY` and `OVD_FWD_PAIR_STATE`, their
//! request numbers and the ABI version number are written by DELIVER step 4
//! together with the header, with the same `const` assertions; the delta
//! pins their fields (§ *Control interface*, "Operations"), not their
//! order.
//!
//! RED scaffold until DELIVER step 4.

#![allow(
    dead_code,
    reason = "RED scaffold: the adapter's bodies consume these mirrors from DELIVER step 4"
)]

use overdrive_core::guest_transport::forwarder::{ForwardEnd, ForwarderError};

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

/// The control device (§ *Control interface*, "Device").
pub const FORWARDER_DEVICE_PATH: &str = "/dev/ovd_fwd";

/// The ABI version this adapter speaks. DELIVER step 4 sets it equal to the
/// header's `OVD_FWD_ABI_VERSION`; the probe's `Version` stage refuses any
/// other value with [`ForwarderError::AbiMismatch`].
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
pub fn decode_end_record(_bytes: &[u8; END_RECORD_LEN]) -> Result<ForwardEnd, ForwarderError> {
    panic!("{SCAFFOLD}")
}

/// The `Version` stage's verdict over the ABI version `OVD_FWD_VERSION`
/// returned. No permanence check (MODULE-REFCOUNT, 2026-10-20).
///
/// # Errors
///
/// [`ForwarderError::AbiMismatch`] for another ABI.
pub fn version_verdict(_version: OvdFwdVersion) -> Result<(), ForwarderError> {
    panic!("{SCAFFOLD}")
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
    #[ignore = "pending DELIVER step 4"]
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
    #[ignore = "pending DELIVER step 4"]
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
    #[ignore = "pending DELIVER step 4"]
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

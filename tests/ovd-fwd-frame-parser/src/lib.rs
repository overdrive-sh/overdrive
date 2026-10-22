//! Thin userspace FFI wrapper around the shipped `ovd_fwd_frame_parse`.

#![cfg(target_os = "linux")]

use std::ffi::c_int;
use std::ptr;

/// The four FRAME-PARSE outcomes and the two outputs written on every return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameParse {
    /// The queued prefix is incomplete.
    NeedMore { need: u32, length: u32 },
    /// One whole frame is queued.
    Complete { need: u32, length: u32 },
    /// The declared payload length exceeds the frame bound.
    Oversize { need: u32, length: u32 },
    /// The caller precondition or frame magic is invalid.
    Invalid { need: u32, length: u32 },
}

unsafe extern "C" {
    fn ovd_fwd_frame_parse(
        buf: *const u8,
        buf_len: u32,
        avail: u64,
        need: *mut u32,
        len: *mut u32,
    ) -> c_int;
}

/// Parse `buf` with the exact shipped C parser and return its typed outcome.
///
/// An empty slice is passed as the contract's permitted null/zero pair. Every
/// non-empty slice supplies its actual allocation pointer and exact length.
pub fn parse(buf: &[u8], avail: u64) -> FrameParse {
    let buf_len = u32::try_from(buf.len()).unwrap_or_else(|_| {
        panic!("FRAME-PARSE accepts a u32 buffer length, got {} bytes", buf.len())
    });
    let buf_ptr = if buf.is_empty() { ptr::null() } else { buf.as_ptr() };
    let mut need = u32::MAX;
    let mut length = u32::MAX;

    // SAFETY: `buf_ptr` is null iff `buf_len == 0`; otherwise it points to
    // exactly `buf_len` initialized bytes for the duration of the call. The
    // two output pointers are valid, aligned, distinct `u32` locals. The C
    // contract retains no pointer and writes both outputs before returning.
    let outcome =
        unsafe { ovd_fwd_frame_parse(buf_ptr, buf_len, avail, &raw mut need, &raw mut length) };

    match outcome {
        0 => FrameParse::NeedMore { need, length },
        1 => FrameParse::Complete { need, length },
        2 => FrameParse::Oversize { need, length },
        3 => FrameParse::Invalid { need, length },
        other => panic!("ovd_fwd_frame_parse returned unknown outcome {other}"),
    }
}

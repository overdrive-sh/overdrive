//! The only `unsafe` module of the host gateway relay.
//!
//! It owns the raw calls the relay needs: `setsockopt` for `TCP_ULP`, `TLS_TX` and
//! `TLS_RX` (arm and re-arm), `recvmsg` with a control buffer for
//! `TLS_GET_RECORD_TYPE`, `sendmsg` with `TLS_SET_RECORD_TYPE`, `splice`, `pipe2`
//! and `fcntl(F_SETPIPE_SZ / F_GETPIPE_SZ)`. Every wrapper is non-blocking and
//! retries `EINTR`; readiness is the caller's concern.
//!
//! Kernel citations (`vendors/linux` @ v7.0): `net/tls/tls_sw.c:1844-1865`
//! (record type through the control buffer, `-EIO` without one),
//! `tls_sw.c:2285-2327` (splice read, `-EINVAL` on a non-DATA record),
//! `net/tls/tls_main.c:228-268` (`TLS_SET_RECORD_TYPE`), `tls_main.c:661-687`
//! (re-arm on an armed socket), `fs/splice.c:874-888` (splice to a socket).

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use rustls::ConnectionTrafficSecrets;

const TLS_SET_RECORD_TYPE: libc::c_int = 1;
const TLS_GET_RECORD_TYPE: libc::c_int = 2;
/// TLS record content types the relay meets.
pub(super) const RECORD_ALERT: u8 = 21;
pub(super) const RECORD_HANDSHAKE: u8 = 22;
pub(super) const RECORD_DATA: u8 = 23;

/// Largest plaintext of one TLS record, with room for the control buffer's record.
pub(super) const CONTROL_RECORD_BUFFER: usize = 16_384 + 256;

/// A control buffer for `recvmsg`/`sendmsg`. `CMSG_FIRSTHDR` hands back a pointer into
/// it that is dereferenced as a `cmsghdr`, so the buffer must be aligned for
/// `cmsghdr` (`cmsg(3)`: the buffer is `cmsghdr`-aligned, `CMSG_ALIGN` pads to
/// `size_t`); 8 covers `size_t` and the `cmsghdr` fields on every supported target.
#[repr(C, align(8))]
struct ControlBuf([u8; 64]);

/// What one `recvmsg` produced.
pub(super) enum Received {
    /// `n` application octets (`0` is a TCP end of stream without close_notify).
    Data(usize),
    /// A control record of `record` type whose payload is `len` octets of the buffer.
    Control { record: u8, len: usize },
}

fn retry<T>(mut call: impl FnMut() -> isize, convert: impl FnOnce(isize) -> T) -> io::Result<T> {
    loop {
        let n = call();
        if n >= 0 {
            return Ok(convert(n));
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

fn to_usize(n: isize) -> usize {
    usize::try_from(n).unwrap_or(0)
}

/// Puts `fd` in non-blocking mode.
pub(super) fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl on a caller-owned descriptor with F_GETFL carries no pointers.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above, F_SETFL takes an integer flag word.
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// `recvmsg` with a control buffer, so a kernel-TLS socket reports the record type.
/// `peek` leaves the octets in the socket.
pub(super) fn recv_record(fd: RawFd, buf: &mut [u8], peek: bool) -> io::Result<Received> {
    let mut iov = libc::iovec { iov_base: buf.as_mut_ptr().cast(), iov_len: buf.len() };
    let mut control = ControlBuf([0u8; 64]);
    // SAFETY: an all-zero msghdr is a valid empty message header.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = (&raw mut control).cast();
    msg.msg_controllen = control.0.len() as _;
    // MSG_CMSG_CLOEXEC: any descriptor the control buffer could carry is created
    // close-on-exec, like every other descriptor the serve closure creates. The
    // flag word is spelled at the call so the cloexec lint can resolve it.
    let n = if peek {
        // SAFETY: `msg` points at live iov/control buffers of the stated lengths.
        let call = || unsafe {
            libc::recvmsg(
                fd,
                &raw mut msg,
                libc::MSG_DONTWAIT | libc::MSG_PEEK | libc::MSG_CMSG_CLOEXEC,
            )
        };
        retry(call, to_usize)?
    } else {
        // SAFETY: `msg` points at live iov/control buffers of the stated lengths.
        let call = || unsafe {
            libc::recvmsg(fd, &raw mut msg, libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC)
        };
        retry(call, to_usize)?
    };
    let mut record = RECORD_DATA;
    // SAFETY: CMSG_* walk the control buffer the kernel just filled; `ControlBuf` is
    // 8-aligned, satisfying the `cmsghdr` alignment `CMSG_FIRSTHDR` relies on.
    unsafe {
        let header = libc::CMSG_FIRSTHDR(&raw const msg);
        if !header.is_null()
            && (*header).cmsg_level == libc::SOL_TLS
            && (*header).cmsg_type == TLS_GET_RECORD_TYPE
        {
            record = *libc::CMSG_DATA(header);
        }
    }
    Ok(if record == RECORD_DATA { Received::Data(n) } else { Received::Control { record, len: n } })
}

/// `send(2)` of application octets (`MSG_NOSIGNAL`, non-blocking).
pub(super) fn send_data(fd: RawFd, data: &[u8]) -> io::Result<usize> {
    // SAFETY: the pointer/length pair is a live slice.
    retry(
        || unsafe {
            libc::send(
                fd,
                data.as_ptr().cast(),
                data.len(),
                libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT,
            )
        },
        to_usize,
    )
}

/// `sendmsg` of one record of type `record`.
pub(super) fn send_record(fd: RawFd, record: u8, data: &[u8]) -> io::Result<usize> {
    let mut iov = libc::iovec { iov_base: data.as_ptr().cast_mut().cast(), iov_len: data.len() };
    let mut control = ControlBuf([0u8; 64]);
    // SAFETY: an all-zero msghdr is a valid empty message header.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = (&raw mut control).cast();
    // SAFETY: CMSG_SPACE is a pure size computation.
    msg.msg_controllen = unsafe { libc::CMSG_SPACE(1) } as _;
    // SAFETY: the 8-aligned control buffer holds room for one one-octet cmsg
    // (`CMSG_SPACE(1)` <= 64), so `CMSG_FIRSTHDR` is a valid, aligned `cmsghdr`.
    unsafe {
        let header = libc::CMSG_FIRSTHDR(&raw const msg);
        (*header).cmsg_level = libc::SOL_TLS;
        (*header).cmsg_type = TLS_SET_RECORD_TYPE;
        (*header).cmsg_len = libc::CMSG_LEN(1) as _;
        *libc::CMSG_DATA(header) = record;
    }
    // SAFETY: `msg` points at live iov/control buffers of the stated lengths.
    retry(
        || unsafe { libc::sendmsg(fd, &raw const msg, libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT) },
        to_usize,
    )
}

/// `shutdown(SHUT_WR)`.
pub(super) fn shutdown_write(fd: RawFd) -> io::Result<()> {
    // SAFETY: shutdown on a caller-owned descriptor carries no pointers.
    let rc = unsafe { libc::shutdown(fd, libc::SHUT_WR) };
    if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// `splice(2)` between a socket and a pipe end. A negative result is the OS error.
pub(super) fn splice(fd_in: RawFd, fd_out: RawFd, len: usize, flags: u32) -> io::Result<usize> {
    // SAFETY: splice takes no pointers when both offsets are null.
    retry(
        || unsafe {
            libc::splice(fd_in, std::ptr::null_mut(), fd_out, std::ptr::null_mut(), len, flags)
        },
        to_usize,
    )
}

/// Non-blocking pipe of `requested` octets: both ends and the capacity read back.
pub(super) fn make_pipe(requested: usize) -> io::Result<(OwnedFd, OwnedFd, usize)> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is a two-element array as pipe2 requires.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: pipe2 just returned two fresh descriptors this function owns.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let size = libc::c_int::try_from(requested).unwrap_or(libc::c_int::MAX);
    // SAFETY: F_SETPIPE_SZ takes an integer argument.
    let set = unsafe { libc::fcntl(write.as_raw_fd(), libc::F_SETPIPE_SZ, size) };
    if set < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: F_GETPIPE_SZ takes no argument.
    let got = unsafe { libc::fcntl(write.as_raw_fd(), libc::F_GETPIPE_SZ) };
    if got < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((read, write, usize::try_from(got).unwrap_or(0)))
}

/// Why a crypto_info could not be installed.
pub(super) enum CryptoInfoFault {
    CipherNotSupported,
    Errno(i32),
}

fn errno_of(err: &io::Error) -> i32 {
    err.raw_os_error().unwrap_or(libc::EIO)
}

/// `setsockopt(SOL_TCP, TCP_ULP, "tls")`; the errno on failure.
pub(super) fn install_tls_ulp(fd: RawFd) -> Result<(), i32> {
    let ulp = b"tls\0";
    // SAFETY: setsockopt with a 3-octet "tls" option string.
    let rc = unsafe { libc::setsockopt(fd, libc::SOL_TCP, libc::TCP_ULP, ulp.as_ptr().cast(), 3) };
    if rc == 0 { Ok(()) } else { Err(errno_of(&io::Error::last_os_error())) }
}

/// `setsockopt(SOL_TLS, dir)` with the `tls12_crypto_info_aes_gcm_{128,256}` layout:
/// version `0x0304`, cipher id, explicit IV `iv[4..12]`, key, salt `iv[0..4]`, record
/// sequence big-endian. Arms and re-arms (rekey).
pub(super) fn set_gcm_crypto_info(
    fd: RawFd,
    dir: libc::c_int,
    sequence: u64,
    traffic: &ConnectionTrafficSecrets,
) -> Result<(), CryptoInfoFault> {
    let (cipher, key, iv): (u16, &[u8], &[u8]) = match traffic {
        ConnectionTrafficSecrets::Aes128Gcm { key, iv } => (51, key.as_ref(), iv.as_ref()),
        ConnectionTrafficSecrets::Aes256Gcm { key, iv } => (52, key.as_ref(), iv.as_ref()),
        _ => return Err(CryptoInfoFault::CipherNotSupported),
    };
    let (Some(salt), Some(explicit)) = (iv.get(0..4), iv.get(4..12)) else {
        return Err(CryptoInfoFault::CipherNotSupported);
    };
    let mut info: Vec<u8> = Vec::with_capacity(56);
    info.extend_from_slice(&0x0304u16.to_ne_bytes());
    info.extend_from_slice(&cipher.to_ne_bytes());
    info.extend_from_slice(explicit);
    info.extend_from_slice(key);
    info.extend_from_slice(salt);
    info.extend_from_slice(&sequence.to_be_bytes());
    let length = libc::socklen_t::try_from(info.len()).unwrap_or(0);
    // SAFETY: `info` holds a complete crypto_info of the stated length.
    let rc = unsafe { libc::setsockopt(fd, libc::SOL_TLS, dir, info.as_ptr().cast(), length) };
    if rc == 0 {
        Ok(())
    } else {
        Err(CryptoInfoFault::Errno(errno_of(&io::Error::last_os_error())))
    }
}

pub(super) const TLS_TX: libc::c_int = libc::TLS_TX;
pub(super) const TLS_RX: libc::c_int = libc::TLS_RX;

/// Plain `read(2)` on a not-yet-armed socket (the handshake phase).
pub(super) fn read_plain(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    // SAFETY: the pointer/length pair is a live mutable slice.
    retry(|| unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) }, to_usize)
}

/// Plain `write(2)` on a not-yet-armed socket (the handshake phase).
pub(super) fn write_plain(fd: RawFd, buf: &[u8]) -> io::Result<usize> {
    // SAFETY: the pointer/length pair is a live slice.
    retry(|| unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) }, to_usize)
}

pub(super) const SPLICE_F_NONBLOCK: u32 = libc::SPLICE_F_NONBLOCK;

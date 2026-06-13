//! kTLS arm helpers — `setsockopt(TCP_ULP "tls")` then `TLS_TX` / `TLS_RX`
//! crypto_info from rustls's `dangerous_extract_secrets`, AES-256-GCM TLS 1.3.
//!
//! The exact `tls12_crypto_info_aes_gcm_256` hand-roll proven in `findings.md` D
//! and reused verbatim across increment-f (TX) and increment-i (RX): version
//! `0x0304`, cipher id 52, the 4-byte salt = `iv[0..4]`, the 8-byte explicit IV =
//! `iv[4..12]`, `rec_seq` = the extracted record sequence big-endian. The arm
//! lands on the AGENT's leg (leg B outbound / leg C inbound), NEVER the workload's
//! socket (workload-holds-nothing, D-MTLS-9).

use std::os::fd::RawFd;

use overdrive_core::traits::mtls_enforcement::MtlsEnforcementError;
use rustls::{ConnectionTrafficSecrets, ExtractedSecrets};

/// `tls12_crypto_info_aes_gcm_256` (the in-tree UAPI shape). `#[repr(C)]`, no
/// padding — fed to `setsockopt(SOL_TLS, TLS_TX|TLS_RX)`.
#[repr(C)]
struct CryptoInfoAes256Gcm {
    version: u16,
    cipher: u16,
    iv: [u8; 8],
    key: [u8; 32],
    salt: [u8; 4],
    rec_seq: [u8; 8],
}

/// Install the TLS ULP on `fd`, then arm BOTH `TLS_TX` and `TLS_RX` from the
/// rustls-extracted secrets. Returns the RX record sequence (the value
/// `liveness`/splice reasoning needs for the kTLS-RX leg). AES-256-GCM only.
///
/// The leg is NOT a sockmap member (the forward path is an agent-light
/// `read → write_all` COPY pump into kTLS-TX, not a sockmap egress redirect), so
/// there is no sockmap-before-ULP ordering constraint — this helper does the
/// ULP+crypto arm directly.
#[allow(
    clippy::needless_pass_by_value,
    reason = "ExtractedSecrets is taken by value so the negotiated key material is moved in and dropped at the end of this scope after the arm — never lingering in the caller"
)]
pub(super) fn arm_ktls_tx_rx(
    fd: RawFd,
    secrets: ExtractedSecrets,
) -> Result<u64, MtlsEnforcementError> {
    install_ulp(fd)?;
    let rx_seq = secrets.rx.0;
    set_crypto_info(fd, libc::TLS_TX, &secrets.tx)?;
    set_crypto_info(fd, libc::TLS_RX, &secrets.rx)?;
    Ok(rx_seq)
}

/// `setsockopt(SOL_TCP, TCP_ULP, "tls")`. Any failure (the ULP already installed,
/// the kernel TLS module absent, etc.) surfaces as `KtlsArmFailed` — the failing
/// layer is the kTLS install.
fn install_ulp(fd: RawFd) -> Result<(), MtlsEnforcementError> {
    let ulp = b"tls\0";
    // SAFETY: `setsockopt` with a 3-byte "tls" option string on a real TCP fd.
    let rc = unsafe { libc::setsockopt(fd, libc::SOL_TCP, libc::TCP_ULP, ulp.as_ptr().cast(), 3) };
    if rc != 0 {
        return Err(MtlsEnforcementError::KtlsArmFailed {
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(())
}

fn set_crypto_info(
    fd: RawFd,
    dir: libc::c_int,
    sec: &(u64, ConnectionTrafficSecrets),
) -> Result<(), MtlsEnforcementError> {
    let (seq, traffic) = sec;
    let ConnectionTrafficSecrets::Aes256Gcm { key, iv } = traffic else {
        return Err(MtlsEnforcementError::KtlsArmFailed {
            source: std::io::Error::other("kTLS arm requires AES-256-GCM TLS 1.3"),
        });
    };
    let ivb = iv.as_ref();
    let mut info = CryptoInfoAes256Gcm {
        version: 0x0304,
        cipher: 52,
        iv: [0; 8],
        key: [0; 32],
        salt: [0; 4],
        rec_seq: seq.to_be_bytes(),
    };
    info.key.copy_from_slice(key.as_ref());
    info.salt.copy_from_slice(&ivb[0..4]);
    info.iv.copy_from_slice(&ivb[4..12]);
    // SAFETY: `info` is a `#[repr(C)]` struct matching the in-tree
    // `tls12_crypto_info_aes_gcm_256` layout; `setsockopt` reads `size_of::<_>()`
    // bytes from it.
    let rc = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_TLS,
            dir,
            std::ptr::from_ref(&info).cast(),
            // The crypto_info struct is 56 bytes — a compile-time constant far
            // below u32::MAX, so the usize→socklen_t cast cannot truncate.
            #[allow(clippy::cast_possible_truncation)]
            {
                std::mem::size_of::<CryptoInfoAes256Gcm>() as libc::socklen_t
            },
        )
    };
    if rc != 0 {
        return Err(MtlsEnforcementError::KtlsArmFailed {
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(())
}

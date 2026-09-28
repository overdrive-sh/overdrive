//! Spike B host-side shared code (GH #303, increment-d). Throwaway probe.
//!
//! The vsock relay protocol between the Unikraft guest hook (`lib-mtlsguard`)
//! and the host relay stub. One frame = `type:u8 | len:u32 BE | payload`.
//!
//! | type | name      | dir          | payload                                         |
//! |------|-----------|--------------|-------------------------------------------------|
//! | 0x01 | OPEN      | guest → host | family:u8 (=4) · ipv4:[u8;4] · port:u16 BE      |
//! | 0x02 | TO_PEER   | host → guest | TLS bytes the guest must write to the TCP peer  |
//! | 0x03 | NEED_PEER | host → guest | empty: read ONE TLS record from the peer, reply 0x04 |
//! | 0x04 | FROM_PEER | guest → host | bytes read from the TCP peer (len 0 = peer EOF) |
//! | 0x05 | SECRETS   | host → guest | see [`encode_secrets`]                          |
//! | 0x06 | DENY      | host → guest | UTF-8 reason                                    |
//! | 0x07 | ERROR     | host → guest | UTF-8 reason                                    |
//! | 0x08 | ACCEPT    | guest → host | family:u8 (=4) · local ip · local port · remote ip · remote port (13 B) |
//!
//! OPEN makes the host the TLS client (guest connect()); ACCEPT makes it the
//! TLS server with the workload's server SVID (guest accept()). NEED_PEER is
//! answered with exactly ONE TLS record.
//!
//! The protocol is host-driven and lock-step: the guest only touches the TCP
//! peer when the host tells it to, so it never reads past the handshake.

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

pub const T_OPEN: u8 = 0x01;
pub const T_TO_PEER: u8 = 0x02;
pub const T_NEED_PEER: u8 = 0x03;
pub const T_FROM_PEER: u8 = 0x04;
pub const T_SECRETS: u8 = 0x05;
pub const T_DENY: u8 = 0x06;
pub const T_ERROR: u8 = 0x07;
pub const T_ACCEPT: u8 = 0x08;

pub const MAX_FRAME: usize = 64 * 1024;

pub fn type_name(t: u8) -> &'static str {
    match t {
        T_OPEN => "OPEN",
        T_TO_PEER => "TO_PEER",
        T_NEED_PEER => "NEED_PEER",
        T_FROM_PEER => "FROM_PEER",
        T_SECRETS => "SECRETS",
        T_DENY => "DENY",
        T_ERROR => "ERROR",
        T_ACCEPT => "ACCEPT",
        _ => "UNKNOWN",
    }
}

pub fn write_frame(w: &mut impl Write, t: u8, payload: &[u8]) -> io::Result<usize> {
    let mut buf = Vec::with_capacity(5 + payload.len());
    buf.push(t);
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf)?;
    w.flush()?;
    Ok(buf.len())
}

pub fn read_frame(r: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut hdr = [0u8; 5];
    r.read_exact(&mut hdr)?;
    let len = u32::from_be_bytes([hdr[1], hdr[2], hdr[3], hdr[4]]) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("frame too large: {len}")));
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok((hdr[0], payload))
}

/// TLS_AES_128_GCM_SHA256 only, TLS 1.3 only — pinned on both host ends.
pub fn provider() -> Arc<CryptoProvider> {
    let base = rustls::crypto::ring::default_provider();
    Arc::new(CryptoProvider {
        cipher_suites: vec![rustls::crypto::ring::cipher_suite::TLS13_AES_128_GCM_SHA256],
        ..base
    })
}

pub fn load_certs(path: &str) -> Vec<CertificateDer<'static>> {
    CertificateDer::pem_file_iter(path)
        .unwrap_or_else(|e| panic!("open {path}: {e}"))
        .map(|c| c.unwrap_or_else(|e| panic!("parse {path}: {e}")))
        .collect()
}

pub fn load_key(path: &str) -> PrivateKeyDer<'static> {
    PrivateKeyDer::from_pem_file(path).unwrap_or_else(|e| panic!("key {path}: {e}"))
}

/// The single `spiffe://` URI SAN of a certificate, if any.
pub fn spiffe_id(cert: &[u8]) -> Option<String> {
    let (_, x) = x509_parser::parse_x509_certificate(cert).ok()?;
    let san = x.subject_alternative_name().ok()??;
    san.value.general_names.iter().find_map(|g| match g {
        x509_parser::extensions::GeneralName::URI(u) if u.starts_with("spiffe://") => {
            Some((*u).to_string())
        }
        _ => None,
    })
}

/// Seconds since the runner's T0 (passed on the command line as epoch seconds).
pub fn since(t0: f64) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64();
    format!("t+{:.6}s", now - t0)
}

/// Printable, escaped rendering of bytes for logs.
pub fn show(b: &[u8]) -> String {
    b.escape_ascii().to_string()
}

/// Count non-overlapping occurrences of `needle` in `hay`.
pub fn count(hay: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() || hay.len() < needle.len() {
        return 0;
    }
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

/// SECRETS payload:
/// `suite:u16 BE (0x1301)`
/// `tx_seq:u64 BE · tx_key_len:u8 · tx_key · tx_iv[12]`
/// `rx_seq:u64 BE · rx_key_len:u8 · rx_key · rx_iv[12]`
/// `peer_id_len:u16 BE · peer_id (UTF-8, informational)`
pub fn encode_secrets(
    tx_seq: u64,
    tx_key: &[u8],
    tx_iv: &[u8],
    rx_seq: u64,
    rx_key: &[u8],
    rx_iv: &[u8],
    peer_id: &str,
) -> Vec<u8> {
    let mut p = Vec::with_capacity(2 + 2 * (8 + 1 + 16 + 12) + 2 + peer_id.len());
    p.extend_from_slice(&0x1301u16.to_be_bytes());
    for (seq, key, iv) in [(tx_seq, tx_key, tx_iv), (rx_seq, rx_key, rx_iv)] {
        p.extend_from_slice(&seq.to_be_bytes());
        p.push(key.len() as u8);
        p.extend_from_slice(key);
        p.extend_from_slice(iv);
    }
    p.extend_from_slice(&(peer_id.len() as u16).to_be_bytes());
    p.extend_from_slice(peer_id.as_bytes());
    p
}

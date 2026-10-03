//! Spike B2 host-side shared code (GH #303, increment-e). Throwaway probe.
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
//! | 0x05 | SECRETS   | host → guest | v2, see [`encode_secrets_v2`]                   |
//! | 0x06 | DENY      | host → guest | UTF-8 reason                                    |
//! | 0x07 | ERROR     | host → guest | UTF-8 reason                                    |
//! | 0x08 | ACCEPT    | guest → host | family:u8 (=4) · local ip · local port · remote ip · remote port (13 B) |
//!
//! Changed versus Spike B (increment-d): SECRETS now carries the TLS 1.3
//! application traffic SECRET of each direction (so the guest can run the
//! KeyUpdate key schedule itself), plus rustls' own key/IV for a cross-check.
//! The SVID private key still never crosses.

use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::KeyLog;

pub const T_OPEN: u8 = 0x01;
pub const T_TO_PEER: u8 = 0x02;
pub const T_NEED_PEER: u8 = 0x03;
pub const T_FROM_PEER: u8 = 0x04;
pub const T_SECRETS: u8 = 0x05;
pub const T_DENY: u8 = 0x06;
pub const T_ERROR: u8 = 0x07;
pub const T_ACCEPT: u8 = 0x08;

pub const MAX_FRAME: usize = 64 * 1024;

/// Test-topology addresses (increment-e uses its own /24 on its own tap).
pub const HOST_IP: [u8; 4] = [192, 168, 204, 1];
pub const GUEST_IP: [u8; 4] = [192, 168, 204, 2];

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

/// TLS_AES_128_GCM_SHA256 only, TLS 1.3 only — pinned on every host end.
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

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Count non-overlapping occurrences of `needle` in `hay`.
pub fn count(hay: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() || hay.len() < needle.len() {
        return 0;
    }
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

/// FNV-1a 32 over a byte string (bulk-payload checksum shared with the guest).
pub fn fnv1a32(b: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for x in b {
        h ^= u32::from(*x);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Captures the TLS 1.3 application traffic secrets rustls hands to a
/// [`KeyLog`] (`CLIENT_TRAFFIC_SECRET_0` / `SERVER_TRAFFIC_SECRET_0`). One
/// instance per connection, so no client_random bookkeeping is needed.
#[derive(Debug, Default)]
pub struct TrafficSecrets(Mutex<Vec<(String, Vec<u8>)>>);

impl TrafficSecrets {
    pub fn get(&self, label: &str) -> Option<Vec<u8>> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .find(|(l, _)| l == label)
            .map(|(_, s)| s.clone())
    }
}

impl KeyLog for TrafficSecrets {
    fn will_log(&self, label: &str) -> bool {
        label == "CLIENT_TRAFFIC_SECRET_0" || label == "SERVER_TRAFFIC_SECRET_0"
    }
    fn log(&self, label: &str, _client_random: &[u8], secret: &[u8]) {
        self.0.lock().unwrap().push((label.to_string(), secret.to_vec()));
    }
}

/// One direction of SECRETS v2.
pub struct DirSecrets<'a> {
    pub seq: u64,
    pub secret: &'a [u8],
    pub key: &'a [u8],
    pub iv: &'a [u8],
}

/// SECRETS v2 payload:
/// `suite:u16 BE (0x1301)`
/// per direction (tx then rx):
/// `seq:u64 BE · secret_len:u8 (32) · secret · key_len:u8 (16) · key · iv[12]`
/// then `peer_id_len:u16 BE · peer_id (UTF-8, informational)`.
pub fn encode_secrets_v2(tx: &DirSecrets<'_>, rx: &DirSecrets<'_>, peer_id: &str) -> Vec<u8> {
    let mut p = Vec::with_capacity(2 + 2 * (8 + 1 + 32 + 1 + 16 + 12) + 2 + peer_id.len());
    p.extend_from_slice(&0x1301u16.to_be_bytes());
    for d in [tx, rx] {
        p.extend_from_slice(&d.seq.to_be_bytes());
        p.push(d.secret.len() as u8);
        p.extend_from_slice(d.secret);
        p.push(d.key.len() as u8);
        p.extend_from_slice(d.key);
        p.extend_from_slice(d.iv);
    }
    p.extend_from_slice(&(peer_id.len() as u16).to_be_bytes());
    p.extend_from_slice(peer_id.as_bytes());
    p
}

// ---- TLS 1.3 record layer helpers for the "raw" peer ports ----------------

struct HkdfLen(usize);
impl ring::hkdf::KeyType for HkdfLen {
    fn len(&self) -> usize {
        self.0
    }
}

/// RFC 8446 §7.1 HKDF-Expand-Label with SHA-256 and an empty context.
pub fn hkdf_expand_label(secret: &[u8], label: &str, len: usize) -> Vec<u8> {
    let full = format!("tls13 {label}");
    let mut info = Vec::with_capacity(4 + full.len());
    info.extend_from_slice(&(len as u16).to_be_bytes());
    info.push(full.len() as u8);
    info.extend_from_slice(full.as_bytes());
    info.push(0);
    let prk = ring::hkdf::Prk::new_less_safe(ring::hkdf::HKDF_SHA256, secret);
    let info_parts = [&info[..]];
    let mut out = vec![0u8; len];
    prk.expand(&info_parts, HkdfLen(len))
        .expect("hkdf expand")
        .fill(&mut out)
        .expect("hkdf fill");
    out
}

/// One direction of a hand-rolled TLS 1.3 AES-128-GCM record layer.
pub struct RawDir {
    key: ring::aead::LessSafeKey,
    iv: [u8; 12],
    pub seq: u64,
    pub secret: Vec<u8>,
    pub generation: u32,
}

impl RawDir {
    pub fn new(key: &[u8], iv: &[u8], seq: u64, secret: Vec<u8>) -> Self {
        let mut ivb = [0u8; 12];
        ivb.copy_from_slice(iv);
        Self {
            key: ring::aead::LessSafeKey::new(
                ring::aead::UnboundKey::new(&ring::aead::AES_128_GCM, key).expect("aes key"),
            ),
            iv: ivb,
            seq,
            secret,
            generation: 0,
        }
    }

    fn nonce(&self) -> ring::aead::Nonce {
        let mut n = self.iv;
        for (i, b) in self.seq.to_be_bytes().iter().enumerate() {
            n[4 + i] ^= b;
        }
        ring::aead::Nonce::assume_unique_for_key(n)
    }

    /// Encrypt `content` with inner type `inner` as one record (outer 0x17).
    pub fn seal(&mut self, inner: u8, content: &[u8]) -> Vec<u8> {
        let mut buf = content.to_vec();
        buf.push(inner);
        let len = buf.len() + 16;
        let hdr = [0x17, 0x03, 0x03, (len >> 8) as u8, len as u8];
        self.key
            .seal_in_place_append_tag(self.nonce(), ring::aead::Aad::from(hdr), &mut buf)
            .expect("seal");
        self.seq += 1;
        let mut rec = hdr.to_vec();
        rec.extend_from_slice(&buf);
        rec
    }

    /// Decrypt one complete record; returns (inner type, content).
    pub fn open(&mut self, rec: &[u8]) -> Result<(u8, Vec<u8>), String> {
        if rec.len() < 5 + 17 || rec[0] != 0x17 {
            return Err(format!("bad record header {:02x?}", &rec[..rec.len().min(5)]));
        }
        let mut hdr = [0u8; 5];
        hdr.copy_from_slice(&rec[..5]);
        let mut body = rec[5..].to_vec();
        let n = self.nonce();
        let pt = self
            .key
            .open_in_place(n, ring::aead::Aad::from(hdr), &mut body)
            .map_err(|_| format!("AEAD open failed at seq {}", self.seq))?;
        self.seq += 1;
        let mut end = pt.len();
        while end > 0 && pt[end - 1] == 0 {
            end -= 1;
        }
        if end == 0 {
            return Err("record without content type".into());
        }
        Ok((pt[end - 1], pt[..end - 1].to_vec()))
    }

    /// RFC 8446 §7.2: next application traffic secret, key and IV; seq = 0.
    pub fn update(&mut self) {
        let next = hkdf_expand_label(&self.secret, "traffic upd", 32);
        let key = hkdf_expand_label(&next, "key", 16);
        let iv = hkdf_expand_label(&next, "iv", 12);
        self.key = ring::aead::LessSafeKey::new(
            ring::aead::UnboundKey::new(&ring::aead::AES_128_GCM, &key).expect("aes key"),
        );
        self.iv.copy_from_slice(&iv);
        self.secret = next;
        self.seq = 0;
        self.generation += 1;
    }
}

/// Read exactly one TLS record (header + body) from a byte stream.
pub fn read_record(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut hdr = [0u8; 5];
    r.read_exact(&mut hdr)?;
    let len = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
    let mut rec = hdr.to_vec();
    rec.resize(5 + len, 0);
    r.read_exact(&mut rec[5..])?;
    Ok(rec)
}

/// A TLS 1.3 handshake message: type(1) · length(3) · body.
pub fn hs_msg(t: u8, body: &[u8]) -> Vec<u8> {
    let mut m = vec![t, (body.len() >> 16) as u8, (body.len() >> 8) as u8, body.len() as u8];
    m.extend_from_slice(body);
    m
}

/// A syntactically valid NewSessionTicket (RFC 8446 §4.6.1) with a 32-byte
/// opaque ticket and no extensions. `n` varies the nonce and ticket bytes.
pub fn fake_nst(n: u8) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&7200u32.to_be_bytes()); // ticket_lifetime
    b.extend_from_slice(&(0x1122_3300u32 | u32::from(n)).to_be_bytes()); // ticket_age_add
    b.push(8); // ticket_nonce<0..255>
    b.extend_from_slice(&[n; 8]);
    b.extend_from_slice(&32u16.to_be_bytes()); // ticket<1..2^16-1>
    b.extend_from_slice(&[0xA0 | (n & 0x0f); 32]);
    b.extend_from_slice(&0u16.to_be_bytes()); // extensions<0..2^16-2>
    hs_msg(4, &b)
}

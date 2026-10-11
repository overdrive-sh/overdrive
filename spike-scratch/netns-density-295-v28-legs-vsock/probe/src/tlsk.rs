//! rustls TLS1.3 AES-256-GCM handshake then kTLS TX+RX arm (hand-rolled like
//! crates/overdrive-dataplane/src/mtls/ktls.rs). Used by BOTH the relay (server, "leg C") and the
//! test client (the originating agent's "leg B" stand-in).
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, ConnectionTrafficSecrets, ExtractedSecrets, RootCertStore, ServerConfig, ServerConnection};
use std::io;
use std::mem::ManuallyDrop;
use std::net::TcpStream;
use std::os::fd::{FromRawFd, RawFd};
use std::sync::Arc;
use std::time::Duration;

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    let mut p = rustls::crypto::ring::default_provider();
    p.cipher_suites = vec![rustls::crypto::ring::cipher_suite::TLS13_AES_256_GCM_SHA384];
    Arc::new(p)
}
pub fn server_cfg(dir: &str) -> Arc<ServerConfig> {
    let certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_file_iter(format!("{dir}/cert.pem")).unwrap().map(|c| c.unwrap()).collect();
    let key = PrivateKeyDer::from_pem_file(format!("{dir}/key.pem")).unwrap();
    let mut c = ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();
    c.enable_secret_extraction = true;
    c.send_tls13_tickets = 0;
    Arc::new(c)
}
pub fn client_cfg(dir: &str) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    for c in CertificateDer::pem_file_iter(format!("{dir}/ca.pem")).unwrap() {
        roots.add(c.unwrap()).unwrap();
    }
    let mut c = ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    c.enable_secret_extraction = true;
    Arc::new(c)
}

/// Server handshake on `fd` (borrowed), returns the extracted secrets + any early plaintext.
pub fn server_handshake(fd: RawFd, cfg: Arc<ServerConfig>) -> io::Result<(ExtractedSecrets, Vec<u8>)> {
    let mut tcp = ManuallyDrop::new(unsafe { TcpStream::from_raw_fd(fd) });
    tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut conn = ServerConnection::new(cfg).map_err(io::Error::other)?;
    while conn.is_handshaking() {
        conn.complete_io(&mut *tcp)?;
    }
    while conn.wants_write() {
        conn.write_tls(&mut *tcp)?;
    }
    let mut early = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        match io::Read::read(&mut conn.reader(), &mut tmp) {
            Ok(0) => break,
            Ok(n) => early.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
    }
    tcp.set_read_timeout(None)?;
    let s = conn.dangerous_extract_secrets().map_err(io::Error::other)?;
    Ok((s, early))
}
pub fn client_handshake(fd: RawFd, cfg: Arc<ClientConfig>) -> io::Result<ExtractedSecrets> {
    let mut tcp = ManuallyDrop::new(unsafe { TcpStream::from_raw_fd(fd) });
    tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
    let name = ServerName::try_from("legs.test").unwrap();
    let mut conn = ClientConnection::new(cfg, name).map_err(io::Error::other)?;
    while conn.is_handshaking() {
        conn.complete_io(&mut *tcp)?;
    }
    while conn.wants_write() {
        conn.write_tls(&mut *tcp)?;
    }
    tcp.set_read_timeout(None)?;
    conn.dangerous_extract_secrets().map_err(io::Error::other)
}

#[repr(C)]
struct CryptoInfoAes256Gcm {
    version: u16,
    cipher: u16,
    iv: [u8; 8],
    key: [u8; 32],
    salt: [u8; 4],
    rec_seq: [u8; 8],
}
pub const SOL_TLS: i32 = 282;
const TLS_TX: i32 = 1;
const TLS_RX: i32 = 2;

pub fn arm_ktls(fd: RawFd, s: ExtractedSecrets) -> io::Result<()> {
    let rc = unsafe { libc::setsockopt(fd, libc::SOL_TCP, libc::TCP_ULP, b"tls\0".as_ptr().cast(), 3) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    set_ci(fd, TLS_TX, &s.tx)?;
    set_ci(fd, TLS_RX, &s.rx)?;
    Ok(())
}
fn set_ci(fd: RawFd, dir: i32, sec: &(u64, ConnectionTrafficSecrets)) -> io::Result<()> {
    let ConnectionTrafficSecrets::Aes256Gcm { key, iv } = &sec.1 else {
        return Err(io::Error::other("need AES-256-GCM"));
    };
    let mut ci = CryptoInfoAes256Gcm { version: 0x0304, cipher: 52, iv: [0; 8], key: [0; 32], salt: [0; 4], rec_seq: sec.0.to_be_bytes() };
    ci.key.copy_from_slice(key.as_ref());
    ci.salt.copy_from_slice(&iv.as_ref()[0..4]);
    ci.iv.copy_from_slice(&iv.as_ref()[4..12]);
    let rc = unsafe { libc::setsockopt(fd, SOL_TLS, dir, (&ci as *const CryptoInfoAes256Gcm).cast(), std::mem::size_of::<CryptoInfoAes256Gcm>() as u32) };
    if rc != 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// Send a TLS record of `typ` (21 = alert) through kTLS TX via sendmsg + TLS_SET_RECORD_TYPE cmsg.
pub fn send_record(fd: RawFd, typ: u8, data: &[u8]) -> io::Result<()> {
    let mut cbuf = [0u8; 32];
    let mut iov = libc::iovec { iov_base: data.as_ptr() as *mut _, iov_len: data.len() };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cbuf.as_mut_ptr().cast();
    msg.msg_controllen = unsafe { libc::CMSG_SPACE(1) } as usize;
    unsafe {
        let c = libc::CMSG_FIRSTHDR(&msg);
        (*c).cmsg_level = SOL_TLS;
        (*c).cmsg_type = 1; // TLS_SET_RECORD_TYPE
        (*c).cmsg_len = libc::CMSG_LEN(1) as usize;
        *libc::CMSG_DATA(c) = typ;
        if libc::sendmsg(fd, &msg, libc::MSG_NOSIGNAL) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
pub fn send_close_notify(fd: RawFd) -> io::Result<()> {
    send_record(fd, 21, &[1, 0])
}
/// Receive one record with its type via recvmsg + TLS_GET_RECORD_TYPE. Returns (type, bytes).
pub fn recv_record(fd: RawFd, buf: &mut [u8]) -> io::Result<(u8, usize)> {
    let mut cbuf = [0u8; 64];
    let mut iov = libc::iovec { iov_base: buf.as_mut_ptr().cast(), iov_len: buf.len() };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cbuf.as_mut_ptr().cast();
    msg.msg_controllen = cbuf.len();
    let n = unsafe { libc::recvmsg(fd, &mut msg, 0) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut typ = 23u8;
    unsafe {
        let c = libc::CMSG_FIRSTHDR(&msg);
        if !c.is_null() && (*c).cmsg_level == SOL_TLS && (*c).cmsg_type == 2 {
            typ = *libc::CMSG_DATA(c);
        }
    }
    Ok((typ, n as usize))
}

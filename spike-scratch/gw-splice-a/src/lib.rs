//! gw-splice-a shared helpers: rustls (unbuffered, kernel-connection) handshake drivers,
//! kTLS arm/rekey, record-aware recvmsg/sendmsg, splice wrappers, mini HTTP/1.1.
//! Throwaway spike code (.claude/rules/spike.md).
#![allow(clippy::all)]

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsRawFd, RawFd};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{ClientConnectionData, UnbufferedClientConnection};
use rustls::kernel::KernelConnection;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::{ServerConnectionData, UnbufferedServerConnection};
use rustls::unbuffered::{ConnectionState, EncodeError};
use rustls::{
    ClientConfig, ConnectionTrafficSecrets, DigitallySignedStruct, ExtractedSecrets, ServerConfig,
    SignatureScheme,
};

pub const TLS_SET_RECORD_TYPE: libc::c_int = 1;
pub const TLS_GET_RECORD_TYPE: libc::c_int = 2;
pub const REC_ALERT: u8 = 21;
pub const REC_HANDSHAKE: u8 = 22;
pub const REC_DATA: u8 = 23;

pub fn uname() -> String {
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    unsafe { libc::uname(&mut u) };
    let c = unsafe { std::ffi::CStr::from_ptr(u.release.as_ptr()) };
    c.to_string_lossy().into_owned()
}
pub fn now_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()
}
pub fn pat(i: usize) -> u8 {
    (i % 251) as u8
}
pub fn pattern(off: usize, len: usize) -> Vec<u8> {
    (off..off + len).map(pat).collect()
}
pub fn errno_name(e: &io::Error) -> String {
    format!("{}(errno {})", e, e.raw_os_error().unwrap_or(-1))
}

// ---------------------------------------------------------------- key log
#[derive(Debug, Default)]
pub struct SecretLog(pub Mutex<HashMap<String, Vec<u8>>>);
impl rustls::KeyLog for SecretLog {
    fn log(&self, label: &str, _cr: &[u8], secret: &[u8]) {
        self.0.lock().unwrap().insert(label.to_string(), secret.to_vec());
    }
}

// ---------------------------------------------------------------- configs
static CERT: OnceLock<(Vec<u8>, Vec<u8>)> = OnceLock::new();
fn cert() -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let (c, k) = CERT.get_or_init(|| {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["localhost".to_string()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        (cert.der().to_vec(), key.serialize_der())
    });
    (
        CertificateDer::from(c.clone()),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(k.clone())),
    )
}
fn provider(suite_256: bool) -> Arc<rustls::crypto::CryptoProvider> {
    let mut p = rustls::crypto::ring::default_provider();
    p.cipher_suites = vec![if suite_256 {
        rustls::crypto::ring::cipher_suite::TLS13_AES_256_GCM_SHA384
    } else {
        rustls::crypto::ring::cipher_suite::TLS13_AES_128_GCM_SHA256
    }];
    Arc::new(p)
}
/// `tickets`: number of TLS1.3 NewSessionTickets the server sends post-handshake.
pub fn server_config(log: Option<Arc<SecretLog>>, tickets: usize, suite_256: bool) -> Arc<ServerConfig> {
    let (c, k) = cert();
    let mut cfg = ServerConfig::builder_with_provider(provider(suite_256))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![c], k)
        .unwrap();
    cfg.send_tls13_tickets = tickets;
    cfg.enable_secret_extraction = true;
    if let Some(l) = log {
        cfg.key_log = l;
    }
    Arc::new(cfg)
}
#[derive(Debug)]
struct NoVerify;
impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes()
    }
}
pub fn client_config(suite_256: bool) -> Arc<ClientConfig> {
    let mut cfg = ClientConfig::builder_with_provider(provider(suite_256))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerify))
        .with_no_client_auth();
    cfg.enable_secret_extraction = true;
    Arc::new(cfg)
}

// ---------------------------------------------------------------- unbuffered handshake drivers
/// Read exactly ONE TLS record (5-byte header + body) from `tcp` and append to `incoming`.
/// Record-at-a-time reads guarantee userspace never over-reads past the handshake.
fn read_one_record(tcp: &mut TcpStream, incoming: &mut Vec<u8>) -> io::Result<()> {
    let mut h = [0u8; 5];
    tcp.read_exact(&mut h)?;
    let len = u16::from_be_bytes([h[3], h[4]]) as usize;
    incoming.extend_from_slice(&h);
    let at = incoming.len();
    incoming.resize(at + len, 0);
    tcp.read_exact(&mut incoming[at..])?;
    Ok(())
}

enum Act { Read, Done, Nothing }

macro_rules! drive {
    ($tcp:expr, $conn:expr) => {{
        let mut incoming: Vec<u8> = Vec::new();
        let mut outgoing = vec![0u8; 4096];
        let mut out_used = 0usize;
        let app: Vec<u8> = Vec::new();
        loop {
            let discard;
            let act;
            {
                let st = $conn.process_tls_records(&mut incoming);
                discard = st.discard;
                act = match st.state.map_err(|e| io::Error::other(format!("tls: {e}")))? {
                    ConnectionState::BlockedHandshake => Act::Read,
                    ConnectionState::EncodeTlsData(mut s) => {
                        loop {
                            match s.encode(&mut outgoing[out_used..]) {
                                Ok(n) => { out_used += n; break; }
                                Err(EncodeError::InsufficientSize(e)) => outgoing.resize(out_used + e.required_size, 0),
                                Err(EncodeError::AlreadyEncoded) => break,
                            }
                        }
                        Act::Nothing
                    }
                    ConnectionState::TransmitTlsData(s) => {
                        $tcp.write_all(&outgoing[..out_used])?;
                        out_used = 0;
                        s.done();
                        Act::Nothing
                    }
                    ConnectionState::WriteTraffic(_) => Act::Done,
                    _other => return Err(io::Error::other("unexpected unbuffered state during handshake")),
                };
            }
            incoming.drain(..discard);
            match act {
                Act::Read => read_one_record($tcp, &mut incoming)?,
                Act::Done => break,
                Act::Nothing => {}
            }
        }
        (incoming, app)
    }};
}

pub struct Established<D> {
    pub secrets: ExtractedSecrets,
    pub kconn: KernelConnection<D>,
    /// bytes read from the socket but not yet consumed by rustls (MUST be empty for kTLS arm)
    pub leftover_ciphertext: usize,
    /// plaintext rustls already decrypted (application data coalesced with client Finished)
    pub early_plaintext: Vec<u8>,
}

pub fn handshake_server(tcp: &mut TcpStream, cfg: Arc<ServerConfig>) -> io::Result<Established<ServerConnectionData>> {
    let mut conn = UnbufferedServerConnection::new(cfg).map_err(io::Error::other)?;
    let (incoming, app) = drive!(tcp, conn);
    let (secrets, kconn) = conn.dangerous_into_kernel_connection().map_err(|e| io::Error::other(format!("into_kernel: {e}")))?;
    Ok(Established { secrets, kconn, leftover_ciphertext: incoming.len(), early_plaintext: app })
}
pub fn handshake_client(tcp: &mut TcpStream, cfg: Arc<ClientConfig>) -> io::Result<Established<ClientConnectionData>> {
    let mut conn = UnbufferedClientConnection::new(cfg, ServerName::try_from("localhost").unwrap()).map_err(io::Error::other)?;
    let (incoming, app) = drive!(tcp, conn);
    let (secrets, kconn) = conn.dangerous_into_kernel_connection().map_err(|e| io::Error::other(format!("into_kernel: {e}")))?;
    Ok(Established { secrets, kconn, leftover_ciphertext: incoming.len(), early_plaintext: app })
}

// ---------------------------------------------------------------- kTLS
pub fn ktls_ulp(fd: RawFd) -> io::Result<()> {
    let rc = unsafe { libc::setsockopt(fd, libc::SOL_TCP, libc::TCP_ULP, b"tls\0".as_ptr().cast(), 3) };
    if rc != 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}
/// dir = libc::TLS_TX / libc::TLS_RX. Also used for rekey (same call on an armed direction).
pub fn ktls_set(fd: RawFd, dir: libc::c_int, seq: u64, t: &ConnectionTrafficSecrets) -> io::Result<()> {
    let (cipher, key, iv): (u16, &[u8], &[u8]) = match t {
        ConnectionTrafficSecrets::Aes128Gcm { key, iv } => (51, key.as_ref(), iv.as_ref()),
        ConnectionTrafficSecrets::Aes256Gcm { key, iv } => (52, key.as_ref(), iv.as_ref()),
        _ => return Err(io::Error::other("unsupported cipher")),
    };
    let mut b: Vec<u8> = Vec::new();
    b.extend_from_slice(&0x0304u16.to_ne_bytes());
    b.extend_from_slice(&cipher.to_ne_bytes());
    b.extend_from_slice(&iv[4..12]); // iv (explicit part)
    b.extend_from_slice(key);
    b.extend_from_slice(&iv[0..4]); // salt
    b.extend_from_slice(&seq.to_be_bytes());
    let rc = unsafe { libc::setsockopt(fd, libc::SOL_TLS, dir, b.as_ptr().cast(), b.len() as libc::socklen_t) };
    if rc != 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}
pub fn arm_both(fd: RawFd, s: &ExtractedSecrets) -> io::Result<()> {
    ktls_ulp(fd)?;
    ktls_set(fd, libc::TLS_TX, s.tx.0, &s.tx.1)?;
    ktls_set(fd, libc::TLS_RX, s.rx.0, &s.rx.1)
}

/// recvmsg returning (record_type, n). Retries EINTR only.
pub fn recv_rec(fd: RawFd, buf: &mut [u8], flags: i32) -> io::Result<(u8, usize)> {
    let mut iov = libc::iovec { iov_base: buf.as_mut_ptr().cast(), iov_len: buf.len() };
    let mut cbuf = [0u8; 64];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cbuf.as_mut_ptr().cast();
    msg.msg_controllen = cbuf.len() as _;
    loop {
        let n = unsafe { libc::recvmsg(fd, &mut msg, flags) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINTR) { continue; }
            return Err(e);
        }
        let mut rtype = REC_DATA;
        unsafe {
            let c = libc::CMSG_FIRSTHDR(&msg);
            if !c.is_null() && (*c).cmsg_level == libc::SOL_TLS && (*c).cmsg_type == TLS_GET_RECORD_TYPE {
                rtype = *libc::CMSG_DATA(c);
            }
        }
        return Ok((rtype, n as usize));
    }
}
pub fn send_rec(fd: RawFd, rtype: u8, data: &[u8]) -> io::Result<usize> {
    let mut iov = libc::iovec { iov_base: data.as_ptr() as *mut _, iov_len: data.len() };
    let mut cbuf = [0u8; 64];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cbuf.as_mut_ptr().cast();
    msg.msg_controllen = unsafe { libc::CMSG_SPACE(1) } as _;
    unsafe {
        let c = libc::CMSG_FIRSTHDR(&msg);
        (*c).cmsg_level = libc::SOL_TLS;
        (*c).cmsg_type = TLS_SET_RECORD_TYPE;
        (*c).cmsg_len = libc::CMSG_LEN(1) as _;
        *libc::CMSG_DATA(c) = rtype;
    }
    let n = unsafe { libc::sendmsg(fd, &msg, 0) };
    if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
}

// ---------------------------------------------------------------- splice
pub fn mkpipe(size: Option<i32>) -> (RawFd, RawFd) {
    let mut p = [0i32; 2];
    assert_eq!(unsafe { libc::pipe2(p.as_mut_ptr(), 0) }, 0);
    if let Some(s) = size {
        let r = unsafe { libc::fcntl(p[0], libc::F_SETPIPE_SZ, s) };
        assert!(r >= 0, "F_SETPIPE_SZ {}", io::Error::last_os_error());
    }
    (p[0], p[1])
}
pub fn pipe_size(fd: RawFd) -> i32 {
    unsafe { libc::fcntl(fd, libc::F_GETPIPE_SZ) }
}
pub fn pipe_bytes(fd: RawFd) -> i32 {
    let mut n = 0i32;
    unsafe { libc::ioctl(fd, libc::FIONREAD, &mut n) };
    n
}
/// raw splice; returns Ok(n) or Err; EINTR retried.
pub fn splice(fin: RawFd, fout: RawFd, len: usize, flags: u32) -> io::Result<usize> {
    loop {
        let r = unsafe { libc::splice(fin, std::ptr::null_mut(), fout, std::ptr::null_mut(), len, flags) };
        if r >= 0 { return Ok(r as usize); }
        let e = io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::EINTR) { continue; }
        return Err(e);
    }
}
pub fn set_nonblock(fd: RawFd, on: bool) {
    unsafe {
        let f = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, if on { f | libc::O_NONBLOCK } else { f & !libc::O_NONBLOCK });
    }
}
pub fn sockopt_i32(fd: RawFd, lvl: i32, name: i32, v: i32) {
    let r = unsafe { libc::setsockopt(fd, lvl, name, (&v as *const i32).cast(), 4) };
    assert_eq!(r, 0, "setsockopt {}", io::Error::last_os_error());
}
pub fn thread_cpu_ms() -> f64 {
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    ts.tv_sec as f64 * 1e3 + ts.tv_nsec as f64 / 1e6
}
pub fn vm_rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status").unwrap().lines().find(|l| l.starts_with("VmRSS")).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse().ok()).unwrap_or(0)
}
pub fn tls_stat() -> String {
    let s = std::fs::read_to_string("/proc/net/tls_stat").unwrap_or_default();
    s.lines().filter(|l| l.contains("Rekey") || l.contains("DecryptError")).map(|l| l.split_whitespace().collect::<Vec<_>>().join("=")).collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------- mini HTTP/1.1 over any Read+Write
pub struct Http<S: Read + Write> {
    pub s: S,
    pub buf: Vec<u8>, // carry (bytes read but not consumed)
}
impl<S: Read + Write> Http<S> {
    pub fn new(s: S) -> Self { Self { s, buf: Vec::new() } }
    fn fill(&mut self) -> io::Result<usize> {
        let mut t = [0u8; 16384];
        let n = self.s.read(&mut t)?;
        self.buf.extend_from_slice(&t[..n]);
        Ok(n)
    }
    /// read a head up to and including CRLFCRLF
    pub fn head(&mut self) -> io::Result<String> {
        loop {
            if let Some(p) = self.buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let h: Vec<u8> = self.buf.drain(..p + 4).collect();
                return Ok(String::from_utf8_lossy(&h).into_owned());
            }
            if self.fill()? == 0 { return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "eof in head")); }
        }
    }
    pub fn take_n(&mut self, n: usize, mut sink: impl FnMut(&[u8])) -> io::Result<()> {
        let mut left = n;
        while left > 0 {
            if self.buf.is_empty() && self.fill()? == 0 { return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "eof in body")); }
            let k = left.min(self.buf.len());
            sink(&self.buf[..k]);
            self.buf.drain(..k);
            left -= k;
        }
        Ok(())
    }
    pub fn line(&mut self) -> io::Result<String> {
        loop {
            if let Some(p) = self.buf.windows(2).position(|w| w == b"\r\n") {
                let h: Vec<u8> = self.buf.drain(..p + 2).collect();
                return Ok(String::from_utf8_lossy(&h[..h.len() - 2]).into_owned());
            }
            if self.fill()? == 0 { return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "eof in line")); }
        }
    }
    /// read a body per head framing; returns total body bytes (verifying pattern from `off`).
    pub fn body(&mut self, head: &str, off: usize) -> io::Result<usize> {
        let lh = head.to_ascii_lowercase();
        let mut total = 0usize;
        let mut bad: Option<usize> = None;
        let mut chk = |d: &[u8], total: &mut usize, bad: &mut Option<usize>| {
            for (i, b) in d.iter().enumerate() {
                if *b != pat(off + *total + i) && bad.is_none() { *bad = Some(*total + i); }
            }
            *total += d.len();
        };
        if lh.contains("transfer-encoding: chunked") {
            loop {
                let l = self.line()?;
                let sz = usize::from_str_radix(l.trim(), 16).map_err(|_| io::Error::other(format!("bad chunk line {l:?}")))?;
                if sz == 0 { let _ = self.line()?; break; }
                let mut t = total;
                self.take_n(sz, |d| chk(d, &mut t, &mut bad))?;
                total = t;
                let c = self.line()?;
                assert!(c.is_empty(), "chunk trailer crlf");
            }
        } else if let Some(p) = lh.find("content-length:") {
            let n: usize = lh[p + 15..].lines().next().unwrap().trim().parse().unwrap();
            let mut t = 0;
            self.take_n(n, |d| chk(d, &mut t, &mut bad))?;
            total = t;
        }
        if let Some(b) = bad { return Err(io::Error::other(format!("PATTERN MISMATCH at body byte {b}"))); }
        Ok(total)
    }
}

pub fn listener() -> (TcpListener, std::net::SocketAddr) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    (l, a)
}
pub fn deadline_guard(secs: u64, what: &'static str) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(secs));
        eprintln!("WATCHDOG: {what} exceeded {secs}s -> abort");
        std::process::exit(99);
    });
}
pub fn fd_of<T: AsRawFd>(t: &T) -> RawFd { t.as_raw_fd() }
pub fn since(t: Instant) -> String { format!("{:>6}ms", t.elapsed().as_millis()) }

// ---------------------------------------------------------------- topology helper
use rustls::{ClientConnection, ServerConnection, StreamOwned};
use std::sync::mpsc;
use std::thread::JoinHandle;

pub type SStream = StreamOwned<ClientConnection, TcpStream>;
pub type RStream = StreamOwned<ServerConnection, TcpStream>;

pub struct Gw {
    pub a: TcpStream,
    pub b: TcpStream,
    pub a_kc: KernelConnection<ServerConnectionData>,
    pub b_kc: KernelConnection<ClientConnectionData>,
    pub log_a: Arc<SecretLog>,
}

/// S (rustls client) -> A=gateway front (TLS server, kTLS TX+RX) ... B=gateway back (TLS client, kTLS TX+RX) -> R (rustls server).
pub fn build_gw<T: Send + 'static>(
    tickets_r: usize,
    suite_256: bool,
    s_script: Box<dyn FnOnce(&mut SStream) + Send>,
    r_script: Box<dyn FnOnce(&mut RStream) -> T + Send>,
) -> (Gw, JoinHandle<()>, JoinHandle<T>) {
    let (la, aa) = listener();
    let (lb, ab) = listener();
    let r = std::thread::spawn(move || {
        let (tcp, _) = lb.accept().unwrap();
        let conn = ServerConnection::new(server_config(None, tickets_r, suite_256)).unwrap();
        let mut st = StreamOwned::new(conn, tcp);
        r_script(&mut st)
    });
    let s = std::thread::spawn(move || {
        let tcp = TcpStream::connect(aa).unwrap();
        let conn = ClientConnection::new(client_config(suite_256), ServerName::try_from("localhost").unwrap()).unwrap();
        let mut st = StreamOwned::new(conn, tcp);
        s_script(&mut st);
    });
    let log_a = Arc::new(SecretLog::default());
    let (mut a, _) = la.accept().unwrap();
    let est_a = handshake_server(&mut a, server_config(Some(log_a.clone()), 0, suite_256)).unwrap();
    assert_eq!(est_a.leftover_ciphertext, 0);
    arm_both(a.as_raw_fd(), &est_a.secrets).unwrap();
    let mut b = TcpStream::connect(ab).unwrap();
    let est_b = handshake_client(&mut b, client_config(suite_256)).unwrap();
    assert_eq!(est_b.leftover_ciphertext, 0);
    arm_both(b.as_raw_fd(), &est_b.secrets).unwrap();
    (Gw { a, b, a_kc: est_a.kconn, b_kc: est_b.kconn, log_a }, s, r)
}

/// R-side standard receiver: read to EOF, verify pattern, return (n, first_mismatch).
pub fn r_sink(st: &mut RStream) -> (usize, Option<usize>) {
    let mut buf = vec![0u8; 65536];
    let mut n = 0usize;
    let mut bad = None;
    loop {
        match st.read(&mut buf) {
            Ok(0) => break,
            Ok(k) => {
                if bad.is_none() {
                    for (i, b) in buf[..k].iter().enumerate() {
                        if *b != pat(n + i) { bad = Some(n + i); break; }
                    }
                }
                n += k;
            }
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => { eprintln!("R read err {e}"); break; }
        }
    }
    (n, bad)
}
pub fn chan() -> (mpsc::Sender<()>, mpsc::Receiver<()>) { mpsc::channel() }

/// Two kTLS-armed endpoints joined by a rustls-negotiated TLS1.3 session (client=C, server=Sv).
pub struct Pair {
    pub c: TcpStream,
    pub s: TcpStream,
    pub c_kc: KernelConnection<ClientConnectionData>,
    pub s_kc: KernelConnection<ServerConnectionData>,
}
pub fn ktls_pair(suite_256: bool) -> Pair {
    let (l, addr) = listener();
    let th = std::thread::spawn(move || {
        let mut c = TcpStream::connect(addr).unwrap();
        let est = handshake_client(&mut c, client_config(suite_256)).unwrap();
        assert_eq!(est.leftover_ciphertext, 0);
        arm_both(c.as_raw_fd(), &est.secrets).unwrap();
        (c, est.kconn)
    });
    let (mut s, _) = l.accept().unwrap();
    let est = handshake_server(&mut s, server_config(None, 0, suite_256)).unwrap();
    assert_eq!(est.leftover_ciphertext, 0);
    arm_both(s.as_raw_fd(), &est.secrets).unwrap();
    let (c, c_kc) = th.join().unwrap();
    Pair { c, s, c_kc, s_kc: est.kconn }
}
pub fn kc_tx_c(kc: &mut KernelConnection<ClientConnectionData>) -> io::Result<(u64, ConnectionTrafficSecrets)> { kc.update_tx_secret().map_err(io::Error::other) }
pub fn kc_rx_c(kc: &mut KernelConnection<ClientConnectionData>) -> io::Result<(u64, ConnectionTrafficSecrets)> { kc.update_rx_secret().map_err(io::Error::other) }
pub fn kc_tx_s(kc: &mut KernelConnection<ServerConnectionData>) -> io::Result<(u64, ConnectionTrafficSecrets)> { kc.update_tx_secret().map_err(io::Error::other) }
pub fn kc_rx_s(kc: &mut KernelConnection<ServerConnectionData>) -> io::Result<(u64, ConnectionTrafficSecrets)> { kc.update_rx_secret().map_err(io::Error::other) }

// ================================================================ relay (the procedure under test)
pub const EKEYEXPIRED: i32 = 127;

#[derive(Debug)]
pub enum Ctl { Continue, PeerClosed }

/// Handles a NON-DATA record the kernel refused to splice (retrieved via recvmsg + TLS_GET_RECORD_TYPE).
pub trait CtlHandler {
    fn on_control(&mut self, rtype: u8, payload: &[u8]) -> io::Result<Ctl>;
}

#[derive(Default, Debug, Clone)]
pub struct RelayStat {
    pub splice_in: u64,
    pub splice_out: u64,
    pub eagain_in: u64,
    pub eagain_out: u64,
    pub ctl: u64,
    pub bytes: u64,
    pub send_calls: u64,
    pub peek_calls: u64,
    pub max_pipe: i32,
}

pub struct Relay {
    pub pr: RawFd,
    pub pw: RawFd,
    pub cap: usize,
    pub nonblocking: bool,
    pub stat: RelayStat,
}

fn poll1(fd: RawFd, ev: i16) {
    let mut p = libc::pollfd { fd, events: ev, revents: 0 };
    unsafe { libc::poll(&mut p, 1, 1000) };
}

impl Relay {
    pub fn new(cap: i32, nonblocking: bool) -> Self {
        let (pr, pw) = mkpipe(Some(cap));
        Relay { pr, pw, cap: pipe_size(pr) as usize, nonblocking, stat: RelayStat::default() }
    }
    fn handle_ctl(&mut self, src: RawFd, h: &mut dyn CtlHandler) -> io::Result<Ctl> {
        let mut buf = vec![0u8; 20000];
        let (t, n) = recv_rec(src, &mut buf, 0)?;
        self.stat.ctl += 1;
        if t == REC_DATA {
            return Err(io::Error::other("EINVAL from splice but recvmsg returned DATA"));
        }
        h.on_control(t, &buf[..n])
    }
    /// Forward EXACTLY `n` body bytes src -> dst. Invariant: splice-in only into an EMPTY pipe,
    /// len = min(remaining, pipe capacity) -> never reads past the message boundary.
    pub fn body_exact(&mut self, src: RawFd, dst: RawFd, n: usize, h: &mut dyn CtlHandler) -> io::Result<Ctl> {
        let mut rem = n;
        let nbflag = if self.nonblocking { libc::SPLICE_F_NONBLOCK } else { 0 };
        while rem > 0 {
            debug_assert_eq!(pipe_bytes(self.pr), 0);
            let want = rem.min(self.cap);
            match splice(src, self.pw, want, nbflag) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, format!("src EOF with {rem} body bytes outstanding"))),
                Ok(k) => {
                    self.stat.splice_in += 1;
                    let pb = pipe_bytes(self.pr);
                    if pb > self.stat.max_pipe { self.stat.max_pipe = pb; }
                    rem -= k;
                    let mut left = k;
                    while left > 0 {
                        let fl = if rem > 0 { libc::SPLICE_F_MORE | nbflag } else { nbflag };
                        match splice(self.pr, dst, left, fl) {
                            Ok(m) => { self.stat.splice_out += 1; left -= m; self.stat.bytes += m as u64; }
                            Err(e) if e.raw_os_error() == Some(libc::EAGAIN) => { self.stat.eagain_out += 1; poll1(dst, libc::POLLOUT); }
                            Err(e) => return Err(e),
                        }
                    }
                }
                Err(e) if e.raw_os_error() == Some(libc::EAGAIN) => { self.stat.eagain_in += 1; poll1(src, libc::POLLIN); }
                Err(e) if e.raw_os_error() == Some(libc::EINVAL) => {
                    if let Ctl::PeerClosed = self.handle_ctl(src, h)? { return Ok(Ctl::PeerClosed); }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(Ctl::Continue)
    }
    /// Read ONE head/line terminated by `term` WITHOUT consuming beyond it: MSG_PEEK, then recv exactly up to the
    /// terminator. Peeked bytes that do not yet contain the terminator are ALL head bytes -> consumed into `acc`
    /// (a peek can never advance past a control record, so a partial head must be consumed to make progress).
    pub fn read_until(&mut self, src: RawFd, term: &[u8], h: &mut dyn CtlHandler) -> io::Result<Option<Vec<u8>>> {
        let mut buf = vec![0u8; 32768];
        let mut acc: Vec<u8> = Vec::new();
        loop {
            self.stat.peek_calls += 1;
            let r = recv_rec(src, &mut buf, libc::MSG_PEEK | if self.nonblocking { libc::MSG_DONTWAIT } else { 0 });
            match r {
                Ok((REC_DATA, 0)) => return if acc.is_empty() { Ok(None) } else { Err(io::Error::new(io::ErrorKind::UnexpectedEof, "eof in head")) },
                Ok((REC_DATA, n)) => {
                    let mut tmp = acc.clone();
                    tmp.extend_from_slice(&buf[..n]);
                    let from = acc.len().saturating_sub(term.len() - 1);
                    let found = tmp[from..].windows(term.len()).position(|w| w == term).map(|p| p + from);
                    let take = match found { Some(p) => p + term.len() - acc.len(), None => n };
                    let mut got = 0;
                    let mut piece = vec![0u8; take];
                    while got < take {
                        let (t, k) = recv_rec(src, &mut piece[got..], 0)?;
                        assert_eq!(t, REC_DATA);
                        got += k;
                    }
                    acc.extend_from_slice(&piece);
                    if found.is_some() { return Ok(Some(acc)); }
                    if acc.len() > 65536 { return Err(io::Error::other("head too large")); }
                }
                Ok((_t, _)) => {
                    // non-data record at the head of the stream: fetch + handle it
                    if let Ctl::PeerClosed = self.handle_ctl(src, h)? { return Ok(None); }
                }
                Err(e) if e.raw_os_error() == Some(libc::EAGAIN) => poll1(src, libc::POLLIN),
                Err(e) => return Err(e),
            }
        }
    }
    pub fn send_all(&mut self, dst: RawFd, d: &[u8]) -> io::Result<()> {
        let mut off = 0;
        while off < d.len() {
            let r = unsafe { libc::send(dst, d[off..].as_ptr().cast(), d.len() - off, 0) };
            self.stat.send_calls += 1;
            if r < 0 {
                let e = io::Error::last_os_error();
                if e.raw_os_error() == Some(libc::EAGAIN) { poll1(dst, libc::POLLOUT); continue; }
                if e.raw_os_error() == Some(libc::EINTR) { continue; }
                return Err(e);
            }
            off += r as usize;
        }
        Ok(())
    }
    /// Relay ONE HTTP/1.1 message. Ok(false) when src EOF before a new head.
    pub fn http_message(&mut self, src: RawFd, dst: RawFd, h: &mut dyn CtlHandler) -> io::Result<bool> {
        let Some(head) = self.read_until(src, b"\r\n\r\n", h)? else { return Ok(false) };
        self.send_all(dst, &head)?;
        let lh = String::from_utf8_lossy(&head).to_ascii_lowercase();
        if lh.contains("transfer-encoding: chunked") {
            loop {
                let line = self.read_until(src, b"\r\n", h)?.ok_or_else(|| io::Error::other("eof in chunk line"))?;
                self.send_all(dst, &line)?;
                let sz = usize::from_str_radix(String::from_utf8_lossy(&line).trim(), 16).map_err(|_| io::Error::other("bad chunk size"))?;
                if sz == 0 {
                    let t = self.read_until(src, b"\r\n", h)?.ok_or_else(|| io::Error::other("eof in trailer"))?;
                    self.send_all(dst, &t)?;
                    break;
                }
                if let Ctl::PeerClosed = self.body_exact(src, dst, sz, h)? { return Ok(false); }
                let crlf = self.read_until(src, b"\r\n", h)?.ok_or_else(|| io::Error::other("eof in chunk crlf"))?;
                assert_eq!(crlf, b"\r\n");
                self.send_all(dst, &crlf)?;
            }
        } else if let Some(p) = lh.find("content-length:") {
            let n: usize = lh[p + 15..].lines().next().unwrap().trim().parse().unwrap();
            if n > 0 { if let Ctl::PeerClosed = self.body_exact(src, dst, n, h)? { return Ok(false); } }
        }
        Ok(true)
    }
}

/// Message builders (stream bytes the sender emits and the downstream must receive verbatim).
pub fn msg_cl(method_line: &str, n: usize, off: usize) -> Vec<u8> {
    let mut v = format!("{method_line}\r\nHost: gw\r\nContent-Length: {n}\r\n\r\n").into_bytes();
    v.extend(pattern(off, n));
    v
}
pub fn msg_chunked(method_line: &str, chunks: &[usize], off: usize) -> Vec<u8> {
    let mut v = format!("{method_line}\r\nHost: gw\r\nTransfer-Encoding: chunked\r\n\r\n").into_bytes();
    let mut o = off;
    for &c in chunks {
        v.extend(format!("{c:x}\r\n").into_bytes());
        v.extend(pattern(o, c));
        v.extend(b"\r\n");
        o += c;
    }
    v.extend(b"0\r\n\r\n");
    v
}

/// Write `data` through a kTLS-TX fd as records of at most `rec` bytes; at the listed stream offsets
/// insert a TLS1.3 KeyUpdate (sender rekeys TX through the rustls kernel connection).
pub fn send_stream<F: FnMut() -> io::Result<(u64, ConnectionTrafficSecrets)>>(
    fd: RawFd, data: &[u8], rec: usize, ku_at: &[usize], mut next_tx: F, progress: Option<&std::sync::atomic::AtomicU64>,
) -> io::Result<()> {
    let mut off = 0;
    let mut kui = 0;
    while off < data.len() {
        let mut end = (off + rec).min(data.len());
        if kui < ku_at.len() {
            if ku_at[kui] <= off {
                send_rec(fd, REC_HANDSHAKE, &[24, 0, 0, 1, 0])?;
                let (seq, sec) = next_tx()?;
                ktls_set(fd, libc::TLS_TX, seq, &sec)?;
                kui += 1;
                continue;
            }
            end = end.min(ku_at[kui]);
        }
        let mut o = off;
        while o < end {
            let r = unsafe { libc::send(fd, data[o..end].as_ptr().cast(), end - o, 0) };
            if r < 0 {
                let e = io::Error::last_os_error();
                if e.raw_os_error() == Some(libc::EINTR) { continue; }
                return Err(e);
            }
            o += r as usize;
            if let Some(p) = progress { p.fetch_add(r as u64, std::sync::atomic::Ordering::Relaxed); }
        }
        off = end;
    }
    Ok(())
}

/// Read to EOF from a kTLS fd and compare with `expect`. Returns (n, first_mismatch). `slow`: (bytes_per_read, sleep).
pub fn recv_verify(fd: RawFd, expect: &[u8], slow: Option<(usize, Duration)>, progress: Option<&std::sync::atomic::AtomicU64>) -> (usize, Option<usize>) {
    let mut buf = vec![0u8; slow.map(|s| s.0).unwrap_or(1 << 18)];
    let mut n = 0usize;
    let mut bad = None;
    loop {
        let r = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
        if r < 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINTR) { continue; }
            eprintln!("recv_verify err {e}");
            break;
        }
        if r == 0 { break; }
        let k = r as usize;
        if bad.is_none() {
            let end = (n + k).min(expect.len());
            if n >= expect.len() || buf[..end - n] != expect[n..end] || n + k > expect.len() {
                // locate first mismatch
                let m = buf[..k].iter().zip(expect[n..].iter()).position(|(a, b)| a != b).map(|p| n + p).unwrap_or(expect.len());
                bad = Some(m);
            }
        }
        n += k;
        if let Some(p) = progress { p.fetch_add(k as u64, std::sync::atomic::Ordering::Relaxed); }
        if let Some((_, d)) = slow { std::thread::sleep(d); }
    }
    (n, bad)
}

/// TLS1.3 KeyUpdate procedure for the SERVER-role kTLS socket `fd` (front side A).
pub struct KuServer<'a> {
    pub fd: RawFd,
    pub kc: &'a mut KernelConnection<ServerConnectionData>,
    pub log: Vec<String>,
    pub rx_rekeys: u32,
    pub tx_rekeys: u32,
}
/// same for the client-role socket
pub struct KuClient<'a> {
    pub fd: RawFd,
    pub kc: &'a mut KernelConnection<ClientConnectionData>,
    pub log: Vec<String>,
    pub rx_rekeys: u32,
    pub tx_rekeys: u32,
}
fn ku_common(fd: RawFd, rtype: u8, payload: &[u8], log: &mut Vec<String>,
             rx: &mut dyn FnMut() -> io::Result<(u64, ConnectionTrafficSecrets)>,
             tx: &mut dyn FnMut() -> io::Result<(u64, ConnectionTrafficSecrets)>,
             rxc: &mut u32, txc: &mut u32) -> io::Result<Ctl> {
    match rtype {
        REC_ALERT => {
            log.push(format!("ALERT level={} desc={}", payload.first().copied().unwrap_or(0), payload.get(1).copied().unwrap_or(0)));
            Ok(Ctl::PeerClosed)
        }
        REC_HANDSHAKE if payload.first() == Some(&24) && payload.len() == 5 => {
            let req = payload[4];
            // (1) probe the documented pause: a read now must fail with EKEYEXPIRED until TLS_RX is re-armed
            let mut b = [0u8; 8];
            let probe = recv_rec(fd, &mut b, libc::MSG_DONTWAIT).map(|(t, n)| (t, n)).map_err(|e| errno_name(&e));
            let sp = { let (pr, pw) = mkpipe(None); let r = splice(fd, pw, 100, libc::SPLICE_F_NONBLOCK).map_err(|e| errno_name(&e)); unsafe { libc::close(pr); libc::close(pw); } r };
            log.push(format!("KeyUpdate(request={req}) received; before TLS_RX re-arm: recvmsg={probe:?} splice={sp:?}"));
            // (2) new RX key, seq 0
            let (seq, sec) = rx()?;
            ktls_set(fd, libc::TLS_RX, seq, &sec)?;
            *rxc += 1;
            // (3) peer asked for an update: answer under the OLD tx key, then switch TX
            if req == 1 {
                send_rec(fd, REC_HANDSHAKE, &[24, 0, 0, 1, 0])?;
                let (seq, sec) = tx()?;
                ktls_set(fd, libc::TLS_TX, seq, &sec)?;
                *txc += 1;
                log.push("answered with KeyUpdate(not_requested) and re-armed TLS_TX".into());
            }
            Ok(Ctl::Continue)
        }
        REC_HANDSHAKE => {
            log.push(format!("other HANDSHAKE msg type={} len={} (ignored)", payload.first().copied().unwrap_or(0), payload.len()));
            Ok(Ctl::Continue)
        }
        t => Err(io::Error::other(format!("unexpected control record type {t}"))),
    }
}
impl CtlHandler for KuServer<'_> {
    fn on_control(&mut self, rtype: u8, payload: &[u8]) -> io::Result<Ctl> {
        // two closures needing &mut kc cannot coexist; they are called strictly sequentially -> raw ptr
        let kc_ptr: *mut KernelConnection<ServerConnectionData> = &mut *self.kc;
        let mut rxf = move || unsafe { (*kc_ptr).update_rx_secret().map_err(io::Error::other) };
        let mut txf = move || unsafe { (*kc_ptr).update_tx_secret().map_err(io::Error::other) };
        ku_common(self.fd, rtype, payload, &mut self.log, &mut rxf, &mut txf, &mut self.rx_rekeys, &mut self.tx_rekeys)
    }
}
impl CtlHandler for KuClient<'_> {
    fn on_control(&mut self, rtype: u8, payload: &[u8]) -> io::Result<Ctl> {
        let kc_ptr: *mut KernelConnection<ClientConnectionData> = &mut *self.kc;
        let mut rxf = move || unsafe { (*kc_ptr).update_rx_secret().map_err(io::Error::other) };
        let mut txf = move || unsafe { (*kc_ptr).update_tx_secret().map_err(io::Error::other) };
        ku_common(self.fd, rtype, payload, &mut self.log, &mut rxf, &mut txf, &mut self.rx_rekeys, &mut self.tx_rekeys)
    }
}
/// Ignores control records (used where none are expected)
pub struct NoCtl;
impl CtlHandler for NoCtl {
    fn on_control(&mut self, rtype: u8, payload: &[u8]) -> io::Result<Ctl> {
        if rtype == REC_ALERT { return Ok(Ctl::PeerClosed); }
        eprintln!("NoCtl: control record type={rtype} len={}", payload.len());
        Ok(Ctl::Continue)
    }
}

// ================================================================ kTLS-peer topology for relay tests
pub struct GwK {
    pub gw: Gw,
    pub c: TcpStream,
    pub c_kc: KernelConnection<ClientConnectionData>,
    pub u: TcpStream,
    pub u_kc: KernelConnection<ServerConnectionData>,
}
/// C(kTLS client) -> A(gateway, TLS server, kTLS) ... B(gateway, TLS client, kTLS) -> U(kTLS server)
pub fn build_gw_k(suite_256: bool) -> GwK {
    let (la, aa) = listener();
    let (lb, ab) = listener();
    let tc = std::thread::spawn(move || {
        let mut c = TcpStream::connect(aa).unwrap();
        let est = handshake_client(&mut c, client_config(suite_256)).unwrap();
        assert_eq!(est.leftover_ciphertext, 0);
        arm_both(c.as_raw_fd(), &est.secrets).unwrap();
        (c, est.kconn)
    });
    let tu = std::thread::spawn(move || {
        let (mut u, _) = lb.accept().unwrap();
        let est = handshake_server(&mut u, server_config(None, 0, suite_256)).unwrap();
        assert_eq!(est.leftover_ciphertext, 0);
        arm_both(u.as_raw_fd(), &est.secrets).unwrap();
        (u, est.kconn)
    });
    let log_a = Arc::new(SecretLog::default());
    let (mut a, _) = la.accept().unwrap();
    let est_a = handshake_server(&mut a, server_config(Some(log_a.clone()), 0, suite_256)).unwrap();
    assert_eq!(est_a.leftover_ciphertext, 0);
    arm_both(a.as_raw_fd(), &est_a.secrets).unwrap();
    let mut b = TcpStream::connect(ab).unwrap();
    let est_b = handshake_client(&mut b, client_config(suite_256)).unwrap();
    assert_eq!(est_b.leftover_ciphertext, 0);
    arm_both(b.as_raw_fd(), &est_b.secrets).unwrap();
    let (c, c_kc) = tc.join().unwrap();
    let (u, u_kc) = tu.join().unwrap();
    GwK { gw: Gw { a, b, a_kc: est_a.kconn, b_kc: est_b.kconn, log_a }, c, c_kc, u, u_kc }
}

/// verify pattern chunk at stream offset n (fast memcmp against a periodic buffer)
pub struct FastPat { buf: Vec<u8> }
impl FastPat {
    pub fn new() -> Self { Self { buf: pattern(0, 251 + (1 << 20)) } }
    pub fn ok(&self, chunk: &[u8], n: usize) -> bool { let s = n % 251; chunk == &self.buf[s..s + chunk.len()] }
}

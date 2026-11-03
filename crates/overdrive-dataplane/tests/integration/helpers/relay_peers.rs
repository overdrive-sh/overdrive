//! Real-kernel peers and oracles for the host kernel-relay Tier-3 suite
//! (`gateway_relay.rs`, `gateway_relay_equivalence.rs`; S-PIG-104..118, 123).
//!
//! Every peer is a test-owned endpoint on loopback: a real rustls handshake
//! (the unbuffered API, one TLS record at a time so nothing is over-read), then
//! kernel TLS armed on the peer's own socket from the `KernelConnection`. The
//! peer therefore speaks to `HostGatewayRelay` over real TLS 1.3 records and can
//! also craft records no userspace TLS stack will emit (a KeyUpdate that does not
//! request an answer, an alert, a record of an unexpected type). Its handshake,
//! cipher choice and record layer share nothing with the code under test except
//! the kernel's TLS implementation.
//!
//! The oracles are independent of the relay: octets compared at the peers,
//! `/proc/net/tls_stat` counter deltas, `/proc/self/fd` counts and `/proc/self/statm`.
//!
//! The peers are TLS clients: `HostGatewayRelay::accept_public` is the only
//! pinned way for a test outside the crate to obtain an armed leg, so both the
//! "public" and the "upstream" legs of a port-level rig are legs the relay
//! accepted as the TLS server. After arming a leg is symmetric; the client-role
//! arm of an authenticated upstream is the same-crate call from
//! `HostGatewayClientMtls` and is covered by the exact-peer mTLS suite.

#![cfg(target_os = "linux")]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::missing_const_for_fn,
    clippy::too_many_lines,
    dead_code,
    reason = "test-owned peers: fixture preconditions are asserted, and a shared helper is not used by every consumer"
)]

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::os::fd::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use overdrive_core::traits::clock::Clock;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{ClientConnectionData, UnbufferedClientConnection};
use rustls::kernel::KernelConnection;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::unbuffered::{ConnectionState, EncodeError};
use rustls::{
    ClientConfig, ConnectionTrafficSecrets, DigitallySignedStruct, ExtractedSecrets, ServerConfig,
    SignatureScheme,
};

/// The Route hostname every peer presents as SNI.
pub const SNI: &str = "api.example.com";

/// The relay pipe capacity the pipe invariant requires (ADR-0246): 64 KiB.
pub const PIPE_CAPACITY: usize = 65_536;

pub const TLS_SET_RECORD_TYPE: libc::c_int = 1;
pub const TLS_GET_RECORD_TYPE: libc::c_int = 2;
pub const RECORD_ALERT: u8 = 21;
pub const RECORD_HANDSHAKE: u8 = 22;
pub const RECORD_DATA: u8 = 23;

// ---------------------------------------------------------------------------
// Time and patterns
// ---------------------------------------------------------------------------

/// Wall-clock `Clock` for tests that drive the real kernel: the relay evaluates
/// its deadlines on the injected clock, and a real socket only makes progress
/// in real time. Test-local so no production binding crate is a dev-dependency.
pub struct WallClock;

#[async_trait]
impl Clock for WallClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn unix_now(&self) -> Duration {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()
    }
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

#[must_use]
pub fn pat(index: usize) -> u8 {
    (index % 251) as u8
}

/// `len` octets of the position-keyed pattern starting at stream offset `offset`.
#[must_use]
pub fn pattern(offset: usize, len: usize) -> Vec<u8> {
    (offset..offset + len).map(pat).collect()
}

/// First stream offset at which `chunk` (which starts at `offset`) departs from
/// the pattern, if any.
#[must_use]
pub fn first_pattern_mismatch(offset: usize, chunk: &[u8]) -> Option<usize> {
    chunk.iter().enumerate().find(|(i, b)| **b != pat(offset + i)).map(|(i, _)| offset + i)
}

/// Deterministic generator for record-size scripts (a printed seed reproduces a run).
pub struct Lcg(pub u64);

impl Lcg {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    /// Uniform-ish in `lo..=hi`.
    pub fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next() as usize) % (hi - lo + 1)
    }
}

// ---------------------------------------------------------------------------
// Kernel oracles
// ---------------------------------------------------------------------------

/// `/proc/net/tls_stat` (per network namespace, node-global): names as the spike recorded them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TlsStat(BTreeMap<String, u64>);

impl TlsStat {
    #[must_use]
    pub fn read() -> Self {
        let text = std::fs::read_to_string("/proc/net/tls_stat")
            .expect("/proc/net/tls_stat is readable on a kernel with the tls module");
        let map = text
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let name = parts.next()?;
                let value = parts.next()?.parse::<u64>().ok()?;
                Some((name.to_owned(), value))
            })
            .collect();
        Self(map)
    }

    /// Value of `name`; a counter this kernel does not list is a fixture failure.
    #[must_use]
    pub fn get(&self, name: &str) -> u64 {
        *self.0.get(name).unwrap_or_else(|| panic!("tls_stat has no counter {name}"))
    }

    /// `self - before` for `name`.
    #[must_use]
    pub fn delta(&self, before: &Self, name: &str) -> i64 {
        self.get(name) as i64 - before.get(name) as i64
    }
}

/// Open descriptors of this process (the directory scan's own descriptor is a
/// constant offset, so only differences are meaningful).
#[must_use]
pub fn open_fds() -> usize {
    std::fs::read_dir("/proc/self/fd").expect("/proc/self/fd is readable").count()
}

/// Resident set in KiB from `/proc/self/statm`.
#[must_use]
pub fn rss_kib() -> u64 {
    let statm = std::fs::read_to_string("/proc/self/statm").expect("/proc/self/statm is readable");
    let pages: u64 =
        statm.split_whitespace().nth(1).and_then(|p| p.parse().ok()).expect("statm resident field");
    // SAFETY: `sysconf` with a valid name has no memory-safety preconditions.
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap_or(4096);
    pages * page / 1024
}

/// Largest value of `/proc/sys/net/ipv4/tcp_rmem` or `tcp_wmem` (the autotuning ceiling).
#[must_use]
pub fn tcp_buffer_ceiling(name: &str) -> u64 {
    let text = std::fs::read_to_string(format!("/proc/sys/net/ipv4/{name}"))
        .expect("tcp buffer sysctl is readable");
    text.split_whitespace().last().and_then(|v| v.parse().ok()).expect("tcp buffer ceiling")
}

// ---------------------------------------------------------------------------
// TLS configurations
// ---------------------------------------------------------------------------

/// The suites a peer offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    Aes128,
    Aes256,
    /// Both AES-GCM suites.
    BothAes,
    ChaChaOnly,
}

pub const ALL_AES: [Offer; 2] = [Offer::Aes128, Offer::Aes256];

static CERT: OnceLock<(Vec<u8>, Vec<u8>)> = OnceLock::new();

fn cert() -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let (cert, key) = CERT.get_or_init(|| {
        let key = rcgen::KeyPair::generate().expect("server key");
        let cert = rcgen::CertificateParams::new(vec![SNI.to_owned()])
            .expect("server params")
            .self_signed(&key)
            .expect("server certificate");
        (cert.der().to_vec(), key.serialize_der())
    });
    (CertificateDer::from(cert.clone()), PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.clone())))
}

fn provider(offer: Offer) -> Arc<rustls::crypto::CryptoProvider> {
    use rustls::crypto::ring::cipher_suite::{
        TLS13_AES_128_GCM_SHA256, TLS13_AES_256_GCM_SHA384, TLS13_CHACHA20_POLY1305_SHA256,
    };
    let mut provider = rustls::crypto::ring::default_provider();
    provider.cipher_suites = match offer {
        Offer::Aes128 => vec![TLS13_AES_128_GCM_SHA256],
        Offer::Aes256 => vec![TLS13_AES_256_GCM_SHA384],
        Offer::BothAes => vec![TLS13_AES_128_GCM_SHA256, TLS13_AES_256_GCM_SHA384],
        Offer::ChaChaOnly => vec![TLS13_CHACHA20_POLY1305_SHA256],
    };
    Arc::new(provider)
}

/// The server configuration the relay is handed at port level, built exactly as
/// ADR-0226 prescribes for the custody snapshot's `server_config()`: TLS 1.3
/// only, ALPN `http/1.1`, no tickets, no 0-RTT, secret extraction enabled, and
/// exactly the two AES-GCM suites (R-8, P-12). The snapshot builder itself is
/// crate-private to `overdrive-gateway` (its suite list is asserted by the
/// S-PIG-118 unit arm there); this fixture is the port-level stand-in.
#[must_use]
pub fn server_config() -> Arc<ServerConfig> {
    let (cert, key) = cert();
    let mut config = ServerConfig::builder_with_provider(provider(Offer::BothAes))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("TLS 1.3 server versions")
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .expect("server certificate chain");
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config.send_tls13_tickets = 0;
    config.max_early_data_size = 0;
    config.enable_secret_extraction = true;
    Arc::new(config)
}

#[derive(Debug)]
struct AcceptAnyServerCert;

impl ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// A peer's client configuration. The peer does not authenticate the relay's
/// certificate (the certificate is not under test); it offers `offer` and `alpn`.
#[must_use]
pub fn client_config(offer: Offer, alpn: &[&[u8]]) -> Arc<ClientConfig> {
    let mut config = ClientConfig::builder_with_provider(provider(offer))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("TLS 1.3 client versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert))
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    config.enable_secret_extraction = true;
    Arc::new(config)
}

/// The default peer configuration: `offer`, ALPN `http/1.1`.
#[must_use]
pub fn http_client_config(offer: Offer) -> Arc<ClientConfig> {
    client_config(offer, &[b"http/1.1"])
}

#[must_use]
pub fn sni_name() -> ServerName<'static> {
    ServerName::try_from(SNI).expect("SNI is a valid name")
}

/// A name that makes the client send no SNI extension.
#[must_use]
pub fn no_sni_name() -> ServerName<'static> {
    ServerName::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST).into())
}

// ---------------------------------------------------------------------------
// Unbuffered client handshake (one TLS record at a time)
// ---------------------------------------------------------------------------

fn read_one_record(tcp: &mut TcpStream, incoming: &mut Vec<u8>) -> io::Result<()> {
    let mut header = [0u8; 5];
    tcp.read_exact(&mut header)?;
    let len = usize::from(u16::from_be_bytes([header[3], header[4]]));
    incoming.extend_from_slice(&header);
    let at = incoming.len();
    incoming.resize(at + len, 0);
    tcp.read_exact(&mut incoming[at..])
}

/// Completes a TLS 1.3 client handshake on `tcp` and returns the secrets and
/// kernel connection. Fails (as an `io::Error`) on a handshake alert.
fn handshake_client(
    tcp: &mut TcpStream,
    config: Arc<ClientConfig>,
    name: ServerName<'static>,
) -> io::Result<(ExtractedSecrets, KernelConnection<ClientConnectionData>)> {
    enum Next {
        Read,
        Done,
        Continue,
    }
    let mut conn = UnbufferedClientConnection::new(config, name).map_err(io::Error::other)?;
    let mut incoming: Vec<u8> = Vec::new();
    let mut outgoing = vec![0u8; 4096];
    let mut out_used = 0usize;
    loop {
        let discard;
        let next;
        {
            let status = conn.process_tls_records(&mut incoming);
            discard = status.discard;
            next = match status.state.map_err(|e| io::Error::other(format!("tls: {e}")))? {
                ConnectionState::BlockedHandshake => Next::Read,
                ConnectionState::EncodeTlsData(mut state) => {
                    loop {
                        match state.encode(&mut outgoing[out_used..]) {
                            Ok(n) => {
                                out_used += n;
                                break;
                            }
                            Err(EncodeError::InsufficientSize(e)) => {
                                outgoing.resize(out_used + e.required_size, 0);
                            }
                            Err(EncodeError::AlreadyEncoded) => break,
                        }
                    }
                    Next::Continue
                }
                ConnectionState::TransmitTlsData(state) => {
                    tcp.write_all(&outgoing[..out_used])?;
                    out_used = 0;
                    state.done();
                    Next::Continue
                }
                ConnectionState::WriteTraffic(_) => Next::Done,
                _ => return Err(io::Error::other("unexpected unbuffered state in handshake")),
            };
        }
        incoming.drain(..discard);
        match next {
            Next::Read => read_one_record(tcp, &mut incoming)?,
            Next::Done => break,
            Next::Continue => {}
        }
    }
    if !incoming.is_empty() {
        return Err(io::Error::other("octets left over after the handshake: cannot arm kTLS"));
    }
    conn.dangerous_into_kernel_connection().map_err(|e| io::Error::other(format!("kernel: {e}")))
}

// ---------------------------------------------------------------------------
// Peer-side kernel TLS
// ---------------------------------------------------------------------------

fn ulp_tls(fd: RawFd) -> io::Result<()> {
    // SAFETY: setsockopt with a 3-byte "tls" string on a connected TCP socket.
    let rc = unsafe { libc::setsockopt(fd, libc::SOL_TCP, libc::TCP_ULP, c"tls".as_ptr().cast(), 3) };
    if rc == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// `setsockopt(SOL_TLS, dir)` with the `tls12_crypto_info_aes_gcm_{128,256}` layout.
/// Used to arm and to re-arm (rekey).
fn set_crypto(fd: RawFd, dir: libc::c_int, seq: u64, traffic: &ConnectionTrafficSecrets) -> io::Result<()> {
    let (cipher, key, iv): (u16, &[u8], &[u8]) = match traffic {
        ConnectionTrafficSecrets::Aes128Gcm { key, iv } => (51, key.as_ref(), iv.as_ref()),
        ConnectionTrafficSecrets::Aes256Gcm { key, iv } => (52, key.as_ref(), iv.as_ref()),
        _ => return Err(io::Error::other("peer arms AES-GCM suites only")),
    };
    let mut info: Vec<u8> = Vec::with_capacity(56);
    info.extend_from_slice(&0x0304u16.to_ne_bytes());
    info.extend_from_slice(&cipher.to_ne_bytes());
    info.extend_from_slice(&iv[4..12]);
    info.extend_from_slice(key);
    info.extend_from_slice(&iv[0..4]);
    info.extend_from_slice(&seq.to_be_bytes());
    // SAFETY: `info` holds a complete crypto_info of the stated length.
    let rc = unsafe {
        libc::setsockopt(fd, libc::SOL_TLS, dir, info.as_ptr().cast(), info.len() as libc::socklen_t)
    };
    if rc == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// `recvmsg` returning `(record type, n)`; `n == 0` with a data type is end of stream.
pub fn recv_record(fd: RawFd, buf: &mut [u8], flags: i32) -> io::Result<(u8, usize)> {
    let mut iov = libc::iovec { iov_base: buf.as_mut_ptr().cast(), iov_len: buf.len() };
    let mut control = [0u8; 64];
    // SAFETY: an all-zero msghdr is a valid empty message header.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    msg.msg_controllen = control.len() as _;
    loop {
        // SAFETY: `msg` points at live iov/control buffers of the stated lengths.
        let n = unsafe { libc::recvmsg(fd, &raw mut msg, flags) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(err);
        }
        let mut record = RECORD_DATA;
        // SAFETY: CMSG_* walk the control buffer the kernel just filled.
        unsafe {
            let c = libc::CMSG_FIRSTHDR(&raw const msg);
            if !c.is_null() && (*c).cmsg_level == libc::SOL_TLS && (*c).cmsg_type == TLS_GET_RECORD_TYPE {
                record = *libc::CMSG_DATA(c);
            }
        }
        return Ok((record, n as usize));
    }
}

/// `sendmsg` of one record of type `record` carrying `data`.
pub fn send_record(fd: RawFd, record: u8, data: &[u8]) -> io::Result<usize> {
    let mut iov = libc::iovec { iov_base: data.as_ptr().cast_mut().cast(), iov_len: data.len() };
    let mut control = [0u8; 64];
    // SAFETY: an all-zero msghdr is a valid empty message header.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    // SAFETY: CMSG_SPACE is a pure size computation.
    msg.msg_controllen = unsafe { libc::CMSG_SPACE(1) } as _;
    // SAFETY: the control buffer holds room for one one-octet cmsg.
    unsafe {
        let c = libc::CMSG_FIRSTHDR(&raw const msg);
        (*c).cmsg_level = libc::SOL_TLS;
        (*c).cmsg_type = TLS_SET_RECORD_TYPE;
        (*c).cmsg_len = libc::CMSG_LEN(1) as _;
        *libc::CMSG_DATA(c) = record;
    }
    loop {
        // SAFETY: `msg` points at live iov/control buffers of the stated lengths.
        let n = unsafe { libc::sendmsg(fd, &raw const msg, 0) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::EINTR) {
            return Err(err);
        }
    }
}

/// What one `recv` on a peer produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Received {
    /// `n` application octets are in the caller's buffer.
    Data(usize),
    /// The relay sent a `close_notify` alert.
    CloseNotify,
    /// The relay sent another alert.
    Alert(u8),
    /// A handshake record the peer handled (a KeyUpdate it re-armed for).
    Handled,
    /// The relay closed the connection without a `close_notify`.
    Eof,
}

struct PeerInner {
    tcp: TcpStream,
    kc: Mutex<KernelConnection<ClientConnectionData>>,
    tx_lock: Mutex<()>,
    key_updates_received: AtomicU32,
    rx_rekeys: AtomicU32,
    tx_rekeys: AtomicU32,
    rearm_rx: AtomicBool,
    recv_sizes: Mutex<Vec<usize>>,
}

/// A test-owned TLS client with kernel TLS armed on its own socket. Cloning
/// shares the connection (one thread reads while another writes).
#[derive(Clone)]
pub struct KtlsPeer {
    inner: Arc<PeerInner>,
}

impl KtlsPeer {
    /// Connects to `addr`, completes a real rustls handshake offering `config`,
    /// and arms kernel TLS on the peer's socket.
    pub fn connect(addr: SocketAddr, config: Arc<ClientConfig>, name: ServerName<'static>) -> io::Result<Self> {
        let mut tcp = TcpStream::connect(addr)?;
        tcp.set_nodelay(true)?;
        let (secrets, kc) = handshake_client(&mut tcp, config, name)?;
        let fd = tcp.as_raw_fd();
        ulp_tls(fd)?;
        set_crypto(fd, libc::TLS_TX, secrets.tx.0, &secrets.tx.1)?;
        set_crypto(fd, libc::TLS_RX, secrets.rx.0, &secrets.rx.1)?;
        Ok(Self {
            inner: Arc::new(PeerInner {
                tcp,
                kc: Mutex::new(kc),
                tx_lock: Mutex::new(()),
                key_updates_received: AtomicU32::new(0),
                rx_rekeys: AtomicU32::new(0),
                tx_rekeys: AtomicU32::new(0),
                rearm_rx: AtomicBool::new(true),
                recv_sizes: Mutex::new(Vec::new()),
            }),
        })
    }

    #[must_use]
    pub fn fd(&self) -> RawFd {
        self.inner.tcp.as_raw_fd()
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) {
        self.inner.tcp.set_read_timeout(timeout).expect("peer read timeout");
    }

    /// Send buffer size as the kernel reports it.
    #[must_use]
    pub fn send_buffer(&self) -> usize {
        sockopt_usize(self.fd(), libc::SO_SNDBUF)
    }

    /// Receive buffer size as the kernel reports it.
    #[must_use]
    pub fn receive_buffer(&self) -> usize {
        sockopt_usize(self.fd(), libc::SO_RCVBUF)
    }

    /// KeyUpdate records the relay sent this peer (the answer to a requesting
    /// KeyUpdate, or an update the relay originated: P-17 says there is none).
    #[must_use]
    pub fn key_updates_received(&self) -> u32 {
        self.inner.key_updates_received.load(Ordering::SeqCst)
    }

    /// RX re-arms this peer performed.
    #[must_use]
    pub fn rx_rekeys(&self) -> u32 {
        self.inner.rx_rekeys.load(Ordering::SeqCst)
    }

    /// TX re-arms this peer performed.
    #[must_use]
    pub fn tx_rekeys(&self) -> u32 {
        self.inner.tx_rekeys.load(Ordering::SeqCst)
    }

    /// Octet counts of every `Received::Data` this peer returned, in order.
    #[must_use]
    pub fn recv_sizes(&self) -> Vec<usize> {
        self.inner.recv_sizes.lock().unwrap().clone()
    }

    /// Make this peer a control receiver: it fetches a KeyUpdate record but does
    /// not re-arm its RX (the negative control: the next read fails `EKEYEXPIRED`).
    pub fn stop_rearming_rx(&self) {
        self.inner.rearm_rx.store(false, Ordering::SeqCst);
    }

    /// Sends `bytes` as one `send(2)` call (one record when at most 16 KiB).
    pub fn send(&self, bytes: &[u8]) -> io::Result<()> {
        let _tx = self.inner.tx_lock.lock().unwrap();
        let mut sent = 0;
        while sent < bytes.len() {
            // SAFETY: the pointer/length pair is a live subslice.
            let n = unsafe {
                libc::send(self.fd(), bytes[sent..].as_ptr().cast(), bytes.len() - sent, 0)
            };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(err);
            }
            sent += n as usize;
        }
        Ok(())
    }

    /// Sends `data` split into consecutive `send(2)` calls of the given sizes
    /// (cycled), so the relay sees records of mixed sizes that straddle heads and
    /// chunk-size lines.
    pub fn send_in_records(&self, data: &[u8], sizes: &[usize]) -> io::Result<()> {
        let mut at = 0;
        let mut i = 0;
        while at < data.len() {
            let n = sizes[i % sizes.len()].min(data.len() - at);
            self.send(&data[at..at + n])?;
            at += n;
            i += 1;
        }
        Ok(())
    }

    /// Sends one record of an arbitrary type.
    pub fn send_raw_record(&self, record: u8, payload: &[u8]) -> io::Result<()> {
        let _tx = self.inner.tx_lock.lock().unwrap();
        send_record(self.fd(), record, payload).map(|_| ())
    }

    /// Sends a TLS 1.3 KeyUpdate (`requested` asks the relay to answer), then
    /// re-arms this peer's TX under the new key, as the protocol requires.
    pub fn send_key_update(&self, requested: bool) -> io::Result<()> {
        let _tx = self.inner.tx_lock.lock().unwrap();
        send_record(self.fd(), RECORD_HANDSHAKE, &[24, 0, 0, 1, u8::from(requested)])?;
        let (seq, traffic) =
            self.inner.kc.lock().unwrap().update_tx_secret().map_err(io::Error::other)?;
        set_crypto(self.fd(), libc::TLS_TX, seq, &traffic)?;
        self.inner.tx_rekeys.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Sends a `close_notify` alert and shuts the write direction down.
    pub fn close_notify(&self) -> io::Result<()> {
        self.send_raw_record(RECORD_ALERT, &[1, 0])?;
        self.inner.tcp.shutdown(std::net::Shutdown::Write)
    }

    /// Sends a fatal alert.
    pub fn alert(&self, description: u8) -> io::Result<()> {
        self.send_raw_record(RECORD_ALERT, &[2, description])
    }

    /// Resets the connection: `SO_LINGER` zero, then close.
    pub fn reset(self) {
        let linger = libc::linger { l_onoff: 1, l_linger: 0 };
        // SAFETY: setsockopt with a live linger struct of the stated size.
        let rc = unsafe {
            libc::setsockopt(
                self.fd(),
                libc::SOL_SOCKET,
                libc::SO_LINGER,
                (&raw const linger).cast(),
                std::mem::size_of::<libc::linger>() as libc::socklen_t,
            )
        };
        assert_eq!(rc, 0, "SO_LINGER: {}", io::Error::last_os_error());
        let _ = self.inner.tcp.shutdown(std::net::Shutdown::Both);
    }

    /// Raw `recvmsg` with the record type and no handling at all.
    pub fn recv_raw(&self, buf: &mut [u8]) -> io::Result<(u8, usize)> {
        recv_record(self.fd(), buf, 0)
    }

    /// One read. Application octets land in `buf`; a KeyUpdate record is handled
    /// (the RX re-arm, and the answer when it requested one); alerts and end of
    /// stream are reported.
    pub fn recv(&self, buf: &mut [u8]) -> io::Result<Received> {
        let (record, n) = recv_record(self.fd(), buf, 0)?;
        match record {
            RECORD_DATA if n == 0 => Ok(Received::Eof),
            RECORD_DATA => {
                self.inner.recv_sizes.lock().unwrap().push(n);
                Ok(Received::Data(n))
            }
            RECORD_ALERT => {
                if n >= 2 && buf[1] == 0 {
                    Ok(Received::CloseNotify)
                } else {
                    Ok(Received::Alert(buf.get(1).copied().unwrap_or(0)))
                }
            }
            RECORD_HANDSHAKE if n == 5 && buf[0] == 24 => {
                self.inner.key_updates_received.fetch_add(1, Ordering::SeqCst);
                if !self.inner.rearm_rx.load(Ordering::SeqCst) {
                    return Ok(Received::Handled);
                }
                let requested = buf[4] == 1;
                let (seq, traffic) =
                    self.inner.kc.lock().unwrap().update_rx_secret().map_err(io::Error::other)?;
                set_crypto(self.fd(), libc::TLS_RX, seq, &traffic)?;
                self.inner.rx_rekeys.fetch_add(1, Ordering::SeqCst);
                if requested {
                    self.send_key_update(false)?;
                }
                Ok(Received::Handled)
            }
            RECORD_HANDSHAKE => Ok(Received::Handled),
            other => Err(io::Error::other(format!("unexpected record type {other}"))),
        }
    }

    /// Reads exactly `n` application octets, handling control records on the way.
    pub fn read_exact_data(&self, n: usize) -> io::Result<Vec<u8>> {
        let mut out = vec![0u8; n];
        let mut at = 0;
        while at < n {
            match self.recv(&mut out[at..])? {
                Received::Data(got) => at += got,
                Received::Handled => {}
                other => {
                    return Err(io::Error::other(format!(
                        "stream ended after {at} of {n} octets: {other:?}"
                    )));
                }
            }
        }
        Ok(out)
    }

    /// Reads application octets until the relay's `close_notify`; `Err` on any other end.
    pub fn read_until_close_notify(&self, limit: usize) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match self.recv(&mut buf)? {
                Received::Data(n) => {
                    out.extend_from_slice(&buf[..n]);
                    if out.len() > limit {
                        return Err(io::Error::other("more octets than the limit"));
                    }
                }
                Received::Handled => {}
                Received::CloseNotify => return Ok(out),
                other => return Err(io::Error::other(format!("ended without close_notify: {other:?}"))),
            }
        }
    }

    /// Verifies the next `expected.len()` octets against the pattern starting at
    /// stream offset `offset` without holding them all.
    pub fn verify_pattern(&self, offset: usize, total: usize) -> io::Result<()> {
        let mut buf = vec![0u8; 256 * 1024];
        let mut done = 0;
        while done < total {
            let want = buf.len().min(total - done);
            match self.recv(&mut buf[..want])? {
                Received::Data(n) => {
                    if let Some(at) = first_pattern_mismatch(offset + done, &buf[..n]) {
                        return Err(io::Error::other(format!("pattern mismatch at stream offset {at}")));
                    }
                    done += n;
                }
                Received::Handled => {}
                other => {
                    return Err(io::Error::other(format!("ended at {done} of {total}: {other:?}")));
                }
            }
        }
        Ok(())
    }
}

fn sockopt_usize(fd: RawFd, name: libc::c_int) -> usize {
    let mut value: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: getsockopt into a live c_int of the stated length.
    let rc = unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, name, (&raw mut value).cast(), &raw mut len) };
    assert_eq!(rc, 0, "getsockopt: {}", io::Error::last_os_error());
    value as usize
}

/// Reads on its own thread, counting and (optionally) collecting every octet the
/// relay sends the peer, and handling control records, until close or error.
pub struct Drain {
    pub collected: Arc<Mutex<Vec<u8>>>,
    pub received: Arc<std::sync::atomic::AtomicU64>,
    paused: Arc<AtomicBool>,
    pub ended: Arc<Mutex<Option<Received>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Drain {
    /// `collect`: keep the octets (otherwise only count them).
    #[must_use]
    pub fn start(peer: &KtlsPeer, collect: bool) -> Self {
        let collected = Arc::new(Mutex::new(Vec::new()));
        let received = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let paused = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(Mutex::new(None));
        let peer = peer.clone();
        let sink = Arc::clone(&collected);
        let counter = Arc::clone(&received);
        let gate = Arc::clone(&paused);
        let outcome = Arc::clone(&ended);
        let handle = std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                while gate.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                match peer.recv(&mut buf) {
                    Ok(Received::Data(count)) => {
                        counter.fetch_add(count as u64, Ordering::SeqCst);
                        if collect {
                            sink.lock().unwrap().extend_from_slice(&buf[..count]);
                        }
                    }
                    Ok(Received::Handled) => {}
                    Ok(end) => {
                        *outcome.lock().unwrap() = Some(end);
                        return;
                    }
                    Err(_) => return,
                }
            }
        });
        Self { collected, received, paused, ended, handle: Some(handle) }
    }

    /// Stop reading (the peer becomes a stalled reader) until `resume`.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    #[must_use]
    pub fn octets(&self) -> Vec<u8> {
        self.collected.lock().unwrap().clone()
    }

    #[must_use]
    pub fn count(&self) -> u64 {
        self.received.load(Ordering::SeqCst)
    }

    /// Waits until at least `n` octets arrived or `within` elapses; true on success.
    #[must_use]
    pub fn wait_for(&self, n: u64, within: Duration) -> bool {
        let stop = Instant::now() + within;
        while Instant::now() < stop {
            if self.count() >= n {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.count() >= n
    }

    /// Joins the reader thread (it ends at close, an alert, or an error).
    pub fn join(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Feeds `data` to `peer` from its own thread; the join handle yields the send result.
#[must_use]
pub fn send_in_background(peer: &KtlsPeer, data: Vec<u8>, sizes: Vec<usize>) -> std::thread::JoinHandle<io::Result<()>> {
    let peer = peer.clone();
    std::thread::spawn(move || peer.send_in_records(&data, &sizes))
}

// ---------------------------------------------------------------------------
// Legs accepted by the relay under test
// ---------------------------------------------------------------------------

use std::net::TcpListener;
use std::num::{NonZeroU64, NonZeroUsize};
use std::os::fd::OwnedFd;

use overdrive_core::public_ingress::PublicHostname;
use overdrive_dataplane::gateway_relay::HostGatewayRelay;
use overdrive_gateway::ports::{
    GatewayRelay, KtlsReadHalf, KtlsWriteHalf, PublicHandshakeError, ReadDeadlines, RelayDeadlines,
};

/// One leg the relay accepted: its two halves and the test-owned peer on the
/// other end of the socket.
pub struct Leg {
    pub read: KtlsReadHalf,
    pub write: KtlsWriteHalf,
    pub peer: KtlsPeer,
    pub sni: PublicHostname,
}

/// The relay under test and two accepted legs: `public` (a client's connection)
/// and `upstream` (an origin's connection). Both are accepted as the TLS server;
/// after arming the legs are symmetric (see the module documentation).
pub struct Rig {
    pub relay: Arc<HostGatewayRelay>,
    pub public: Leg,
    pub upstream: Leg,
}

/// A fresh relay on the wall clock.
#[must_use]
pub fn host_relay() -> Arc<HostGatewayRelay> {
    Arc::new(HostGatewayRelay::new(Arc::new(WallClock)))
}

/// Absolute deadline `secs` from now.
#[must_use]
pub fn in_secs(secs: u64) -> Instant {
    Instant::now() + Duration::from_secs(secs)
}

/// `ReadDeadlines` for a read that must start and finish within `secs`.
#[must_use]
pub fn read_within(secs: u64) -> ReadDeadlines {
    ReadDeadlines { first_octet: in_secs(secs), complete_within: None, complete_by: Some(in_secs(secs)) }
}

/// `RelayDeadlines` with the given absolute bound and no-progress window.
#[must_use]
pub fn relay_within(absolute_secs: u64, no_progress: Duration) -> RelayDeadlines {
    RelayDeadlines { absolute: in_secs(absolute_secs), no_progress }
}

#[must_use]
pub fn nz64(n: u64) -> NonZeroU64 {
    NonZeroU64::new(n).expect("non-zero length")
}

#[must_use]
pub fn nzu(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero limit")
}

/// A loopback listener plus a thread that connects as `connect` describes.
/// Returns the accepted socket (non-blocking, as the production listener hands
/// it over) and the peer thread's result.
fn accept_with<T: Send + 'static>(
    connect: impl FnOnce(SocketAddr) -> T + Send + 'static,
) -> (OwnedFd, std::thread::JoinHandle<T>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("loopback listener");
    let addr = listener.local_addr().expect("listener address");
    let peer = std::thread::spawn(move || connect(addr));
    let (stream, _) = listener.accept().expect("accept the peer");
    stream.set_nonblocking(true).expect("non-blocking accepted socket");
    stream.set_nodelay(true).expect("nodelay");
    (OwnedFd::from(stream), peer)
}

/// Accepts one client that offers `offer` with ALPN `http/1.1` and SNI [`SNI`]
/// and returns the armed leg.
pub async fn accept_leg(relay: &dyn GatewayRelay, offer: Offer) -> Leg {
    accept_leg_with(relay, http_client_config(offer), sni_name()).await.expect("handshake succeeds")
}

/// As [`accept_leg`] with an explicit client configuration and server name; the
/// relay's typed refusal is returned instead of unwrapped.
pub async fn accept_leg_with(
    relay: &dyn GatewayRelay,
    config: Arc<ClientConfig>,
    name: ServerName<'static>,
) -> Result<Leg, PublicHandshakeError> {
    let (fd, peer) = accept_with(move |addr| KtlsPeer::connect(addr, config, name));
    let accepted = relay.accept_public(fd, server_config(), in_secs(10)).await;
    let peer = tokio::task::spawn_blocking(move || peer.join().expect("peer thread")).await.expect("join");
    let accepted = accepted?;
    let peer = peer.expect("peer completed the handshake the relay accepted");
    let (read, write) = accepted.leg.into_halves();
    Ok(Leg { read, write, peer, sni: accepted.sni })
}

/// Accepts a connection from a closure-defined client with a short passed
/// deadline: for the refusal cases whose peer is not a [`KtlsPeer`].
pub async fn accept_raw<T: Send + 'static>(
    relay: &dyn GatewayRelay,
    deadline: Instant,
    connect: impl FnOnce(SocketAddr) -> T + Send + 'static,
) -> (Result<overdrive_gateway::ports::AcceptedPublicLeg, PublicHandshakeError>, T) {
    let (fd, peer) = accept_with(connect);
    let accepted = relay.accept_public(fd, server_config(), deadline).await;
    let peer = tokio::task::spawn_blocking(move || peer.join().expect("peer thread")).await.expect("join");
    (accepted, peer)
}

impl Rig {
    pub async fn new(offer: Offer) -> Self {
        let relay = host_relay();
        let public = accept_leg(relay.as_ref(), offer).await;
        let upstream = accept_leg(relay.as_ref(), offer).await;
        Self { relay, public, upstream }
    }
}

/// Runs `f` on a blocking thread and awaits it.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.expect("blocking task")
}

/// A thread that streams `total` pattern octets (from stream offset `offset`)
/// through `peer` in 16 KiB sends, counting what the kernel accepted.
#[must_use]
pub fn stream_pattern(
    peer: &KtlsPeer,
    offset: usize,
    total: usize,
    sent: Arc<std::sync::atomic::AtomicU64>,
) -> std::thread::JoinHandle<io::Result<()>> {
    let peer = peer.clone();
    std::thread::spawn(move || {
        let mut at = 0;
        while at < total {
            let n = (16 * 1024).min(total - at);
            peer.send(&pattern(offset + at, n))?;
            sent.fetch_add(n as u64, Ordering::SeqCst);
            at += n;
        }
        Ok(())
    })
}

/// A thread that reads `head.len()` octets and requires them to equal `head`,
/// then verifies `body` pattern octets from `body_offset`.
#[must_use]
pub fn expect_head_then_pattern(
    peer: &KtlsPeer,
    head: Vec<u8>,
    body_offset: usize,
    body: usize,
) -> std::thread::JoinHandle<io::Result<()>> {
    let peer = peer.clone();
    std::thread::spawn(move || {
        let got = peer.read_exact_data(head.len())?;
        if got != head {
            return Err(io::Error::other(format!(
                "head differs: got {:?}",
                String::from_utf8_lossy(&got)
            )));
        }
        peer.verify_pattern(body_offset, body)
    })
}

/// A thread that reads exactly `expected.len()` octets and requires equality.
#[must_use]
pub fn expect_exact(peer: &KtlsPeer, expected: Vec<u8>) -> std::thread::JoinHandle<io::Result<()>> {
    let peer = peer.clone();
    std::thread::spawn(move || {
        let got = peer.read_exact_data(expected.len())?;
        if got == expected {
            Ok(())
        } else {
            let at = got.iter().zip(&expected).position(|(a, b)| a != b);
            Err(io::Error::other(format!("octets differ at {at:?} (len {})", expected.len())))
        }
    })
}

/// Joins a peer thread and requires success.
pub async fn joined(handle: std::thread::JoinHandle<io::Result<()>>, what: &str) {
    let result = blocking(move || handle.join().expect("peer thread did not panic")).await;
    assert!(result.is_ok(), "{what}: {result:?}");
}

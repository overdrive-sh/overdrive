//! Public-ingress-gateway end-to-end harness (native metal).
//!
//! One [`GatewayWorld`] is one production composition: `run_server(config,
//! NodeAdapters::host(..))` over the real cgroupfs, the real `EbpfDataplane`,
//! the real kernel-TLS relay and a real Cloud Hypervisor VM Service, with the
//! gateway enabled through `ServerConfig.gateway` (the gateway address, the
//! manual certified-key files). The hermetic `SimKek` replaces only the
//! production KEK, as every in-process `serve` fixture of this repository does
//! (the production credential keyring refuses a cold boot). Nothing is
//! installed that the production path does not install: the Service and the
//! Route enter through the server's public HTTPS API, and the external client
//! is a real TLS 1.3 client on a real socket (system trust is never used; the
//! test trust anchor is minted per composition).
//!
//! Kernel observations are independent oracles of the code under test:
//! `/proc/net/tls_stat`, `ss -tni`, `bpftool`, `/proc/self/fd`,
//! `/proc/self/statm`, `F_GETPIPE_SZ` and an `AF_PACKET` capture of loopback.
//!
//! # Gateway addresses (R-6)
//!
//! Each composition binds its own `127.<octet>.0.1:443`. This crate owns second
//! octets `100..=119`; the table lives in `tests/public_ingress_gateway_end_to_end.rs`.

#![allow(
    clippy::doc_markdown,
    reason = "TLS and kernel terms (close_notify, ULP) read as prose in the harness docs"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use overdrive_control_plane::{NodeAdapters, ServerConfig, ServerHandle, run_server};
use overdrive_core::guest_transport::NodeVsockPortConfig;
use overdrive_core::id::PublicCertifiedKeyId;
use overdrive_gateway::{GatewayConfig, ManualCertifiedKeyConfig};
use overdrive_host::RealCgroupFs;
use overdrive_sim::adapters::SimKek;
use overdrive_testing::vm_fixture::{VmFixture, default_staging_root};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, PKCS_ECDSA_P256_SHA256, date_time_ymd,
};
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{CipherSuite, ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::{
    CleanupObservation, PublicApi, TraceEvent, TraceHistory, assert_native_metal, command_output,
    wait_until,
};

// ---------------------------------------------------------------------------
// Fixed names and the first-slice limits the oracles are stated against
// ---------------------------------------------------------------------------

/// The Route hostname: URL host, SNI, `Host` header and certificate identity.
pub const PUBLIC_HOSTNAME: &str = "api.example.com";
/// The configured certified-key identity, byte-equal to the Route's.
pub const CERTIFIED_KEY_ID: &str = "api-origin";
/// The one Route's identifier.
pub const ROUTE_ID: &str = "public-api";
/// The VM Service's logical identifier.
pub const SERVICE_ID: &str = "api";
/// The Service's TCP listener port.
pub const SERVICE_PORT: u16 = 8080;
/// `GatewayLimits::first_slice`: the request and response body ceilings.
pub const BODY_CEILING_OCTETS: u64 = 16 * 1024 * 1024;
/// `GatewayLimits::first_slice`: the request and response head ceilings.
pub const HEAD_CEILING_OCTETS: usize = 32 * 1024;
/// `GatewayLimits::first_slice`: `max_public_connections`.
pub const MAX_PUBLIC_CONNECTIONS: u32 = 128;
/// ADR-0246: the relay pipe capacity (the empty-pipe invariant's floor).
pub const PIPE_CAPACITY_OCTETS: u64 = 65_536;
/// `upstream_connect_deadline` and `response_head_deadline`.
pub const FIVE_SECOND_DEADLINE: Duration = Duration::from_secs(5);
/// `body_no_progress_deadline`.
pub const BODY_NO_PROGRESS_DEADLINE: Duration = Duration::from_secs(30);
/// The one graceful drain of `ServerHandle::shutdown`.
pub const DRAIN_DEADLINE: Duration = Duration::from_secs(5);
/// Readiness window: at most this many polls, one per second, each bounded.
pub const READINESS_POLLS: u32 = 120;
/// The guest listens here for every flavour of the Service.
const EXAMPLE_GUEST_COMMAND: &str = "/opt/overdrive/examples/pig/api-server";
const CONFORMANCE_GUEST_COMMAND: &str = "/opt/overdrive/conformance/pig-guest";
/// The fixed materialisation `examples/public-ingress-gateway/prepare.sh` owns.
const EXAMPLE_OUTPUT_ROOT: &str = "/srv/vm/overdrive-testing/pig-example";

/// `pattern(i)`: the deterministic response/request body octet.
#[must_use]
pub const fn pattern(index: u64) -> u8 {
    (index.wrapping_mul(31).wrapping_add(7) % 251) as u8
}

/// `len` pattern octets starting at `offset`.
#[must_use]
pub fn pattern_bytes(offset: u64, len: usize) -> Vec<u8> {
    (0..len as u64).map(|i| pattern(offset + i)).collect()
}

/// The first index at which `bytes` leaves the pattern started at `offset`.
#[must_use]
pub fn first_pattern_mismatch(offset: u64, bytes: &[u8]) -> Option<usize> {
    bytes.iter().enumerate().position(|(i, byte)| *byte != pattern(offset + i as u64))
}

/// FNV-1a 64 over `bytes` (the digest the conformance guest reports).
#[must_use]
pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

// ---------------------------------------------------------------------------
// Test trust anchor and the Public Certified Key files
// ---------------------------------------------------------------------------

/// A bounded test CA and a leaf for [`PUBLIC_HOSTNAME`], written as the
/// operator's manual certified-key files (leaf-first chain, PKCS#8 key 0600).
pub struct PublicPki {
    /// The test trust anchor the external client trusts (and nothing else).
    pub ca_der: CertificateDer<'static>,
    /// Absolute leaf-first chain file.
    pub chain_path: PathBuf,
    /// Absolute PKCS#8 private-key file.
    pub key_path: PathBuf,
}

impl PublicPki {
    /// Mint the CA and the leaf under `dir`.
    #[must_use]
    pub fn mint(dir: &Path, hostname: &str) -> Self {
        std::fs::create_dir_all(dir).expect("create the PKI directory");
        let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("CA key");
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("CA params");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        ca_params.distinguished_name.push(DnType::CommonName, "Public Ingress Conformance CA");
        ca_params.not_before = date_time_ymd(2025, 1, 1);
        ca_params.not_after = date_time_ymd(2035, 1, 1);
        let ca_cert = ca_params.self_signed(&ca_key).expect("CA certificate");
        let issuer = Issuer::from_params(&ca_params, &ca_key);

        let leaf_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("leaf key");
        let mut leaf_params =
            CertificateParams::new(vec![hostname.to_owned()]).expect("leaf params");
        leaf_params.is_ca = IsCa::ExplicitNoCa;
        leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        leaf_params.distinguished_name.push(DnType::CommonName, hostname);
        leaf_params.not_before = date_time_ymd(2025, 1, 1);
        leaf_params.not_after = date_time_ymd(2035, 1, 1);
        let leaf_cert = leaf_params.signed_by(&leaf_key, &issuer).expect("leaf certificate");

        let chain_path = dir.join("api-origin-chain.pem");
        let key_path = dir.join("api-origin-key.pem");
        std::fs::write(&chain_path, format!("{}{}", leaf_cert.pem(), ca_cert.pem()))
            .expect("write the certificate chain");
        let mut key_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&key_path)
            .expect("create the private-key file");
        key_file.write_all(leaf_key.serialize_pem().as_bytes()).expect("write the private key");
        Self { ca_der: ca_cert.der().clone(), chain_path, key_path }
    }
}

/// Run blocking client or kernel-observation work off the async runtime that
/// hosts the gateway.
pub async fn blocking<R, F>(work: F) -> R
where
    R: Send + 'static,
    F: FnOnce() -> R + Send + 'static,
{
    tokio::task::spawn_blocking(work).await.expect("the blocking task completes")
}

// ---------------------------------------------------------------------------
// The external public client
// ---------------------------------------------------------------------------

/// Which TLS 1.3 suites the external client offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suite {
    /// Every suite the provider offers.
    Any,
    /// `TLS13_AES_128_GCM_SHA256` only.
    Aes128,
    /// `TLS13_AES_256_GCM_SHA384` only.
    Aes256,
}

/// How a TLS byte stream ended or stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEnd {
    /// The peer sent close_notify (a clean end).
    CloseNotify,
    /// The transport closed with no close_notify.
    Eof,
    /// The transport was reset (`ECONNRESET`, `ECONNABORTED`, `EPIPE`).
    Reset,
    /// The read timed out with the stream still open.
    Timeout,
    /// Any other I/O failure.
    Other(io::ErrorKind),
}

impl From<io::Error> for StreamEnd {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::UnexpectedEof => Self::Eof,
            io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe => Self::Reset,
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => Self::Timeout,
            other => Self::Other(other),
        }
    }
}

/// A response that did not reach its framed end.
#[derive(Debug)]
pub struct ResponseError {
    /// How the stream stopped.
    pub end: StreamEnd,
    /// The status line's code when the head had arrived.
    pub status: Option<u16>,
    /// Body octets received before the stop.
    pub partial_body: Vec<u8>,
}

/// One parsed response head.
#[derive(Debug, Clone)]
pub struct ResponseHead {
    /// Status code.
    pub status: u16,
    /// Header fields in order.
    pub headers: Vec<(String, String)>,
    /// Interim (`1xx`) heads read before this one.
    pub interim: Vec<u16>,
}

impl ResponseHead {
    /// The first header named `name` (case-insensitive).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(field, _)| field.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// One whole response.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// The head.
    pub head: ResponseHead,
    /// The decoded body.
    pub body: Vec<u8>,
    /// Chunk sizes when the body was chunked.
    pub chunk_sizes: Vec<usize>,
    /// Whether the body was delimited by the stream's end.
    pub close_delimited: bool,
}

impl HttpResponse {
    /// Status code.
    #[must_use]
    pub const fn status(&self) -> u16 {
        self.head.status
    }
}

/// One request, encoded exactly as written.
#[derive(Debug, Clone)]
pub struct Request {
    method: String,
    path: String,
    host: String,
    headers: Vec<(String, String)>,
    body: RequestBody,
}

/// How a request carries its body.
#[derive(Debug, Clone)]
pub enum RequestBody {
    /// No body, no framing header.
    None,
    /// `Content-Length` and these octets.
    Length(Vec<u8>),
    /// `Content-Length: declared` with no octets encoded (the test streams them).
    Declared(u64),
    /// `Transfer-Encoding: chunked` with these chunks.
    Chunked(Vec<Vec<u8>>),
}

impl Request {
    /// `GET path`.
    #[must_use]
    pub fn get(path: &str) -> Self {
        Self {
            method: "GET".to_owned(),
            path: path.to_owned(),
            host: PUBLIC_HOSTNAME.to_owned(),
            headers: Vec::new(),
            body: RequestBody::None,
        }
    }

    /// `POST path` with `body`.
    #[must_use]
    pub fn post(path: &str, body: RequestBody) -> Self {
        Self { method: "POST".to_owned(), body, ..Self::get(path) }
    }

    /// Replace the `Host` value.
    #[must_use]
    pub fn host(mut self, host: &str) -> Self {
        host.clone_into(&mut self.host);
        self
    }

    /// Append one header field.
    #[must_use]
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// The request head, ending at the blank line.
    #[must_use]
    pub fn head_bytes(&self) -> Vec<u8> {
        let mut text = format!("{} {} HTTP/1.1\r\nHost: {}\r\n", self.method, self.path, self.host);
        for (name, value) in &self.headers {
            let _ = write!(text, "{name}: {value}\r\n");
        }
        match &self.body {
            RequestBody::None => {}
            RequestBody::Length(bytes) => {
                let _ = write!(text, "Content-Length: {}\r\n", bytes.len());
            }
            RequestBody::Declared(declared) => {
                let _ = write!(text, "Content-Length: {declared}\r\n");
            }
            RequestBody::Chunked(_) => text.push_str("Transfer-Encoding: chunked\r\n"),
        }
        text.push_str("\r\n");
        text.into_bytes()
    }

    /// The whole request: the head and, where encoded, the body.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = self.head_bytes();
        match &self.body {
            RequestBody::None | RequestBody::Declared(_) => {}
            RequestBody::Length(body) => bytes.extend_from_slice(body),
            RequestBody::Chunked(chunks) => {
                for chunk in chunks {
                    bytes.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
                    bytes.extend_from_slice(chunk);
                    bytes.extend_from_slice(b"\r\n");
                }
                bytes.extend_from_slice(b"0\r\n\r\n");
            }
        }
        bytes
    }
}

/// The external user: a real TLS 1.3 client on a real socket.
///
/// Its only trust anchor is the composition's test CA, with the Route hostname
/// as SNI and the address override applied client-side (the gateway's distinct
/// `127/8` address stands for the operator's A record).
#[derive(Clone)]
pub struct PublicClient {
    address: SocketAddr,
    roots: Arc<RootCertStore>,
}

impl PublicClient {
    /// A client for the gateway at `address:443` trusting `pki`'s CA.
    #[must_use]
    pub fn new(address: Ipv4Addr, pki: &PublicPki) -> Self {
        let mut roots = RootCertStore::empty();
        roots.add(pki.ca_der.clone()).expect("trust the test CA");
        Self { address: SocketAddr::V4(SocketAddrV4::new(address, 443)), roots: Arc::new(roots) }
    }

    /// The gateway's public address.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    fn config(&self, suite: Suite) -> Arc<ClientConfig> {
        let mut provider = rustls::crypto::ring::default_provider();
        provider.cipher_suites.retain(|offered| {
            matches!(
                (suite, offered.suite()),
                (Suite::Any, _)
                    | (Suite::Aes128, CipherSuite::TLS13_AES_128_GCM_SHA256)
                    | (Suite::Aes256, CipherSuite::TLS13_AES_256_GCM_SHA384)
            )
        });
        let mut config = ClientConfig::builder_with_provider(Arc::new(provider))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("TLS 1.3")
            .with_root_certificates((*self.roots).clone())
            .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Arc::new(config)
    }

    /// A TLS 1.3 connection offering every suite, with the Route hostname as SNI.
    ///
    /// # Errors
    ///
    /// The TCP connect or the handshake failed.
    pub fn connect(&self) -> io::Result<PublicConnection> {
        self.connect_with(Suite::Any, PUBLIC_HOSTNAME)
    }

    /// A TLS 1.3 connection offering `suite`, with `sni` as the server name.
    ///
    /// # Errors
    ///
    /// The TCP connect or the handshake failed.
    pub fn connect_with(&self, suite: Suite, sni: &str) -> io::Result<PublicConnection> {
        let mut tcp = TcpStream::connect_timeout(&self.address, FIVE_SECOND_DEADLINE)?;
        tcp.set_nodelay(true)?;
        tcp.set_read_timeout(Some(FIVE_SECOND_DEADLINE))?;
        tcp.set_write_timeout(Some(FIVE_SECOND_DEADLINE))?;
        let name = ServerName::try_from(sni.to_owned()).map_err(io::Error::other)?;
        let mut connection =
            ClientConnection::new(self.config(suite), name).map_err(io::Error::other)?;
        while connection.is_handshaking() {
            connection.complete_io(&mut tcp)?;
        }
        Ok(PublicConnection { stream: StreamOwned::new(connection, tcp), pending: Vec::new() })
    }

    /// A bare TCP connection with no TLS octet written.
    ///
    /// # Errors
    ///
    /// The TCP connect failed.
    pub fn raw_tcp(&self) -> io::Result<TcpStream> {
        TcpStream::connect_timeout(&self.address, FIVE_SECOND_DEADLINE)
    }

    /// Whether the gateway closes a connection before any TLS octet in either
    /// direction: the first read returns end-of-stream or a reset, never data.
    ///
    /// # Errors
    ///
    /// A description of what the connection did instead.
    pub fn closed_before_any_tls_octet(&self, observe: Duration) -> Result<(), String> {
        let mut tcp = self.raw_tcp().map_err(|error| format!("connect: {error}"))?;
        tcp.set_read_timeout(Some(observe)).map_err(|error| error.to_string())?;
        let mut byte = [0_u8; 1];
        match tcp.read(&mut byte) {
            Ok(0) => Ok(()),
            Ok(_) => Err("the gateway wrote an octet to a connection beyond the cap".to_owned()),
            Err(error) => match StreamEnd::from(error) {
                StreamEnd::Reset => Ok(()),
                other => Err(format!("the connection stayed open: {other:?}")),
            },
        }
    }
}

/// What happened to a client that kept sending a body after a rejection.
#[derive(Debug)]
pub struct LingerOutcome {
    /// The response status read, if any.
    pub status: Option<u16>,
    /// The first write failure, if the transport rejected a write.
    pub write_failure: Option<StreamEnd>,
    /// Body octets written.
    pub sent: u64,
    /// How the stream ended once the client closed its side.
    pub end: StreamEnd,
}

impl PublicClient {
    /// Send `head`, then keep writing up to `body_octets` of body in 64 KiB
    /// pieces while reading the response; then send close_notify and read to
    /// the end. A gateway that answers and then resets the connection shows as
    /// a write failure or a `Reset` end.
    pub fn send_body_after_rejection(&self, head: &[u8], body_octets: u64) -> LingerOutcome {
        let mut connection = self.connect().expect("connect for the rejection stimulus");
        let mut status = None;
        let mut write_failure = None;
        let mut sent = 0_u64;
        if let Err(error) = connection.send(head) {
            write_failure = Some(StreamEnd::from(error));
        }
        let piece = pattern_bytes(0, 64 * 1024);
        while write_failure.is_none() && sent < body_octets {
            let take = usize::try_from((body_octets - sent).min(piece.len() as u64)).expect("fits");
            if let Err(error) = connection.send(&piece[..take]) {
                write_failure = Some(StreamEnd::from(error));
                break;
            }
            sent += take as u64;
            if status.is_none() {
                connection.set_read_timeout(Some(Duration::from_millis(5)));
                if let Ok(head) = connection.read_head() {
                    status = Some(head.status);
                    connection.set_read_timeout(Some(Duration::from_secs(2)));
                    let _ = connection.read_body(&head, false);
                }
            }
        }
        if status.is_none() {
            connection.set_read_timeout(Some(Duration::from_secs(5)));
            status = connection.read_head().ok().map(|head| head.status);
        }
        let _ = connection.send_close_notify();
        let (end, _) = connection.read_to_end(Duration::from_secs(10));
        LingerOutcome { status, write_failure, sent, end }
    }
}

/// One established public TLS connection.
pub struct PublicConnection {
    stream: StreamOwned<ClientConnection, TcpStream>,
    pending: Vec<u8>,
}

impl PublicConnection {
    /// The TCP socket (timeouts, descriptor).
    #[must_use]
    pub const fn tcp(&self) -> &TcpStream {
        &self.stream.sock
    }

    /// The negotiated TLS 1.3 suite.
    #[must_use]
    pub fn negotiated_suite(&self) -> Option<CipherSuite> {
        self.stream.conn.negotiated_cipher_suite().map(|suite| suite.suite())
    }

    /// Bound every following read by `timeout`.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) {
        self.stream.sock.set_read_timeout(timeout).expect("set the read timeout");
    }

    /// Write `bytes` and flush.
    ///
    /// # Errors
    ///
    /// The write failed (including a reset).
    pub fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.stream.write_all(bytes)?;
        self.stream.flush()
    }

    /// Send our close_notify and flush it; the read side stays open.
    ///
    /// # Errors
    ///
    /// The write failed.
    pub fn send_close_notify(&mut self) -> io::Result<()> {
        self.stream.conn.send_close_notify();
        while self.stream.conn.wants_write() {
            self.stream.conn.write_tls(&mut self.stream.sock)?;
        }
        Ok(())
    }

    fn fill(&mut self) -> Result<usize, StreamEnd> {
        let mut buf = vec![0_u8; 16 * 1024];
        match self.stream.read(&mut buf) {
            Ok(0) => Err(StreamEnd::CloseNotify),
            Ok(n) => {
                self.pending.extend_from_slice(&buf[..n]);
                Ok(n)
            }
            Err(error) => Err(StreamEnd::from(error)),
        }
    }

    fn take_line_end(&self, from: usize) -> Option<usize> {
        self.pending[from..].windows(2).position(|pair| pair == b"\r\n").map(|at| from + at)
    }

    /// Read one final response head, skipping interim heads.
    ///
    /// # Errors
    ///
    /// The stream stopped before a final head arrived.
    pub fn read_head(&mut self) -> Result<ResponseHead, ResponseError> {
        let mut interim = Vec::new();
        loop {
            let end = loop {
                if let Some(at) = self.pending.windows(4).position(|window| window == b"\r\n\r\n") {
                    break at + 4;
                }
                self.fill().map_err(|end| ResponseError {
                    end,
                    status: None,
                    partial_body: Vec::new(),
                })?;
            };
            let text = String::from_utf8_lossy(&self.pending[..end]).into_owned();
            self.pending.drain(..end);
            let mut lines = text.split("\r\n");
            let status_line = lines.next().unwrap_or_default();
            let status: u16 = status_line
                .split(' ')
                .nth(1)
                .and_then(|code| code.parse().ok())
                .unwrap_or_else(|| panic!("a parsable status line, got {status_line:?}"));
            let headers = lines
                .filter(|line| !line.is_empty())
                .filter_map(|line| line.split_once(':'))
                .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
                .collect();
            if (100..200).contains(&status) && status != 101 {
                interim.push(status);
                continue;
            }
            return Ok(ResponseHead { status, headers, interim });
        }
    }

    /// Read the body `head` frames, then the end of the message.
    ///
    /// # Errors
    ///
    /// The stream stopped before the framed end; the partial body is retained.
    pub fn read_body(
        &mut self,
        head: &ResponseHead,
        head_request: bool,
    ) -> Result<(Vec<u8>, Vec<usize>, bool), ResponseError> {
        let status = head.status;
        let fail = |end: StreamEnd, body: Vec<u8>| ResponseError {
            end,
            status: Some(status),
            partial_body: body,
        };
        if head_request || status == 204 || status == 304 {
            return Ok((Vec::new(), Vec::new(), false));
        }
        if head.header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
            let mut body = Vec::new();
            let mut sizes = Vec::new();
            loop {
                let line_end = loop {
                    if let Some(at) = self.take_line_end(0) {
                        break at;
                    }
                    self.fill().map_err(|end| fail(end, body.clone()))?;
                };
                let line = String::from_utf8_lossy(&self.pending[..line_end]).into_owned();
                self.pending.drain(..line_end + 2);
                let size = usize::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16)
                    .unwrap_or_else(|_| panic!("a parsable chunk size line, got {line:?}"));
                if size == 0 {
                    // Trailers end at the blank line.
                    loop {
                        let at = loop {
                            if let Some(at) = self.take_line_end(0) {
                                break at;
                            }
                            self.fill().map_err(|end| fail(end, body.clone()))?;
                        };
                        let blank = at == 0;
                        self.pending.drain(..at + 2);
                        if blank {
                            return Ok((body, sizes, false));
                        }
                    }
                }
                while self.pending.len() < size + 2 {
                    self.fill().map_err(|end| fail(end, body.clone()))?;
                }
                body.extend_from_slice(&self.pending[..size]);
                self.pending.drain(..size + 2);
                sizes.push(size);
            }
        }
        if let Some(length) = head.header("content-length").and_then(|v| v.parse::<usize>().ok()) {
            let mut body = Vec::with_capacity(length);
            while body.len() < length {
                if self.pending.is_empty() {
                    self.fill().map_err(|end| fail(end, body.clone()))?;
                }
                let take = (length - body.len()).min(self.pending.len());
                body.extend(self.pending.drain(..take));
            }
            return Ok((body, Vec::new(), false));
        }
        // Close-delimited: ends at close_notify.
        let mut body = std::mem::take(&mut self.pending);
        loop {
            match self.fill() {
                Ok(_) => body.append(&mut self.pending),
                Err(StreamEnd::CloseNotify) => return Ok((body, Vec::new(), true)),
                Err(end) => return Err(fail(end, body)),
            }
        }
    }

    /// Read one whole response.
    ///
    /// # Errors
    ///
    /// The stream stopped before the framed end.
    pub fn read_response(&mut self, head_request: bool) -> Result<HttpResponse, ResponseError> {
        let head = self.read_head()?;
        let (body, chunk_sizes, close_delimited) = self.read_body(&head, head_request)?;
        Ok(HttpResponse { head, body, chunk_sizes, close_delimited })
    }

    /// Send `request` and read its response.
    ///
    /// # Errors
    ///
    /// The stream stopped before the framed end (a failed send is reported as
    /// the matching [`StreamEnd`]).
    pub fn exchange(&mut self, request: &Request) -> Result<HttpResponse, ResponseError> {
        self.send(&request.encode()).map_err(|error| ResponseError {
            end: StreamEnd::from(error),
            status: None,
            partial_body: Vec::new(),
        })?;
        self.read_response(request.method == "HEAD")
    }

    /// Read until the stream ends within `bound`; returns how it ended and how
    /// many octets arrived meanwhile.
    pub fn read_to_end(&mut self, bound: Duration) -> (StreamEnd, usize) {
        let deadline = Instant::now() + bound;
        let mut octets = self.pending.len();
        self.pending.clear();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return (StreamEnd::Timeout, octets);
            }
            self.set_read_timeout(Some(left.min(Duration::from_secs(1))));
            match self.fill() {
                Ok(n) => {
                    octets += n;
                    self.pending.clear();
                }
                Err(StreamEnd::Timeout) => {}
                Err(end) => return (end, octets),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Kernel and process observations (independent of the code under test)
// ---------------------------------------------------------------------------

/// `/proc/net/tls_stat` (per network namespace, node-global).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TlsStat(BTreeMap<String, u64>);

impl TlsStat {
    /// Read the counters.
    #[must_use]
    pub fn read() -> Self {
        let text = std::fs::read_to_string("/proc/net/tls_stat")
            .expect("/proc/net/tls_stat is readable on a kernel with the tls module");
        Self(
            text.lines()
                .filter_map(|line| {
                    let mut parts = line.split_whitespace();
                    let name = parts.next()?;
                    Some((name.to_owned(), parts.next()?.parse::<u64>().ok()?))
                })
                .collect(),
        )
    }

    /// The value of `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> u64 {
        *self.0.get(name).unwrap_or_else(|| panic!("tls_stat has no counter {name}"))
    }

    /// `self - before` for `name`.
    #[must_use]
    pub fn delta(&self, before: &Self, name: &str) -> i64 {
        i64::try_from(self.get(name)).expect("counter fits")
            - i64::try_from(before.get(name)).expect("counter fits")
    }
}

/// One established TCP socket from `ss -tni`.
#[derive(Debug, Clone)]
pub struct SsSocket {
    /// Local address.
    pub local: SocketAddr,
    /// Peer address.
    pub peer: SocketAddr,
    /// Whether the kernel-TLS ULP is attached to it.
    pub tls_ulp: bool,
}

/// Established sockets with their ULP, read with `ss` (an independent kernel
/// oracle; there is no typed API for socket diagnostics with ULP info here).
#[must_use]
pub fn established_sockets() -> Vec<SsSocket> {
    let output = command_output("ss", &["-H", "-t", "-n", "-i", "state", "established"]);
    assert!(output.status.success(), "ss -tni failed: {}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8_lossy(&output.stdout);
    let mut sockets: Vec<SsSocket> = Vec::new();
    for line in text.lines() {
        if line.starts_with(char::is_whitespace) {
            if let Some(last) = sockets.last_mut() {
                last.tls_ulp |= line.contains("tcp-ulp-tls");
            }
            continue;
        }
        let addresses: Vec<SocketAddr> =
            line.split_whitespace().filter_map(|token| token.parse().ok()).collect();
        if let [.., local, peer] = addresses.as_slice() {
            sockets.push(SsSocket {
                local: *local,
                peer: *peer,
                tls_ulp: line.contains("tcp-ulp-tls"),
            });
        }
    }
    sockets
}

/// The gateway's public-leg sockets: established, local `address:443`.
#[must_use]
pub fn public_leg_sockets(address: Ipv4Addr) -> Vec<SsSocket> {
    established_sockets()
        .into_iter()
        .filter(|socket| socket.local == SocketAddr::from((address, 443)))
        .collect()
}

/// Upstream-leg sockets: armed loopback sockets that are not on a public
/// listener (the gateway's dial and leg-C's accepted end).
#[must_use]
pub fn armed_upstream_sockets(address: Ipv4Addr) -> Vec<SsSocket> {
    established_sockets()
        .into_iter()
        .filter(|socket| {
            socket.tls_ulp
                && socket.local.ip() == Ipv4Addr::LOCALHOST
                && socket.peer.ip() == Ipv4Addr::LOCALHOST
                && socket.local.port() != 443
                && socket.peer.port() != 443
                && socket.local != SocketAddr::from((address, 443))
        })
        .collect()
}

/// The process's open descriptor count.
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
    let page =
        u64::try_from(unsafe { nix::libc::sysconf(nix::libc::_SC_PAGESIZE) }).unwrap_or(4096);
    pages * page / 1024
}

/// The summed capacity (`F_GETPIPE_SZ`) in octets of the distinct pipes this
/// process holds descriptors for.
#[must_use]
pub fn pipe_capacity_octets() -> u64 {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut total = 0_u64;
    for entry in std::fs::read_dir("/proc/self/fd").expect("/proc/self/fd is readable").flatten() {
        let Ok(target) = std::fs::read_link(entry.path()) else { continue };
        let target = target.to_string_lossy().into_owned();
        if !target.starts_with("pipe:") || !seen.insert(target) {
            continue;
        }
        let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NONBLOCK)
            .open(entry.path())
        else {
            continue;
        };
        if let Ok(size) = nix::fcntl::fcntl(&file, nix::fcntl::FcntlArg::F_GETPIPE_SZ) {
            total += u64::try_from(size).unwrap_or(0);
        }
    }
    total
}

/// `RLIMIT_NOFILE` (soft, hard), reported and never raised by the test.
#[must_use]
pub fn nofile_limit() -> (u64, u64) {
    nix::sys::resource::getrlimit(nix::sys::resource::Resource::RLIMIT_NOFILE)
        .expect("getrlimit(RLIMIT_NOFILE)")
}

fn bpftool_json(args: &[&str]) -> serde_json::Value {
    let mut full = vec!["-j"];
    full.extend_from_slice(args);
    let output = command_output("bpftool", &full);
    assert!(
        output.status.success(),
        "bpftool {full:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("bpftool emits JSON")
}

/// Entries in the one loaded map whose name starts with `prefix` (kernel map
/// names are truncated to 15 octets). Read with `bpftool`, an independent oracle.
#[must_use]
pub fn bpf_map_entries(prefix: &str) -> usize {
    let maps = bpftool_json(&["map", "show"]);
    let matching: Vec<u64> = maps
        .as_array()
        .expect("bpftool map show lists maps")
        .iter()
        .filter(|map| map["name"].as_str().is_some_and(|name| name.starts_with(prefix)))
        .filter_map(|map| map["id"].as_u64())
        .collect();
    let [id] = matching.as_slice() else {
        panic!("exactly one loaded map named {prefix}*, got {matching:?}");
    };
    bpftool_json(&["map", "dump", "id", &id.to_string()]).as_array().map_or(0, Vec::len)
}

/// The live gateway intent and receipt counts read from the loaded maps:
/// `(intents, receipts)`.
#[must_use]
pub fn live_gateway_counts() -> (usize, usize) {
    (bpf_map_entries("GATEWAY_CONNECT"), bpf_map_entries("GATEWAY_SELECT"))
}

/// Kernel TLS state at one instant: the node-global counters and the armed
/// legs of the gateway at `address`.
#[derive(Debug, Clone)]
pub struct KernelSample {
    /// `/proc/net/tls_stat`.
    pub tls: TlsStat,
    /// Public-leg sockets with the kernel-TLS ULP attached.
    pub public_armed: usize,
    /// Upstream-leg sockets (the gateway's dial and its peer) with the ULP.
    pub upstream_armed: usize,
}

/// Sample the kernel TLS state for the gateway at `address`.
#[must_use]
pub fn kernel_sample(address: Ipv4Addr) -> KernelSample {
    KernelSample {
        tls: TlsStat::read(),
        public_armed: public_leg_sockets(address).iter().filter(|s| s.tls_ulp).count(),
        upstream_armed: armed_upstream_sockets(address).len(),
    }
}

/// The pids of running `cloud-hypervisor` processes (matched on `argv[0]`,
/// not the 15-octet `comm`).
#[must_use]
pub fn vmm_pids() -> Vec<u32> {
    let mut pids = Vec::new();
    for entry in std::fs::read_dir("/proc").expect("/proc is readable").flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
        let argv0 = cmdline.split(|byte| *byte == 0).next().unwrap_or_default();
        if String::from_utf8_lossy(argv0).rsplit('/').next() == Some("cloud-hypervisor") {
            pids.push(pid);
        }
    }
    pids
}

/// Kill the Service's VMM from the host, the way a host-side `SIGKILL` ends it.
pub fn kill_vmm() {
    let pids = vmm_pids();
    assert!(!pids.is_empty(), "a running VMM exists to kill");
    for pid in pids {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(i32::try_from(pid).expect("pid fits")),
            nix::sys::signal::Signal::SIGKILL,
        )
        .expect("SIGKILL the VMM");
    }
}

/// Entries of the pinned outer `SERVICE_MAP` (one per programmed Service).
#[must_use]
pub fn service_map_entries() -> usize {
    bpftool_json(&["map", "dump", "pinned", "/sys/fs/bpf/overdrive/SERVICE_MAP"])
        .as_array()
        .map_or(0, Vec::len)
}

// ---------------------------------------------------------------------------
// Loopback wire capture (AF_PACKET): the leg-B/leg-C segment
// ---------------------------------------------------------------------------

/// One captured TCP segment with a payload.
#[derive(Debug, Clone)]
pub struct Segment {
    /// Source.
    pub src: SocketAddrV4,
    /// Destination.
    pub dst: SocketAddrV4,
    /// TCP payload octets.
    pub payload: Vec<u8>,
}

/// A running loopback capture.
pub struct WireCapture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Vec<Segment>>>,
}

impl WireCapture {
    /// Start capturing every TCP segment with a payload on loopback.
    #[must_use]
    pub fn start() -> Self {
        use nix::sys::socket::{
            AddressFamily, MsgFlags, SockFlag, SockProtocol, SockType, recv, setsockopt, socket,
            sockopt::RcvBuf,
        };
        let fd =
            socket(AddressFamily::Packet, SockType::Raw, SockFlag::empty(), SockProtocol::EthAll)
                .expect("open an AF_PACKET socket (root required)");
        let _ = setsockopt(&fd, RcvBuf, &(16 * 1024 * 1024));
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            let mut segments = Vec::new();
            let mut frame = vec![0_u8; 70_000];
            loop {
                match recv(fd.as_raw_fd(), &mut frame, MsgFlags::MSG_DONTWAIT) {
                    Ok(n) if n > 0 => {
                        if let Some(segment) = parse_segment(&frame[..n]) {
                            segments.push(segment);
                        }
                    }
                    _ if flag.load(Ordering::SeqCst) => return segments,
                    _ => thread::sleep(Duration::from_millis(2)),
                }
            }
        });
        Self { stop, thread: Some(thread) }
    }

    /// Stop and return what was captured.
    #[must_use]
    pub fn finish(mut self) -> WireEvidence {
        thread::sleep(Duration::from_millis(100));
        self.stop.store(true, Ordering::SeqCst);
        let segments =
            self.thread.take().expect("capture thread").join().expect("capture thread joins");
        WireEvidence { segments }
    }
}

fn parse_segment(frame: &[u8]) -> Option<Segment> {
    // Ethernet (14) + IPv4 + TCP.
    let ip = frame.get(14..)?;
    if ip.first()? >> 4 != 4 || *ip.get(9)? != 6 {
        return None;
    }
    let ihl = usize::from(ip[0] & 0x0f) * 4;
    let total = usize::from(u16::from_be_bytes([ip[2], ip[3]]));
    let tcp = ip.get(ihl..total.min(ip.len()))?;
    let offset = usize::from(tcp.get(12)? >> 4) * 4;
    let payload = tcp.get(offset..)?;
    if payload.is_empty() {
        return None;
    }
    let src = SocketAddrV4::new(
        Ipv4Addr::new(ip[12], ip[13], ip[14], ip[15]),
        u16::from_be_bytes([tcp[0], tcp[1]]),
    );
    let dst = SocketAddrV4::new(
        Ipv4Addr::new(ip[16], ip[17], ip[18], ip[19]),
        u16::from_be_bytes([tcp[2], tcp[3]]),
    );
    Some(Segment { src, dst, payload: payload.to_vec() })
}

/// What a [`WireCapture`] saw.
pub struct WireEvidence {
    segments: Vec<Segment>,
}

impl WireEvidence {
    /// Segments of flows touching neither port in `excluding_ports`.
    #[must_use]
    pub fn segments_excluding_ports(&self, excluding_ports: &[u16]) -> Vec<&Segment> {
        self.segments
            .iter()
            .filter(|s| {
                !excluding_ports.contains(&s.src.port()) && !excluding_ports.contains(&s.dst.port())
            })
            .collect()
    }

    /// Whether any captured payload anywhere contains `needle`.
    #[must_use]
    pub fn any_payload_contains(&self, needle: &[u8]) -> bool {
        self.segments.iter().any(|s| s.payload.windows(needle.len()).any(|w| w == needle))
    }
}

/// The TLS record content types seen at the start of payloads in `segments`
/// (`0x16` handshake, `0x17` application data), accepting only TLS 1.2/1.3
/// record versions.
#[must_use]
pub fn tls_record_types(segments: &[&Segment]) -> BTreeSet<u8> {
    segments
        .iter()
        .filter(|s| s.payload.len() > 5 && s.payload[1] == 3 && s.payload[2] <= 4)
        .map(|s| s.payload[0])
        .collect()
}

// ---------------------------------------------------------------------------
// Tracing occurrences
// ---------------------------------------------------------------------------

/// The receipt occurrences with `operation` among `events`.
#[must_use]
pub fn receipt_occurrences<'a>(events: &'a [TraceEvent], operation: &str) -> Vec<&'a TraceEvent> {
    events
        .iter()
        .filter(|event| {
            event.fields.get("event").map(String::as_str)
                == Some("gateway.upstream.receipt.validated")
                && event.fields.get("operation").map(String::as_str) == Some(operation)
        })
        .collect()
}

/// `(stage, "begin" | "complete")` in emission order for the shutdown stages.
#[must_use]
pub fn shutdown_stage_events(events: &[TraceEvent]) -> Vec<(String, &'static str)> {
    events
        .iter()
        .filter_map(|event| {
            let phase = match event.name.as_str() {
                "serve.shutdown.stage_begin" => "begin",
                "serve.shutdown.stage_complete" => "complete",
                _ => return None,
            };
            Some((event.fields.get("stage")?.clone(), phase))
        })
        .collect()
}

/// The `reason` of every `health.startup.refused` occurrence among `events`.
#[must_use]
pub fn startup_refusal_reasons(events: &[TraceEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.name == "health.startup.refused")
        .filter_map(|event| event.fields.get("reason").cloned())
        .collect()
}

// ---------------------------------------------------------------------------
// The VM Service guests
// ---------------------------------------------------------------------------

/// Which guest serves the Service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestKind {
    /// The checked-in example guest, materialised by
    /// `examples/public-ingress-gateway/prepare.sh` (the walking skeleton).
    Example,
    /// The conformance guest (`guest/pig_guest.rs`): the example's superset.
    Conformance,
}

struct StagedGuest {
    kernel: PathBuf,
    rootfs: PathBuf,
    command: &'static str,
    example_owned: bool,
}

fn example_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/public-ingress-gateway")
}

fn run_checked(program: &str, args: &[&str]) {
    let output = command_output(program, args);
    assert!(
        output.status.success(),
        "{program} {args:?} failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stage_guest(kind: GuestKind, work: &Path) -> StagedGuest {
    let fixture = VmFixture::provision(&default_staging_root()).expect("provision the VM fixture");
    match kind {
        GuestKind::Example => {
            let script = example_dir().join("prepare.sh");
            let script = script.to_string_lossy().into_owned();
            // A previous run may have left its marker-owned materialisation.
            let _ = command_output("bash", &[&script, "cleanup"]);
            run_checked("bash", &[&script, "prepare"]);
            StagedGuest {
                kernel: Path::new(EXAMPLE_OUTPUT_ROOT).join("kernel"),
                rootfs: Path::new(EXAMPLE_OUTPUT_ROOT).join("rootfs.ext4"),
                command: EXAMPLE_GUEST_COMMAND,
                example_owned: true,
            }
        }
        GuestKind::Conformance => {
            let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("guest/pig_guest.rs");
            let binary = work.join("pig-guest");
            run_checked(
                "rustc",
                &[
                    "--edition",
                    "2021",
                    "-C",
                    "opt-level=2",
                    "-C",
                    "target-feature=+crt-static",
                    "--target",
                    "x86_64-unknown-linux-musl",
                    "-o",
                    &binary.to_string_lossy(),
                    &source.to_string_lossy(),
                ],
            );
            // A private rootfs copy on the staging filesystem (the per-launch
            // clone is intra-filesystem), the guest binary injected through a
            // loop mount on the host before the guest ever boots. There is no
            // typed API for loop devices; the mount and the loop device are
            // released before this returns.
            let rootfs = work.join("rootfs.ext4");
            std::fs::copy(&fixture.rootfs_path, &rootfs).expect("copy the base rootfs");
            let mountpoint = work.join("rootfs-mnt");
            std::fs::create_dir_all(&mountpoint).expect("create the mount point");
            let attach =
                command_output("losetup", &["--find", "--show", &rootfs.to_string_lossy()]);
            assert!(
                attach.status.success(),
                "losetup failed: {}",
                String::from_utf8_lossy(&attach.stderr)
            );
            let loop_device = String::from_utf8_lossy(&attach.stdout).trim().to_owned();
            let mounted = command_output("mount", &[&loop_device, &mountpoint.to_string_lossy()]);
            if mounted.status.success() {
                let target = mountpoint.join("opt/overdrive/conformance");
                std::fs::create_dir_all(&target).expect("create the guest directory");
                let dest = target.join("pig-guest");
                std::fs::copy(&binary, &dest).expect("install the guest binary");
                std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
                    .expect("chmod the guest binary");
                run_checked("umount", &[&mountpoint.to_string_lossy()]);
            }
            run_checked("losetup", &["-d", &loop_device]);
            assert!(
                mounted.status.success(),
                "mount of the private rootfs failed: {}",
                String::from_utf8_lossy(&mounted.stderr)
            );
            StagedGuest {
                kernel: fixture.kernel_path,
                rootfs,
                command: CONFORMANCE_GUEST_COMMAND,
                example_owned: false,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// One production composition
// ---------------------------------------------------------------------------

/// One production composition with the gateway enabled on its own address.
pub struct GatewayWorld {
    root: tempfile::TempDir,
    data_dir: PathBuf,
    config_dir: PathBuf,
    pki: PublicPki,
    address: Ipv4Addr,
    handle: Option<ServerHandle>,
    api: PublicApi,
    staged: StagedGuest,
    trace: TraceHistory,
    cleanup: CleanupObservation,
}

/// The cgroup directory every Service allocation scope lives under.
const WORKLOAD_SCOPES: &str = "/sys/fs/cgroup/overdrive.slice/workloads.slice";

impl GatewayWorld {
    /// Compose the production server over the real adapters with the gateway
    /// enabled on `127.<second_octet>.0.1:443`, staging `guest` first.
    pub async fn boot(trace: &TraceHistory, second_octet: u8, guest: GuestKind) -> Self {
        assert_native_metal();
        overdrive_testing::forwarder_module::expect_forwarder_module();
        let cleanup = CleanupObservation::capture(&[Path::new(WORKLOAD_SCOPES)]);
        // The serve data directory lives on the reflink-capable staging
        // filesystem: each per-launch rootfs clone is intra-filesystem.
        let root = tempfile::Builder::new()
            .prefix("pig-conformance-")
            .tempdir_in(default_staging_root())
            .expect("conformance directory on the staging filesystem");
        let data_dir = root.path().join("data");
        let config_dir = root.path().join("conf");
        tokio::fs::create_dir_all(&data_dir).await.expect("create the data dir");
        tokio::fs::create_dir_all(&config_dir).await.expect("create the operator config dir");
        let pki = PublicPki::mint(&root.path().join("pki"), PUBLIC_HOSTNAME);
        let staged = stage_guest(guest, root.path());
        let address = Ipv4Addr::new(127, second_octet, 0, 1);
        let (handle, api) = start_server(&data_dir, &config_dir, address, &pki).await;
        Self {
            root,
            data_dir,
            config_dir,
            pki,
            address,
            handle: Some(handle),
            api,
            staged,
            trace: trace.clone(),
            cleanup,
        }
    }

    /// Boot, deploy the Service, declare the whole-host Route and wait until
    /// the public URL answers (the readiness window already bounds the
    /// allocation reaching Running).
    pub async fn serving(trace: &TraceHistory, second_octet: u8, guest: GuestKind) -> Self {
        let world = Self::boot(trace, second_octet, guest).await;
        world.deploy_service().await;
        world.declare_route().await;
        world.wait_until_serving().await;
        world
    }

    /// Wait (at most 120 s) for one desired and one Running replica.
    pub async fn wait_running(&self) {
        tokio::time::timeout(Duration::from_secs(120), async {
            while !self.api.one_replica_running(SERVICE_ID).await {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
        .await
        .expect("the Service allocation reaches Running within 120 s");
    }

    /// Wait (at most `bound`) for `listener_bound` in the gateway status.
    pub async fn wait_listener_bound(&self, bound: Duration) {
        tokio::time::timeout(bound, async {
            while !self.gateway_status().await["listener_bound"].as_bool().unwrap_or(false) {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        })
        .await
        .expect("the public listener binds");
    }

    /// The workload's request-count marker, read through the public Route
    /// (`/count`; health and count requests are not counted by the guest).
    pub async fn workload_request_count(&self) -> u64 {
        let client = self.client();
        blocking(move || {
            let response = client
                .connect()
                .expect("connect for /count")
                .exchange(&Request::get("/count"))
                .expect("GET /count");
            assert_eq!(response.status(), 200, "the marker is readable through the Route");
            String::from_utf8_lossy(&response.body)
                .trim()
                .strip_prefix("requests=")
                .and_then(|n| n.parse().ok())
                .expect("a requests=<N> marker")
        })
        .await
    }

    /// Stop the Service and wait (at most 20 s) until no replica runs.
    pub async fn stop_service_and_wait(&self) {
        self.stop_service().await;
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let stopped = match self.api.get(&format!("/v1/allocs?job={SERVICE_ID}")).await {
                    Ok(response) => response
                        .json::<serde_json::Value>()
                        .await
                        .is_ok_and(|status| status["replicas_running"] == 0),
                    Err(_) => false,
                };
                if stopped {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
        .await
        .expect("the Service allocation stops within 20 s");
    }

    /// Withdraw the Route, stop the Service, assert the live gateway counts are
    /// zero, shut down within the one drain, and assert that the host's
    /// managed links, network namespaces and workload cgroup scopes are as
    /// they were before the boot.
    pub async fn tear_down(self) {
        self.withdraw_route().await;
        self.stop_service_and_wait().await;
        let counts = blocking(live_gateway_counts).await;
        assert_eq!(counts, (0, 0), "no connect intent or receipt remains after teardown");
        let cleanup = self.cleanup.clone();
        let (elapsed, result) = self.shutdown().await;
        result.expect("the graceful shutdown converges: gateway stage clean, identity slot empty");
        assert!(
            elapsed <= DRAIN_DEADLINE + Duration::from_secs(2),
            "shutdown fits the one {DRAIN_DEADLINE:?} drain: {elapsed:?}"
        );
        blocking(move || {
            wait_until(
                Duration::from_secs(15),
                "host links, namespaces and scopes restored",
                || cleanup.is_restored(),
            );
        })
        .await;
    }

    /// The gateway's public address.
    #[must_use]
    pub const fn gateway_address(&self) -> Ipv4Addr {
        self.address
    }

    /// The operator HTTPS API.
    #[must_use]
    pub const fn api(&self) -> &PublicApi {
        &self.api
    }

    /// The tracing history shared by this world.
    #[must_use]
    pub const fn trace(&self) -> &TraceHistory {
        &self.trace
    }

    /// The external public client.
    #[must_use]
    pub fn client(&self) -> PublicClient {
        PublicClient::new(self.address, &self.pki)
    }

    /// The test CA in DER form, written for a child process.
    #[must_use]
    pub fn write_ca_der(&self) -> PathBuf {
        let path = self.root.path().join("test-ca.der");
        std::fs::write(&path, self.pki.ca_der.as_ref()).expect("write the CA");
        path
    }

    /// Submit the VM Service through the public API.
    pub async fn deploy_service(&self) {
        let command = self.staged.command.to_owned();
        let body = serde_json::json!({
            "spec": {
                "kind": "service",
                "id": SERVICE_ID,
                "replicas": 1,
                "resources": { "cpu_milli": 500, "memory_bytes": 134_217_728_u64 },
                "vm": {
                    "command": command,
                    "args": [],
                    "kernel": self.staged.kernel,
                    "rootfs": self.staged.rootfs
                },
                "listeners": [{ "port": SERVICE_PORT, "protocol": "tcp" }],
                "startup_probes": [],
                "readiness_probes": [],
                "liveness_probes": []
            }
        });
        self.api.submit_workload(SERVICE_ID, &body).await;
    }

    /// Declare the whole-host Route (`/` as a segment prefix) through the
    /// public API.
    pub async fn declare_route(&self) {
        let body = serde_json::json!({
            "id": ROUTE_ID,
            "hostname": PUBLIC_HOSTNAME,
            "path": "/",
            "path_match": "segment_prefix",
            "certified_key": CERTIFIED_KEY_ID,
            "target": { "service": SERVICE_ID, "port": SERVICE_PORT, "protocol": "tcp" }
        });
        let response = self.api.post_json("/v1/routes", &body).await.expect("POST /v1/routes");
        assert!(
            matches!(response.status().as_u16(), 200 | 201),
            "the Route is declared or replaced: {}",
            response.status()
        );
    }

    /// Withdraw the Route through the public API.
    pub async fn withdraw_route(&self) {
        let response = self
            .api
            .delete(&format!("/v1/routes/{ROUTE_ID}"))
            .await
            .expect("DELETE /v1/routes/{id}");
        assert!(response.status().is_success(), "the Route is withdrawn: {}", response.status());
    }

    /// Stop the Service through the public API.
    pub async fn stop_service(&self) {
        self.api.stop_workload(SERVICE_ID).await;
    }

    /// `GET /v1/gateway/status` as JSON.
    pub async fn gateway_status(&self) -> serde_json::Value {
        let response = self.api.get("/v1/gateway/status").await.expect("GET /v1/gateway/status");
        assert!(response.status().is_success(), "gateway status: {}", response.status());
        response.json().await.expect("gateway status body")
    }

    /// The api allocation's identifier, once an allocation exists.
    pub async fn allocation_id(&self) -> Option<String> {
        let response = self.api.get(&format!("/v1/allocs?job={SERVICE_ID}")).await.ok()?;
        let body: serde_json::Value = response.json().await.ok()?;
        body["rows"].as_array()?.first()?["alloc_id"].as_str().map(str::to_owned)
    }

    /// Poll the public URL (`GET /ready`) once per second, at most
    /// [`READINESS_POLLS`] times, each attempt bounded by 5 s, until it answers
    /// `204`. A `503` (or `502`) is expected until the hydrator programs the
    /// Service. Returns the statuses seen, in order (`0` = no response).
    pub async fn wait_until_serving(&self) -> Vec<u16> {
        let mut seen = Vec::new();
        for _ in 0..READINESS_POLLS {
            let started = Instant::now();
            let client = self.client();
            let status = blocking(move || {
                client
                    .connect()
                    .and_then(|mut connection| {
                        connection
                            .exchange(&Request::get("/ready"))
                            .map_err(|error| io::Error::other(format!("{error:?}")))
                    })
                    .map_or(0, |response| response.status())
            })
            .await;
            seen.push(status);
            if status == 204 {
                return seen;
            }
            tokio::time::sleep(Duration::from_secs(1).saturating_sub(started.elapsed())).await;
        }
        panic!("the public URL never answered 204 within {READINESS_POLLS} polls: {seen:?}");
    }

    /// Stop the server gracefully and start it again over the same data and
    /// config directories, address and key files.
    pub async fn restart(&mut self) {
        self.handle
            .take()
            .expect("live server handle")
            .shutdown(DRAIN_DEADLINE)
            .await
            .expect("graceful shutdown before the restart");
        let (handle, api) =
            start_server(&self.data_dir, &self.config_dir, self.address, &self.pki).await;
        self.handle = Some(handle);
        self.api = api;
    }

    /// Gracefully shut the server down; returns the stage's duration and the
    /// typed result.
    pub async fn shutdown(
        mut self,
    ) -> (Duration, Result<(), overdrive_control_plane::ServerShutdownError>) {
        let started = Instant::now();
        let result = self.handle.take().expect("live server handle").shutdown(DRAIN_DEADLINE).await;
        (started.elapsed(), result)
    }
}

impl Drop for GatewayWorld {
    fn drop(&mut self) {
        if self.staged.example_owned {
            let script = example_dir().join("prepare.sh");
            let cleanup = command_output("bash", &[&script.to_string_lossy(), "cleanup"]);
            if !cleanup.status.success() {
                eprintln!(
                    "example materialisation cleanup failed: {}",
                    String::from_utf8_lossy(&cleanup.stderr)
                );
            }
        }
    }
}

async fn start_server(
    data_dir: &Path,
    config_dir: &Path,
    address: Ipv4Addr,
    pki: &PublicPki,
) -> (ServerHandle, PublicApi) {
    let key = ManualCertifiedKeyConfig::new(
        PublicCertifiedKeyId::new(CERTIFIED_KEY_ID).expect("certified-key id"),
        pki.chain_path.clone(),
        pki.key_path.clone(),
    );
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("loopback bind"),
        data_dir: data_dir.to_path_buf(),
        operator_config_dir: config_dir.to_path_buf(),
        vsock_ports: NodeVsockPortConfig::kernel_assigned(),
        gateway: Some(GatewayConfig::new(address, key)),
        ..ServerConfig::new()
    };
    // The production host bundle over this data dir; the hermetic KEK is the
    // only simulated edge (the production keyring refuses a cold boot).
    let adapters =
        NodeAdapters::host(&config, Arc::new(RealCgroupFs::new()), Arc::new(SimKek::for_boot()))
            .expect("the host bundle opens the stores");
    let handle = run_server(config, adapters).await.expect("start the production server");
    let api = PublicApi::from_trust_config(&config_dir.join(".overdrive/config"))
        .expect("load the server trust triple");
    (handle, api)
}

// ---------------------------------------------------------------------------
// Client load in a child process (S-PIG-113)
// ---------------------------------------------------------------------------

/// The environment variable that turns a re-executed test binary into the load
/// client: `<gateway-ip>|<connections>|<ca-der-path>`.
pub const LOAD_CHILD_ENV: &str = "PIG_CONFORMANCE_LOAD_CHILD";
const LOAD_LINE: &str = "PIG-LOAD ";

/// The parent's handle on the load-client child.
///
/// The child is the test binary re-executed, never the `overdrive` binary. The
/// clients live in another process so the parent's descriptor and resident-set
/// observations describe the gateway's process only.
pub struct LoadChild {
    child: Child,
    lines: BufReader<ChildStdout>,
    stdin: ChildStdin,
}

impl LoadChild {
    /// Start a child holding `connections` concurrent `GET /hold` exchanges.
    #[must_use]
    pub fn spawn(entry_test: &str, address: Ipv4Addr, connections: u32, ca_der: &Path) -> Self {
        let mut child = Command::new(std::env::current_exe().expect("this test binary"))
            .args(["--exact", entry_test, "--include-ignored", "--nocapture"])
            .env(LOAD_CHILD_ENV, format!("{address}|{connections}|{}", ca_der.display()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn the load-client child");
        let lines = BufReader::new(child.stdout.take().expect("child stdout"));
        let stdin = child.stdin.take().expect("child stdin");
        Self { child, lines, stdin }
    }

    /// Wait for the child's next protocol line starting with `prefix`.
    pub fn wait_for(&mut self, prefix: &str) -> String {
        let mut line = String::new();
        loop {
            line.clear();
            let read = self.lines.read_line(&mut line).expect("read the child's stdout");
            assert!(read > 0, "the load-client child ended before reporting {prefix:?}");
            if let Some(rest) = line.trim().strip_prefix(LOAD_LINE)
                && rest.starts_with(prefix)
            {
                return rest.to_owned();
            }
        }
    }

    /// Tell the child to read every body to its end.
    pub fn drain(&mut self) {
        self.stdin.write_all(b"DRAIN\n").expect("tell the child to drain");
        self.stdin.flush().expect("flush");
    }

    /// Reap the child.
    pub fn finish(mut self) {
        let status = self.child.wait().expect("wait for the load-client child");
        assert!(status.success(), "the load-client child exits cleanly: {status}");
    }
}

/// The child's entry: when [`LOAD_CHILD_ENV`] is set, hold the exchanges and
/// speak the line protocol; otherwise do nothing.
pub fn run_load_child_if_requested() {
    let Ok(plan) = std::env::var(LOAD_CHILD_ENV) else { return };
    let mut parts = plan.split('|');
    let address: Ipv4Addr = parts.next().and_then(|p| p.parse().ok()).expect("gateway address");
    let connections: u32 = parts.next().and_then(|p| p.parse().ok()).expect("connection count");
    let ca_path = PathBuf::from(parts.next().expect("CA path"));
    let pki = PublicPki {
        ca_der: CertificateDer::from(std::fs::read(&ca_path).expect("read the CA")),
        chain_path: PathBuf::new(),
        key_path: PathBuf::new(),
    };
    let client = PublicClient::new(address, &pki);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<bool>();
    let go = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let mut workers = Vec::new();
    for _ in 0..connections {
        let client = client.clone();
        let ready = ready_tx.clone();
        let go = Arc::clone(&go);
        workers.push(
            thread::Builder::new()
                .stack_size(256 * 1024)
                .spawn(move || {
                    let opened = client.connect().ok().and_then(|mut connection| {
                        connection.send(&Request::get("/hold").encode()).ok()?;
                        connection.set_read_timeout(Some(Duration::from_secs(60)));
                        let head = connection.read_head().ok()?;
                        (head.status == 200).then_some((connection, head))
                    });
                    let _ = ready.send(opened.is_some());
                    let (lock, signal) = &*go;
                    let mut released = lock.lock().expect("go lock");
                    while !*released {
                        released = signal.wait(released).expect("go wait");
                    }
                    drop(released);
                    let Some((mut connection, head)) = opened else { return false };
                    connection.read_body(&head, false).is_ok_and(|(body, _, _)| {
                        body.len() == 1024 * 1024 && first_pattern_mismatch(0, &body).is_none()
                    })
                })
                .expect("spawn a client thread"),
        );
    }
    drop(ready_tx);
    let established = ready_rx.iter().filter(|ok| *ok).count();
    writeln!(io::stdout(), "{LOAD_LINE}INFLIGHT {established}").expect("report to the parent");
    io::stdout().flush().expect("flush");
    let mut command = String::new();
    io::stdin().read_line(&mut command).expect("read the parent's command");
    {
        let (lock, signal) = &*go;
        *lock.lock().expect("go lock") = true;
        signal.notify_all();
    }
    let complete =
        workers.into_iter().filter_map(|worker| worker.join().ok()).filter(|ok| *ok).count();
    writeln!(io::stdout(), "{LOAD_LINE}DONE {complete}").expect("report to the parent");
    io::stdout().flush().expect("flush");
}

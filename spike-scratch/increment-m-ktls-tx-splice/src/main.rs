//! PROBE increment-m — does a BLOCKING `splice(legF → pipe → legB kTLS-TX)`
//! pump deliver application bytes losslessly and byte-exact to the TLS peer,
//! including under leg-B send-buffer exhaustion (small SO_SNDBUF + slow peer
//! reader) — the exact condition where the retired non-blocking
//! (`MSG_DONTWAIT`) paths lost ~10–15% of records?
//!
//! Topology (TX mirror of increment-h):
//!
//! ```text
//! W (writer thread, plain TCP client) ──▶ legF_peer (accepted, agent-owned)
//!   pump: splice(legF_peer → pipe[1], SPLICE_F_MOVE)   [BLOCKING]
//!         splice(pipe[0]   → legB,    SPLICE_F_MOVE)   [BLOCKING] ← kernel encrypts
//! legB (agent's TCP client to P, kTLS TLS_TX armed, AES-256-GCM TLS 1.3)
//!   ──0x17 ciphertext──▶ P (rustls TLS 1.3 SERVER thread, byte-compares)
//! ```
//!
//! Env: PUMP_MODE=splice|copy  SNDBUF=<bytes>  SLOW_READER=1  WATCHDOG_SECS=<n>
//! Arg1: payload length in bytes.
//!
//! Throwaway spike code per .claude/rules/spike.md. The kTLS arm logic is
//! COPIED from crates/overdrive-dataplane/src/mtls/ktls.rs (error type adapted
//! to std::io::Error); no overdrive-* crate is imported.

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{
    ClientConfig, ClientConnection, ConnectionTrafficSecrets, DigitallySignedStruct,
    ExtractedSecrets, ServerConfig, ServerConnection, SignatureScheme,
};

// ---------------------------------------------------------------------------
// Payload pattern — byte[i] = (i % 251), byte-varying so truncation / reorder /
// duplication is detectable at any offset.
// ---------------------------------------------------------------------------

#[inline]
fn pattern_byte(i: usize) -> u8 {
    (i % 251) as u8
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(pattern_byte).collect()
}

// ---------------------------------------------------------------------------
// Shared pump tallies (watchdog dumps them on stall).
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Tallies {
    in_calls: AtomicU64,
    in_bytes: AtomicU64,
    out_calls: AtomicU64,
    out_bytes: AtomicU64,
    eagain: AtomicU64,
    eintr: AtomicU64,
}

impl Tallies {
    fn dump(&self, mode: &str) -> String {
        format!(
            "PUMP: mode={} splice_in calls={} bytes={}  splice_out calls={} bytes={} eagain={} eintr={}",
            mode,
            self.in_calls.load(Ordering::Relaxed),
            self.in_bytes.load(Ordering::Relaxed),
            self.out_calls.load(Ordering::Relaxed),
            self.out_bytes.load(Ordering::Relaxed),
            self.eagain.load(Ordering::Relaxed),
            self.eintr.load(Ordering::Relaxed),
        )
    }
}

// ---------------------------------------------------------------------------
// kTLS arm — COPIED from crates/overdrive-dataplane/src/mtls/ktls.rs, error
// type adapted to std::io::Error. TX-only (sufficient for this probe: leg B is
// never read by the agent).
// ---------------------------------------------------------------------------

/// `tls12_crypto_info_aes_gcm_256` (in-tree UAPI shape). `#[repr(C)]`, no padding.
#[repr(C)]
struct CryptoInfoAes256Gcm {
    version: u16,
    cipher: u16,
    iv: [u8; 8],
    key: [u8; 32],
    salt: [u8; 4],
    rec_seq: [u8; 8],
}

fn arm_ktls_tx(fd: RawFd, secrets: ExtractedSecrets) -> std::io::Result<u64> {
    install_ulp(fd)?;
    let tx_seq = secrets.tx.0;
    set_crypto_info(fd, libc::TLS_TX, &secrets.tx)?;
    Ok(tx_seq)
}

fn install_ulp(fd: RawFd) -> std::io::Result<()> {
    let ulp = b"tls\0";
    // SAFETY: setsockopt with a 3-byte "tls" option string on a real TCP fd.
    let rc = unsafe { libc::setsockopt(fd, libc::SOL_TCP, libc::TCP_ULP, ulp.as_ptr().cast(), 3) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn set_crypto_info(
    fd: RawFd,
    dir: libc::c_int,
    sec: &(u64, ConnectionTrafficSecrets),
) -> std::io::Result<()> {
    let (seq, traffic) = sec;
    let ConnectionTrafficSecrets::Aes256Gcm { key, iv } = traffic else {
        return Err(std::io::Error::other("kTLS arm requires AES-256-GCM TLS 1.3"));
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
    // `tls12_crypto_info_aes_gcm_256` layout; setsockopt reads size_of bytes.
    let rc = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_TLS,
            dir,
            std::ptr::from_ref(&info).cast(),
            std::mem::size_of::<CryptoInfoAes256Gcm>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// rustls plumbing — probe-grade no-verify client verifier + pinned-suite server.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct NoVerify;

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Server config: rcgen self-signed cert, TLS 1.3 only, suite PINNED to
/// TLS13_AES_256_GCM_SHA384 (the arm helper is AES-256-GCM-only — pinning on
/// the SERVER means negotiation cannot drift). Tickets suppressed (mirrors
/// production `send_tls13_tickets = 0`).
fn server_config() -> Arc<ServerConfig> {
    let key = rcgen::KeyPair::generate().expect("rcgen keypair");
    let cert = rcgen::CertificateParams::new(vec!["localhost".to_string()])
        .expect("rcgen params")
        .self_signed(&key)
        .expect("rcgen self-signed");
    let cert_der = CertificateDer::from(cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));

    let mut provider = rustls::crypto::ring::default_provider();
    provider.cipher_suites =
        vec![rustls::crypto::ring::cipher_suite::TLS13_AES_256_GCM_SHA384];
    let mut cfg = ServerConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("server protocol versions")
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .expect("server single cert");
    cfg.send_tls13_tickets = 0;
    Arc::new(cfg)
}

fn client_config() -> Arc<ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("client protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerify))
        .with_no_client_auth();
    cfg.enable_secret_extraction = true;
    Arc::new(cfg)
}

/// Drive the rustls CLIENT handshake to completion on a BLOCKING socket
/// (shape copied from outbound.rs `drive_handshake_client`, deadline dropped —
/// the watchdog owns stall detection here).
fn drive_handshake_client(conn: &mut ClientConnection, tcp: &mut TcpStream) {
    loop {
        while conn.wants_write() {
            conn.write_tls(tcp).expect("write_tls");
        }
        if !conn.is_handshaking() {
            return;
        }
        match conn.read_tls(tcp) {
            Ok(0) => panic!("EOF during handshake (peer closed)"),
            Ok(_) => {
                conn.process_new_packets().expect("process_new_packets");
            }
            Err(e) => panic!("read_tls: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Peer P — rustls TLS 1.3 server thread. Does NOT arm kTLS: its rustls session
// successfully decoding every record IS the proof the kernel emitted
// well-formed TLS 1.3 ciphertext. Reads to EOF, byte-compares against the
// regenerated pattern, prints PEER_RESULT.
// ---------------------------------------------------------------------------

fn peer_thread(listener: TcpListener, expected_len: usize, slow_reader: bool) -> bool {
    let cfg = server_config();
    let (mut tcp, _) = listener.accept().expect("peer accept");
    let mut conn = ServerConnection::new(cfg).expect("ServerConnection");
    let mut stream = rustls::Stream::new(&mut conn, &mut tcp);

    let chunk = if slow_reader { 4096 } else { 16384 };
    let mut buf = vec![0u8; chunk];
    let mut n: usize = 0;
    let mut mismatch: Option<(usize, u8, u8)> = None;

    loop {
        match stream.read(&mut buf) {
            Ok(0) => break, // clean close_notify EOF
            Ok(got) => {
                if mismatch.is_none() {
                    for (j, &b) in buf[..got].iter().enumerate() {
                        let want = pattern_byte(n + j);
                        if b != want {
                            mismatch = Some((n + j, b, want));
                            break;
                        }
                    }
                }
                n += got;
                if slow_reader {
                    std::thread::sleep(Duration::from_millis(3));
                }
            }
            // Peer closed without close_notify — the agent's shutdown(SHUT_WR)
            // on a kTLS leg sends a bare FIN, no close_notify. Expected EOF.
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => {
                println!("PEER_RESULT: MISMATCH at={n} got=READ_ERR({e}) want=data n={n}");
                return false;
            }
        }
    }

    match mismatch {
        Some((at, got, want)) => {
            println!("PEER_RESULT: MISMATCH at={at} got={got} want={want} n={n}");
            false
        }
        None if n == expected_len => {
            println!("PEER_RESULT: EXACT n={n}");
            true
        }
        None => {
            println!("PEER_RESULT: MISMATCH at={n} got=EOF want=len_{expected_len} n={n}");
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Writer W — plain TCP client to legF. Odd-sized varying chunks, then
// shutdown(SHUT_WR), then holds the socket open until told to exit.
// ---------------------------------------------------------------------------

fn writer_thread(addr: SocketAddr, payload: Vec<u8>, hold: mpsc::Receiver<()>) {
    let mut tcp = TcpStream::connect(addr).expect("writer connect legF");
    const CHUNKS: [usize; 7] = [1, 777, 1777, 13, 4096, 25, 999];
    let mut off = 0usize;
    let mut ci = 0usize;
    while off < payload.len() {
        let take = CHUNKS[ci % CHUNKS.len()].min(payload.len() - off);
        tcp.write_all(&payload[off..off + take]).expect("writer write");
        off += take;
        ci += 1;
    }
    tcp.shutdown(Shutdown::Write).expect("writer shutdown WR");
    let _ = hold.recv(); // hold the socket open until the run is done
}

// ---------------------------------------------------------------------------
// legB connect — optional SO_SNDBUF set BEFORE connect.
// ---------------------------------------------------------------------------

fn connect_leg_b(addr: SocketAddr, sndbuf: Option<i32>) -> TcpStream {
    let Some(val) = sndbuf else {
        return TcpStream::connect(addr).expect("legB connect");
    };
    let SocketAddr::V4(v4) = addr else { panic!("legB addr must be v4") };
    // SAFETY: plain socket/setsockopt/connect syscall sequence on a fresh fd;
    // ownership transfers to TcpStream at the end (single owner).
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        assert!(fd >= 0, "socket: {}", std::io::Error::last_os_error());
        let rc = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            std::ptr::from_ref(&val).cast(),
            std::mem::size_of::<i32>() as libc::socklen_t,
        );
        assert_eq!(rc, 0, "setsockopt SO_SNDBUF: {}", std::io::Error::last_os_error());
        let sa = libc::sockaddr_in {
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: v4.port().to_be(),
            sin_addr: libc::in_addr { s_addr: u32::from(Ipv4Addr::LOCALHOST).to_be() },
            sin_zero: [0; 8],
        };
        let rc = libc::connect(
            fd,
            std::ptr::from_ref(&sa).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        );
        assert_eq!(rc, 0, "connect legB: {}", std::io::Error::last_os_error());
        TcpStream::from_raw_fd(fd)
    }
}

// ---------------------------------------------------------------------------
// The pump under test — BLOCKING splice(legF → pipe → legB). No O_NONBLOCK,
// no SPLICE_F_NONBLOCK anywhere. Any -1 on the kTLS leg is evidence: print
// errno + tallies and abort the run.
// ---------------------------------------------------------------------------

fn splice_retry_eintr(
    fd_in: RawFd,
    fd_out: RawFd,
    len: usize,
    tallies: &Tallies,
) -> Result<isize, std::io::Error> {
    loop {
        // SAFETY: splice(2) between two owned open fds; NULL offsets (stream fds).
        let r = unsafe {
            libc::splice(
                fd_in,
                std::ptr::null_mut(),
                fd_out,
                std::ptr::null_mut(),
                len,
                libc::SPLICE_F_MOVE,
            )
        };
        if r >= 0 {
            return Ok(r);
        }
        let e = std::io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EINTR) => {
                tallies.eintr.fetch_add(1, Ordering::Relaxed);
            }
            Some(libc::EAGAIN) => {
                tallies.eagain.fetch_add(1, Ordering::Relaxed);
                return Err(e);
            }
            _ => return Err(e),
        }
    }
}

fn pump_splice(legf_fd: RawFd, legb_fd: RawFd, tallies: &Tallies) -> Result<(), String> {
    let mut pipefd = [0i32; 2];
    // SAFETY: pipe2 with flags=0 — a BLOCKING pipe.
    let rc = unsafe { libc::pipe2(pipefd.as_mut_ptr(), 0) };
    assert_eq!(rc, 0, "pipe2: {}", std::io::Error::last_os_error());
    let (pipe_r, pipe_w) = (pipefd[0], pipefd[1]);

    loop {
        let n = match splice_retry_eintr(legf_fd, pipe_w, 65536, tallies) {
            Ok(n) => n,
            Err(e) => {
                return Err(format!(
                    "splice_in(legF→pipe) errno={} ({e})",
                    e.raw_os_error().unwrap_or(-1)
                ))
            }
        };
        if n == 0 {
            break; // legF EOF
        }
        tallies.in_calls.fetch_add(1, Ordering::Relaxed);
        tallies.in_bytes.fetch_add(n as u64, Ordering::Relaxed);

        let mut remaining = n as usize;
        while remaining > 0 {
            let m = match splice_retry_eintr(pipe_r, legb_fd, remaining, tallies) {
                Ok(m) => m,
                Err(e) => {
                    return Err(format!(
                        "splice_out(pipe→legB kTLS-TX) errno={} ({e})",
                        e.raw_os_error().unwrap_or(-1)
                    ))
                }
            };
            if m == 0 {
                return Err("splice_out returned 0 mid-stream (unexpected)".into());
            }
            tallies.out_calls.fetch_add(1, Ordering::Relaxed);
            tallies.out_bytes.fetch_add(m as u64, Ordering::Relaxed);
            remaining -= m as usize;
        }
    }
    // SAFETY: closing the pipe fds this fn created.
    unsafe {
        libc::close(pipe_r);
        libc::close(pipe_w);
    }
    Ok(())
}

/// COPY control mode — the production forward-pump shape (mtls/mod.rs:
/// bounded userspace `read → write_all` into kTLS-TX).
fn pump_copy(legf: &mut TcpStream, legb: &mut TcpStream, tallies: &Tallies) -> Result<(), String> {
    let mut buf = vec![0u8; 65536];
    loop {
        let n = match legf.read(&mut buf) {
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => {
                tallies.eintr.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            Err(e) => return Err(format!("read(legF): {e}")),
        };
        if n == 0 {
            break; // legF EOF
        }
        tallies.in_calls.fetch_add(1, Ordering::Relaxed);
        tallies.in_bytes.fetch_add(n as u64, Ordering::Relaxed);
        if let Err(e) = legb.write_all(&buf[..n]) {
            return Err(format!("write_all(legB kTLS-TX): {e}"));
        }
        tallies.out_calls.fetch_add(1, Ordering::Relaxed);
        tallies.out_bytes.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    let payload_len: usize = std::env::args()
        .nth(1)
        .expect("arg1: payload length in bytes")
        .parse()
        .expect("payload length must be a usize");
    let mode = std::env::var("PUMP_MODE").unwrap_or_else(|_| "splice".into());
    let sndbuf: Option<i32> = std::env::var("SNDBUF").ok().map(|v| v.parse().expect("SNDBUF"));
    let slow_reader = std::env::var("SLOW_READER").map(|v| v == "1").unwrap_or(false);
    let watchdog_secs: u64 = std::env::var("WATCHDOG_SECS")
        .ok()
        .map(|v| v.parse().expect("WATCHDOG_SECS"))
        .unwrap_or(30);

    println!(
        "RUN: mode={mode} payload={payload_len} sndbuf={sndbuf:?} slow_reader={slow_reader} watchdog={watchdog_secs}s"
    );

    let tallies = Arc::new(Tallies::default());
    let done = Arc::new(AtomicBool::new(false));

    // Watchdog — a stall must be a visible non-zero exit, never a hang.
    {
        let tallies = Arc::clone(&tallies);
        let done = Arc::clone(&done);
        let mode = mode.clone();
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(watchdog_secs);
            while std::time::Instant::now() < deadline {
                if done.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            println!("WATCHDOG: STALL after {watchdog_secs}s");
            println!("{}", tallies.dump(&mode));
            std::process::exit(2);
        });
    }

    // Peer P listener + thread.
    let p_listener = TcpListener::bind("127.0.0.1:0").expect("bind P");
    let p_addr = p_listener.local_addr().expect("P addr");
    let peer = std::thread::spawn(move || peer_thread(p_listener, payload_len, slow_reader));

    // legF listener + writer W.
    let f_listener = TcpListener::bind("127.0.0.1:0").expect("bind legF");
    let f_addr = f_listener.local_addr().expect("legF addr");
    let (hold_tx, hold_rx) = mpsc::channel::<()>();
    let payload = pattern(payload_len);
    let writer = std::thread::spawn(move || writer_thread(f_addr, payload, hold_rx));

    // legB: connect (optional small SO_SNDBUF), rustls TLS 1.3 handshake,
    // extract secrets, arm kTLS TLS_TX. Socket stays BLOCKING throughout.
    let mut legb = connect_leg_b(p_addr, sndbuf);
    let mut conn = ClientConnection::new(
        client_config(),
        ServerName::try_from("localhost").expect("SNI"),
    )
    .expect("ClientConnection");
    drive_handshake_client(&mut conn, &mut legb);
    let suite = conn
        .negotiated_cipher_suite()
        .map(|s| format!("{:?}", s.suite()))
        .unwrap_or_else(|| "unknown".into());
    let secrets = conn.dangerous_extract_secrets().expect("extract secrets");
    let tx_seq = arm_ktls_tx(legb.as_raw_fd(), secrets).expect("kTLS TLS_TX arm");
    println!("KTLS: armed TLS_TX only (tx_seq={tx_seq}) suite={suite}");

    // Accept legF_peer and run the pump under test.
    let (mut legf_peer, _) = f_listener.accept().expect("accept legF_peer");
    let pump_result = match mode.as_str() {
        "splice" => pump_splice(legf_peer.as_raw_fd(), legb.as_raw_fd(), &tallies),
        "copy" => pump_copy(&mut legf_peer, &mut legb, &tallies),
        other => panic!("PUMP_MODE must be splice|copy, got {other}"),
    };

    // legF EOF reached (or pump aborted) — signal EOF to P.
    legb.shutdown(Shutdown::Write).expect("legB shutdown WR");

    if let Err(e) = &pump_result {
        println!("PUMP_ERR: {e}");
        println!("{}", tallies.dump(&mode));
        done.store(true, Ordering::Relaxed);
        let _ = hold_tx.send(());
        let _ = writer.join();
        let _ = peer.join();
        std::process::exit(3);
    }

    // Collect the peer verdict, then release the writer.
    let peer_exact = peer.join().expect("peer thread panicked");
    println!("{}", tallies.dump(&mode));
    let out_bytes = tallies.out_bytes.load(Ordering::Relaxed);
    let pumped_all = out_bytes == payload_len as u64;
    if !pumped_all {
        println!("PUMP_ASSERT: FAIL out_bytes={out_bytes} != payload={payload_len}");
    }

    done.store(true, Ordering::Relaxed);
    let _ = hold_tx.send(());
    let _ = writer.join();

    if peer_exact && pumped_all {
        println!("RUN_RESULT: PASS");
    } else {
        println!("RUN_RESULT: FAIL");
        std::process::exit(4);
    }
}

//! Spike B host relay stub (GH #303, increment-d). Throwaway probe.
//!
//!   relay <uds-path> <certdir> <t0-epoch-secs>
//!
//! Listens on the Firecracker vsock muxer path `<uds_path>_<port>`: a guest
//! connect to (CID 2, port) arrives here as a Unix-socket connection. Per
//! connection it runs one TLS 1.3 state machine in rustls and relays the
//! handshake records through the guest (see the protocol table in `lib.rs`):
//!   OPEN   (guest connect()) -> rustls CLIENT with the client SVID `client.pem`
//!   ACCEPT (guest accept())  -> rustls SERVER with the server SVID `guest-server.pem`,
//!                               client certificate required
//! Policy is decided here at handshake time from the peer's verified SPIFFE ID
//! and the local identity. On Allow it extracts the traffic secrets and sends
//! the guest only key/IV/sequence per direction; the certificate private keys
//! never leave this process. On Deny it aborts the handshake and sends DENY.
use std::collections::BTreeMap;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use igkmd_host::*;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::WebPkiClientVerifier;
use rustls::{
    CertificateError, ClientConfig, ClientConnection, Connection, ConnectionTrafficSecrets,
    DigitallySignedStruct, DistinguishedName, RootCertStore, ServerConfig, ServerConnection,
    SignatureScheme,
};

/// Allowed (local identity, peer identity) pairs. Everything else is denied.
const POLICY_ALLOW: &[(&str, &str)] = &[
    (
        "spiffe://overdrive.test/ns/default/sa/guest-client",
        "spiffe://overdrive.test/ns/default/sa/peer-allowed",
    ),
    (
        "spiffe://overdrive.test/ns/default/sa/guest-server",
        "spiffe://overdrive.test/ns/default/sa/peer-client",
    ),
];

type Decision = Arc<Mutex<Option<(bool, String)>>>;

fn decide(local: &str, ee: &[u8], decision: &Decision) -> Result<(), rustls::Error> {
    let peer = spiffe_id(ee)
        .ok_or_else(|| rustls::Error::General("peer certificate has no spiffe:// URI SAN".into()))?;
    let allow = POLICY_ALLOW.iter().any(|(l, p)| *l == local && *p == peer);
    *decision.lock().unwrap() = Some((allow, peer));
    if allow {
        Ok(())
    } else {
        Err(rustls::Error::InvalidCertificate(
            CertificateError::ApplicationVerificationFailure,
        ))
    }
}

/// Client role: chain + IP SAN + validity (host clock) via webpki, then policy.
#[derive(Debug)]
struct PolicyServerVerifier {
    inner: Arc<WebPkiServerVerifier>,
    local: String,
    decision: Decision,
}

impl ServerCertVerifier for PolicyServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)?;
        decide(&self.local, end_entity, &self.decision).map(|_| ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(m, c, d)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// Server role: chain + validity (host clock) via webpki, then policy.
#[derive(Debug)]
struct PolicyClientVerifier {
    inner: Arc<dyn ClientCertVerifier>,
    local: String,
    decision: Decision,
}

impl ClientCertVerifier for PolicyClientVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        self.inner.root_hint_subjects()
    }
    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        self.inner.verify_client_cert(end_entity, intermediates, now)?;
        decide(&self.local, end_entity, &self.decision).map(|_| ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(m, c, d)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

struct Held {
    name: &'static str,
    der: Vec<u8>,
    pem: Vec<u8>,
    scalar: Vec<u8>,
}

struct Ctx {
    certdir: String,
    client_id: String,
    server_id: String,
    roots: Arc<RootCertStore>,
    keys: Vec<Held>,
    t0: f64,
}

/// Parses complete TLS record headers out of a byte stream for the log.
#[derive(Default)]
struct RecordTap {
    buf: Vec<u8>,
}
impl RecordTap {
    fn feed(&mut self, b: &[u8]) -> String {
        self.buf.extend_from_slice(b);
        let mut out = Vec::new();
        while self.buf.len() >= 5 {
            let len = u16::from_be_bytes([self.buf[3], self.buf[4]]) as usize;
            if self.buf.len() < 5 + len {
                break;
            }
            out.push(format!("0x{:02x}/{}", self.buf[0], len));
            self.buf.drain(..5 + len);
        }
        if out.is_empty() {
            "(partial record)".into()
        } else {
            out.join(" ")
        }
    }
}

/// Byte/frame accounting per direction and type, for the custody evidence.
#[derive(Default)]
struct Tally {
    frames: BTreeMap<(&'static str, &'static str), (usize, usize)>,
    sent: Vec<u8>,
}
impl Tally {
    fn add(&mut self, dir: &'static str, t: u8, wire_len: usize) {
        let e = self.frames.entry((dir, type_name(t))).or_default();
        e.0 += 1;
        e.1 += wire_len;
    }
}

fn send(s: &mut UnixStream, tally: &mut Tally, tag: &str, t0: f64, t: u8, p: &[u8], note: &str) -> bool {
    match write_frame(s, t, p) {
        Ok(n) => {
            tally.add("host->guest", t, n);
            tally.sent.push(t);
            tally.sent.extend_from_slice(&(p.len() as u32).to_be_bytes());
            tally.sent.extend_from_slice(p);
            println!(
                "RELAY[{tag}] vsock tx {} payload={} wire={} {note} {}",
                type_name(t),
                p.len(),
                n,
                since(t0)
            );
            true
        }
        Err(e) => {
            println!("RELAY[{tag}] vsock tx {} FAILED: {e} {}", type_name(t), since(t0));
            false
        }
    }
}

fn ip4(b: &[u8]) -> Ipv4Addr {
    Ipv4Addr::new(b[0], b[1], b[2], b[3])
}

fn handle(n: usize, mut s: UnixStream, ctx: &Ctx) {
    let tag = format!("c{n}");
    let t0 = ctx.t0;
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let mut tally = Tally::default();
    let (t, p) = match read_frame(&mut s) {
        Ok(f) => f,
        Err(e) => {
            println!("RELAY[{tag}] no OPEN/ACCEPT frame: {e} {}", since(t0));
            return;
        }
    };
    tally.add("guest->host", t, 5 + p.len());
    let decision: Decision = Arc::new(Mutex::new(None));
    let (mut conn, local): (Connection, String) = match (t, p.len(), p.first()) {
        (T_OPEN, 7, Some(4)) => {
            let ip = ip4(&p[1..5]);
            let port = u16::from_be_bytes([p[5], p[6]]);
            println!(
                "RELAY[{tag}] vsock rx OPEN payload=7 dst={ip}:{port} role=TLS client, local identity={} {}",
                ctx.client_id,
                since(t0)
            );
            let inner = WebPkiServerVerifier::builder_with_provider(ctx.roots.clone(), provider())
                .build()
                .unwrap();
            let v = Arc::new(PolicyServerVerifier {
                inner,
                local: ctx.client_id.clone(),
                decision: decision.clone(),
            });
            let mut cfg = ClientConfig::builder_with_provider(provider())
                .with_protocol_versions(&[&rustls::version::TLS13])
                .unwrap()
                .dangerous()
                .with_custom_certificate_verifier(v)
                .with_client_auth_cert(
                    load_certs(&format!("{}/client.pem", ctx.certdir)),
                    load_key(&format!("{}/client.key", ctx.certdir)),
                )
                .unwrap();
            cfg.enable_secret_extraction = true;
            cfg.resumption = rustls::client::Resumption::disabled();
            let c = ClientConnection::new(Arc::new(cfg), ServerName::from(IpAddr::V4(ip))).unwrap();
            (Connection::Client(c), ctx.client_id.clone())
        }
        (T_ACCEPT, 13, Some(4)) => {
            let (lip, lport) = (ip4(&p[1..5]), u16::from_be_bytes([p[5], p[6]]));
            let (rip, rport) = (ip4(&p[7..11]), u16::from_be_bytes([p[11], p[12]]));
            println!(
                "RELAY[{tag}] vsock rx ACCEPT payload=13 local={lip}:{lport} remote={rip}:{rport} role=TLS server, local identity={} {}",
                ctx.server_id,
                since(t0)
            );
            let inner = WebPkiClientVerifier::builder_with_provider(ctx.roots.clone(), provider())
                .build()
                .unwrap();
            let v = Arc::new(PolicyClientVerifier {
                inner,
                local: ctx.server_id.clone(),
                decision: decision.clone(),
            });
            let mut cfg = ServerConfig::builder_with_provider(provider())
                .with_protocol_versions(&[&rustls::version::TLS13])
                .unwrap()
                .with_client_cert_verifier(v)
                .with_single_cert(
                    load_certs(&format!("{}/guest-server.pem", ctx.certdir)),
                    load_key(&format!("{}/guest-server.key", ctx.certdir)),
                )
                .unwrap();
            cfg.enable_secret_extraction = true;
            cfg.send_tls13_tickets = 0;
            cfg.send_half_rtt_data = false;
            let c = ServerConnection::new(Arc::new(cfg)).unwrap();
            (Connection::Server(c), ctx.server_id.clone())
        }
        _ => {
            println!("RELAY[{tag}] bad first frame type={} len={} {}", type_name(t), p.len(), since(t0));
            return;
        }
    };
    let mut tap_out = RecordTap::default();
    let mut tap_in = RecordTap::default();

    let outcome: Result<(), (u8, String)> = loop {
        let mut write_ok = true;
        while write_ok && conn.wants_write() {
            let mut buf = Vec::new();
            conn.write_tls(&mut buf).unwrap();
            let recs = tap_out.feed(&buf);
            write_ok = send(&mut s, &mut tally, &tag, t0, T_TO_PEER, &buf, &format!("records[{recs}]"));
        }
        if !write_ok {
            break Err((0, "vsock write failed".into()));
        }
        if !conn.is_handshaking() {
            break Ok(());
        }
        if !send(&mut s, &mut tally, &tag, t0, T_NEED_PEER, &[], "") {
            break Err((0, "vsock write failed".into()));
        }
        let (t, p) = match read_frame(&mut s) {
            Ok(f) => f,
            Err(e) => break Err((0, format!("vsock read failed: {e}"))),
        };
        tally.add("guest->host", t, 5 + p.len());
        if t != T_FROM_PEER {
            break Err((T_ERROR, format!("unexpected frame {}", type_name(t))));
        }
        let recs = if p.is_empty() { "EOF".into() } else { tap_in.feed(&p) };
        println!(
            "RELAY[{tag}] vsock rx FROM_PEER payload={} records[{recs}] {}",
            p.len(),
            since(t0)
        );
        if p.is_empty() {
            break Err((T_ERROR, "peer closed during handshake".into()));
        }
        let mut rd = &p[..];
        let mut failed = None;
        while !rd.is_empty() {
            if let Err(e) = conn.read_tls(&mut rd) {
                failed = Some(format!("read_tls: {e}"));
                break;
            }
            if let Err(e) = conn.process_new_packets() {
                failed = Some(format!("{e}"));
                break;
            }
        }
        if let Some(e) = failed {
            let dec = decision.lock().unwrap().clone();
            match dec {
                Some((false, peer)) => {
                    break Err((
                        T_DENY,
                        format!("policy deny: local {local} <-> peer {peer} (handshake aborted: {e})"),
                    ))
                }
                _ => break Err((T_ERROR, format!("handshake failed: {e}"))),
            }
        }
    };

    match outcome {
        Ok(()) => {
            let peer = decision
                .lock()
                .unwrap()
                .clone()
                .map(|d| d.1)
                .unwrap_or_default();
            let mut probe = [0u8; 1];
            let buffered_plaintext = !matches!(
                conn.reader().read(&mut probe),
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock
            );
            let suite = conn.negotiated_cipher_suite().unwrap().suite();
            let secrets = conn.dangerous_extract_secrets().unwrap();
            let (tx_seq, tx) = secrets.tx;
            let (rx_seq, rx) = secrets.rx;
            let (ConnectionTrafficSecrets::Aes128Gcm { key: tk, iv: ti }, ConnectionTrafficSecrets::Aes128Gcm { key: rk, iv: ri }) = (tx, rx) else {
                println!("RELAY[{tag}] unexpected suite; sending ERROR {}", since(t0));
                send(&mut s, &mut tally, &tag, t0, T_ERROR, b"unexpected suite", "");
                return;
            };
            println!(
                "RELAY[{tag}] handshake complete: policy ALLOW local {local} <-> peer {peer}; suite={suite:?} tx_seq={tx_seq} rx_seq={rx_seq} buffered_plaintext={buffered_plaintext} {}",
                since(t0)
            );
            if rx_seq != 0 || tx_seq != 0 || buffered_plaintext {
                println!("RELAY[{tag}] invariant broken (host consumed post-handshake data); sending ERROR");
                send(&mut s, &mut tally, &tag, t0, T_ERROR, b"host consumed post-handshake data", "");
                return;
            }
            let payload = encode_secrets(tx_seq, tk.as_ref(), ti.as_ref(), rx_seq, rk.as_ref(), ri.as_ref(), &peer);
            send(
                &mut s,
                &mut tally,
                &tag,
                t0,
                T_SECRETS,
                &payload,
                &format!(
                    "(suite=0x1301 tx: seq {tx_seq} key {}B iv {}B; rx: seq {rx_seq} key {}B iv {}B; peer id {}B)",
                    tk.as_ref().len(),
                    ti.as_ref().len(),
                    rk.as_ref().len(),
                    ri.as_ref().len(),
                    peer.len()
                ),
            );
        }
        Err((t, why)) => {
            println!("RELAY[{tag}] handshake NOT completed: {why} {}", since(t0));
            if t != 0 {
                send(&mut s, &mut tally, &tag, t0, t, why.as_bytes(), "");
            }
            // The alert rustls queued on failure is deliberately NOT relayed.
        }
    }

    // The guest closes its vsock end once it has what it needs.
    match read_frame(&mut s) {
        Ok((t, p)) => println!("RELAY[{tag}] unexpected trailing frame {} len={}", type_name(t), p.len()),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            println!("RELAY[{tag}] guest closed the vsock connection (agent out of the data path) {}", since(t0))
        }
        Err(e) => println!("RELAY[{tag}] vsock after outcome: {e} {}", since(t0)),
    }
    for ((dir, name), (frames, bytes)) in &tally.frames {
        println!("RELAY[{tag}] tally {dir} {name}: frames={frames} wire_bytes={bytes}");
    }
    let mut c = format!("RELAY[{tag}] custody: host->guest total {} bytes;", tally.sent.len());
    for k in &ctx.keys {
        c += &format!(
            " {} key occurrences PKCS#8 DER={} PEM={} private scalar={};",
            k.name,
            count(&tally.sent, &k.der),
            count(&tally.sent, &k.pem),
            count(&tally.sent, &k.scalar)
        );
    }
    println!("{c}");
}

/// The 32-byte P-256 private scalar inside the PKCS#8 ECPrivateKey
/// (`02 01 01 04 20 <scalar>`).
fn scalar_of(der: &[u8]) -> Vec<u8> {
    let pat = [0x02, 0x01, 0x01, 0x04, 0x20];
    let i = der
        .windows(pat.len())
        .position(|w| w == pat)
        .expect("ECPrivateKey scalar not found");
    der[i + 5..i + 5 + 32].to_vec()
}

fn held(certdir: &str, name: &'static str) -> Held {
    let der = load_key(&format!("{certdir}/{name}.key")).secret_der().to_vec();
    let pem = std::fs::read(format!("{certdir}/{name}.key")).unwrap();
    let scalar = scalar_of(&der);
    Held { name, der, pem, scalar }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (uds, certdir, t0) = (a[1].clone(), a[2].clone(), a[3].parse::<f64>().unwrap());
    let mut roots = RootCertStore::empty();
    for c in load_certs(&format!("{certdir}/ca.pem")) {
        roots.add(c).unwrap();
    }
    let client_id = spiffe_id(&load_certs(&format!("{certdir}/client.pem"))[0]).unwrap();
    let server_id = spiffe_id(&load_certs(&format!("{certdir}/guest-server.pem"))[0]).unwrap();
    let keys = vec![held(&certdir, "client"), held(&certdir, "guest-server")];
    let ctx = Ctx {
        certdir,
        client_id,
        server_id,
        roots: Arc::new(roots),
        keys,
        t0,
    };
    let l = UnixListener::bind(&uds).unwrap();
    println!(
        "RELAY listening on unix {uds} pid={} client identity={} server identity={} (keys: client PKCS#8 {} B, guest-server PKCS#8 {} B, P-256 scalars 32 B, held in this process only) {}",
        std::process::id(),
        ctx.client_id,
        ctx.server_id,
        ctx.keys[0].der.len(),
        ctx.keys[1].der.len(),
        since(t0)
    );
    for (i, c) in l.incoming().enumerate() {
        match c {
            Ok(s) => {
                println!("RELAY[c{}] vsock connection accepted {}", i + 1, since(t0));
                handle(i + 1, s, &ctx);
            }
            Err(e) => println!("RELAY accept error: {e}"),
        }
    }
}

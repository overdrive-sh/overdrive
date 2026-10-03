//! Spike D host agent / relay (GH #303, increment-i). Throwaway probe.
//!
//!   relay <uds-path> <certdir> <t0-epoch-secs> <services-file> <health-dir>
//!
//! Listens on the Firecracker vsock muxer path `<uds_path>_<port>`: a guest
//! connect to (CID 2, port) arrives here as a Unix-socket connection. Each
//! connection is handled on its own thread. The first frame selects the role
//! (protocol table in `lib.rs`):
//!   RESOLVE (guest connect() hook) -> service-name resolution + first-healthy
//!                               backend selection (ADR-0072 MtlsResolve, D4),
//!                               reply RESOLVED{backend, expected_peer} or DENY.
//!                               A short lookup; NO TLS state machine, no keys.
//!   OPEN   (guest first I/O) -> rustls CLIENT with the client SVID `client.pem`
//!   ACCEPT (guest accept())  -> rustls SERVER with the server SVID `guest-server.pem`,
//!                               client certificate required, no tickets
//! Policy is decided at handshake time from the peer's verified SPIFFE ID and
//! the local identity. On Allow it sends SECRETS v2: per direction the TLS 1.3
//! application traffic secret (captured through rustls' `KeyLog` interface,
//! labels CLIENT_TRAFFIC_SECRET_0 / SERVER_TRAFFIC_SECRET_0) plus rustls' own
//! key/IV from `dangerous_extract_secrets()` for the guest to cross-check its
//! HKDF derivation. The certificate private keys never leave this process.
use std::collections::BTreeMap;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use igkme_host::*;
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

/// Spike D (D4): one service in the agent's registry. The VIP is the stable
/// address the guest dials; `backends` is the service's backend set; `allow` is
/// the service-level policy verdict (a denied service is refused at resolve, so
/// the guest's connect() fails with no TCP); `expected_peer` is the SPIFFE ID
/// the backends present (a service's instances share the service identity).
#[derive(Clone, Debug)]
struct Service {
    name: String,
    vip: (Ipv4Addr, u16),
    allow: bool,
    backends: Vec<(Ipv4Addr, u16)>,
    expected_peer: String,
}

fn parse_hostport(s: &str) -> (Ipv4Addr, u16) {
    let (ip, port) = s.rsplit_once(':').unwrap_or_else(|| panic!("bad host:port {s}"));
    (ip.parse().unwrap_or_else(|e| panic!("bad ip {ip}: {e}")), port.parse().unwrap_or_else(|e| panic!("bad port {port}: {e}")))
}

/// Load the service registry. One line per service:
///   `<name> <vip_ip:port> <allow|deny> <backend_ip:port,...|-> <expected_peer_spiffe>`
fn load_services(path: &str) -> Vec<Service> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read services {path}: {e}"));
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        assert!(f.len() == 5, "services line needs 5 columns, got {}: {line}", f.len());
        let backends = if f[3] == "-" {
            Vec::new()
        } else {
            f[3].split(',').map(parse_hostport).collect()
        };
        out.push(Service {
            name: f[0].to_string(),
            vip: parse_hostport(f[1]),
            allow: f[2] == "allow",
            backends,
            expected_peer: f[4].to_string(),
        });
    }
    out
}

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
    services: Vec<Service>,
    health_dir: String,
}

impl Ctx {
    /// A backend is healthy iff its health flag file exists. This mirrors the
    /// `service_backends.healthy` column that ADR-0072 `MtlsResolve` reads
    /// (readiness → membership), driven here by the test creating/removing the
    /// flag file between connects. Read fresh on every resolve.
    fn is_healthy(&self, (ip, port): &(Ipv4Addr, u16)) -> bool {
        Path::new(&format!("{}/{ip}_{port}", self.health_dir)).exists()
    }
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

/// Spike D (D4): service-name resolution + first-healthy backend selection over
/// the SAME host-agent vsock channel. Relocates ADR-0072 `MtlsResolve` (first
/// healthy by `Ord` over `service_backends`) out of the host L4 proxy. The guest
/// module's connect() hook sends one RESOLVE for the dialed VIP; the agent picks
/// the backend and returns RESOLVED (addr + expected peer SPIFFE ID) — or DENY
/// for a policy-denied service (guest connect() then fails, no TCP). No key
/// material crosses in either direction.
fn handle_resolve(tag: &str, s: &mut UnixStream, ctx: &Ctx, p: &[u8]) {
    let t0 = ctx.t0;
    if p.len() != 7 || p[0] != 4 {
        println!("RELAY[{tag}] bad RESOLVE frame len={} {}", p.len(), since(t0));
        write_frame(s, T_ERROR, b"bad resolve frame").ok();
        return;
    }
    let vip = ip4(&p[1..5]);
    let vport = u16::from_be_bytes([p[5], p[6]]);
    println!(
        "RELAY[{tag}] vsock rx RESOLVE payload=7 vip={vip}:{vport} (service-name resolution on the SAME vsock channel — D4) {}",
        since(t0)
    );
    let Some(svc) = ctx.services.iter().find(|s| s.vip == (vip, vport)) else {
        println!("RELAY[{tag}] resolve: no service registered for vip {vip}:{vport} -> ERROR {}", since(t0));
        write_frame(s, T_ERROR, format!("no service for {vip}:{vport}").as_bytes()).ok();
        return;
    };
    if !svc.allow {
        println!(
            "RELAY[{tag}] resolve service={} vip={vip}:{vport} policy=DENY for local {} -> DENY (no backend; guest connect() fails, no TCP) {}",
            svc.name, ctx.client_id, since(t0)
        );
        write_frame(s, T_DENY, format!("policy deny: service {} not permitted for {}", svc.name, ctx.client_id).as_bytes()).ok();
        return;
    }
    // First-healthy by Ord over the backend set — ADR-0072 MtlsResolve, relocated.
    let mut backends = svc.backends.clone();
    backends.sort();
    let health_str = backends
        .iter()
        .map(|b| format!("{}:{}={}", b.0, b.1, if ctx.is_healthy(b) { "healthy" } else { "unhealthy" }))
        .collect::<Vec<_>>()
        .join(" ");
    match backends.iter().find(|b| ctx.is_healthy(b)) {
        Some(&(bip, bport)) => {
            println!(
                "RELAY[{tag}] resolve service={} backends(by Ord)[{health_str}] -> chose {bip}:{bport} (first-healthy, MtlsResolve-style) expected_peer={} {}",
                svc.name, svc.expected_peer, since(t0)
            );
            let payload = encode_resolved(bip.octets(), bport, &svc.expected_peer);
            write_frame(s, T_RESOLVED, &payload).ok();
            println!(
                "RELAY[{tag}] vsock tx RESOLVED backend={bip}:{bport} peer={} (payload={} B = addr 7 B + peer-id string; custody: NO key material in a resolve) {}",
                svc.expected_peer, payload.len(), since(t0)
            );
        }
        None => {
            println!(
                "RELAY[{tag}] resolve service={} backends[{health_str}] -> NO HEALTHY BACKEND -> ERROR {}",
                svc.name, since(t0)
            );
            write_frame(s, T_ERROR, b"no healthy backend").ok();
        }
    }
}

fn handle(n: usize, mut s: UnixStream, ctx: &Ctx) {
    let tag = format!("c{n}");
    let t0 = ctx.t0;
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let mut tally = Tally::default();
    let (t, p) = match read_frame(&mut s) {
        Ok(f) => f,
        Err(e) => {
            println!("RELAY[{tag}] no OPEN/ACCEPT/RESOLVE frame: {e} {}", since(t0));
            return;
        }
    };
    tally.add("guest->host", t, 5 + p.len());
    // Spike D (D4): a RESOLVE is a short lookup on the SAME vsock channel — no
    // TLS state machine, no key material. Answer it and close.
    if t == T_RESOLVE {
        handle_resolve(&tag, &mut s, ctx, &p);
        return;
    }
    let decision: Decision = Arc::new(Mutex::new(None));
    let secrets_log = Arc::new(TrafficSecrets::default());
    let (mut conn, local, role): (Connection, String, &str) = match (t, p.len(), p.first()) {
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
            cfg.key_log = secrets_log.clone();
            cfg.resumption = rustls::client::Resumption::disabled();
            let c = ClientConnection::new(Arc::new(cfg), ServerName::from(IpAddr::V4(ip))).unwrap();
            (Connection::Client(c), ctx.client_id.clone(), "client")
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
            cfg.key_log = secrets_log.clone();
            cfg.send_tls13_tickets = 0;
            cfg.send_half_rtt_data = false;
            let c = ServerConnection::new(Arc::new(cfg)).unwrap();
            (Connection::Server(c), ctx.server_id.clone(), "server")
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

    let mut sent_secrets: Vec<Vec<u8>> = Vec::new();
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
            // KeyLog labels are from the TLS client's point of view.
            let (tx_label, rx_label) = if role == "client" {
                ("CLIENT_TRAFFIC_SECRET_0", "SERVER_TRAFFIC_SECRET_0")
            } else {
                ("SERVER_TRAFFIC_SECRET_0", "CLIENT_TRAFFIC_SECRET_0")
            };
            let (Some(tx_secret), Some(rx_secret)) = (secrets_log.get(tx_label), secrets_log.get(rx_label)) else {
                println!("RELAY[{tag}] KeyLog did not capture both traffic secrets; sending ERROR {}", since(t0));
                send(&mut s, &mut tally, &tag, t0, T_ERROR, b"traffic secrets unavailable", "");
                return;
            };
            // Host-side self-check: the secret really is the one behind rustls' key/IV.
            let tx_ok = hkdf_expand_label(&tx_secret, "key", 16) == tk.as_ref()
                && hkdf_expand_label(&tx_secret, "iv", 12) == ti.as_ref();
            let rx_ok = hkdf_expand_label(&rx_secret, "key", 16) == rk.as_ref()
                && hkdf_expand_label(&rx_secret, "iv", 12) == ri.as_ref();
            println!(
                "RELAY[{tag}] handshake complete: policy ALLOW local {local} <-> peer {peer}; suite={suite:?} tx_seq={tx_seq} rx_seq={rx_seq} buffered_plaintext={buffered_plaintext}; traffic secrets from rustls KeyLog tx={tx_label} rx={rx_label} (32 B each), host check HKDF(secret)==rustls key/iv tx={tx_ok} rx={rx_ok} {}",
                since(t0)
            );
            if rx_seq != 0 || tx_seq != 0 || buffered_plaintext || !tx_ok || !rx_ok {
                println!("RELAY[{tag}] invariant broken; sending ERROR");
                send(&mut s, &mut tally, &tag, t0, T_ERROR, b"relay invariant broken", "");
                return;
            }
            let payload = encode_secrets_v2(
                &DirSecrets { seq: tx_seq, secret: &tx_secret, key: tk.as_ref(), iv: ti.as_ref() },
                &DirSecrets { seq: rx_seq, secret: &rx_secret, key: rk.as_ref(), iv: ri.as_ref() },
                &peer,
            );
            send(
                &mut s,
                &mut tally,
                &tag,
                t0,
                T_SECRETS,
                &payload,
                &format!(
                    "(v2 suite=0x1301 tx: seq {tx_seq} secret 32B key {}B iv {}B; rx: seq {rx_seq} secret 32B key {}B iv {}B; peer id {}B)",
                    tk.as_ref().len(),
                    ti.as_ref().len(),
                    rk.as_ref().len(),
                    ri.as_ref().len(),
                    peer.len()
                ),
            );
            sent_secrets.push(tx_secret);
            sent_secrets.push(rx_secret);
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
            " {} SVID key occurrences PKCS#8 DER={} PEM={} private scalar={};",
            k.name,
            count(&tally.sent, &k.der),
            count(&tally.sent, &k.pem),
            count(&tally.sent, &k.scalar)
        );
    }
    let ts: Vec<String> = sent_secrets
        .iter()
        .map(|s| count(&tally.sent, s).to_string())
        .collect();
    c += &format!(
        " application traffic secrets sent (changed vs Spike B: expected 1 each when ALLOW) = [{}]",
        ts.join(",")
    );
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
    // relay <uds-path> <certdir> <t0-epoch-secs> <services-file> <health-dir>
    let a: Vec<String> = std::env::args().collect();
    let (uds, certdir, t0) = (a[1].clone(), a[2].clone(), a[3].parse::<f64>().unwrap());
    let services_file = a[4].clone();
    let health_dir = a[5].clone();
    let mut roots = RootCertStore::empty();
    for c in load_certs(&format!("{certdir}/ca.pem")) {
        roots.add(c).unwrap();
    }
    let client_id = spiffe_id(&load_certs(&format!("{certdir}/client.pem"))[0]).unwrap();
    let server_id = spiffe_id(&load_certs(&format!("{certdir}/guest-server.pem"))[0]).unwrap();
    let keys = vec![held(&certdir, "client"), held(&certdir, "guest-server")];
    let services = load_services(&services_file);
    let ctx = Arc::new(Ctx {
        certdir,
        client_id,
        server_id,
        roots: Arc::new(roots),
        keys,
        t0,
        services,
        health_dir,
    });
    let l = UnixListener::bind(&uds).unwrap();
    println!(
        "RELAY listening on unix {uds} pid={} client identity={} server identity={} (keys: client PKCS#8 {} B, guest-server PKCS#8 {} B, P-256 scalars 32 B, held in this process only; one thread per vsock connection) {}",
        std::process::id(),
        ctx.client_id,
        ctx.server_id,
        ctx.keys[0].der.len(),
        ctx.keys[1].der.len(),
        since(t0)
    );
    println!("RELAY service registry (D4, health-dir {}):", ctx.health_dir);
    for svc in &ctx.services {
        let b: Vec<String> = svc.backends.iter().map(|b| format!("{}:{}", b.0, b.1)).collect();
        println!(
            "RELAY   service {} vip {}:{} policy={} backends=[{}] expected_peer={}",
            svc.name, svc.vip.0, svc.vip.1, if svc.allow { "allow" } else { "deny" }, b.join(","), svc.expected_peer
        );
    }
    for (i, c) in l.incoming().enumerate() {
        match c {
            Ok(s) => {
                println!("RELAY[c{}] vsock connection accepted {}", i + 1, since(t0));
                let ctx = ctx.clone();
                thread::spawn(move || handle(i + 1, s, &ctx));
            }
            Err(e) => println!("RELAY accept error: {e}"),
        }
    }
}

//! Spike B2 TCP peer (GH #303, increment-e). Throwaway probe.
//!
//!   peer <certdir> <bind-ip> <t0-epoch-secs>
//!
//! Plays the far end of the guest's connections, on the tap address. Every TLS
//! listener requires a client certificate chained to the test CA, pins TLS 1.3
//! + TLS_AES_128_GCM_SHA256 and sends no 0.5-RTT data. Unlike Spike B, the
//! rustls listeners keep rustls' DEFAULT ticket behaviour (`send_tls13_tickets`
//! = 2 NewSessionTickets after the handshake), like an ordinary rustls server.
//!   :6443  echo, identity peer-allowed (client-first; non-blocking cases)
//!   :6444  echo, identity peer-denied   (the relay's policy denies it)
//!   :6445  server-speaks-first greeting, identity peer-allowed
//!   :6446  echo that calls refresh_traffic_keys() before every response, so
//!          each response is preceded by a KeyUpdate(update_requested)
//!   :6447  echo that follows its response with a multi-record bulk payload
//!   :6448  RAW: rustls handshake, then a hand-rolled record layer sends crafted
//!          post-handshake records in ONE TCP write: a NewSessionTicket split
//!          across two records, two tickets in one record, then app data; later
//!          a KeyUpdate(update_not_requested) followed by data under the new key
//!   :6449  RAW: a NewSessionTicket then an unexpected post-handshake handshake
//!          message (CertificateRequest, type 13) then app data that must never
//!          reach the guest application
//!   :6450  echo, identity peer-allowed (the blocking relay-kill case E)
//!   :5001  plain TCP line echo (the non-mesh pass-through destination)
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use igkme_host::{
    fake_nst, fnv1a32, hex, hkdf_expand_label, hs_msg, load_certs, load_key, provider,
    read_record, show, since, spiffe_id, RawDir, TrafficSecrets,
};
use rustls::server::WebPkiClientVerifier;
use rustls::{ConnectionTrafficSecrets, RootCertStore, ServerConfig, ServerConnection, StreamOwned};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Echo,
    ServerFirst,
    KeyUpdateEcho,
    Bulk,
    RawOk,
    RawBad,
}

const GREETING: &[u8] = b"IGKM-E-GREETING server-first peer->guest\n";
const RAW_GREETING: &[u8] = b"IGKM-E-GREETING raw-nst peer->guest\n";
const RAW_BAD_DATA: &[u8] = b"IGKM-E-SHOULD-NOT-ARRIVE raw-bad peer->guest\n";
pub const BULK_LEN: usize = 40_000;

fn bulk() -> Vec<u8> {
    (0..BULK_LEN).map(|i| ((i * 7 + 3) % 251) as u8).collect()
}

fn response_for(line: &[u8]) -> Vec<u8> {
    let s = String::from_utf8_lossy(line);
    s.replace("-REQ", "-RESP")
        .replace("guest->peer", "peer->guest")
        .replace("guest->host", "host->guest")
        .into_bytes()
}

/// Read one '\n'-terminated line from a byte stream (keeps leftovers).
fn read_line(r: &mut impl Read, carry: &mut Vec<u8>) -> io::Result<Option<Vec<u8>>> {
    loop {
        if let Some(i) = carry.iter().position(|b| *b == b'\n') {
            let rest = carry.split_off(i + 1);
            let line = std::mem::replace(carry, rest);
            return Ok(Some(line));
        }
        let mut buf = [0u8; 4096];
        let n = r.read(&mut buf)?;
        if n == 0 {
            return Ok(None);
        }
        carry.extend_from_slice(&buf[..n]);
    }
}

fn server_config(
    certdir: &str,
    identity: &str,
    raw: Option<Arc<TrafficSecrets>>,
) -> Arc<ServerConfig> {
    let mut roots = RootCertStore::empty();
    for c in load_certs(&format!("{certdir}/ca.pem")) {
        roots.add(c).unwrap();
    }
    let prov = provider();
    let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), prov.clone())
        .build()
        .unwrap();
    let mut cfg = ServerConfig::builder_with_provider(prov)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            load_certs(&format!("{certdir}/{identity}.pem")),
            load_key(&format!("{certdir}/{identity}.key")),
        )
        .unwrap();
    cfg.send_half_rtt_data = false;
    if let Some(kl) = raw {
        // Raw ports: nothing may be queued by rustls after the handshake, and
        // the hand-rolled record layer needs the traffic secrets.
        cfg.send_tls13_tickets = 0;
        cfg.enable_secret_extraction = true;
        cfg.key_log = kl;
    }
    Arc::new(cfg)
}

fn serve_tls(tcp: TcpStream, cfg: Arc<ServerConfig>, mode: Mode, tag: &str, t0: f64) {
    let conn = ServerConnection::new(cfg.clone()).unwrap();
    let mut s = StreamOwned::new(conn, tcp);
    let mut rx_total = 0usize;
    while s.conn.is_handshaking() {
        if let Err(e) = s.conn.complete_io(&mut s.sock) {
            println!(
                "PEER[{tag}] handshake FAILED: {e}; application plaintext bytes received=0 {}",
                since(t0)
            );
            return;
        }
    }
    let client_id = s
        .conn
        .peer_certificates()
        .and_then(|c| c.first())
        .and_then(|c| spiffe_id(c))
        .unwrap_or_else(|| "<none>".into());
    println!(
        "PEER[{tag}] handshake OK version={:?} suite={:?} verified client SPIFFE ID={client_id} mode={mode:?} send_tls13_tickets={} {}",
        s.conn.protocol_version().unwrap(),
        s.conn.negotiated_cipher_suite().unwrap().suite(),
        cfg.send_tls13_tickets,
        since(t0)
    );
    if mode == Mode::ServerFirst {
        s.write_all(GREETING).and_then(|_| s.flush()).unwrap();
        println!(
            "PEER[{tag}] sent greeting first ({} bytes): {} {}",
            GREETING.len(),
            show(GREETING),
            since(t0)
        );
    }
    let mut carry = Vec::new();
    let mut n = 0;
    loop {
        match read_line(&mut s, &mut carry) {
            Ok(Some(line)) => {
                n += 1;
                rx_total += line.len();
                println!(
                    "PEER[{tag}] rx plaintext line {n} ({} bytes): {} {}",
                    line.len(),
                    show(&line),
                    since(t0)
                );
                if mode == Mode::KeyUpdateEcho {
                    match s.conn.refresh_traffic_keys() {
                        Ok(()) => println!(
                            "PEER[{tag}] refresh_traffic_keys(): KeyUpdate(update_requested) queued ahead of response {n}; peer tx keys advance {}",
                            since(t0)
                        ),
                        Err(e) => println!("PEER[{tag}] refresh_traffic_keys FAILED: {e} {}", since(t0)),
                    }
                }
                let resp = response_for(&line);
                if let Err(e) = s.write_all(&resp).and_then(|_| s.flush()) {
                    println!("PEER[{tag}] tx FAILED: {e} {}", since(t0));
                    break;
                }
                println!(
                    "PEER[{tag}] tx plaintext line {n} ({} bytes): {} {}",
                    resp.len(),
                    show(&resp),
                    since(t0)
                );
                if mode == Mode::Bulk {
                    let b = bulk();
                    if let Err(e) = s.write_all(&b).and_then(|_| s.flush()) {
                        println!("PEER[{tag}] bulk tx FAILED: {e} {}", since(t0));
                        break;
                    }
                    println!(
                        "PEER[{tag}] tx bulk {} bytes (pattern (i*7+3)%251, fnv1a32=0x{:08x}; rustls splits it into 16384-byte records) {}",
                        b.len(),
                        fnv1a32(&b),
                        since(t0)
                    );
                }
            }
            Ok(None) => {
                println!(
                    "PEER[{tag}] clean close (close_notify received); application plaintext bytes received={rx_total} {}",
                    since(t0)
                );
                break;
            }
            Err(e) => {
                println!(
                    "PEER[{tag}] read ended: {e}; application plaintext bytes received={rx_total} {}",
                    since(t0)
                );
                break;
            }
        }
    }
    s.conn.send_close_notify();
    let _ = s.conn.complete_io(&mut s.sock);
    let _ = s.sock.shutdown(Shutdown::Both);
}

/// Raw ports: rustls runs the handshake, then the peer drops to its own record
/// layer (ring AES-128-GCM with the extracted keys) to send crafted records.
fn serve_raw(tcp: TcpStream, cfg: Arc<ServerConfig>, kl: Arc<TrafficSecrets>, mode: Mode, tag: &str, t0: f64) {
    let conn = ServerConnection::new(cfg).unwrap();
    let mut s = StreamOwned::new(conn, tcp);
    while s.conn.is_handshaking() {
        if let Err(e) = s.conn.complete_io(&mut s.sock) {
            println!("PEER[{tag}] handshake FAILED: {e} {}", since(t0));
            return;
        }
    }
    let client_id = s
        .conn
        .peer_certificates()
        .and_then(|c| c.first())
        .and_then(|c| spiffe_id(c))
        .unwrap_or_else(|| "<none>".into());
    let StreamOwned { conn, sock: mut tcp } = s;
    let ex = match conn.dangerous_extract_secrets() {
        Ok(x) => x,
        Err(e) => {
            println!("PEER[{tag}] extract secrets FAILED: {e} {}", since(t0));
            return;
        }
    };
    let (tx_seq, tx) = ex.tx;
    let (rx_seq, rx) = ex.rx;
    let (ConnectionTrafficSecrets::Aes128Gcm { key: tk, iv: ti }, ConnectionTrafficSecrets::Aes128Gcm { key: rk, iv: ri }) = (tx, rx) else {
        println!("PEER[{tag}] unexpected suite");
        return;
    };
    let tx_secret = kl.get("SERVER_TRAFFIC_SECRET_0").unwrap_or_default();
    let rx_secret = kl.get("CLIENT_TRAFFIC_SECRET_0").unwrap_or_default();
    let tx_ok = hkdf_expand_label(&tx_secret, "key", 16) == tk.as_ref();
    let mut txd = RawDir::new(tk.as_ref(), ti.as_ref(), tx_seq, tx_secret);
    let mut rxd = RawDir::new(rk.as_ref(), ri.as_ref(), rx_seq, rx_secret);
    println!(
        "PEER[{tag}] handshake OK verified client SPIFFE ID={client_id} mode={mode:?}; rustls tickets=0; switched to a hand-rolled record layer (tx_seq={tx_seq} rx_seq={rx_seq}, KeyLog secret matches rustls tx key={tx_ok}) {}",
        since(t0)
    );
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(15)));
    match mode {
        Mode::RawOk => raw_ok(&mut tcp, &mut txd, &mut rxd, tag, t0),
        Mode::RawBad => raw_bad(&mut tcp, &mut txd, &mut rxd, tag, t0),
        _ => unreachable!("serve_raw called for a non-raw mode"),
    }
    let _ = tcp.shutdown(Shutdown::Both);
}

fn describe(recs: &[(u8, Vec<u8>)]) -> String {
    recs.iter()
        .map(|(t, c)| format!("inner 0x{t:02x} {}B", c.len()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Read one record and expect an application-data line.
fn raw_read_line(tcp: &mut TcpStream, rxd: &mut RawDir, tag: &str, t0: f64) -> Option<Vec<u8>> {
    let rec = match read_record(tcp) {
        Ok(r) => r,
        Err(e) => {
            println!("PEER[{tag}] raw read ended: {e} {}", since(t0));
            return None;
        }
    };
    match rxd.open(&rec) {
        Ok((0x17, c)) => {
            println!(
                "PEER[{tag}] raw rx record rx_seq={} wire={} inner 0x17 plaintext ({} bytes): {} {}",
                rxd.seq - 1,
                rec.len(),
                c.len(),
                show(&c),
                since(t0)
            );
            Some(c)
        }
        Ok((t, c)) => {
            println!("PEER[{tag}] raw rx record inner 0x{t:02x} {} bytes: {} {}", c.len(), hex(&c), since(t0));
            None
        }
        Err(e) => {
            println!("PEER[{tag}] raw rx record FAILED: {e} {}", since(t0));
            None
        }
    }
}

fn raw_ok(tcp: &mut TcpStream, txd: &mut RawDir, rxd: &mut RawDir, tag: &str, t0: f64) {
    // Flight 1, ONE write: NST1 + first 20 bytes of NST2 | rest of NST2 |
    // NST3 + NST4 | application data.
    let (n1, n2, n3, n4) = (fake_nst(1), fake_nst(2), fake_nst(3), fake_nst(4));
    let mut r1 = n1.clone();
    r1.extend_from_slice(&n2[..20]);
    let r2 = n2[20..].to_vec();
    let mut r3 = n3.clone();
    r3.extend_from_slice(&n4);
    let plan: Vec<(u8, Vec<u8>)> = vec![(0x16, r1), (0x16, r2), (0x16, r3), (0x17, RAW_GREETING.to_vec())];
    let mut wire = Vec::new();
    let mut sizes = Vec::new();
    for (t, c) in &plan {
        let rec = txd.seal(*t, c);
        sizes.push(format!("0x17/{}", rec.len() - 5));
        wire.extend_from_slice(&rec);
    }
    if let Err(e) = tcp.write_all(&wire) {
        println!("PEER[{tag}] raw tx FAILED: {e}");
        return;
    }
    println!(
        "PEER[{tag}] raw tx ONE write of {} bytes = records [{}] = [{}]: NST1 (57 B) + first 20 B of NST2 | remaining 37 B of NST2 | NST3 + NST4 | greeting {} {}",
        wire.len(),
        sizes.join(" "),
        describe(&plan),
        show(RAW_GREETING),
        since(t0)
    );
    let Some(req) = raw_read_line(tcp, rxd, tag, t0) else { return };
    // Flight 2, ONE write: KeyUpdate(update_not_requested) under the old key,
    // then switch keys and send the response under the new key.
    let ku = txd.seal(0x16, &hs_msg(24, &[0]));
    let old_gen = txd.generation;
    txd.update();
    let resp = response_for(&req);
    let r = txd.seal(0x17, &resp);
    let mut w = ku.clone();
    w.extend_from_slice(&r);
    if let Err(e) = tcp.write_all(&w) {
        println!("PEER[{tag}] raw tx FAILED: {e}");
        return;
    }
    println!(
        "PEER[{tag}] raw tx ONE write: KeyUpdate(update_not_requested) record 0x17/{} under tx generation {old_gen}, then tx keys -> generation {} (seq 0), then response record 0x17/{} under the NEW key: {} {}",
        ku.len() - 5,
        txd.generation,
        r.len() - 5,
        show(&resp),
        since(t0)
    );
    // The guest was not asked to update: its tx keys (our rx) stay put.
    let Some(req2) = raw_read_line(tcp, rxd, tag, t0) else { return };
    let resp2 = response_for(&req2);
    let r2 = txd.seal(0x17, &resp2);
    let _ = tcp.write_all(&r2);
    println!(
        "PEER[{tag}] raw tx response 2 under tx generation {} seq {}: {} {}",
        txd.generation,
        txd.seq - 1,
        show(&resp2),
        since(t0)
    );
    // Expect the guest's close_notify.
    match read_record(tcp).map_err(|e| e.to_string()).and_then(|rec| rxd.open(&rec)) {
        Ok((0x15, c)) => println!("PEER[{tag}] raw rx alert {} (close_notify = 0100) {}", hex(&c), since(t0)),
        Ok((t, c)) => println!("PEER[{tag}] raw rx inner 0x{t:02x} {} {}", hex(&c), since(t0)),
        Err(e) => println!("PEER[{tag}] raw read ended: {e} {}", since(t0)),
    }
    let cn = txd.seal(0x15, &[1, 0]);
    let _ = tcp.write_all(&cn);
    println!("PEER[{tag}] raw tx close_notify; done {}", since(t0));
}

fn raw_bad(tcp: &mut TcpStream, txd: &mut RawDir, rxd: &mut RawDir, tag: &str, t0: f64) {
    // ONE write: NST | CertificateRequest (type 13, empty context, no
    // extensions) | application data that must never reach the application.
    let plan: Vec<(u8, Vec<u8>)> = vec![
        (0x16, fake_nst(9)),
        (0x16, hs_msg(13, &[0, 0, 0])),
        (0x17, RAW_BAD_DATA.to_vec()),
    ];
    let mut wire = Vec::new();
    let mut sizes = Vec::new();
    for (t, c) in &plan {
        let rec = txd.seal(*t, c);
        sizes.push(format!("0x17/{}", rec.len() - 5));
        wire.extend_from_slice(&rec);
    }
    if let Err(e) = tcp.write_all(&wire) {
        println!("PEER[{tag}] raw tx FAILED: {e}");
        return;
    }
    println!(
        "PEER[{tag}] raw tx ONE write of {} bytes = records [{}] = [{}]: NST | CertificateRequest(13) | {} {}",
        wire.len(),
        sizes.join(" "),
        describe(&plan),
        show(RAW_BAD_DATA),
        since(t0)
    );
    // Whatever the guest sends back (an alert, or nothing) before EOF.
    loop {
        match read_record(tcp) {
            Ok(rec) => match rxd.open(&rec) {
                Ok((t, c)) => println!(
                    "PEER[{tag}] raw rx record inner 0x{t:02x}: {} (0x15 0232 = fatal unexpected_message) {}",
                    hex(&c),
                    since(t0)
                ),
                Err(e) => {
                    println!("PEER[{tag}] raw rx record FAILED: {e} {}", since(t0));
                    break;
                }
            },
            Err(e) => {
                println!("PEER[{tag}] raw read ended: {e} (guest closed the connection) {}", since(t0));
                break;
            }
        }
    }
}

fn serve_plain(mut tcp: TcpStream, tag: &str, t0: f64) {
    let mut carry = Vec::new();
    let mut n = 0;
    loop {
        match read_line(&mut tcp, &mut carry) {
            Ok(Some(line)) => {
                n += 1;
                println!(
                    "PEER[{tag}] rx plaintext line {n} ({} bytes): {} {}",
                    line.len(),
                    show(&line),
                    since(t0)
                );
                let resp = response_for(&line);
                tcp.write_all(&resp).unwrap();
                println!(
                    "PEER[{tag}] tx plaintext line {n} ({} bytes): {} {}",
                    resp.len(),
                    show(&resp),
                    since(t0)
                );
            }
            Ok(None) => {
                println!("PEER[{tag}] EOF {}", since(t0));
                break;
            }
            Err(e) => {
                println!("PEER[{tag}] read ended: {e} {}", since(t0));
                break;
            }
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (certdir, ip, t0) = (a[1].clone(), a[2].clone(), a[3].parse::<f64>().unwrap());
    let tls: [(u16, &str, Mode); 8] = [
        (6443, "peer-allowed", Mode::Echo),
        (6444, "peer-denied", Mode::Echo),
        (6445, "peer-allowed", Mode::ServerFirst),
        (6446, "peer-allowed", Mode::KeyUpdateEcho),
        (6447, "peer-allowed", Mode::Bulk),
        (6448, "peer-allowed", Mode::RawOk),
        (6449, "peer-allowed", Mode::RawBad),
        (6450, "peer-allowed", Mode::Echo),
    ];
    let mut handles = Vec::new();
    for (port, identity, mode) in tls {
        let l = TcpListener::bind((ip.as_str(), port)).unwrap();
        let raw = matches!(mode, Mode::RawOk | Mode::RawBad);
        let id = spiffe_id(&load_certs(&format!("{certdir}/{identity}.pem"))[0]).unwrap();
        let tickets = server_config(&certdir, identity, None).send_tls13_tickets;
        println!(
            "PEER listening tls {ip}:{port} identity={id} mode={mode:?} tickets={} {}",
            if raw { "0 via rustls (crafted by the raw record layer)".to_string() } else { format!("{tickets} (rustls default)") },
            since(t0)
        );
        let certdir = certdir.clone();
        handles.push(thread::spawn(move || {
            for (i, c) in l.incoming().enumerate() {
                let tcp = c.unwrap();
                tcp.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
                let tag = format!("{port}#{}", i + 1);
                println!("PEER[{tag}] accepted tcp from {} {}", tcp.peer_addr().unwrap(), since(t0));
                if raw {
                    let kl = Arc::new(TrafficSecrets::default());
                    let cfg = server_config(&certdir, identity, Some(kl.clone()));
                    serve_raw(tcp, cfg, kl, mode, &tag, t0);
                } else {
                    serve_tls(tcp, server_config(&certdir, identity, None), mode, &tag, t0);
                }
            }
        }));
    }
    let l = TcpListener::bind((ip.as_str(), 5001)).unwrap();
    println!("PEER listening plain {ip}:5001 (non-mesh pass-through) {}", since(t0));
    handles.push(thread::spawn(move || {
        for (i, c) in l.incoming().enumerate() {
            let tcp = c.unwrap();
            tcp.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
            let tag = format!("5001#{}", i + 1);
            println!("PEER[{tag}] accepted tcp from {} {}", tcp.peer_addr().unwrap(), since(t0));
            serve_plain(tcp, &tag, t0);
        }
    }));
    println!("PEER ready {}", since(t0));
    for h in handles {
        h.join().unwrap();
    }
}

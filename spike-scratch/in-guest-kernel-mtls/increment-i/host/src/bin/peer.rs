//! Spike D TCP peer / backend set (GH #303, increment-i). Throwaway probe.
//!
//!   peer <certdir> <bind-ip> <t0-epoch-secs>
//!
//! The far end of the guest's mesh connections. Spike D stands up TWO backends
//! for ONE service ("svc-a"), on the tap address, so the agent can pick one by
//! first-healthy-by-Ord (ADR-0072 MtlsResolve) and the test can prove which one
//! a connection landed on:
//!
//!   :6443  echo, identity peer-allowed, label A, tickets=0  (svc-a backend, Ord-first)
//!   :6444  echo, identity peer-allowed, label B, tickets=0  (svc-a backend)
//!   :5001  plain TCP line echo (the non-mesh pass-through positive control)
//!
//! Both backends present the SAME service SPIFFE ID (peer-allowed) — a service's
//! instances share the service identity — on the SAME IP (192.168.204.1, the
//! cert's IP SAN) but distinct ports, so the serving port is the distinguishing
//! evidence. Each backend's echo response embeds `from-<label>` so the guest's
//! own plain read() proves which backend served it. Production-faithful TLS:
//! TLS 1.3 + TLS_AES_128_GCM_SHA256, client cert required, zero NewSessionTickets
//! (`send_tls13_tickets = 0`), no KeyUpdate.
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use igkme_host::{load_certs, load_key, provider, show, since, spiffe_id};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};

const GREETING: &[u8] = b"IGKM-D-GREETING server-first peer->guest\n";

/// Build the per-backend response: REQ->RESP, guest->peer->peer->guest, and the
/// serving backend's label spliced in so the guest read proves which backend
/// answered.
fn response_for(line: &[u8], label: &str) -> Vec<u8> {
    let s = String::from_utf8_lossy(line);
    s.replace("-REQ-", "-RESP-")
        .replace("guest->peer", &format!("from-{label} peer->guest"))
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

/// TLS 1.3 server config. Production-faithful: zero session tickets, no 0.5-RTT.
fn server_config(certdir: &str, identity: &str) -> Arc<ServerConfig> {
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
    cfg.send_tls13_tickets = 0;
    Arc::new(cfg)
}

fn serve_tls(tcp: TcpStream, cfg: Arc<ServerConfig>, label: &str, tag: &str, t0: f64) {
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
        "PEER[{tag}] handshake OK version={:?} suite={:?} verified client SPIFFE ID={client_id} backend-label={label} send_tls13_tickets=0 (production-faithful) {}",
        s.conn.protocol_version().unwrap(),
        s.conn.negotiated_cipher_suite().unwrap().suite(),
        since(t0)
    );
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
                let resp = response_for(&line, label);
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
                let resp = response_for(&line, "PT");
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
    let _ = GREETING; // reserved for a server-first variant; unused in Spike D's LB cases
    let a: Vec<String> = std::env::args().collect();
    let (certdir, ip, t0) = (a[1].clone(), a[2].clone(), a[3].parse::<f64>().unwrap());
    // (port, identity, backend-label). svc-a has two peer-allowed backends on
    // distinct ports; the agent picks one by first-healthy-by-Ord.
    let tls: [(u16, &str, &str); 2] = [
        (6443, "peer-allowed", "A"),
        (6444, "peer-allowed", "B"),
    ];
    let mut handles = Vec::new();
    for (port, identity, label) in tls {
        let l = TcpListener::bind((ip.as_str(), port)).unwrap();
        let id = spiffe_id(&load_certs(&format!("{certdir}/{identity}.pem"))[0]).unwrap();
        println!(
            "PEER listening tls {ip}:{port} identity={id} backend-label={label} send_tls13_tickets=0 (production-faithful: no NewSessionTicket) {}",
            since(t0)
        );
        let certdir = certdir.clone();
        handles.push(thread::spawn(move || {
            for (i, c) in l.incoming().enumerate() {
                let tcp = c.unwrap();
                tcp.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
                let tag = format!("{label}:{port}#{}", i + 1);
                println!("PEER[{tag}] accepted tcp from {} {}", tcp.peer_addr().unwrap(), since(t0));
                serve_tls(tcp, server_config(&certdir, identity), label, &tag, t0);
            }
        }));
    }
    let l = TcpListener::bind((ip.as_str(), 5001)).unwrap();
    println!("PEER listening plain {ip}:5001 (non-mesh pass-through) {}", since(t0));
    handles.push(thread::spawn(move || {
        for (i, c) in l.incoming().enumerate() {
            let tcp = c.unwrap();
            tcp.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
            let tag = format!("PT:5001#{}", i + 1);
            println!("PEER[{tag}] accepted tcp from {} {}", tcp.peer_addr().unwrap(), since(t0));
            serve_plain(tcp, &tag, t0);
        }
    }));
    println!("PEER ready {}", since(t0));
    for h in handles {
        h.join().unwrap();
    }
}

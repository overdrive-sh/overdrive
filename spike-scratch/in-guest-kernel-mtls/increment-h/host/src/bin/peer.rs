//! Spike C re-run TCP peer (GH #303, increment-h). Throwaway probe.
//!
//!   peer <certdir> <bind-ip> <t0-epoch-secs>
//!
//! Plays the far end of the guest's mesh connections, on the tap address. This
//! is the increment-h PRODUCTION-FAITHFUL peer: in production Overdrive controls
//! both mesh agents, so it configures the handshake to emit NO post-handshake
//! TLS 1.3 control records. Every rustls server here therefore pins TLS 1.3 +
//! TLS_AES_128_GCM_SHA256, requires a client certificate chained to the test CA,
//! sends no 0.5-RTT data, and sends ZERO NewSessionTickets
//! (`send_tls13_tickets = 0`). None of these listeners ever calls
//! `refresh_traffic_keys()`, so no KeyUpdate is ever initiated.
//!
//!   :6443  echo, identity peer-allowed, tickets=0   (CLEAN production echo path)
//!   :6444  echo, identity peer-denied,  tickets=0   (the relay's policy denies it)
//!   :6445  server-speaks-first greeting, peer-allowed, tickets=0 (CLEAN server-first)
//!   :6450  echo, identity peer-allowed, tickets=0   (the blocking relay-kill case)
//!   :6452  echo, identity peer-allowed, tickets=2   (CONTRAST ONLY: rustls'
//!          default NewSessionTicket behaviour, to show WHY the clean path needs
//!          tickets off -- a plain read() on this port hits the kTLS control
//!          record and returns EIO; a recvmsg()+cmsg reader drains it)
//!   :5001  plain TCP line echo (the non-mesh pass-through destination)
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use igkme_host::{load_certs, load_key, provider, show, since, spiffe_id};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Echo,
    ServerFirst,
}

const GREETING: &[u8] = b"IGKM-H-GREETING server-first peer->guest\n";

fn response_for(line: &[u8]) -> Vec<u8> {
    let s = String::from_utf8_lossy(line);
    s.replace("-REQ", "-RESP")
        .replace("guest->peer", "peer->guest")
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

/// Build a TLS 1.3 server config. `tickets` is the only increment-h knob:
/// the clean production path passes 0 (no NewSessionTicket, no post-handshake
/// control records); the single contrast port passes rustls' default of 2.
fn server_config(certdir: &str, identity: &str, tickets: usize) -> Arc<ServerConfig> {
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
    // The production-faithful lever: no session tickets on the clean mesh path.
    cfg.send_tls13_tickets = tickets;
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
        "PEER[{tag}] handshake OK version={:?} suite={:?} verified client SPIFFE ID={client_id} mode={mode:?} send_tls13_tickets={} (0 = production-faithful, no post-handshake control records) {}",
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
    // (port, identity, mode, send_tls13_tickets). Tickets=0 on every clean mesh
    // port; 6452 keeps rustls' default of 2 ONLY to demonstrate the EIO contrast.
    let tls: [(u16, &str, Mode, usize); 5] = [
        (6443, "peer-allowed", Mode::Echo, 0),
        (6444, "peer-denied", Mode::Echo, 0),
        (6445, "peer-allowed", Mode::ServerFirst, 0),
        (6450, "peer-allowed", Mode::Echo, 0),
        (6452, "peer-allowed", Mode::Echo, 2),
    ];
    let mut handles = Vec::new();
    for (port, identity, mode, tickets) in tls {
        let l = TcpListener::bind((ip.as_str(), port)).unwrap();
        let id = spiffe_id(&load_certs(&format!("{certdir}/{identity}.pem"))[0]).unwrap();
        println!(
            "PEER listening tls {ip}:{port} identity={id} mode={mode:?} send_tls13_tickets={tickets} {} {}",
            if tickets == 0 { "(production-faithful: no NewSessionTicket)" } else { "(rustls default: CONTRAST port, shows the EIO)" },
            since(t0)
        );
        let certdir = certdir.clone();
        handles.push(thread::spawn(move || {
            for (i, c) in l.incoming().enumerate() {
                let tcp = c.unwrap();
                tcp.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
                let tag = format!("{port}#{}", i + 1);
                println!("PEER[{tag}] accepted tcp from {} {}", tcp.peer_addr().unwrap(), since(t0));
                serve_tls(tcp, server_config(&certdir, identity, tickets), mode, &tag, t0);
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

//! Spike B inbound mTLS caller (GH #303, increment-d, stretch). Throwaway probe.
//!
//!   inclient <certdir> <identity> <ip:port> <t0-epoch-secs>
//!
//! A rustls TLS 1.3 client (TLS_AES_128_GCM_SHA256 only) presenting
//! `<identity>.pem` to the guest's plain accept() server. Verifies the server
//! certificate chains to the test CA with IP SAN = the guest address, logs the
//! server's SPIFFE ID (the relay-held `guest-server` SVID), writes one request
//! line and expects one byte-distinct response line.
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use igkmd_host::{load_certs, load_key, provider, show, since, spiffe_id};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

const REQ: &[u8] = b"IGKM-D-REQI inbound peer->guest\n";

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (certdir, ident, addr, t0) = (&a[1], &a[2], a[3].parse::<SocketAddr>().unwrap(), a[4].parse::<f64>().unwrap());
    let tag = format!("INCLIENT[{ident}]");
    let mut roots = RootCertStore::empty();
    for c in load_certs(&format!("{certdir}/ca.pem")) {
        roots.add(c).unwrap();
    }
    let mut cfg = ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots)
        .with_client_auth_cert(
            load_certs(&format!("{certdir}/{ident}.pem")),
            load_key(&format!("{certdir}/{ident}.key")),
        )
        .unwrap();
    cfg.resumption = rustls::client::Resumption::disabled();
    let me = spiffe_id(&load_certs(&format!("{certdir}/{ident}.pem"))[0]).unwrap();
    let conn = ClientConnection::new(Arc::new(cfg), ServerName::from(IpAddr::from(addr.ip()))).unwrap();
    let tcp = match TcpStream::connect_timeout(&addr, Duration::from_secs(10)) {
        Ok(t) => t,
        Err(e) => {
            println!("{tag} tcp connect {addr} FAILED: {e} {}", since(t0));
            return;
        }
    };
    tcp.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    println!("{tag} tcp connected to {addr} presenting {me} {}", since(t0));
    let mut s = StreamOwned::new(conn, tcp);
    while s.conn.is_handshaking() {
        if let Err(e) = s.conn.complete_io(&mut s.sock) {
            println!("{tag} handshake FAILED: {e} {}", since(t0));
            return;
        }
    }
    let server_id = s
        .conn
        .peer_certificates()
        .and_then(|c| c.first())
        .and_then(|c| spiffe_id(c))
        .unwrap_or_else(|| "<none>".into());
    println!(
        "{tag} client-side handshake done version={:?} suite={:?} server SPIFFE ID={server_id} {}",
        s.conn.protocol_version().unwrap(),
        s.conn.negotiated_cipher_suite().unwrap().suite(),
        since(t0)
    );
    match s.write_all(REQ).and_then(|_| s.flush()) {
        Ok(()) => println!("{tag} wrote {} bytes: {} {}", REQ.len(), show(REQ), since(t0)),
        Err(e) => {
            println!("{tag} write FAILED: {e} {}", since(t0));
            return;
        }
    }
    let mut got = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        match s.read(&mut buf) {
            Ok(0) => {
                println!("{tag} EOF (close_notify) after {} bytes {}", got.len(), since(t0));
                break;
            }
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                if got.ends_with(b"\n") {
                    println!("{tag} read {} bytes: {} {}", got.len(), show(&got), since(t0));
                }
            }
            Err(e) => {
                println!("{tag} read ended: {e}; received {} bytes {}", got.len(), since(t0));
                break;
            }
        }
    }
}

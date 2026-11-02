//! (e) does kTLS + sockmap coexist on this kernel? raw bpf(2) SOCKMAP, no program attached (membership only).
use gwk::*;
use std::net::TcpStream;
use std::os::fd::AsRawFd;

fn bpf(cmd: i32, attr: &mut [u8; 128]) -> i64 {
    unsafe { libc::syscall(libc::SYS_bpf, cmd, attr.as_mut_ptr(), 128) }
}
fn sockmap_create() -> i32 {
    let mut a = [0u8; 128];
    a[0..4].copy_from_slice(&15u32.to_ne_bytes()); // BPF_MAP_TYPE_SOCKMAP
    a[4..8].copy_from_slice(&4u32.to_ne_bytes());
    a[8..12].copy_from_slice(&4u32.to_ne_bytes());
    a[12..16].copy_from_slice(&8u32.to_ne_bytes());
    let r = bpf(0, &mut a);
    assert!(r >= 0, "BPF_MAP_CREATE sockmap: {}", std::io::Error::last_os_error());
    r as i32
}
fn sockmap_update(map: i32, key: u32, sockfd: i32) -> Result<(), std::io::Error> {
    let k = key; let v = sockfd as u32;
    let mut a = [0u8; 128];
    a[0..4].copy_from_slice(&(map as u32).to_ne_bytes());
    a[8..16].copy_from_slice(&((&k as *const u32) as u64).to_ne_bytes());
    a[16..24].copy_from_slice(&((&v as *const u32) as u64).to_ne_bytes());
    a[24..32].copy_from_slice(&0u64.to_ne_bytes());
    let r = bpf(2, &mut a);
    if r < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
}
fn sockmap_delete(map: i32, key: u32) {
    let k = key;
    let mut a = [0u8; 128];
    a[0..4].copy_from_slice(&(map as u32).to_ne_bytes());
    a[8..16].copy_from_slice(&((&k as *const u32) as u64).to_ne_bytes());
    bpf(3, &mut a);
}
fn tcp_pair_handshaken(suite_256: bool) -> (TcpStream, Established<rustls::client::ClientConnectionData>, TcpStream, Established<rustls::server::ServerConnectionData>) {
    let (l, addr) = listener();
    let th = std::thread::spawn(move || {
        let mut c = TcpStream::connect(addr).unwrap();
        let est = handshake_client(&mut c, client_config(suite_256)).unwrap();
        (c, est)
    });
    let (mut s, _) = l.accept().unwrap();
    let est_s = handshake_server(&mut s, server_config(None, 0, suite_256)).unwrap();
    let (c, est_c) = th.join().unwrap();
    (c, est_c, s, est_s)
}
fn r<T: std::fmt::Debug>(x: std::io::Result<T>) -> String { match x { Ok(v) => format!("OK {v:?}"), Err(e) => format!("ERR {}", errno_name(&e)) } }

fn main() {
    deadline_guard(60, "e");
    println!("uname -r: {}  t={}", uname(), now_ms());
    let map = sockmap_create();
    println!("sockmap created fd={map}");
    // E1: kTLS first, then sockmap
    {
        let (c, ec, s, es) = tcp_pair_handshaken(true);
        let (cf, sf) = (c.as_raw_fd(), s.as_raw_fd());
        println!("E1 kTLS-first: ULP={} TX={} RX={}", r(ktls_ulp(sf)), r(ktls_set(sf, libc::TLS_TX, es.secrets.tx.0, &es.secrets.tx.1)), r(ktls_set(sf, libc::TLS_RX, es.secrets.rx.0, &es.secrets.rx.1)));
        let _ = (cf, ec);
        println!("E1 then sockmap_update(sockmap, 0, tls_socket) => {}", r(sockmap_update(map, 0, sf)));
    }
    // E2: sockmap first, then kTLS
    {
        let (c, ec, s, es) = tcp_pair_handshaken(true);
        let (cf, sf) = (c.as_raw_fd(), s.as_raw_fd());
        println!("E2 sockmap-first: sockmap_update(sockmap, 1, plain_tcp_socket) => {}", r(sockmap_update(map, 1, sf)));
        println!("E2 then ULP=tls => {}", r(ktls_ulp(sf)));
        println!("E2 then TLS_TX => {}", r(ktls_set(sf, libc::TLS_TX, es.secrets.tx.0, &es.secrets.tx.1)));
        println!("E2 then TLS_RX => {}", r(ktls_set(sf, libc::TLS_RX, es.secrets.rx.0, &es.secrets.rx.1)));
        println!("E2 client side arm (plain, not in map): {}", r(arm_both(cf, &ec.secrets)));
        // data path on the sockmap member with kTLS (no verdict program attached)
        let w = send_rec(cf, REC_DATA, &pattern(0, 4000));
        let mut b = vec![0u8; 8192];
        let rr = recv_rec(sf, &mut b, 0);
        println!("E2 data path C->S(sockmap+kTLS): send={:?} recv={:?} exact={}", w.map_err(|e| errno_name(&e)), rr.as_ref().map(|x| x.1).map_err(|e| errno_name(e)), rr.as_ref().map(|x| b[..x.1] == pattern(0, 4000)[..]).unwrap_or(false));
        let w = send_rec(sf, REC_DATA, &pattern(0, 3000));
        let rr = recv_rec(cf, &mut b, 0);
        println!("E2 data path S->C: send={:?} recv={:?}", w.map_err(|e| errno_name(&e)), rr.as_ref().map(|x| x.1).map_err(|e| errno_name(e)));
        sockmap_delete(map, 1);
    }
    // E3: sockmap-first then ULP only on a *listening-accepted* socket via different sequence: skip
    println!("tls_stat: {}", tls_stat());
}

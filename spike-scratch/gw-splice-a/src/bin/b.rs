//! Spike (b): TLS1.3 KeyUpdate with kTLS RX/TX + rustls kernel connection.
use gwk::*;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::time::Duration;

fn run(suite_256: bool, n_updates: usize, chunk: usize) {
    println!("\n=== b1 KeyUpdate via rustls peer: suite={} updates={n_updates} chunk={chunk} uname={}", if suite_256 { "AES256GCM" } else { "AES128GCM" }, uname());
    let (txd, rxd) = chan();
    let total = chunk * (n_updates + 1);
    let s_script: Box<dyn FnOnce(&mut SStream) + Send> = Box::new(move |st| {
        let mut off = 0;
        for i in 0..=n_updates {
            st.write_all(&pattern(off, chunk)).unwrap();
            st.flush().unwrap();
            off += chunk;
            if i < n_updates {
                st.conn.refresh_traffic_keys().unwrap(); // KeyUpdate(update_requested)
                st.flush().unwrap();
            }
        }
        txd.send(()).unwrap();
        // gateway answers "OK" after relaying everything; reading proves our RX follows the gateway's TX rekey
        let mut b = [0u8; 2];
        match st.read_exact(&mut b) {
            Ok(()) => println!("  S(rustls peer) read after rekeys: {:?}  => {}", String::from_utf8_lossy(&b), if &b == b"OK" { "OK" } else { "BAD" }),
            Err(e) => println!("  S(rustls peer) read FAILED: {e}"),
        }
        std::thread::sleep(Duration::from_millis(100));
    });
    let (mut gw, s, r) = build_gw(0, suite_256, s_script, Box::new(r_sink));
    rxd.recv().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let (a, b) = (gw.a.as_raw_fd(), gw.b.as_raw_fd());
    let mut relay = Relay::new(1 << 16, true);
    let mut ku = KuServer { fd: a, kc: &mut gw.a_kc, log: vec![], rx_rekeys: 0, tx_rekeys: 0 };
    let res = relay.body_exact(a, b, total, &mut ku);
    println!("  body_exact({total}) => {:?}  stat={:?}", res.as_ref().map(|c| matches!(c, Ctl::Continue)).map_err(|e| errno_name(e)), relay.stat);
    for l in &ku.log { println!("  gw: {l}"); }
    println!("  gw rekeys: rx={} tx={}", ku.rx_rekeys, ku.tx_rekeys);
    let w = send_rec(a, REC_DATA, b"OK");
    println!("  gw sends OK on A (new TX key): {:?}", w.map_err(|e| errno_name(&e)));
    unsafe { libc::shutdown(b, libc::SHUT_WR) };
    s.join().unwrap();
    let res = r.join().unwrap();
    println!("  R(upstream) received n={} (want {total}) mismatch={:?} => {}", res.0, res.1, if res.0 == total && res.1.is_none() { "EXACT" } else { "BAD" });
    println!("  tls_stat: {}", tls_stat());
}

/// negative control: gateway ignores the KeyUpdate (does not re-arm RX).
fn no_rearm() {
    println!("\n=== b2 negative control: KeyUpdate record handled but TLS_RX NOT re-armed");
    let p = ktls_pair(true);
    let (cfd, sfd) = (p.c.as_raw_fd(), p.s.as_raw_fd());
    let mut c_kc = p.c_kc;
    send_rec(cfd, REC_DATA, &pattern(0, 100)).unwrap();
    send_rec(cfd, REC_HANDSHAKE, &[24, 0, 0, 1, 0]).unwrap();
    let (seq, sec) = kc_tx_c(&mut c_kc).unwrap();
    ktls_set(cfd, libc::TLS_TX, seq, &sec).unwrap();
    send_rec(cfd, REC_DATA, &pattern(100, 100)).unwrap(); // under the NEW key
    std::thread::sleep(Duration::from_millis(100));
    let mut buf = [0u8; 256];
    let r = recv_rec(sfd, &mut buf, libc::MSG_DONTWAIT);
    println!("  recvmsg#1: {:?}", r.map_err(|e| errno_name(&e)));
    let r = recv_rec(sfd, &mut buf, libc::MSG_DONTWAIT);
    println!("  recvmsg#2 (KeyUpdate rec): {:?} bytes={:02x?}", r.as_ref().map_err(|e| errno_name(e)), &buf[..5]);
    let r = recv_rec(sfd, &mut buf, libc::MSG_DONTWAIT);
    println!("  recvmsg#3 (without TLS_RX re-arm): {:?}", r.map_err(|e| errno_name(&e)));
    let r = recv_rec(sfd, &mut buf, libc::MSG_DONTWAIT);
    println!("  recvmsg#4 (again, sticky?):        {:?}", r.map_err(|e| errno_name(&e)));
    let mut pfd = libc::pollfd { fd: sfd, events: libc::POLLIN, revents: 0 };
    let n = unsafe { libc::poll(&mut pfd, 1, 200) };
    println!("  poll(POLLIN) while paused: n={n} revents={:#x}", pfd.revents);
    println!("  tls_stat: {}", tls_stat());
}

/// both ends kTLS, KeyUpdate requested=1 sent by raw record; responder procedure on both roles; data flows both ways after.
fn pair_roundtrip() {
    println!("\n=== b3 kTLS<->kTLS full procedure incl. update_requested=1 both directions, 3 rounds");
    let p = ktls_pair(true);
    let (cfd, sfd) = (p.c.as_raw_fd(), p.s.as_raw_fd());
    let mut c_kc = p.c_kc;
    let mut s_kc = p.s_kc;
    for fd in [cfd, sfd] { let tv = libc::timeval { tv_sec: 2, tv_usec: 0 }; unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVTIMEO, (&tv as *const libc::timeval).cast(), 16) }; }
    let mut seq_off = 0usize;
    for round in 0..3 {
        // C initiates: KeyUpdate(update_requested=1), switch TX
        send_rec(cfd, REC_DATA, &pattern(seq_off, 64)).unwrap();
        send_rec(cfd, REC_HANDSHAKE, &[24, 0, 0, 1, 1]).unwrap();
        let (seq, sec) = kc_tx_c(&mut c_kc).unwrap();
        ktls_set(cfd, libc::TLS_TX, seq, &sec).unwrap();
        send_rec(cfd, REC_DATA, &pattern(seq_off + 64, 64)).unwrap();
        // S (server role) runs the procedure through the relay machinery
        let mut relay = Relay::new(1 << 16, true);
        let (pr, pw) = (relay.pr, relay.pw);
        let _ = (pr, pw);
        let mut ku = KuServer { fd: sfd, kc: &mut s_kc, log: vec![], rx_rekeys: 0, tx_rekeys: 0 };
        let (spr, spw) = mkpipe(None);
        let mut got = vec![];
        loop {
            match splice(sfd, spw, 4096, 0) {
                Ok(n) if n > 0 => { got.extend(drain_vec(spr)); let _ = n; if got.len() >= 128 { break; } }
                Ok(_) => break,
                Err(e) if e.raw_os_error() == Some(libc::EINVAL) => {
                    let mut buf = [0u8; 64];
                    let (t, n) = recv_rec(sfd, &mut buf, 0).unwrap();
                    ku.on_control(t, &buf[..n]).unwrap();
                }
                Err(e) => { println!("  round {round}: splice err {}", errno_name(&e)); break; }
            }
        }
        println!("  round {round}: S received {} bytes, exact={}; S log: {:?}", got.len(), got == pattern(seq_off, 128), ku.log);
        // S -> C data after S re-keyed TX: C must decode with updated RX (C handles the KeyUpdate it receives)
        send_rec(sfd, REC_DATA, &pattern(1000, 32)).unwrap();
        let mut cku = KuClient { fd: cfd, kc: &mut c_kc, log: vec![], rx_rekeys: 0, tx_rekeys: 0 };
        let mut data = vec![];
        for _ in 0..4 {
            let mut buf = [0u8; 256];
            match recv_rec(cfd, &mut buf, 0) {
                Ok((REC_DATA, n)) => { data.extend_from_slice(&buf[..n]); if data.len() >= 32 { break; } }
                Ok((t, n)) => { cku.on_control(t, &buf[..n]).unwrap(); }
                Err(e) => { println!("  round {round}: C recv err {}", errno_name(&e)); break; }
            }
        }
        println!("  round {round}: C received {} bytes, exact={}; C log: {:?}", data.len(), data == pattern(1000, 32), cku.log);
        seq_off += 128;
    }
    println!("  tls_stat: {}", tls_stat());
}

fn drain_vec(pr: i32) -> Vec<u8> {
    let n = pipe_bytes(pr) as usize;
    let mut v = vec![0u8; n];
    let mut off = 0;
    while off < n {
        let k = unsafe { libc::read(pr, v[off..].as_mut_ptr().cast(), n - off) };
        assert!(k > 0);
        off += k as usize;
    }
    v
}

/// upstream (rustls server) sends 2 NewSessionTickets right after the handshake: a NON-DATA record at the head of the gateway's back-side RX.
fn tickets() {
    println!("\n=== b4 upstream sends TLS1.3 NewSessionTickets (non-DATA, type 22/NewSessionTicket) before the response body");
    let (txd, rxd) = chan();
    let s_script: Box<dyn FnOnce(&mut SStream) + Send> = Box::new(move |st| {
        st.write_all(&pattern(0, 10)).unwrap();
        st.flush().unwrap();
        txd.send(()).unwrap();
        let mut b = vec![0u8; 500];
        let r = st.read_exact(&mut b);
        println!("  S(rustls peer) read 500 response bytes: {:?} exact={}", r.map_err(|e| e.to_string()), b == pattern(0, 500));
    });
    let r_script: Box<dyn FnOnce(&mut RStream) -> usize + Send> = Box::new(move |st| {
        let mut b = [0u8; 10];
        st.read_exact(&mut b).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        st.write_all(&pattern(0, 500)).unwrap();
        st.flush().unwrap();
        std::thread::sleep(Duration::from_millis(200));
        0
    });
    let (mut gw, s, r) = build_gw(2, true, s_script, r_script);
    rxd.recv().unwrap();
    let (a, b) = (gw.a.as_raw_fd(), gw.b.as_raw_fd());
    let mut relay = Relay::new(1 << 16, false);
    let mut kua = KuServer { fd: a, kc: &mut gw.a_kc, log: vec![], rx_rekeys: 0, tx_rekeys: 0 };
    let mut kub = KuClient { fd: b, kc: &mut gw.b_kc, log: vec![], rx_rekeys: 0, tx_rekeys: 0 };
    let r1 = relay.body_exact(a, b, 10, &mut kua);
    let r2 = relay.body_exact(b, a, 500, &mut kub);
    println!("  gw req 10B => {:?}; resp 500B => {:?}; ctl records handled on back side: {} log={:?}", r1.map_err(|e| errno_name(&e)), r2.map_err(|e| errno_name(&e)), relay.stat.ctl, kub.log);
    s.join().unwrap();
    let _ = r.join().unwrap();
}

fn main() {
    deadline_guard(300, "b");
    println!("uname -r: {} t={}", uname(), now_ms());
    run(true, 2, 5000);
    run(false, 1, 30000);
    no_rearm();
    pair_roundtrip();
    tickets();
}

//! Spike (c): exact-length / chunked splice relay, backpressure, keep-alive, perf.
use gwk::*;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn env_usize(k: &str, d: usize) -> usize { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }

/// Keep-alive relay over `rec`-sized records; messages straddle record boundaries.
fn c1(rec: usize, ku_c: bool, ku_u: bool, label: &str) { c1x(rec, ku_c, ku_u, false, label) }
fn c1x(rec: usize, ku_c: bool, ku_u: bool, edge: bool, label: &str) {
    println!("\n=== c1 {label}: keep-alive 4 req/resp, C record size={rec}, KeyUpdate from C={ku_c} from U={ku_u}; uname={}", uname());
    let GwK { mut gw, c, mut c_kc, u, mut u_kc } = build_gw_k(true);
    let reqs: Vec<Vec<u8>> = vec![
        msg_cl("POST /m1 HTTP/1.1", 20000, 0),
        msg_chunked("POST /m2 HTTP/1.1", &[1000, 5000, 70000], 0),
        msg_cl("GET /m3 HTTP/1.1", 0, 0),
        msg_cl("POST /m4 HTTP/1.1", 100, 0),
    ];
    let resps: Vec<Vec<u8>> = vec![
        msg_cl("HTTP/1.1 200 OK", 70000, 0),
        msg_chunked("HTTP/1.1 200 OK", &[3000, 9000, 1], 0),
        msg_cl("HTTP/1.1 204 No Content", 0, 0),
        msg_cl("HTTP/1.1 200 OK", 5, 0),
    ];
    let wire_c: Vec<u8> = reqs.concat();
    let wire_u: Vec<u8> = resps.concat();
    // KeyUpdate offsets in the sender streams (mid-body of message 1/2)
    let mut ku_at_c: Vec<usize> = if ku_c { vec![reqs[0].len() - 9000, reqs[0].len() + 20000] } else { vec![] };
    let mut ku_at_u: Vec<usize> = if ku_u { vec![resps[0].len() - 30000, resps[0].len() + 2000] } else { vec![] };
    if edge {
        // KeyUpdate exactly at a message boundary, INSIDE a head, and inside chunk framing
        ku_at_c = vec![reqs[0].len(), reqs[0].len() + 10, reqs[0].len() + reqs[1].len() - 3];
        ku_at_u = vec![resps[0].len(), resps[0].len() + 7, resps[0].len() + resps[1].len() - 3];
    }
    let (cfd, ufd) = (c.as_raw_fd(), u.as_raw_fd());
    // --- U: upstream, parses each request (over kTLS), validates it, answers with its response
    let u_res = {
        let wire_u = wire_u.clone();
        let resps_len: Vec<usize> = resps.iter().map(|r| r.len()).collect();
        let nreq = reqs.len();
        std::thread::spawn(move || {
            let mut http = Http::new(u.try_clone().unwrap());
            let mut log = vec![];
            let mut woff = 0usize;
            // answer KeyUpdate from the gateway: U must also run the control-record procedure when reading.
            for i in 0..nreq {
                let head = match read_head_ku(&mut http, ufd, &mut u_kc) { Ok(h) => h, Err(e) => { log.push(format!("U head err {e}")); break; } };
                let n = match body_ku(&mut http, &head, ufd, &mut u_kc) { Ok(n) => n, Err(e) => { log.push(format!("U body err {e}")); break; } };
                log.push(format!("U req{} ok: head='{}' body={}", i + 1, head.lines().next().unwrap_or(""), n));
                let end = woff + resps_len[i];
                send_stream(ufd, &wire_u[woff..end], 16384, &ku_rel(&ku_at_u, woff, end), || kc_tx_s(&mut u_kc), None).unwrap();
                woff = end;
            }
            (log, u)
        })
    };
    // --- C: writer thread + reader (main)
    let cw = {
        let c2 = c.try_clone().unwrap();
        let wire = wire_c.clone();
        std::thread::spawn(move || {
            let fd = c2.as_raw_fd();
            send_stream(fd, &wire, rec, &ku_at_c, || kc_tx_c(&mut c_kc), None).unwrap();
            (c2, c_kc)
        })
    };
    // --- gateway
    std::thread::sleep(Duration::from_millis(300));
    let (a, b) = (gw.a.as_raw_fd(), gw.b.as_raw_fd());
    let mut relay = Relay::new(1 << 16, false);
    let mut kua = KuServer { fd: a, kc: &mut gw.a_kc, log: vec![], rx_rekeys: 0, tx_rekeys: 0 };
    let mut kub = KuClient { fd: b, kc: &mut gw.b_kc, log: vec![], rx_rekeys: 0, tx_rekeys: 0 };
    let t0 = Instant::now();
    let mut stage_ok = true;
    for i in 0..4 {
        let before = relay.stat.clone();
        match relay.http_message(a, b, &mut kua) { Ok(true) => {}, other => { println!("  gw req{} => {:?}", i + 1, other.map_err(|e| errno_name(&e))); stage_ok = false; break; } }
        if i == 0 {
            // prove: nothing beyond message 1 was consumed - the next message head is still the first thing on A
            let mut pk = [0u8; 24];
            let r = recv_rec(a, &mut pk, libc::MSG_PEEK | libc::MSG_DONTWAIT);
            println!("  after req1 body: PEEK on A -> {:?} = {:?}", r.as_ref().map(|x| x.1).map_err(|e| errno_name(e)), String::from_utf8_lossy(&pk[..r.as_ref().map(|x| x.1).unwrap_or(0).min(24)]));
        }
        match relay.http_message(b, a, &mut kub) { Ok(true) => {}, other => { println!("  gw resp{} => {:?}", i + 1, other.map_err(|e| errno_name(&e))); stage_ok = false; break; } }
        let st = &relay.stat;
        println!("  msg{}: splice_in+{} splice_out+{} send+{} peek+{} ctl+{}", i + 1, st.splice_in - before.splice_in, st.splice_out - before.splice_out, st.send_calls - before.send_calls, st.peek_calls - before.peek_calls, st.ctl - before.ctl);
    }
    println!("  gateway relayed all 4 exchanges: {stage_ok}  in {:?}  stat={:?}", t0.elapsed(), relay.stat);
    for l in kua.log.iter().chain(kub.log.iter()) { println!("  gw: {l}"); }
    // --- C verifies responses (needs its own KeyUpdate handling when U rekeys; c_kc was moved: use writer's returned kc)
    let (c2, mut c_kc) = cw.join().unwrap();
    let mut http = Http::new(c2.try_clone().unwrap());
    let mut ok = true;
    for i in 0..4 {
        let h = match read_head_ku_c(&mut http, cfd, &mut c_kc) { Ok(h) => h, Err(e) => { println!("  C resp{} head err {e}", i + 1); ok = false; break } };
        match body_ku_c(&mut http, &h, cfd, &mut c_kc) { Ok(n) => println!("  C resp{} ok: '{}' body={}", i + 1, h.lines().next().unwrap(), n), Err(e) => { println!("  C resp{} body err {e}", i + 1); ok = false; break } }
    }
    let (ulog, _u) = u_res.join().unwrap();
    for l in ulog { println!("  {l}"); }
    println!("  RESULT c1 {label}: {}", if ok && stage_ok { "EXACT end-to-end (every body pattern-verified by the far peer)" } else { "FAILED" });
    println!("  tls_stat: {}", tls_stat());
}

fn ku_rel(abs: &[usize], lo: usize, hi: usize) -> Vec<usize> { abs.iter().filter(|&&p| p >= lo && p < hi).map(|p| p - lo).collect() }

// Userspace HTTP readers over a kTLS fd that run the KeyUpdate procedure when read() reports a control record.
macro_rules! ku_reader {
    ($head:ident, $body:ident, $kc:ty, $upd_rx:ident, $upd_tx:ident) => {
        fn $head(h: &mut Http<std::net::TcpStream>, fd: i32, kc: &mut KernelConnection<$kc>) -> std::io::Result<String> {
            loop {
                match h.head() {
                    Ok(x) => return Ok(x),
                    Err(e) if e.raw_os_error() == Some(5) => ctl_fallback(fd, kc)?,
                    Err(e) => return Err(e),
                }
            }
        }
        fn $body(h: &mut Http<std::net::TcpStream>, head: &str, fd: i32, kc: &mut KernelConnection<$kc>) -> std::io::Result<usize> {
            // re-implement body with ctl fallback: wrap by attempting repeatedly via a manual loop
            let lh = head.to_ascii_lowercase();
            let mut total = 0usize;
            let mut chk = |d: &[u8], t: &mut usize| -> std::io::Result<()> {
                for (i, b) in d.iter().enumerate() { if *b != pat(*t + i) { return Err(std::io::Error::other(format!("PATTERN MISMATCH at {}", *t + i))); } }
                *t += d.len(); Ok(())
            };
            let mut take = |h: &mut Http<std::net::TcpStream>, n: usize, t: &mut usize, kc: &mut KernelConnection<$kc>| -> std::io::Result<()> {
                let mut left = n;
                while left > 0 {
                    if h.buf.is_empty() {
                        let mut tmp = vec![0u8; 16384];
                        match recv_rec(fd, &mut tmp, 0) {
                            Ok((REC_DATA, 0)) => return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof")),
                            Ok((REC_DATA, k)) => h.buf.extend_from_slice(&tmp[..k]),
                            Ok((ty, k)) => ctl_apply(fd, kc, ty, &tmp[..k])?,
                            Err(e) => return Err(e),
                        }
                        continue;
                    }
                    let k = left.min(h.buf.len());
                    let d: Vec<u8> = h.buf.drain(..k).collect();
                    chk(&d, t)?;
                    left -= k;
                }
                Ok(())
            };
            let mut line = |h: &mut Http<std::net::TcpStream>, kc: &mut KernelConnection<$kc>| -> std::io::Result<String> {
                loop {
                    if let Some(p) = h.buf.windows(2).position(|w| w == b"\r\n") {
                        let v: Vec<u8> = h.buf.drain(..p + 2).collect();
                        return Ok(String::from_utf8_lossy(&v[..v.len() - 2]).into_owned());
                    }
                    let mut tmp = vec![0u8; 16384];
                    match recv_rec(fd, &mut tmp, 0) {
                        Ok((REC_DATA, 0)) => return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof")),
                        Ok((REC_DATA, k)) => h.buf.extend_from_slice(&tmp[..k]),
                        Ok((ty, k)) => ctl_apply(fd, kc, ty, &tmp[..k])?,
                        Err(e) => return Err(e),
                    }
                }
            };
            if lh.contains("transfer-encoding: chunked") {
                loop {
                    let l = line(h, kc)?;
                    let sz = usize::from_str_radix(l.trim(), 16).map_err(|_| std::io::Error::other("bad chunk"))?;
                    if sz == 0 { let _ = line(h, kc)?; break; }
                    let mut t = total;
                    take(h, sz, &mut t, kc)?;
                    total = t;
                    let c = line(h, kc)?;
                    assert!(c.is_empty());
                }
            } else if let Some(p) = lh.find("content-length:") {
                let n: usize = lh[p + 15..].lines().next().unwrap().trim().parse().unwrap();
                let mut t = 0;
                take(h, n, &mut t, kc)?;
                total = t;
            }
            Ok(total)
        }
    };
}
trait KcOps { fn rx(&mut self) -> std::io::Result<(u64, rustls::ConnectionTrafficSecrets)>; fn tx(&mut self) -> std::io::Result<(u64, rustls::ConnectionTrafficSecrets)>; }
impl KcOps for KernelConnection<rustls::server::ServerConnectionData> { fn rx(&mut self) -> std::io::Result<(u64, rustls::ConnectionTrafficSecrets)> { kc_rx_s(self) } fn tx(&mut self) -> std::io::Result<(u64, rustls::ConnectionTrafficSecrets)> { kc_tx_s(self) } }
impl KcOps for KernelConnection<rustls::client::ClientConnectionData> { fn rx(&mut self) -> std::io::Result<(u64, rustls::ConnectionTrafficSecrets)> { kc_rx_c(self) } fn tx(&mut self) -> std::io::Result<(u64, rustls::ConnectionTrafficSecrets)> { kc_tx_c(self) } }
use rustls::kernel::KernelConnection;
fn ctl_apply<K>(fd: i32, kc: &mut K, ty: u8, p: &[u8]) -> std::io::Result<()> where K: KcOps {
    if ty == REC_HANDSHAKE && p.first() == Some(&24) && p.len() == 5 {
        let (s, k) = kc.rx()?; ktls_set(fd, libc::TLS_RX, s, &k)?;
        if p[4] == 1 { send_rec(fd, REC_HANDSHAKE, &[24, 0, 0, 1, 0])?; let (s, k) = kc.tx()?; ktls_set(fd, libc::TLS_TX, s, &k)?; }
        Ok(())
    } else if ty == REC_ALERT { Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "alert")) } else { Ok(()) }
}
fn ctl_fallback<K>(fd: i32, kc: &mut K) -> std::io::Result<()> where K: KcOps {
    let mut tmp = vec![0u8; 64];
    let (ty, k) = recv_rec(fd, &mut tmp, 0)?;
    ctl_apply(fd, kc, ty, &tmp[..k])
}
ku_reader!(read_head_ku, body_ku, rustls::server::ServerConnectionData, rx, tx);
ku_reader!(read_head_ku_c, body_ku_c, rustls::client::ClientConnectionData, rx, tx);

/// backpressure: slow reader at U, bounded in-flight, RSS flat.
fn c3(total_mib: usize, nonblocking: bool) { c3s(total_mib, nonblocking, 0) }
fn c3s(total_mib: usize, nonblocking: bool, stall_ms: u64) {
    println!("\n=== c3 backpressure: POST CL={total_mib}MiB, U reads 16KiB/2ms (~8MB/s), SO_SNDBUF(C,B)=SO_RCVBUF(A,U)=64KiB, gateway nonblocking+poll={nonblocking}");
    let GwK { gw, c, c_kc: _, u, u_kc: _ } = build_gw_k(true);
    let (cfd, ufd, a, b) = (c.as_raw_fd(), u.as_raw_fd(), gw.a.as_raw_fd(), gw.b.as_raw_fd());
    for (fd, opt) in [(cfd, libc::SO_SNDBUF), (b, libc::SO_SNDBUF), (a, libc::SO_RCVBUF), (ufd, libc::SO_RCVBUF)] { sockopt_i32(fd, libc::SOL_SOCKET, opt, 65536); }
    if nonblocking { set_nonblock(a, true); set_nonblock(b, true); }
    let total = total_mib << 20;
    let sent = Arc::new(AtomicU64::new(0));
    let recvd = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let head = format!("POST /big HTTP/1.1\r\nHost: gw\r\nContent-Length: {total}\r\n\r\n").into_bytes();
    // C sender
    let cs = { let sent = sent.clone(); let c2 = c.try_clone().unwrap(); std::thread::spawn(move || {
        let fd = c2.as_raw_fd();
        let fp = pattern(0, 251 * 261);
        let mut data = head.clone();
        send_stream(fd, &data, 16384, &[], || unreachable!(), None).unwrap();
        data.clear();
        let mut n = 0usize;
        while n < total {
            let take = (total - n).min(fp.len());
            let s = n % 251;
            let mut chunk = pattern(n, take); // exact stream offset pattern
            let _ = (s, &fp);
            send_stream(fd, &chunk, 16384, &[], || unreachable!(), Some(&sent)).unwrap();
            chunk.clear();
            n += take;
        }
        c2
    }) };
    if stall_ms > 0 { let sent = sent.clone(); let rc = recvd.clone(); std::thread::spawn(move || { for t in [500u64, 1500, 2500] { std::thread::sleep(Duration::from_millis(if t == 500 { 500 } else { 1000 })); println!("  [stall] t={t}ms C_sent={} KiB U_received={} KiB rss={} KiB", sent.load(Ordering::Relaxed) / 1024, rc.load(Ordering::Relaxed) / 1024, vm_rss_kb()); } }); }
    // U slow receiver: parse head via recv then count body
    let ur = { let recvd = recvd.clone(); let u2 = u.try_clone().unwrap(); std::thread::spawn(move || {
        let fd = u2.as_raw_fd();
        if stall_ms > 0 { std::thread::sleep(Duration::from_millis(stall_ms)); }
        let fpat = FastPat::new();
        let mut h = Http::new(u2.try_clone().unwrap());
        let head = h.head().unwrap();
        let mut n = 0usize;
        let mut bad = None;
        // consume carried bytes first
        if !h.buf.is_empty() { if !fpat.ok(&h.buf, 0) { bad = Some(0); } n += h.buf.len(); recvd.fetch_add(h.buf.len() as u64, Ordering::Relaxed); h.buf.clear(); }
        let mut buf = vec![0u8; 16384];
        while n < total {
            let r = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
            if r <= 0 { break; }
            let k = r as usize;
            if bad.is_none() && !fpat.ok(&buf[..k], n) { bad = Some(n); }
            n += k;
            recvd.fetch_add(k as u64, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(2));
        }
        (head, n, bad, u2)
    }) };
    // sampler
    let sm = { let (sent, recvd, stop) = (sent.clone(), recvd.clone(), stop.clone()); std::thread::spawn(move || {
        let mut max_gap = 0i64; let mut samples = 0; let mut last = (0u64, 0u64);
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(50));
            let (s, r) = (sent.load(Ordering::Relaxed), recvd.load(Ordering::Relaxed));
            max_gap = max_gap.max(s as i64 - r as i64);
            samples += 1; last = (s, r);
        }
        (max_gap, samples, last)
    }) };
    let rss0 = vm_rss_kb();
    let mut relay = Relay::new(1 << 16, nonblocking);
    let mut ctl = NoCtl;
    let t0 = Instant::now();
    let cpu0 = thread_cpu_ms();
    let mut peak_rss = rss0;
    let r = relay.http_message(a, b, &mut ctl);
    let cpu = thread_cpu_ms() - cpu0;
    peak_rss = peak_rss.max(vm_rss_kb());
    println!("  gateway http_message => {:?} in {:?}; gateway thread CPU {cpu:.0}ms; stat={:?}", r.map_err(|e| errno_name(&e)), t0.elapsed(), relay.stat);
    stop.store(true, Ordering::Relaxed);
    let (max_gap, samples, last) = sm.join().unwrap();
    let (head, n, bad, _u2) = ur.join().unwrap();
    let _ = cs.join().unwrap();
    println!("  U got head='{}' body={n}/{total} mismatch={bad:?}", head.lines().next().unwrap());
    println!("  sampled every 50ms x{samples}: max(C_sent - U_received) = {} KiB ({:.2} MiB); final sent/recv={:?}", max_gap / 1024, max_gap as f64 / 1048576.0, last);
    println!("  gateway RSS before={rss0} KiB after={} KiB peak(sampled end)={peak_rss} KiB; max pipe occupancy observed={} bytes (cap {})", vm_rss_kb(), relay.stat.max_pipe, relay.cap);
}

/// perf: 256MiB through splice relay vs read/write copy relay (indicative only).
fn c4(mib: usize, rec: usize, mode: &str) {
    let rec_s = rec;
    let total = mib << 20;
    let GwK { gw, c, c_kc: _, u, u_kc: _ } = build_gw_k(true);
    let (cfd, ufd, a, b) = (c.as_raw_fd(), u.as_raw_fd(), gw.a.as_raw_fd(), gw.b.as_raw_fd());
    let rec = rec_s;
    let chunk = pattern(0, 251 * 1000);
    let cs = std::thread::spawn(move || {
        let fd = c.as_raw_fd();
        let mut n = 0usize;
        while n < total {
            let take = (total - n).min(chunk.len());
            let s = n % 251;
            // chunk is periodic with period 251 and length 251000 -> slice at phase
            let mut o = 0;
            let sl = &chunk[s..s + take.min(chunk.len() - s)];
            while o < sl.len() {
                let r = unsafe { libc::send(fd, sl[o..].as_ptr().cast(), (sl.len() - o).min(rec), 0) };
                assert!(r > 0);
                o += r as usize;
            }
            n += sl.len();
        }
        unsafe { libc::shutdown(fd, libc::SHUT_WR) };
        c
    });
    let ur = std::thread::spawn(move || {
        let fp = FastPat::new();
        let mut buf = vec![0u8; 1 << 18];
        let mut n = 0usize; let mut bad = None;
        loop {
            let r = unsafe { libc::recv(ufd, buf.as_mut_ptr().cast(), buf.len(), 0) };
            if r <= 0 { break; }
            let k = r as usize;
            if bad.is_none() && !fp.ok(&buf[..k], n) { bad = Some(n); }
            n += k;
        }
        (n, bad, u)
    });
    let _ = cfd;
    let cpu0 = thread_cpu_ms();
    let t0 = Instant::now();
    let mut stat = RelayStat::default();
    if mode == "splice" {
        let mut relay = Relay::new(1 << 16, false);
        let mut ctl = NoCtl;
        let r = relay.body_exact(a, b, total, &mut ctl);
        assert!(r.is_ok(), "{:?}", r.map_err(|e| errno_name(&e)));
        stat = relay.stat.clone();
    } else {
        let mut buf = vec![0u8; 1 << 16];
        let mut left = total;
        while left > 0 {
            let r = unsafe { libc::recv(a, buf.as_mut_ptr().cast(), buf.len().min(left), 0) };
            assert!(r > 0);
            let mut o = 0;
            while o < r as usize {
                let w = unsafe { libc::send(b, buf[o..].as_ptr().cast(), r as usize - o, 0) };
                assert!(w > 0);
                o += w as usize;
                stat.send_calls += 1;
            }
            stat.splice_in += 1; // counts recv calls in copy mode
            left -= r as usize;
        }
    }
    let wall = t0.elapsed();
    let cpu = thread_cpu_ms() - cpu0;
    unsafe { libc::shutdown(b, libc::SHUT_WR) };
    let _c = cs.join().unwrap();
    let (n, bad, _u) = ur.join().unwrap();
    println!("  c4 mode={mode:<6} C-record={rec:>5} {mib}MiB: gateway CPU {cpu:>7.0} ms  wall {:>6.0} ms  ({:.0} MB/s)  in_calls={} out/send_calls={}  calls/MiB in={:.1} out={:.1}  U got {n} exact={}",
        wall.as_millis(), (total as f64 / 1e6) / wall.as_secs_f64(), stat.splice_in, stat.splice_out + stat.send_calls,
        stat.splice_in as f64 / mib as f64, (stat.splice_out + stat.send_calls) as f64 / mib as f64, n == total && bad.is_none());
}

fn main() {
    deadline_guard(1500, "c");
    println!("uname -r: {} t={}", uname(), now_ms());
    let which = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    let w = |n: &str| which == "all" || which == n;
    if w("c1") {
        c1(16384, false, false, "plain");
        c1(1000, false, false, "tiny 1000B records (many records, boundaries straddle)");
        c1(16384, true, true, "with KeyUpdate mid-body from both peers");
        c1x(16384, true, true, true, "KeyUpdate at message boundary / inside head / inside chunk framing");
    }
    if w("c3") {
        c3(32, false);
        c3(32, true);
    }
    if w("c6") {
        println!("\n--- c6 hard stall: U does not read at all for 3000ms");
        c3s(64, false, 3000);
        c3s(64, true, 3000);
    }
    if w("c4") {
        let mib = env_usize("MIB", 256);
        for mode in ["splice", "copy", "splice", "copy"] { c4(mib, 16384, mode); }
    }
    if w("c5") {
        let mib = env_usize("MIB", 64);
        for rec in [16384usize, 8192, 4096, 1024, 256] { c4(mib, rec, "splice"); c4(mib, rec, "copy"); }
    }
}

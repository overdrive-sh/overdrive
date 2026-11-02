//! Spike (a): splice kTLS-RX -> pipe -> kTLS-TX. Record boundaries, non-DATA records, pipe-full.
use gwk::*;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::sync::mpsc;
use std::time::Duration;

fn s_script(sizes: Vec<usize>, tx_done: mpsc::Sender<()>, close_notify: bool) -> Box<dyn FnOnce(&mut SStream) + Send> {
    Box::new(move |st| {
        let mut off = 0;
        for sz in sizes {
            st.write_all(&pattern(off, sz)).unwrap();
            st.flush().unwrap();
            off += sz;
        }
        tx_done.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        if close_notify {
            st.conn.send_close_notify();
            st.flush().unwrap();
        }
        std::thread::sleep(Duration::from_millis(100));
    })
}

fn case(name: &str, sizes: Vec<usize>, len: usize, pipesz: Option<i32>, flags: u32) -> String {
    case_q(name, sizes, len, pipesz, flags, false)
}

fn case_q(name: &str, sizes: Vec<usize>, len: usize, pipesz: Option<i32>, flags: u32, quiet: bool) -> String {
    if !quiet { println!("\n=== CASE {name}: record sizes {:?}, splice len={len}, pipe={:?}, flags={flags:#x}", &sizes[..sizes.len().min(10)], pipesz); }
    let (txd, rxd) = chan();
    let total: usize = sizes.iter().sum();
    let (gw, s, r) = build_gw(0, true, s_script(sizes, txd, true), Box::new(r_sink));
    rxd.recv().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    std::thread::sleep(Duration::from_millis(200)); // all records now queued in A's receive path
    let (pr, pw) = mkpipe(pipesz);
    if !quiet { println!("pipe capacity={}", pipe_size(pr)); }
    let (a, b) = (gw.a.as_raw_fd(), gw.b.as_raw_fd());
    let mut calls = vec![];
    let mut eagain = 0;
    let mut moved = 0usize;
    'l: loop {
        match splice(a, pw, len, flags) {
            Ok(0) => {
                calls.push("EOF(0)".to_string());
                break;
            }
            Ok(n) => {
                calls.push(format!("{n}"));
                moved += n;
                let mut left = n;
                while left > 0 {
                    match splice(pr, b, left, 0) {
                        Ok(m) => left -= m,
                        Err(e) => {
                            calls.push(format!("OUT-ERR {}", errno_name(&e)));
                            break 'l;
                        }
                    }
                }
            }
            Err(e) if e.raw_os_error() == Some(libc::EAGAIN) => {
                eagain += 1;
                if eagain > 3 {
                    calls.push("EAGAIN(x4) -> stop".into());
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) if e.raw_os_error() == Some(libc::EINVAL) => {
                calls.push("EINVAL".into());
                let again = splice(a, pw, len, flags);
                calls.push(format!("(again: {:?})", again.map_err(|e| errno_name(&e))));
                let mut buf = [0u8; 256];
                let r1 = recv_rec(a, &mut buf, libc::MSG_DONTWAIT);
                match &r1 {
                    Ok((t, n)) => calls.push(format!("recvmsg-> type={t} n={n} bytes={:02x?}", &buf[..(*n).min(8)])),
                    Err(e) => calls.push(format!("recvmsg-> {}", errno_name(e))),
                }
                if let Ok((t, _)) = r1 {
                    if t == REC_ALERT {
                        break;
                    }
                }
            }
            Err(e) => {
                calls.push(format!("ERR {}", errno_name(&e)));
                break;
            }
        }
    }
    if !quiet { println!("splice(A->pipe) sequence: {}", calls.join(" | ")); println!("moved={moved} of {total}; pipe_left={}", pipe_bytes(pr)); }
    unsafe { libc::shutdown(b, libc::SHUT_WR) };
    s.join().unwrap();
    let res = r.join().unwrap();
    let verdict = if res.0 == total && res.1.is_none() { "EXACT" } else { "MISMATCH/LOSS" };
    if !quiet {
        println!("R received n={} first_mismatch={:?}  => {}", res.0, res.1, verdict);
        println!("tls_stat: {}", tls_stat());
    }
    format!("moved={moved} recv={} total={total} first_mismatch={:?} {verdict}", res.0, res.1)
}

/// deterministic xorshift
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 }
    fn range(&mut self, lo: usize, hi: usize) -> usize { lo + (self.next() as usize) % (hi - lo + 1) }
}

/// returns (moved, recv, mismatch)
fn fuzz_one(sizes: Vec<usize>, pipecap: i32, len: usize, drain_pct: usize, seed: u64) -> (usize, usize, Option<usize>, usize) {
    let (txd, rxd) = chan();
    let total: usize = sizes.iter().sum();
    let (gw, s, r) = build_gw(0, true, s_script(sizes, txd, false), Box::new(r_sink));
    rxd.recv().unwrap();
    std::thread::sleep(Duration::from_millis(150));
    let (pr, pw) = mkpipe(Some(pipecap));
    let (a, b) = (gw.a.as_raw_fd(), gw.b.as_raw_fd());
    let mut rng = Rng(seed | 1);
    let mut moved = 0usize;
    let mut short = 0usize; // calls that returned < min(len, ...) cannot know; count calls
    let mut calls = 0usize;
    loop {
        let l = if len == 0 { rng.range(1, 70000) } else { len };
        let r1 = splice(a, pw, l, libc::SPLICE_F_NONBLOCK);
        match r1 {
            Ok(n) if n > 0 => {
                calls += 1;
                moved += n;
            }
            Ok(_) => break,
            Err(e) if e.raw_os_error() == Some(libc::EAGAIN) => {
                // src empty OR pipe full: if pipe has data keep draining, else we are done
                if pipe_bytes(pr) == 0 { break; }
            }
            Err(e) => panic!("splice in: {e}"),
        }
        let pending = pipe_bytes(pr) as usize;
        if pending > 0 {
            let want = if drain_pct >= 100 { pending } else { (pending * rng.range(1, drain_pct) / 100).max(1).min(pending) };
            let mut left = want;
            while left > 0 {
                let m = splice(pr, b, left, 0).unwrap();
                left -= m;
            }
        }
    }
    // final drain
    loop {
        let pending = pipe_bytes(pr) as usize;
        if pending == 0 { break; }
        splice(pr, b, pending, 0).unwrap();
    }
    unsafe { libc::shutdown(b, libc::SHUT_WR) };
    s.join().unwrap();
    let res = r.join().unwrap();
    let _ = (moved, short, total);
    (total, res.0, res.1, calls)
}

fn fuzz(iters: usize) {
    use std::collections::BTreeMap;
    let mut rng = Rng(0x9E3779B97F4A7C15);
    // key: (pipecap, lenpolicy, drain_pct) -> (runs, losses)
    let mut tab: BTreeMap<(i32, String, usize), (usize, usize)> = BTreeMap::new();
    let mut first_fail: Vec<String> = vec![];
    for it in 0..iters {
        let n = rng.range(4, 14);
        let sizes: Vec<usize> = (0..n).map(|_| match rng.range(0, 3) { 0 => 16384, 1 => rng.range(1, 16384), 2 => rng.range(8000, 16384), _ => rng.range(1, 600) }).collect();
        let pipecap = [4096, 8192, 16384, 32768, 65536][rng.range(0, 4)];
        let (lname, len) = match rng.range(0, 3) { 0 => ("pipecap".to_string(), pipecap as usize), 1 => ("1MiB".to_string(), 1 << 20), 2 => ("16384".to_string(), 16384), _ => ("5000".to_string(), 5000) };
        let drain_pct = [100usize, 50, 10][rng.range(0, 2)];
        let seed = rng.next();
        let (total, got, bad, calls) = fuzz_one(sizes.clone(), pipecap, len, drain_pct, seed);
        let loss = got != total || bad.is_some();
        let e = tab.entry((pipecap, lname.clone(), drain_pct)).or_insert((0, 0));
        e.0 += 1;
        if loss {
            e.1 += 1;
            if first_fail.len() < 6 { first_fail.push(format!("it={it} pipecap={pipecap} len={lname} drain={drain_pct}% total={total} got={got} mismatch={bad:?} calls={calls} sizes={sizes:?}")); }
        }
    }
    println!("pipecap len_policy drain% runs losses");
    for ((pc, l, d), (runs, loss)) in &tab {
        println!("{pc:>7} {l:>8} {d:>4}% {runs:>4} {loss:>4}");
    }
    for f in first_fail { println!("FAIL {f}"); }
}

/// fixed pipe + drain policy, random record sizes (many per conn) and random per-call len.
fn fuzz2(pipecap: i32, drain_pct: usize, iters: usize) {
    let mut rng = Rng(0xD1B54A32D192ED03 ^ (pipecap as u64) ^ ((drain_pct as u64) << 40));
    let mut losses = 0;
    let mut bytes = 0usize;
    let mut fails = vec![];
    for it in 0..iters {
        let n = rng.range(20, 60);
        let sizes: Vec<usize> = (0..n).map(|_| match rng.range(0, 3) { 0 => 16384, 1 => rng.range(1, 16384), 2 => rng.range(8000, 16384), _ => rng.range(1, 600) }).collect();
        let seed = rng.next();
        let (total, got, bad, calls) = fuzz_one(sizes.clone(), pipecap, 0, drain_pct, seed);
        bytes += total;
        if got != total || bad.is_some() {
            losses += 1;
            if fails.len() < 3 { fails.push(format!("it={it} total={total} got={got} lost={} mismatch={bad:?} calls={calls}", total as i64 - got as i64)); }
        }
    }
    println!("FUZZ2 pipecap={pipecap} drain<= {drain_pct}% iters={iters} bytes={bytes} runs_with_loss={losses}");
    for f in fails { println!("  {f}"); }
}

fn ev(label: &str, r: String) { println!("  {label:<46} {r}"); }

fn drain_pipe(pr: i32) -> Vec<u8> {
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

/// a6: non-DATA records between DATA records.
fn nondata(no_cmsg_probe: bool) {
    println!("\n=== a6 non-DATA record handling (no_cmsg_probe={no_cmsg_probe}) [S=kTLS sender -> R=kTLS receiver] uname={}", uname());
    let p = ktls_pair(true);
    let (cfd, rfd) = (p.c.as_raw_fd(), p.s.as_raw_fd());
    send_rec(cfd, REC_DATA, &pattern(0, 50)).unwrap();
    send_rec(cfd, REC_HANDSHAKE, &[0x04, 0, 0, 5, 1, 2, 3, 4, 5]).unwrap(); // NewSessionTicket-ish, NOT KeyUpdate
    send_rec(cfd, REC_DATA, &pattern(50, 60)).unwrap();
    send_rec(cfd, REC_ALERT, &[1, 0]).unwrap(); // close_notify
    std::thread::sleep(Duration::from_millis(200));
    let (pr, pw) = mkpipe(Some(1 << 20));
    let r = splice(rfd, pw, 1 << 20, libc::SPLICE_F_NONBLOCK);
    ev("splice#1 (expect 50 DATA)", format!("{:?}  pipe_bytes={}", r.map_err(|e| errno_name(&e)), pipe_bytes(pr)));
    let got = drain_pipe(pr);
    ev("   bytes==pattern(0..50)", format!("{}", got == pattern(0, 50)));
    let r = splice(rfd, pw, 1 << 20, libc::SPLICE_F_NONBLOCK);
    ev("splice#2 (next record is HANDSHAKE)", format!("{:?}  pipe_bytes={}", r.map_err(|e| errno_name(&e)), pipe_bytes(pr)));
    let r = splice(rfd, pw, 1 << 20, libc::SPLICE_F_NONBLOCK);
    ev("splice#3 same state again (sticky?)", format!("{:?}  pipe_bytes={}", r.map_err(|e| errno_name(&e)), pipe_bytes(pr)));
    let r = splice(rfd, pw, 1 << 20, 0);
    ev("splice#3b BLOCKING flags=0 (must not hang)", format!("{:?}", r.map_err(|e| errno_name(&e))));
    if no_cmsg_probe {
        let mut b = [0u8; 64];
        let n = unsafe { libc::recv(rfd, b.as_mut_ptr().cast(), 64, libc::MSG_DONTWAIT) };
        ev("recv() WITHOUT cmsg buffer on HANDSHAKE rec", format!("n={n} {}", if n < 0 { errno_name(&std::io::Error::last_os_error()) } else { String::new() }));
    }
    let mut b = [0u8; 64];
    let r = recv_rec(rfd, &mut b, libc::MSG_DONTWAIT);
    ev("recvmsg+cmsg (get the HANDSHAKE record)", match &r { Ok((t, n)) => format!("type={t} n={n} bytes={:02x?}", &b[..*n]), Err(e) => errno_name(e) });
    let r = splice(rfd, pw, 1 << 20, libc::SPLICE_F_NONBLOCK);
    ev("splice#4 (DATA 60 after control record)", format!("{:?}", r.map_err(|e| errno_name(&e))));
    let got = drain_pipe(pr);
    ev("   bytes==pattern(50..110)", format!("{}", got == pattern(50, 60)));
    let r = splice(rfd, pw, 1 << 20, libc::SPLICE_F_NONBLOCK);
    ev("splice#5 (next is ALERT close_notify)", format!("{:?}", r.map_err(|e| errno_name(&e))));
    let r = recv_rec(rfd, &mut b, libc::MSG_DONTWAIT);
    ev("recvmsg+cmsg (ALERT)", match &r { Ok((t, n)) => format!("type={t} n={n} bytes={:02x?}", &b[..*n]), Err(e) => errno_name(e) });
    unsafe { libc::shutdown(cfd, libc::SHUT_WR) };
    std::thread::sleep(Duration::from_millis(100));
    let r = splice(rfd, pw, 1 << 20, libc::SPLICE_F_NONBLOCK);
    ev("splice#6 after peer FIN", format!("{:?}", r.map_err(|e| errno_name(&e))));
    let r = recv_rec(rfd, &mut b, libc::MSG_DONTWAIT);
    ev("recvmsg after FIN", format!("{:?}", r.map_err(|e| errno_name(&e))));
}

fn main() {
    deadline_guard(900, "a");
    println!("uname -r: {}  rustls=0.23.39  t={}", uname(), now_ms());
    let which = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    let nb = libc::SPLICE_F_NONBLOCK;
    if which == "a6" {
        nondata(false);
        nondata(true);
        return;
    }
    if which == "fuzz2" {
        let pc: i32 = std::env::args().nth(2).unwrap().parse().unwrap();
        let dp: usize = std::env::args().nth(3).unwrap().parse().unwrap();
        let it: usize = std::env::args().nth(4).unwrap().parse().unwrap();
        fuzz2(pc, dp, it);
        return;
    }
    if which == "fuzz" {
        let n: usize = std::env::args().nth(2).and_then(|v| v.parse().ok()).unwrap_or(200);
        fuzz(n);
        return;
    }
    let w = |n: &str| which == "all" || which == n;
    if w("a1") {
        case("a1 varied record sizes, big len", vec![1, 100, 5000, 16384, 16384, 300, 16000, 7, 16384], 1 << 20, None, nb);
    }
    if w("a1b") {
        case("a1b odd sizes not 4K aligned", vec![4093, 8191, 1777, 12345, 9999, 16383], 1 << 20, None, nb);
    }
    if w("a2") {
        case("a2 splice len < record", vec![16384, 16384, 3000], 1000, None, nb);
    }
    if w("a2b") {
        case("a2b splice len straddles records (len=20000)", vec![16384, 16384, 16384, 100], 20000, None, nb);
    }
    if w("a3") {
        case("a3 pipe 4096 bytes, len 65536, 16K records", vec![16384; 4], 65536, Some(4096), nb);
    }
    if w("a3b") {
        case("a3b pipe 16384, len 16384, 16K records", vec![16384; 4], 16384, Some(16384), nb);
    }
    if w("a4") {
        println!("\n=== a4 matrix: pipe size x record size, splice len = pipe capacity (clamp irrelevant), 3 records each");
        for rec in [16384usize, 8192, 5000, 4096, 1000] {
            for ps in [4096, 8192, 16384, 32768, 65536, 1 << 20] {
                let r = case_q("m", vec![rec; 3], ps as usize, Some(ps), nb, true);
                println!("record={rec:>5} pipe={ps:>7} len={ps:>7}: {r}");
            }
        }
    }
    if w("a5") {
        println!("\n=== a5: splice-in into a NON-EMPTY pipe (free space < record). 64KiB pipe pre-filled, 3x16384 records, len=16384");
        for free in [65536usize, 20480, 16384, 12288, 8192, 5000, 4096] {
            let (txd, rxd) = chan();
            let (gw, s, r) = build_gw(0, true, s_script(vec![16384; 3], txd, false), Box::new(r_sink));
            rxd.recv().unwrap();
            std::thread::sleep(Duration::from_millis(200));
            let (pr, pw) = mkpipe(Some(65536));
            let pre = 65536 - free;
            if pre > 0 {
                let z = vec![0xEEu8; pre];
                let w = unsafe { libc::write(pw, z.as_ptr().cast(), pre) };
                assert_eq!(w as usize, pre);
            }
            let a = gw.a.as_raw_fd();
            let mut got: Vec<u8> = vec![];
            let mut rets = vec![];
            let mut prefill = pre;
            loop {
                match splice(a, pw, 16384, libc::SPLICE_F_NONBLOCK) {
                    Ok(n) => {
                        rets.push(n);
                        let mut tmp = vec![0u8; prefill + n];
                        let mut off = 0;
                        while off < tmp.len() {
                            let k = unsafe { libc::read(pr, tmp[off..].as_mut_ptr().cast(), tmp.len() - off) };
                            assert!(k > 0);
                            off += k as usize;
                        }
                        got.extend_from_slice(&tmp[prefill..]);
                        prefill = 0;
                    }
                    Err(e) => { rets.push(0); let _ = e; break; }
                }
            }
            let exact = got == pattern(0, got.len());
            println!("free={free:>5}: rets={rets:?} total={} (want 49152) pattern_exact_prefix={exact} => {}", got.len(), if got.len() == 49152 && exact { "EXACT" } else { "LOSS/CORRUPT" });
            drop(gw);
            drop(s); drop(r);
        }
    }
    if w("a3c") {
        case("a3c pipe 65536 default, len 1MiB, 6x16K records", vec![16384; 6], 1 << 20, None, nb);
    }
}

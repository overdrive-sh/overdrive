//! T3: vsock shutdown ordering. A sender fills the peer's credit (non-blocking send until EAGAIN,
//! or a capped amount), then half-closes (shutdown(SHUT_WR)) or closes. The receiver stalls, then
//! reads to EOF. Falsified by an end-of-stream observed before the final accepted byte.
use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::sys::*;
use crate::Opts;

fn byte(id: u64, i: u64) -> u8 {
    (i.wrapping_mul(31).wrapping_add(id.wrapping_mul(7))) as u8
}

fn stall_ms(id: u64) -> u64 {
    if id % 7 == 0 {
        0
    } else {
        20 + id.wrapping_mul(2654435761) % 80
    }
}

#[derive(Clone, Debug)]
struct Verdict {
    id: u64,
    got: u64,
    sent: u64,
    ok_pattern: bool,
    early_eof: bool,
    excess: bool,
    close_mode: bool,
    fill: bool,
}

type Reports = Arc<Mutex<HashMap<u64, (u64, u8)>>>;
type Verdicts = Arc<Mutex<Vec<Verdict>>>;

fn recv_conn(fd: OwnedFd, reports: Reports, out: Verdicts) {
    let raw = fd.as_raw_fd();
    set_timeouts(raw, Duration::from_secs(30));
    let mut idb = [0u8; 8];
    let mut off = 0;
    while off < 8 {
        let n = recv_blocking(raw, &mut idb[off..]);
        if n <= 0 {
            return;
        }
        off += n as usize;
    }
    let id = u64::from_ne_bytes(idb);
    sleep_ms(stall_ms(id));
    let mut got = 8u64;
    let mut ok = true;
    let mut buf = vec![0u8; 65536];
    loop {
        let n = recv_blocking(raw, &mut buf);
        if n <= 0 {
            break;
        }
        for (j, b) in buf[..n as usize].iter().enumerate() {
            if *b != byte(id, got - 8 + j as u64) {
                ok = false;
                break;
            }
        }
        got += n as u64;
    }
    let end = Instant::now() + Duration::from_secs(15);
    let (sent, mode) = loop {
        if let Some(r) = reports.lock().unwrap().get(&id).copied() {
            break r;
        }
        if Instant::now() > end {
            return;
        }
        sleep_ms(2);
    };
    out.lock().unwrap().push(Verdict { id, got, sent, ok_pattern: ok, early_eof: got < sent, excess: got > sent, close_mode: mode & 1 != 0, fill: mode & 2 != 0 });
}

pub fn recv_side(port: u32, trials: u64, wait_s: u64) -> (Verdicts, Arc<Mutex<u64>>) {
    let reports: Reports = Arc::new(Mutex::new(HashMap::new()));
    let out: Verdicts = Arc::new(Mutex::new(vec![]));
    let dl = vsock_listen(libc::VMADDR_CID_ANY, port, libc::SOCK_STREAM).expect("data listen");
    let rl = vsock_listen(libc::VMADDR_CID_ANY, port + 1, libc::SOCK_STREAM).expect("report listen");
    let rep2 = reports.clone();
    std::thread::spawn(move || loop {
        let Ok(c) = accept(rl.as_raw_fd()) else { continue };
        let mut b = [0u8; 24];
        let mut off = 0;
        while off < 24 {
            let n = recv_blocking(c.as_raw_fd(), &mut b[off..]);
            if n <= 0 {
                break;
            }
            off += n as usize;
        }
        if off == 24 {
            let id = u64::from_ne_bytes(b[0..8].try_into().unwrap());
            let x = u64::from_ne_bytes(b[8..16].try_into().unwrap());
            let mode = u64::from_ne_bytes(b[16..24].try_into().unwrap()) as u8;
            rep2.lock().unwrap().insert(id, (x, mode));
        }
    });
    let (r3, o3) = (reports.clone(), out.clone());
    std::thread::spawn(move || loop {
        let Ok(c) = accept(dl.as_raw_fd()) else { continue };
        let (r, o) = (r3.clone(), o3.clone());
        let _ = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || recv_conn(c, r, o));
    });
    let _ = (trials, wait_s);
    (out, Arc::new(Mutex::new(0)))
}

pub fn send_side(cid: u32, port: u32, trials: u64, conc: u64, id0: u64) -> serde_json::Value {
    let next = Arc::new(AtomicU64::new(0));
    let fails = Arc::new(AtomicU64::new(0));
    let mut hs = vec![];
    for _ in 0..conc {
        let (next, fails) = (next.clone(), fails.clone());
        hs.push(std::thread::spawn(move || loop {
            let i = next.fetch_add(1, Ordering::SeqCst);
            if i >= trials {
                return;
            }
            let id = id0 + i;
            if send_one(cid, port, id).is_err() {
                fails.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }
    for h in hs {
        h.join().unwrap();
    }
    json!({"sender_trials":trials,"sender_errors":fails.load(Ordering::SeqCst)})
}

fn send_one(cid: u32, port: u32, id: u64) -> Result<(), String> {
    let caps: [u64; 10] = [0, 1, 100, 4096, 65536, 200_000, 262_144, 1 << 20, 4 << 20, u64::MAX];
    let h = id.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let cap = caps[((h >> 8) % 10) as usize];
    let fill = cap == u64::MAX;
    let close_mode = (h >> 20) % 3 == 0;
    let fd = vsock_connect(cid, port, libc::SOCK_STREAM).map_err(|e| e.to_string())?;
    let raw = fd.as_raw_fd();
    set_nonblock(raw);
    let mut total = 0u64; // bytes accepted by sendmsg, including the id
    let idb = id.to_ne_bytes();
    let n = send_nb(raw, &idb);
    if n != 8 {
        return Err("id send".into());
    }
    total += 8;
    let mut buf = vec![0u8; 65536];
    let want = if fill { u64::MAX } else { cap };
    let mut payload = 0u64;
    let end = Instant::now() + Duration::from_secs(20);
    while payload < want {
        let chunk = ((want - payload).min(buf.len() as u64)) as usize;
        for (j, b) in buf[..chunk].iter_mut().enumerate() {
            *b = byte(id, payload + j as u64);
        }
        let n = send_nb(raw, &buf[..chunk]);
        if n > 0 {
            payload += n as u64;
            total += n as u64;
            continue;
        }
        let e = errno().raw_os_error();
        if e == Some(libc::EAGAIN) {
            if fill {
                break; // credit exhausted: stop here, never blocking
            }
            let mut p = libc::pollfd { fd: raw, events: libc::POLLOUT, revents: 0 };
            unsafe { libc::poll(&mut p, 1, 200) };
            if Instant::now() > end {
                return Err("send stalled".into());
            }
            continue;
        }
        return Err(format!("send errno {e:?}"));
    }
    // half-close (or close) right after the last accepted byte
    if close_mode {
        drop(fd);
    } else {
        shutdown_wr(raw).map_err(|e| e.to_string())?;
    }
    let rep = vsock_connect(cid, port + 1, libc::SOCK_STREAM).map_err(|e| e.to_string())?;
    let mut b = [0u8; 24];
    b[0..8].copy_from_slice(&id.to_ne_bytes());
    b[8..16].copy_from_slice(&total.to_ne_bytes());
    b[16..24].copy_from_slice(&((close_mode as u64) | ((fill as u64) << 1)).to_ne_bytes());
    send_blocking(rep.as_raw_fd(), &b);
    Ok(())
}

fn summarize(v: &[Verdict], expect: u64) -> serde_json::Value {
    let early = v.iter().filter(|x| x.early_eof).count();
    let excess = v.iter().filter(|x| x.excess).count();
    let bad = v.iter().filter(|x| !x.ok_pattern).count();
    let fill_n = v.iter().filter(|x| x.fill).count();
    let close_n = v.iter().filter(|x| x.close_mode).count();
    let fill_early = v.iter().filter(|x| x.fill && x.early_eof).count();
    let exh = v.iter().filter(|x| x.fill && x.sent >= 100_000).count();
    let ex: Vec<_> = v.iter().filter(|x| x.early_eof || x.excess || !x.ok_pattern).take(8).map(|x| format!("{x:?}")).collect();
    json!({"expected":expect,"verdicts":v.len(),"early_eof":early,"excess_bytes":excess,"pattern_errors":bad,
        "credit_fill_trials":fill_n,"credit_fill_early_eof":fill_early,"credit_exhausted_confirmed(>=100kB accepted)":exh,"close_variant_trials":close_n,
        "shutwr_variant_trials":v.len()-close_n,"examples":ex,
        "max_bytes_accepted":v.iter().map(|x| x.sent).max().unwrap_or(0)})
}

pub fn run(o: &Opts) {
    let port = o.u("port", 7950) as u32;
    let trials = o.u("trials", 1000);
    let conc = o.u("conc", 32);
    match o.s("mode", "loop").as_str() {
        "loop" => {
            let (out, _) = recv_side(port, trials, 30);
            sleep_ms(200);
            let s = send_side(1, port, trials, conc, 1000);
            let end = Instant::now() + Duration::from_secs(40);
            while Instant::now() < end && (out.lock().unwrap().len() as u64) < trials {
                sleep_ms(100);
            }
            let v = out.lock().unwrap().clone();
            println!("{}", json!({"cmd":"t3","mode":"loop","uname":crate::uname(),"sender":s,"summary":summarize(&v, trials)}));
        }
        "recv" => {
            let (out, _) = recv_side(port, trials, 600);
            println!("T3_RECV_READY");
            let end = Instant::now() + Duration::from_secs(o.u("wait-s", 600));
            while Instant::now() < end && (out.lock().unwrap().len() as u64) < trials {
                sleep_ms(200);
            }
            sleep_ms(500);
            let v = out.lock().unwrap().clone();
            println!("{}", json!({"cmd":"t3","mode":"recv","uname":crate::uname(),"summary":summarize(&v, trials)}));
        }
        "send" => {
            let s = send_side(o.u("peer-cid", 2) as u32, port, trials, conc, o.u("id0", 1_000_000));
            println!("{}", json!({"cmd":"t3","mode":"send","uname":crate::uname(),"sender":s}));
        }
        m => panic!("t3 --mode {m}"),
    }
}

//! Application endpoints for the drain matrix: a client that talks to the guest intake and a
//! server standing in for the destination. Both are plain AF_INET programs with no knowledge
//! of vsock or the forwarders. The trial header travels inside the byte stream.
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::sys::*;

pub static IO_TIMEOUT_S: AtomicU64 = AtomicU64::new(60);
/// bytes the server sends the instant a connection is accepted (before it reads anything)
pub static BANNER: AtomicU64 = AtomicU64::new(0);
const MAGIC: u32 = 0x5452_4c31;
pub const HDR: usize = 56;
const TBL: usize = 65521;

fn table() -> &'static Vec<u8> {
    static T: OnceLock<Vec<u8>> = OnceLock::new();
    T.get_or_init(|| {
        let mut x: u64 = 0x1234_5678_9abc_def1;
        (0..TBL)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect()
    })
}

/// byte `off + j` of the (trial, direction) pattern
fn fill(buf: &mut [u8], trial: u64, dir: u8, off: u64) {
    let t = table();
    let mut pos = ((off + trial * 7 + dir as u64 * 13) % TBL as u64) as usize;
    let mut i = 0;
    while i < buf.len() {
        let n = (TBL - pos).min(buf.len() - i);
        buf[i..i + n].copy_from_slice(&t[pos..pos + n]);
        i += n;
        pos = (pos + n) % TBL;
    }
}

fn check(buf: &[u8], trial: u64, dir: u8, off: u64) -> bool {
    let t = table();
    let mut pos = ((off + trial * 7 + dir as u64 * 13) % TBL as u64) as usize;
    let mut i = 0;
    while i < buf.len() {
        let n = (TBL - pos).min(buf.len() - i);
        if buf[i..i + n] != t[pos..pos + n] {
            return false;
        }
        i += n;
        pos = (pos + n) % TBL;
    }
    true
}

#[derive(Clone, Copy, Debug)]
pub struct Trial {
    pub id: u64,
    pub n: u64,
    pub m: u64,
    pub srv_pause_ms: u32,
    pub cli_pause_ms: u32,
    pub fin_data: bool,
    pub srv_rst_after: Option<u64>,
    pub srv_first: bool,
}

impl Trial {
    fn enc(&self) -> [u8; HDR] {
        let mut b = [0u8; HDR];
        b[0..4].copy_from_slice(&MAGIC.to_ne_bytes());
        let flags = (self.srv_rst_after.is_some() as u32) | ((self.srv_first as u32) << 1);
        b[4..8].copy_from_slice(&flags.to_ne_bytes());
        b[8..16].copy_from_slice(&self.id.to_ne_bytes());
        b[16..24].copy_from_slice(&self.n.to_ne_bytes());
        b[24..32].copy_from_slice(&self.m.to_ne_bytes());
        b[32..36].copy_from_slice(&self.srv_pause_ms.to_ne_bytes());
        b[40..48].copy_from_slice(&self.srv_rst_after.unwrap_or(0).to_ne_bytes());
        b
    }
    fn dec(b: &[u8; HDR]) -> Option<Trial> {
        if u32::from_ne_bytes(b[0..4].try_into().unwrap()) != MAGIC {
            return None;
        }
        let flags = u32::from_ne_bytes(b[4..8].try_into().unwrap());
        Some(Trial {
            id: u64::from_ne_bytes(b[8..16].try_into().unwrap()),
            n: u64::from_ne_bytes(b[16..24].try_into().unwrap()),
            m: u64::from_ne_bytes(b[24..32].try_into().unwrap()),
            srv_pause_ms: u32::from_ne_bytes(b[32..36].try_into().unwrap()),
            cli_pause_ms: 0,
            fin_data: false,
            srv_rst_after: if flags & 1 != 0 { Some(u64::from_ne_bytes(b[40..48].try_into().unwrap())) } else { None },
            srv_first: flags & 2 != 0,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct CliRes {
    pub trial: u64,
    pub sent: u64,
    pub send_err: Option<String>,
    pub recv: u64,
    pub recv_ok: bool,
    pub eof: bool,
    pub recv_err: Option<String>,
    pub ms: u64,
    pub lport: u16,
    pub bad_off: u64,
    pub bad_dup: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SrvRes {
    pub trial: u64,
    pub recv: u64,
    pub recv_ok: bool,
    pub eof: bool,
    pub recv_err: Option<String>,
    pub sent: u64,
    pub send_err: Option<String>,
    pub rst_done: bool,
    pub bad_off: u64,
    pub bad_dup: bool,
}

fn errs(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(libc::EPIPE) => "EPIPE".into(),
        Some(libc::ECONNRESET) => "ECONNRESET".into(),
        Some(libc::EAGAIN) => "TIMEOUT".into(),
        Some(libc::ETIMEDOUT) => "ETIMEDOUT".into(),
        _ => e.to_string(),
    }
}

fn recv_some(fd: RawFd, buf: &mut [u8]) -> Result<usize, String> {
    let n = recv_blocking(fd, buf);
    if n < 0 {
        Err(errs(&errno()))
    } else {
        Ok(n as usize)
    }
}

fn send_all(fd: RawFd, buf: &[u8]) -> Result<(), String> {
    let mut off = 0;
    while off < buf.len() {
        let n = send_blocking(fd, &buf[off..]);
        if n < 0 {
            return Err(errs(&errno()));
        }
        off += n as usize;
    }
    Ok(())
}

fn send_pattern(fd: RawFd, trial: u64, dir: u8, total: u64, sent: &mut u64) -> Result<(), String> {
    let mut chunk = vec![0u8; 65536];
    while *sent < total {
        let n = ((total - *sent) as usize).min(chunk.len());
        fill(&mut chunk[..n], trial, dir, *sent);
        send_all(fd, &chunk[..n])?;
        *sent += n as u64;
    }
    Ok(())
}

/// Read until EOF verifying the pattern; returns (bytes, verified, eof, err).
/// first byte index in `buf` that differs from the pattern at stream offset `off`
fn first_mismatch(buf: &[u8], trial: u64, dir: u8, off: u64) -> Option<usize> {
    let t = table();
    let mut pos = ((off + trial * 7 + dir as u64 * 13) % TBL as u64) as usize;
    for (i, b) in buf.iter().enumerate() {
        if *b != t[pos] {
            return Some(i);
        }
        pos += 1;
        if pos == TBL {
            pos = 0;
        }
    }
    None
}

/// (bytes, pattern ok, eof, error, first bad offset or u64::MAX, bytes after the first bad one match the pattern 47616 earlier)
fn recv_pattern(fd: RawFd, trial: u64, dir: u8, first: Option<&[u8]>, stop_after: Option<u64>) -> (u64, bool, bool, Option<String>, u64, bool) {
    let mut buf = vec![0u8; 262144];
    let mut got = 0u64;
    let mut ok = true;
    let mut bad_off = u64::MAX;
    let mut bad_dup = false;
    if let Some(f) = first {
        ok &= check(f, trial, dir, 0);
        got += f.len() as u64;
    }
    loop {
        if let Some(s) = stop_after {
            if got >= s {
                return (got, ok, false, None, bad_off, bad_dup);
            }
        }
        match recv_some(fd, &mut buf) {
            Ok(0) => return (got, ok, true, None, bad_off, bad_dup),
            Ok(n) => {
                if ok {
                    if let Some(i) = first_mismatch(&buf[..n], trial, dir, got) {
                        ok = false;
                        bad_off = got + i as u64;
                        if bad_off >= 47616 {
                            let k = (n - i).min(64);
                            bad_dup = check(&buf[i..i + k], trial, dir, bad_off - 47616);
                        }
                    }
                }
                got += n as u64;
            }
            Err(e) => return (got, ok, false, Some(e), bad_off, bad_dup),
        }
    }
}

pub fn run_client(intake: SocketAddrV4, t: &Trial) -> CliRes {
    let t0 = Instant::now();
    let mut r = CliRes { trial: t.id, ..Default::default() };
    let fd = match socket(libc::AF_INET, libc::SOCK_STREAM) {
        Ok(f) => f,
        Err(e) => {
            r.send_err = Some(e.to_string());
            return r;
        }
    };
    let raw = fd.as_raw_fd();
    set_timeouts(raw, Duration::from_secs(IO_TIMEOUT_S.load(Ordering::Relaxed)));
    if let Err(e) = connect4(raw, intake) {
        r.send_err = Some(format!("connect:{}", errs(&e)));
        return r;
    }
    r.lport = local4(raw).map(|a| a.port()).unwrap_or(0);
    if t.fin_data {
        let _ = setopt_i32(raw, libc::IPPROTO_TCP, libc::TCP_CORK, 1);
    }
    let hdr = t.enc();
    let res = send_all(raw, &hdr).and_then(|_| {
        let mut sent = 0u64;
        let e = send_pattern(raw, t.id, 0, t.n, &mut sent);
        r.sent = sent;
        e
    });
    if let Err(e) = res {
        r.send_err = Some(e);
    } else {
        if !t.fin_data {
            // FIN-only: wait until every written byte has left the send queue
            let end = Instant::now() + Duration::from_secs(5);
            while outq_nsd(raw) > 0 && Instant::now() < end {
                std::thread::sleep(Duration::from_micros(50));
            }
        }
        let _ = shutdown_wr(raw);
    }
    if t.cli_pause_ms > 0 {
        sleep_ms(t.cli_pause_ms as u64);
    }
    let banner = BANNER.load(Ordering::Relaxed);
    if banner > 0 {
        let mut b = vec![0u8; 65536];
        let mut got = 0u64;
        while got < banner {
            let want = ((banner - got) as usize).min(b.len());
            match recv_some(raw, &mut b[..want]) {
                Ok(0) => break,
                Ok(n) => {
                    if !check(&b[..n], 0, 2, got) {
                        r.recv_err = Some("banner_corrupt".into());
                    }
                    got += n as u64;
                }
                Err(e) => {
                    r.recv_err = Some(e);
                    break;
                }
            }
        }
        if got < banner && r.recv_err.is_none() {
            r.recv_err = Some("banner_short".into());
        }
    }
    let (got, ok, eof, err, bo, bd) = recv_pattern(raw, t.id, 1, None, None);
    r.bad_off = bo;
    r.bad_dup = bd;
    r.recv = got;
    r.recv_ok = ok;
    r.eof = eof;
    r.recv_err = err;
    r.ms = t0.elapsed().as_millis() as u64;
    drop(fd);
    r
}

pub type SrvMap = Arc<Mutex<HashMap<u64, SrvRes>>>;

fn serve_conn(fd: OwnedFd, out: SrvMap) {
    let raw = fd.as_raw_fd();
    set_timeouts(raw, Duration::from_secs(IO_TIMEOUT_S.load(Ordering::Relaxed)));
    let banner = BANNER.load(Ordering::Relaxed);
    if banner > 0 {
        let mut s = 0u64;
        if send_pattern(raw, 0, 2, banner, &mut s).is_err() {
            return;
        }
    }
    let mut hdr = [0u8; HDR];
    let mut off = 0;
    while off < HDR {
        match recv_some(raw, &mut hdr[off..]) {
            Ok(0) | Err(_) => return,
            Ok(n) => off += n,
        }
    }
    let Some(t) = Trial::dec(&hdr) else { return };
    let mut r = SrvRes { trial: t.id, ..Default::default() };
    if t.srv_first {
        // server speaks first: a banner before reading anything
        let mut s = 0u64;
        if let Err(e) = send_pattern(raw, t.id, 1, t.m, &mut s) {
            r.send_err = Some(e);
        }
        r.sent = s;
    }
    if t.srv_pause_ms > 0 {
        sleep_ms(t.srv_pause_ms as u64);
    }
    let (got, ok, eof, err, bo, bd) = recv_pattern(raw, t.id, 0, None, t.srv_rst_after);
    r.bad_off = bo;
    r.bad_dup = bd;
    r.recv = got;
    r.recv_ok = ok;
    r.eof = eof;
    r.recv_err = err;
    if t.srv_rst_after.is_some() {
        let l = libc::linger { l_onoff: 1, l_linger: 0 };
        unsafe { libc::setsockopt(raw, libc::SOL_SOCKET, libc::SO_LINGER, &l as *const _ as *const _, 8) };
        r.rst_done = true;
        drop(fd);
        out.lock().unwrap().insert(t.id, r);
        return;
    }
    if !t.srv_first && r.recv_err.is_none() {
        let mut s = 0u64;
        if let Err(e) = send_pattern(raw, t.id, 1, t.m, &mut s) {
            r.send_err = Some(e);
        }
        r.sent = s;
    }
    let _ = shutdown_wr(raw);
    // wait for the client to close so the connection ends cleanly
    let mut b = [0u8; 64];
    loop {
        match recv_blocking(raw, &mut b) {
            n if n <= 0 => break,
            _ => {}
        }
    }
    out.lock().unwrap().insert(t.id, r);
}

pub fn spawn_server(dest: SocketAddrV4, out: SrvMap, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    let l = tcp_listen(dest).expect("dest listen");
    set_nonblock(l.as_raw_fd());
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            let n = unsafe { libc::accept4(l.as_raw_fd(), std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) };
            if n < 0 {
                std::thread::sleep(Duration::from_micros(200));
                continue;
            }
            let fd = unsafe { OwnedFd::from_raw_fd(n) };
            let o = out.clone();
            let _ = std::thread::Builder::new().stack_size(512 * 1024).spawn(move || serve_conn(fd, o));
        }
        let _ = Ipv4Addr::LOCALHOST;
    })
}

// ---------------------------------------------------------------- application crash (process SIGKILL) children
/// A client process that is killed by its parent mid-flow. Progress goes to a 16-byte mmap'd file BEFORE and AFTER each
/// send: `hi` = bytes whose send() may have been issued, `lo` = bytes whose send() returned. After the last byte it
/// half-closes, sets `done`, and parks without ever reading (so a server banner stays unread: close then sends RST).
pub fn run_crash_child(intake: SocketAddrV4, t: &Trial, progress_path: &str, chunk: usize, gap_us: u64) {
    let f = std::fs::OpenOptions::new().read(true).write(true).create(true).open(progress_path).expect("progress file");
    f.set_len(4096).unwrap();
    let p = mmap_shared(f.as_raw_fd(), 4096).expect("mmap progress") as *mut u64;
    let lo = unsafe { std::sync::atomic::AtomicU64::from_ptr(p) };
    let hi = unsafe { std::sync::atomic::AtomicU64::from_ptr(p.add(1)) };
    let done = unsafe { std::sync::atomic::AtomicU64::from_ptr(p.add(2)) };
    let fd = socket(libc::AF_INET, libc::SOCK_STREAM).expect("socket");
    let raw = fd.as_raw_fd();
    connect4(raw, intake).expect("connect");
    let hdr = t.enc();
    send_all(raw, &hdr).expect("hdr");
    let ready = unsafe { std::sync::atomic::AtomicU64::from_ptr(p.add(3)) };
    if t.srv_first {
        // wait until the server's banner is queued unread on this socket: dying then makes the kernel send RST, not FIN
        let end = Instant::now() + Duration::from_secs(5);
        while inq(raw) <= 0 && Instant::now() < end {
            std::thread::sleep(Duration::from_micros(200));
        }
    }
    ready.store(1, Ordering::SeqCst);
    let mut buf = vec![0u8; chunk];
    let mut sent = 0u64;
    while sent < t.n {
        let n = ((t.n - sent) as usize).min(chunk);
        fill(&mut buf[..n], t.id, 0, sent);
        hi.store(sent + n as u64, Ordering::SeqCst);
        if send_all(raw, &buf[..n]).is_err() {
            break;
        }
        sent += n as u64;
        lo.store(sent, Ordering::SeqCst);
        if gap_us > 0 {
            std::thread::sleep(Duration::from_micros(gap_us));
        }
    }
    if sent == t.n {
        let _ = shutdown_wr(raw);
        done.store(1, Ordering::SeqCst);
    }
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

pub fn read_progress(path: &str) -> (u64, u64, u64, u64) {
    let b = std::fs::read(path).unwrap_or_default();
    let u = |o: usize| b.get(o..o + 8).map(|s| u64::from_ne_bytes(s.try_into().unwrap())).unwrap_or(0);
    (u(0), u(8), u(16), u(24))
}

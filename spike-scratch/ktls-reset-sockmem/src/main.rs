//! usage: probe <scenario> [piece_kib] [stop_instead]
//! scenarios: reset | close | plain-reset | noktls-reset | shutwr
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

extern "C" {
    fn setsockopt(fd: i32, level: i32, name: i32, val: *const u8, len: u32) -> i32;
}
const SOL_TCP: i32 = 6;
const TCP_ULP: i32 = 31;
const SOL_TLS: i32 = 282;
const TLS_TX: i32 = 1;
const TLS_RX: i32 = 2;
const SOL_SOCKET: i32 = 1;
const SO_LINGER: i32 = 13;

fn tcp_mem() -> String {
    let s = std::fs::read_to_string("/proc/net/sockstat").unwrap_or_default();
    s.lines().find(|l| l.starts_with("TCP:")).unwrap_or("?").to_string()
}
fn mem_pages() -> u64 {
    tcp_mem().split_whitespace().skip_while(|w| *w != "mem").nth(1).and_then(|v| v.parse().ok()).unwrap_or(0)
}
fn stamp(t0: Instant, what: &str) {
    println!("[{:>7.3}s] {:<34} {}", t0.elapsed().as_secs_f64(), what, tcp_mem());
}
fn arm(fd: i32, dir: i32) {
    // tls12_crypto_info_aes_gcm_128: version 0x0304, cipher 51, iv[8], key[16], salt[4], rec_seq[8]
    let mut info: Vec<u8> = Vec::new();
    info.extend_from_slice(&0x0304u16.to_ne_bytes());
    info.extend_from_slice(&51u16.to_ne_bytes());
    info.extend_from_slice(&[1u8; 8]);
    info.extend_from_slice(&[2u8; 16]);
    info.extend_from_slice(&[3u8; 4]);
    info.extend_from_slice(&0u64.to_be_bytes());
    let rc = unsafe { setsockopt(fd, SOL_TLS, dir, info.as_ptr(), info.len() as u32) };
    assert_eq!(rc, 0, "TLS setsockopt dir={dir}: {}", std::io::Error::last_os_error());
}
fn ulp(fd: i32) {
    let rc = unsafe { setsockopt(fd, SOL_TCP, TCP_ULP, b"tls\0".as_ptr(), 3) };
    assert_eq!(rc, 0, "TCP_ULP: {}", std::io::Error::last_os_error());
}
fn linger0(fd: i32) {
    let l: [i32; 2] = [1, 0];
    unsafe { setsockopt(fd, SOL_SOCKET, SO_LINGER, l.as_ptr().cast(), 8) };
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let scenario = a.get(1).map(String::as_str).unwrap_or("reset").to_string();
    let piece = a.get(2).and_then(|v| v.parse::<usize>().ok()).unwrap_or(64) * 1024;
    let t0 = Instant::now();
    println!("# scenario={scenario} piece={piece}");
    println!("# uname: {}", std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim());
    println!("# tcp_mem: {}", std::fs::read_to_string("/proc/sys/net/ipv4/tcp_mem").unwrap_or_default().trim());
    stamp(t0, "baseline");

    let ln = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = ln.local_addr().unwrap();
    let sender = TcpStream::connect(addr).unwrap();
    let (recv, _) = ln.accept().unwrap();
    let ktls = !scenario.starts_with("plain") && !scenario.starts_with("nok");
    if ktls {
        ulp(sender.as_raw_fd());
        arm(sender.as_raw_fd(), TLS_TX);
        ulp(recv.as_raw_fd());
        arm(recv.as_raw_fd(), TLS_RX);
    }
    stamp(t0, "connected (kTLS armed=true/false below)");
    println!("# ktls={ktls}");

    let sent = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let result = Arc::new(std::sync::Mutex::new(String::new()));
    let h = {
        let mut s = sender.try_clone().unwrap();
        let (sent, done, result) = (sent.clone(), done.clone(), result.clone());
        std::thread::spawn(move || {
            let buf = vec![0x5au8; piece];
            loop {
                match s.write(&buf) {
                    Ok(n) => { sent.fetch_add(n as u64, Ordering::SeqCst); }
                    Err(e) => { *result.lock().unwrap() = format!("{e} (raw {:?})", e.raw_os_error()); break; }
                }
            }
            done.store(true, Ordering::SeqCst);
        })
    };
    // let the sender fill buffers and block
    let mut last = 0;
    let mut quiet = 0;
    while quiet < 6 {
        std::thread::sleep(Duration::from_millis(250));
        let now = sent.load(Ordering::SeqCst);
        if now == last { quiet += 1 } else { quiet = 0; last = now }
    }
    stamp(t0, &format!("sender blocked, sent={} B", sent.load(Ordering::SeqCst)));
    let peak_before = mem_pages();

    match scenario.as_str() {
        "reset" | "plain-reset" | "noktls-reset" => { linger0(recv.as_raw_fd()); drop(recv); }
        "close" => { drop(recv); }
        "shutwr" => { let _ = recv.shutdown(std::net::Shutdown::Both); std::mem::forget(recv); }
        other => panic!("scenario {other}"),
    }
    stamp(t0, "peer reset/closed (t+0)");
    for i in 1..=8 {
        std::thread::sleep(Duration::from_millis(100));
        stamp(t0, &format!("t+{}00ms done={}", i, done.load(Ordering::SeqCst)));
    }
    let wait = Instant::now();
    while !done.load(Ordering::SeqCst) && wait.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("# sender done={} err={}", done.load(Ordering::SeqCst), result.lock().unwrap());
    stamp(t0, "after sender woke/returned");
    let after_wake = mem_pages();
    if done.load(Ordering::SeqCst) { h.join().unwrap(); }
    std::thread::sleep(Duration::from_secs(2));
    stamp(t0, "after wake +2s (sender socket OPEN)");
    drop(sender);
    for i in 1..=6 {
        std::thread::sleep(Duration::from_millis(500));
        stamp(t0, &format!("after sender close +{}ms", i * 500));
    }
    println!("# SUMMARY peak_before_reset={peak_before} after_wake={after_wake}");
    // process exit sampled by the wrapper
}

//! V-29 test applications (throwaway spike, netns-density-295).
//!   vfwd v29-send  guest application: sends numbered datagrams (and empty datagrams) over an ordinary UDP socket.
//!   vfwd v29-peer  off-host peer: logs {i, len, sha, head, t_ns} of every datagram it RECEIVES, in arrival order.
//!   vfwd hazard    host-only: SK_PASS of a zero-length skb into a psock (recvmsg behaviour).
use crate::v18::sha256;
use aya::{
    maps::{MapData, SockHash},
    programs::SkSkb,
    EbpfLoader,
};
use serde_json::json;
use std::io::Write;
use std::net::{SocketAddrV4, UdpSocket};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

fn arg(args: &[String], k: &str) -> Option<String> {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1).cloned())
}
fn flag(args: &[String], k: &str) -> bool {
    args.iter().any(|a| a == k)
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn mono_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// Payload of data datagram `seq` of `size` bytes: "V29D" + seq(be32) + deterministic pattern.
pub fn payload(seq: u32, size: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(size);
    let hdr = [b'V', b'2', b'9', b'D', (seq >> 24) as u8, (seq >> 16) as u8, (seq >> 8) as u8, seq as u8];
    for i in 0..size {
        v.push(if i < 8 { hdr[i] } else { (seq.wrapping_mul(31).wrapping_add((i as u32).wrapping_mul(7))) as u8 });
    }
    v
}

pub fn peer(args: &[String]) {
    let bind: SocketAddrV4 = arg(args, "--bind").unwrap().parse().unwrap();
    let want_sha = flag(args, "--sha");
    let s = UdpSocket::bind(bind).unwrap();
    unsafe {
        let v: i32 = 64 << 20;
        libc::setsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVBUF, &v as *const _ as *const _, 4);
    }
    s.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
    let mut out = std::io::BufWriter::with_capacity(1 << 20, std::io::stdout());
    let mut buf = vec![0u8; 70000];
    let mut i: u64 = 0;
    loop {
        match s.recv_from(&mut buf) {
            Ok((n, from)) => {
                let t = mono_ns();
                let sha = if want_sha { hex(&sha256(&buf[..n]))[..16].to_string() } else { String::new() };
                let head = hex(&buf[..n.min(8)]);
                let _ = writeln!(out, "{}", json!({"ev":"rx","i":i,"len":n,"sha":sha,"head":head,"t_ns":t,"from":from.to_string()}));
                i += 1;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                let _ = out.flush();
            }
            Err(e) => {
                let _ = writeln!(out, "{}", json!({"ev":"peer_err","err":e.to_string()}));
                let _ = out.flush();
            }
        }
    }
}

fn spin_until(t: Instant) {
    while Instant::now() < t {
        std::hint::spin_loop();
    }
}

pub fn send(args: &[String]) {
    let dst: SocketAddrV4 = arg(args, "--dst").unwrap().parse().unwrap();
    let connected = !flag(args, "--unconnected");
    let s = UdpSocket::bind("0.0.0.0:0").unwrap();
    if connected {
        s.connect(dst).unwrap();
    }
    let do_send = |p: &[u8]| -> std::io::Result<usize> {
        if connected {
            s.send(p)
        } else {
            s.send_to(p, dst)
        }
    };
    // --sizes a,b,c : one numbered datagram per size (data only), --reps R times
    // otherwise : --n data datagrams of --size bytes, an empty datagram after every --k data, --gap-us between sends
    let mut sent_data = 0u64;
    let mut sent_empty = 0u64;
    let mut errs = 0u64;
    let mut first_err = String::new();
    if flag(args, "--warm") {
        // establish the association first (the first datagram triggers it); seq u32::MAX is ignored by the analysis
        let _ = do_send(&payload(u32::MAX, 64));
        std::thread::sleep(Duration::from_millis(600));
    }
    let t0 = Instant::now();
    let gap = Duration::from_micros(arg(args, "--gap-us").map(|v| v.parse().unwrap()).unwrap_or(0));
    if let Some(sz) = arg(args, "--sizes") {
        let reps: u32 = arg(args, "--reps").map(|v| v.parse().unwrap()).unwrap_or(1);
        let sizes: Vec<usize> = sz.split(',').map(|x| x.parse().unwrap()).collect();
        let mut seq = 0u32;
        for _ in 0..reps {
            for &n in &sizes {
                let p = if n == 0 { vec![] } else { payload(seq, n) };
                match do_send(&p) {
                    Ok(_) => {
                        if n == 0 {
                            sent_empty += 1
                        } else {
                            sent_data += 1
                        }
                    }
                    Err(e) => {
                        errs += 1;
                        if first_err.is_empty() {
                            first_err = format!("size {n}: {e}");
                        }
                    }
                }
                println!("{}", json!({"ev":"v29_sent","seq":seq,"len":n}));
                seq += 1;
                if gap.as_micros() > 0 {
                    std::thread::sleep(gap);
                }
            }
        }
    } else {
        let n: u32 = arg(args, "--n").map(|v| v.parse().unwrap()).unwrap_or(1000);
        let size: usize = arg(args, "--size").map(|v| v.parse().unwrap()).unwrap_or(1400);
        let k: u32 = arg(args, "--k").map(|v| v.parse().unwrap()).unwrap_or(1);
        let lead: u32 = arg(args, "--lead-empties").map(|v| v.parse().unwrap()).unwrap_or(0);
        let burst: u32 = arg(args, "--burst").map(|v| v.parse().unwrap()).unwrap_or(1); // sends per gap
        let mut next = Instant::now();
        let mut in_burst = 0u32;
        let mut track = |r: std::io::Result<usize>, errs: &mut u64, first: &mut String| {
            if let Err(e) = r {
                *errs += 1;
                if first.is_empty() {
                    *first = e.to_string();
                }
            }
        };
        for _ in 0..lead {
            track(do_send(&[]), &mut errs, &mut first_err);
            sent_empty += 1;
        }
        for i in 0..n {
            if gap.as_micros() > 0 && in_burst == 0 {
                spin_until(next);
                next += gap;
            }
            track(do_send(&payload(i, size)), &mut errs, &mut first_err);
            sent_data += 1;
            if k > 0 && (i + 1) % k == 0 {
                track(do_send(&[]), &mut errs, &mut first_err);
                sent_empty += 1;
            }
            in_burst = (in_burst + 1) % burst.max(1);
        }
    }
    println!(
        "{}",
        json!({"ev":"v29_done","sent_data":sent_data,"sent_empty":sent_empty,"errs":errs,"first_err":first_err,"secs":t0.elapsed().as_secs_f64(),"connected":connected})
    );
    // keep the application socket (and so the host association) alive while the host drains its backlog
    if let Some(ms) = arg(args, "--linger-ms") {
        std::thread::sleep(Duration::from_millis(ms.parse().unwrap()));
    }
}

// ------------------------------------------------------------------ zero-length SK_PASS hazard
fn recv_once(fd: i32, cap: usize) -> (isize, i32) {
    let mut b = vec![0u8; cap];
    let r = unsafe { libc::recv(fd, b.as_mut_ptr() as *mut _, b.len(), libc::MSG_DONTWAIT) };
    (r, if r < 0 { std::io::Error::last_os_error().raw_os_error().unwrap_or(0) } else { 0 })
}
fn recvmsg_once(fd: i32, cap: usize) -> (isize, i32) {
    let mut b = vec![0u8; cap];
    let mut iov = libc::iovec { iov_base: b.as_mut_ptr() as *mut _, iov_len: b.len() };
    let mut m: libc::msghdr = unsafe { std::mem::zeroed() };
    m.msg_iov = &mut iov;
    m.msg_iovlen = 1;
    let r = unsafe { libc::recvmsg(fd, &mut m, libc::MSG_DONTWAIT) };
    (r, if r < 0 { std::io::Error::last_os_error().raw_os_error().unwrap_or(0) } else { 0 })
}
fn poll_in(fd: i32) -> i16 {
    let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    unsafe { libc::poll(&mut p, 1, 200) };
    p.revents
}

/// vfwd hazard --obj <elf>
/// Receiver R is a UDP socket in PASSMAP (psock + pass_verdict returning SK_PASS). Sender S sends to it.
pub fn hazard(args: &[String]) {
    let obj = arg(args, "--obj").unwrap();
    let mut bpf = EbpfLoader::new().load_file(&obj).unwrap();
    let mfd = SockHash::<_, u64>::try_from(bpf.map("PASSMAP").unwrap()).unwrap().fd().try_clone().unwrap();
    {
        let p: &mut SkSkb = bpf.program_mut("pass_verdict").unwrap().try_into().unwrap();
        p.load().unwrap();
        p.attach(&mfd).unwrap();
    }
    let mut report = vec![];
    for (case, in_map) in [("control_no_psock", false), ("psock_sk_pass", true)] {
        let r = UdpSocket::bind("127.0.0.1:0").unwrap();
        let ra = match r.local_addr().unwrap() {
            std::net::SocketAddr::V4(a) => a,
            _ => unreachable!(),
        };
        let s = UdpSocket::bind("127.0.0.1:0").unwrap();
        s.connect(ra).unwrap();
        let rfd = r.as_raw_fd();
        if in_map {
            let mut m: SockHash<&mut MapData, u64> = SockHash::try_from(bpf.map_mut("PASSMAP").unwrap()).unwrap();
            let ck = crate::sys::cookie(rfd);
            m.insert(ck, rfd, 0).unwrap();
        }
        let mut steps = vec![];
        let mut step = |name: &str, what: &str, rr: (isize, i32)| {
            steps.push(json!({"step":name,"call":what,"ret":rr.0,"errno":rr.1,"errno_name":if rr.1==0 {"".to_string()} else {std::io::Error::from_raw_os_error(rr.1).to_string()}}));
        };
        // 1. a normal datagram first
        s.send(b"AAAA").unwrap();
        std::thread::sleep(Duration::from_millis(50));
        step("1 poll", &format!("revents=0x{:x}", poll_in(rfd)), (0, 0));
        step("1 after AAAA", "recv", recv_once(rfd, 64));
        // 2. an empty datagram
        s.send(&[]).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        step("2 poll after empty", &format!("revents=0x{:x}", poll_in(rfd)), (0, 0));
        step("2 empty #1", "recv", recv_once(rfd, 64));
        step("2 empty #2", "recvmsg", recvmsg_once(rfd, 64));
        // 3. data behind it
        for w in [b"BBBB", b"CCCC"] {
            s.send(w).unwrap();
        }
        std::thread::sleep(Duration::from_millis(50));
        for i in 0..6 {
            step(&format!("3 after B,C try {i}"), "recv", recv_once(rfd, 64));
        }
        // 4. keep it for a while, send more, retry
        std::thread::sleep(Duration::from_millis(1500));
        for w in [b"DDDD", b"EEEE"] {
            s.send(w).unwrap();
        }
        std::thread::sleep(Duration::from_millis(50));
        for i in 0..3 {
            step(&format!("4 after 1.5s + D,E try {i}"), "recv", recv_once(rfd, 64));
        }
        step("4 poll", &format!("revents=0x{:x}", poll_in(rfd)), (0, 0));
        // 5. remove the socket from the map (psock dropped): does the data come back?
        if in_map {
            let mut m: SockHash<&mut MapData, u64> = SockHash::try_from(bpf.map_mut("PASSMAP").unwrap()).unwrap();
            let _ = m.remove(&crate::sys::cookie(rfd));
            std::thread::sleep(Duration::from_millis(100));
            for i in 0..4 {
                step(&format!("5 after map remove try {i}"), "recv", recv_once(rfd, 64));
            }
        }
        report.push(json!({"case":case,"steps":steps}));
    }
    let cnt: aya::maps::PerCpuArray<_, u64> = aya::maps::PerCpuArray::try_from(bpf.map("CNT").unwrap()).unwrap();
    let get = |i: u32| -> u64 { cnt.get(&i, 0).map(|v| v.iter().sum()).unwrap_or(0) };
    println!("{}", json!({"ev":"hazard","report":report,"pass_all":get(47),"pass_zero":get(48)}));
}

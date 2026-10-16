//! V-2 / K-PS2 guest side. The harness (host) releases flow A and then dials/sends flow B; this guest
//! (a) spin-observes, from user space, whether A's end is visible no later than B's first request byte,
//! and (b) records the guest kernel's own receive order of the host->guest packets from the
//! `vsock:virtio_transport_recv_pkt` tracepoint (same site as the design's flow_end classifier,
//! net/vmw_vsock/virtio_transport_common.c:1622 @ v7.0). Reports are JSON lines on a guest-opened
//! command connection.
#![allow(clippy::all)]
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read};
use std::mem;
use std::os::fd::{FromRawFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst as SS};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

const CMD_PORT: u32 = 52520;
const HOST_CID: u32 = 2;
const TRACING: &str = "/sys/kernel/tracing";

static OUT: Mutex<Option<RawFd>> = Mutex::new(None);
static TRACE: Mutex<VecDeque<Pkt>> = Mutex::new(VecDeque::new());
static SEQ: AtomicU64 = AtomicU64::new(0);
static LOST: AtomicU64 = AtomicU64::new(0);
static BURN: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
struct Pkt {
    seq: u64,
    ts: String,
    src_cid: u32,
    src_port: u32,
    dst_cid: u32,
    dst_port: u32,
    len: u32,
    op: String,
}

fn now_ns() -> u64 {
    let mut ts: libc::timespec = unsafe { mem::zeroed() };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}
fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn write_all(fd: RawFd, mut b: &[u8]) -> io::Result<()> {
    while !b.is_empty() {
        let n = unsafe { libc::send(fd, b.as_ptr() as *const _, b.len(), libc::MSG_NOSIGNAL) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        b = &b[n as usize..];
    }
    Ok(())
}

fn rpt(line: String) {
    let g = OUT.lock().unwrap();
    let mut s = line;
    s.push('\n');
    if let Some(fd) = *g {
        if write_all(fd, s.as_bytes()).is_err() {
            print!("RPT_FAIL {}", s);
        }
    }
}

fn sockaddr_vm(cid: u32, port: u32) -> libc::sockaddr_vm {
    let mut a: libc::sockaddr_vm = unsafe { mem::zeroed() };
    a.svm_family = libc::AF_VSOCK as _;
    a.svm_cid = cid;
    a.svm_port = port;
    a
}

fn vsock_connect(cid: u32, port: u32) -> (RawFd, i32) {
    let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return (-1, errno());
    }
    let a = sockaddr_vm(cid, port);
    let rc = unsafe { libc::connect(fd, &a as *const _ as *const libc::sockaddr, mem::size_of_val(&a) as u32) };
    if rc < 0 {
        let e = errno();
        unsafe { libc::close(fd) };
        return (-1, e);
    }
    (fd, 0)
}

fn vsock_listen_nb(port: u32) -> (RawFd, i32) {
    let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, 0) };
    if fd < 0 {
        return (-1, errno());
    }
    let a = sockaddr_vm(libc::VMADDR_CID_ANY, port);
    if unsafe { libc::bind(fd, &a as *const _ as *const libc::sockaddr, mem::size_of_val(&a) as u32) } < 0
        || unsafe { libc::listen(fd, 64) } < 0
    {
        let e = errno();
        unsafe { libc::close(fd) };
        return (-1, e);
    }
    (fd, 0)
}

fn accept_nb(lfd: RawFd) -> RawFd {
    unsafe { libc::accept4(lfd, std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC) }
}

fn poll0(fd: RawFd) -> i32 {
    let mut p = libc::pollfd { fd, events: (libc::POLLIN | libc::POLLOUT | libc::POLLRDHUP) as i16, revents: 0 };
    let rc = unsafe { libc::poll(&mut p, 1, 0) };
    if rc < 0 { -1 } else { p.revents as i32 }
}

const END_BITS: i32 = (libc::POLLRDHUP | libc::POLLHUP | libc::POLLERR) as i32;

fn write_file(path: &str, val: &str) -> Result<(), String> {
    std::fs::write(path, val).map_err(|e| format!("{path}: {e}"))
}

fn setup_trace() -> Result<(), String> {
    write_file(&format!("{TRACING}/tracing_on"), "0")?;
    let _ = write_file(&format!("{TRACING}/trace_clock"), "global");
    write_file(&format!("{TRACING}/events/vsock/virtio_transport_recv_pkt/enable"), "1")?;
    write_file(&format!("{TRACING}/tracing_on"), "1")?;
    Ok(())
}

fn parse_line(line: &str) -> Option<Pkt> {
    // ... <ts>: virtio_transport_recv_pkt: 2:5000 -> 3:300000 len=5 type=STREAM op=RW flags=0x0 ...
    let i = line.find("virtio_transport_recv_pkt:")?;
    let head = &line[..i];
    let ts = head.split_whitespace().last().unwrap_or("").trim_end_matches(':').to_string();
    let rest = &line[i + "virtio_transport_recv_pkt:".len()..];
    let tok: Vec<&str> = rest.split_whitespace().collect();
    // tok: [src_cid:src_port, ->, dst_cid:dst_port, len=.., type=.., op=.., ...]
    let sp = |s: &str| -> Option<(u32, u32)> {
        let (a, b) = s.split_once(':')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    };
    let (src_cid, src_port) = sp(tok.first()?)?;
    let (dst_cid, dst_port) = sp(tok.get(2)?)?;
    let mut len = 0;
    let mut op = String::new();
    for t in &tok[3..] {
        if let Some(v) = t.strip_prefix("len=") {
            len = v.parse().unwrap_or(0);
        } else if let Some(v) = t.strip_prefix("op=") {
            op = v.to_string();
        }
    }
    Some(Pkt { seq: 0, ts, src_cid, src_port, dst_cid, dst_port, len, op })
}

fn trace_reader() {
    let mut f = match std::fs::File::open(format!("{TRACING}/trace_pipe")) {
        Ok(f) => f,
        Err(e) => {
            rpt(format!("{{\"k\":\"trace_reader_fail\",\"err\":\"{}\"}}", e));
            return;
        }
    };
    let mut buf = vec![0u8; 1 << 16];
    let mut carry = String::new();
    loop {
        let n = match f.read(&mut buf) {
            Ok(0) => continue,
            Ok(n) => n,
            Err(_) => continue,
        };
        carry.push_str(&String::from_utf8_lossy(&buf[..n]));
        while let Some(nl) = carry.find('\n') {
            let line: String = carry.drain(..=nl).collect();
            if line.contains("LOST EVENTS") {
                LOST.fetch_add(1, SS);
            }
            if let Some(mut p) = parse_line(&line) {
                p.seq = SEQ.fetch_add(1, SS);
                let mut g = TRACE.lock().unwrap();
                g.push_back(p);
                while g.len() > 40000 {
                    g.pop_front();
                }
            }
        }
    }
}

struct Trial {
    id: u64,
    var: String,   // v1 dial-drop | v2 paired-drop | v3 guest-opened-close | v4 guest-opened-linger0
    lpa: u32,
    lpb: u32,
    hl: u32,
}

fn run_trial(t: Trial) {
    let guest_opened = t.var == "v3" || t.var == "v4";
    let (lis_b, e) = vsock_listen_nb(t.lpb);
    if lis_b < 0 {
        rpt(format!("{{\"k\":\"setup_fail\",\"id\":{},\"what\":\"lis_b\",\"errno\":{}}}", t.id, e));
        return;
    }
    let mut lis_a = -1;
    let mut fd_a = -1;
    let mut a_local_port = 0u32;
    if guest_opened {
        let (fd, e) = vsock_connect(HOST_CID, t.hl);
        if fd < 0 {
            rpt(format!("{{\"k\":\"setup_fail\",\"id\":{},\"what\":\"connect_a\",\"errno\":{}}}", t.id, e));
            unsafe { libc::close(lis_b) };
            return;
        }
        fd_a = fd;
        let mut sa: libc::sockaddr_vm = unsafe { mem::zeroed() };
        let mut l = mem::size_of_val(&sa) as u32;
        unsafe { libc::getsockname(fd, &mut sa as *mut _ as *mut libc::sockaddr, &mut l) };
        a_local_port = sa.svm_port;
    } else {
        let (l, e) = vsock_listen_nb(t.lpa);
        if l < 0 {
            rpt(format!("{{\"k\":\"setup_fail\",\"id\":{},\"what\":\"lis_a\",\"errno\":{}}}", t.id, e));
            unsafe { libc::close(lis_b) };
            return;
        }
        lis_a = l;
    }
    rpt(format!("{{\"k\":\"ready\",\"id\":{},\"t\":{}}}", t.id, now_ns()));

    let deadline = now_ns() + 3_000_000_000;
    let mut fd_b = -1;
    let (mut t_b_acc, mut t_b_data, mut t_a_acc, mut t_a_end, mut t_sample) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut a_at_b_acc = -3i32; // -3 = not applicable yet (B accepted before A existed / not sampled)
    let mut a_at_b_data = -3i32;
    let mut a_late_accept = false;
    let mut b_bytes = -9i64;
    let mut acked = false;
    let mut followup = 0u64;
    let mut timeout = false;
    loop {
        let now = now_ns();
        if now > deadline {
            timeout = true;
            break;
        }
        // B first, then A (A's state is monotone, so an A sample taken after B is seen is a valid check)
        if fd_b < 0 && poll0(lis_b) & libc::POLLIN as i32 != 0 {
            let fd = accept_nb(lis_b);
            if fd >= 0 {
                fd_b = fd;
                t_b_acc = now_ns();
                if fd_a >= 0 {
                    a_at_b_acc = poll0(fd_a);
                }
            }
        }
        if fd_b >= 0 && t_b_data == 0 {
            let p = poll0(fd_b);
            if p & libc::POLLIN as i32 != 0 {
                t_b_data = now_ns();
                let mut b = [0u8; 64];
                let n = unsafe { libc::recv(fd_b, b.as_mut_ptr() as *mut _, b.len(), libc::MSG_DONTWAIT) };
                b_bytes = n as i64;
                if fd_a < 0 && lis_a >= 0 && poll0(lis_a) & libc::POLLIN as i32 != 0 {
                    let fd = accept_nb(lis_a);
                    if fd >= 0 {
                        fd_a = fd;
                        a_late_accept = true;
                        t_a_acc = now_ns();
                    }
                }
                a_at_b_data = if fd_a >= 0 { poll0(fd_a) } else { -1 };
                t_sample = now_ns();
                followup = t_sample + 20_000_000;
            }
        }
        if fd_a < 0 && lis_a >= 0 && poll0(lis_a) & libc::POLLIN as i32 != 0 {
            let fd = accept_nb(lis_a);
            if fd >= 0 {
                fd_a = fd;
                t_a_acc = now_ns();
            }
        }
        if fd_a >= 0 {
            let p = poll0(fd_a);
            if p & libc::POLLIN as i32 != 0 && t.var == "v2" && !acked {
                let mut b = [0u8; 64];
                let n = unsafe { libc::recv(fd_a, b.as_mut_ptr() as *mut _, b.len(), libc::MSG_DONTWAIT) };
                if n > 0 {
                    unsafe { libc::send(fd_a, b"k".as_ptr() as *const _, 1, libc::MSG_NOSIGNAL) };
                    acked = true;
                }
            }
            if p & END_BITS != 0 && t_a_end == 0 {
                t_a_end = now_ns();
            }
        }
        if t_b_data != 0 && (t_a_end != 0 || now_ns() > followup) {
            break;
        }
    }
    // settle: let the tracepoint lines for this trial reach the reader
    thread::sleep(Duration::from_millis(40));
    let mut lines = Vec::new();
    {
        let g = TRACE.lock().unwrap();
        for p in g.iter() {
            if p.src_cid == HOST_CID
                && (p.dst_port == t.lpa || p.dst_port == t.lpb || (guest_opened && p.src_port == t.hl && p.dst_port == a_local_port))
            {
                lines.push(format!(
                    "[{},\"{}\",{},{},{},{},\"{}\"]",
                    p.seq, p.ts, p.src_port, p.dst_port, p.dst_cid, p.len, p.op
                ));
            }
        }
    }
    rpt(format!(
        "{{\"k\":\"res\",\"id\":{},\"var\":\"{}\",\"timeout\":{},\"a_at_b_acc\":{},\"a_at_b_data\":{},\"a_late_accept\":{},\"b_bytes\":{},\"acked\":{},\"t_b_acc\":{},\"t_b_data\":{},\"t_sample\":{},\"t_a_acc\":{},\"t_a_end\":{},\"a_local_port\":{},\"lost\":{},\"trace\":[{}]}}",
        t.id, t.var, timeout, a_at_b_acc, a_at_b_data, a_late_accept, b_bytes, acked, t_b_acc, t_b_data, t_sample, t_a_acc, t_a_end,
        a_local_port, LOST.load(SS), lines.join(",")
    ));
    for fd in [fd_a, fd_b, lis_a, lis_b] {
        if fd >= 0 {
            unsafe { libc::close(fd) };
        }
    }
}

fn burner() {
    while BURN.load(SS) {
        std::hint::spin_loop();
    }
}

fn local_cid() -> u32 {
    let fd = unsafe { libc::open(b"/dev/vsock\0".as_ptr() as *const _, libc::O_RDONLY) };
    if fd < 0 {
        return 0;
    }
    let mut cid: u32 = 0;
    unsafe { libc::ioctl(fd, 0x7b9, &mut cid as *mut u32) };
    unsafe { libc::close(fd) };
    cid
}

fn main() {
    let rl = libc::rlimit { rlim_cur: 65536, rlim_max: 65536 };
    unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &rl) };
    println!("GUEST_LOCAL_CID {}", local_cid());
    let trace_status = setup_trace();
    println!("GUEST_TRACE_SETUP {:?}", trace_status);
    if trace_status.is_ok() {
        thread::spawn(trace_reader);
    }
    let fd = loop {
        let (fd, e) = vsock_connect(HOST_CID, CMD_PORT);
        if fd >= 0 {
            break fd;
        }
        println!("GUEST_CMD_CONNECT_RETRY errno={}", e);
        thread::sleep(Duration::from_millis(200));
    };
    *OUT.lock().unwrap() = Some(fd);
    let mut uts: libc::utsname = unsafe { mem::zeroed() };
    unsafe { libc::uname(&mut uts) };
    let rel: String = uts.release.iter().take_while(|&&c| c != 0).map(|&c| c as u8 as char).collect();
    let ncpu = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    rpt(format!(
        "{{\"k\":\"hello\",\"uname_r\":\"{}\",\"local_cid\":{},\"ncpu\":{},\"trace_ok\":{},\"trace_err\":\"{}\"}}",
        rel, local_cid(), ncpu, trace_status.is_ok(), trace_status.err().unwrap_or_default()
    ));
    let f = unsafe { std::fs::File::from_raw_fd(libc::dup(fd)) };
    for line in BufReader::new(f).lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let p: Vec<&str> = line.split_whitespace().collect();
        match p.as_slice() {
            ["P"] => rpt("{\"k\":\"pong\"}".into()),
            ["N", id, var, lpa, lpb, hl] => {
                let t = Trial {
                    id: id.parse().unwrap(),
                    var: var.to_string(),
                    lpa: lpa.parse().unwrap(),
                    lpb: lpb.parse().unwrap(),
                    hl: hl.parse().unwrap(),
                };
                thread::spawn(move || run_trial(t));
            }
            ["BURN", n] => {
                BURN.store(true, SS);
                for _ in 0..n.parse::<usize>().unwrap() {
                    thread::spawn(burner);
                }
                rpt("{\"k\":\"burning\"}".into());
            }
            ["UNBURN"] => {
                BURN.store(false, SS);
                rpt("{\"k\":\"unburned\"}".into());
            }
            ["POWEROFF"] => {
                rpt("{\"k\":\"bye\"}".into());
                break;
            }
            other => rpt(format!("{{\"k\":\"bad_cmd\",\"cmd\":\"{:?}\"}}", other)),
        }
    }
    println!("GUEST_CMD_LOOP_END");
}

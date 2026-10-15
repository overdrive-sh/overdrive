//! K-A5 guest side: a model of the guest owner's GUEST-OPENED control connections to the host
//! (connect() to CID 2). The harness commands trials over a command channel (also guest-opened);
//! each trial opens one connection to a host listener and then waits with NO guest-side timer
//! (blocking recv() or epoll_wait(-1)) until the kernel reports the end. Reports are JSON lines.
#![allow(clippy::all)]
use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader};
use std::mem;
use std::os::fd::{FromRawFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::SeqCst as SS};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

const CMD_PORT: u32 = 52520;
const CTRL_PORT: u32 = 52521;
const HOST_CID: u32 = 2;

static OUT: Mutex<Option<RawFd>> = Mutex::new(None);

struct Trial {
    fd: AtomicI32,
    ended: AtomicBool,
    abandoned: AtomicBool,
}
static TRIALS: OnceLock<Mutex<BTreeMap<u64, Arc<Trial>>>> = OnceLock::new();

fn trials() -> &'static Mutex<BTreeMap<u64, Arc<Trial>>> {
    TRIALS.get_or_init(|| Mutex::new(BTreeMap::new()))
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

/// returns (fd, errno-of-connect (0 ok))
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

fn poll0(fd: RawFd) -> i32 {
    let mut p = libc::pollfd { fd, events: (libc::POLLIN | libc::POLLOUT | libc::POLLRDHUP) as i16, revents: 0 };
    let rc = unsafe { libc::poll(&mut p, 1, 0) };
    if rc < 0 { -1 } else { p.revents as i32 }
}

fn so_error(fd: RawFd) -> i32 {
    let mut v: i32 = 0;
    let mut l = 4u32;
    unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_ERROR, &mut v as *mut _ as *mut _, &mut l) };
    v
}

/// What the guest sees at the wake: the first return, then the drain (reads until 0/error/EAGAIN),
/// then a write attempt, the poll bits, SO_ERROR.
fn post_wake(id: u64, fd: RawFd, mode: &str, t_obs: u64, first: String) {
    let mut bytes = 0i64;
    let mut end = String::from("none");
    let mut end_errno = 0;
    let mut buf = [0u8; 4096];
    for _ in 0..64 {
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), libc::MSG_DONTWAIT) };
        if n > 0 {
            bytes += n as i64;
            continue;
        }
        if n == 0 {
            end = "eof".into();
        } else {
            end_errno = errno();
            end = if end_errno == libc::EAGAIN { "again".into() } else { "err".into() };
        }
        break;
    }
    let bits = poll0(fd);
    let soerr = so_error(fd);
    let w = unsafe { libc::send(fd, b"x".as_ptr() as *const _, 1, libc::MSG_NOSIGNAL) };
    let werr = if w < 0 { errno() } else { 0 };
    rpt(format!(
        "{{\"k\":\"end\",\"id\":{},\"mode\":\"{}\",\"t\":{},\"first\":{},\"drain_bytes\":{},\"drain_end\":\"{}\",\"drain_errno\":{},\"poll\":{},\"so_error\":{},\"write_rc\":{},\"write_errno\":{}}}",
        id, mode, t_obs, first, bytes, end, end_errno, bits, soerr, w, werr
    ));
}

fn run_trial(id: u64, mode: String, port: u32, tr: Arc<Trial>) {
    let t0 = now_ns();
    let (fd, e) = vsock_connect(HOST_CID, port);
    let t1 = now_ns();
    if fd < 0 {
        rpt(format!("{{\"k\":\"connect_fail\",\"id\":{},\"t\":{},\"t_start\":{},\"errno\":{}}}", id, t1, t0, e));
        tr.ended.store(true, SS);
        return;
    }
    tr.fd.store(fd, SS);
    rpt(format!("{{\"k\":\"connected\",\"id\":{},\"t\":{},\"t_start\":{}}}", id, t1, t0));
    rpt(format!("{{\"k\":\"waiting\",\"id\":{},\"t\":{}}}", id, now_ns()));
    match mode.as_str() {
        "read" => {
            // blocking read, no timeout: the kernel's report of the end is the only wake-up
            let mut buf = [0u8; 4096];
            let mut total = 0i64;
            loop {
                let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), 0) };
                let t = now_ns();
                if tr.abandoned.load(SS) {
                    rpt(format!("{{\"k\":\"abandoned\",\"id\":{},\"t\":{}}}", id, t));
                    break;
                }
                if n > 0 {
                    total += n as i64;
                    rpt(format!("{{\"k\":\"data\",\"id\":{},\"t\":{},\"n\":{}}}", id, t, n));
                    continue;
                }
                let first = if n == 0 {
                    format!("{{\"ret\":0,\"errno\":0,\"data_before\":{}}}", total)
                } else {
                    format!("{{\"ret\":-1,\"errno\":{},\"data_before\":{}}}", errno(), total)
                };
                tr.ended.store(true, SS);
                post_wake(id, fd, "read", t, first);
                break;
            }
        }
        _ => {
            let ep = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
            let mut ev = libc::epoll_event { events: (libc::EPOLLIN | libc::EPOLLRDHUP) as u32, u64: id };
            unsafe { libc::epoll_ctl(ep, libc::EPOLL_CTL_ADD, fd, &mut ev) };
            let mut total = 0i64;
            loop {
                let mut out: libc::epoll_event = unsafe { mem::zeroed() };
                // timeout -1: no guest-side timer
                let n = unsafe { libc::epoll_wait(ep, &mut out, 1, -1) };
                let t = now_ns();
                if n < 0 {
                    if errno() == libc::EINTR {
                        continue;
                    }
                    rpt(format!("{{\"k\":\"epoll_err\",\"id\":{},\"errno\":{}}}", id, errno()));
                    break;
                }
                if tr.abandoned.load(SS) {
                    rpt(format!("{{\"k\":\"abandoned\",\"id\":{},\"t\":{}}}", id, t));
                    break;
                }
                let bits = out.events;
                let end_bits = bits & ((libc::EPOLLRDHUP | libc::EPOLLHUP | libc::EPOLLERR) as u32);
                if bits & libc::EPOLLIN as u32 != 0 && end_bits == 0 {
                    // plain data wake-up: consume what is there and keep waiting
                    let mut buf = [0u8; 4096];
                    let r = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), libc::MSG_DONTWAIT) };
                    if r > 0 {
                        total += r as i64;
                        rpt(format!("{{\"k\":\"data\",\"id\":{},\"t\":{},\"n\":{}}}", id, t, r));
                        continue;
                    }
                }
                let first = format!("{{\"epoll_events\":{},\"data_before\":{}}}", bits, total);
                tr.ended.store(true, SS);
                post_wake(id, fd, "epoll", t, first);
                break;
            }
            unsafe { libc::close(ep) };
        }
    }
}

/// Query: has the trial's wait ended (event recorded)? plus the instantaneous poll bits (poll with 0 timeout).
fn query(id: u64) -> String {
    let t = trials().lock().unwrap().get(&id).cloned();
    match t {
        None => format!("{{\"k\":\"status\",\"id\":{},\"known\":false}}", id),
        Some(t) => {
            let fd = t.fd.load(SS);
            let bits = if fd >= 0 { poll0(fd) } else { -2 };
            format!(
                "{{\"k\":\"status\",\"id\":{},\"known\":true,\"ended\":{},\"poll\":{}}}",
                id,
                t.ended.load(SS),
                bits
            )
        }
    }
}

fn abandon(id: u64) {
    let t = trials().lock().unwrap().get(&id).cloned();
    if let Some(t) = t {
        t.abandoned.store(true, SS);
        let fd = t.fd.load(SS);
        if fd >= 0 {
            // local operation only: wakes the waiting thread so it can be reaped (not a timer, not a host signal)
            unsafe { libc::shutdown(fd, libc::SHUT_RDWR) };
        }
    }
}

fn release(id: u64) {
    let t = trials().lock().unwrap().remove(&id);
    if let Some(t) = t {
        let fd = t.fd.swap(-1, SS);
        if fd >= 0 {
            unsafe { libc::close(fd) };
        }
    }
}

/// Unrelated idle connection: guest-opened, the host never writes or closes it until the final check.
fn ctrl_thread() {
    let fd = loop {
        let (fd, _e) = vsock_connect(HOST_CID, CTRL_PORT);
        if fd >= 0 {
            break fd;
        }
        thread::sleep(Duration::from_millis(100));
    };
    rpt(format!("{{\"k\":\"ctrl_connected\",\"t\":{}}}", now_ns()));
    let ep = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
    let mut ev = libc::epoll_event { events: (libc::EPOLLIN | libc::EPOLLRDHUP) as u32, u64: 0 };
    unsafe { libc::epoll_ctl(ep, libc::EPOLL_CTL_ADD, fd, &mut ev) };
    loop {
        let mut out: libc::epoll_event = unsafe { mem::zeroed() };
        let n = unsafe { libc::epoll_wait(ep, &mut out, 1, -1) };
        if n < 0 {
            continue;
        }
        let bits = out.events;
        let mut buf = [0u8; 16];
        let r = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), libc::MSG_DONTWAIT) };
        if r == 1 && bits & ((libc::EPOLLRDHUP | libc::EPOLLHUP | libc::EPOLLERR) as u32) == 0 {
            rpt(format!("{{\"k\":\"ctrl_byte\",\"t\":{},\"events\":{}}}", now_ns(), bits));
            continue;
        }
        // anything else on the unrelated idle connection is a FALSE END
        rpt(format!("{{\"k\":\"ctrl_false_end\",\"t\":{},\"events\":{},\"recv\":{},\"errno\":{}}}", now_ns(), bits, r, errno()));
        thread::sleep(Duration::from_millis(100));
    }
}

fn local_cid() -> u32 {
    let fd = unsafe { libc::open(b"/dev/vsock\0".as_ptr() as *const _, libc::O_RDONLY) };
    if fd < 0 {
        return 0;
    }
    let mut cid: u32 = 0;
    // IOCTL_VM_SOCKETS_GET_LOCAL_CID = 0x7b9
    unsafe { libc::ioctl(fd, 0x7b9, &mut cid as *mut u32) };
    unsafe { libc::close(fd) };
    cid
}

fn main() {
    let rl = libc::rlimit { rlim_cur: 65536, rlim_max: 65536 };
    unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &rl) };
    println!("GUEST_LOCAL_CID {}", local_cid());
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
    rpt(format!("{{\"k\":\"hello\",\"uname_r\":\"{}\",\"local_cid\":{},\"t\":{}}}", rel, local_cid(), now_ns()));
    thread::spawn(ctrl_thread);
    let f = unsafe { std::fs::File::from_raw_fd(libc::dup(fd)) };
    let rd = BufReader::new(f);
    for line in rd.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let p: Vec<&str> = line.split_whitespace().collect();
        match p.as_slice() {
            ["P"] => rpt(format!("{{\"k\":\"pong\",\"t\":{}}}", now_ns())),
            ["T", id, mode, port] => {
                let id: u64 = id.parse().unwrap();
                let port: u32 = port.parse().unwrap();
                let tr = Arc::new(Trial { fd: AtomicI32::new(-1), ended: AtomicBool::new(false), abandoned: AtomicBool::new(false) });
                trials().lock().unwrap().insert(id, tr.clone());
                let mode = mode.to_string();
                thread::spawn(move || run_trial(id, mode, port, tr));
            }
            ["Q", id] => rpt(query(id.parse().unwrap())),
            ["X", id] => abandon(id.parse().unwrap()),
            ["R", id] => release(id.parse().unwrap()),
            ["POWEROFF"] => {
                rpt("{\"k\":\"bye\"}".into());
                break;
            }
            other => rpt(format!("{{\"k\":\"bad_cmd\",\"cmd\":\"{:?}\"}}", other)),
        }
    }
    println!("GUEST_CMD_LOOP_END");
}

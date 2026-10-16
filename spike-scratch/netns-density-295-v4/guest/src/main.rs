//! V-4 guest side: a model of the guest owner holding V_g (the guest end of a flow vsock).
//! The harness commands everything over a guest-opened command channel. The guest NEVER reads or
//! writes on a flow unless commanded, so "unread bytes" are exactly what the harness arranges.
#![allow(clippy::all)]
use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader};
use std::mem;
use std::os::fd::{FromRawFd, RawFd};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

const CMD_PORT: u32 = 52520;
const HOST_CID: u32 = 2;

static OUT: Mutex<Option<RawFd>> = Mutex::new(None);
static FDS: Mutex<BTreeMap<u64, RawFd>> = Mutex::new(BTreeMap::new());

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
fn local_port(fd: RawFd) -> u32 {
    let mut a: libc::sockaddr_vm = unsafe { mem::zeroed() };
    let mut l = mem::size_of_val(&a) as u32;
    unsafe { libc::getsockname(fd, &mut a as *mut _ as *mut libc::sockaddr, &mut l) };
    a.svm_port
}
fn peer_port(fd: RawFd) -> u32 {
    let mut a: libc::sockaddr_vm = unsafe { mem::zeroed() };
    let mut l = mem::size_of_val(&a) as u32;
    unsafe { libc::getpeername(fd, &mut a as *mut _ as *mut libc::sockaddr, &mut l) };
    a.svm_port
}
fn poll0(fd: RawFd) -> i32 {
    let mut p = libc::pollfd { fd, events: (libc::POLLIN | libc::POLLOUT | libc::POLLRDHUP) as i16, revents: 0 };
    let rc = unsafe { libc::poll(&mut p, 1, 0) };
    if rc < 0 { -1 } else { p.revents as i32 }
}
fn peek_len(fd: RawFd) -> i64 {
    let mut buf = vec![0u8; 1 << 20];
    let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), libc::MSG_PEEK | libc::MSG_DONTWAIT) };
    if n >= 0 { n as i64 } else { -(errno() as i64) }
}
fn get(id: u64) -> Option<RawFd> {
    FDS.lock().unwrap().get(&id).copied()
}

fn listen_accept(id: u64, port: u32) {
    let l = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    let a = sockaddr_vm(libc::VMADDR_CID_ANY, port);
    let rc = unsafe { libc::bind(l, &a as *const _ as *const libc::sockaddr, mem::size_of_val(&a) as u32) };
    let rc2 = unsafe { libc::listen(l, 8) };
    rpt(format!("{{\"k\":\"listening\",\"id\":{},\"t\":{},\"bind_rc\":{},\"listen_rc\":{}}}", id, now_ns(), rc, rc2));
    let c = unsafe { libc::accept4(l, std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) };
    let t = now_ns();
    unsafe { libc::close(l) };
    if c < 0 {
        rpt(format!("{{\"k\":\"accept_fail\",\"id\":{},\"errno\":{}}}", id, errno()));
        return;
    }
    FDS.lock().unwrap().insert(id, c);
    rpt(format!("{{\"k\":\"accepted\",\"id\":{},\"t\":{},\"lport\":{},\"pport\":{}}}", id, t, local_port(c), peer_port(c)));
}

fn main() {
    let rl = libc::rlimit { rlim_cur: 65536, rlim_max: 65536 };
    unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &rl) };
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
    rpt(format!("{{\"k\":\"hello\",\"uname_r\":\"{}\",\"t\":{}}}", rel, now_ns()));
    let f = unsafe { std::fs::File::from_raw_fd(libc::dup(fd)) };
    for line in BufReader::new(f).lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        let p: Vec<&str> = line.split_whitespace().collect();
        match p.as_slice() {
            ["P"] => rpt(format!("{{\"k\":\"pong\",\"t\":{}}}", now_ns())),
            ["OC", id, port] => {
                let id: u64 = id.parse().unwrap();
                let (fd, e) = vsock_connect(HOST_CID, port.parse().unwrap());
                if fd < 0 {
                    rpt(format!("{{\"k\":\"connect_fail\",\"id\":{},\"errno\":{}}}", id, e));
                } else {
                    FDS.lock().unwrap().insert(id, fd);
                    rpt(format!("{{\"k\":\"connected\",\"id\":{},\"t\":{},\"lport\":{}}}", id, now_ns(), local_port(fd)));
                }
            }
            ["LS", id, port] => {
                let (id, port): (u64, u32) = (id.parse().unwrap(), port.parse().unwrap());
                thread::spawn(move || listen_accept(id, port));
            }
            ["WR", id, n] => {
                let id: u64 = id.parse().unwrap();
                let n: usize = n.parse().unwrap();
                if let Some(fd) = get(id) {
                    let buf = vec![b'g'; n];
                    let r = write_all(fd, &buf);
                    rpt(format!("{{\"k\":\"wrote\",\"id\":{},\"n\":{},\"ok\":{},\"t\":{}}}", id, n, r.is_ok(), now_ns()));
                }
            }
            ["RD", id] => {
                // drain what is readable now (used for the clean-exchange pre-step)
                let id: u64 = id.parse().unwrap();
                if let Some(fd) = get(id) {
                    let mut tot = 0i64;
                    let mut end = String::from("again");
                    let mut buf = vec![0u8; 1 << 20];
                    loop {
                        let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), libc::MSG_DONTWAIT) };
                        if n > 0 { tot += n as i64; continue; }
                        end = if n == 0 { "eof".into() } else if errno() == libc::EAGAIN { "again".into() } else { format!("errno{}", errno()) };
                        break;
                    }
                    rpt(format!("{{\"k\":\"read\",\"id\":{},\"n\":{},\"end\":\"{}\"}}", id, tot, end));
                }
            }
            ["SW", id] => {
                let id: u64 = id.parse().unwrap();
                if let Some(fd) = get(id) {
                    let rc = unsafe { libc::shutdown(fd, libc::SHUT_WR) };
                    rpt(format!("{{\"k\":\"shut_wr\",\"id\":{},\"rc\":{},\"t\":{}}}", id, rc, now_ns()));
                }
            }
            ["Q", id] => {
                let id: u64 = id.parse().unwrap();
                match get(id) {
                    Some(fd) => rpt(format!("{{\"k\":\"status\",\"id\":{},\"peek\":{},\"poll\":{}}}", id, peek_len(fd), poll0(fd))),
                    None => rpt(format!("{{\"k\":\"status\",\"id\":{},\"peek\":-9999,\"poll\":-1}}", id)),
                }
            }
            ["CL", id] => {
                let id: u64 = id.parse().unwrap();
                let fd = FDS.lock().unwrap().remove(&id);
                if let Some(fd) = fd {
                    let t0 = now_ns();
                    let rc = unsafe { libc::close(fd) };
                    let t1 = now_ns();
                    rpt(format!("{{\"k\":\"closed\",\"id\":{},\"t0\":{},\"t1\":{},\"rc\":{},\"errno\":{}}}", id, t0, t1, rc, if rc < 0 { errno() } else { 0 }));
                }
            }
            ["R", id] => {
                let id: u64 = id.parse().unwrap();
                if let Some(fd) = FDS.lock().unwrap().remove(&id) {
                    unsafe { libc::close(fd) };
                }
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

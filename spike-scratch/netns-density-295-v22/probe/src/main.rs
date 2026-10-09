//! V-22 spike guest agent (throwaway). Static, libc only.
//!
//! Guest control port 4000 (host connects to guest CID:4000, one line command,
//! reply lines then END):
//!   ident
//!   hold  <host_port> <count> <local_base> <tag>   connect from local ports
//!         local_base.., send "<tag>-<i>\n" (nothing if tag is "-"), KEEP the fds open.
//!   close <host_port> <count> <local_base> <tag> <withdata 0|1>
//!         connect, optionally send a marker, close immediately.
//!   tick  <tag>                                   write "<tag>\n" on every held fd
//!   forget                                        drop all held fds
use std::ffi::CString;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Mutex;

const CTL_PORT: u32 = 4000;
static HELD: Mutex<Vec<OwnedFd>> = Mutex::new(Vec::new());

fn sa(cid: u32, port: u32) -> libc::sockaddr_vm {
    let mut s: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
    s.svm_family = libc::AF_VSOCK as libc::sa_family_t;
    s.svm_cid = cid;
    s.svm_port = port;
    s
}

fn errno() -> io::Error { io::Error::last_os_error() }

fn sock() -> io::Result<OwnedFd> {
    let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 { return Err(errno()); }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn bind_port(fd: RawFd, port: u32) -> io::Result<()> {
    let s = sa(u32::MAX, port);
    let rc = unsafe { libc::bind(fd, &s as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_vm>() as u32) };
    if rc < 0 { Err(errno()) } else { Ok(()) }
}

fn timeout(fd: RawFd) {
    let tv = libc::timeval { tv_sec: 10, tv_usec: 0 };
    unsafe { libc::setsockopt(fd, 40, 8, &tv as *const _ as *const libc::c_void, std::mem::size_of::<libc::timeval>() as u32) };
}

fn dial(host_port: u32, local_port: u32) -> io::Result<OwnedFd> {
    let fd = sock()?;
    timeout(fd.as_raw_fd());
    bind_port(fd.as_raw_fd(), local_port)?;
    let s = sa(2, host_port);
    let rc = unsafe { libc::connect(fd.as_raw_fd(), &s as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_vm>() as u32) };
    if rc < 0 { return Err(errno()); }
    Ok(fd)
}

fn send(fd: RawFd, b: &[u8]) -> io::Result<()> {
    let n = unsafe { libc::send(fd, b.as_ptr() as *const _, b.len(), libc::MSG_NOSIGNAL) };
    if n < 0 { Err(errno()) } else { Ok(()) }
}

fn local_cid() -> String {
    let p = CString::new("/dev/vsock").unwrap();
    let fd = unsafe { libc::open(p.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 { return format!("err:{}", errno()); }
    let mut cid: u32 = 0;
    let rc = unsafe { libc::ioctl(fd, 0x7b9, &mut cid) };
    unsafe { libc::close(fd) };
    if rc < 0 { format!("err:{}", errno()) } else { cid.to_string() }
}

fn command(cmd: &str) -> String {
    let w: Vec<&str> = cmd.split_whitespace().collect();
    match w.as_slice() {
        ["ident"] => {
            let up = std::fs::read_to_string("/proc/uptime").unwrap_or_default();
            let un = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
            format!("IDENT local_cid={} uname={} uptime_s={}", local_cid(), un.trim(), up.split_whitespace().next().unwrap_or("?"))
        }
        ["hold", hp, n, base, tag] => {
            let (hp, n, base): (u32, u32, u32) = (hp.parse().unwrap(), n.parse().unwrap(), base.parse().unwrap());
            let (mut ok, mut fail) = (0, 0);
            let mut errs: Vec<String> = Vec::new();
            for i in 0..n {
                match dial(hp, base + i).and_then(|fd| if *tag == "-" { Ok(fd) } else { send(fd.as_raw_fd(), format!("{tag}-{i}\n").as_bytes()).map(|_| fd) }) {
                    Ok(fd) => { ok += 1; HELD.lock().unwrap().push(fd); }
                    Err(e) => { fail += 1; if errs.len() < 3 { errs.push(format!("i={i}:{e}")); } }
                }
            }
            format!("HOLD port={hp} ok={ok} fail={fail} errs={errs:?}")
        }
        ["close", hp, n, base, tag, data] => {
            let (hp, n, base): (u32, u32, u32) = (hp.parse().unwrap(), n.parse().unwrap(), base.parse().unwrap());
            let (mut ok, mut fail) = (0, 0);
            let mut errs: Vec<String> = Vec::new();
            for i in 0..n {
                match dial(hp, base + i) {
                    Ok(fd) => {
                        if *data == "1" { let _ = send(fd.as_raw_fd(), format!("{tag}-{i}\n").as_bytes()); }
                        drop(fd);
                        ok += 1;
                    }
                    Err(e) => { fail += 1; if errs.len() < 3 { errs.push(format!("i={i}:{e}")); } }
                }
            }
            format!("CLOSE port={hp} ok={ok} fail={fail} errs={errs:?}")
        }
        ["tick", tag] => {
            let held = HELD.lock().unwrap();
            let (mut ok, mut fail) = (0, 0);
            for fd in held.iter() {
                if send(fd.as_raw_fd(), format!("{tag}\n").as_bytes()).is_ok() { ok += 1 } else { fail += 1 }
            }
            format!("TICK held={} ok={ok} fail={fail}", held.len())
        }
        ["forget"] => { let n = HELD.lock().unwrap().len(); HELD.lock().unwrap().clear(); format!("FORGET dropped={n}") }
        _ => format!("UNKNOWN {cmd}"),
    }
}

fn read_line(fd: RawFd) -> Option<String> {
    let mut out = Vec::new();
    let mut b = [0u8; 1];
    loop {
        let n = unsafe { libc::recv(fd, b.as_mut_ptr() as *mut _, 1, 0) };
        if n <= 0 { return None; }
        if b[0] == b'\n' { return Some(String::from_utf8_lossy(&out).into_owned()); }
        out.push(b[0]);
    }
}

fn main() {
    let rl = libc::rlimit { rlim_cur: 65536, rlim_max: 65536 };
    unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &rl) };
    // listen on ctl port, retry until the vsock transport exists
    let l = loop {
        let fd = match sock() { Ok(f) => f, Err(_) => { std::thread::sleep(std::time::Duration::from_millis(200)); continue } };
        let s = sa(u32::MAX, CTL_PORT);
        let rc = unsafe { libc::bind(fd.as_raw_fd(), &s as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_vm>() as u32) };
        if rc == 0 && unsafe { libc::listen(fd.as_raw_fd(), 16) } == 0 { break fd; }
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    println!("AGENT_READY local_cid={}", local_cid());
    io::stdout().flush().ok();
    loop {
        let c = unsafe { libc::accept4(l.as_raw_fd(), std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) };
        if c < 0 { continue; }
        let c = unsafe { OwnedFd::from_raw_fd(c) };
        let Some(cmd) = read_line(c.as_raw_fd()) else { continue };
        let reply = command(&cmd);
        println!("AGENT_CMD {cmd} => {reply}");
        io::stdout().flush().ok();
        let _ = send(c.as_raw_fd(), format!("{reply}\nEND\n").as_bytes());
    }
}

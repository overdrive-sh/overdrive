//! Unmodified-workload stand-in for the workload-cgroup spike: plain libc socket calls, no BPF awareness
//! (the `corr` mode reads BPF maps only to REPORT what the guest programs did; it changes nothing).
//!   wcwl single       tcp 18080 + udp 18081 in the started process; socket/bind/listen are its first actions
//!   wcwl childonly    fork first; only the forked child listens (tcp 18082 + udp 18083)
//!   wcwl parentchild  parent listens (18080/18081), then a forked child listens (18082/18083)
//!   wcwl corr         as `single`, then a TCP connect to a non-local destination, then dump the guest maps
use std::fs;
use std::io::Write;

fn say(s: String) {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{s}");
    let _ = o.flush();
}
fn ck(r: i32, what: &str) -> i32 {
    if r < 0 {
        say(format!("WL_ERR {what}: {}", std::io::Error::last_os_error()));
        std::process::exit(2);
    }
    r
}
fn sa4(ip: [u8; 4], port: u16) -> libc::sockaddr_in {
    let mut a: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    a.sin_family = libc::AF_INET as u16;
    a.sin_port = port.to_be();
    a.sin_addr.s_addr = u32::from_ne_bytes(ip);
    a
}
fn tcp_listen(port: u16) -> i32 {
    let fd = ck(unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) }, "socket tcp");
    let a = sa4([0; 4], port);
    ck(unsafe { libc::bind(fd, (&a as *const libc::sockaddr_in).cast(), 16) }, "bind tcp");
    ck(unsafe { libc::listen(fd, 128) }, "listen tcp");
    fd
}
fn udp_bind(port: u16) -> i32 {
    let fd = ck(unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) }, "socket udp");
    let a = sa4([0; 4], port);
    ck(unsafe { libc::bind(fd, (&a as *const libc::sockaddr_in).cast(), 16) }, "bind udp");
    fd
}
fn cg() -> String {
    fs::read_to_string("/proc/self/cgroup").unwrap_or_default().trim().to_owned()
}
fn pid() -> i32 {
    unsafe { libc::getpid() }
}

fn bpf(cmd: i32, attr: &mut [u8; 128]) -> i64 {
    unsafe { libc::syscall(libc::SYS_bpf, cmd, attr.as_mut_ptr(), 128usize) }
}
fn u32at(b: &[u8], o: usize) -> u32 {
    u32::from_ne_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64at(b: &[u8], o: usize) -> u64 {
    u64::from_ne_bytes(b[o..o + 8].try_into().unwrap())
}
fn put64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_ne_bytes());
}
fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_ne_bytes());
}

/// fd of the first map whose name starts with `prefix` (map names are cut at 15 bytes).
fn map_fd_by_name(prefix: &str) -> Option<(i32, u32, u32)> {
    let mut id = 0u32;
    loop {
        let mut a = [0u8; 128];
        put32(&mut a, 0, id);
        if bpf(12, &mut a) < 0 {
            return None;
        }
        id = u32at(&a, 4);
        let mut a = [0u8; 128];
        put32(&mut a, 0, id);
        let fd = bpf(14, &mut a);
        if fd < 0 {
            continue;
        }
        let mut info = [0u8; 128];
        let mut a = [0u8; 128];
        put32(&mut a, 0, fd as u32);
        put32(&mut a, 4, 128);
        put64(&mut a, 8, info.as_mut_ptr() as u64);
        if bpf(15, &mut a) == 0 {
            let name = &info[24..40];
            let end = name.iter().position(|&c| c == 0).unwrap_or(16);
            if name[..end].starts_with(prefix.as_bytes()) {
                return Some((fd as i32, u32at(&info, 8), u32at(&info, 12)));
            }
        }
        unsafe { libc::close(fd as i32) };
    }
}

fn dump(fd: i32, ksz: u32, vsz: u32) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    let mut key = vec![0u8; ksz as usize];
    let mut have = false;
    loop {
        let mut next = vec![0u8; ksz as usize];
        let mut a = [0u8; 128];
        put32(&mut a, 0, fd as u32);
        put64(&mut a, 8, if have { key.as_ptr() as u64 } else { 0 });
        put64(&mut a, 16, next.as_mut_ptr() as u64);
        if bpf(4, &mut a) < 0 {
            return out;
        }
        let mut val = vec![0u8; vsz as usize];
        let mut a = [0u8; 128];
        put32(&mut a, 0, fd as u32);
        put64(&mut a, 8, next.as_ptr() as u64);
        put64(&mut a, 16, val.as_mut_ptr() as u64);
        if bpf(1, &mut a) == 0 {
            out.push((next.clone(), val));
        }
        key = next;
        have = true;
    }
}

fn dotted(a: u32) -> String {
    let b = a.to_ne_bytes();
    format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

fn corr() {
    let dest = ([10, 250, 77, 9], 8080u16);
    let fd = ck(unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) }, "socket conn");
    let mut cookie: u64 = 0;
    let mut len = 8u32;
    ck(unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, 57, (&mut cookie as *mut u64).cast(), &mut len) }, "SO_COOKIE");
    let a = sa4(dest.0, dest.1);
    let rc = unsafe { libc::connect(fd, (&a as *const libc::sockaddr_in).cast(), 16) };
    let err = if rc < 0 { std::io::Error::last_os_error().to_string() } else { "ok".into() };
    say(format!("WL_CORR connect={err} cookie={cookie:#x} dest={}.{}.{}.{}:{}", dest.0[0], dest.0[1], dest.0[2], dest.0[3], dest.1));
    // a few bytes so the option-carrying ACK and the data path both happen
    unsafe { libc::send(fd, b"x".as_ptr().cast(), 1, 0) };
    // The guest flow owner consumes a CHILD_CORRELATION entry once it accepts the intake connection, so poll
    // for ~300 ms and print every distinct entry state seen, in order.
    match map_fd_by_name("CHILD_CORRELAT") {
        Some((mfd, k, v)) => {
            let mut seen: Vec<String> = Vec::new();
            for _ in 0..150 {
                for (key, val) in dump(mfd, k, v) {
                    let line = format!(
                        "WL_CHILD key={:#x} state={} appcookie={:#x} dip={} dport={}",
                        u64at(&key, 0),
                        u32at(&val, 0),
                        u64at(&val, 8),
                        dotted(u32at(&val, 16)),
                        u16::from_ne_bytes([val[20], val[21]])
                    );
                    if !seen.contains(&line) {
                        say(line.clone());
                        seen.push(line);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            if seen.is_empty() {
                say("WL_CHILD none-observed".into());
            }
        }
        None => say("WL_CHILD map CHILD_CORRELAT not found".into()),
    }
    match map_fd_by_name("ORIGIN") {
        Some((mfd, k, v)) => {
            for (key, val) in dump(mfd, k, v) {
                if u64at(&key, 0) == cookie {
                    say(format!("WL_ORIGIN cookie={:#x} ip={} port={}", cookie, dotted(u32at(&val, 0)), u16::from_ne_bytes([val[4], val[5]])));
                }
            }
        }
        None => say("WL_ORIGIN map not found".into()),
    }
    unsafe { libc::close(fd) };
}

fn child_listens(tcp: u16, udp: u16, ready_fd: i32) {
    let _t = tcp_listen(tcp);
    let _u = udp_bind(udp);
    say(format!("WL role=child pid={} ppid={} cg={} tcp={tcp} udp={udp}", pid(), unsafe { libc::getppid() }, cg()));
    unsafe { libc::write(ready_fd, b"r".as_ptr().cast(), 1) };
    std::thread::sleep(std::time::Duration::from_millis(60));
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    match mode.as_str() {
        "single" | "corr" | "parentchild" => {
            let _t = tcp_listen(18080);
            let _u = udp_bind(18081);
            say(format!("WL role=main pid={} cg={} tcp=18080 udp=18081", pid(), cg()));
            if mode == "corr" {
                corr();
            }
            if mode == "parentchild" {
                fork_child();
            }
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
        "childonly" => {
            fork_child();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        other => {
            say(format!("WL_ERR unknown mode {other:?}"));
            std::process::exit(2);
        }
    }
    say("WL_DONE".into());
}

fn fork_child() {
    let mut p = [0i32; 2];
    ck(unsafe { libc::pipe(p.as_mut_ptr()) }, "pipe");
    let pid = ck(unsafe { libc::fork() }, "fork");
    if pid == 0 {
        child_listens(18082, 18083, p[1]);
        unsafe { libc::_exit(0) };
    }
    let mut b = [0u8; 1];
    unsafe { libc::read(p[0], b.as_mut_ptr().cast(), 1) };
    let mut st = 0;
    unsafe { libc::waitpid(pid, &mut st, 0) };
}

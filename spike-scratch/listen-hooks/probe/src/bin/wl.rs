//! Unmodified-workload stand-in: plain libc socket calls, no BPF awareness.
//! `wl <case> [ip] [port]`; prints what it did (with the kernel-assigned port).
use std::io::{self, BufRead, Write};
use std::time::Instant;

fn die(what: &str) -> ! {
    eprintln!("wl: {what}: {}", io::Error::last_os_error());
    std::process::exit(2)
}
fn ck(r: i32, what: &str) -> i32 {
    if r < 0 {
        die(what)
    }
    r
}
fn sock(af: i32, ty: i32) -> i32 {
    ck(unsafe { libc::socket(af, ty | libc::SOCK_CLOEXEC, 0) }, "socket")
}
fn setopt(fd: i32, level: i32, name: i32, val: i32) {
    ck(
        unsafe {
            libc::setsockopt(fd, level, name, (&val as *const i32).cast(), 4)
        },
        "setsockopt",
    );
}
fn sa4(ip: [u8; 4], port: u16) -> libc::sockaddr_in {
    let mut a: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    a.sin_family = libc::AF_INET as u16;
    a.sin_port = port.to_be();
    a.sin_addr.s_addr = u32::from_ne_bytes(ip);
    a
}
fn sa6(ip: [u8; 16], port: u16) -> libc::sockaddr_in6 {
    let mut a: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
    a.sin6_family = libc::AF_INET6 as u16;
    a.sin6_port = port.to_be();
    a.sin6_addr.s6_addr = ip;
    a
}
fn bind4(fd: i32, ip: [u8; 4], port: u16) {
    let a = sa4(ip, port);
    ck(unsafe { libc::bind(fd, (&a as *const libc::sockaddr_in).cast(), 16) }, "bind4");
}
fn bind6(fd: i32, ip: [u8; 16], port: u16) {
    let a = sa6(ip, port);
    ck(unsafe { libc::bind(fd, (&a as *const libc::sockaddr_in6).cast(), 28) }, "bind6");
}
fn port_of(fd: i32) -> u16 {
    let mut ss: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_storage>() as u32;
    ck(unsafe { libc::getsockname(fd, (&mut ss as *mut _ as *mut libc::sockaddr), &mut len) }, "getsockname");
    let b = unsafe { std::slice::from_raw_parts((&ss as *const _ as *const u8).add(2), 2) };
    u16::from_be_bytes([b[0], b[1]])
}
fn listen(fd: i32, backlog: i32) {
    ck(unsafe { libc::listen(fd, backlog) }, "listen");
}
fn say(case: &str, msg: String) {
    println!("wl {case}: {msg}");
    let _ = io::stdout().flush();
}
fn parse_ip(s: &str) -> [u8; 4] {
    let p: Vec<u8> = s.split('.').map(|x| x.parse().unwrap()).collect();
    [p[0], p[1], p[2], p[3]]
}
const ANY6: [u8; 16] = [0; 16];
const LO6: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];

fn stats(case: &str, what: &str, mut v: Vec<u64>) {
    v.sort_unstable();
    let n = v.len();
    let mean = v.iter().sum::<u64>() / n as u64;
    say(
        case,
        format!(
            "{what}_ns n={n} mean={mean} p50={} p99={} max={}",
            v[n / 2],
            v[n * 99 / 100],
            v[n - 1]
        ),
    );
}

fn flood(case: &str, tcp: bool, n: usize) {
    let lim = libc::rlimit { rlim_cur: 200_000, rlim_max: 200_000 };
    ck(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &lim) }, "setrlimit");
    let mut fds = Vec::with_capacity(n);
    let (mut tb, mut tl) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut ports = std::collections::BTreeSet::new();
    for _ in 0..n {
        let fd = sock(libc::AF_INET, if tcp { libc::SOCK_STREAM } else { libc::SOCK_DGRAM });
        let t = Instant::now();
        bind4(fd, [0; 4], 0);
        tb.push(t.elapsed().as_nanos() as u64);
        if tcp {
            let t = Instant::now();
            listen(fd, 16);
            tl.push(t.elapsed().as_nanos() as u64);
        }
        ports.insert(port_of(fd));
        fds.push(fd);
    }
    say(case, format!("sockets={} distinct_ports={}", fds.len(), ports.len()));
    stats(case, "bind", tb);
    if tcp {
        stats(case, "listen", tl);
    }
    for fd in fds {
        unsafe { libc::close(fd) };
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let case = a[1].as_str();
    let ip = a.get(2).filter(|s| s.contains('.')).map(|s| parse_ip(s)).unwrap_or([0; 4]);
    let port: u16 = a.get(3).map(|s| s.parse().unwrap()).unwrap_or(0);
    let n_arg: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    match case {
        // ---- TCP
        "tcp4_wild" => {
            let fd = sock(libc::AF_INET, libc::SOCK_STREAM);
            bind4(fd, [0; 4], 0);
            listen(fd, 16);
            say(case, format!("listening 0.0.0.0:{}", port_of(fd)));
        }
        "tcp4_loop" => {
            let fd = sock(libc::AF_INET, libc::SOCK_STREAM);
            bind4(fd, [127, 0, 0, 1], 0);
            listen(fd, 16);
            say(case, format!("listening 127.0.0.1:{}", port_of(fd)));
        }
        "tcp4_specific" => {
            let fd = sock(libc::AF_INET, libc::SOCK_STREAM);
            bind4(fd, ip, 0);
            listen(fd, 16);
            say(case, format!("listening {:?}:{}", ip, port_of(fd)));
        }
        "tcp6_wild_v6only1" | "tcp6_wild_v6only0" => {
            let fd = sock(libc::AF_INET6, libc::SOCK_STREAM);
            setopt(fd, libc::IPPROTO_IPV6, libc::IPV6_V6ONLY, (case.ends_with('1')) as i32);
            bind6(fd, ANY6, 0);
            listen(fd, 16);
            say(case, format!("listening [::]:{} v6only={}", port_of(fd), case.ends_with('1')));
        }
        "tcp6_loop" => {
            let fd = sock(libc::AF_INET6, libc::SOCK_STREAM);
            bind6(fd, LO6, 0);
            listen(fd, 16);
            say(case, format!("listening [::1]:{}", port_of(fd)));
        }
        "tcp_relisten" => {
            let fd = sock(libc::AF_INET, libc::SOCK_STREAM);
            bind4(fd, [0; 4], 0);
            listen(fd, 16);
            listen(fd, 64);
            listen(fd, 128);
            say(case, format!("listen x3 on port {}", port_of(fd)));
        }
        "tcp_shutdown_relisten" => {
            let fd = sock(libc::AF_INET, libc::SOCK_STREAM);
            bind4(fd, [0; 4], port);
            listen(fd, 16);
            let r = unsafe { libc::shutdown(fd, libc::SHUT_RD) };
            say(case, format!("shutdown(SHUT_RD) rc={r} errno={}", io::Error::last_os_error()));
            let r = unsafe { libc::listen(fd, 16) };
            say(case, format!("second listen rc={r} port={}", port_of(fd)));
        }
        "tcp_fork_child" => {
            let pid = unsafe { libc::fork() };
            if pid == 0 {
                let fd = sock(libc::AF_INET, libc::SOCK_STREAM);
                bind4(fd, [0; 4], 0);
                listen(fd, 16);
                say(case, format!("child pid={} listening 0.0.0.0:{}", std::process::id(), port_of(fd)));
                std::process::exit(0);
            }
            let mut st = 0;
            unsafe { libc::waitpid(pid, &mut st, 0) };
            say(case, format!("parent pid={} child pid={pid} done", std::process::id()));
        }
        "tcp_reuseport2" => {
            let a = sock(libc::AF_INET, libc::SOCK_STREAM);
            setopt(a, libc::SOL_SOCKET, libc::SO_REUSEPORT, 1);
            bind4(a, [0; 4], 0);
            let p = port_of(a);
            let b = sock(libc::AF_INET, libc::SOCK_STREAM);
            setopt(b, libc::SOL_SOCKET, libc::SO_REUSEPORT, 1);
            bind4(b, [0; 4], p);
            listen(a, 16);
            listen(b, 16);
            say(case, format!("two REUSEPORT listeners on {p}"));
        }
        "tcp_accept_children" => {
            let l = sock(libc::AF_INET, libc::SOCK_STREAM);
            bind4(l, [127, 0, 0, 1], 0);
            listen(l, 16);
            let p = port_of(l);
            let c = sock(libc::AF_INET, libc::SOCK_STREAM);
            let a = sa4([127, 0, 0, 1], p);
            ck(unsafe { libc::connect(c, (&a as *const libc::sockaddr_in).cast(), 16) }, "connect");
            let s = ck(unsafe { libc::accept(l, std::ptr::null_mut(), std::ptr::null_mut()) }, "accept");
            say(case, format!("listener {p}; client connected; accepted fd {s}"));
        }
        "tcp4_bind_only" => {
            let fd = sock(libc::AF_INET, libc::SOCK_STREAM);
            bind4(fd, [0; 4], 0);
            say(case, format!("bound (no listen) 0.0.0.0:{}", port_of(fd)));
        }
        "tcp4_listen_unbound" => {
            // listen() with no bind: kernel autobinds an ephemeral port.
            let fd = sock(libc::AF_INET, libc::SOCK_STREAM);
            listen(fd, 16);
            say(case, format!("listen without bind: port {}", port_of(fd)));
        }
        // ---- UDP
        "udp4_wild" => {
            let fd = sock(libc::AF_INET, libc::SOCK_DGRAM);
            bind4(fd, [0; 4], port);
            say(case, format!("bound 0.0.0.0:{}", port_of(fd)));
        }
        "udp4_specific" => {
            let fd = sock(libc::AF_INET, libc::SOCK_DGRAM);
            bind4(fd, ip, port);
            say(case, format!("bound {:?}:{}", ip, port_of(fd)));
        }
        "udp4_loop" => {
            let fd = sock(libc::AF_INET, libc::SOCK_DGRAM);
            bind4(fd, [127, 0, 0, 1], port);
            say(case, format!("bound 127.0.0.1:{}", port_of(fd)));
        }
        "udp4_port0" => {
            let fd = sock(libc::AF_INET, libc::SOCK_DGRAM);
            bind4(fd, [0; 4], 0);
            say(case, format!("bind(port 0) -> assigned {}", port_of(fd)));
        }
        "udp4_autobind_sendto" => {
            let fd = sock(libc::AF_INET, libc::SOCK_DGRAM);
            let a = sa4([127, 0, 0, 1], 9);
            let r = unsafe { libc::sendto(fd, b"x".as_ptr().cast(), 1, 0, (&a as *const libc::sockaddr_in).cast(), 16) };
            say(case, format!("sendto without bind rc={r}; kernel-assigned local port {}", port_of(fd)));
        }
        "udp4_connected_client" => {
            let fd = sock(libc::AF_INET, libc::SOCK_DGRAM);
            let a = sa4([127, 0, 0, 1], 9);
            ck(unsafe { libc::connect(fd, (&a as *const libc::sockaddr_in).cast(), 16) }, "connect");
            say(case, format!("connected UDP, no bind; local port {}", port_of(fd)));
        }
        "udp4_reuseport2" => {
            let a = sock(libc::AF_INET, libc::SOCK_DGRAM);
            setopt(a, libc::SOL_SOCKET, libc::SO_REUSEPORT, 1);
            bind4(a, [0; 4], port);
            let b = sock(libc::AF_INET, libc::SOCK_DGRAM);
            setopt(b, libc::SOL_SOCKET, libc::SO_REUSEPORT, 1);
            bind4(b, [0; 4], port);
            say(case, format!("two REUSEPORT UDP binds on {port}"));
        }
        "udp4_bind_no_port" => {
            let fd = sock(libc::AF_INET, libc::SOCK_DGRAM);
            setopt(fd, libc::IPPROTO_IP, 24 /* IP_BIND_ADDRESS_NO_PORT */, 1);
            bind4(fd, [0; 4], 0);
            say(case, format!("IP_BIND_ADDRESS_NO_PORT bind(0): port now {}", port_of(fd)));
        }
        "udp4_bind_conflict" => {
            let a = sock(libc::AF_INET, libc::SOCK_DGRAM);
            bind4(a, [0; 4], port);
            let b = sock(libc::AF_INET, libc::SOCK_DGRAM);
            let addr = sa4([0; 4], port);
            let r = unsafe { libc::bind(b, (&addr as *const libc::sockaddr_in).cast(), 16) };
            say(case, format!("second bind rc={r} errno={}", io::Error::last_os_error()));
        }
        "udp6_wild_v6only1" | "udp6_wild_v6only0" => {
            let fd = sock(libc::AF_INET6, libc::SOCK_DGRAM);
            setopt(fd, libc::IPPROTO_IPV6, libc::IPV6_V6ONLY, (case.ends_with('1')) as i32);
            bind6(fd, ANY6, port);
            say(case, format!("bound [::]:{} v6only={}", port_of(fd), case.ends_with('1')));
        }
        "udp6_loop" => {
            let fd = sock(libc::AF_INET6, libc::SOCK_DGRAM);
            bind6(fd, LO6, port);
            say(case, format!("bound [::1]:{}", port_of(fd)));
        }
        "udp6_port0" => {
            let fd = sock(libc::AF_INET6, libc::SOCK_DGRAM);
            bind6(fd, ANY6, 0);
            say(case, format!("bind(port 0) -> assigned {}", port_of(fd)));
        }
        // ---- regression helper: listener on ip:port, one accepted self-connection, hold.
        "tcp_intake_hold" => {
            let l = sock(libc::AF_INET, libc::SOCK_STREAM);
            setopt(l, libc::SOL_SOCKET, libc::SO_REUSEADDR, 1);
            bind4(l, ip, port);
            listen(l, 16);
            let c = sock(libc::AF_INET, libc::SOCK_STREAM);
            let a = sa4(ip, port);
            ck(unsafe { libc::connect(c, (&a as *const libc::sockaddr_in).cast(), 16) }, "connect");
            let _s = ck(unsafe { libc::accept(l, std::ptr::null_mut(), std::ptr::null_mut()) }, "accept");
            say(case, "HOLD".to_string());
            let mut line = String::new();
            let _ = io::stdin().lock().read_line(&mut line);
        }
        // ---- floods (hold every socket so every port is distinct)
        "flood_udp" => flood(case, false, n_arg),
        "flood_tcp" => flood(case, true, n_arg),
        other => {
            eprintln!("wl: unknown case {other}");
            std::process::exit(64)
        }
    }
}
